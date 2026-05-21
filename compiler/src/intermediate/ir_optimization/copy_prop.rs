use std::collections::{HashMap, HashSet};

use crate::intermediate::{
  ir_asm::{Instr, Operand},
  ir_context::IRContext,
};

/// Copy propagation.
pub fn copy_propagation(ctx: &mut IRContext) -> bool {
  let mut copy_map: HashMap<usize, Operand> = HashMap::new();
  let blocks = ctx.get_blocks_mut();

  for block in blocks.values() {
    for instr in &block.body {
      if let Some((src, dest)) = match instr {
        Instr::Move { dest, src } => Some((src.clone(), dest.clone())),
        _ => None,
      } && !matches!(&src, Operand::Temp(temp) if temp == &dest)
      {
        copy_map.insert(dest.0, src);
      }
    }
  }

  if copy_map.is_empty() {
    return false;
  }

  let mut changed = false;
  for block in blocks.values_mut() {
    for instr in &mut block.body {
      changed |= propagate_on_instr(instr, &copy_map);
    }
    if let Some(terminator) = &mut block.terminator {
      changed |= propagate_on_terminator(terminator, &copy_map);
    }
  }
  changed
}

/// Constant propagation on an instruction in the basic block's body.
fn propagate_on_instr(instr: &mut Instr, copies: &HashMap<usize, Operand>) -> bool {
  match instr {
    Instr::BinOp { lhs, rhs, .. } => {
      let mut changed = false;
      changed |= copy_operand(lhs, copies);
      changed |= copy_operand(rhs, copies);
      changed
    }
    Instr::UnOp { src, .. } => copy_operand(src, copies),
    Instr::Call { args, .. } | Instr::TailCall { args, .. } => {
      let mut changed = false;
      for arg in args {
        changed |= copy_operand(arg, copies);
      }
      changed
    }
    Instr::Return(Some(op)) => copy_operand(op, copies),
    Instr::Phi { srcs, .. } => {
      let mut changed = false;
      for (_, src) in srcs {
        changed |= copy_operand(src, copies);
      }
      changed
    }
    Instr::Move { src, .. } => copy_operand(src, copies),
    Instr::Load { addr, .. } => copy_operand(addr, copies),
    Instr::Store { addr, src } => {
      let mut changed = false;
      changed |= copy_operand(addr, copies);
      changed |= copy_operand(src, copies);
      changed
    }
    Instr::Alloc { size, .. } => copy_operand(size, copies),
    Instr::AllocArray { size, count, .. } => {
      let mut changed = false;
      changed |= copy_operand(size, copies);
      changed |= copy_operand(count, copies);
      changed
    }
    Instr::Label(_)
    | Instr::JumpTo(_)
    | Instr::JumpIf { .. }
    | Instr::Return(None)
    | Instr::Throw(_) => false,
  }
}

/// Constant propagation on a basic block's terminator instruction.
fn propagate_on_terminator(terminator: &mut Instr, copies: &HashMap<usize, Operand>) -> bool {
  if let Instr::JumpIf { pred, .. } = terminator {
    copy_operand(pred, copies)
  } else {
    false
  }
}

/// Try replacing an operand with its source alias.
fn copy_operand(operand: &mut Operand, copies: &HashMap<usize, Operand>) -> bool {
  let resolved = resolve_copy_operand(operand.clone(), copies);
  if operand.clone() == resolved {
    false
  } else {
    *operand = resolved;
    true
  }
}

/// Get source alias of an operand.
fn resolve_copy_operand(operand: Operand, copies: &HashMap<usize, Operand>) -> Operand {
  let mut current = operand;
  let mut seen: HashSet<usize> = HashSet::new();

  loop {
    let Operand::Temp((id, _)) = &current else {
      return current;
    };

    if !seen.insert(*id) {
      return current;
    }

    let Some(next) = copies.get(id) else {
      return current;
    };

    current = next.clone();
  }
}
