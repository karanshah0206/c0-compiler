use std::collections::HashMap;

use crate::common::symbol_table::SymbolTable;
use crate::front::ast::*;
use crate::intermediate::{
  ir_asm::{Exception, Instr, Operand, Temp},
  ir_context::IRContext,
};

/// Program's IR is a collection of IRs for all defined source functions.
pub type ProgramIR = HashMap<Ident, IRContext>;

/// Perform a convenient munch on the ASTs of defined functions in the source program.
/// Generate an SSA IR using Braun et al.'s technique.
pub fn munch_program(ast: &ProgramAST, symbol_table: &SymbolTable) -> ProgramIR {
  ast
    .iter()
    .filter_map(|decl| {
      if let GlobalDeclaration::FDefn(ret_typ, name, params, body) = decl
        && symbol_table.is_defined(name)
      {
        Some((name.clone(), munch_function_body(ret_typ, params, body)))
      } else {
        None
      }
    })
    .collect()
}

/// Transform a function's AST into IR.
fn munch_function_body(typ: &Typ, params: &[Variable], body: &Stmt) -> IRContext {
  let mut ctx = IRContext::new();

  // parameters are treated as defined in the entry block.
  let entry_label = ctx.current_block_label;
  for (typ, param_id) in params {
    let temp = ctx.create_temp(typ.clone());
    ctx.write_variable(param_id, temp, entry_label);
  }

  if !munch_statement(body, &mut ctx) && *typ == Typ::Void {
    // implicit void return
    ctx.set_block_terminator(Instr::Return(None));
  }

  ctx.finalize_trivial_phis();

  ctx
}

