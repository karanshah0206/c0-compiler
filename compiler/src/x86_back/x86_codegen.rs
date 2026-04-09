use std::collections::{HashMap, HashSet};

use crate::common::symbol_table::SymbolTable;
use crate::front::ast::Typ;
use crate::intermediate::ir_codegen::ProgramIR;
use crate::x86_back::{
  regalloc::Regalloc,
  x86_asm::{
    StackVar,
    Width::{self, *},
    X86Instr, X86Operand, X86Reg,
  },
};

/// An x86-64 program is the assembly instructions of its functions and traps.
pub struct X86Program {
  pub functions: HashMap<String, Vec<X86Instr>>,
  pub traps: Vec<X86Instr>,
}

/// Potential kinds of traps that can be called in the program.
enum TrapKind {
  MemError,
  Abort,
}

/// x86-64 code generation context.
struct X86Context {
  instructions: Vec<X86Instr>,
  used_callee_saved: HashSet<X86Reg>,
  register_allocation: Regalloc,
  stack_allocation: HashMap<X86Operand, StackVar>,
  stack_depth: usize,
  label_prefix: String,
  param_types: Vec<Typ>,
}

impl X86Context {
  fn new(register_allocation: Regalloc, label_prefix: String, param_types: Vec<Typ>) -> Self {
    X86Context {
      instructions: Vec::new(),
      used_callee_saved: HashSet::new(),
      register_allocation,
      stack_allocation: HashMap::new(),
      stack_depth: 0,
      label_prefix,
      param_types,
    }
  }
}

pub fn generate_x86_assembly(
  program: &ProgramIR,
  register_allocation: &Regalloc,
  symbol_table: &SymbolTable,
) -> X86Program {
  todo!("Pending implementation of x86-64 code generator.");
}

/// Get the bit-width for fundamental C0 types.
fn width_for_type(typ: &Typ) -> Width {
  match typ {
    Typ::Int => W32,
    Typ::Bool => W8,
    Typ::Void | Typ::Typedef(_) => {
      unreachable!("Bad width evaluation in x86-64 codegen for type {typ}.")
    }
  }
}
