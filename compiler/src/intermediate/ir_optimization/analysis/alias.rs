use std::collections::{HashMap, HashSet};

use crate::front::ast::{BinOp, Typ};
use crate::intermediate::{
  ir_asm::{Instr, Operand},
  ir_context::IRContext,
};

/// The canonical address class for a temp that is of a pointer type.
#[derive(Clone, Copy, PartialEq)]
struct AddressClass {
  /// Temp id of the originating `alloc`/`alloc_array` instruction.
  alloc_id: usize,
  /// Byte offset.
  offset: i64,
}

/// Known state of a pointer temporary.
#[derive(Clone, Copy, PartialEq)]
enum PointerTarget {
  /// Pointer to a known, function-local `alloc` at a known byte offset.
  Class(AddressClass),
  /// Pointer to a known offset from an unknown base.
  Relative { base: usize, offset: i64 },
}

/// Function-scoped alias information.
pub struct AliasInfo {
  /// Mapping from pointer temps to their known state.
  targets: HashMap<usize, PointerTarget>,
  /// Set of pointer temps that escape function scope.
  escaping_allocs: HashSet<usize>,
}

impl AliasInfo {
  /// Perform alias analysis on a function's IR.
  pub fn new(ctx: &IRContext) -> Self {
    let mut targets: HashMap<usize, PointerTarget> = HashMap::new();
    loop {
      let mut changed = false;
      for block in ctx.get_blocks().values() {
        for instr in &block.body {
          if let Some((temp_id, target)) = get_pointer_target_from_def(instr, &targets)
            && targets.get(&temp_id) != Some(&target)
          {
            targets.insert(temp_id, target);
            changed = true;
          }
        }
      }
      if !changed {
        break;
      }
    }

    let mut escaping_allocs = HashSet::new();
    for block in ctx.get_blocks().values() {
      for instr in &block.body {
        check_escapes(instr, &targets, &mut escaping_allocs);
      }
      if let Some(terminator) = &block.terminator {
        check_escapes(terminator, &targets, &mut escaping_allocs);
      }
    }

    AliasInfo {
      targets,
      escaping_allocs,
    }
  }

  /// Resolve an operand to its canonical address class, if known.
  pub fn classify_operand(&self, op: &Operand) -> Option<AddressClass> {
    match self.target_of_operand(op)? {
      PointerTarget::Class(address_class) => Some(address_class),
      PointerTarget::Relative { .. } => None,
    }
  }

  /// Resolve an operand to its pointer target.
  pub fn target_of_operand(&self, op: &Operand) -> Option<PointerTarget> {
    if let Operand::Temp((id, _)) = op {
      self.targets.get(id).copied()
    } else {
      None
    }
  }

  /// Resolve the temp id to its underlying alloc id, if known.
  pub fn alloc_of(&self, temp_id: usize) -> Option<usize> {
    match self.targets.get(&temp_id)? {
      PointerTarget::Class(address_class) => Some(address_class.alloc_id),
      PointerTarget::Relative { .. } => None,
    }
  }

  /// Check if an allocation escapes the function scope.
  pub fn escapes(&self, alloc_id: usize) -> bool {
    self.escaping_allocs.contains(&alloc_id)
  }
}

