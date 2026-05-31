use std::collections::{HashMap, HashSet};

use crate::front::ast::Typ;
use crate::intermediate::{
  ir_asm::{Instr, Label, Operand, Temp},
  ir_context::IRContext,
  ir_optimization::analysis::alias::*,
};

/// Scalar replacement of aggregates.
pub fn sroa(ctx: &mut IRContext) -> bool {
  let alias_analysis = AliasInfo::new(ctx);
  let mut changed = local_forward(ctx, &alias_analysis);
  if promote_allocs(ctx, &alias_analysis) {
    ctx.finalize_trivial_phis();
    changed = true;
  }
  changed
}

/// Forward-store to subsequent loads within each basic block.
fn local_forward(ctx: &mut IRContext, alias_analysis: &AliasInfo) -> bool {
  let mut changed = false;

  for block in ctx.get_blocks_mut().values_mut() {
    let mut store_map: HashMap<AddressClass, Operand> = HashMap::new();
    let mut new_body: Vec<Instr> = Vec::with_capacity(block.body.len());

    for instr in std::mem::take(&mut block.body) {
      match instr {
        Instr::Store { addr, src } => {
          if let Some(address_class) = alias_analysis.classify_operand(&addr) {
            store_map.insert(address_class, src.clone());
          } else {
            // opque store may alias any escaping slot, but not a non-escaping one
            store_map.retain(|address_class, _| !alias_analysis.escapes(address_class.alloc_id));
          }
          new_body.push(Instr::Store { addr, src });
        }
        Instr::Load { dest, addr } => {
          if let Some(address_class) = alias_analysis.classify_operand(&addr) {
            if let Some(prev) = store_map.get(&address_class).cloned() {
              changed = true;
              new_body.push(Instr::Move { dest, src: prev });
              continue;
            }
            store_map.insert(address_class, Operand::Temp(dest.clone()));
          }
          new_body.push(Instr::Load { dest, addr });
        }
        Instr::Call { .. } | Instr::TailCall { .. } => {
          // callee may write through any escaping pointer.
          store_map.retain(|address_class, _| !alias_analysis.escapes(address_class.alloc_id));
          new_body.push(instr);
        }
        other => new_body.push(other),
      }
    }

    block.body = new_body;
  }

  changed
}

/// Allocation analysis for promotion.
struct AllocInfo {
  /// Offset ceiling (bytes for `alloc`, element count for `alloc_array`).
  size: i64,
  /// Block label of the alloc instruction.
  block_label: Label,
  /// Distinct (offset, type) slots accessed by loads or stores.
  offsets: HashMap<i64, Typ>,
  /// Is allocation is safe to promote.
  promotable: bool,
}

/// Promote safe, nonescaping allocations to virtual registers.
fn promote_allocs(ctx: &mut IRContext, alias: &AliasInfo) -> bool {
  let mut info_by_alloc: HashMap<usize, AllocInfo> = collect_alloc_candidates(ctx, alias);
  if info_by_alloc.is_empty() {
    return false;
  }

  collect_alloc_slots(ctx, alias, &mut info_by_alloc);

  let promotable: HashSet<usize> = info_by_alloc
    .iter()
    .filter(|(_, info)| info.promotable && !info.offsets.is_empty())
    .map(|(id, _)| *id)
    .collect();

  if promotable.is_empty() {
    return false;
  }

  let block_labels: Vec<Label> = ctx.get_blocks().keys().copied().collect();

  // unsealing blocks because predecessors may now be unwritten
  for label in &block_labels {
    ctx.get_blocks_mut().get_mut(label).unwrap().sealed = false;
  }

  initialize_promoted_temps(ctx, &promotable, &info_by_alloc);
  rewrite_memory_ops(ctx, alias, &promotable, &block_labels);

  // and sealing it back
  for &label in &block_labels {
    ctx.seal_block(label);
  }

  remove_dead_instructions(ctx, alias, &promotable, &block_labels);

  true
}

