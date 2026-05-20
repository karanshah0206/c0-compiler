use crate::x86_back::{
  x86_codegen::X86Program,
  x86_optimization::{control_flow_simp::*, strength_reduction::*},
};

/// Apply post-register allocation, x86_64-specific optimizations.
pub fn optimize(program: &mut X86Program, optimizer_level: u8) {
  if optimizer_level > 0 {
    for instrs in program.functions.values_mut() {
      if !instrs.is_empty() {
        loop {
          let mut changed = false;
          changed |= simplify_control_flow(instrs);
          changed |= strength_reduction(instrs);
          if !changed {
            break;
          }
        }
      }
    }
  }
}
