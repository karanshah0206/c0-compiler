use std::{
  collections::{HashMap, HashSet},
  mem::take,
};

use crate::front::ast::Typ;
use crate::intermediate::{
  ir_asm::{Instr, Label, Operand, Temp},
  ir_context::IRContext,
};
use crate::x86_back::x86_regalloc::interference_graph::TempTypes;

pub fn rewrite_spill(ctx: &mut IRContext, spilt: &[usize], temp_types: &TempTypes) {
  if spilt.is_empty() {
    return;
  }

  let spilt: HashSet<usize> = spilt.iter().copied().collect();
  let mut labels: Vec<Label> = ctx.get_blocks().keys().copied().collect();
  labels.sort_by_key(|label| label.0);

  for label in labels {
    let body: Vec<Instr>;
    let mut terminator: Option<Instr>;
    {
      let block = ctx.get_blocks_mut().get_mut(&label).unwrap();
      body = take(&mut block.body);
      terminator = block.terminator.take();
    }

    let mut new_body: Vec<Instr> = Vec::with_capacity(body.len());
    for mut instr in body {
      // each spill becomes a fresh temp
      let use_patches = collect_use_patches(&instr, &spilt, temp_types);
      let mut use_map: HashMap<usize, Temp> = HashMap::new();
      for (orig_id, typ) in &use_patches {
        use_map.insert(*orig_id, ctx.create_temp(typ.clone()));
      }

      // emitting loads, patched use, and then the instruction
      for (orig_id, _) in &use_patches {
        let fresh = use_map.get(orig_id).unwrap().clone();
        let typ = fresh.1.clone();
        new_body.push(Instr::Move {
          dest: fresh,
          src: Operand::Temp((*orig_id, typ)),
        });
      }
      apply_use_patches(&mut instr, &use_map);

      // replace patched definition with fresh temp
      let def_patch = collect_def_patch(&instr, &spilt, temp_types);
      let store_after: Option<Instr> = if let Some((orig_id, typ)) = def_patch {
        let fresh = ctx.create_temp(typ.clone());
        apply_def_patch(&mut instr, &fresh);
        Some(Instr::Move {
          dest: (orig_id, typ),
          src: Operand::Temp(fresh),
        })
      } else {
        None
      };

      new_body.push(instr);

      if let Some(store) = store_after {
        new_body.push(store);
      }
    }

    // terminator has only uses, no defines
    if let Some(ref mut terminator) = terminator {
      let use_patches = collect_use_patches(terminator, &spilt, temp_types);
      let mut use_map: HashMap<usize, Temp> = HashMap::new();
      for (orig_id, typ) in &use_patches {
        use_map.insert(*orig_id, ctx.create_temp(typ.clone()));
      }
      for (orig_id, _) in &use_patches {
        let fresh = use_map.get(orig_id).unwrap().clone();
        let typ = fresh.1.clone();
        new_body.push(Instr::Move {
          dest: fresh,
          src: Operand::Temp((*orig_id, typ)),
        });
      }
      apply_use_patches(terminator, &use_map);
    }

    let block = ctx.get_blocks_mut().get_mut(&label).unwrap();
    block.body = new_body;
    block.terminator = terminator;
  }
}

/// Collect all spilt temp uses of an instruction that need a load.
fn collect_use_patches(
  instr: &Instr,
  spilt: &HashSet<usize>,
  temp_types: &TempTypes,
) -> Vec<(usize, Typ)> {
  let mut seen: HashSet<usize> = HashSet::new();
  let mut patches: Vec<(usize, Typ)> = Vec::new();
  let mut visit = |op: &Operand| {
    if let Operand::Temp((id, typ)) = op {
      if spilt.contains(id) && seen.insert(*id) {
        let typ = temp_types.get(id).cloned().unwrap_or_else(|| typ.clone());
        patches.push((*id, typ));
      }
    }
  };

  match instr {
    Instr::BinOp { lhs, rhs, .. } => {
      visit(lhs);
      visit(rhs);
    }
    Instr::UnOp { src, .. } => visit(src),
    Instr::JumpIf { pred, .. } => visit(pred),
    Instr::Call { args, .. } | Instr::TailCall { args, .. } => {
      for arg in args {
        visit(arg);
      }
    }
    Instr::Return(Some(op)) => visit(op),
    Instr::Move { src, .. } => visit(src),
    Instr::Load { addr, .. } => visit(addr),
    Instr::Store { addr, src, .. } => {
      visit(addr);
      visit(src);
    }
    Instr::Alloc { size, .. } => visit(size),
    Instr::AllocArray { size, count, .. } => {
      visit(size);
      visit(count);
    }
    Instr::Phi { srcs, .. } => {
      for (_, src) in srcs {
        visit(src);
      }
    }
    _ => {}
  }

  patches.sort_by_key(|(id, _)| *id);
  patches
}

/// Colllect the spilt defined temp of an instruction, if any.
fn collect_def_patch(
  instr: &Instr,
  spilt: &HashSet<usize>,
  temp_types: &TempTypes,
) -> Option<(usize, Typ)> {
  let dest = match instr {
    Instr::BinOp { dest, .. }
    | Instr::UnOp { dest, .. }
    | Instr::Move { dest, .. }
    | Instr::Load { dest, .. }
    | Instr::Alloc { dest, .. }
    | Instr::AllocArray { dest, .. } => Some(dest.clone()),
    Instr::Call { dest: Some(d), .. } => Some(d.clone()),
    Instr::Phi { dest, .. } => Some(dest.clone()),
    _ => None,
  }?;

  if spilt.contains(&dest.0) {
    let typ = temp_types
      .get(&dest.0)
      .cloned()
      .unwrap_or_else(|| dest.1.clone());
    Some((dest.0, typ))
  } else {
    None
  }
}

/// Patches the used temporaries of an instruction.
fn apply_use_patches(instr: &mut Instr, use_map: &HashMap<usize, Temp>) {
  let patch = |op: &mut Operand| {
    if let Operand::Temp((id, _)) = op {
      if let Some(fresh) = use_map.get(id) {
        *op = Operand::Temp(fresh.clone());
      }
    }
  };

  match instr {
    Instr::BinOp { lhs, rhs, .. } => {
      patch(lhs);
      patch(rhs);
    }
    Instr::UnOp { src, .. } => patch(src),
    Instr::JumpIf { pred, .. } => patch(pred),
    Instr::Call { args, .. } | Instr::TailCall { args, .. } => {
      for arg in args.iter_mut() {
        patch(arg);
      }
    }
    Instr::Return(Some(op)) => patch(op),
    Instr::Move { src, .. } => patch(src),
    Instr::Load { addr, .. } => patch(addr),
    Instr::Store { addr, src } => {
      patch(addr);
      patch(src);
    }
    Instr::Alloc { size, .. } => patch(size),
    Instr::AllocArray { size, count, .. } => {
      patch(size);
      patch(count);
    }
    Instr::Phi { srcs, .. } => {
      for (_, src) in srcs.iter_mut() {
        patch(src);
      }
    }
    _ => {}
  }
}

/// Patch the defined temporary of an instruction.
fn apply_def_patch(instr: &mut Instr, fresh: &Temp) {
  match instr {
    Instr::Phi { dest, .. }
    | Instr::BinOp { dest, .. }
    | Instr::UnOp { dest, .. }
    | Instr::Move { dest, .. }
    | Instr::Load { dest, .. }
    | Instr::Alloc { dest, .. }
    | Instr::AllocArray { dest, .. }
    | Instr::Call {
      dest: Some(dest), ..
    } => *dest = fresh.clone(),
    _ => {}
  }
}
