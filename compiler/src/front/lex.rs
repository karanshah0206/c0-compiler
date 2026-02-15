use std::fmt::{Display, Formatter, Result};

use logos::Logos;

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

  lex.bump(bytes.len());
  logos::Skip
}

#[derive(Debug, PartialEq, Logos)]
#[logos(skip r"[ \n\r\t\f\v]+")]
pub enum Token<'a> {
  #[token("(")]
  LParan,
  #[token(")")]
  RParan,
  #[token("{")]
  LBrace,
  #[token("}")]
  RBrace,

  #[token("~")]
  Tilde,
  #[token(":")]
  Colon,
  #[token(";")]
  Semicolon,
  #[token("?")]
  Question,

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

  #[token("int")]
  Int,
  #[token("bool")]
  Bool,

  #[regex("[A-Za-z][A-Za-z0-9_]*")]
  Ident(&'a str),

  #[regex(r"0[xX][0-9a-fA-F]+", |lex| {
    let slice = lex.slice();
    i64::from_str_radix(&slice[2..], 16).ok()
  })]
  #[regex(r"[0-9]+", |lex| lex.slice().parse::<i64>().ok())]
  Number(i64),
  #[token("true")]
  True,
  #[token("false")]
  False,

  #[regex(r"//[^\n]*", logos::skip, allow_greedy = true)]
  #[token("/*", skip_block_comment)]
  Comment,
}

impl Display for Token<'_> {
  fn fmt(&self, fmt: &mut Formatter<'_>) -> Result {
    write!(fmt, "{:#?}", self)
  }
}
