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

/// Perform semantic analysis on a statement.
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
    Stmt::Cond(cond_expr, if_stmt, else_stmt) => {
      // an if-else statement

      analyze_expr(id, cond_expr, st);
      assert!(
        cond_expr.get_type() == Typ::Bool,
        "Condition expression must evaluate to bool."
      );

      st.get_function_context(id).scope_context.enter_scope();
      let if_res = analyze_stmt(id, if_stmt, st);
      st.get_function_context(id).scope_context.exit_scope();

      st.get_function_context(id).scope_context.enter_scope();
      let else_res = analyze_stmt(id, else_stmt, st);
      st.get_function_context(id).scope_context.exit_scope();

      // outer variables that are defined in both branches become defined in outer scope.
      // if both branches return, the statement returns.
      let function_ctx = st.get_function_context(id);

      let mut res = TcResult::ok_def(
        if_res
          .defines
          .intersection(&else_res.defines)
          .cloned()
          .collect(),
      );

      if if_res.returns == else_res.returns {
        res.returns = if_res.returns
      }

      let defined_vars = res
        .defines
        .iter()
        .map(|var_id| (function_ctx.get_var_type(var_id), var_id.to_string()))
        .collect::<Vec<_>>();

      for var in defined_vars {
        if function_ctx.is_var_declared(&var.1) {
          function_ctx.define_var(var);
        } else {
          res.defines.remove(&var.1);
        }
      }

      res
    }
    Stmt::While(cond_expr, body_stmt) => {
      // a while loop

      analyze_expr(id, cond_expr, st);
      assert!(
        cond_expr.get_type() == Typ::Bool,
        "While loop condition must evaluate to bool."
      );

      st.get_function_context(id).scope_context.enter_scope();
      analyze_stmt(id, body_stmt, st);
      st.get_function_context(id).scope_context.exit_scope();

      TcResult::ok()
    }
    Stmt::For(init_stmt, cond_expr, step_stmt, body_stmt) => {
      // a for loop

      let mut res = TcResult::ok();

      st.get_function_context(id).scope_context.enter_scope();

      if let Some(init_stmt) = init_stmt.as_mut() {
        res = analyze_stmt(id, init_stmt, st);
      }

      analyze_expr(id, cond_expr, st);
      assert!(
        cond_expr.get_type() == Typ::Bool,
        "For loop condition must evaluate to bool."
      );

      if let Some(step_stmt) = step_stmt.as_mut() {
        analyze_stmt(id, step_stmt, st);
      }

      st.get_function_context(id).scope_context.enter_scope();
      analyze_stmt(id, body_stmt, st);
      st.get_function_context(id).scope_context.exit_scope();

      st.get_function_context(id).scope_context.exit_scope();

      // if an outer variable is defined in initializer, it is defined in parent scope
      let function_ctx = st.get_function_context(id);

      res
        .defines
        .retain(|var_id| function_ctx.is_var_declared(var_id));

      for var_id in res.defines.iter() {
        let var_typ = function_ctx.get_var_type(var_id);
        function_ctx.define_var((var_typ, var_id.to_string()));
      }

      res
    }
    Stmt::Block(stmts) => {
      // basic block (scoped collection of statements)

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
          block_res
            .defines
            .extend(st.get_function_context(id).define_all_vars());
        } else {
          block_res.defines.extend(res.defines);
        }
      }

      st.get_function_context(id).scope_context.exit_scope();

      // outer variables defined in inner scopes become defined on the block's scope.
      let function_ctx = st.get_function_context(id);

      let defined_vars = block_res
        .defines
        .iter()
        .map(|var_id| (function_ctx.get_var_type(var_id), var_id.to_string()))
        .collect::<Vec<_>>();

      for var in defined_vars {
        if function_ctx.is_var_declared(&var.1) {
          function_ctx.define_var(var);
        } else {
          block_res.defines.remove(&var.1);
        }
      }

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
            "Returning {expr_typ}, but function {id} returns {typ}."
          );
        }
        None => {
          assert!(
            typ == Typ::Void,
            "Returning void, but function {id} returns {typ}."
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

/// Perform semantic analysis on an expression.
fn analyze_expr(id: &Ident, expr: &mut Expr, st: &mut SymbolTable) -> TcResult {
  use Expr::*;

  match expr {
    Number(_) => TcResult::ok(),
    Bool(_) => TcResult::ok(),
    Variable(var_id, typ) => {
      // variable in the source code

      let function_ctx = st.get_function_context(id);
      *typ = Some(function_ctx.get_var_type(var_id));
      assert!(
        function_ctx.is_var_defined(var_id),
        "Variable {var_id} not defined in this scope."
      );
      TcResult::ok()
    }
    Binop(l_expr, bin_op, r_expr, typ) => {
      // binary operator

      analyze_expr(id, l_expr, st);
      analyze_expr(id, r_expr, st);

      let e_typ = l_expr.get_type();

      assert!(
        e_typ == r_expr.get_type(),
        "Binary operands must be of the same type."
      );

      *typ = match bin_op {
        BinOp::Add
        | BinOp::Sub
        | BinOp::Mul
        | BinOp::Div
        | BinOp::Mod
        | BinOp::And
        | BinOp::Xor
        | BinOp::Or
        | BinOp::Sal
        | BinOp::Sar => {
          assert!(
            e_typ == Typ::Int,
            "Binary operator {bin_op} expected int but got {e_typ}."
          );
          Some(Typ::Int)
        }
        BinOp::LAnd | BinOp::LOr => {
          assert!(
            e_typ == Typ::Bool,
            "Binary operator {bin_op} expected bool but got {e_typ}."
          );
          Some(Typ::Bool)
        }
        BinOp::CmpEq | BinOp::CmpNeq => {
          assert!(
            e_typ != Typ::Void,
            "Binary operator {bin_op} doesn't support the void type."
          );
          Some(Typ::Bool)
        }
        BinOp::Lt | BinOp::Gt | BinOp::Lte | BinOp::Gte => {
          assert!(
            e_typ == Typ::Int,
            "Binary operator {bin_op} expected int but got {e_typ}."
          );
          Some(Typ::Bool)
        }
      };

      TcResult::ok()
    }
    Unop(un_op, expr, typ) => {
      // unary opeartor

      *typ = match un_op {
        UnOp::Neg | UnOp::Not => Some(Typ::Int),
        UnOp::LNot => Some(Typ::Bool),
      };

      analyze_expr(id, expr, st);
      assert!(
        Some(expr.get_type()) == *typ,
        "Operand to the unary operator {un_op} is of unsupported type."
      );

      TcResult::ok()
    }
    Ternop(cond_expr, if_expr, else_expr, typ) => todo!(),
    Call(_, exprs, typ) => todo!(),
  }
}