/// Compute the pointer target produced by a defining instruction, if any.
fn get_pointer_target_from_def(
  instr: &Instr,
  targets: &HashMap<usize, PointerTarget>,
) -> Option<(usize, PointerTarget)> {
  match instr {
    Instr::Alloc { dest, .. } | Instr::AllocArray { dest, .. } => Some((
      dest.0,
      PointerTarget::Class(AddressClass {
        alloc_id: dest.0,
        offset: 0,
      }),
    )),
    Instr::BinOp {
      op: BinOp::Add,
      dest,
      lhs,
      rhs,
    } => {
      // pointer arithmetic in this IR is always (pointer + constant) for struct field offsets
      if !matches!(dest.1, Typ::Pointer(_, _)) {
        return None;
      }
      let (base, offset) = match (lhs, rhs) {
        (Operand::Temp(t), Operand::Const((c, _))) => (t, *c),
        (Operand::Const((c, _)), Operand::Temp(t)) => (t, *c),
        _ => return None,
      };
      let new_target = match targets.get(&base.0) {
        Some(PointerTarget::Class(class)) => PointerTarget::Class(AddressClass {
          alloc_id: class.alloc_id,
          offset: class.offset + offset,
        }),
        Some(PointerTarget::Relative {
          base: rel_base,
          offset: rel_offset,
        }) => PointerTarget::Relative {
          base: *rel_base,
          offset: rel_offset + offset,
        },
        // base is an untracked pointer
        None => PointerTarget::Relative {
          base: base.0,
          offset,
        },
      };
      Some((dest.0, new_target))
    }
    Instr::Move {
      dest,
      src: Operand::Temp(src),
    } => targets.get(&src.0).copied().map(|t| (dest.0, t)),
    Instr::Phi { dest, srcs } if !srcs.is_empty() => {
      let mut merged: Option<PointerTarget> = None;
      for (_, op) in srcs {
        let target = match op {
          Operand::Temp((id, _)) => *targets.get(id)?,
          _ => return None,
        };
        match merged {
          Some(seen) if seen != target => return None,
          _ => merged = Some(target),
        }
      }
      merged.map(|t| (dest.0, t))
    }
    _ => None,
  }
}

/// Mark pointer-type temporaries that escape the function scope.
fn check_escapes(
  instr: &Instr,
  targets: &HashMap<usize, PointerTarget>,
  escaping: &mut HashSet<usize>,
) {
  let mut mark = |op: &Operand| {
    if let Operand::Temp((id, _)) = op
      && let Some(alloc_id) = get_alloc_id_of_ptr_temp(targets, *id)
    {
      escaping.insert(alloc_id);
    }
  };

  match instr {
    Instr::Store { src, .. } => mark(src),
    Instr::Move {
      dest,
      src: Operand::Temp(src),
    } => {
      let src_alloc = get_alloc_id_of_ptr_temp(targets, src.0);
      let dest_alloc = get_alloc_id_of_ptr_temp(targets, dest.0);
      if let Some(alloc_id) = src_alloc
        && dest_alloc != Some(alloc_id)
      {
        escaping.insert(alloc_id);
      }
    }
    Instr::Move { .. } => {}
    Instr::Phi { dest, srcs } => {
      let dest_alloc = get_alloc_id_of_ptr_temp(targets, dest.0);
      for (_, op) in srcs {
        if let Operand::Temp((id, _)) = op
          && let Some(alloc_id) = get_alloc_id_of_ptr_temp(targets, *id)
          && dest_alloc != Some(alloc_id)
        {
          escaping.insert(alloc_id);
        }
      }
    }
    Instr::BinOp {
      op: BinOp::Add,
      dest,
      lhs,
      rhs,
    } => {
      let dest_alloc = get_alloc_id_of_ptr_temp(targets, dest.0);
      for op in [lhs, rhs] {
        if let Operand::Temp((id, _)) = op
          && let Some(alloc_id) = get_alloc_id_of_ptr_temp(targets, *id)
          && dest_alloc != Some(alloc_id)
        {
          escaping.insert(alloc_id);
        }
      }
    }
    Instr::BinOp { lhs, rhs, .. } => {
      mark(lhs);
      mark(rhs);
    }
    Instr::UnOp { src, .. } => mark(src),
    Instr::Call { args, .. } | Instr::TailCall { args, .. } => {
      for arg in args {
        mark(arg);
      }
    }
    Instr::Return(Some(op)) => mark(op),
    Instr::JumpIf { pred, .. } => mark(pred),
    Instr::Alloc { size, .. } => mark(size),
    Instr::AllocArray { size, count, .. } => {
      mark(size);
      mark(count);
    }
    Instr::Return(None)
    | Instr::Label(_)
    | Instr::JumpTo(_)
    | Instr::Throw(_)
    | Instr::Load { .. } => {}
  }
}

/// Get the underlying allocation id of a pointer-type temporary.
fn get_alloc_id_of_ptr_temp(
  targets: &HashMap<usize, PointerTarget>,
  temp_id: usize,
) -> Option<usize> {
  match targets.get(&temp_id) {
    Some(PointerTarget::Class(address_class)) => Some(address_class.alloc_id),
    _ => None,
  }
}
