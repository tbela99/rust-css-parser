//! Port of `src/validation/utils/list.ts`.

use crate::ast::arena::AstArena;
use crate::ast::types::EnumToken;

/// remove comma and comment tokens
pub fn strip_comma_token(arena: &AstArena, token_list: &[u32]) -> Vec<u32> {
    token_list
        .iter()
        .copied()
        .filter(|&t| !matches!(arena.data_types.get(t), EnumToken::CommentTokenType | EnumToken::CommaTokenType))
        .collect()
}

/// split a token list. `split` defaults to `[CommaTokenType]`
pub fn split_token_list(arena: &AstArena, token_list: &[u32], split: &[u8], include_split_token: bool) -> Vec<Vec<u32>> {
    let mut result: Vec<Vec<u32>> = vec![Vec::new()];
    for &curr in token_list {
        if split.contains(&arena.data_types.get(curr)) {
            if include_split_token {
                result.last_mut().unwrap().push(curr);
            }
            result.push(Vec::new());
        } else {
            result.last_mut().unwrap().push(curr);
        }
    }
    result
}

/// `splitTokenList(arena, tokens)` with the default comma separator
pub fn split_token_list_comma(arena: &AstArena, token_list: &[u32]) -> Vec<Vec<u32>> {
    split_token_list(arena, token_list, &[EnumToken::CommaTokenType], false)
}
