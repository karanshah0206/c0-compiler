use crate::front::{ast::Program, c0, lexer::Token};

/// Invoke the generated LR(1) parser on the token stream coming from the lexer.
pub fn parse<'a>(
  token_stream: impl Iterator<Item = (usize, Token<'a>, usize)>,
) -> Result<Program, String> {
  c0::ProgramParser::new()
    .parse(token_stream)
    .map_err(|e| format!("Couldn't parse faile. Failed with message {e:?}"))
}
