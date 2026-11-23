//! Helper of the color ports (no typescript counterpart): the `xxxToken(arena, values)` functions of the color spaces
//! create the same color tokens.

use crate::ast::arena::{AstArena, TokenValue};
use crate::ast::types::EnumToken;

/// number token
pub fn number(arena: &AstArena, v: f64) -> u32 {
    arena.create_token(EnumToken::NumberTokenType, TokenValue::Num(v), None)
}

/// percentage token
pub fn percentage(arena: &AstArena, v: f64) -> u32 {
    arena.create_token(EnumToken::PercentageTokenType, TokenValue::Num(v), None)
}

/// `/` literal token
pub fn slash(arena: &AstArena) -> u32 {
    arena.create_token(EnumToken::LiteralTokenType, TokenValue::Str("/"), None)
}

/// `[number, number, number]` + `/ alpha%` when `alpha` is set
pub fn numbers_with_alpha(arena: &AstArena, values: &[f64], with_alpha: bool) -> Vec<u32> {
    let at = |i: usize| values.get(i).copied().unwrap_or(f64::NAN);
    let mut chi = vec![number(arena, at(0)), number(arena, at(1)), number(arena, at(2))];
    if with_alpha {
        chi.push(slash(arena));
        chi.push(percentage(arena, at(3) * 100.0));
    }
    chi
}
