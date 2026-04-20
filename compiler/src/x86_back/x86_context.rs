use std::collections::{BTreeSet, HashMap};

use crate::front::ast::{BinOp, Typ, UnOp};
use crate::intermediate::ir_asm::{Operand, Temp};
use crate::x86_back::{
  regalloc::*,
  x86_asm::{Width::*, *},
};

/// Width in bytes for a slot on stack allotted to a temporary.
const STACK_SLOT_WIDTH: usize = 8;
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
        value: 12,
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
  /// Stack depth (in bytes) added in this context.
  stack_depth: usize,
  /// Prefix for labels created in this context.
  label_prefix: String,
}

impl X86Context {
  /// Generate a new x86-64 code generation context for a function.
  pub fn new(regalloc: Vec<Color>, params_count: usize, label_prefix: String) -> Self {
    let mut ctx = X86Context {
      instructions: Vec::new(),
      used_callee_saved: BTreeSet::new(),
      used_caller_saved: BTreeSet::new(),
      regalloc,
      stack_allocation: HashMap::new(),
      stack_depth: 0,
      label_prefix,
    };

    // determine stack space for spilt temporaries
    for (temp_id, &color) in ctx.regalloc.iter().enumerate() {
      if color == SPILL && temp_id >= params_count {
        ctx.stack_allocation.insert(
          temp_id,
          StackVar {
            offset: ctx.stack_depth,
            width: W64,
          },
        );
        ctx.stack_depth += STACK_SLOT_WIDTH;
      }
    }

    // calculate function frame size to get offsets for arguments on stack
    let alignment = ctx.stack_depth % STACK_ALIGNMENT;
    let frame_size = ctx.stack_depth
      + if alignment != 8 {
        if alignment < 8 {
          8 - alignment
        } else {
          24 - alignment
        }
      } else {
        0
      };

    // calculate offsets for function params on the stack
    for (index, temp_id) in (X86Reg::call_argument().len()..params_count).enumerate() {
      ctx.stack_allocation.insert(
        temp_id,
        StackVar {
          offset: frame_size + (index + 1) * STACK_SLOT_WIDTH,
          width: W64,
        },
      );
    }

    // treat caller-saved argument registers are defined
    let arg_regs = X86Reg::call_argument();
    let caller_saved_regs = X86Reg::caller_saved();
    for index in 0..params_count.min(arg_regs.len()) {
      if caller_saved_regs.contains(&arg_regs[index]) {
        ctx.used_caller_saved.insert(arg_regs[index]);
      }
    }

    ctx
  }

