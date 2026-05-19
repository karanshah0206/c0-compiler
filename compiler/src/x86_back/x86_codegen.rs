use std::collections::HashMap;

use crate::common::symbol_table::SymbolTable;
use crate::front::ast::{BinOp, Typ};
use crate::intermediate::{
  ir_asm::{Exception, Instr, Operand},
  ir_codegen::ProgramIR,
};
use crate::x86_back::{
  x86_asm::{
    Immediate, MemVar,
    Width::{self, *},
    X86Instr, X86Operand, X86WReg,
  },
  x86_context::{Trap, X86Context, generate_traps},
  x86_regalloc::Regalloc,
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
      .unwrap_or_else(|| unreachable!("No register allocation found for function {function_name}."))
      .clone();
    let params_count = symbol_table
      .get_function_context(function_name)
      .get_params()
      .len();

    let mut ctx = X86Context::new(
      coloring,
      params_count,
      format!(".L_c0_{function_name}_"),
      ir_context,
    );

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
        let lhs_typ = match &lhs {
          Operand::Const((_, typ)) => typ.clone(),
          Operand::Temp((_, typ)) => typ.clone(),
        };
        let rhs_typ = match &rhs {
          Operand::Const((_, typ)) => typ.clone(),
          Operand::Temp((_, typ)) => typ.clone(),
        };
        let is_pointer_add = matches!(op, BinOp::Add)
          && matches!(lhs_typ, Typ::Pointer(..) | Typ::Array(..))
          && matches!(rhs_typ, Typ::Int);
        let is_array_index_addr = is_pointer_add && matches!(lhs_typ, Typ::Array(..));

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
            if let X86Operand::Immediate(imm) = rhs {
              if imm.value < 0 || imm.value > 31 {
                ctx.emit_trap_jump(Trap::Sigfpe);
                continue;
              }
            } else if !allow_unsafe {
              ctx.emit_trap_if_lesser(rhs, 0, Trap::Sigfpe);
              ctx.emit_trap_if_greater(rhs, 31, Trap::Sigfpe);
            }

            match rhs {
              X86Operand::Immediate(_) => {
                ctx.emit_move(lhs, dest);
                ctx.emit_binary_op(op, Some(rhs), Some(dest));
              }
              X86Operand::Register(_) | X86Operand::Stack(_) | X86Operand::Memory(_) => {
                let scratch = X86Operand::Register(X86WReg::scratch(dest.width()));
                let shift_reg = X86WReg::shift().register;
                let cl = X86Operand::Register(X86WReg {
                  register: shift_reg,
                  width: rhs.width(),
                });

                let dest_in_shift =
                  matches!(dest, X86Operand::Register(reg) if reg.register == shift_reg);
                let temp_dest = if dest_in_shift { scratch } else { dest };

                let reg_of = |op: X86Operand| match op {
                  X86Operand::Register(reg) => Some(reg.register),
                  _ => None,
                };
                let lhs_in_shift = reg_of(lhs) == Some(shift_reg);
                let rhs_in_temp_dest = match (reg_of(rhs), reg_of(temp_dest)) {
                  (Some(r1), Some(r2)) => r1 == r2,
                  _ => false,
                };

                if lhs_in_shift && rhs_in_temp_dest {
                  ctx.emit_move(rhs, scratch);
                  ctx.emit_move(lhs, temp_dest);
                  ctx.emit_move(scratch, cl);
                } else if rhs_in_temp_dest {
                  ctx.emit_move(rhs, cl);
                  ctx.emit_move(lhs, temp_dest);
                } else {
                  ctx.emit_move(lhs, temp_dest);
                  ctx.emit_move(rhs, cl);
                }

                ctx.emit_binary_op(op, None, Some(temp_dest));

                if dest_in_shift {
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
            let mut rhs = if is_pointer_add {
              match rhs {
                X86Operand::Immediate(imm) => X86Operand::Immediate(Immediate {
                  value: imm.value,
                  width: W64,
                }),
                X86Operand::Register(wreg) => X86Operand::Register(X86WReg {
                  register: wreg.register,
                  width: W64,
                }),
                X86Operand::Stack(_) | X86Operand::Memory(_) => {
                  to_w64_reg(ctx, rhs, X86WReg::scratch2(W64))
                }
              }
            } else {
              rhs
            };

            if !allow_unsafe && is_pointer_add {
              let base = if matches!(lhs, X86Operand::Register(_)) {
                lhs
              } else {
                let base = X86Operand::Register(X86WReg::scratch(W64));
                ctx.emit_move(lhs, base);
                base
              };

              ctx.emit_trap_if_zero(base, Trap::MemError);

              if is_array_index_addr {
                let index = to_w64_reg(ctx, rhs, X86WReg::scratch2(W64));
                ctx.emit_trap_if_lesser(index, 0, Trap::MemError);

                let elem_size = if let Typ::Array(_, _) = &lhs_typ {
                  array_index_elem_size(&lhs_typ, symbol_table)
                } else {
                  unreachable!("Invalid array type found in x86 generation.")
                };

                let scaled = index;
                if elem_size != 1 {
                  ctx.emit_binary_op(
                    BinOp::Mul,
                    Some(X86Operand::Immediate(Immediate {
                      value: elem_size,
                      width: W64,
                    })),
                    Some(scaled),
                  );
                }

                let base_reg = match base {
                  X86Operand::Register(wreg) => wreg.register,
                  _ => unreachable!(),
                };
                ctx.emit_trap_if_geq_bound(
                  scaled,
                  X86Operand::Memory(MemVar {
                    base: base_reg,
                    offset: -8,
                    width: W64,
                  }),
                  Trap::MemError,
                );

                rhs = scaled;
              } else {
                rhs = to_w64_reg(ctx, rhs, X86WReg::scratch2(W64));
              }
            } else if is_array_index_addr {
              let elem_size = if let Typ::Array(_, _) = &lhs_typ {
                array_index_elem_size(&lhs_typ, symbol_table)
              } else {
                unreachable!("Invalid array type found in x86 generation.")
              };

              let scaled = to_w64_reg(ctx, rhs, X86WReg::scratch2(W64));
              if elem_size != 1 {
                ctx.emit_binary_op(
                  BinOp::Mul,
                  Some(X86Operand::Immediate(Immediate {
                    value: elem_size,
                    width: W64,
                  })),
                  Some(scaled),
                );
              }
              rhs = scaled;
            }

            let target = if matches!(dest, X86Operand::Register(_))
              || !(matches!(op, BinOp::Mul) || matches!(rhs, X86Operand::Stack(_)))
            {
              dest
            } else {
              X86Operand::Register(X86WReg::scratch(dest.width()))
            };

            let mut sub_done = false;
            if target == rhs && target != lhs {
              if matches!(op, BinOp::Sub) {
                let scratch = X86Operand::Register(X86WReg::scratch(target.width()));
                ctx.emit_move(lhs, scratch);
                ctx.emit_binary_op(BinOp::Sub, Some(rhs), Some(scratch));
                ctx.emit_move(scratch, dest);
                sub_done = true;
              } else {
                ctx.emit_binary_op(op, Some(lhs), Some(rhs));
              }
            } else {
              ctx.emit_move(lhs, target);
              ctx.emit_binary_op(op, Some(rhs), Some(target));
            }

            if !sub_done {
              ctx.emit_move(target, dest);
            }
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
      // Tail-position function call
      Instr::TailCall { name, args } => {
        let args = args
          .iter()
          .map(|arg| ctx.get_operand_location(arg.clone()))
          .collect::<Vec<_>>();
        let return_typ = symbol_table
          .get_function_signature(&name)
          .unwrap_or_else(|| unreachable!("Unknown function {name} found in x86 codegen."))
          .0;
        let call_dest = if return_typ == Typ::Void {
          None
        } else {
          Some(X86Operand::Register(X86WReg::ret(return_type_width(
            return_typ,
          ))))
        };
        let name = if symbol_table.is_header_function(&name) {
          name
        } else {
          format!("_c0_{name}")
        };

        if !ctx.emit_tail_call(args.clone(), name.clone()) {
          // if tail call emit fails, emitting regular call
          ctx.emit_call(call_dest, args, name);
          ctx.emit_return(None);
        }
      }
      // Return from function
      Instr::Return(src) => {
        let src = src.map(|src| ctx.get_operand_location(src));
        ctx.emit_return(src);
      }
      // Throw an exception
      Instr::Throw(exception) => ctx.emit_trap_jump(match exception {
        Exception::Abort => Trap::Abort,
      }),
      // Copy (move) instruction
      Instr::Move { dest, src } => {
        let src = ctx.get_operand_location(src);
        let dest = ctx.get_temp_location(dest);
        ctx.emit_move(src, dest);
      }
      // Load data from the heap
      Instr::Load { dest, addr } => {
        let addr = ctx.get_operand_location(addr);
        let dest = ctx.get_temp_location(dest);

        let base = if matches!(addr, X86Operand::Register(_)) {
          addr
        } else {
          let base = X86Operand::Register(X86WReg::scratch(W64));
          ctx.emit_move(addr, base);
          base
        };

        if !allow_unsafe {
          ctx.emit_trap_if_zero(base, Trap::MemError);
        }

        ctx.emit_move(
          X86Operand::Memory(MemVar {
            base: match base {
              X86Operand::Register(wreg) => wreg.register,
              _ => unreachable!(),
            },
            offset: 0,
            width: dest.width(),
          }),
          dest,
        );
      }
      // Store data on the heap
      Instr::Store { addr, src } => {
        let addr = ctx.get_operand_location(addr);
        let src = ctx.get_operand_location(src);

        let base = if matches!(addr, X86Operand::Register(_)) {
          addr
        } else {
          let base = X86Operand::Register(X86WReg::scratch2(W64));
          ctx.emit_move(addr, base);
          base
        };

        if !allow_unsafe {
          ctx.emit_trap_if_zero(base, Trap::MemError);
        }

        ctx.emit_move(
          src,
          X86Operand::Memory(MemVar {
            base: match base {
              X86Operand::Register(wreg) => wreg.register,
              _ => unreachable!(),
            },
            offset: 0,
            width: src.width(),
          }),
        );
      }
      // Allocate memory on the heap
      Instr::Alloc { dest, size } => {
        let size = ctx.get_operand_location(size);
        let size = if matches!(size, X86Operand::Immediate(_)) {
          let tmp = X86Operand::Register(X86WReg::scratch(W64));
          ctx.emit_move(size, tmp);
          tmp
        } else {
          size
        };

        let dest = ctx.get_temp_location(dest);
        ctx.emit_call(
          Some(dest),
          vec![
            X86Operand::Immediate(Immediate {
              value: 1,
              width: W64,
            }),
            size,
          ],
          "calloc".to_string(),
        );
      }
      // Allocate contiguous memory block on the heap
      Instr::AllocArray { dest, size, count } => {
        let size = ctx.get_operand_location(size);
        let count = ctx.get_operand_location(count);

        if !allow_unsafe {
          ctx.emit_trap_if_lesser(count, 0, Trap::MemError);
        }

        let bytes = X86Operand::Register(X86WReg {
          register: X86WReg::scratch2(W64).register,
          width: W64,
        });
        let count_wide = to_w64_reg(ctx, count, X86WReg::scratch2(W64));
        ctx.emit_move(count_wide, bytes);
        ctx.emit_binary_op(BinOp::Mul, Some(size), Some(bytes));

        let alloc_size = X86Operand::Register(X86WReg::scratch(W64));
        ctx.emit_move(bytes, alloc_size);
        ctx.emit_binary_op(
          BinOp::Add,
          Some(X86Operand::Immediate(Immediate {
            value: 8,
            width: W64,
          })),
          Some(alloc_size),
        );

        ctx.emit_binary_op(
          BinOp::Sub,
          Some(X86Operand::Immediate(Immediate {
            value: 16,
            width: W64,
          })),
          Some(X86Operand::Register(X86WReg::stack_pointer())),
        );
        ctx.emit_move(
          bytes,
          X86Operand::Memory(MemVar {
            base: X86WReg::stack_pointer().register,
            offset: 0,
            width: W64,
          }),
        );

        let pointer = X86Operand::Register(X86WReg::scratch(W64));
        ctx.emit_call(
          Some(pointer),
          vec![
            X86Operand::Immediate(Immediate {
              value: 1,
              width: W64,
            }),
            alloc_size,
          ],
          "calloc".to_string(),
        );

        ctx.emit_move(
          X86Operand::Memory(MemVar {
            base: X86WReg::stack_pointer().register,
            offset: 0,
            width: W64,
          }),
          bytes,
        );

        ctx.emit_binary_op(
          BinOp::Add,
          Some(X86Operand::Immediate(Immediate {
            value: 16,
            width: W64,
          })),
          Some(X86Operand::Register(X86WReg::stack_pointer())),
        );

        ctx.emit_move(
          bytes,
          X86Operand::Memory(MemVar {
            base: X86WReg::scratch(W64).register,
            offset: 0,
            width: W64,
          }),
        );

        ctx.emit_binary_op(
          BinOp::Add,
          Some(X86Operand::Immediate(Immediate {
            value: 8,
            width: W64,
          })),
          Some(pointer),
        );

        let dest = ctx.get_temp_location(dest);
        ctx.emit_move(pointer, dest);
      }
      // SSA deconstruction should ensure Phi never gets here
      Instr::Phi { .. } => unreachable!("Not expecting Phi instructions in x86 codegen."),
    };
  }

  ctx.assemble()
}

/// Move arbitrarily sized source operand into a 64-bit register.
fn to_w64_reg(ctx: &mut X86Context, src: X86Operand, reg: X86WReg) -> X86Operand {
  ctx.emit_move(
    src,
    X86Operand::Register(X86WReg {
      register: reg.register,
      width: src.width(),
    }),
  );

  X86Operand::Register(X86WReg {
    register: reg.register,
    width: W64,
  })
}

/// Get size of the data type stored at the index of an array.
fn array_index_elem_size(array_typ: &Typ, symbol_table: &SymbolTable) -> i64 {
  match array_typ {
    Typ::Array(inner, depth) => {
      if *depth > 1 {
        8
      } else {
        type_size_bytes(inner.as_ref(), symbol_table)
      }
    }
    _ => unreachable!("Expected array type when evaluating array index element size."),
  }
}

/// Get the byte-size of a type.
fn type_size_bytes(typ: &Typ, symbol_table: &SymbolTable) -> i64 {
  match typ {
    Typ::Bool => 1,
    Typ::Int => 4,
    Typ::Pointer(..) | Typ::Array(..) => 8,
    Typ::Struct(struct_id) => symbol_table
      .get_struct_fields(struct_id)
      .unwrap_or_else(|| {
        unreachable!("Attempted size evaluation of unknown struct {struct_id} in x86 generation.")
      })
      .iter()
      .map(|(field_typ, _)| type_size_bytes(field_typ, symbol_table))
      .sum(),
    Typ::Void | Typ::Null | Typ::Typedef(_) => {
      unreachable!("Attempted size evaluation of invalid type {typ} in x86 generation.")
    }
  }
}

/// Get the width of a valid return type.
fn return_type_width(typ: Typ) -> Width {
  match typ {
    Typ::Bool => W8,
    Typ::Int => W32,
    Typ::Null | Typ::Pointer(..) | Typ::Array(..) => W64,
    Typ::Void | Typ::Typedef(_) | Typ::Struct(_) => {
      unreachable!("Invalid function return type {typ} in x86 generation.")
    }
  }
}
