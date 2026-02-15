mod args;
mod front;

use logos::Logos;
use std::fs;

use crate::front::lex::Token;

fn main() {
  let config = args::parse_args();

  // Testing the lexical analyzer
  let file_name = config.file.unwrap();
  let file_str = fs::read_to_string(file_name).expect("Could not read file");
  let token_stream = Token::lexer(&file_str).spanned().map(|(t, y)| (y.start, t, y.end));

  for token in token_stream {
    if let Err(_) = token.1 {
      println!("Lexer failed");
    } else {
      println!("{} {} {}", token.0, token.1.unwrap(), token.2);
    }
  }
}
