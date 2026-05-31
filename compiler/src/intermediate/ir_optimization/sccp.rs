use std::collections::{HashMap, HashSet};

use crate::front::ast::{BinOp, Typ, UnOp};
use crate::intermediate::{
  ir_asm::{Instr, Label, Operand},
  ir_context::{BasicBlock, IRContext},
  ir_optimization::{
    analysis::{cfg_helpers::*, lattice::*},
    cfg_simp::*,
  },
};

/// Sparse conditional constant propagation and folding.
pub fn sccp_and_fold(ctx: &mut IRContext, is_unsafe: bool) -> bool {
  let blocks = ctx.get_blocks_mut();
  if !blocks.contains_key(&Label(0)) {
    return false;
  }

  let mut state: HashSet<Label> = HashSet::from([Label(0)]);
  let mut lattice = Lattice::new();
  overdefine_live_in_temps(&mut lattice, blocks);

  loop {
    let mut changed = false;
    let init_state: Vec<Label> = state.iter().copied().collect();

    for label in init_state {
      let Some(block) = blocks.get(&label) else {
        continue;
      };

      for instr in &block.body {
        if let Some(dest) = get_dest_temp_from_instruction(instr) {
          let new_val = propagate_on_instr(instr, &lattice, &state, is_unsafe);
          changed |= update_lattice(&mut lattice, dest.0, new_val);
        }
      }

      if let Some(terminator) = &block.terminator {
        changed |= propagate_on_terminator(terminator, &lattice, &mut state);
      }
    }

    if !changed {
      break;
    }
  }

  let mut changed = false;

  let before = blocks.len();
  blocks.retain(|label, _| state.contains(label));
  changed |= before != blocks.len();

  let existing_labels: HashSet<Label> = blocks.keys().copied().collect();
  for block in blocks.values_mut() {
    block.body.retain_mut(|instr| {
      if let Instr::Phi { srcs, .. } = instr {
        srcs.retain(|(pred, _)| existing_labels.contains(pred));
      }
      true
    });

    for instr in &mut block.body {
      changed |= replace_instr_consts(instr, &lattice);
      changed |= fold_instr_from_lattice(instr, &lattice);
      changed |= simplify_trivial_phi(instr);
    }

    if let Some(terminator) = &mut block.terminator {
      changed |= replace_terminator_consts(terminator, &lattice);
      changed |= simplify_terminator(terminator);
    }
  }

  changed |= cfg_simplification(ctx);
  changed
}

/// Seed temporaries without in-function definitions (i.e., function parameters/live-ins).
fn overdefine_live_in_temps(lattice: &mut Lattice, blocks: &HashMap<Label, BasicBlock>) {
  let mut defined_temp_ids = HashSet::new();
  let mut used_temp_ids = HashSet::new();

  for block in blocks.values() {
    for instr in &block.body {
      if let Some(dest) = get_dest_temp_from_instruction(instr) {
        defined_temp_ids.insert(dest.0);
      }
      collect_used_temp_ids(instr, &mut used_temp_ids);
    }

    if let Some(terminator) = &block.terminator {
      collect_used_temp_ids(terminator, &mut used_temp_ids);
    }
  }

  for temp_id in used_temp_ids {
    if !defined_temp_ids.contains(&temp_id) {
      lattice.insert(temp_id, LatticeValue::Overdefined);
    }
  }
}

/// Accumulate used temporaries from an instruction.
fn collect_used_temp_ids(instr: &Instr, used_temp_ids: &mut HashSet<usize>) {
  let mut insert_if_temp = |op: &Operand| {
    if let Operand::Temp((temp_id, _)) = op {
      used_temp_ids.insert(*temp_id);
    }
  };

  match instr {
    Instr::BinOp { lhs, rhs, .. } => {
      insert_if_temp(lhs);
      insert_if_temp(rhs);
    }
    Instr::UnOp { src, .. } => insert_if_temp(src),
    Instr::JumpIf { pred, .. } => insert_if_temp(pred),
    Instr::Call { args, .. } | Instr::TailCall { args, .. } => {
      for arg in args {
        insert_if_temp(arg);
      }
    }
    Instr::Return(Some(op)) => insert_if_temp(op),
    Instr::Phi { srcs, .. } => {
      for (_, op) in srcs {
        insert_if_temp(op);
      }
    }
    Instr::Move { src, .. } => insert_if_temp(src),
    Instr::Load { addr, .. } => insert_if_temp(addr),
    Instr::Store { addr, src } => {
      insert_if_temp(addr);
      insert_if_temp(src);
    }
    Instr::Alloc { size, .. } => insert_if_temp(size),
    Instr::AllocArray { size, count, .. } => {
      insert_if_temp(size);
      insert_if_temp(count);
    }
    Instr::Label(_) | Instr::JumpTo(_) | Instr::Return(None) | Instr::Throw(_) => {}
  }
}

