//! Port of `src/parser/utils/at-rule-when-else.ts`.

use super::at_rule_media::parse_mediaquery_list;
use super::at_rule_support::parse_at_rule_support_syntax;
use crate::ast::arena::{AstArena, TokenField, TokenValue};
use crate::ast::types::EnumToken;
use crate::options::ParserOptions;
use crate::syntax::constants::{is_function_def_type, tokens_func_def_map};
use crate::types::ErrorDescription;
use crate::validation::match_::trim_array;

/// parse the condition of `@when` / `@else`
pub fn match_at_rule_when_else_syntax(arena: &AstArena, stream: &mut Vec<u32>, context: u32, options: &ParserOptions) -> (bool, Vec<ErrorDescription>) {
    let t = |x: u32| arena.data_types.get(x);
    let mut tokens: Vec<u32> = Vec::new();
    let mut stack: Vec<u32> = Vec::new();
    let mut expect_and_or = false;
    let mut i = 0usize;
    while i < stream.len() {
        tokens.push(stream[i]);
        if expect_and_or {
            let mut k = i;
            while k < stream.len() && matches!(t(stream[k]), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType) {
                tokens.push(stream[k]);
                k += 1;
            }
            expect_and_or = false;
        }
        let current = stream[i];
        match t(current) {
            EnumToken::IdenTokenType => {
                let val = arena.resolve_string(arena.string_value.get(current)).to_lowercase();
                if val == "and" || val == "or" {
                    arena.data_types.set(current, if val == "or" { EnumToken::OrTokenType } else { EnumToken::AndTokenType });
                    stack.push(current);
                }
            }
            EnumToken::SupportsFunctionTokenDefType | EnumToken::WhenElseFunctionTokenDefType => {
                let mut match_count = 1;
                let mut j = i + 1;
                while j < stream.len() {
                    let tj = t(stream[j]);
                    if tj == EnumToken::EndParensTokenType {
                        match_count -= 1;
                        if match_count == 0 {
                            break;
                        }
                    } else if tj == EnumToken::StartParensTokenType || is_function_def_type(tj) {
                        match_count += 1;
                    }
                    j += 1;
                }
                let end_j = j.min(stream.len().saturating_sub(1));
                let slice: Vec<u32> = stream[i..(j + 1).min(stream.len())].to_vec();
                let func_name = arena.resolve_string(arena.string_value.get(current)).to_lowercase();
                let is_supports_function = t(current) == EnumToken::SupportsFunctionTokenDefType;
                let func_type = tokens_func_def_map(t(current)).unwrap_or(0);
                let parens = arena.create_token(EnumToken::StartParensTokenType, TokenValue::None, None);
                arena.span_location(parens, current, stream[end_j]);
                let mut token_list: Vec<u32> = std::iter::once(parens).chain(slice.iter().skip(1).copied()).collect();
                if func_name == "media" {
                    let result = parse_mediaquery_list(arena, &mut token_list, options);
                    if !result.0 {
                        return result;
                    }
                } else if func_name == "supports" {
                    let result = parse_at_rule_support_syntax(arena, &mut token_list, context, options);
                    if !result.0 {
                        return result;
                    }
                } else if is_supports_function {
                    token_list = slice.clone();
                    let mut sub = slice.clone();
                    let result = parse_at_rule_support_syntax(arena, &mut sub, context, options);
                    if !result.0 {
                        return result;
                    }
                }
                arena.span_location(current, current, stream[end_j]);
                let chi: Vec<u32> = if is_supports_function {
                    if arena.first_token.get(current) != 0 {
                        arena.tokens(current)
                    } else {
                        let mut inner: Vec<u32> = if slice.len() >= 2 { slice[1..slice.len() - 1].to_vec() } else { Vec::new() };
                        trim_array(arena, &mut inner);
                        inner
                    }
                } else {
                    token_list.first().map_or_else(Vec::new, |&first| arena.tokens(first))
                };
                arena.data_types.set(current, func_type);
                arena.set_token_list(current, &chi);
                if stack.last().is_some_and(|&s| matches!(t(s), EnumToken::AndTokenType | EnumToken::OrTokenType)) {
                    let op = *stack.last().unwrap();
                    let index = tokens.iter().position(|&x| x == op).map_or(-1, |p| p as isize);
                    let index2 = if stack.len() > 1 {
                        tokens.iter().position(|&x| x == stack[stack.len() - 2]).map_or(-1, |p| p as isize) + 1
                    } else {
                        0
                    };
                    let condition = arena.create_token(EnumToken::WhenElseQueryConditionTokenType, TokenValue::None, None);
                    let (i2, i1) = (index2.max(0) as usize, index.max(0) as usize);
                    let mut left: Vec<u32> = if i2 < i1 { tokens[i2..i1].to_vec() } else { Vec::new() };
                    trim_array(arena, &mut left);
                    let mut right: Vec<u32> = tokens[(i1 + 1).min(tokens.len())..].to_vec();
                    trim_array(arena, &mut right);
                    arena.set_token_list(arena.field_list(condition, TokenField::L), &left);
                    arena.set_token_list(arena.field_list(condition, TokenField::R), &right);
                    arena.set_token_list(arena.field_list(condition, TokenField::Op), &[op]);
                    arena.span_location(condition, op, current);
                    if i2 < tokens.len() {
                        tokens[i2] = condition;
                    } else {
                        tokens.push(condition);
                    }
                    tokens.truncate(i2 + 1);
                    stack.pop();
                }
                i = j;
                expect_and_or = true;
            }
            _ => {}
        }
        i += 1;
    }
    trim_array(arena, &mut tokens);
    *stream = tokens;
    (true, Vec::new())
}