/// Transform an AST statement into an IR instruction.
/// Returns `true` if statement is terminal.
fn munch_statement(stmt: &Stmt, ctx: &mut IRContext) -> bool {
  match stmt {
    Stmt::Decl(_) | Stmt::NoOp() => false,
    Stmt::Defn((var_typ, var_id), expr) => {
      let dest = get_operand_temp(munch_expression(expr, ctx), var_typ.clone(), ctx);
      ctx.write_variable(var_id, dest, ctx.current_block_label);
      false
    }
    Stmt::Asgn(var_id, asn_op, expr) => {
      let assigned_temp = match asn_op {
        AsnOp::Equal => get_operand_temp(munch_expression(expr, ctx), expr.get_type(), ctx),
        op => {
          let op = op.to_binop().unwrap(); // only equality doesn't have binop

          let operand_temp = ctx.read_variable(var_id, ctx.current_block_label);
          let rhs = munch_expression(expr, ctx);
          let dest = ctx.create_temp(operand_temp.1.clone());

          ctx.add_instr_to_block(Instr::BinOp {
            op,
            dest: dest.clone(),
            lhs: Operand::Temp(operand_temp),
            rhs,
          });

          dest
        }
      };

      ctx.write_variable(var_id, assigned_temp, ctx.current_block_label);

      false
    }
    Stmt::PostOp(var_id, post_op) => {
      let operand_temp = ctx.read_variable(var_id, ctx.current_block_label);
      let dest = ctx.create_temp(operand_temp.1.clone());

      ctx.add_instr_to_block(Instr::BinOp {
        op: post_op.to_binop(),
        dest: dest.clone(),
        lhs: Operand::Temp(operand_temp.clone()),
        rhs: Operand::Const((1, Typ::Int)), // currently post-ops are only inc/dec
      });

      ctx.write_variable(var_id, dest, ctx.current_block_label);

      false
    }
    Stmt::Cond(expr, if_stmt, else_stmt) => {
      let if_label = ctx.create_block();
      let else_label = ctx.create_block();
      let merge_label = ctx.create_block();

      // evaluate condition block (which is the current block)
      let condition_label = ctx.current_block_label;
      let condition = munch_expression(expr, ctx);
      ctx.set_block_terminator(Instr::JumpIf {
        pred: condition,
        holds: if_label,
        fails: else_label,
      });

      // evaluate if block
      ctx.switch_to_block(if_label);
      ctx.add_pred_to_block(condition_label);
      ctx.seal_block(if_label); // only predecessor is predecessor
      let if_terminated = munch_statement(if_stmt, ctx);
      let if_end_label = ctx.current_block_label;
      if !if_terminated {
        ctx.set_block_terminator(Instr::JumpTo(merge_label));
      }

      // evaluate else block
      ctx.switch_to_block(else_label);
      ctx.add_pred_to_block(condition_label);
      ctx.seal_block(else_label); // only predecessor is predecessor
      let else_terminated = munch_statement(else_stmt, ctx);
      let else_end_label = ctx.current_block_label;
      if !else_terminated {
        ctx.set_block_terminator(Instr::JumpTo(merge_label));
      }

      // no need to enter merge block if conditional terminates on both branches
      if if_terminated && else_terminated {
        return true;
      }

      // enter merge block
      ctx.switch_to_block(merge_label);
      if !if_terminated {
        ctx.add_pred_to_block(if_end_label);
      }
      if !else_terminated {
        ctx.add_pred_to_block(else_end_label);
      }
      ctx.seal_block(merge_label);

      false
    }
    Stmt::While(expr, body_stmt) => {
      let header_label = ctx.create_block();
      let body_label = ctx.create_block();
      let exit_label = ctx.create_block();

      // enter the loop header (condition block)
      let parent_label = ctx.current_block_label;
      ctx.set_block_terminator(Instr::JumpTo(header_label));
      ctx.switch_to_block(header_label);
      ctx.add_pred_to_block(parent_label);
      let condition = munch_expression(expr, ctx);
      ctx.set_block_terminator(Instr::JumpIf {
        pred: condition,
        holds: body_label,
        fails: exit_label,
      });
      let header_end_label = ctx.current_block_label;

      // evaluate the loop body
      ctx.switch_to_block(body_label);
      ctx.add_pred_to_block(header_end_label);
      ctx.seal_block(body_label); // only predecessor is header_end block

      // if loop body is not guaranteed to terminate, we have back-edge to header
      if !munch_statement(body_stmt, ctx) {
        let body_end_label = ctx.current_block_label;
        ctx.set_block_terminator(Instr::JumpTo(header_label)); // add back-edge
        ctx.switch_to_block(header_label); // go back to header
        ctx.add_pred_to_block(body_end_label); // header has predecessor in loop body end
      }

      ctx.seal_block(header_label); // we know all predecessors of loop header now

      // enter exit block
      ctx.switch_to_block(exit_label);
      ctx.add_pred_to_block(header_end_label);
      ctx.seal_block(exit_label); // only predecessor is header_end block

      false
    }
    Stmt::For(init_stmt, expr, step_stmt, body_stmt) => {
      // evaluate loop initializer
      if let Some(init_stmt) = init_stmt.as_ref() {
        if munch_statement(init_stmt, ctx) {
          // if loop initializer terminates, no need to evaluate the rest of the loop
          return true;
        }
      }

      let header_label = ctx.create_block();
      let body_label = ctx.create_block();
      let step_label = ctx.create_block();
      let exit_label = ctx.create_block();

      // enter loop header block
      let parent_label = ctx.current_block_label;
      ctx.set_block_terminator(Instr::JumpTo(header_label));
      ctx.switch_to_block(header_label);
      ctx.add_pred_to_block(parent_label);
      let condition = munch_expression(expr, ctx);
      ctx.set_block_terminator(Instr::JumpIf {
        pred: condition,
        holds: body_label,
        fails: exit_label,
      });
      let header_end_label = ctx.current_block_label;

      // enter loop body
      ctx.switch_to_block(body_label);
      ctx.add_pred_to_block(header_end_label);
      ctx.seal_block(body_label); // only predecessor is header_end block
      if !munch_statement(body_stmt, ctx) {
        // if loop body doesn't terminate, we add an edge from it to step block
        let body_end_label = ctx.current_block_label;
        ctx.set_block_terminator(Instr::JumpTo(step_label));
        ctx.switch_to_block(step_label);
        ctx.add_pred_to_block(body_end_label);
        ctx.seal_block(step_label); // only predecessor is body_end block

        let step_terminates = if let Some(step_stmt) = step_stmt.as_ref() {
          munch_statement(step_stmt, ctx)
        } else {
          false
        };

        if !step_terminates {
          // if step doesn't terminate, we add a back-edge from step to header
          let step_end_label = ctx.current_block_label;
          ctx.set_block_terminator(Instr::JumpTo(header_label));
          ctx.switch_to_block(header_label);
          ctx.add_pred_to_block(step_end_label);
        }
      }

      ctx.seal_block(header_label); // we now know all predecessors of header

      // evaluate loop exit block
      ctx.switch_to_block(exit_label);
      ctx.add_pred_to_block(header_end_label);
      ctx.seal_block(exit_label); // only predecessor is header_end block

      false
    }
    Stmt::Block(stmts) => {
      let mut terminates = false;
      for stmt in stmts {
        terminates = munch_statement(stmt, ctx);
        if terminates {
          break;
        }
      }
      terminates
    }
    Stmt::Ret(expr) => {
      let return_operand = expr.as_ref().map(|expr| munch_expression(expr, ctx));
      ctx.set_block_terminator(Instr::Return(return_operand));
      true
    }
    Stmt::Expr(expr) => {
      munch_expression(expr, ctx);
      false
    }
    Stmt::Assert(expr) => {
      let pass_label = ctx.create_block();
      let fail_label = ctx.create_block();

      // evaluate condition
      let parent_label = ctx.current_block_label;
      let condition = munch_expression(expr, ctx);
      ctx.set_block_terminator(Instr::JumpIf {
        pred: condition,
        holds: pass_label,
        fails: fail_label,
      });

      // evaluate fail block
      ctx.switch_to_block(fail_label);
      ctx.add_pred_to_block(parent_label);
      ctx.set_block_terminator(Instr::Throw(Exception::Abort));
      ctx.seal_block(fail_label); // only predecessor in parent block

      // evaluate pass block
      ctx.switch_to_block(pass_label);
      ctx.add_pred_to_block(parent_label);
      ctx.seal_block(pass_label); // only predecessor in parent block

      false
    }
  }
}