/// Constant propagate on an instruction and return the resultant lattice value.
fn propagate_on_instr(
  instr: &Instr,
  lattice: &Lattice,
  state: &HashSet<Label>,
  is_unsafe: bool,
) -> LatticeValue {
  match instr {
    Instr::Move { src, .. } => get_lattice_value_of_operand(src, lattice),
    Instr::UnOp { op, src, .. } => match get_lattice_value_of_operand(src, lattice) {
      LatticeValue::Const((value, typ)) => LatticeValue::Const((
        propagate_on_unop(*op, value),
        if matches!(*op, UnOp::LNot) {
          Typ::Bool
        } else {
          typ.clone()
        },
      )),
      value => value,
    },
    Instr::BinOp { op, dest, lhs, rhs } => {
      if matches!(&dest.1, Typ::Pointer(_, _) | Typ::Null) {
        return LatticeValue::Overdefined;
      }

      let l_value = get_lattice_value_of_operand(lhs, lattice);
      let r_value = get_lattice_value_of_operand(rhs, lattice);

      match (&l_value, &r_value) {
        (LatticeValue::Const((l, _)), LatticeValue::Const((r, _))) => {
          if let Some(value) = propagate_on_binop(*op, *l, *r, is_unsafe) {
            LatticeValue::Const((value as i64, dest.1.clone()))
          } else {
            LatticeValue::Overdefined
          }
        }
        (LatticeValue::Overdefined, _) | (_, LatticeValue::Overdefined) => {
          LatticeValue::Overdefined
        }
        _ => LatticeValue::Undefined,
      }
    }
    Instr::Phi { srcs, .. } => {
      let mut merged = LatticeValue::Undefined;
      for (pred, src) in srcs {
        if state.contains(pred) {
          merged = join_lattice(merged, get_lattice_value_of_operand(src, lattice));
          if matches!(merged, LatticeValue::Overdefined) {
            // cannot propagate because we can't reason about predecessor path merges
            break;
          }
        }
      }
      merged
    }
    Instr::Call { .. }
    | Instr::TailCall { .. }
    | Instr::Load { .. }
    | Instr::Alloc { .. }
    | Instr::AllocArray { .. }
    | Instr::Label(_)
    | Instr::JumpTo(_)
    | Instr::JumpIf { .. }
    | Instr::Return(_)
    | Instr::Store { .. }
    | Instr::Throw(_) => LatticeValue::Overdefined,
  }
}

/// Constant propagation on a unary operator for known compile-time constants.
fn propagate_on_unop(op: UnOp, value: i64) -> i64 {
  match op {
    UnOp::Neg => value.wrapping_neg(),
    UnOp::Not => !value,
    UnOp::LNot => {
      if value == 0 {
        1
      } else {
        0
      }
    }
  }
}

/// Try constant propagation on a unary operator for known compile-time constants.
fn propagate_on_binop(op: BinOp, lhs: i64, rhs: i64, is_unsafe: bool) -> Option<i32> {
  let lhs = lhs as i32;
  let rhs = rhs as i32;

  Some(match op {
    BinOp::Add => lhs.wrapping_add(rhs),
    BinOp::Sub => lhs.wrapping_sub(rhs),
    BinOp::Mul => lhs.wrapping_mul(rhs),
    BinOp::Div => {
      if rhs == 0 || (lhs == i32::MIN && rhs == -1) {
        if is_unsafe {
          if rhs == 0 { 0 } else { lhs }
        } else {
          return None;
        }
      } else {
        lhs.wrapping_div(rhs)
      }
    }
    BinOp::Mod => {
      if rhs == 0 || (lhs == i32::MIN && rhs == -1) {
        if is_unsafe {
          0
        } else {
          return None;
        }
      } else {
        lhs.wrapping_rem(rhs)
      }
    }
    BinOp::And => lhs & rhs,
    BinOp::Xor => lhs ^ rhs,
    BinOp::Or => lhs | rhs,
    BinOp::Sal => {
      if (0..=31).contains(&rhs) {
        lhs.wrapping_shl(rhs as u32)
      } else if is_unsafe {
        0
      } else {
        return None;
      }
    }
    BinOp::Sar => {
      if (0..=31).contains(&rhs) {
        lhs.wrapping_shr(rhs as u32)
      } else if is_unsafe {
        0
      } else {
        return None;
      }
    }
    BinOp::LAnd => (lhs != 0 && rhs != 0) as i32,
    BinOp::LOr => (lhs != 0 || rhs != 0) as i32,
    BinOp::CmpEq => (lhs == rhs) as i32,
    BinOp::CmpNeq => (lhs != rhs) as i32,
    BinOp::Lt => (lhs < rhs) as i32,
    BinOp::Gt => (lhs > rhs) as i32,
    BinOp::Lte => (lhs <= rhs) as i32,
    BinOp::Gte => (lhs >= rhs) as i32,
  })
}

