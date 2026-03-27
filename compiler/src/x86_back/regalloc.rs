use std::collections::HashMap;

use crate::common::symbol_table::SymbolTable;
use crate::front::ast::Ident;
use crate::intermediate::ir_context::IRContext;
use crate::x86_back::x86_asm::{Width::*, X86Reg, X86WReg};

/// Colors correspond directly to registers.
/// Colors [1-14] correspond to the 14 registers (RSP and R10 reserved).
/// Color 0 indicates the temp hasn't been colored; color 15 indicates spill.
pub type Color = usize;
pub const UNCOLORED: Color = 0;
pub const SPILL: Color = 15;

/// Get the color corresponding to a register.
pub fn register_to_color(register: X86Reg) -> Color {
  X86Reg::allocatable()
    .iter()
    .position(|reg| reg == &register)
    .expect(&format!(
      "Register {} is not allocatable.",
      X86WReg {
        register,
        width: W64
      }
    ))
}

/// Get the register corresponding to a color.
pub fn color_to_register(color: Color) -> X86Reg {
  X86Reg::allocatable()
    .get(
      color
        .checked_sub(1)
        .expect("Attempted fetching register for an uncolored temporary."),
    )
    .copied()
    .expect(&format!("No register corresponds to color {color}."))
}

/// Maps temporaries in functions of program IR to registers or spills to stack.
/// Spills all temps (i.e., no register allocation) at optiimzer level 0.
pub fn register_allocation(
  program_ir: &HashMap<Ident, IRContext>,
  symbol_table: &SymbolTable,
  optimizer_level: u8,
) -> HashMap<Ident, Vec<Color>> {
  let mut coloring = HashMap::new();

  for (func_name, func_ir) in program_ir {
    if optimizer_level == 0 {
      coloring.insert(
        func_name.to_string(),
        vec![SPILL; func_ir.get_temps_count()],
      );
    } else {
      todo!("Pending implementation for graph coloring register allocation.");
    }
  }

  coloring
}
