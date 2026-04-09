use std::collections::{HashMap, HashSet};

use crate::front::ast::{BinOp, Typ, UnOp};
use crate::intermediate::ir_asm::{Operand, Temp};
use crate::x86_back::{
  regalloc::*,
  x86_asm::{Width::*, *},
};

/// Potential kinds of traps that can be called in the program.
pub enum Trap {
  /// Explicit abort
  Abort,
  /// Memory error
  MemError,
}

impl Trap {
  /// Get the global label for a trap.
  fn get_global_label(self) -> String {
    match self {
      Trap::Abort => ".L_abort",
      Trap::MemError => ".L_memerror",
    }
    .to_string()
  }
}

/// x86-64 code generation context.
pub struct X86Context {
  /// Generated assembly instructions in this context.
  instructions: Vec<X86Instr>,
  /// Set of callee-saved registers used in this context.
  used_callee_saved: HashSet<X86Reg>,
  /// Register allocation for temporaries in this context.
  regalloc: Vec<Color>,
  /// Stack allocation for operands in this context.
  stack_allocation: HashMap<usize, StackVar>,
  /// Stack depth (in bytes) added in this context.
  stack_depth: usize,
  /// Parameter types for the function in this context.
  param_types: Vec<Typ>,
  /// Prefix for labels created in this context.
  label_prefix: String,
}

impl X86Context {
  /// Generate a new x86-64 code generation context for a function.
  pub fn new(regalloc: Vec<Color>, param_types: Vec<Typ>, label_prefix: String) -> Self {
    X86Context {
      instructions: Vec::new(),
      used_callee_saved: HashSet::new(),
      regalloc,
      stack_allocation: HashMap::new(),
      stack_depth: 0,
      param_types,
      label_prefix,
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

  /// Get concrete location assigned to a compile-time temporary.
  pub fn get_temp_location(&mut self, temp: Temp) -> X86Operand {
    let color = *self.regalloc.get(temp.0).expect(&format!(
      "Unknown temporary with id {} found in x86 codegen.",
      temp.0
    ));

    if color == SPILL {
      X86Operand::Stack(
        self
          .stack_allocation
          .get(&temp.0)
          .expect(&format!(
            "Missing stack allocation for temporary with id {} in x86 codegen.",
            temp.0
          ))
          .clone(),
      )
    } else {
      X86Operand::Register(X86WReg {
        register: color_to_register(color),
        width: width_for_type(&temp.1),
      })
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
      self.instructions.push(X86Instr::Mov(src, dest));
    }
  }

  /// Emit a compare instruction.
  pub fn emit_cmp(&mut self, src: X86Operand, dest: X86Operand) {
    self.instructions.push(X86Instr::Cmp(src, dest));
  }

  /// Emit a (void or non-void) return instruction.
  pub fn emit_return(&mut self, src: Option<X86Operand>) {
    if let Some(src) = src {
      self.emit_move(src.clone(), X86Operand::Register(X86WReg::ret(src.width())));
    }
    self.instructions.push(X86Instr::Ret);
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

  /// Emit an if-else conditional jump instruction block.
  pub fn emit_conditional(
    &mut self,
    pred: X86Operand,
    holds_label_id: usize,
    fails_label_id: usize,
  ) {
    self.emit_cmp(
      pred.clone(),
      X86Operand::Immediate(Immediate {
        value: 0,
        width: pred.width(),
      }),
    );
    self
      .instructions
      .push(X86Instr::Jne(self.format_label(holds_label_id)));
    self.emit_jump(fails_label_id);
  }

  /// Emit a unary operation on the destination operand.
  pub fn emit_unary_operation(&mut self, op: UnOp, dest: X86Operand) {
    match op {
      UnOp::Neg => self.instructions.push(X86Instr::Neg(dest)),
      UnOp::Not | UnOp::LNot => self.instructions.push(X86Instr::Not(dest)),
    }
  }

  pub fn emit_binary_operation(&mut self, op: BinOp, src: X86Operand, dest: X86Operand) {}

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

    self.emit_save_caller_saved(return_reg);
    self.emit_call_args_placement(&args);
    self.instructions.push(X86Instr::Call(name));
    if let Some(dest) = dest {
      self.emit_move(X86Operand::Register(X86WReg::ret(dest.width())), dest);
    }
    if args.len() > 6 {
      self.instructions.push(X86Instr::Sub(
        X86Operand::Immediate(Immediate {
          value: ((args.len() - 6) * 8) as i64,
          width: W64,
        }),
        X86Operand::Register(X86WReg::stack_pointer()),
      ));
    }
    self.emit_restore_caller_saved(return_reg);
  }

  /// Emit instructions to save caller-saved registers.
  fn emit_save_caller_saved(&mut self, return_reg: Option<X86Reg>) {
    let mut caller_saved = X86Reg::caller_saved();
    if let Some(return_reg) = return_reg {
      caller_saved.retain(|&reg| reg != return_reg);
    }
    for register in caller_saved {
      self
        .instructions
        .push(X86Instr::Push(X86Operand::Register(X86WReg {
          register,
          width: W64,
        })));
    }
  }

  /// Emit instructions to restore caller-saved registers.
  fn emit_restore_caller_saved(&mut self, return_reg: Option<X86Reg>) {
    let mut caller_saved = X86Reg::caller_saved();
    if let Some(return_reg) = return_reg {
      caller_saved.retain(|&reg| reg != return_reg);
    }
    for &register in caller_saved.iter().rev() {
      self
        .instructions
        .push(X86Instr::Pop(X86Operand::Register(X86WReg {
          register,
          width: W64,
        })))
    }
  }

  /// Emit instructions to place args according to System V ABI for function calls.
  fn emit_call_args_placement(&mut self, args: &Vec<X86Operand>) {
    for (index, &register) in X86Reg::call_argument().iter().enumerate() {
      if index >= args.len() {
        return;
      }
      let arg = args[index].clone();
      let width = arg.width();
      self.emit_move(arg, X86Operand::Register(X86WReg { register, width }));
    }

    for arg in args.iter().skip(6).rev() {
      let arg = match arg {
        X86Operand::Register(wreg) => X86Operand::Register(X86WReg {
          register: wreg.register,
          width: W64,
        }),
        X86Operand::Immediate(imm) => X86Operand::Immediate(Immediate {
          value: imm.value,
          width: W64,
        }),
        X86Operand::Stack(_) => *arg,
      };

      self.instructions.push(X86Instr::Push(arg));
    }
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
