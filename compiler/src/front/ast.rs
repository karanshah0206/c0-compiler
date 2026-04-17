use std::fmt::{Display, Error, Formatter};

/// Top-level AST is a list of global declarations.
pub type ProgramAST = Vec<GlobalDeclaration>;

/// An identifier is a string.
pub type Ident = String;

/// A variable is a (type, identifier) tuple.
pub type Variable = (Typ, Ident);

/// Primitive types and typedefs.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Typ {
  Void,
  Int,
  Bool,
  Typedef(Ident),
}

/// Global declaration in the program.
pub enum GlobalDeclaration {
  /// Function declaration (type, identifier, parameters)
  FDecl(Typ, Ident, Vec<Variable>),
  /// Function definition (type, identifier, parameters, body)
  FDefn(Typ, Ident, Vec<Variable>, Stmt),
  /// Type definition (underlying type, alias)
  Typedef(Typ, Ident),
}

/// Local statement.
pub enum Stmt {
  /// Variable declaration
  Decl(Variable),
  /// Variable definition
  Defn(Variable, Expr),
  /// Variable assignment
  Asgn(Ident, AsnOp, Expr),
  /// Post-operator (++/--
  PostOp(Ident, PostOp),
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

/// Expression tree.
/// For non-immediates, type is determined during semantic analysis. Parser should set type to `None`.
#[derive(Clone)]
pub enum Expr {
  /// Numeric immediate
  Number(i64),
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
  Call(Ident, Vec<Expr>, Option<Typ>),
}

impl Expr {
  /// Get the type produced as result of computing this expression.
  pub fn get_type(&self) -> Typ {
    use Expr::*;

    match self {
      Number(_) => Typ::Int,
      Bool(_) => Typ::Bool,
      Variable(_, typ)
      | Binop(_, _, _, typ)
      | Unop(_, _, typ)
      | Ternop(_, _, _, typ)
      | Call(_, _, typ) => typ.clone().unwrap_or(Typ::Void),
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

/// Binary operators.
#[derive(Copy, Clone, PartialEq, Eq)]
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

/// Post-operators.
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum PostOp {
  /// Post-increment (++)
  Inc,
  /// Post-decrement (--)
  Dec,
}

impl PostOp {
  /// Transform post-operator to binary operator.
  pub fn to_binop(self) -> BinOp {
    use BinOp::*;

    match self {
      PostOp::Inc => Add,
      PostOp::Dec => Sub,
    }
  }
}

/// Assignment operators.
#[derive(Copy, Clone, PartialEq, Eq)]
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
  pub fn to_binop(self) -> Option<BinOp> {
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

// Implementing display for types in AST, useful when compiler is passed the `--dump-ast` flag.

impl Display for GlobalDeclaration {
  fn fmt(&self, fmt: &mut Formatter) -> Result<(), Error> {
    match self {
      GlobalDeclaration::FDecl(_, _, _) | GlobalDeclaration::Typedef(_, _) => write!(fmt, ""),
      GlobalDeclaration::FDefn(typ, id, params, body) => {
        let params = params
          .iter()
          .map(|(param_typ, param_id)| format!("({param_typ}, \"{param_id}\")"))
          .collect::<Vec<_>>()
          .join(", ");

        if params.is_empty() {
          write!(fmt, "{typ} {id}():\n{body}")
        } else {
          write!(fmt, "{typ} {id}({params}):\n{body}")
        }
      }
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

impl Display for Stmt {
  fn fmt(&self, fmt: &mut Formatter) -> Result<(), Error> {
    fn fmt_opt_stmt(stmt: &Option<Stmt>) -> String {
      match stmt {
        Some(stmt) => format!("{stmt}"),
        None => "".to_string(),
      }
    }

    match self {
      Stmt::Decl((typ, id)) => write!(fmt, "Decl({typ}, \"{id}\")"),
      Stmt::Defn((typ, id), expr) => write!(fmt, "Defn({typ}, \"{id}\", {expr})"),
      Stmt::Asgn(id, asn_op, expr) => write!(fmt, "Asgn(\"{id}\", {asn_op}, {expr})"),
      Stmt::PostOp(id, post_op) => write!(fmt, "PostOp(\"{id}\", {post_op})"),
      Stmt::Cond(expr, stmt, stmt1) => write!(fmt, "Cond({expr}, {stmt}, {stmt1})"),
      Stmt::While(expr, stmt) => write!(fmt, "While({expr}, {stmt})"),
      Stmt::For(stmt, expr, stmt1, stmt2) => {
        write!(
          fmt,
          "For({}, {expr}, {}, {stmt2})",
          fmt_opt_stmt(stmt.as_ref()),
          fmt_opt_stmt(stmt1.as_ref())
        )
      }
      Stmt::Block(stmts) => {
        let rendered = stmts
          .iter()
          .map(|stmt| format!("{stmt}"))
          .collect::<Vec<_>>()
          .join(", ");
        write!(fmt, "Block([{rendered}])")
      }
      Stmt::Ret(expr) => match expr {
        Some(expr) => write!(fmt, "Ret({expr})"),
        None => write!(fmt, "Ret(void)"),
      },
      Stmt::Expr(expr) => write!(fmt, "Expr({expr})"),
      Stmt::Assert(expr) => write!(fmt, "Assert({expr})"),
      Stmt::NoOp() => write!(fmt, "NoOp"),
    }
  }
}

impl Display for Expr {
  fn fmt(&self, fmt: &mut Formatter) -> Result<(), Error> {
    fn fmt_opt_typ(typ: &Option<Typ>) -> String {
      match typ {
        Some(typ) => format!("{typ}"),
        None => "void".to_string(),
      }
    }

    match self {
      Expr::Number(value) => write!(fmt, "Number({value})"),
      Expr::Bool(value) => write!(fmt, "Bool({value})"),
      Expr::Variable(id, typ) => write!(fmt, "Variable(\"{id}\", {})", fmt_opt_typ(typ)),
      Expr::Binop(lhs, op, rhs, typ) => {
        write!(fmt, "Binop({lhs}, {op}, {rhs}, {})", fmt_opt_typ(typ))
      }
      Expr::Unop(op, rhs, typ) => write!(fmt, "Unop({op}, {rhs}, {})", fmt_opt_typ(typ)),
      Expr::Ternop(cond, if_expr, else_expr, typ) => {
        write!(
          fmt,
          "Ternop({cond}, {if_expr}, {else_expr}, {})",
          fmt_opt_typ(typ)
        )
      }
      Expr::Call(id, args, typ) => {
        let rendered_args = args
          .iter()
          .map(|arg| format!("{arg}"))
          .collect::<Vec<_>>()
          .join(", ");
        write!(
          fmt,
          "Call(\"{id}\", [{rendered_args}], {})",
          fmt_opt_typ(typ)
        )
      }
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
