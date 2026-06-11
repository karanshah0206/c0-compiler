use std::collections::{BTreeSet, HashMap};

use crate::front::ast::{BinOp, Typ, UnOp};
use crate::intermediate::{
  ir_asm::{Instr, Operand, Temp},
  ir_context::IRContext,
};
use crate::x86_back::{
  x86_asm::{Width::*, *},
  x86_regalloc::*,
};

/// Width in bytes for a slot on stack allotted to a temporary.
const STACK_SLOT_WIDTH: usize = 8;

/// SysV AMD64 stack alignment.
const STACK_ALIGNMENT: usize = 16;

/// Potential kinds of traps that can be called in the program.
pub enum Trap {
  /// Explicit abort
  Abort,
  /// Memory error
  MemError,
  /// Arithmetic error
  Sigfpe,
}

impl Trap {
  /// Get the global label for a trap.
  fn get_global_label(self) -> String {
    match self {
      Trap::Abort => ".L_abort",
      Trap::MemError => ".L_memerror",
      Trap::Sigfpe => ".L_sigfpe",
    }
    .to_string()
  }
}

/// Generate global trap instructions.
pub fn generate_traps() -> Vec<X86Instr> {
  const MEM_ERROR_SIGNAL_CODE: i64 = 12;
  vec![
    // abort trap
    X86Instr::Label(Trap::Abort.get_global_label()),
    X86Instr::Call("abort".to_string()),
    // arithmetic trap
    X86Instr::Label(Trap::Sigfpe.get_global_label()),
    X86Instr::Mov(
      X86Operand::Immediate(Immediate {
        value: 0,
        width: W64,
      }),
      X86Operand::Register(X86WReg::scratch(W64)),
    ),
    X86Instr::IDiv(X86Operand::Register(X86WReg::scratch(W64))),
    // memory trap
    X86Instr::Label(Trap::MemError.get_global_label()),
    X86Instr::Mov(
      X86Operand::Immediate(Immediate {
        value: MEM_ERROR_SIGNAL_CODE,
        width: W64,
      }),
      X86Operand::Register(X86WReg {
        register: X86Reg::call_argument()[0],
        width: W64,
      }),
    ),
    X86Instr::Call("raise".to_string()),
  ]
}

/// x86-64 code generation context.
pub struct X86Context {
  /// Generated assembly instructions in this context.
  instructions: Vec<X86Instr>,
  /// Set of callee-saved registers used in this context.
  used_callee_saved: BTreeSet<X86Reg>,
  /// Set of caller-saved registers used in this context.
  used_caller_saved: BTreeSet<X86Reg>,
  /// Register allocation for temporaries in this context.
  regalloc: Vec<Color>,
  /// Stack allocation for operands in this context.
  stack_allocation: HashMap<usize, StackVar>,
  /// Stack frame size (in bytes) for this context.
  frame_size: usize,
  /// Prefix for labels created in this context.
  label_prefix: String,
}

