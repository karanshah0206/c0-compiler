use std::fs::File;
use std::io::{BufWriter, Result, prelude::*};

use crate::common::symbol_table::SymbolTable;
use crate::intermediate::{ir_asm::Instr, ir_codegen::ProgramIR};
use crate::x86_back::x86_codegen::X86Program;

/// Emit intermediate representation.
pub fn emit_ir(filename: String, program_ir: ProgramIR, symbol_table: SymbolTable) -> Result<()> {
  let file = File::create(format!("{filename}.abs")).unwrap();
  let mut writer = BufWriter::new(file);

  writeln!(writer, "// C0 Compiler")?;
  writeln!(writer, "// {filename:?}")?;

  for (func_name, func_ir) in program_ir {
    let (ret_typ, params) = symbol_table.get_function_signature(&func_name).unwrap();

    writeln!(writer)?;
    writeln!(writer, "// Function: {func_name}")?;
    writeln!(writer, "// Parameter Types: {:?}", params)?;
    writeln!(writer, "// Return Type: {:?}", ret_typ)?;
    for ir_instr in func_ir.linearize() {
      if !matches!(ir_instr, Instr::Label(_)) {
        write!(writer, "\t")?;
      }
      writeln!(writer, "{}", ir_instr)?;
    }
  }

  Ok(())
}

/// Emit System V x86-64 assembly.
pub fn emit_x86(filename: String, x86_program: X86Program) -> Result<()> {
  let file = File::create(format!("{filename}.s")).unwrap();
  let mut writer = BufWriter::new(file);

  writeln!(writer, ".ident\t\"C0 Compiler\"")?;
  writeln!(writer, ".file\t{filename:?}")?;

  for (function_name, instructions) in x86_program.functions {
    writeln!(writer, "\n.globl\t{function_name}")?;
    writeln!(writer, "{function_name}:")?;
    for instruction in instructions {
      writeln!(writer, "{instruction}")?;
    }
  }

  if !x86_program.traps.is_empty() {
    writeln!(writer)?;
    for instruction in x86_program.traps {
      writeln!(writer, "{instruction}")?;
    }
  }

  Ok(())
}

/// Emit LLVM.
pub fn emit_llvm(filename: String, llvm_str: String) -> Result<()> {
  let file = File::create(format!("{filename}.ll")).unwrap();
  let mut writer = BufWriter::new(file);
  writeln!(writer, "; C0 Compiler")?;
  writeln!(writer, "; {filename:?}")?;
  writeln!(writer)?;
  write!(writer, "{llvm_str}")?;
  Ok(())
}
