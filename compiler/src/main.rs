#![allow(unused)]

mod args;
mod front;

use logos::Logos;
use std::fs;

use crate::front::{lexer::Token, parser::parse};

fn main() {
  let config = args::parse_args();

  let header_str = if let Some(header_file) = config.header {
    fs::read_to_string(header_file).expect("Could not read header file")
  } else {
    "".to_string()
  };
  let file_str = fs::read_to_string(config.source.unwrap()).expect("Could not read source file");

  // 1. Lexical analysis
  let header_token_stream = Token::lexer(&header_str)
    .spanned()
    .map(|(t, y)| (y.start, t.expect("Badly formatted header code"), y.end));
  let src_token_stream = Token::lexer(&file_str)
    .spanned()
    .map(|(t, y)| (y.start, t.expect("Badly formatted source code"), y.end));

  // 2. Syntactic analysis
  let header_ast = match parse(header_token_stream) {
    Ok(header_ast) => header_ast,
    Err(e) => {
      eprintln!("Syntax error in header file. {e}");
      return;
    }
  };
  let src_ast = match parse(src_token_stream) {
    Ok(src_ast) => src_ast,
    Err(e) => {
      eprintln!("Syntax error in source file. {e}");
      return;
    }
  };
}
