use crate::intermediate::{
  ir_asm::{Instr, Operand},
  ir_context::IRContext,
};

/// Tail-call elimination on explicit call-returns.
pub fn tail_call_elimination(ctx: &mut IRContext) -> bool {
  let mut changed = false;

  for block in ctx.get_blocks_mut().values_mut() {
    let Some(terminator) = block.terminator.clone() else {
      continue;
    };
    if block.body.is_empty() {
      continue;
    }

    let mut new_terminator: Option<(usize, Instr)> = None;

    match terminator {
      Instr::Return(None) => {
        if let Some(Instr::Call {
          dest: None,
          name,
          args,
        }) = block.body.last().cloned()
        {
          new_terminator = Some((1, Instr::TailCall { name, args }));
        }
      }
      Instr::Return(Some(Operand::Temp(ret_temp))) => {
        if let Some(Instr::Call {
          dest: Some(dest),
          name,
          args,
        }) = block.body.last().cloned()
          && dest == ret_temp
        {
          new_terminator = Some((1, Instr::TailCall { name, args }));
        } else if block.body.len() >= 2
          && let (
            Instr::Call {
              dest: Some(call_dest),
              name,
              args,
            },
            Instr::Move {
              dest: moved_dest,
              src: Operand::Temp(moved_src),
            },
          ) = (
            block.body[block.body.len() - 2].clone(),
            block.body[block.body.len() - 1].clone(),
          )
          && moved_dest == ret_temp
          && moved_src == call_dest
        {
          new_terminator = Some((2, Instr::TailCall { name, args }));
        }
      }
      _ => {}
    }

    if let Some((pop_count, tail_call)) = new_terminator {
      for _ in 0..pop_count {
        block.body.pop();
      }
      block.terminator = Some(tail_call);
      changed = true;
    }
  }

  changed
}