impl X86Context {
  /// Generate a new x86-64 code generation context for a function.
  pub fn new(
    regalloc: Vec<Color>,
    params_count: usize,
    label_prefix: String,
    ir_context: &IRContext,
  ) -> Self {
    let (spill_stack_allocation, spill_slot_count) =
      allocate_spill_slots(ir_context, &regalloc, params_count);
    let mut ctx = X86Context {
      instructions: Vec::new(),
      used_callee_saved: BTreeSet::new(),
      used_caller_saved: BTreeSet::new(),
      regalloc,
      stack_allocation: HashMap::new(),
      frame_size: 0,
      label_prefix,
    };

    let mut needs_stack_alignment = false;
    for block in ir_context.get_blocks().values() {
      if block
        .body
        .iter()
        .any(|instr| matches!(instr, Instr::Call { .. }))
      {
        needs_stack_alignment = true;
        break;
      } else if matches!(block.terminator, Some(Instr::Call { .. })) {
        needs_stack_alignment = true;
        break;
      }
    }

    let arg_regs = X86Reg::call_argument();
    let stack_depth = spill_slot_count * STACK_SLOT_WIDTH;

    for (temp_id, slot) in spill_stack_allocation {
      ctx.stack_allocation.insert(temp_id, slot);
    }

    // determine callee-saved registers used in this context
    for &allocation in ctx.regalloc.iter() {
      match allocation {
        UNCOLORED | SPILL => {}
        color => {
          let register = color_to_register(color);
          if X86Reg::callee_saved().contains(&register) {
            ctx.used_callee_saved.insert(register);
          }
        }
      }
    }

    // determine stack frame size with alignment
    ctx.frame_size = ctx.used_callee_saved.len() * STACK_SLOT_WIDTH + stack_depth;
    if ctx.frame_size.is_multiple_of(STACK_ALIGNMENT) && needs_stack_alignment {
      ctx.frame_size += STACK_SLOT_WIDTH;
    }

    // allocate stack frame
    if ctx.frame_size > 0 {
      ctx.instructions.push(X86Instr::Sub(
        X86Operand::Immediate(Immediate {
          value: ctx.frame_size as i64,
          width: W64,
        }),
        X86Operand::Register(X86WReg::stack_pointer()),
      ));
    }

    // save callee-saved registers
    for (index, &register) in ctx.used_callee_saved.clone().iter().enumerate() {
      ctx.emit_move(
        X86Operand::Register(X86WReg {
          register,
          width: W64,
        }),
        X86Operand::Stack(StackVar {
          offset: (ctx.frame_size - ((index + 1) * STACK_SLOT_WIDTH)) as i64,
          width: W64,
        }),
      );
    }

    // calculate offsets for arguments on the stack
    for (index, temp_id) in (X86Reg::call_argument().len()..params_count).enumerate() {
      ctx.stack_allocation.insert(
        temp_id,
        StackVar {
          offset: (ctx.frame_size + (index + 1) * STACK_SLOT_WIDTH) as i64,
          width: W64,
        },
      );
    }

    // move arguments temporaries to their colored destination
    for temp_id in 0..params_count {
      let arg_src = if temp_id < arg_regs.len() {
        X86Operand::Register(X86WReg {
          register: arg_regs[temp_id],
          width: W64,
        })
      } else {
        X86Operand::Stack(*ctx.stack_allocation.get(&temp_id).unwrap_or_else(|| {
          unreachable!("Missing stack allocation for argument temporary with id {temp_id}.")
        }))
      };

      match ctx.regalloc[temp_id] {
        UNCOLORED => {
          unreachable!("Found uncolored argument temporary with id {temp_id}.")
        }
        SPILL => ctx.emit_move(
          arg_src,
          X86Operand::Stack(*ctx.stack_allocation.get(&temp_id).unwrap_or_else(|| {
            unreachable!("Missing stack allocation for argument temporary with id {temp_id}.")
          })),
        ),
        register_color => ctx.emit_move(
          arg_src,
          X86Operand::Register(X86WReg {
            register: color_to_register(register_color),
            width: W64,
          }),
        ),
      };
    }

    ctx
  }

  /// Get concrete location assigned to a compile-time temporary.
  pub fn get_temp_location(&mut self, temp: Temp) -> X86Operand {
    let color = *self.regalloc.get(temp.0).unwrap_or_else(|| {
      unreachable!("Unknown temporary with id {} found in x86 codegen.", temp.0)
    });

    if color == SPILL {
      X86Operand::Stack(
        self
          .stack_allocation
          .get(&temp.0)
          .unwrap_or_else(|| {
            unreachable!(
              "Missing stack allocation for temporary with id {} in x86 codegen.",
              temp.0
            )
          })
          .as_width(width_for_type(&temp.1)),
      )
    } else {
      let register = color_to_register(color);
      if X86Reg::caller_saved().contains(&register) {
        self.used_caller_saved.insert(register);
      }
      X86Operand::Register(X86WReg {
        register,
        width: width_for_type(&temp.1),
      })
    }
  }

