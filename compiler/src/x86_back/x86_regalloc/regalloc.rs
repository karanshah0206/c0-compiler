use std::{
  cmp::Reverse,
  collections::{HashMap, HashSet},
};

use crate::common::symbol_table::SymbolTable;
use crate::front::ast::Ident;
use crate::intermediate::{ir_codegen::ProgramIR, ir_context::IRContext};
use crate::x86_back::{
  x86_asm::{StackVar, Width::*, X86Reg, X86WReg},
  x86_regalloc::*,
};

/// Per-function register coloring (indexed by temp id).
pub type Regalloc = HashMap<Ident, Vec<Color>>;

/// Colors correspond directly to registers.
/// Colors `[1..=K]` correspond to the K allocatable registers.
pub type Color = usize;
pub const UNCOLORED: Color = 0;
pub const SPILL: Color = 15;

/// Width in bytes of a stack slot allocated to a temporary.
const STACK_SLOT_WIDTH: usize = 8;

/// Maps temporaries in functions of program IR to registers or spills to stack.
/// At optimizer level 0, spills all temps.
pub fn register_allocation(
  program_ir: &mut ProgramIR,
  symbol_table: &SymbolTable,
  optimizer_level: u8,
) -> Regalloc {
  let mut coloring = HashMap::new();

  let mut func_names: Vec<Ident> = program_ir.keys().cloned().collect();
  func_names.sort();

  for func_name in func_names {
    let func_ir = program_ir
      .get_mut(&func_name)
      .unwrap_or_else(|| panic!("Unknown function {func_name} found in x86 register allocation."));

    func_ir.deconstruct_ssa();

    // Spill everything at -O0.
    if optimizer_level == 0 {
      coloring.insert(func_name.clone(), vec![SPILL; func_ir.get_temps_count()]);
      continue;
    }

    let params_count = symbol_table
      .get_function_context(&func_name)
      .get_params()
      .len();

    let func_coloring = allocator::allocate_function(func_ir, params_count);
    coloring.insert(func_name.clone(), func_coloring);
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
    .unwrap_or_else(|| unreachable!("No register corresponds to color {color}."))
}

/// Stack slot allocation for spilt temporaries.
pub fn allocate_spill_slots(
  ctx: &IRContext,
  coloring: &[Color],
  params_count: usize,
) -> (HashMap<usize, StackVar>, usize) {
  use interference_graph::Node;

  let liveness = liveness_analysis::analyze_liveness(ctx);
  let graph = interference_graph::build(ctx, &liveness, params_count);
  let arg_regs = X86Reg::call_argument();

  let mut spill_temps: Vec<usize> = coloring
    .iter()
    .enumerate()
    .filter_map(|(temp_id, &color)| {
      if color == SPILL && (temp_id < arg_regs.len() || temp_id >= params_count) {
        Some(temp_id)
      } else {
        None
      }
    })
    .collect();

  spill_temps.sort_by_key(|temp_id| {
    Reverse(
      graph
        .degree
        .get(&Node::Temp(*temp_id))
        .copied()
        .unwrap_or(0),
    )
  });

  let mut assigned_slots: HashMap<usize, usize> = HashMap::new();
  for temp_id in spill_temps {
    let mut used_slots: HashSet<usize> = HashSet::new();
    if let Some(neighbors) = graph.adj.get(&Node::Temp(temp_id)) {
      for neighbor in neighbors {
        if let Node::Temp(neighbor_id) = neighbor
          && coloring.get(*neighbor_id) == Some(&SPILL)
          && (*neighbor_id < arg_regs.len() || *neighbor_id >= params_count)
          && let Some(slot) = assigned_slots.get(neighbor_id)
        {
          used_slots.insert(*slot);
        }
      }
    }

    let mut slot = 0;
    while used_slots.contains(&slot) {
      slot += 1;
    }
    assigned_slots.insert(temp_id, slot);
  }

  let spill_slot_count = assigned_slots
    .values()
    .copied()
    .max()
    .map(|slot| slot + 1)
    .unwrap_or(0);

  let stack_allocation = assigned_slots
    .into_iter()
    .map(|(temp_id, slot)| {
      (
        temp_id,
        StackVar {
          offset: (slot * STACK_SLOT_WIDTH) as i64,
          width: W64,
        },
      )
    })
    .collect();

  (stack_allocation, spill_slot_count)
}
