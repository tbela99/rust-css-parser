//! Port of `src/parser/utils/token.ts`.

use crate::ast::arena::AstArena;
use crate::ast::types::EnumToken;
use crate::syntax::constants::is_function_type;

fn trim_token_space(t: u8) -> bool {
    matches!(
        t,
        EnumToken::CommaTokenType
            | EnumToken::DelimTokenType
            | EnumToken::GtTokenType
            | EnumToken::GteTokenType
            | EnumToken::LtTokenType
            | EnumToken::LteTokenType
            | EnumToken::ChildCombinatorTokenType
            | EnumToken::PipeTokenType
            | EnumToken::ColonTokenType
    )
}

fn trim_space_after(t: u8) -> bool {
    is_function_type(t)
        || matches!(
            t,
            EnumToken::ChildCombinatorTokenType
                | EnumToken::SupportsQueryConditionTokenType
                | EnumToken::SupportsQueryUnaryConditionTokenType
                | EnumToken::CommaTokenType
                | EnumToken::PipeTokenType
        )
}

/// remove leading, trailing and redundant whitespace tokens (in place)
pub fn trim_white_space_tokens(arena: &AstArena, tokens: &mut Vec<u32>) {
    let ws = |t: u32| arena.data_types.get(t) == EnumToken::WhitespaceTokenType;
    if tokens.first().is_some_and(|&t| ws(t)) {
        tokens.remove(0);
    }
    if tokens.last().is_some_and(|&t| ws(t)) {
        tokens.pop();
    }
    if tokens.len() == 1 {
        if ws(tokens[0]) {
            tokens.clear();
        }
        return;
    }
    let mut i: isize = 0;
    while (i as usize) < tokens.len() {
        let iu = i as usize;
        let t = arena.data_types.get(tokens[iu]);
        if trim_token_space(t) {
            if iu + 1 < tokens.len() && ws(tokens[iu + 1]) {
                tokens.remove(iu + 1);
            }
            if iu > 0 && ws(tokens[iu - 1]) {
                i -= 1;
                tokens.remove(i as usize);
                i += 1;
                continue;
            }
        }
        let iu = i as usize;
        if trim_space_after(arena.data_types.get(tokens[iu])) && iu + 1 < tokens.len() && ws(tokens[iu + 1]) {
            tokens.remove(iu + 1);
        }
        i += 1;
    }
}
