use std::collections::HashMap;

use crate::common::symbol_table::SymbolTable;
use crate::front::ast::{BinOp, UnOp};
use crate::intermediate::{
  ir_asm::{Exception, Instr},
  ir_codegen::ProgramIR,
};
use crate::x86_back::{
  regalloc::Regalloc,
  x86_asm::{X86Instr, X86Operand, X86WReg},
  x86_context::{Trap, X86Context, generate_traps},
};

/// An x86-64 program is the assembly instructions of its functions and traps.
pub struct X86Program {
  pub functions: HashMap<String, Vec<X86Instr>>,
  pub traps: Vec<X86Instr>,
}

/// Generate x86-64 assembly from program's IR.
pub fn generate_assembly(
  program: &ProgramIR,
  regalloc: Regalloc,
  symbol_table: &SymbolTable,
  allow_unsafe: bool,
) -> X86Program {
  let mut functions = HashMap::new();

  for (function_name, ir_context) in program {
    let coloring = regalloc
      .get(function_name)
      .unwrap_or_else(|| panic!("No register allocation found for function {function_name}."))
      .clone();
    let param_types = symbol_table
      .get_function_context(function_name)
      .get_params()
      .clone();

    let mut ctx = X86Context::new(coloring, param_types, format!(".L_c0_{function_name}_"));

    functions.insert(
      format!("_c0_{function_name}"),
      generate_function(&mut ctx, ir_context.linearize(), symbol_table, allow_unsafe),
    );
  }

  X86Program {
    functions,
    traps: generate_traps(),
  }
}