  /// Get the concrete location of sized immediate assigned to an operand in the IR.
  pub fn get_operand_location(&mut self, operand: Operand) -> X86Operand {
    match operand {
      Operand::Const((value, typ)) => X86Operand::Immediate(Immediate {
        value,
        width: width_for_type(&typ),
      }),
      Operand::Temp(temp) => self.get_temp_location(temp),
    }
  }

  /// Emit a label with given label identifier.
  pub fn emit_label(&mut self, label_id: usize) {
    self
      .instructions
      .push(X86Instr::Label(self.format_label(label_id)));
  }

  /// Emit a copy/move instruction.
  pub fn emit_move(&mut self, src: X86Operand, dest: X86Operand) {
    assert!(
      !matches!(dest, X86Operand::Immediate(_)),
      "Invalid move to an immediate destination in x86 codegen."
    );

    let is_memory = |op: X86Operand| matches!(op, X86Operand::Stack(_) | X86Operand::Memory(_));

    if src != dest {
      if is_memory(src) && is_memory(dest) {
        self.instructions.push(X86Instr::Mov(
          src,
          X86Operand::Register(X86WReg::scratch(src.width())),
        ));
        self.instructions.push(X86Instr::Mov(
          X86Operand::Register(X86WReg::scratch(dest.width())),
          dest,
        ));
      } else {
        if matches!(
          (src, dest),
          (
            X86Operand::Immediate(Immediate { value: 0, .. }),
            X86Operand::Register(_)
          )
        ) {
          self.emit_binary_op(BinOp::Xor, Some(dest), Some(dest));
        } else {
          self.instructions.push(X86Instr::Mov(src, dest));
        }
      }
    }
  }

  /// Emit a compare instruction.
  pub fn emit_cmp(&mut self, src: X86Operand, dest: X86Operand) {
    self.instructions.push(X86Instr::Cmp(src, dest));
  }

  /// Emit a test instruction.
  pub fn emit_test(&mut self, src: X86Operand, dest: X86Operand) {
    self.instructions.push(X86Instr::Test(src, dest));
  }

  /// Emit a (void or non-void) return instruction.
  pub fn emit_return(&mut self, src: Option<X86Operand>) {
    if let Some(src) = src {
      self.emit_move(src, X86Operand::Register(X86WReg::ret(src.width())));
    }
    self
      .instructions
      .push(X86Instr::Jmp(format!("{}exit", self.label_prefix)));
  }

  /// Emit an unconditional jump instruction to destination label.
  pub fn emit_jump(&mut self, label_id: usize) {
    self
      .instructions
      .push(X86Instr::Jmp(self.format_label(label_id)));
  }

  /// Emit an unconditional jump to a trap.
  pub fn emit_trap_jump(&mut self, trap: Trap) {
    self
      .instructions
      .push(X86Instr::Jmp(trap.get_global_label()));
  }

  /// Emit a jump if predicate holds.
  pub fn emit_jump_if(&mut self, pred: X86Operand, label_id: usize) {
    self.emit_cmp(
      X86Operand::Immediate(Immediate {
        value: 0,
        width: pred.width(),
      }),
      pred,
    );
    self
      .instructions
      .push(X86Instr::Jne(self.format_label(label_id)));
  }

  /// Emit a jump to trap if predicate less than value.
  pub fn emit_trap_if_lesser(&mut self, pred: X86Operand, value: i64, trap: Trap) {
    if let X86Operand::Immediate(imm) = pred {
      if Self::signed_immediate_value(imm) < value {
        self.emit_trap_jump(trap);
      }
      return;
    }

    self.emit_cmp(
      X86Operand::Immediate(Immediate {
        value,
        width: pred.width(),
      }),
      pred,
    );
    self
      .instructions
      .push(X86Instr::Jl(trap.get_global_label()));
  }

