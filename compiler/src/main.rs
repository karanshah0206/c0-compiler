mod args;
mod front;

use logos::Logos;
use std::fs;

use crate::front::{lexer::Token, parser::parse};

fn main() {
  let config = args::parse_args();
  let file_str = fs::read_to_string(config.file.unwrap()).expect("Could not read file");

  // 1. Lexical analysis
  let token_stream = Token::lexer(&file_str)
    .spanned()
    .map(|(t, y)| (y.start, t.expect("Badly formatted source code."), y.end));

  // 2. Syntactic analysis
  let program = parse(token_stream);
}
