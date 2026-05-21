use crate::{
  front::ast::{BinOp, Typ},
  intermediate::{ir_asm::Instr, ir_context::IRContext, ir_optimization::analysis::cfg_helpers::*},
};

/// Aggressive dead-code elimination.
pub fn adce(ctx: &mut IRContext, is_unsafe: bool) -> bool {
  let mut changed = false;

  loop {
    let uses = compute_uses_of_all_temps(ctx);
    let mut did_change = false;

    for block in ctx.get_blocks_mut().values_mut() {
      let mut new_body = Vec::with_capacity(block.body.len());
      for instr in block.body.drain(..) {
        if let Some(dest) = get_dest_temp_from_instruction(&instr)
          && uses.get(&dest.0).copied().unwrap_or(0) == 0
          && is_instruction_eliminable(&instr, is_unsafe)
        {
          did_change = true;
          changed = true;
          continue;
        }
        new_body.push(instr);
      }
      block.body = new_body;
    }

    if !did_change {
      break;
    }
  }

  changed
}

/// Check that if an instruction's result is unused, can it be eliminated?
fn is_instruction_eliminable(instr: &Instr, is_unsafe: bool) -> bool {
  match instr {
    Instr::Move { .. } | Instr::UnOp { .. } | Instr::Phi { .. } => true,
    Instr::BinOp { op, dest, .. } => {
      is_unsafe
        || !(matches!(*op, BinOp::Div | BinOp::Mod | BinOp::Sal | BinOp::Sar)
          || matches!(&dest.1, Typ::Pointer(_, _)))
    }
    Instr::Load { .. } => is_unsafe,
    Instr::Label(_)
    | Instr::JumpTo(_)
    | Instr::JumpIf { .. }
    | Instr::Call { .. }
    | Instr::TailCall { .. }
    | Instr::Return(_)
    | Instr::Throw(_)
    | Instr::Store { .. }
    | Instr::Alloc { .. }
    | Instr::AllocArray { .. } => false,
  }
}
