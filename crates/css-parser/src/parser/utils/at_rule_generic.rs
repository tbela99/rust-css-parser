//! Port of `src/parser/utils/at-rule-generic.ts`.

use crate::ast::arena::AstArena;
use crate::ast::types::EnumToken;
use crate::options::ParserOptions;
use crate::syntax::constants::is_function_def_type;
use crate::types::ErrorDescription;
use crate::validation::match_::{drop_error, token_type_name};

/// validate the `and` / `or` / `,` structure of a generic at-rule prelude
pub fn match_generic_syntax(arena: &AstArena, stream: &[u32], options: &ParserOptions) -> (bool, Vec<ErrorDescription>) {
    let t = |x: u32| arena.data_types.get(x);
    let mut stack: Vec<u32> = Vec::new();
    let mut success = true;
    let mut expect_and_or = false;
    let mut expect_comma = false;
    let mut errors: Vec<ErrorDescription> = Vec::new();
    let mut scopes: Vec<(bool, bool)> = vec![(false, false)];
    let unexpected = |token: u32| drop_error(arena, format!("unexpected token {}", token_type_name(t(token))), token, options);
    for &token in stream {
        let tt = t(token);
        if tt == EnumToken::WhitespaceTokenType || tt == EnumToken::CommentTokenType {
            continue;
        }
        if tt == EnumToken::StartParensTokenType || is_function_def_type(tt) {
            stack.push(token);
            scopes.push((false, false));
            expect_and_or = false;
            continue;
        }
        if tt == EnumToken::EndParensTokenType {
            let top = stack.last().map(|&s| t(s));
            if top.is_none_or(|s| s != EnumToken::StartParensTokenType && !is_function_def_type(s)) {
                errors.push(unexpected(token));
                success = false;
                break;
            }
            stack.pop();
            scopes.pop();
            continue;
        }
        if tt == EnumToken::IdenTokenType && arena.string_equals_ignore_case(arena.string_value.get(token), "and") {
            if !expect_and_or || scopes.last().is_some_and(|s| s.1) {
                errors.push(unexpected(token));
                success = false;
                break;
            }
            arena.data_types.set(token, EnumToken::AndTokenType);
            if let Some(s) = scopes.last_mut() {
                s.0 = true;
            }
            expect_comma = false;
            continue;
        }
        if tt == EnumToken::IdenTokenType && arena.string_equals_ignore_case(arena.string_value.get(token), "or") {
            if !expect_and_or || scopes.last().is_some_and(|s| s.0) {
                errors.push(unexpected(token));
                success = false;
                break;
            }
            expect_comma = false;
            arena.data_types.set(token, EnumToken::OrTokenType);
            if let Some(s) = scopes.last_mut() {
                s.1 = true;
            }
            continue;
        }
        if tt == EnumToken::CommaTokenType {
            if !expect_comma {
                errors.push(unexpected(token));
                success = false;
                break;
            }
            stack.push(token);
            expect_comma = false;
            continue;
        }
        expect_and_or = true;
        expect_comma = true;
        if stack.last().is_some_and(|&s| t(s) == EnumToken::CommaTokenType) {
            stack.pop();
        }
    }
    if let Some(&last) = stack.last() {
        errors.push(drop_error(arena, format!("unexpected token {}", token_type_name(t(last))), last, options));
        success = false;
    }
    (success, errors)
}
