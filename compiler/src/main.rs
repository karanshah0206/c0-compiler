#![allow(unused)]

mod args;
mod common;
mod front;

use std::{fs, process, thread};

use logos::Logos;

use crate::front::{lexer::Token, parser::parse, semantics};

fn main() {
  let config = args::parse_args();

  let compiler_thread = thread::Builder::new()
    .stack_size(256 * 1024 * 1024)
    .spawn(move || {
      let header_str = if let Some(header_file) = config.header {
        fs::read_to_string(header_file).expect("Could not read header file.")
      } else {
        "".to_string()
      };
      let source_str =
        fs::read_to_string(config.source.unwrap()).expect("Could not read source file.");

      // 1. Lexical analysis
      let header_token_stream = Token::lexer(&header_str)
        .spanned()
        .map(|(t, y)| (y.start, t.expect("Badly formatted header code."), y.end));
      let source_token_stream = Token::lexer(&source_str)
        .spanned()
        .map(|(t, y)| (y.start, t.expect("Badly formatted source code."), y.end));

      // 2. Syntactic analysis
      let mut header_ast = match parse(header_token_stream) {
        Ok(header_ast) => header_ast,
        Err(e) => {
          eprintln!("Syntax error in header file. {e}");
          return 1;
        }
      };
      let mut source_ast = match parse(source_token_stream) {
        Ok(source_ast) => source_ast,
        Err(e) => {
          eprintln!("Syntax error in source file. {e}");
          return 1;
        }
      };

      // 3. Semantic analysis
      let symbol_table = semantics::analyze_program(&mut header_ast, &mut source_ast);

      return 0;
    })
    .unwrap();

  process::exit(compiler_thread.join().expect("Compiler failed."))
}