  /// Emit a jump to trap if `pred` is greater-than-or-equal to `upper`.
  pub fn emit_trap_if_geq_bound(&mut self, pred: X86Operand, upper: X86Operand, trap: Trap) {
    let pred = if matches!(pred, X86Operand::Stack(_) | X86Operand::Memory(_))
      && matches!(upper, X86Operand::Stack(_) | X86Operand::Memory(_))
    {
      let scratch = X86Operand::Register(X86WReg::scratch(pred.width()));
      self.emit_move(pred, scratch);
      scratch
    } else {
      pred
    };

    self.emit_cmp(pred, upper);
    self
      .instructions
      .push(X86Instr::Jle(trap.get_global_label()));
  }

  /// Emit a jump to trap if predicate is greater than value.
  pub fn emit_trap_if_greater(&mut self, pred: X86Operand, value: i64, trap: Trap) {
    if let X86Operand::Immediate(imm) = pred {
      if Self::signed_immediate_value(imm) > value {
        self.emit_trap_jump(trap);
      }
      return;
    }

    self.emit_cmp(
      X86Operand::Immediate(Immediate {
        value,
        width: pred.width(),
      }),
      pred,
    );
    self
      .instructions
      .push(X86Instr::Jg(trap.get_global_label()));
  }

  /// Emit a jump to trap if predicate is zero.
  pub fn emit_trap_if_zero(&mut self, pred: X86Operand, trap: Trap) {
    if let X86Operand::Immediate(imm) = pred {
      if imm.value == 0 {
        self.emit_trap_jump(trap);
      }
      return;
    }

    self.emit_test(pred, pred);
    self
      .instructions
      .push(X86Instr::Je(trap.get_global_label()));
  }

  /// Emit an if-else conditional jump instruction block.
  pub fn emit_conditional(
    &mut self,
    pred: X86Operand,
    holds_label_id: usize,
    fails_label_id: usize,
  ) {
    self.emit_jump_if(pred, holds_label_id);
    self.emit_jump(fails_label_id);
  }

  /// Emit a unary operation on the destination operand.
  pub fn emit_unary_op(&mut self, op: UnOp, dest: X86Operand) {
    match op {
      UnOp::Neg => self.instructions.push(X86Instr::Neg(dest)),
      UnOp::Not => self.instructions.push(X86Instr::Not(dest)),
      UnOp::LNot => {
        self.emit_cmp(
          X86Operand::Immediate(Immediate {
            value: 0,
            width: dest.width(),
          }),
          dest,
        );
        self.instructions.push(X86Instr::Sete(dest));
      }
    }
  }

