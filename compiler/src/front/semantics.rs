use std::collections::HashSet;

use crate::common::{function_context::FunctionContext, symbol_table::SymbolTable};
use crate::front::ast::*;

/// Perform semantic analysis on the program.
pub fn analyze_program(header_ast: &mut Program, source_ast: &mut Program) -> SymbolTable {
  use GlobalDeclaration::*;

  let mut symbol_table = SymbolTable::new();

  // analyze declarations in header
  for declaration in header_ast {
    match declaration {
      Typedef(typ, id) => symbol_table.add_typedef(id, typ),
      FDecl(typ, id, params) => symbol_table.declare_function(id, typ, params, true),
      FDefn(_, _, _, _) => panic!("Function definitions are illegal in a header file."),
    }
  }

  // functions called in source code must be defined (even if those function calls are unreachable)
  // main is the entry point, it must always be defined
  let mut functions_to_define: HashSet<Ident> = ["main".to_string()].into_iter().collect();
  symbol_table.declare_function(&"main".to_string(), &mut Typ::Int, &mut vec![], false);

  // analyze source program
  for declaration in source_ast {
    match declaration {
      Typedef(typ, id) => symbol_table.add_typedef(id, typ),
      FDecl(typ, id, params) => symbol_table.declare_function(id, typ, params, false),
      FDefn(typ, id, params, ast) => {
        symbol_table.define_function(id, typ, params);
        functions_to_define.extend(analyze_function(id, ast, typ, &mut symbol_table));
      }
    }
  }

  // ensure that all functions that are called are defined
  for function in functions_to_define {
    assert!(
      symbol_table.is_defined(&function),
      "Missing definition for function {function}."
    );
  }

  symbol_table
}

/// Result of typechecking an expression or statement.
struct TcResult {
  /// Type that the statement returns (if any).
  returns: Option<Typ>,
  /// Variables defined in this expression/statement.
  defines: HashSet<Ident>,
}

impl TcResult {
  /// No returns, no defines in statement/expression.
  fn ok() -> Self {
    TcResult {
      returns: None,
      defines: HashSet::new(),
    }
  }

  /// Statement/expression defines variables, no return.
  fn ok_def(defines: HashSet<Ident>) -> Self {
    TcResult {
      returns: None,
      defines,
    }
  }

  /// Statement returns.
  fn ok_ret(typ: Typ, defines: HashSet<Ident>) -> Self {
    TcResult {
      returns: Some(typ),
      defines,
    }
  }
}

/// Perform semantic analysis on a function's AST.
fn analyze_function(id: &Ident, ast: &mut Stmt, typ: &Typ, st: &mut SymbolTable) -> HashSet<Ident> {
  assert!(
    &analyze_stmt(id, ast, st).returns.unwrap_or(Typ::Void) == typ,
    "{id} must always return {typ}."
  );
  st.get_function_context(id).get_function_calls()
}

