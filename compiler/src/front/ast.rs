use std::fmt::{Display, Error, Formatter};

/// Program is a list of global declarations.
pub type Program = Vec<GlobalDeclaration>;

/// Named identity.
pub type Ident = String;

/// Function Parameter (type, named identity).
pub type Param = (Typ, Ident);

/// Primitive types supported by the language (and typedefs).
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum Typ {
  Void,
  Int,
  Bool,
  Typedef(Ident),
}

/// Global declaration in the program.
pub enum GlobalDeclaration {
  /// Function declaration (type, identifier, parameters)
  FDecl(Typ, Ident, Vec<Param>),
  /// Function definition (type, identifier, parameters, body)
  FDefn(Typ, Ident, Vec<Param>, Stmt),
  /// Type definition (underlying type, alias)
  Typedef(Typ, Ident),
}

/// Local statement.
pub enum Stmt {
  /// Variable declaration
  Decl(Typ, Ident),
  /// Variable definition
  Defn(Typ, Ident, Expr),
  /// Variable assignment
  Asgn(Ident, AsnOp, Expr),
  /// Post-operator (++/--
  PostOp(Ident, PostOp),
  /// Couple ordered statements
  Seq(Box<Stmt>, Box<Stmt>),
  /// Conditional (bool expression, if-branch, else-branch)
  Cond(Expr, Box<Stmt>, Box<Stmt>),
  /// While loop (bool expression, loop body)
  While(Expr, Box<Stmt>),
  /// For loop (init, boolean expr, step, body)
  For(Box<Option<Stmt>>, Expr, Box<Option<Stmt>>, Box<Stmt>),
  /// Block of ordered statements
  Block(Vec<Stmt>),
  /// Return
  Ret(Option<Expr>),
  /// Standalone expression
  Expr(Expr),
  /// Assertion (bool expression)
  Assert(Expr),
  /// No operation/empty statement
  NoOp(),
}

#[derive(Clone)]
/// Expression tree.
/// For non-immediates, type is determined during semantic analysis. Parser should set type to `None`.
pub enum Expr {
  /// Numeric immediate
  Number(i32),
  /// Boolean immediate
  Bool(bool),
  /// Variable identifier
  Variable(Ident, Option<Typ>),
  /// Binary operation (lhs, operator, rhs, type)
  Binop(Box<Expr>, BinOp, Box<Expr>, Option<Typ>),
  /// Unary operator (operator, operand, type)
  Unop(UnOp, Box<Expr>, Option<Typ>),
  /// Ternary operator (boolean expr, if-expr, else-expr, type)
  Ternop(Box<Expr>, Box<Expr>, Box<Expr>, Option<Typ>),
  /// Function call (identifier, arguments list, type)
  Call(Ident, Vec<Box<Expr>>, Option<Typ>),
}

impl Expr {
  /// Get the type produced as result of computing this expression.
  pub fn get_type(&self) -> Option<Typ> {
    use Expr::*;

    match self {
      Number(_) => Some(Typ::Int),
      Bool(_) => Some(Typ::Bool),
      Variable(_, typ)
      | Binop(_, _, _, typ)
      | Unop(_, _, typ)
      | Ternop(_, _, _, typ)
      | Call(_, _, typ) => typ.clone(),
    }
  }
}

/// Unary operators.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum UnOp {
  /// Arithmetic negation
  Neg,
  /// Bitwise NOT
  Not,
  /// Logical NOT
  LNot,
}

#[derive(Copy, Clone, PartialEq, Eq)]
/// Binary operators.
pub enum BinOp {
  /// Arithmetic add (+)
  Add,
  /// Arithmetic subtract (-)
  Sub,
  /// Arithmetic multiply (*)
  Mul,
  /// Arithmetic divide (/)
  Div,
  /// Arithmetic remainder/modulo (%)
  Mod,
  /// Bitwise AND (&)
  And,
  /// Bitwise Exclusive OR (^)
  Xor,
  /// Bitwise OR (|)
  Or,
  /// Arithmetic left-shift (<<)
  Sal,
  /// Arithmetic right-shift (>>)
  Sar,
  /// Logical AND (&&)
  LAnd,
  /// Logical OR (||)
  LOr,
  /// Compare equality (==)
  CmpEq,
  /// Compare inequality (!=)
  CmpNeq,
  /// Compare less-than (<)
  Lt,
  /// Compare greater-than (>)
  Gt,
  /// Compare less-than-equal (<=)
  Lte,
  /// Compare greater-than-equal (>=)
  Gte,
}