  /// Emit a binary operation from source to destination operand.
  /// Either of the operands may be implicit depending on the oepration.
  pub fn emit_binary_op(&mut self, op: BinOp, src: Option<X86Operand>, dest: Option<X86Operand>) {
    let is_32_bit = |value: i64| i32::try_from(value).is_ok();

    let op_instr = match op {
      BinOp::Add => {
        if matches!(src, Some(X86Operand::Immediate(Immediate { value: 0, .. }))) {
          return;
        } else {
          let src = src.unwrap();
          let dest = dest.unwrap();

          if let X86Operand::Immediate(Immediate { value, width }) = src
            && !is_32_bit(value)
          {
            let scratch = X86Operand::Register(X86WReg::scratch2(width));
            self.emit_move(src, scratch);
            X86Instr::Add(scratch, dest)
          } else {
            X86Instr::Add(src, dest)
          }
        }
      }
      BinOp::Sub => {
        if matches!(src, Some(X86Operand::Immediate(Immediate { value: 0, .. }))) {
          return;
        } else {
          let src = src.unwrap();
          let dest = dest.unwrap();

          if let X86Operand::Immediate(Immediate { value, width }) = src
            && !is_32_bit(value)
          {
            let scratch = X86Operand::Register(X86WReg::scratch2(width));
            self.emit_move(src, scratch);
            X86Instr::Sub(scratch, dest)
          } else {
            X86Instr::Sub(src, dest)
          }
        }
      }
      BinOp::Mul => match src.unwrap() {
        X86Operand::Immediate(imm) => {
          if matches!(imm, Immediate { value: 1, .. }) {
            return;
          } else if matches!(imm, Immediate { value: -1, .. }) {
            X86Instr::Neg(dest.unwrap())
          } else if matches!(imm, Immediate { value: 0, .. }) {
            self.emit_move(src.unwrap(), dest.unwrap());
            return;
          } else {
            X86Instr::IMul(Some(src.unwrap()), dest.unwrap(), dest.unwrap())
          }
        }
        _ => X86Instr::IMul(None, src.unwrap(), dest.unwrap()),
      },
      BinOp::Div | BinOp::Mod => {
        let src = src.unwrap();
        let src = if let X86Operand::Immediate(imm) = src {
          if matches!(imm, Immediate { value: 1, .. }) {
            if op == BinOp::Mod {
              self.emit_move(
                X86Operand::Immediate(Immediate {
                  value: 0,
                  width: src.width(),
                }),
                X86Operand::Register(X86WReg::modulo(src.width())),
              );
            }
            return;
          }
          let scratch = X86Operand::Register(X86WReg::scratch(src.width()));
          self.emit_move(src, scratch);
          scratch
        } else {
          src
        };
        self.instructions.push(X86Instr::Cqo(src.width()));
        X86Instr::IDiv(src)
      }
      BinOp::And | BinOp::LAnd => X86Instr::And(src.unwrap(), dest.unwrap()),
      BinOp::Xor => X86Instr::Xor(src.unwrap(), dest.unwrap()),
      BinOp::Or | BinOp::LOr => X86Instr::Or(src.unwrap(), dest.unwrap()),
      BinOp::Sal => X86Instr::Sal(src, dest.unwrap()),
      BinOp::Sar => X86Instr::Sar(src, dest.unwrap()),
      BinOp::CmpEq | BinOp::CmpNeq | BinOp::Gt | BinOp::Gte | BinOp::Lt | BinOp::Lte => {
        unreachable!("Use emit_binary_cmp for binary comparators like {op}.")
      }
    };

    self.instructions.push(op_instr);
  }

  /// Emit a binary comparison between `lhs` and `rhs`, storing the result in `dest`.
  pub fn emit_binary_cmp(&mut self, op: BinOp, lhs: X86Operand, rhs: X86Operand, dest: X86Operand) {
    self.emit_cmp(lhs, rhs);

    let dest = match dest {
      X86Operand::Register(wreg) => X86Operand::Register(X86WReg {
        register: wreg.register,
        width: W8,
      }),
      X86Operand::Stack(stack_var) => X86Operand::Stack(StackVar {
        offset: stack_var.offset,
        width: W8,
      }),
      X86Operand::Memory(_) | X86Operand::Immediate(_) => {
        unreachable!("Invalid function return operand in x86 codegen.")
      }
    };

    self.instructions.push(match op {
      BinOp::CmpEq => X86Instr::Sete(dest),
      BinOp::CmpNeq => X86Instr::Setne(dest),
      BinOp::Lt => X86Instr::Setl(dest),
      BinOp::Gt => X86Instr::Setg(dest),
      BinOp::Lte => X86Instr::Setle(dest),
      BinOp::Gte => X86Instr::Setge(dest),
      _ => unreachable!("Unknown binary comparator instruction {op}."),
    });
  }