/// Try unrolling terminator branches with propagated/folded values.
fn propagate_on_terminator(
  terminator: &Instr,
  lattice: &Lattice,
  state: &mut HashSet<Label>,
) -> bool {
  match terminator {
    Instr::JumpTo(label) => state.insert(*label),
    Instr::JumpIf { pred, holds, fails } => match get_lattice_value_of_operand(pred, lattice) {
      LatticeValue::Const((value, _)) => {
        if value != 0 {
          state.insert(*holds)
        } else {
          state.insert(*fails)
        }
      }
      _ => state.insert(*holds) | state.insert(*fails),
    },
    _ => false,
  }
}

/// Try constant folding on an instruction using data-flow analysis.
fn fold_instr_from_lattice(instr: &mut Instr, lattice: &Lattice) -> bool {
  let Some(dest) = get_dest_temp_from_instruction(instr) else {
    return false;
  };
  let Some(LatticeValue::Const(constant)) = lattice.get(&dest.0) else {
    return false;
  };
  if matches!(instr, Instr::Move { dest: move_dest, src: Operand::Const(src_const) } if move_dest == &dest && src_const == constant)
  {
    return false;
  }

  *instr = Instr::Move {
    dest,
    src: Operand::Const(constant.clone()),
  };
  true
}

/// Try replacing an instruction operands with known constants from lattice.
fn replace_instr_consts(instr: &mut Instr, lattice: &Lattice) -> bool {
  match instr {
    Instr::BinOp { lhs, rhs, .. } => {
      replace_const_operand(lhs, lattice) | replace_const_operand(rhs, lattice)
    }
    Instr::UnOp { src, .. } => replace_const_operand(src, lattice),
    Instr::Call { args, .. } | Instr::TailCall { args, .. } => args
      .iter_mut()
      .any(|arg| replace_const_operand(arg, lattice)),
    Instr::Return(Some(op)) => replace_const_operand(op, lattice),
    Instr::Phi { srcs, .. } => srcs
      .iter_mut()
      .any(|(_, op)| replace_const_operand(op, lattice)),
    Instr::Move { src, .. } => replace_const_operand(src, lattice),
    Instr::Load { addr, .. } => replace_const_operand(addr, lattice),
    Instr::Store { addr, src } => {
      replace_const_operand(addr, lattice) | replace_const_operand(src, lattice)
    }
    Instr::Alloc { size, .. } => replace_const_operand(size, lattice),
    Instr::AllocArray { size, count, .. } => {
      replace_const_operand(size, lattice) | replace_const_operand(count, lattice)
    }
    Instr::Label(_)
    | Instr::JumpTo(_)
    | Instr::JumpIf { .. }
    | Instr::Return(None)
    | Instr::Throw(_) => false,
  }
}

/// Try replacing terminator instruction operands with known constants from lattice.
fn replace_terminator_consts(terminator: &mut Instr, lattice: &Lattice) -> bool {
  if let Instr::JumpIf { pred, .. } = terminator {
    replace_const_operand(pred, lattice)
  } else {
    false
  }
}

/// Helper to try replacing operands with known constants from lattice.
fn replace_const_operand(op: &mut Operand, lattice: &Lattice) -> bool {
  let Operand::Temp((temp_id, _)) = op else {
    return false;
  };
  let Some(LatticeValue::Const((value, typ))) = lattice.get(temp_id) else {
    return false;
  };
  *op = Operand::Const((*value, typ.clone()));
  true
}