#[derive(Copy, Clone, PartialEq, Eq)]
/// Post-operators.
pub enum PostOp {
  /// Post-increment (++)
  Inc,
  /// Post-decrement (--)
  Dec,
}

impl PostOp {
  /// Transform post-operator to binary operator.
  pub fn to_binop(&self) -> BinOp {
    use BinOp::*;

    match self {
      PostOp::Inc => Add,
      PostOp::Dec => Sub,
    }
  }
}

#[derive(Copy, Clone, PartialEq, Eq)]
/// Assignment operators.
pub enum AsnOp {
  /// =
  Equal,
  /// +=
  Plus,
  /// -=
  Minus,
  /// *=
  Times,
  /// /=
  Div,
  /// %=
  Mod,
  /// &=
  And,
  /// ^=
  Xor,
  /// |=
  Or,
  /// <<=
  Sal,
  /// \>>=
  Sar,
}

impl AsnOp {
  /// Try transforming assignment operator to binary operator.
  pub fn to_binop(&self) -> Option<BinOp> {
    use BinOp::*;

    match self {
      AsnOp::Equal => None,
      AsnOp::Plus => Some(Add),
      AsnOp::Minus => Some(Sub),
      AsnOp::Times => Some(Mul),
      AsnOp::Div => Some(Div),
      AsnOp::Mod => Some(Mod),
      AsnOp::And => Some(And),
      AsnOp::Xor => Some(Xor),
      AsnOp::Or => Some(Or),
      AsnOp::Sal => Some(Sal),
      AsnOp::Sar => Some(Sar),
    }
  }
}

impl Display for Typ {
  fn fmt(&self, fmt: &mut Formatter) -> Result<(), Error> {
    use Typ::*;

    match self {
      Void => write!(fmt, "void"),
      Int => write!(fmt, "int"),
      Bool => write!(fmt, "bool"),
      Typedef(id) => write!(fmt, "{id}"),
    }
  }
}

impl Display for UnOp {
  fn fmt(&self, fmt: &mut Formatter) -> Result<(), Error> {
    use UnOp::*;

    match *self {
      Neg => write!(fmt, "-"),
      Not => write!(fmt, "~"),
      LNot => write!(fmt, "!"),
    }
  }
}

impl Display for PostOp {
  fn fmt(&self, fmt: &mut Formatter) -> Result<(), Error> {
    use PostOp::*;

    match *self {
      Inc => write!(fmt, "++"),
      Dec => write!(fmt, "--"),
    }
  }
}

impl Display for BinOp {
  fn fmt(&self, fmt: &mut Formatter) -> Result<(), Error> {
    use BinOp::*;

    match *self {
      Add => write!(fmt, "+"),
      Sub => write!(fmt, "-"),
      Mul => write!(fmt, "*"),
      Div => write!(fmt, "/"),
      Mod => write!(fmt, "%"),
      And => write!(fmt, "&"),
      Xor => write!(fmt, "^"),
      Or => write!(fmt, "|"),
      Sal => write!(fmt, "<<"),
      Sar => write!(fmt, ">>"),
      LAnd => write!(fmt, "&&"),
      LOr => write!(fmt, "||"),
      CmpEq => write!(fmt, "=="),
      CmpNeq => write!(fmt, "!="),
      Lt => write!(fmt, "<"),
      Gt => write!(fmt, ">"),
      Lte => write!(fmt, "<="),
      Gte => write!(fmt, ">="),
    }
  }
}

impl Display for AsnOp {
  fn fmt(&self, fmt: &mut Formatter) -> Result<(), Error> {
    use AsnOp::*;

    match *self {
      Equal => write!(fmt, "="),
      Times => write!(fmt, "*="),
      Div => write!(fmt, "/="),
      Mod => write!(fmt, "%="),
      Plus => write!(fmt, "+="),
      Minus => write!(fmt, "-="),
      And => write!(fmt, "&="),
      Xor => write!(fmt, "^="),
      Or => write!(fmt, "|="),
      Sal => write!(fmt, "<<="),
      Sar => write!(fmt, ">>="),
    }
  }
}
