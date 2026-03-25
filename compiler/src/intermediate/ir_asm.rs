use std::fmt::{Display, Error, Formatter};

use crate::front::ast::{BinOp, Ident, Typ, UnOp};

/// Temporary (compiler-generated value store) is an (id, type) tuple.
pub type Temp = (usize, Typ);

/// Operand to an operator.
#[derive(Clone)]
pub enum Operand {
  /// Immediate
  Const(i32),
  /// Temporary
  Temp(Temp),
}

/// Runtime exception.
#[derive(Clone)]
pub enum Exception {
  /// Forceful termination exception
  Abort,
}

/// Label identified by a unique integer.
#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub struct Label(pub usize);

/// Instructions supported by the three-address IR.
#[derive(Clone)]
pub enum Instr {
  /// Binary operator
  BinOp {
    op: BinOp,
    dest: Temp,
    lhs: Operand,
    rhs: Operand,
  },
  /// Unary operator
  UnOp { op: UnOp, dest: Temp, src: Operand },
  /// Label (block-start delimiter)
  Label(Label),
  /// Jump to a block
  JumpTo(Label),
  /// Conditional jump
  JumpIf {
    pred: Operand,
    holds: Label,
    fails: Option<Label>,
  },
  /// Function call
  Call {
    dest: Option<Temp>,
    name: Ident,
    args: Vec<Operand>,
  },
  /// Return from a function
  Return(Option<Operand>),
  /// Throw a runtime exception
  Throw(Exception),
  /// Phi node for control-flow merges
  Phi {
    dest: Temp,                  // temp that stores the resolved value
    srcs: Vec<(Label, Operand)>, // sources (with scope) from where to resolve the dest
  },
  /// Copy value from source operand to dest temporary
  Move { dest: Temp, src: Operand },
}

// Implementing display for types in AST, useful when compiler is passed the `--dump-ir` flag.

impl Display for Operand {
  fn fmt(&self, fmt: &mut Formatter) -> Result<(), Error> {
    match self {
      Operand::Const(imm) => write!(fmt, "${imm}"),
      Operand::Temp((id, _)) => write!(fmt, "T{id}"),
    }
  }
}

impl Display for Exception {
  fn fmt(&self, fmt: &mut Formatter) -> Result<(), Error> {
    match self {
      Exception::Abort => write!(fmt, "ABORT"),
    }
  }
}

impl Display for Label {
  fn fmt(&self, fmt: &mut Formatter) -> Result<(), Error> {
    write!(fmt, ".L{}", self.0)
  }
}

impl Display for Instr {
  fn fmt(&self, fmt: &mut Formatter) -> Result<(), Error> {
    // helper to print comma-separated function args
    let display_args = |args: &Vec<Operand>| {
      args
        .iter()
        .map(|arg| arg.to_string())
        .collect::<Vec<_>>()
        .join(", ")
    };

    match self {
      Instr::BinOp { op, dest, lhs, rhs } => write!(fmt, "T{} <- {lhs} {op} {rhs}", dest.0),
      Instr::UnOp { op, dest, src } => write!(fmt, "T{} <- {op}{src}", dest.0),
      Instr::Label(label) => write!(fmt, "{label}"),
      Instr::JumpTo(l) => write!(fmt, "JUMP {l}"),
      Instr::JumpIf { pred, holds, fails } => match fails {
        Some(fails) => write!(fmt, "IF {pred} JUMP {holds} ELSE {fails}"),
        None => write!(fmt, "IF {pred} JUMP {holds}"),
      },
      Instr::Call { dest, name, args } => match dest {
        Some(dest) => write!(fmt, "T{} <- CALL {name}({})", dest.0, display_args(args)),
        None => write!(fmt, "CALL {name}({})", display_args(args)),
      },
      Instr::Return(o) => match o {
        Some(o) => write!(fmt, "RETURN {o}"),
        None => write!(fmt, "RETURN"),
      },
      Instr::Throw(e) => write!(fmt, "{e}"),
      Instr::Phi { dest, srcs } => {
        write!(
          fmt,
          "T{} <- PHI |{}|",
          dest.0,
          srcs
            .iter()
            .map(|(l, op)| format!("{l} : {op}"))
            .collect::<Vec<_>>()
            .join(", ")
        )
      }
      Instr::Move { dest, src } => write!(fmt, "T{} <- {src}", dest.0),
    }
  }
}