  /// Emit instructions for a function call.
  pub fn emit_call(&mut self, dest: Option<X86Operand>, args: Vec<X86Operand>, name: String) {
    let return_reg = if let Some(dest) = dest {
      match dest {
        X86Operand::Register(wreg) => Some(wreg.register),
        _ => None,
      }
    } else {
      None
    };

    let arg_regs = X86Reg::call_argument();

    // get caller-saved registers to save
    let mut caller_saved = Vec::new();
    for arg in self.used_caller_saved.iter() {
      if return_reg != Some(*arg) {
        caller_saved.push(*arg);
      }
    }

    // calculate offset added to RSP in saving caller-saved and on-stack arguments
    let caller_saved_offset = caller_saved.len() * STACK_SLOT_WIDTH;
    let args_offset = if args.len() > arg_regs.len() {
      (args.len() - arg_regs.len()) * STACK_SLOT_WIDTH
    } else {
      0
    };
    let call_offset = caller_saved_offset
      + args_offset
      + if !(caller_saved_offset + args_offset).is_multiple_of(STACK_ALIGNMENT) {
        STACK_SLOT_WIDTH
      } else {
        0
      };

    // allocate memory on stack for caller-saved registers and arguments
    self.emit_binary_op(
      BinOp::Sub,
      Some(X86Operand::Immediate(Immediate {
        value: call_offset as i64,
        width: W64,
      })),
      Some(X86Operand::Register(X86WReg::stack_pointer())),
    );

    // save caller-saved
    for (index, &register) in caller_saved.iter().enumerate() {
      self.emit_move(
        X86Operand::Register(X86WReg {
          register,
          width: W64,
        }),
        X86Operand::Stack(StackVar {
          offset: (args_offset + index * STACK_SLOT_WIDTH) as i64,
          width: W64,
        }),
      );
    }

    // place arguments
    for (index, &arg) in args.iter().enumerate() {
      if index < arg_regs.len() {
        let src = match arg {
          X86Operand::Stack(stack_var) => X86Operand::Stack(StackVar {
            offset: call_offset as i64 + stack_var.offset,
            width: arg.width(),
          }),
          X86Operand::Register(wreg) => {
            // check need for potential parallel move resolution
            if let Some(reg_index) = caller_saved.iter().position(|&reg| reg == wreg.register)
              && reg_index != index
            {
              X86Operand::Stack(StackVar {
                offset: (args_offset + reg_index * STACK_SLOT_WIDTH) as i64,
                width: arg.width(),
              })
            } else {
              arg
            }
          }
          X86Operand::Immediate(_) => arg,
          X86Operand::Memory(mem_var) => {
            let scratch = X86Operand::Register(X86WReg::scratch(mem_var.width));
            self.emit_move(arg, scratch);
            scratch
          }
        };

        self.emit_move(
          src,
          X86Operand::Register(X86WReg {
            register: arg_regs[index],
            width: arg.width(),
          }),
        );
      } else {
        self.emit_move(
          match arg {
            X86Operand::Stack(stack_var) => X86Operand::Stack(StackVar {
              offset: call_offset as i64 + stack_var.offset,
              width: arg.width(),
            }),
            _ => arg,
          },
          X86Operand::Stack(StackVar {
            offset: ((index - arg_regs.len()) * STACK_SLOT_WIDTH) as i64,
            width: arg.width(),
          }),
        )
      }
    }

    // make the call and save the return value (if any)
    self.instructions.push(X86Instr::Call(name));
    if let Some(dest) = dest {
      match dest {
        X86Operand::Stack(stack_var) => self.emit_move(
          X86Operand::Register(X86WReg::ret(dest.width())),
          X86Operand::Stack(StackVar {
            offset: stack_var.offset + call_offset as i64,
            width: dest.width(),
          }),
        ),
        X86Operand::Register(_) => {
          self.emit_move(X86Operand::Register(X86WReg::ret(dest.width())), dest)
        }
        X86Operand::Immediate(_) | X86Operand::Memory(_) => {
          unreachable!("Invalid function return operand in x86 codegen.")
        }
      }
    }

    // restore caller-saved
    for (index, &register) in caller_saved.iter().enumerate() {
      self.emit_move(
        X86Operand::Stack(StackVar {
          offset: (args_offset + index * STACK_SLOT_WIDTH) as i64,
          width: W64,
        }),
        X86Operand::Register(X86WReg {
          register,
          width: W64,
        }),
      );
    }

    // clear stack allocation for caller-saved and arguments
    self.emit_binary_op(
      BinOp::Add,
      Some(X86Operand::Immediate(Immediate {
        value: call_offset as i64,
        width: W64,
      })),
      Some(X86Operand::Register(X86WReg::stack_pointer())),
    );
  }