/// Transform expressions in AST to IR instructions
fn munch_expression(expr: &Expr, ctx: &mut IRContext) -> Operand {
  match expr {
    Expr::Number(number) => Operand::Const((*number, Typ::Int)),
    Expr::Bool(boolean) => Operand::Const((if *boolean { 1 } else { 0 }, Typ::Bool)),
    Expr::Variable(var_id, _) => Operand::Temp(ctx.read_variable(var_id, ctx.current_block_label)),
    Expr::Binop(lhs, bin_op, rhs, typ) => match bin_op {
      BinOp::LAnd | BinOp::LOr => short_circuit_binop(bin_op, lhs, rhs, typ, expr, ctx),
      _ => {
        let lhs = munch_expression(lhs, ctx);
        let rhs = munch_expression(rhs, ctx);

        let dest = ctx.create_temp(typ.clone().unwrap_or_else(|| expr.get_type()));

        ctx.add_instr_to_block(Instr::BinOp {
          op: *bin_op,
          dest: dest.clone(),
          lhs,
          rhs,
        });

        Operand::Temp(dest)
      }
    },
    Expr::Unop(un_op, expr, typ) => {
      let src = munch_expression(expr, ctx);
      let dest = ctx.create_temp(typ.clone().unwrap_or_else(|| expr.get_type()));
      ctx.add_instr_to_block(Instr::UnOp {
        op: *un_op,
        dest: dest.clone(),
        src,
      });
      Operand::Temp(dest)
    }
    Expr::Ternop(condition_expr, if_expr, else_expr, typ) => {
      let if_label = ctx.create_block();
      let else_label = ctx.create_block();
      let merge_label = ctx.create_block();

      // evaluate condition expression
      let parent_label = ctx.current_block_label;
      let condition = munch_expression(condition_expr, ctx);
      ctx.set_block_terminator(Instr::JumpIf {
        pred: condition,
        holds: if_label,
        fails: else_label,
      });

      // evaluate if block
      ctx.switch_to_block(if_label);
      ctx.add_pred_to_block(parent_label);
      ctx.seal_block(if_label); // only one predecessor in parent
      let if_operand = munch_expression(if_expr, ctx);
      let if_end_label = ctx.current_block_label;
      ctx.set_block_terminator(Instr::JumpTo(merge_label));

      // evaluate else block
      ctx.switch_to_block(else_label);
      ctx.add_pred_to_block(parent_label);
      ctx.seal_block(else_label); // only one predecessor in parent
      let else_operand = munch_expression(else_expr, ctx);
      let else_end_label = ctx.current_block_label;
      ctx.set_block_terminator(Instr::JumpTo(merge_label));

      // evaluate merge block and merge both branches using a Phi node
      ctx.switch_to_block(merge_label);
      ctx.add_pred_to_block(if_end_label);
      ctx.add_pred_to_block(else_end_label);
      ctx.seal_block(merge_label); // only two predecessors in if_end and else_end blocks
      let dest = ctx.create_temp(typ.clone().unwrap_or_else(|| expr.get_type()));
      ctx.add_instr_to_block(Instr::Phi {
        dest: dest.clone(),
        srcs: vec![(if_end_label, if_operand), (else_end_label, else_operand)],
      });

      Operand::Temp(dest)
    }
    Expr::Call(func_id, args, typ) => {
      let args = args
        .iter()
        .map(|arg| munch_expression(arg, ctx))
        .collect::<Vec<_>>();

      let result_typ = typ.clone().unwrap_or_else(|| expr.get_type());

      if result_typ == Typ::Void {
        ctx.add_instr_to_block(Instr::Call {
          dest: None,
          name: func_id.to_string(),
          args,
        });
        Operand::Const((0, Typ::Bool)) // sentinel for void return, sema already confirms it is never read
      } else {
        let dest = ctx.create_temp(result_typ);
        ctx.add_instr_to_block(Instr::Call {
          dest: Some(dest.clone()),
          name: func_id.to_string(),
          args,
        });
        Operand::Temp(dest)
      }
    }
  }
}

