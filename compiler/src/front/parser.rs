use crate::front::lexer::Token;

pub fn parse<'a>(token_stream: impl Iterator<Item = (usize, Result<Token<'a>, ()>, usize)>) {}