  /// Emit instructions for a tail call.
  /// Returns `false` is cannot emit tail call.
  pub fn emit_tail_call(&mut self, args: Vec<X86Operand>, name: String) -> bool {
    let arg_regs = X86Reg::call_argument();

    if args.len() > arg_regs.len()
      || args
        .iter()
        .any(|arg| matches!(arg, X86Operand::Register(wreg) if arg_regs.contains(&wreg.register)))
    {
      return false;
    }

    for (index, &arg) in args.iter().enumerate() {
      self.emit_move(
        arg,
        X86Operand::Register(X86WReg {
          register: arg_regs[index],
          width: arg.width(),
        }),
      );
    }

    if self.frame_size > 0 {
      self.instructions.push(X86Instr::Add(
        X86Operand::Immediate(Immediate {
          value: self.frame_size as i64,
          width: W64,
        }),
        X86Operand::Register(X86WReg::stack_pointer()),
      ));
    }

    for (index, &register) in self.used_callee_saved.iter().enumerate() {
      self.instructions.push(X86Instr::Mov(
        X86Operand::Stack(StackVar {
          offset: -(((index + 1) * STACK_SLOT_WIDTH) as i64),
          width: W64,
        }),
        X86Operand::Register(X86WReg {
          register,
          width: W64,
        }),
      ))
    }

    self.instructions.push(X86Instr::Jmp(name));
    true
  }

  /// Generate final assembly for this context.
  pub fn assemble(&mut self) -> Vec<X86Instr> {
    let mut assembly: Vec<X86Instr> = Vec::new();

    // append program prologue and body
    assembly.append(&mut self.instructions);

    // add exit label
    assembly.push(X86Instr::Label(format!("{}exit", self.label_prefix)));

    // deallocate stack space
    if self.frame_size > 0 {
      assembly.push(X86Instr::Add(
        X86Operand::Immediate(Immediate {
          value: self.frame_size as i64,
          width: W64,
        }),
        X86Operand::Register(X86WReg::stack_pointer()),
      ));
    }

    // restore callee-saved registers
    for (index, &register) in self.used_callee_saved.iter().enumerate() {
      assembly.push(X86Instr::Mov(
        X86Operand::Stack(StackVar {
          offset: -(((index + 1) * STACK_SLOT_WIDTH) as i64),
          width: W64,
        }),
        X86Operand::Register(X86WReg {
          register,
          width: W64,
        }),
      ));
    }

    // add the return instruction
    assembly.push(X86Instr::Ret);

    assembly
  }

  /// Helper to transform label index to label string.
  fn format_label(&self, label_id: usize) -> String {
    format!("{}{label_id}", self.label_prefix)
  }

  /// Helper to get casted immediates.
  fn signed_immediate_value(imm: Immediate) -> i64 {
    match imm.width {
      W8 => (imm.value as i8) as i64,
      W16 => (imm.value as i16) as i64,
      W32 => (imm.value as i32) as i64,
      W64 => imm.value,
    }
  }
}

/// Get the bit-width for fundamental C0 types.
fn width_for_type(typ: &Typ) -> Width {
  match typ {
    Typ::Bool => W8,
    Typ::Char => W8,
    Typ::Int => W32,
    Typ::Null | Typ::Pointer(..) | Typ::Array(..) => W64,
    Typ::Void | Typ::Typedef(_) | Typ::Struct(_) => {
      unreachable!("Bad width evaluation in x86 codegen for type {typ}.")
    }
  }
}
