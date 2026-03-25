use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Result, prelude::*};

use crate::common::symbol_table::SymbolTable;
use crate::front::ast::Ident;
use crate::intermediate::{ir_asm::Instr, ir_context::IRContext};

/// Emit intermediate representation.
pub fn emit_ir(
  filename: String,
  program_ir: HashMap<Ident, IRContext>,
  symbol_table: SymbolTable,
) -> Result<()> {
  let file = File::create(format!("{filename}.abs")).unwrap();
  let mut writer = BufWriter::new(file);

  writeln!(writer, "// C0 Compiler")?;
  writeln!(writer)?;

  for (func_name, func_ir) in program_ir {
    let (ret_typ, params) = symbol_table.get_function_signature(&func_name).unwrap();

    writeln!(writer, "// Function: {func_name}")?;
    writeln!(writer, "// Parameter Types: {:?}", params)?;
    writeln!(writer, "// Return Type: {:?}", ret_typ)?;
    for ir_instr in func_ir.linearize() {
      if !matches!(ir_instr, Instr::Label(_)) {
        write!(writer, "\t")?;
      }
      writeln!(writer, "{}", ir_instr)?;
    }
    writeln!(writer)?;
  }

  Ok(())
}

/// Emit LLVM.
pub fn emit_llvm(filename: String, llvm_str: String) -> Result<()> {
  let file = File::create(format!("{filename}.ll")).unwrap();
  let mut writer = BufWriter::new(file);
  write!(writer, "{llvm_str}")?;
  Ok(())
}