  /// Get concrete location assigned to a compile-time temporary.
  pub fn get_temp_location(&mut self, temp: Temp) -> X86Operand {
    let color = *self
      .regalloc
      .get(temp.0)
      .unwrap_or_else(|| panic!("Unknown temporary with id {} found in x86 codegen.", temp.0));

    if color == SPILL {
      X86Operand::Stack(
        self
          .stack_allocation
          .get(&temp.0)
          .unwrap_or_else(|| {
            panic!(
              "Missing stack allocation for temporary with id {} in x86 codegen.",
              temp.0
            )
          })
          .as_width(width_for_type(&temp.1)),
      )
    } else {
      let register = color_to_register(color);
      if X86Reg::callee_saved().contains(&register) {
        self.used_callee_saved.insert(register);
      } else if X86Reg::caller_saved().contains(&register) {
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

    if src != dest {
      if matches!((src, dest), (X86Operand::Stack(_), X86Operand::Stack(_))) {
        self.instructions.push(X86Instr::Mov(
          src,
          X86Operand::Register(X86WReg::scratch(src.width())),
        ));
        self.instructions.push(X86Instr::Mov(
          X86Operand::Register(X86WReg::scratch(dest.width())),
          dest,
        ));
      } else {
        self.instructions.push(X86Instr::Mov(src, dest));
      }
    }
  }

  /// Emit a compare instruction.
  pub fn emit_cmp(&mut self, src: X86Operand, dest: X86Operand) {
    self.instructions.push(X86Instr::Cmp(src, dest));
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

  /// Emit a jump if predicate less than value.
  pub fn emit_trap_if_lesser(&mut self, pred: X86Operand, value: i64, trap: Trap) {
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

  /// Emit a jump if predicate is greater than value.
  pub fn emit_trap_if_greater(&mut self, pred: X86Operand, value: i64, trap: Trap) {
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

  /// Push an operand onto the stack. May clobber the scratch register if `src` is on memory.
  pub fn emit_push(&mut self, src: X86Operand) {
    let src = if src.width() == W64 {
      src
    } else {
      match src {
        X86Operand::Register(wreg) => X86Operand::Register(X86WReg {
          register: wreg.register,
          width: W64,
        }),
        X86Operand::Stack(_) => {
          self.emit_move(src, X86Operand::Register(X86WReg::scratch(src.width())));
          X86Operand::Register(X86WReg::scratch(W64))
        }
        X86Operand::Immediate(imm) => X86Operand::Immediate(Immediate {
          value: imm.value,
          width: W64,
        }),
      }
    };
    self.instructions.push(X86Instr::Push(src));
  }

  /// Pop an operand from the stack. May clobber the scratch register if `dest` is on memory.
  pub fn emit_pop(&mut self, dest: X86Operand) {
    match dest {
      X86Operand::Register(wreg) => {
        self
          .instructions
          .push(X86Instr::Pop(X86Operand::Register(X86WReg {
            register: wreg.register,
            width: W64,
          })));
      }
      X86Operand::Stack(stack_var) => {
        if stack_var.width == W64 {
          self.instructions.push(X86Instr::Pop(dest));
        } else {
          self
            .instructions
            .push(X86Instr::Pop(X86Operand::Register(X86WReg::scratch(W64))));
          self.emit_move(
            X86Operand::Register(X86WReg::scratch(stack_var.width)),
            dest,
          );
        }
      }
      X86Operand::Immediate(_) => unreachable!("Illegal pop instruction to an immediate."),
    };
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
    let op_instr = match op {
      BinOp::Add => X86Instr::Add(src.unwrap(), dest.unwrap()),
      BinOp::Sub => X86Instr::Sub(src.unwrap(), dest.unwrap()),
      BinOp::Mul => match src.unwrap() {
        X86Operand::Immediate(_) => {
          X86Instr::IMul(Some(src.unwrap()), dest.unwrap(), dest.unwrap())
        }
        _ => X86Instr::IMul(None, src.unwrap(), dest.unwrap()),
      },
      BinOp::Div | BinOp::Mod => {
        let src = src.unwrap();
        let src = if let X86Operand::Immediate(_) = src {
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
      X86Operand::Immediate(_) => {
        unreachable!("Dest cannot be an immediate for binary comparator operations.")
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
      + if (caller_saved_offset + args_offset) % STACK_ALIGNMENT != 0 {
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
          offset: args_offset + index * STACK_SLOT_WIDTH,
          width: W64,
        }),
      );
    }

    // place arguments
    for (index, &arg) in args.iter().enumerate() {
      if index < arg_regs.len() {
        self.emit_move(
          match arg {
            X86Operand::Stack(stack_var) => X86Operand::Stack(StackVar {
              offset: call_offset + stack_var.offset,
              width: arg.width(),
            }),
            _ => arg,
          },
          X86Operand::Register(X86WReg {
            register: arg_regs[index],
            width: arg.width(),
          }),
        );
      } else {
        self.emit_move(
          match arg {
            X86Operand::Stack(stack_var) => X86Operand::Stack(StackVar {
              offset: call_offset + stack_var.offset,
              width: arg.width(),
            }),
            _ => arg,
          },
          X86Operand::Stack(StackVar {
            offset: (index - arg_regs.len()) * STACK_SLOT_WIDTH,
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
            offset: stack_var.offset + call_offset,
            width: dest.width(),
          }),
        ),
        X86Operand::Register(_) => {
          self.emit_move(X86Operand::Register(X86WReg::ret(dest.width())), dest)
        }
        X86Operand::Immediate(_) => {
          unreachable!("Destination of a function return value cannot be an immediate.")
        }
      }
    }

    // restore caller-saved
    for (index, &register) in caller_saved.iter().enumerate() {
      self.emit_move(
        X86Operand::Stack(StackVar {
          offset: args_offset + index * STACK_SLOT_WIDTH,
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

  /// Generate final assembly for this context.
  pub fn assemble(&mut self) -> Vec<X86Instr> {
    let mut assembly: Vec<X86Instr> = Vec::new();

    // Save callee-saved registers
    for &register in self.used_callee_saved.iter() {
      assembly.push(X86Instr::Push(X86Operand::Register(X86WReg {
        register,
        width: W64,
      })));
    }

    // Allocate and align stack slots for spilled temporaries
    let alignment =
      (self.used_callee_saved.len() * STACK_SLOT_WIDTH + self.stack_depth) % STACK_ALIGNMENT;
    let frame_size = self.stack_depth
      + if alignment != 8 {
        if alignment < 8 {
          8 - alignment
        } else {
          24 - alignment
        }
      } else {
        0
      };
    if frame_size > 0 {
      assembly.push(X86Instr::Sub(
        X86Operand::Immediate(Immediate {
          value: frame_size as i64,
          width: W64,
        }),
        X86Operand::Register(X86WReg::stack_pointer()),
      ));
    }

    // Append program body
    assembly.append(&mut self.instructions);

    // Add exit label
    assembly.push(X86Instr::Label(format!("{}exit", self.label_prefix)));

    // Deallocate stack space
    if frame_size > 0 {
      assembly.push(X86Instr::Add(
        X86Operand::Immediate(Immediate {
          value: frame_size as i64,
          width: W64,
        }),
        X86Operand::Register(X86WReg::stack_pointer()),
      ));
    }

    // Restore callee-saved registers
    for &register in self.used_callee_saved.iter().rev() {
      assembly.push(X86Instr::Pop(X86Operand::Register(X86WReg {
        register,
        width: W64,
      })));
    }

    // Add the return instruction
    assembly.push(X86Instr::Ret);

    assembly
  }

  /// Helper to transform label index to label string.
  fn format_label(&self, label_id: usize) -> String {
    format!("{}{label_id}", self.label_prefix)
  }
}

/// Get the bit-width for fundamental C0 types.
fn width_for_type(typ: &Typ) -> Width {
  match typ {
    Typ::Int => W32,
    Typ::Bool => W8,
    Typ::Void | Typ::Typedef(_) => {
      unreachable!("Bad width evaluation in x86 codegen for type {typ}.")
    }
  }
}
