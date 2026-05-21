use std::time::{Duration, Instant};

use crate::intermediate::{
  ir_codegen::ProgramIR,
  ir_context::IRContext,
  ir_optimization::{adce::*, tail_call_elim::*},
};

const SECONDS_LIMIT_AT_O1: u64 = 12;
const SECONDS_LIMIT_AT_O2: u64 = u64::MAX; // unbounded at -O2

pub fn optimize(program: &mut ProgramIR, optimizer_level: u8, is_unsafe: bool) {
  if optimizer_level == 0 {
    return;
  }

  for ir_context in program.values_mut() {
    let start = Instant::now();
    let timeout = Duration::from_secs(if optimizer_level == 1 {
      SECONDS_LIMIT_AT_O1
    } else {
      SECONDS_LIMIT_AT_O2
    });
    while start.elapsed() < timeout {
      if !optimize_ir(ir_context, is_unsafe) {
        break;
      }
    }
  }
}

fn optimize_ir(ir_context: &mut IRContext, is_unsafe: bool) -> bool {
  let mut changed = false;
  changed |= adce(ir_context, is_unsafe);
  changed |= tail_call_elimination(ir_context);
  changed
}