/// Collect non-escaping allocations with constant size as promotion candidates.
fn collect_alloc_candidates(ctx: &IRContext, alias: &AliasInfo) -> HashMap<usize, AllocInfo> {
  let mut info_by_alloc: HashMap<usize, AllocInfo> = HashMap::new();
  for block in ctx.get_blocks().values() {
    for instr in &block.body {
      let (dest_id, size) = match instr {
        Instr::Alloc {
          dest,
          size: Operand::Const((sz, _)),
        } => (dest.0, *sz),
        Instr::AllocArray {
          dest,
          count: Operand::Const((cnt, _)),
          ..
        } => (dest.0, *cnt),
        _ => continue,
      };
      if alias.escapes(dest_id) {
        continue;
      }
      info_by_alloc.entry(dest_id).or_insert(AllocInfo {
        size,
        block_label: block.label,
        offsets: HashMap::new(),
        promotable: true,
      });
    }
  }
  info_by_alloc
}

/// Record (offset, type) slots for each candidate and mark unsafe ones.
fn collect_alloc_slots(
  ctx: &IRContext,
  alias: &AliasInfo,
  info_by_alloc: &mut HashMap<usize, AllocInfo>,
) {
  for block in ctx.get_blocks().values() {
    for instr in &block.body {
      let (addr, ty) = match instr {
        Instr::Load { dest, addr } => (addr.clone(), dest.1.clone()),
        Instr::Store { addr, src } => {
          let ty = match src {
            Operand::Temp(t) => t.1.clone(),
            Operand::Const((_, t)) => t.clone(),
          };
          (addr.clone(), ty)
        }
        _ => continue,
      };
      let Some(AddressClass { alloc_id, offset }) = alias.classify_operand(&addr) else {
        continue;
      };
      let Some(info) = info_by_alloc.get_mut(&alloc_id) else {
        continue;
      };

      if offset < 0 || offset >= info.size {
        info.promotable = false;
        continue;
      }

      let entry = info.offsets.entry(offset).or_insert(ty.clone());
      if *entry != ty {
        info.promotable = false;
      }
    }
  }
}

/// Seed the default value to be assigned to a temporary if it were still at `alloc`.
fn initialize_promoted_temps(
  ctx: &mut IRContext,
  promotable: &HashSet<usize>,
  info_by_alloc: &HashMap<usize, AllocInfo>,
) {
  for &alloc_id in promotable {
    let alloc_block = info_by_alloc[&alloc_id].block_label;
    let mut sorted_offsets: Vec<(i64, Typ)> = info_by_alloc[&alloc_id]
      .offsets
      .iter()
      .map(|(o, t)| (*o, t.clone()))
      .collect();
    sorted_offsets.sort_by_key(|(o, _)| *o);

    let init_temps: Vec<(i64, Temp)> = sorted_offsets
      .iter()
      .map(|(offset, ty)| (*offset, ctx.create_temp(ty.clone())))
      .collect();

    let block = ctx.get_blocks_mut().get_mut(&alloc_block).unwrap();
    let phi_count = block
      .body
      .iter()
      .take_while(|i| matches!(i, Instr::Phi { .. }))
      .count();

    let init_moves: Vec<Instr> = init_temps
      .iter()
      .zip(sorted_offsets.iter())
      .map(|((_, init_temp), (_, ty))| Instr::Move {
        dest: init_temp.clone(),
        src: Operand::Const((0, ty.clone())),
      })
      .collect();

    let mut new_block_body: Vec<Instr> = Vec::with_capacity(block.body.len() + init_moves.len());
    new_block_body.extend(block.body.iter().take(phi_count).cloned());
    new_block_body.extend(init_moves);
    new_block_body.extend(block.body.iter().skip(phi_count).cloned());
    block.body = new_block_body;

    for (offset, init_temp) in init_temps {
      let var_name = create_var_name(alloc_id, offset);
      ctx.write_variable(&var_name, init_temp, alloc_block);
    }
  }
}

