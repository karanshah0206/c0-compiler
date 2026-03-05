use std::collections::HashSet;

use crate::common::symbol_table::SymbolTable;
use crate::front::ast::*;

/// Perform semantic analysis on the program.
pub fn analyze_program(header_ast: &mut Program, source_ast: &mut Program) -> SymbolTable {
  use GlobalDeclaration::*;

  let mut symbol_table = SymbolTable::new();

  for declaration in header_ast {
    match declaration {
      Typedef(typ, id) => symbol_table.add_typedef(id, typ),
      FDecl(typ, id, params) => symbol_table.declare_function(id, typ, params, true),
      FDefn(_, _, _, _) => panic!("Function definitions are illegal in a header file."),
    }
  }

  let mut functions_to_define: HashSet<Ident> = ["main".to_string()].into_iter().collect();

  for declaration in source_ast {
    match declaration {
      Typedef(typ, id) => symbol_table.add_typedef(id, typ),
      FDecl(typ, id, params) => symbol_table.declare_function(id, typ, params, false),
      FDefn(typ, id, params, ast) => {
        symbol_table.define_function(id, typ, params);
        match analyze_function(ast, typ, params, &mut symbol_table) {
          Ok(functions_called) => functions_to_define.extend(functions_called),
          Err(error) => panic!("{error}"),
        };
      }
    }
  }

  for function in functions_to_define {
    assert!(
      symbol_table.is_defined(&function),
      "Missing definition for function {function}."
    );
  }

  symbol_table
}

/// Perform semantic analysis on a function's AST.
fn analyze_function(
  ast: &mut Stmt,
  typ: &Typ,
  params: &[Param],
  symbol_table: &mut SymbolTable,
) -> Result<HashSet<String>, String> {
  todo!();
}
