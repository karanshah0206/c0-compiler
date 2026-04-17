use std::fmt::{Display, Error, Formatter};

/// X86-64 general-purpose registers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum X86Reg {
  Rax,
  Rbx,
  Rcx,
  Rdx,
  Rdi,
  Rsi,
  R8,
  R9,
  R10,
  R11,
  R12,
  R13,
  R14,
  R15,
  Rbp,
  Rsp,
}
use X86Reg::*;

impl X86Reg {
  /// Registers that can be allocated to temporaries.
  /// Position in array corresponds to color for register allocation.
  pub fn allocatable() -> Vec<Self> {
    vec![
      Rax, Rbx, Rcx, Rdx, Rdi, Rsi, R8, R9, R11, R12, R13, R14, R15, Rbp,
    ]
  }

  /// Non-volatile registers.
  pub fn callee_saved() -> Vec<Self> {
    vec![Rbx, R12, R13, R14, R15, Rbp]
  }

  /// Volatile/call-clobbered registers.
  pub fn caller_saved() -> Vec<Self> {
    vec![Rax, Rcx, Rdx, Rdi, Rsi, R8, R9, R10, R11]
  }

  /// Function argument registers in order.
  pub fn call_argument() -> Vec<Self> {
    vec![Rdi, Rsi, Rdx, Rcx, R8, R9]
  }
}

/// Supported bit-widths for x86-64 operands.
#[derive(Clone, Copy, PartialEq)]
pub enum Width {
  /// 8-bit operand
  W8,
  /// 16-bit operand
  W16,
  /// 32-bit operand
  W32,
  /// 64-bit operand
  W64,
}
use Width::*;

/// x86-64 general-purpose registers with width.
#[derive(Clone, Copy, PartialEq)]
pub struct X86WReg {
  /// General-purpose register to use as operand.
  pub register: X86Reg,
  /// Width under which to evaluate the operand.
  pub width: Width,
}

impl X86WReg {
  /// Register that stores quotient from `idiv`.
  pub fn quotient(width: Width) -> Self {
    X86WReg {
      register: Rax,
      width,
    }
  }

  /// Register that stores remainder from `idiv`.
  pub fn modulo(width: Width) -> Self {
    X86WReg {
      register: Rdx,
      width,
    }
  }

  /// Register that stores the shift magnitude.
  pub fn shift() -> Self {
    X86WReg {
      register: Rcx,
      width: W8,
    }
  }

  /// Register that stores function return value.
  pub fn ret(width: Width) -> Self {
    X86WReg {
      register: Rax,
      width,
    }
  }

  /// Register that stores the current depth on stack.
  pub fn stack_pointer() -> Self {
    X86WReg {
      register: Rsp,
      width: W64,
    }
  }

  /// Register reserved exclusively for scratch/swap.
  pub fn scratch(width: Width) -> Self {
    X86WReg {
      register: R10,
      width,
    }
  }
}

/// Immediate/numeric literal operand.
#[derive(Clone, Copy, PartialEq)]
pub struct Immediate {
  /// Concrete value of the operand.
  pub value: i64,
  /// Target width under which to evaluate the operand.
  pub width: Width,
}

/// Temporary stored on stack.
#[derive(Clone, Copy, PartialEq)]
pub struct StackVar {
  /// Offset from stack pointer.
  pub offset: usize,
  /// Temporary width.
  pub width: Width,
}

impl StackVar {
  /// Return stack variable of custom width with identical offset.
  pub fn as_width(&self, width: Width) -> Self {
    StackVar {
      offset: self.offset,
      width,
    }
  }
}

/// Operand to x86-64 instructions.
#[derive(Clone, Copy, PartialEq)]
pub enum X86Operand {
  /// Operand stored on a register.
  Register(X86WReg),
  /// Operand stored on the stack.
  Stack(StackVar),
  /// Operand that is an immediate/compile-time constant.
  Immediate(Immediate),
}
use X86Operand::*;

impl X86Operand {
  /// Get width of an x86-64 assembly operand.
  pub fn width(&self) -> Width {
    match self {
      Register(register) => register.width,
      Stack(stack_var) => stack_var.width,
      Immediate(immediate) => immediate.width,
    }
  }
}

