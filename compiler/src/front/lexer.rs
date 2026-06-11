use std::fmt::{Display, Formatter, Result};

use logos::Logos;

/// Lexical tokens for the C0 language.
#[derive(Clone, Debug, PartialEq, Logos)]
#[logos(skip r"[ \n\r\t\f\v]+")]
pub enum Token<'a> {
  #[token("(")]
  LParen,
  #[token(")")]
  RParen,
  #[token("{")]
  LBrace,
  #[token("}")]
  RBrace,
  #[token("[")]
  LBracket,
  #[token("]")]
  RBracket,
  #[regex(r"\[\s*\]")]
  #[regex(r"\[(\s*(//)[^\n]*\s*)+\]")]
  LBracketRBracket,

  #[token("~")]
  Tilde,
  #[token(":")]
  Colon,
  #[token(",")]
  Comma,
  #[token(";")]
  Semicolon,
  #[token("?")]
  Question,

  #[token(".")]
  Period,
  #[token("->")]
  Arrow,

  #[token("=")]
  Equal,
  #[token("==")]
  EqualEqual,

  #[token("!")]
  Bang,
  #[token("!=")]
  BangEqual,

  #[token("-")]
  Hyphen,
  #[token("--")]
  HyphenHyphen,
  #[token("-=")]
  HyphenEqual,

  #[token("+")]
  Plus,
  #[token("++")]
  PlusPlus,
  #[token("+=")]
  PlusEqual,

  #[token("*")]
  Asterisk,
  #[token("*=")]
  AsteriskEqual,

  #[token("/")]
  FSlash,
  #[token("/=")]
  FSlashEqual,

  #[token("%")]
  Percent,
  #[token("%=")]
  PercentEqual,

  #[token("&")]
  Ampersand,
  #[token("&&")]
  AmpersandAmpersand,
  #[token("&=")]
  AmpersandEqual,

  #[token("^")]
  Caret,
  #[token("^=")]
  CaretEqual,

  #[token("|")]
  Pipe,
  #[token("||")]
  PipePipe,
  #[token("|=")]
  PipeEqual,

  #[token("<")]
  Lt,
  #[token("<<")]
  LtLt,
  #[token("<=")]
  LtEqual,
  #[token("<<=")]
  LtLtEqual,

  #[token(">")]
  Gt,
  #[token(">>")]
  GtGt,
  #[token(">=")]
  GtEqual,
  #[token(">>=")]
  GtGtEqual,

  #[token("if")]
  If,
  #[token("else")]
  Else,
  #[token("while")]
  While,
  #[token("for")]
  For,
  #[token("return")]
  Return,
  #[token("assert")]
  Assert,

  #[token("int")]
  Int,
  #[token("bool")]
  Bool,
  #[token("char")]
  Char,
  #[token("void")]
  Void,
  #[token("typedef")]
  Typedef,
  #[token("struct")]
  Struct,

  #[token("alloc")]
  Alloc,
  #[token("alloc_array")]
  AllocArray,

  #[regex("[A-Za-z_][A-Za-z0-9_]*")]
  Ident(&'a str),

  #[regex(r"0[xX][0-9a-fA-F]+", |lex| {
    let slice = lex.slice();
    let res = i64::from_str_radix(&slice[2..], 16).expect("Failed to parse hexadecimal constant.");
    assert!(res <= 0xffffffff, "Hexadecimal constant is out of bounds.");
    res
  })]
  #[regex(r"0|[1-9][0-9]*", |lex| {
    let res = lex.slice().parse::<i64>().expect("Failed to parse decimal constant.");
    assert!(res <= i64::from(i32::MIN).abs(), "Decimal constant is out of bounds.");
    res
  })]
  Number(i64),
  #[token("true")]
  True,
  #[token("false")]
  False,
  #[regex(r##"'(?:[^'\\]|\\['\"\\ntrfabv0])'"##, parse_char_literal)]
  CharLit(i8),
  #[token("NULL")]
  Null,

  #[regex(r"//[^\n]*", logos::skip, allow_greedy = true)]
  #[token("/*", skip_block_comment)]
  Comment,

  // keywords reserved for future use
  #[token("break")]
  #[token("continue")]
  #[token("string")]
  Error,
}

impl Display for Token<'_> {
  fn fmt(&self, fmt: &mut Formatter<'_>) -> Result {
    write!(fmt, "{:#?}", self)
  }
}

/// Parse a character literal token (e.g. `'a'`, `'\n'`).
fn parse_char_literal<'a>(lex: &mut logos::Lexer<'a, Token<'a>>) -> i8 {
  let slice = lex.slice();
  let inner = &slice[1..slice.len() - 1];

  if inner.len() == 1 {
    let byte = inner.as_bytes()[0];
    assert!(
      (32..=126).contains(&byte),
      "Character literal must be a printable ASCII character or an escape sequence."
    );
    return byte as i8;
  }

  assert!(
    inner.starts_with('\\'),
    "Invalid character literal {slice}."
  );
  match inner.as_bytes()[1] {
    b't' => 9,
    b'r' => 13,
    b'f' => 12,
    b'a' => 7,
    b'b' => 8,
    b'n' => 10,
    b'v' => 11,
    b'\'' => 39,
    b'"' => 34,
    b'0' => 0,
    b'\\' => 92,
    escaped => unreachable!("Invalid character escape \\{escaped} in literal {slice}."),
  }
}

/// Helper method to skip (potentially nested) multi-line comments.
fn skip_block_comment<'a>(lex: &mut logos::Lexer<'a, Token<'a>>) -> logos::Skip {
  let mut depth = 1;
  let mut idx = 0;
  let remainder = lex.remainder();
  let bytes = remainder.as_bytes();

  while idx + 1 < bytes.len() {
    if bytes[idx] == b'/' && bytes[idx + 1] == b'*' {
      depth += 1;
      idx += 2;
      continue;
    }

    if bytes[idx] == b'*' && bytes[idx + 1] == b'/' {
      depth -= 1;
      idx += 2;
      if depth == 0 {
        lex.bump(idx);
        return logos::Skip;
      }
      continue;
    }

    idx += 1;
  }

  assert!(depth == 0, "Unclosed block comments.");

  lex.bump(bytes.len());
  logos::Skip
}
