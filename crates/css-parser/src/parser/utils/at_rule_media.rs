//! Port of `src/parser/utils/at-rule-media.ts`.

use crate::ast::arena::{AstArena, TokenField, TokenValue};
use crate::ast::math::expression::evaluate;
use crate::ast::math::math::gcd;
use crate::ast::types::EnumToken;
use crate::options::ParserOptions;
use crate::parser::utils::text::equals_ignore_case;
use crate::syntax::constants::{is_function_def_type, mFGT, mFLT, tokens_func_def_map, MEDIA_TYPES};
use crate::types::ErrorDescription;
use crate::validation::match_::{create_validation_context, drop_error, function_syntax, get_mf_info, is_mf_value, match_all_syntaxes, trim_array, Syntax};
use crate::validation::parser::typedef::MediaFeatureType;

fn trimmed(arena: &AstArena, tokens: &[u32]) -> Vec<u32> {
    let mut v = tokens.to_vec();
    trim_array(arena, &mut v);
    v
}

fn slice(v: &[u32], start: isize, end: isize) -> Vec<u32> {
    let len = v.len() as isize;
    let norm = |i: isize| if i < 0 { (len + i).max(0) } else { i.min(len) } as usize;
    let (s, e) = (norm(start), norm(end));
    if s < e {
        v[s..e].to_vec()
    } else {
        Vec::new()
    }
}

fn index_of(v: &[u32], value: Option<u32>) -> isize {
    value.and_then(|value| v.iter().position(|&x| x == value)).map_or(-1, |i| i as isize)
}

/// `tokens.splice(start, count, item)`
fn splice_one(v: &mut Vec<u32>, start: isize, count: isize, item: u32) {
    let len = v.len() as isize;
    let s = if start < 0 { (len + start).max(0) } else { start.min(len) } as usize;
    let c = count.clamp(0, len - s as isize) as usize;
    v.splice(s..s + c, [item]);
}

fn is_ws(arena: &AstArena, t: u32) -> bool {
    matches!(arena.data_types.get(t), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType)
}

fn mf_type(info: Option<&serde_json::Value>) -> Option<&str> {
    info.and_then(|i| i["type"].as_str())
}

/// evaluate the calc() functions of a media feature value
fn evaluate_calc(arena: &AstArena, val: &mut [u32]) {
    for l in 0..val.len() {
        if arena.data_types.get(val[l]) == EnumToken::MathFunctionTokenType && arena.string_equals(arena.string_value.get(val[l]), "calc") {
            let value = evaluate(arena, &[val[l]]);
            if value.len() == 1 {
                arena.copy_location(value[0], val[l]);
                val[l] = value[0];
            }
        }
    }
}

/// reduce a ratio by the greatest common divisor
fn reduce_ratio(arena: &AstArena, val: &mut Vec<u32>, options: &ParserOptions) {
    let filtered: Vec<u32> = val.iter().copied().filter(|&n| !is_ws(arena, n)).collect();
    if filtered.len() == 3 {
        if options.compute_calc_expression == Some(true) {
            let div = gcd(arena.numeric_value.get(filtered[0]), arena.numeric_value.get(filtered[2]));
            if div > 1.0 {
                arena.numeric_value.set(filtered[0], arena.numeric_value.get(filtered[0]) / div);
                arena.numeric_value.set(filtered[2], arena.numeric_value.get(filtered[2]) / div);
            }
        }
        *val = filtered;
    }
}