/// x86-64 assembly instructions.
#[derive(Clone)]
pub enum X86Instr {
  /// label
  Label(String),
  /// mov src, dest
  Mov(X86Operand, X86Operand),
  /// add src, dest
  Add(X86Operand, X86Operand),
  /// sub src, dest
  Sub(X86Operand, X86Operand),
  /// imul (imm,)? src, dest
  IMul(Option<X86Operand>, X86Operand, X86Operand),
  /// idiv src
  IDiv(X86Operand),
  /// and src, dest
  And(X86Operand, X86Operand),
  /// xor src, dest
  Xor(X86Operand, X86Operand),
  /// or src, dest
  Or(X86Operand, X86Operand),
  /// sal (imm,)? dest
  Sal(Option<X86Operand>, X86Operand),
  /// sar (imm,)? dest
  Sar(Option<X86Operand>, X86Operand),
  /// not dest
  Not(X86Operand),
  /// neg dest
  Neg(X86Operand),
  /// cwd
  Cqo(Width),
  /// cmp src, dest
  Cmp(X86Operand, X86Operand),
  /// sete dest
  Sete(X86Operand),
  /// setne dest
  Setne(X86Operand),
  /// setl dest
  Setl(X86Operand),
  /// setg dest
  Setg(X86Operand),
  /// setle dest
  Setle(X86Operand),
  /// setge dest
  Setge(X86Operand),
  /// push src
  Push(X86Operand),
  /// pop dest
  Pop(X86Operand),
  /// jmp label
  Jmp(String),
  /// jne label
  Jne(String),
  /// jl label
  Jl(String),
  // jg label
  Jg(String),
  /// call `function_name`
  Call(String),
  /// ret
  Ret,
}
use X86Instr::*;

// Implementing display for types in x86-64 assembly, useful when emitting assembly code.

impl Display for X86WReg {
  fn fmt(&self, fmt: &mut Formatter) -> Result<(), Error> {
    write!(
      fmt,
      "{}",
      match (self.register, self.width) {
        (Rax, W8) => "al",
        (Rax, W16) => "ax",
        (Rax, W32) => "eax",
        (Rax, W64) => "rax",
        (Rbx, W8) => "bl",
        (Rbx, W16) => "bx",
        (Rbx, W32) => "ebx",
        (Rbx, W64) => "rbx",
        (Rcx, W8) => "cl",
        (Rcx, W16) => "cx",
        (Rcx, W32) => "ecx",
        (Rcx, W64) => "rcx",
        (Rdx, W8) => "dl",
        (Rdx, W16) => "dx",
        (Rdx, W32) => "edx",
        (Rdx, W64) => "rdx",
        (Rdi, W8) => "dil",
        (Rdi, W16) => "di",
        (Rdi, W32) => "edi",
        (Rdi, W64) => "rdi",
        (Rsi, W8) => "sil",
        (Rsi, W16) => "si",
        (Rsi, W32) => "esi",
        (Rsi, W64) => "rsi",
        (R8, W8) => "r8b",
        (R8, W16) => "r8w",
        (R8, W32) => "r8d",
        (R8, W64) => "r8",
        (R9, W8) => "r9b",
        (R9, W16) => "r9w",
        (R9, W32) => "r9d",
        (R9, W64) => "r9",
        (R10, W8) => "r10b",
        (R10, W16) => "r10w",
        (R10, W32) => "r10d",
        (R10, W64) => "r10",
        (R11, W8) => "r11b",
        (R11, W16) => "r11w",
        (R11, W32) => "r11d",
        (R11, W64) => "r11",
        (R12, W8) => "r12b",
        (R12, W16) => "r12w",
        (R12, W32) => "r12d",
        (R12, W64) => "r12",
        (R13, W8) => "r13b",
        (R13, W16) => "r13w",
        (R13, W32) => "r13d",
        (R13, W64) => "r13",
        (R14, W8) => "r14b",
        (R14, W16) => "r14w",
        (R14, W32) => "r14d",
        (R14, W64) => "r14",
        (R15, W8) => "r15b",
        (R15, W16) => "r15w",
        (R15, W32) => "r15d",
        (R15, W64) => "r15",
        (Rbp, W8) => "bpl",
        (Rbp, W16) => "bp",
        (Rbp, W32) => "ebp",
        (Rbp, W64) => "rbp",
        (Rsp, W8) => "spl",
        (Rsp, W16) => "sp",
        (Rsp, W32) => "esp",
        (Rsp, W64) => "rsp",
      }
    )
  }
}