/// Replace alloc/load/store with promoted variable.
fn rewrite_memory_ops(
  ctx: &mut IRContext,
  alias: &AliasInfo,
  promotable: &HashSet<usize>,
  block_labels: &[Label],
) {
  for &label in block_labels {
    let body = std::mem::take(&mut ctx.get_blocks_mut().get_mut(&label).unwrap().body);
    let mut new_body: Vec<Instr> = Vec::with_capacity(body.len());

    for instr in body {
      match instr {
        Instr::Store { addr, src } => {
          if let Some((alloc_id, offset)) = tracked_slot(&addr, alias, promotable) {
            let var_name = create_var_name(alloc_id, offset);
            // SSA needs a temp source, so just using a constant
            let temp = match src {
              Operand::Temp(t) => t,
              Operand::Const((c, ty)) => {
                let new_temp = ctx.create_temp(ty.clone());
                new_body.push(Instr::Move {
                  dest: new_temp.clone(),
                  src: Operand::Const((c, ty)),
                });
                new_temp
              }
            };
            ctx.write_variable(&var_name, temp, label);
          } else {
            new_body.push(Instr::Store { addr, src });
          }
        }

        Instr::Load { dest, addr } => {
          if let Some((alloc_id, offset)) = tracked_slot(&addr, alias, promotable) {
            let var_name = create_var_name(alloc_id, offset);
            let var_temp = ctx.read_variable(&var_name, label);
            new_body.push(Instr::Move {
              dest,
              src: Operand::Temp(var_temp),
            });
          } else {
            new_body.push(Instr::Load { dest, addr });
          }
        }

        Instr::Alloc { dest, size } => {
          if !promotable.contains(&dest.0) {
            new_body.push(Instr::Alloc { dest, size });
          }
        }

        Instr::AllocArray { dest, size, count } => {
          if !promotable.contains(&dest.0) {
            new_body.push(Instr::AllocArray { dest, size, count });
          }
        }

        other => new_body.push(other),
      }
    }

    ctx.get_blocks_mut().get_mut(&label).unwrap().body = new_body;
  }
}

/// Remove operations and instructions that were required for the promoted pointer.
fn remove_dead_instructions(
  ctx: &mut IRContext,
  alias: &AliasInfo,
  promotable: &HashSet<usize>,
  block_labels: &[Label],
) {
  for &label in block_labels {
    let block = ctx.get_blocks_mut().get_mut(&label).unwrap();
    block.body.retain(|instr| {
      let dest_id = match instr {
        Instr::BinOp { dest, .. }
        | Instr::UnOp { dest, .. }
        | Instr::Move { dest, .. }
        | Instr::Phi { dest, .. } => dest.0,
        _ => return true,
      };
      !is_dead_promoted_pointer(dest_id, alias, promotable)
    });
  }
}

/// Check if temp is a pointer to a promoted temp.
fn is_dead_promoted_pointer(
  temp_id: usize,
  alias: &AliasInfo,
  promotable: &HashSet<usize>,
) -> bool {
  match alias.alloc_of(temp_id) {
    Some(alloc_id) => promotable.contains(&alloc_id),
    None => false,
  }
}

/// Get (alloc_id, offset) if the address is a tracked slot of a promotable alloc.
fn tracked_slot(
  addr: &Operand,
  alias: &AliasInfo,
  promotable: &HashSet<usize>,
) -> Option<(usize, i64)> {
  let AddressClass { alloc_id, offset } = alias.classify_operand(addr)?;
  if promotable.contains(&alloc_id) {
    Some((alloc_id, offset))
  } else {
    None
  }
}

/// Create a variable name for a promoted temporary.
/// Using `#` in naming because it is illegal for program variables to use it.
/// Also helps to distinguish promoted variables in IR display.
fn create_var_name(alloc_id: usize, offset: i64) -> String {
  format!("#T{alloc_id}_off{offset}")
}
