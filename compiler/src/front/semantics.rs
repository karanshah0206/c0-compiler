use std::collections::HashSet;

use crate::common::symbol_table::SymbolTable;
use crate::front::ast::*;

/// Perform semantic analysis on the program.
pub fn analyze_program(header_ast: &mut ProgramAST, source_ast: &mut ProgramAST) -> SymbolTable {
  use GlobalDeclaration::*;

  let mut symbol_table = SymbolTable::new();

  // analyze declarations in header
  for declaration in header_ast {
    match declaration {
      TDefn(typ, id) => symbol_table.add_typedef(id, typ),
      SDefn(id, fields) => symbol_table.define_struct(id, fields),
      FDecl(typ, id, params) => symbol_table.declare_function(id, typ, params, true),
      FDefn(_, _, _, _) => unreachable!("Function definitions are illegal in a header file."),
      SDecl(_) => {} // struct declarations are inconsequential
    }
  }

  // functions called in source code must be defined (even if those function calls are unreachable)
  // main is the entry point, it must always be defined
  let mut functions_to_define: HashSet<Ident> = ["main".to_string()].into_iter().collect();
  symbol_table.declare_function(&"main".to_string(), &mut Typ::Int, &mut [], false);

  // analyze source program
  for declaration in source_ast {
    match declaration {
      TDefn(typ, id) => symbol_table.add_typedef(id, typ),
      SDefn(id, fields) => symbol_table.define_struct(id, fields),
      FDecl(typ, id, params) => symbol_table.declare_function(id, typ, params, false),
      FDefn(typ, id, params, ast) => {
        symbol_table.define_function(id, typ, params);
        functions_to_define.extend(analyze_function(id, ast, typ, &mut symbol_table));
      }
      SDecl(_) => {} // struct declarations are inconsequential
    }
  }

  // ensure that all functions that are called are defined
  for function in functions_to_define {
    assert!(
      symbol_table.is_function_defined(&function),
      "Missing definition for function {function}."
    );
  }

  symbol_table
}

/// Result of typechecking an expression or statement.
struct TcResult {
  /// Does the statement always return?
  returns: bool,
  /// Variables defined in this expression/statement.
  defines: HashSet<Ident>,
}

impl TcResult {
  /// No returns, no defines in statement/expression.
  fn ok() -> Self {
    TcResult {
      returns: false,
      defines: HashSet::new(),
    }
  }

  /// Statement/expression defines variables, no return.
  fn ok_def(defines: HashSet<Ident>) -> Self {
    TcResult {
      returns: false,
      defines,
    }
  }

  /// Statement returns.
  fn ok_ret(defines: HashSet<Ident>) -> Self {
    TcResult {
      returns: true,
      defines,
    }
  }
}

