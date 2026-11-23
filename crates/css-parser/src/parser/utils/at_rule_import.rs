//! Port of `src/parser/utils/at-rule-import.ts`.

use super::at_rule_media::parse_mediaquery_list;
use super::at_rule_support::parse_at_rule_support_syntax;
use crate::ast::arena::AstArena;
use crate::ast::types::EnumToken;
use crate::options::ParserOptions;
use crate::syntax::constants::{is_function_def_type, tokens_func_def_map};
use crate::types::ErrorDescription;
use crate::validation::config::get_syntax_rule;
use crate::validation::match_::{token_location, trim_array};
use crate::validation::parser::typedef::ValidationSyntaxGroupEnum;

fn import_error(arena: &AstArena, message: &str, node: Option<u32>, options: &ParserOptions) -> ErrorDescription {
    let mut error = ErrorDescription::drop(message).with_syntax("@import");
    error.node = node;
    error.location = token_location(arena, node.unwrap_or(0), options);
    error
}

/// `tokens.splice(start, count)`, trimmed
fn splice_trimmed(arena: &AstArena, v: &mut Vec<u32>, start: isize, count: isize) -> Vec<u32> {
    let len = v.len() as isize;
    let s = if start < 0 { (len + start).max(0) } else { start.min(len) } as usize;
    let c = count.clamp(0, len - s as isize) as usize;
    let mut out: Vec<u32> = v.drain(s..s + c).collect();
    trim_array(arena, &mut out);
    out
}

/// `tokens.splice(index, 1)`
fn remove_at(v: &mut Vec<u32>, index: usize) {
    if index < v.len() {
        v.remove(index);
    }
}

/// match the prelude of `@import`
pub fn match_at_rule_import_syntax(arena: &AstArena, at_rule: u32, stream: &mut Vec<u32>, options: &ParserOptions) -> (bool, Vec<ErrorDescription>) {
    let t = |x: Option<u32>| x.map_or(0u8, |x| arena.data_types.get(x));
    let at = |stream: &Vec<u32>, i: usize| stream.get(i).copied();
    let mut success = true;
    let mut index = 0usize;
    let mut errors: Vec<ErrorDescription> = Vec::new();
    let mut tokens: Vec<u32> = Vec::new();
    let mut stack: Vec<u32> = Vec::new();
    let prelude_length = get_syntax_rule(ValidationSyntaxGroupEnum::AtRules, "@import")
        .and_then(|r| r.get_prelude_rules())
        .map_or(0, |p| p.len().saturating_sub(1));
    trim_array(arena, stream);
    if t(at(stream, index)) == EnumToken::StringTokenType {
        tokens.push(stream[index]);
        index += 1;
    } else if t(at(stream, index)) == EnumToken::UrlFunctionTokenDefType {
        let mut match_count = 0;
        let mut k = 0;
        while k < stream.len() {
            let tk = arena.data_types.get(stream[k]);
            if tk == EnumToken::EndParensTokenType {
                match_count -= 1;
            } else if tk == EnumToken::StartParensTokenType || is_function_def_type(tk) {
                match_count += 1;
            }
            if match_count == 0 {
                break;
            }
            k += 1;
        }
        let mut slice: Vec<u32> = stream[(index + 1).min(stream.len())..k.min(stream.len()).max((index + 1).min(stream.len()))].to_vec();
        if stream.len() > 1 {
            arena.span_location(stream[0], stream[0], stream[1]);
        }
        arena.data_types.set(stream[0], tokens_func_def_map(arena.data_types.get(stream[0])).unwrap_or(0));
        trim_array(arena, &mut slice);
        arena.set_token_list(stream[0], &slice);
        tokens.push(stream[0]);
        index = k + 1;
    } else {
        return (false, vec![import_error(arena, "Expected string or url()", stream.first().copied(), options)]);
    }
    while index + 1 < prelude_length && matches!(t(at(stream, index)), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType) {
        tokens.push(stream[index]);
        index += 1;
    }
    let is_layer = |i: usize| at(stream, i).is_some_and(|x| arena.string_equals(arena.string_value.get(x), "layer"));
    if t(at(stream, index)) == EnumToken::IdenTokenType && is_layer(index) {
        tokens.push(stream[index]);
        index += 1;
    } else if is_function_def_type(t(at(stream, index))) && is_layer(index) {
        stack.push(stream[index]);
        tokens.push(stream[index]);
        index += 1;
        if t(at(stream, index)) == EnumToken::EndParensTokenType {
            remove_at(&mut tokens, index);
            return (false, vec![import_error(arena, "Expected <layer-name>", at(stream, index), options)]);
        } else if t(at(stream, index)) == EnumToken::IdenTokenType {
            tokens.push(stream[index]);
            index += 1;
            while t(at(stream, index)) == EnumToken::ClassSelectorTokenType {
                tokens.push(stream[index]);
                index += 1;
            }
            let func = *stack.last().unwrap();
            let i = tokens.iter().position(|&x| x == func).map_or(-1, |p| p as isize);
            remove_at(&mut tokens, index);
            arena.data_types.set(func, EnumToken::FunctionTokenType);
            let count = index as isize - i - 1;
            index += 1;
            let inner = splice_trimmed(arena, &mut tokens, i + 1, count);
            arena.set_token_list(func, &inner);
            stack.pop();
        } else {
            return (false, vec![import_error(arena, "Expected <layer-name>", at(stream, index), options)]);
        }
    }
    while matches!(t(at(stream, index)), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType) {
        tokens.push(stream[index]);
        index += 1;
    }
    if index < stream.len()
        && is_function_def_type(t(at(stream, index)))
        && arena.string_equals(arena.string_value.get(stream[index]), "supports")
    {
        stack.push(stream[index]);
        tokens.push(stream[index]);
        index += 1;
        let mut match_count = 1;
        while index < stream.len() && match_count > 0 {
            let ti = arena.data_types.get(stream[index]);
            if ti == EnumToken::StartParensTokenType || is_function_def_type(ti) {
                match_count += 1;
            } else if ti == EnumToken::EndParensTokenType {
                match_count -= 1;
                if match_count == 0 {
                    remove_at(&mut tokens, index);
                    let func = *stack.last().unwrap();
                    let i = tokens.iter().position(|&x| x == func).map_or(-1, |p| p as isize);
                    arena.data_types.set(func, tokens_func_def_map(arena.data_types.get(func)).unwrap_or(0));
                    let count = index as isize - i - 1;
                    index += 1;
                    let inner = splice_trimmed(arena, &mut tokens, i + 1, count);
                    arena.set_token_list(func, &inner);
                    stack.pop();
                    break;
                }
            }
            tokens.push(stream[index]);
            index += 1;
        }
        if let Some(&supports) = tokens.last() {
            let mut chi = arena.tokens(supports);
            let result = parse_at_rule_support_syntax(arena, &mut chi, at_rule, options);
            arena.set_token_list(supports, &chi);
            if !result.0 && !result.1.is_empty() {
                errors.extend(result.1);
                return (false, errors);
            }
        }
    }
    let mut splice: Vec<u32> = if index < stream.len() { stream.drain(index..).collect() } else { Vec::new() };
    let sliced = parse_mediaquery_list(arena, &mut splice, options);
    tokens.extend(splice);
    errors.extend(sliced.1);
    if !sliced.0 {
        success = false;
    }
    trim_array(arena, &mut tokens);
    *stream = tokens;
    (success, errors)
}