impl Display for X86Operand {
  fn fmt(&self, fmt: &mut Formatter) -> Result<(), Error> {
    match self {
      Register(register) => write!(fmt, "%{register}"),
      Stack(stack_var) => write!(fmt, "{}(%{})", stack_var.offset, X86WReg::stack_pointer()),
      Immediate(immediate) => write!(fmt, "${}", immediate.value),
    }
  }
}

impl Display for X86Instr {
  fn fmt(&self, fmt: &mut Formatter) -> Result<(), Error> {
    let suf = |width: Width| match width {
      W8 => "b",
      W16 => "w",
      W32 => "l",
      W64 => "q",
    };

    match self {
      Label(label) => write!(fmt, "{label}:"),
      Mov(s, d) => write!(fmt, "\tmov{}\t{s}, {d}", suf(d.width())),
      Add(s, d) => write!(fmt, "\tadd{}\t{s}, {d}", suf(d.width())),
      Sub(s, d) => write!(fmt, "\tsub{}\t{s}, {d}", suf(d.width())),
      IMul(i, s, d) => match i {
        Some(i) => write!(fmt, "\timul{}\t{i}, {s}, {d}", suf(d.width())),
        None => write!(fmt, "\timul{}\t{s}, {d}", suf(d.width())),
      },
      IDiv(s) => write!(fmt, "\tidiv{}\t{s}", suf(s.width())),
      And(s, d) => write!(fmt, "\tand{}\t{s}, {d}", suf(d.width())),
      Xor(s, d) => write!(fmt, "\txor{}\t{s}, {d}", suf(d.width())),
      Or(s, d) => write!(fmt, "\tor{}\t{s}, {d}", suf(d.width())),
      Sal(s, d) => match s {
        Some(i) => write!(fmt, "\tsal{}\t{i}, {d}", suf(d.width())),
        None => write!(
          fmt,
          "\tsal{}\t{}, {d}",
          suf(d.width()),
          Register(X86WReg::shift())
        ),
      },
      Sar(s, d) => match s {
        Some(i) => write!(fmt, "\tsar{}\t{i}, {d}", suf(d.width())),
        None => write!(
          fmt,
          "\tsar{}\t{}, {d}",
          suf(d.width()),
          Register(X86WReg::shift())
        ),
      },
      Not(s) => write!(fmt, "\tnot{}\t{s}", suf(s.width())),
      Neg(s) => write!(fmt, "\tneg{}\t{s}", suf(s.width())),
      Cqo(w) => match w {
        W8 => write!(fmt, "\tcbw"),
        W16 => write!(fmt, "\tcwd"),
        W32 => write!(fmt, "\tcdq"),
        W64 => write!(fmt, "\tcqo"),
      },
      Cmp(s, d) => write!(fmt, "\tcmp{}\t{s}, {d}", suf(d.width())),
      Sete(d) => write!(fmt, "\tsete\t{d}"),
      Setne(d) => write!(fmt, "\tsetne\t{d}"),
      Setl(d) => write!(fmt, "\tsetl\t{d}"),
      Setg(d) => write!(fmt, "\tsetg\t{d}"),
      Setle(d) => write!(fmt, "\tsetle\t{d}"),
      Setge(d) => write!(fmt, "\tsetge\t{d}"),
      Push(s) => write!(fmt, "\tpush{}\t{s}", suf(s.width())),
      Pop(d) => write!(fmt, "\tpop{}\t{d}", suf(d.width())),
      Jmp(label) => write!(fmt, "\tjmp\t{label}"),
      Jne(label) => write!(fmt, "\tjne\t{label}"),
      Jl(label) => write!(fmt, "\tjl\t{label}"),
      Jg(label) => write!(fmt, "\tjg\t{label}"),
      Call(name) => write!(fmt, "\tcall\t{name}"),
      Ret => write!(fmt, "\tret"),
    }
  }
}