/// Perform semantic analysis on a function's AST.
fn analyze_function(id: &Ident, ast: &mut Stmt, typ: &Typ, st: &mut SymbolTable) -> HashSet<Ident> {
  assert!(
    analyze_stmt(id, ast, st).returns || typ == &Typ::Void,
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

      assert!(
        is_var_type_valid(&var.0),
        "Variable {} declared with type {}.",
        var.1,
        var.0
      );

      st.get_mut_function_context(id).declare_var(var.clone());
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

      assert!(
        is_var_type_valid(&var.0),
        "Variable {} declared with type {}.",
        var.1,
        var.0
      );

      analyze_expr(id, expr, st);

      assert!(
        var.0 == expr.get_type()
          || (matches!(var.0, Typ::Pointer(..)) && expr.get_type() == Typ::Null),
        "Mismatching types in defining variable {}.",
        var.1
      );

      st.get_mut_function_context(id).declare_var(var.clone());
      st.get_mut_function_context(id).define_var(var.clone());
      TcResult::ok_def(HashSet::from_iter(vec![var.1.to_string()]))
    }
    Stmt::Asgn(lhs, asn_op, expr) => {
      // assignment to an assignable expression

      if *asn_op == AsnOp::Equal
        && let Some(mut var) = resolve_pointer_mul_ambiguity(lhs, st)
      {
        assert!(
          !st.is_typedef(&var.1),
          "Cannot use variable identifier {} because it is a type definition.",
          var.1
        );

        var.0 = st.resolve_type(var.0.clone());

        assert!(
          is_var_type_valid(&var.0),
          "Variable {} declared with type {}.",
          var.1,
          var.0
        );

        analyze_expr(id, expr, st);

        assert!(
          var.0 == expr.get_type()
            || (matches!(var.0, Typ::Pointer(..)) && expr.get_type() == Typ::Null),
          "Mismatching types in defining variable {}.",
          var.1
        );

        st.get_mut_function_context(id).declare_var(var.clone());
        st.get_mut_function_context(id).define_var(var.clone());
        return TcResult::ok_def(HashSet::from_iter(vec![var.1.to_string()]));
      }

      let target_var = match lhs {
        Expr::Variable(var_id, _) => Some(var_id.clone()),
        _ => None,
      };

      let lhs_typ = analyze_assign_target(id, lhs, st);
      analyze_expr(id, expr, st);

      assert!(
        is_var_type_valid(&lhs_typ),
        "Assignment target has invalid type {lhs_typ}."
      );

      assert!(
        expr.get_type() != Typ::Void,
        "Cannot assign to the void type."
      );

      if *asn_op == AsnOp::Equal {
        assert!(
          expr.get_type() == lhs_typ
            || (matches!(lhs_typ, Typ::Pointer(..)) && expr.get_type() == Typ::Null),
          "Mismatching types in assignment."
        );
      } else {
        if let Some(var_id) = target_var.as_ref() {
          assert!(
            st.get_function_context(id).is_var_defined(var_id),
            "Variable {var_id} not defined in this scope."
          );
        }

        // typecheck elaboration into binary operation
        if let Some(binop) = asn_op.to_binop() {
          let mut binop_expr =
            Expr::Binop(Box::new(lhs.clone()), binop, Box::new(expr.clone()), None);
          analyze_expr(id, &mut binop_expr, st);

          assert!(
            binop_expr.get_type() == lhs_typ,
            "Mismatching types in assignment."
          );
        }
      }

      if let Some(var_id) = target_var {
        st.get_mut_function_context(id)
          .define_var((lhs_typ, var_id.clone()));
        TcResult::ok_def(HashSet::from_iter(vec![var_id]))
      } else {
        TcResult::ok()
      }
    }
    Stmt::Cond(cond_expr, if_stmt, else_stmt) => {
      // an if-else statement

      analyze_expr(id, cond_expr, st);
      assert!(
        cond_expr.get_type() == Typ::Bool,
        "Condition expression must evaluate to bool."
      );

      st.get_mut_function_context(id).scope_context.enter_scope();
      let if_res = analyze_stmt(id, if_stmt, st);
      st.get_mut_function_context(id).scope_context.exit_scope();

      st.get_mut_function_context(id).scope_context.enter_scope();
      let else_res = analyze_stmt(id, else_stmt, st);
      st.get_mut_function_context(id).scope_context.exit_scope();

      // outer variables that are defined in both branches become defined in outer scope.
      // if both branches return, the statement returns.
      let function_ctx = st.get_mut_function_context(id);

      let mut res = TcResult::ok_def(
        if_res
          .defines
          .intersection(&else_res.defines)
          .cloned()
          .collect(),
      );

      if if_res.returns && else_res.returns {
        res.returns = true
      }

      let defined_vars = res
        .defines
        .iter()
        .map(|var_id| (function_ctx.get_var_type(var_id), var_id.to_string()))
        .collect::<Vec<_>>();

      for var in defined_vars {
        if function_ctx.is_var_declared(&var.1) {
          function_ctx.define_var(var);
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

      st.get_mut_function_context(id).scope_context.enter_scope();
      analyze_stmt(id, body_stmt, st);
      st.get_mut_function_context(id).scope_context.exit_scope();

      TcResult::ok()
    }
    Stmt::For(init_stmt, cond_expr, step_stmt, body_stmt) => {
      // a for loop

      let mut res = TcResult::ok();

      st.get_mut_function_context(id).scope_context.enter_scope();

      if let Some(init_stmt) = init_stmt.as_mut() {
        res = analyze_stmt(id, init_stmt, st);
      }

      analyze_expr(id, cond_expr, st);
      assert!(
        cond_expr.get_type() == Typ::Bool,
        "For loop condition must evaluate to bool."
      );

      st.get_mut_function_context(id).scope_context.enter_scope();
      let body_res = analyze_stmt(id, body_stmt, st);
      st.get_mut_function_context(id).scope_context.exit_scope();

      // Step executes after the body, but still lives in the for-loop scope.
      // So, we promote outer variables defined in body to the for-loop scope.
      let function_ctx = st.get_mut_function_context(id);
      let defined_in_body = body_res
        .defines
        .iter()
        .filter(|var_id| function_ctx.is_var_declared(var_id))
        .map(|var_id| (function_ctx.get_var_type(var_id), var_id.to_string()))
        .collect::<Vec<_>>();

      for var in defined_in_body {
        function_ctx.define_var(var);
      }

      if let Some(step_stmt) = step_stmt.as_mut() {
        analyze_stmt(id, step_stmt, st);
      }

      st.get_mut_function_context(id).scope_context.exit_scope();

      // if an outer variable is defined in initializer, it is defined in parent scope
      let function_ctx = st.get_mut_function_context(id);

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

      st.get_mut_function_context(id).scope_context.enter_scope();

      for stmt in stmts {
        let res = analyze_stmt(id, stmt, st);
        block_res.defines.extend(res.defines);

        if res.returns && !block_res.returns {
          block_res.returns = true;
          block_res
            .defines
            .extend(st.get_mut_function_context(id).define_all_vars());
        } else if res.returns {
          block_res.returns = true;
        }
      }

      st.get_mut_function_context(id).scope_context.exit_scope();

      // outer variables defined in inner scopes become defined on the block's scope.
      let function_ctx = st.get_mut_function_context(id);

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
        .unwrap_or_else(|| unreachable!("Unknown function {id}."));

      match expr {
        Some(expr) => {
          analyze_expr(id, expr, st);
          let expr_typ = expr.get_type();
          assert!(
            expr_typ == typ
              || (matches!(expr_typ, Typ::Pointer(..)) && typ == Typ::Null)
              || (matches!(typ, Typ::Pointer(..)) && expr_typ == Typ::Null),
            "Returning {expr_typ}, but function {id} returns {typ}."
          );
          assert!(
            expr_typ != Typ::Void,
            "Return in function {id} cannot use a void expression."
          );
        }
        None => {
          assert!(
            typ == Typ::Void,
            "Returning void, but function {id} returns {typ}."
          );
        }
      }

      TcResult::ok_ret(st.get_mut_function_context(id).define_all_vars())
    }
    Stmt::Assert(expr) => {
      // assertion

      analyze_expr(id, expr, st);
      assert!(
        expr.get_type() == Typ::Bool,
        "Assert expression must evaluate to bool."
      );
      TcResult::ok()
    }
    // standalone expression
    Stmt::Expr(expr) => {
      if let Some(mut var) = resolve_pointer_mul_ambiguity(expr, st) {
        assert!(
          !st.is_typedef(&var.1),
          "Cannot use variable identifier {} because it is a type definition.",
          var.1
        );

        var.0 = st.resolve_type(var.0.clone());

        assert!(
          is_var_type_valid(&var.0),
          "Illegal type {} for variable {}.",
          var.0,
          var.1
        );

        st.get_mut_function_context(id).declare_var(var);
        TcResult::ok()
      } else {
        let res = analyze_expr(id, expr, st);
        assert!(
          matches!(
            expr,
            Expr::Call(..) | Expr::Alloc(..) | Expr::AllocArray(..)
          ) || matches!(
            expr.get_type(),
            Typ::Int | Typ::Bool | Typ::Pointer(..) | Typ::Array(..) | Typ::Null | Typ::Void
          ),
          "Bad expression statement {expr}."
        );
        res
      }
    }
    // no operation (do nothing)
    Stmt::NoOp() => TcResult::ok(),
  }
}

/// Perform semantic analysis on an expression.
fn analyze_expr(id: &Ident, expr: &mut Expr, st: &mut SymbolTable) -> TcResult {
  use Expr::*;

  match expr {
    Number(_) => TcResult::ok(),
    Bool(_) => TcResult::ok(),
    Null => TcResult::ok(),
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
        e_typ == r_expr.get_type()
          || (matches!(e_typ, Typ::Null) && matches!(r_expr.get_type(), Typ::Pointer(..)))
          || (matches!(e_typ, Typ::Pointer(..)) && matches!(r_expr.get_type(), Typ::Null)),
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
            e_typ != Typ::Void && !matches!(e_typ, Typ::Struct(..)),
            "Binary operator {bin_op} doesn't support the type {e_typ}."
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
    Ternop(cond_expr, if_expr, else_expr, typ) => {
      // ternary operator

      analyze_expr(id, cond_expr, st);
      assert!(
        cond_expr.get_type() == Typ::Bool,
        "Condition expression in ternary must evaluate to bool."
      );

      analyze_expr(id, if_expr, st);
      analyze_expr(id, else_expr, st);

      let if_typ = if_expr.get_type();
      let else_typ = else_expr.get_type();

      let e_typ = if if_typ == Typ::Null {
        else_typ.clone()
      } else {
        if_typ.clone()
      };

      assert!(
        if_typ == else_typ
          || (if_typ == Typ::Null && matches!(else_typ, Typ::Pointer(..)))
          || (matches!(if_typ, Typ::Pointer(..)) && else_typ == Typ::Null),
        "Ternary's arms must be of matching type."
      );
      assert!(
        e_typ != Typ::Void && !matches!(e_typ, Typ::Struct(..)),
        "Ternary does not support operating over the type {e_typ}."
      );

      *typ = Some(e_typ);

      TcResult::ok()
    }
    Call(func_id, args, typ) => {
      // function call

      let (ret_typ, params) = st
        .get_function_signature(func_id)
        .unwrap_or_else(|| unreachable!("Call to unknown function {func_id}"));

      assert!(
        !st.get_function_context(id).is_var_declared(func_id),
        "Cannot call function {func_id} as it is shadowed by an identical identifier."
      );

      assert!(
        args.len() == params.len(),
        "Mismatching argument list length in function call to {func_id}."
      );

      for (i, arg) in args.iter_mut().enumerate() {
        analyze_expr(id, arg, st);
        assert!(
          arg.get_type() == params[i]
            || (arg.get_type() == Typ::Null && matches!(params[i], Typ::Pointer(..)))
        );
      }

      st.get_mut_function_context(id)
        .insert_function_call(func_id.to_string());

      *typ = Some(ret_typ);
      TcResult::ok()
    }
    Deref(pointer_expr, depth, typ) => {
      // pointer dereference

      analyze_expr(id, pointer_expr, st);

      if let Typ::Pointer(inner, ref mut pointer_depth) = pointer_expr.get_type() {
        *typ = if *pointer_depth > *depth {
          *pointer_depth -= *depth;
          Some(Typ::Pointer(inner, *pointer_depth))
        } else if *pointer_depth == *depth {
          Some(*inner)
        } else {
          panic!("Dimension mismatch for pointer dereferencing in {id}.");
        };
      } else {
        panic!("Bad source for pointer dereference in {id}.");
      }

      TcResult::ok()
    }
    ArrayIndex(array_expr, index_expr, typ) => {
      // array indexing

      analyze_expr(id, array_expr, st);
      analyze_expr(id, index_expr, st);

      assert!(
        index_expr.get_type() == Typ::Int,
        "Array indices must evaluate to integers."
      );

      if let Typ::Array(inner, depth) = array_expr.get_type() {
        *typ = if depth > 1 {
          Some(Typ::Array(inner.clone(), depth - 1))
        } else {
          Some(*inner)
        };
      } else {
        panic!("Cannot index on a non-array type.")
      }

      TcResult::ok()
    }
    StructDeref(struct_expr, field_id, typ) => {
      // field dereferencing for structs

      analyze_expr(id, struct_expr, st);

      if let Typ::Struct(struct_id) = struct_expr.get_type() {
        assert!(
          st.is_struct_defined(&struct_id),
          "Attempting to fetch field for undefined struct {struct_id}."
        );

        *typ = Some(
          st.get_struct_field_type(&struct_id, field_id)
            .unwrap_or_else(|| panic!("Struct {struct_id} does not have a field {field_id}.")),
        );
      } else {
        panic!("Attempting to fetch field from a non-struct type.");
      }

      TcResult::ok()
    }
    Alloc(alloc_type, typ) => {
      // heap allocation for a pointer

      *alloc_type = st.resolve_type(alloc_type.clone());

      if let Typ::Struct(struct_id) = alloc_type {
        assert!(
          st.is_struct_defined(struct_id),
          "Attempting to allocate memory for undefined struct {struct_id}."
        );
      }
      assert!(
        matches!(*alloc_type, Typ::Struct(_)) || is_var_type_valid(alloc_type),
        "Bad type for heap allocation."
      );

      *typ = Some(if let Typ::Pointer(inner, depth) = alloc_type.clone() {
        Typ::Pointer(inner, depth + 1)
      } else {
        Typ::Pointer(Box::new(alloc_type.clone()), 1)
      });
      TcResult::ok()
    }
    AllocArray(elem_type, size_expr, typ) => {
      // heap allocation for an array

      *elem_type = st.resolve_type(elem_type.clone());

      if let Typ::Struct(struct_id) = elem_type {
        assert!(
          st.is_struct_defined(struct_id),
          "Attempting to allocate memory for undefined struct {struct_id}."
        );
      }
      assert!(
        matches!(*elem_type, Typ::Struct(_)) || is_var_type_valid(elem_type),
        "Bad type for heap allocation."
      );

      analyze_expr(id, size_expr, st);
      assert!(
        size_expr.get_type() == Typ::Int,
        "Array allocation requires integer value for size parameter."
      );

      *typ = Some(if let Typ::Array(inner, depth) = elem_type.clone() {
        Typ::Array(inner, depth + 1)
      } else {
        Typ::Array(Box::new(elem_type.clone()), 1)
      });
      TcResult::ok()
    }
  }
}

/// Perform semantic analysis on an assignment target.
fn analyze_assign_target(id: &Ident, expr: &mut Expr, st: &mut SymbolTable) -> Typ {
  use Expr::*;

  fn is_lvalue_expr(expr: &Expr) -> bool {
    match expr {
      Expr::Variable(_, _) => true,
      Expr::Deref(inner, _, _) | Expr::ArrayIndex(inner, _, _) | Expr::StructDeref(inner, _, _) => {
        is_lvalue_expr(inner)
      }
      _ => false,
    }
  }

  

  match expr {
    Variable(var_id, typ) => {
      let function_ctx = st.get_function_context(id);

      assert!(
        function_ctx.is_var_declared(var_id),
        "Variable {var_id} not declared in this scope."
      );

      let var_typ = function_ctx.get_var_type(var_id);
      *typ = Some(var_typ.clone());
      var_typ
    }
    Deref(pointer_expr, depth, typ) => {
      analyze_expr(id, pointer_expr, st);

      assert!(
        is_lvalue_expr(pointer_expr.as_ref()),
        "Pointer dereference assignment target must be based on an lvalue expression."
      );

      if let Typ::Pointer(inner, pointer_depth) = pointer_expr.get_type() {
        *typ = if pointer_depth > *depth {
          Some(Typ::Pointer(inner, pointer_depth - *depth))
        } else if pointer_depth == *depth {
          Some(*inner)
        } else {
          panic!("Dimension mismatch for pointer dereferencing in {id}.");
        };
      } else {
        panic!("Bad source for pointer dereference in {id}.");
      }

      typ.clone().unwrap_or(Typ::Void)
    }
    ArrayIndex(array_expr, index_expr, typ) => {
      analyze_expr(id, array_expr, st);
      analyze_expr(id, index_expr, st);

      assert!(
        is_lvalue_expr(array_expr.as_ref()),
        "Array indexing assignment target must be based on an lvalue expression."
      );

      assert!(
        index_expr.get_type() == Typ::Int,
        "Array indices must evaluate to integers."
      );

      if let Typ::Array(inner, depth) = array_expr.get_type() {
        *typ = if depth > 1 {
          Some(Typ::Array(inner.clone(), depth - 1))
        } else {
          Some(*inner)
        };
      } else {
        panic!("Cannot index on a non-array type.")
      }

      typ.clone().unwrap_or(Typ::Void)
    }
    StructDeref(struct_expr, field_id, typ) => {
      assert!(
        is_lvalue_expr(struct_expr.as_ref()),
        "Struct field assignment target must be based on an lvalue expression."
      );

      analyze_expr(id, struct_expr, st);

      if let Typ::Struct(struct_id) = struct_expr.get_type() {
        assert!(
          st.is_struct_defined(&struct_id),
          "Attempting to fetch field for undefined struct {struct_id}."
        );

        *typ = Some(
          st.get_struct_field_type(&struct_id, field_id)
            .unwrap_or_else(|| panic!("Struct {struct_id} does not have a field {field_id}.")),
        );
      } else {
        panic!("Attempting to fetch field from a non-struct type.");
      }

      typ.clone().unwrap_or(Typ::Void)
    }
    _ => panic!("Invalid assignment target."),
  }
}

/// Resolve ambiguity between pointers and multiplication operation.
/// If the expression is a pointer, the function returns Some(pointer_var).
fn resolve_pointer_mul_ambiguity(expr: &Expr, st: &SymbolTable) -> Option<Variable> {
  match expr {
    Expr::Binop(lhs, BinOp::Mul, rhs, _) => {
      if let Expr::Variable(id, _) = lhs.as_ref()
        && st.is_typedef(id)
      {
        let (var_id, pointer_depth) = get_pointer_var_depth(rhs)?;
        let typedef_expr = Typ::Typedef(id.clone());
        let pointer = if let Typ::Pointer(inner, depth) = typedef_expr {
          Typ::Pointer(inner, depth + pointer_depth + 1)
        } else {
          Typ::Pointer(Box::new(typedef_expr), pointer_depth + 1)
        };
        Some((pointer, var_id))
      } else {
        None
      }
    }
    _ => None,
  }
}

/// Determine the identity and dimensions of a pointer type.
fn get_pointer_var_depth(expr: &Expr) -> Option<(Ident, usize)> {
  match expr {
    Expr::Variable(var, _) => Some((var.clone(), 0)),
    Expr::Deref(inner, depth, _) => {
      get_pointer_var_depth(inner).map(|(var, inner_depth)| (var, inner_depth + depth))
    }
    _ => None,
  }
}

/// Helper to check whether a variable can be of given type.
fn is_var_type_valid(typ: &Typ) -> bool {
  match typ {
    Typ::Int | Typ::Bool => true,
    Typ::Array(inner, _) | Typ::Pointer(inner, _) => is_ptr_type_valid(inner),
    Typ::Void | Typ::Null | Typ::Typedef(_) | Typ::Struct(_) => false,
  }
}

/// Helper to check if a pointer's underlying type is valid.
fn is_ptr_type_valid(typ: &Typ) -> bool {
  let mut cur = typ;

  loop {
    match cur {
      Typ::Int | Typ::Bool | Typ::Struct(_) => return true,
      Typ::Array(inner, _) | Typ::Pointer(inner, _) => cur = inner.as_ref(),
      Typ::Void | Typ::Null | Typ::Typedef(_) => return false,
    }
  }
}