/// Generate x86-64 instructions for a function body.
fn generate_function(
  ctx: &mut X86Context,
  ir_instructions: Vec<Instr>,
  symbol_table: &SymbolTable,
  allow_unsafe: bool,
) -> Vec<X86Instr> {
  for instruction in ir_instructions {
    match instruction {
      // Binary operation
      Instr::BinOp { op, dest, lhs, rhs } => {
        let dest = ctx.get_temp_location(dest);
        let lhs = ctx.get_operand_location(lhs);
        let rhs = ctx.get_operand_location(rhs);

        match op {
          BinOp::Div | BinOp::Mod => {
            let rhs = if matches!(rhs, X86Operand::Register(_))
              && (rhs == X86Operand::Register(X86WReg::quotient(rhs.width()))
                || rhs == X86Operand::Register(X86WReg::modulo(rhs.width())))
            {
              let scratch = X86Operand::Register(X86WReg::scratch(rhs.width()));
              ctx.emit_move(rhs, scratch);
              scratch
            } else {
              rhs
            };

            ctx.emit_move(lhs, X86Operand::Register(X86WReg::quotient(dest.width())));
            ctx.emit_binary_op(op, Some(rhs), None);

            if matches!(op, BinOp::Div) {
              ctx.emit_move(X86Operand::Register(X86WReg::quotient(dest.width())), dest);
            } else {
              ctx.emit_move(X86Operand::Register(X86WReg::modulo(dest.width())), dest);
            }
          }
          BinOp::Sal | BinOp::Sar => {
            // generate shift validator in safe mode
            if !allow_unsafe {
              if let X86Operand::Immediate(imm) = rhs {
                if imm.value < 0 || imm.value > 31 {
                  ctx.emit_trap_jump(Trap::Sigfpe);
                }
              } else {
                ctx.emit_trap_if_lesser(rhs, 0, Trap::Sigfpe);
                ctx.emit_trap_if_greater(rhs, 31, Trap::Sigfpe);
              }
            }

            match rhs {
              X86Operand::Immediate(_) => {
                ctx.emit_move(lhs, dest);
                ctx.emit_binary_op(op, Some(rhs), Some(dest));
              }
              X86Operand::Register(_) | X86Operand::Stack(_) => {
                let scratch = X86Operand::Register(X86WReg::scratch(dest.width()));
                let shift_reg = X86WReg::shift().register;
                let save_lhs = matches!(lhs, X86Operand::Register(reg) if lhs != dest && reg.register == shift_reg);
                let save_dest =
                  matches!(dest, X86Operand::Register(reg) if reg.register == shift_reg);
                let temp_dest = if save_dest { scratch } else { dest };

                ctx.emit_move(lhs, temp_dest);

                if save_lhs {
                  if save_dest {
                    ctx.emit_push(lhs);
                  } else {
                    ctx.emit_move(lhs, scratch);
                  }
                }

                ctx.emit_move(
                  rhs,
                  X86Operand::Register(X86WReg {
                    register: shift_reg,
                    width: rhs.width(),
                  }),
                );
                ctx.emit_binary_op(op, None, Some(temp_dest));

                if save_lhs {
                  if save_dest {
                    ctx.emit_pop(lhs);
                  } else {
                    ctx.emit_move(scratch, lhs);
                  }
                }

                if save_dest {
                  ctx.emit_move(temp_dest, dest);
                }
              }
            };
          }
          BinOp::CmpEq | BinOp::CmpNeq | BinOp::Lt | BinOp::Gt | BinOp::Lte | BinOp::Gte => {
            let scratch = X86Operand::Register(X86WReg::scratch(rhs.width()));
            let mut op = op;

            let (lhs, rhs) = match (lhs, rhs) {
              (X86Operand::Immediate(_), X86Operand::Immediate(_))
              | (X86Operand::Stack(_), X86Operand::Stack(_)) => {
                ctx.emit_move(lhs, scratch);
                (rhs, scratch)
              }
              (X86Operand::Immediate(_), _) => {
                op = match op {
                  BinOp::Lt => BinOp::Gt,
                  BinOp::Gt => BinOp::Lt,
                  BinOp::Lte => BinOp::Gte,
                  BinOp::Gte => BinOp::Lte,
                  _ => op,
                };
                (lhs, rhs)
              }
              _ => (rhs, lhs),
            };

            ctx.emit_binary_cmp(op, lhs, rhs, dest);
          }
          BinOp::Add
          | BinOp::Sub
          | BinOp::Mul
          | BinOp::And
          | BinOp::Xor
          | BinOp::Or
          | BinOp::LAnd
          | BinOp::LOr => {
            let target = if matches!(dest, X86Operand::Register(_))
              || !(matches!(op, BinOp::Mul) || matches!(rhs, X86Operand::Stack(_)))
            {
              dest
            } else {
              X86Operand::Register(X86WReg::scratch(dest.width()))
            };

            if target == rhs && target != lhs {
              if matches!(op, BinOp::Sub) {
                // (a - b) == (-b + a)
                ctx.emit_unary_op(UnOp::Neg, rhs);
                ctx.emit_binary_op(BinOp::Add, Some(lhs), Some(rhs));
              } else {
                ctx.emit_binary_op(op, Some(lhs), Some(rhs));
              }
            } else {
              ctx.emit_move(lhs, target);
              ctx.emit_binary_op(op, Some(rhs), Some(target));
            }

            ctx.emit_move(target, dest);
          }
        };
      }
      // Unary operation
      Instr::UnOp { op, dest, src } => {
        let src = ctx.get_operand_location(src);
        let dest = ctx.get_temp_location(dest);
        ctx.emit_move(src, dest);
        ctx.emit_unary_op(op, dest);
      }
      // Function-scoped label
      Instr::Label(label) => ctx.emit_label(label.0),
      // Unconditional jump to label
      Instr::JumpTo(label) => ctx.emit_jump(label.0),
      // Conditional jump to label
      Instr::JumpIf { pred, holds, fails } => {
        let pred = ctx.get_operand_location(pred);
        if let X86Operand::Immediate(imm) = pred {
          ctx.emit_jump(if imm.value != 0 { holds.0 } else { fails.0 });
        } else {
          ctx.emit_conditional(pred, holds.0, fails.0);
        }
      }
      // Function call
      Instr::Call { dest, name, args } => {
        let dest = dest.map(|dest| ctx.get_temp_location(dest));
        let args = args
          .iter()
          .map(|arg| ctx.get_operand_location(arg.clone()))
          .collect::<Vec<_>>();
        let name = if symbol_table.is_header_function(&name) {
          name
        } else {
          format!("_c0_{name}")
        };

        ctx.emit_call(dest, args, name);
      }
      // Return from function
      Instr::Return(src) => {
        let src = src.map(|src| ctx.get_operand_location(src));
        ctx.emit_return(src);
      }
      // Throw an exception
      Instr::Throw(exception) => ctx.emit_trap_jump(match exception {
        Exception::Abort => Trap::Abort,
        Exception::Arith => Trap::Sigfpe,
      }),
      // Copy (move) instruction
      Instr::Move { dest, src } => {
        let src = ctx.get_operand_location(src);
        let dest = ctx.get_temp_location(dest);
        ctx.emit_move(src, dest);
      }
      // SSA deconstruction should ensure Phi never gets here
      Instr::Phi { .. } => unreachable!("Not expecting Phi instructions in x86 codegen."),
    };
  }

  ctx.assemble()
}
