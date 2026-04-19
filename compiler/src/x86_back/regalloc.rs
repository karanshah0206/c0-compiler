use std::collections::HashMap;

use crate::common::symbol_table::SymbolTable;
use crate::front::ast::Ident;
use crate::intermediate::{ir_codegen::ProgramIR, ir_context::IRContext};
use crate::x86_back::x86_asm::{Width::*, X86Reg, X86WReg};

pub type Regalloc = HashMap<Ident, Vec<Color>>;

/// Colors correspond directly to registers.
/// Colors [1-14] correspond to the 14 registers (RSP and R10 reserved).
/// Color 0 indicates the temp hasn't been colored; color 15 indicates spill.
pub type Color = usize;
pub const UNCOLORED: Color = 0;
pub const SPILL: Color = 15;

/// Maps temporaries in functions of program IR to registers or spills to stack.
/// Spills all temps (i.e., no register allocation) at optiimzer level 0.
pub fn register_allocation(
  program_ir: &mut ProgramIR,
  symbol_table: &SymbolTable,
  optimizer_level: u8,
) -> Regalloc {
  let mut coloring = HashMap::new();

  for (func_name, func_ir) in program_ir {
    if optimizer_level == 0 {
      func_ir.deconstruct_ssa();

      let params_count = symbol_table
        .get_function_signature(func_name)
        .unwrap_or_else(|| panic!("Unknown function {func_name} found in x86 regalloc."))
        .1
        .len();

      coloring.insert(
        func_name.to_string(),
        spill_all_temps(func_ir, params_count),
      );
    } else {
      todo!("Pending implementation for graph coloring register allocation.");
    }
  }

  coloring
}

/// At optimizer level 0, precolor function arguments and spill all other temporaries.
fn spill_all_temps(func_ir: &IRContext, params_count: usize) -> Vec<usize> {
  let mut coloring = vec![SPILL; func_ir.get_temps_count()];
  let arg_registers = X86Reg::call_argument();
  for temp_id in 0..params_count.min(arg_registers.len()) {
    coloring[temp_id] = register_to_color(arg_registers[temp_id]);
  }
  coloring
}

/// Get the color corresponding to a register.
pub fn register_to_color(register: X86Reg) -> Color {
  X86Reg::allocatable()
    .iter()
    .position(|reg| reg == &register)
    .unwrap_or_else(|| {
      panic!(
        "Register {} is not allocatable.",
        X86WReg {
          register,
          width: W64
        }
      )
    })
    + 1
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
    .unwrap_or_else(|| panic!("No register corresponds to color {color}."))
}