/// Perform semantic analysis on a statement in the source code.
fn analyze_stmt(id: &Ident, stmt: &mut Stmt, st: &mut SymbolTable) -> TcResult {
  match stmt {
    Stmt::Decl(var) => {
      // variable declaration without initialization

      assert!(
        !st.is_typedef(&var.1),
        "Cannot use variable identifier {} because it is a type definition.",
        var.1
      );

      var.0 = st.resolve_type(var.0.clone());

      st.get_function_context(id).declare_var(var.clone());

      TcResult::ok()
    }
    Stmt::Defn(var, expr) => {
      // variable declaration with initialization

      assert!(
        !st.is_typedef(&var.1),
        "Cannot use variable identifier {} because it is a type definition.",
        var.1
      );

      var.0 = st.resolve_type(var.0.clone());

      st.get_function_context(id).declare_var(var.clone());

      analyze_expr(id, expr, st);
      assert!(
        var.0 == expr.get_type(),
        "Mismatching types in defining variable {}.",
        var.1
      );

      st.get_function_context(id).define_var(var.clone());

      TcResult::ok_def(HashSet::from_iter(vec![var.1.to_string()]))
    }
    Stmt::Asgn(var_id, asn_op, expr) => {
      // assignment to declared variable

      let var_typ = st.get_function_context(id).get_var_type(var_id);

      analyze_expr(id, expr, st);
      assert!(
        expr.get_type() == var_typ,
        "Mismatching types in assignment to variable {var_id}."
      );

      // typecheck elaboration into binary operation
      if let Some(binop) = asn_op.to_binop() {
        let mut binop_expr = Expr::Binop(
          Box::new(Expr::Variable(var_id.to_string(), None)),
          binop,
          Box::new(expr.clone()),
          None,
        );

        analyze_expr(id, &mut binop_expr, st);
        assert!(
          binop_expr.get_type() == var_typ,
          "Mismatching types in assignment to variable {var_id}",
        );
      }

      st.get_function_context(id)
        .define_var((var_typ, var_id.to_string()));

      TcResult::ok_def(HashSet::from_iter(vec![var_id.to_string()]))
    }
    Stmt::PostOp(var_id, _) => {
      // post-operation (++/--) statement

      let var_typ = st.get_function_context(id).get_var_type(var_id);
      assert!(
        var_typ == Typ::Int,
        "Post-operations can only be performed on int, but {var_id} is of type {var_typ}."
      );

      assert!(
        st.get_function_context(id).is_var_defined(var_id),
        "Cannot perform post-op on undefined variable {var_id}."
      );

      st.get_function_context(id)
        .define_var((var_typ, var_id.to_string()));

      TcResult::ok_def(HashSet::from_iter(vec![var_id.to_string()]))
    }
    Stmt::Seq(stmt_1, stmt_2) => {
      // A couple statements in sequence

      let res_1 = analyze_stmt(id, stmt_1, st);
      if res_1.returns.is_some() {
        return res_1;
      }

      let mut res_2 = analyze_stmt(id, stmt_2, st);
      res_2.defines.extend(res_1.defines.iter().cloned());
      res_2
    }
    Stmt::Cond(expr, stmt, stmt1) => todo!(),
    Stmt::While(expr, stmt) => todo!(),
    Stmt::For(stmt, expr, stmt1, stmt2) => todo!(),
    Stmt::Block(stmts) => {
      let mut block_res = TcResult::ok();

      if stmts.is_empty() {
        return block_res;
      }

      st.get_function_context(id).scope_context.enter_scope();

      for stmt in stmts {
        let res = analyze_stmt(id, stmt, st);

        if block_res.returns.is_none() {
          block_res.returns = res.returns;
        }

        if block_res.returns.is_some() {
          st.get_function_context(id).define_all_vars();
        } else {
          block_res.defines.extend(res.defines);
        }
      }

      st.get_function_context(id).scope_context.exit_scope();

      block_res
    }
    Stmt::Ret(expr) => {
      // return statement

      let (typ, _) = st
        .get_function_signature(id)
        .expect(&format!("Unknown function {id}."));
      let typ = typ.clone();

      match expr {
        Some(expr) => {
          analyze_expr(id, expr, st);
          let expr_typ = expr.get_type();
          assert!(
            expr_typ == typ,
            "Function {id} returns {expr_typ} but must return {typ}."
          );
        }
        None => {
          assert!(
            typ == Typ::Void,
            "Function {id} returns void but must return {typ}."
          );
        }
      }

      TcResult::ok_ret(typ, st.get_function_context(id).define_all_vars())
    }
    Stmt::Expr(expr) => {
      // standalone expression

      let mut res = analyze_expr(id, expr, st);
      res.returns = None;
      res
    }
    Stmt::Assert(expr) => {
      // assertion elaborated into a conditional

      analyze_expr(id, expr, st);
      assert!(
        expr.get_type() == Typ::Bool,
        "Assert expression must evaluate to a boolean."
      );
      TcResult::ok()
    }
    Stmt::NoOp() => TcResult::ok(),
  }
}

fn analyze_expr(id: &Ident, expr: &mut Expr, st: &mut SymbolTable) -> TcResult {
  todo!();
}