/// parse a media query list. the invalid queries are removed
pub fn parse_mediaquery_list(arena: &AstArena, stream: &mut Vec<u32>, options: &ParserOptions) -> (bool, Vec<ErrorDescription>) {
    let t = |x: u32| arena.data_types.get(x);
    let mut match_count = 0;
    let mut errors: Vec<ErrorDescription> = Vec::new();
    let mut has_errors = false;
    let mut parts: Vec<Vec<u32>> = vec![Vec::new()];
    for &token in stream.iter() {
        let typ = t(token);
        if typ == EnumToken::CommaTokenType && match_count == 0 {
            parts.push(Vec::new());
            continue;
        }
        parts.last_mut().unwrap().push(token);
        if typ == EnumToken::StartParensTokenType || is_function_def_type(typ) {
            match_count += 1;
        } else if typ == EnumToken::EndParensTokenType && match_count > 0 {
            match_count -= 1;
        }
    }
    let mut kept: Vec<Vec<u32>> = Vec::new();
    for part in parts {
        let stream = part;
        let mut success = true;
        // scopes: (has and, has or)
        let mut scopes: Vec<(bool, bool)> = vec![(false, false)];
        let mut stack: Vec<u32> = Vec::new();
        let mut tokens: Vec<u32> = Vec::new();
        let mut i = 0usize;
        while i < stream.len() && is_ws(arena, stream[i]) {
            tokens.push(stream[i]);
            i += 1;
        }
        if i < stream.len() {
            if t(stream[i]) == EnumToken::IdenTokenType {
                let val = arena.resolve_string(arena.string_value.get(stream[i])).to_lowercase();
                if MEDIA_TYPES.contains(&val.as_str()) {
                    tokens.push(stream[i]);
                    i += 1;
                } else {
                    success = false;
                    errors.push(drop_error(arena, "expecting '<media-type>'".into(), stream[i], options));
                }
            } else if t(stream[i]) != EnumToken::StartParensTokenType {
                success = false;
                errors.push(drop_error(arena, "expecting '('".into(), stream[i], options));
            }
        }
        while i < stream.len() {
            let current = stream[i];
            tokens.push(current);
            if is_ws(arena, current) {
                i += 1;
                continue;
            }
            if t(current) == EnumToken::StartParensTokenType || is_function_def_type(t(current)) {
                scopes.push((false, false));
                stack.push(current);
                i += 1;
                continue;
            }
            'sw: {
                match t(current) {
                    EnumToken::ColonTokenType
                    | EnumToken::LtTokenType
                    | EnumToken::LteTokenType
                    | EnumToken::GtTokenType
                    | EnumToken::GteTokenType
                    | EnumToken::DelimTokenType => stack.push(current),
                    EnumToken::IdenTokenType => {
                        let val = arena.resolve_string(arena.string_value.get(current)).to_lowercase();
                        if val == "and" || val == "or" {
                            let op = if val == "and" { EnumToken::AndTokenType } else { EnumToken::OrTokenType };
                            arena.data_types.set(current, op);
                            if val == "or" && scopes.len() == 1 && (scopes[0].0 || scopes[0].1) && !scopes[0].1 {
                                success = false;
                                errors.push(drop_error(arena, "<or> is not allowed outside of parentheses".into(), current, options));
                                break 'sw;
                            }
                            let scope = scopes.last_mut().unwrap();
                            if (val == "or" && scope.0) || (val == "and" && scope.1) {
                                success = false;
                                errors.push(drop_error(arena, "cannot mix <and> and <or> at the same level".into(), current, options));
                            }
                            if op == EnumToken::AndTokenType {
                                scope.0 = true;
                            } else {
                                scope.1 = true;
                            }
                            stack.push(current);
                        }
                    }
                    EnumToken::EndParensTokenType => {
                        if stack.last().is_some_and(|&s| is_function_def_type(t(s))) {
                            let index = index_of(&tokens, stack.last().copied());
                            let func = tokens[index as usize];
                            arena.span_location(func, func, current);
                            arena.data_types.set(func, tokens_func_def_map(t(func)).unwrap_or(0));
                            let inner = trimmed(arena, &slice(&tokens, index + 1, tokens.len() as isize - 1));
                            arena.set_token_list(func, &inner);
                            tokens.truncate(index as usize + 1);
                            // looked up once per function name, the children of the syntax are borrowed
                            let syntax = function_syntax(arena, func);
                            let chi: Option<&[Syntax]> = syntax.as_ref().and_then(|s| s.first()).map(|f| f.children());
                            let result = match_all_syntaxes(arena, chi, &create_validation_context(arena, &arena.tokens(func)), options);
                            stack.pop();
                            scopes.pop();
                            if !result.success {
                                errors.extend(result.errors);
                                success = false;
                            }
                            break 'sw;
                        }
                        // range: <mf-value> <mf-lt> <mf-name> <mf-lt> <mf-value>
                        let top_t = stack.last().map(|&s| t(s));
                        if top_t.is_some_and(|s| mFLT.contains(&s) || mFGT.contains(&s)) {
                            let index = index_of(&tokens, stack.last().copied());
                            let prev_token = if stack.len() >= 2 { Some(stack[stack.len() - 2]) } else { None };
                            if prev_token.is_some_and(|p| mFLT.contains(&t(p)) || mFGT.contains(&t(p))) {
                                let prev_token = prev_token.unwrap();
                                let index2 = index_of(&tokens, Some(prev_token));
                                let index3 = index_of(&tokens, if stack.len() >= 3 { Some(stack[stack.len() - 3]) } else { None });
                                let mut left = trimmed(arena, &slice(&tokens, index3 + 1, index2));
                                let mut right = trimmed(arena, &slice(&tokens, index + 1, tokens.len() as isize - 1));
                                let names = trimmed(arena, &slice(&tokens, index2 + 1, index));
                                let filtered_names: Vec<u32> = names.iter().copied().filter(|&n| !is_ws(arena, n)).collect();
                                let name = filtered_names.first().map_or(String::new(), |&n| arena.resolve_string(arena.string_value.get(n)).to_string());
                                let info = get_mf_info(&name);
                                if options.compute_calc_expression == Some(true)
                                    && (info.is_none() || !matches!(mf_type(info), Some(MediaFeatureType::KeywordType) | Some(MediaFeatureType::StringType)))
                                {
                                    evaluate_calc(arena, &mut left);
                                    evaluate_calc(arena, &mut right);
                                }
                                if mf_type(info) == Some(MediaFeatureType::RatioType) {
                                    reduce_ratio(arena, &mut left, options);
                                    reduce_ratio(arena, &mut right, options);
                                }
                                let range_query = arena.create_token(EnumToken::MediaRangeQueryTokenType, TokenValue::None, None);
                                arena.set_token_list(arena.field_list(range_query, TokenField::L), &left);
                                arena.set_token_list(arena.field_list(range_query, TokenField::Val), &filtered_names);
                                arena.set_token_list(arena.field_list(range_query, TokenField::Op), &[prev_token]);
                                arena.set_token_list(arena.field_list(range_query, TokenField::Op2), &[*stack.last().unwrap()]);
                                arena.set_token_list(arena.field_list(range_query, TokenField::R), &right);
                                if !left.is_empty() && !right.is_empty() {
                                    arena.span_location(range_query, left[0], *right.last().unwrap());
                                }
                                let count = tokens.len() as isize - index3 - 2;
                                splice_one(&mut tokens, index3 + 1, count, range_query);
                                stack.pop();
                                stack.pop();
                            }
                        }
                        let top_t = stack.last().map(|&s| t(s));
                        if top_t.is_some_and(|s| mFGT.contains(&s) || mFLT.contains(&s) || s == EnumToken::DelimTokenType || s == EnumToken::ColonTokenType) {
                            let op = *stack.last().unwrap();
                            let index2 = index_of(&tokens, Some(op));
                            let index3 = index_of(&tokens, if stack.len() >= 2 { Some(stack[stack.len() - 2]) } else { None });
                            let names = trimmed(arena, &slice(&tokens, index3 + 1, index2));
                            let mut values = trimmed(arena, &slice(&tokens, index2 + 1, tokens.len() as isize - 1));
                            let filtered_names: Vec<u32> = names.iter().copied().filter(|&n| !is_ws(arena, n)).collect();
                            let name = filtered_names.first().map_or(String::new(), |&n| arena.resolve_string(arena.string_value.get(n)).to_string());
                            let info = get_mf_info(&name);
                            if options.compute_calc_expression == Some(true)
                                && (info.is_none() || !matches!(mf_type(info), Some(MediaFeatureType::KeywordType) | Some(MediaFeatureType::StringType)))
                            {
                                evaluate_calc(arena, &mut values);
                            }
                            let is_range = mFGT.contains(&t(op)) || mFLT.contains(&t(op)) || t(op) == EnumToken::DelimTokenType;
                            let mf_value = is_mf_value(arena, &name, &values, is_range);
                            if !mf_value.success {
                                success = false;
                                let message = if mf_value.is_value_allowed == Some(false) { "invalid <mf-name>" } else { "expected <mf-value>" };
                                let node = values.first().copied().unwrap_or(0);
                                errors.push(drop_error(arena, message.into(), node, options));
                                break 'sw;
                            }
                            if mf_type(info) == Some(MediaFeatureType::RatioType) {
                                reduce_ratio(arena, &mut values, options);
                            }
                            if t(op) == EnumToken::ColonTokenType {
                                if let Some(rest) = name.strip_prefix("min-") {
                                    arena.string_value.set(filtered_names[0], arena.intern_string(rest));
                                    arena.data_types.set(op, EnumToken::GteTokenType);
                                } else if let Some(rest) = name.strip_prefix("max-") {
                                    arena.string_value.set(filtered_names[0], arena.intern_string(rest));
                                    arena.data_types.set(op, EnumToken::LteTokenType);
                                }
                            }
                            let condition = arena.create_token(EnumToken::MediaQueryConditionTokenType, TokenValue::None, None);
                            arena.set_token_list(arena.field_list(condition, TokenField::L), &names);
                            let op = stack.pop().unwrap();
                            arena.set_token_list(arena.field_list(condition, TokenField::Op), &[op]);
                            arena.set_token_list(arena.field_list(condition, TokenField::R), &values);
                            if !names.is_empty() && !values.is_empty() {
                                arena.span_location(condition, names[0], *values.last().unwrap());
                            }
                            let count = tokens.len() as isize - index3 - 2;
                            splice_one(&mut tokens, index3 + 1, count, condition);
                        }
                        if stack.is_empty() {
                            success = false;
                            errors.push(drop_error(arena, "unmatched ')'".into(), current, options));
                            break 'sw;
                        }
                        let index = index_of(&tokens, stack.last().copied());
                        let parens = tokens[index as usize];
                        arena.data_types.set(parens, EnumToken::ParensTokenType);
                        let inner = slice(&tokens, index + 1, tokens.len() as isize - 1);
                        arena.set_token_list(parens, &inner);
                        arena.span_location(parens, parens, current);
                        tokens.truncate(index as usize + 1);
                        scopes.pop();
                        stack.pop();
                        if stack.last().is_some_and(|&s| matches!(t(s), EnumToken::AndTokenType | EnumToken::OrTokenType)) {
                            let index = index_of(&tokens, stack.last().copied());
                            let mut l = index - 1;
                            while l > 0 && is_ws(arena, tokens[l as usize]) {
                                l -= 1;
                            }
                            let left = trimmed(arena, &slice(&tokens, l, index));
                            let right = trimmed(arena, &slice(&tokens, index + 1, tokens.len() as isize));
                            let op = stack.pop().unwrap();
                            let condition = arena.create_token(EnumToken::MediaQueryConditionTokenType, TokenValue::None, None);
                            arena.set_token_list(arena.field_list(condition, TokenField::Op), &[op]);
                            arena.set_token_list(arena.field_list(condition, TokenField::L), &left);
                            arena.set_token_list(arena.field_list(condition, TokenField::R), &right);
                            if !left.is_empty() && !right.is_empty() {
                                arena.span_location(condition, left[0], *right.last().unwrap());
                            }
                            let l = l.max(0) as usize;
                            if l < tokens.len() {
                                tokens[l] = condition;
                            } else {
                                tokens.push(condition);
                            }
                            tokens.truncate(l + 1);
                            if options.minify == Some(true) && t(op) == EnumToken::AndTokenType {
                                let single = |side: &Vec<u32>| {
                                    (side.len() == 1 && t(side[0]) == EnumToken::ParensTokenType && arena.token_count(side[0]) == 1)
                                        .then(|| arena.first_token.get(side[0]))
                                };
                                if let (Some(lc), Some(rc)) = (single(&left), single(&right)) {
                                    if t(lc) == EnumToken::MediaQueryConditionTokenType && t(rc) == EnumToken::MediaQueryConditionTokenType {
                                        let left_names = arena.field_tokens(lc, TokenField::L);
                                        let right_names = arena.field_tokens(rc, TokenField::L);
                                        let left_op = arena.field(lc, TokenField::Op);
                                        let right_op = arena.field(rc, TokenField::Op);
                                        if left_names.len() == 1
                                            && t(left_names[0]) == EnumToken::IdenTokenType
                                            && matches!(t(left_op), EnumToken::GtTokenType | EnumToken::GteTokenType)
                                            && right_names.len() == 1
                                            && t(right_names[0]) == EnumToken::IdenTokenType
                                            && matches!(t(right_op), EnumToken::LtTokenType | EnumToken::LteTokenType)
                                            && equals_ignore_case(
                                                &arena.resolve_string(arena.string_value.get(left_names[0])),
                                                &arena.resolve_string(arena.string_value.get(right_names[0])),
                                            )
                                        {
                                            let range_query = arena.create_token(EnumToken::MediaRangeQueryTokenType, TokenValue::None, None);
                                            arena.data_types.set(
                                                left_op,
                                                if t(left_op) == EnumToken::GteTokenType { EnumToken::LteTokenType } else { EnumToken::LtTokenType },
                                            );
                                            arena.set_token_list(arena.field_list(range_query, TokenField::L), &arena.field_tokens(lc, TokenField::R));
                                            arena.set_token_list(arena.field_list(range_query, TokenField::Val), &left_names);
                                            arena.set_token_list(arena.field_list(range_query, TokenField::Op), &[left_op]);
                                            arena.set_token_list(arena.field_list(range_query, TokenField::Op2), &[right_op]);
                                            arena.set_token_list(arena.field_list(range_query, TokenField::R), &arena.field_tokens(rc, TokenField::R));
                                            arena.span_location(range_query, left[0], *right.last().unwrap());
                                            let parens = arena.create_token(EnumToken::ParensTokenType, TokenValue::None, None);
                                            arena.set_token_list(parens, &[range_query]);
                                            arena.copy_location(parens, range_query);
                                            tokens[l] = parens;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            if !success {
                break;
            }
            i += 1;
        }
        if !success {
            has_errors = true;
        } else {
            trim_array(arena, &mut tokens);
            kept.push(tokens);
        }
    }
    stream.clear();
    let mut has_part = false;
    for part in kept {
        if part.is_empty() {
            continue;
        }
        if has_part {
            stream.push(arena.create_token(EnumToken::CommaTokenType, TokenValue::None, None));
        }
        stream.extend(part);
        has_part = true;
    }
    let success = !has_errors
        || stream.iter().any(|&t0| !matches!(t(t0), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType | EnumToken::CommaTokenType));
    (success, errors)
}