/// Transform binary logical operators into short-circuit evaluation instructions.
/// For logical AND, we short-circuit early to `false` if LHS is `false`.
/// For logical OR, we short-circuit early to `true` if LHS is `true`.
fn short_circuit_binop(
  bin_op: &BinOp,
  lhs: &Expr,
  rhs: &Expr,
  typ: &Option<Typ>,
  whole_expr: &Expr,
  ctx: &mut IRContext,
) -> Operand {
  let rhs_label = ctx.create_block();
  let short_circuit_label = ctx.create_block();
  let merge_label = ctx.create_block();

  let parent_label = ctx.current_block_label;
  let lhs = munch_expression(lhs, ctx); // both 

  match bin_op {
    BinOp::LAnd => ctx.set_block_terminator(Instr::JumpIf {
      pred: lhs,
      holds: rhs_label,
      fails: short_circuit_label,
    }),
    BinOp::LOr => ctx.set_block_terminator(Instr::JumpIf {
      pred: lhs,
      holds: short_circuit_label,
      fails: rhs_label,
    }),
    _ => unreachable!("Cannot short-circuit evaluation of non-logical binop in IR codegen."),
  };

  // evaluate RHS expression block
  ctx.switch_to_block(rhs_label);
  ctx.add_pred_to_block(parent_label);
  ctx.seal_block(rhs_label); // only predecessor is short-circuit parent
  let rhs_operand = munch_expression(rhs, ctx);
  let rhs_end_label = ctx.current_block_label;
  ctx.set_block_terminator(Instr::JumpTo(merge_label));

  // evaluate short-circuit block (case where no need to evaluate RHS)
  ctx.switch_to_block(short_circuit_label);
  ctx.add_pred_to_block(parent_label);
  ctx.seal_block(short_circuit_label); // only predecessor is short-circuit parent
  let short_circuit_operand = match bin_op {
    BinOp::LAnd => Operand::Const((0, Typ::Bool)),
    BinOp::LOr => Operand::Const((1, Typ::Bool)),
    _ => unreachable!("Cannot short-circuit evaluation of non-logical binop in IR codegen."),
  };
  let short_circuit_end_label = ctx.current_block_label;
  ctx.set_block_terminator(Instr::JumpTo(merge_label));

  // enter merge block and use a Phi node to merge from predecessors
  ctx.switch_to_block(merge_label);
  ctx.add_pred_to_block(rhs_end_label);
  ctx.add_pred_to_block(short_circuit_end_label);
  ctx.seal_block(merge_label); // only two predecessors in short-circuit and RHS blocks
  let dest = ctx.create_temp(typ.clone().unwrap_or_else(|| whole_expr.get_type()));
  ctx.add_instr_to_block(Instr::Phi {
    dest: dest.clone(),
    srcs: vec![
      (rhs_end_label, rhs_operand),
      (short_circuit_end_label, short_circuit_operand),
    ],
  });

  Operand::Temp(dest)
}

/// Ensure that an operand is mapped to a temp and return the temp.
fn get_operand_temp(op: Operand, typ: Typ, ctx: &mut IRContext) -> Temp {
  match op {
    Operand::Const(_) => {
      let dest = ctx.create_temp(typ);
      ctx.add_instr_to_block(Instr::Move {
        dest: dest.clone(),
        src: op,
      });
      dest
    }
    Operand::Temp(temp) => temp,
  }
}
