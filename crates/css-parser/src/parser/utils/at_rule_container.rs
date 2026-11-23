//! Port of `src/parser/utils/at-rule-container.ts`.

use crate::ast::arena::{AstArena, TokenField, TokenValue};
use crate::ast::types::EnumToken;
use crate::options::ParserOptions;
use crate::parser::utils::text::equals_ignore_case;
use crate::syntax::constants::{is_function_def_type, mFGT, mFLT, tokens_func_def_map};
use crate::types::ErrorDescription;
use crate::validation::config::get_syntax_rule;
use crate::validation::match_::{create_validation_context, drop_error, match_all_syntaxes, trim_array};
use crate::validation::parser::typedef::ValidationSyntaxGroupEnum;

fn trimmed(arena: &AstArena, v: &[u32]) -> Vec<u32> {
    let mut v = v.to_vec();
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

fn is_ws(arena: &AstArena, t: u32) -> bool {
    matches!(arena.data_types.get(t), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType)
}

/// parse the prelude of `@container`
pub fn parse_at_rule_container_query_list(arena: &AstArena, stream: &mut Vec<u32>, context: u32, options: &ParserOptions) -> (bool, Vec<ErrorDescription>) {
    let t = |x: u32| arena.data_types.get(x);
    let ty = |v: &Vec<u32>, i: usize| v.get(i).map(|&x| t(x));
    let mut match_count = 0;
    let has_errors = false;
    let mut errors: Vec<ErrorDescription> = Vec::new();
    let rules = get_syntax_rule(ValidationSyntaxGroupEnum::AtRules, &format!("@{}", arena.get_name(context)));
    let syntax: Option<Vec<_>> = rules.and_then(|r| r.get_prelude_rules()).map(|p| p.iter().skip(1).cloned().collect());
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
    let result = match_all_syntaxes(arena, syntax.as_deref(), &create_validation_context(arena, stream), options);
    if !result.success {
        errors.extend(result.errors);
        return (false, errors);
    }
    for part in parts.iter_mut() {
        let stream = part.clone();
        let mut success = true;
        let mut scopes: Vec<(bool, bool)> = vec![(false, false)];
        let mut stack: Vec<u32> = Vec::new();
        let mut tokens: Vec<u32> = Vec::new();
        let mut expect_and_or = false;
        let mut i = 0usize;
        while i < stream.len() && is_ws(arena, stream[i]) {
            tokens.push(stream[i]);
            i += 1;
        }
        if ty(&stream, i) == Some(EnumToken::IdenTokenType) {
            tokens.push(stream[i]);
            i += 1;
        }
        while i < stream.len() && is_ws(arena, stream[i]) {
            tokens.push(stream[i]);
            i += 1;
        }
        if i < stream.len() && t(stream[i]) != EnumToken::StartParensTokenType && t(stream[i]) != EnumToken::ContainerFunctionTokenDefType {
            return (false, vec![drop_error(arena, "expecting <container-condition>".into(), stream[i], options)]);
        }
        while i < stream.len() {
            let current = stream[i];
            tokens.push(current);
            if is_ws(arena, current) {
                i += 1;
                continue;
            }
            if expect_and_or && t(current) == EnumToken::CommaTokenType {
                success = false;
                errors.push(drop_error(arena, "expecting <and>, <or> or comma".into(), current, options));
                break;
            }
            if t(current) == EnumToken::StartParensTokenType || is_function_def_type(t(current)) {
                scopes.push((false, false));
                stack.push(current);
                i += 1;
                continue;
            }
            let top = |stack: &Vec<u32>| stack.last().map(|&s| t(s));
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
                        if val == "not" {
                            arena.data_types.set(current, EnumToken::NotTokenType);
                            stack.push(current);
                        } else if val == "and" || val == "or" {
                            let op = if val == "and" { EnumToken::AndTokenType } else { EnumToken::OrTokenType };
                            arena.data_types.set(current, op);
                            if val == "or" && scopes.len() <= 1 {
                                success = false;
                                errors.push(drop_error(arena, "<or> is not allowed outside of parentheses".into(), current, options));
                                break 'sw;
                            }
                            if let Some(scope) = scopes.last_mut() {
                                if op == EnumToken::AndTokenType {
                                    scope.0 = true;
                                } else {
                                    scope.1 = true;
                                }
                            }
                            stack.push(current);
                        }
                    }
                    EnumToken::EndParensTokenType => {
                        if top(&stack).is_some_and(|s| mFGT.contains(&s) || mFLT.contains(&s) || s == EnumToken::DelimTokenType || s == EnumToken::ColonTokenType) {
                            let index2 = index_of(&tokens, stack.last().copied());
                            let index3 = index_of(&tokens, if stack.len() >= 2 { Some(stack[stack.len() - 2]) } else { None });
                            let names = trimmed(arena, &slice(&tokens, index3 + 1, index2));
                            let values = trimmed(arena, &slice(&tokens, index2 + 1, tokens.len() as isize - 1));
                            let op = stack.pop().unwrap();
                            let token = arena.create_token(EnumToken::MediaQueryConditionTokenType, TokenValue::None, None);
                            arena.set_token_list(arena.field_list(token, TokenField::L), &names);
                            arena.set_token_list(arena.field_list(token, TokenField::Op), &[op]);
                            arena.set_token_list(arena.field_list(token, TokenField::R), &values);
                            if !names.is_empty() && !values.is_empty() {
                                arena.span_location(token, names[0], *values.last().unwrap());
                            }
                            let len = tokens.len() as isize;
                            let s = (index3 + 1).clamp(0, len) as usize;
                            let c = (len - index3 - 2).clamp(0, len - s as isize) as usize;
                            tokens.splice(s..s + c, [token]);
                            if t(op) == EnumToken::ColonTokenType {
                                if let Some(name) = names.iter().copied().find(|&n| t(n) == EnumToken::IdenTokenType) {
                                    let val = arena.resolve_string(arena.string_value.get(name));
                                    if let Some(rest) = val.strip_prefix("min-") {
                                        arena.string_value.set(name, arena.intern_string(rest));
                                        arena.data_types.set(op, EnumToken::GteTokenType);
                                    } else if let Some(rest) = val.strip_prefix("max-") {
                                        arena.string_value.set(name, arena.intern_string(rest));
                                        arena.data_types.set(op, EnumToken::LteTokenType);
                                    }
                                }
                            }
                        }
                        let index = index_of(&tokens, stack.last().copied());
                        let empty_message;
                        if top(&stack).is_some_and(is_function_def_type) {
                            let func = tokens[index as usize];
                            arena.data_types.set(func, tokens_func_def_map(t(func)).unwrap_or(0));
                            arena.set_token_list(func, &trimmed(arena, &slice(&tokens, index + 1, tokens.len() as isize - 1)));
                            arena.span_location(func, func, current);
                            empty_message = format!("expecting '<{}-query>'", arena.resolve_string(arena.string_value.get(func)));
                        } else {
                            if index < 0 {
                                // typescript: `tokens[-1]` is undefined
                                success = false;
                                break 'sw;
                            }
                            let parens = tokens[index as usize];
                            arena.data_types.set(parens, EnumToken::ParensTokenType);
                            arena.set_token_list(parens, &slice(&tokens, index + 1, tokens.len() as isize - 1));
                            arena.span_location(parens, parens, current);
                            empty_message = "expecting '<query-in-parens>'".to_string();
                        }
                        if arena.token_iter(tokens[index as usize]).all(|x| is_ws(arena, x)) {
                            success = false;
                            errors.push(drop_error(arena, empty_message, current, options));
                            break 'sw;
                        }
                        tokens.truncate(index as usize + 1);
                        stack.pop();
                        scopes.pop();
                        if top(&stack) == Some(EnumToken::NotTokenType) {
                            let mut j = index_of(&tokens, stack.last().copied());
                            let k = j;
                            loop {
                                if j == 0 {
                                    j = -1;
                                    break;
                                }
                                j -= 1;
                                if j < 0 || !is_ws(arena, tokens[j as usize]) {
                                    break;
                                }
                            }
                            if j >= 0 && t(tokens[j as usize]) != EnumToken::StartParensTokenType {
                                success = false;
                                errors.push(drop_error(arena, "unexpected token 'not'".into(), tokens[k as usize], options));
                                break 'sw;
                            }
                        }
                        if matches!(top(&stack), Some(EnumToken::AndTokenType) | Some(EnumToken::OrTokenType)) {
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
                            expect_and_or = true;
                            if t(op) == EnumToken::AndTokenType {
                                let not_name_or_space = |x: &u32| !matches!(t(*x), EnumToken::WhitespaceTokenType | EnumToken::IdenTokenType);
                                let lc = left.iter().copied().find(not_name_or_space);
                                let rc = right.iter().copied().find(not_name_or_space);
                                if let (Some(lc), Some(rc)) = (lc, rc) {
                                    if t(lc) == EnumToken::ParensTokenType && t(rc) == EnumToken::ParensTokenType {
                                        let lc = arena.tokens(lc).into_iter().find(not_name_or_space);
                                        let rc = arena.tokens(rc).into_iter().find(not_name_or_space);
                                        if let (Some(lc), Some(rc)) = (lc, rc) {
                                            if t(lc) == EnumToken::MediaQueryConditionTokenType && t(rc) == EnumToken::MediaQueryConditionTokenType {
                                                let left_op = arena.field(lc, TokenField::Op);
                                                let right_op = arena.field(rc, TokenField::Op);
                                                let is_cmp = |o: u32| mFLT.contains(&t(o)) || mFGT.contains(&t(o));
                                                if is_cmp(left_op) && is_cmp(right_op) {
                                                    let name = arena.field_tokens(lc, TokenField::L).into_iter().find(|&x| t(x) == EnumToken::IdenTokenType);
                                                    let name2 = arena.field_tokens(rc, TokenField::L).into_iter().find(|&x| t(x) == EnumToken::IdenTokenType);
                                                    if let (Some(name), Some(name2)) = (name, name2) {
                                                        if equals_ignore_case(
                                                            &arena.resolve_string(arena.string_value.get(name)),
                                                            &arena.resolve_string(arena.string_value.get(name2)),
                                                        ) {
                                                            let flipped = match t(left_op) {
                                                                EnumToken::LtTokenType => Some(EnumToken::GtTokenType),
                                                                EnumToken::LteTokenType => Some(EnumToken::GteTokenType),
                                                                EnumToken::GtTokenType => Some(EnumToken::LtTokenType),
                                                                EnumToken::GteTokenType => Some(EnumToken::LteTokenType),
                                                                _ => None,
                                                            };
                                                            if let Some(f) = flipped {
                                                                arena.data_types.set(left_op, f);
                                                            }
                                                            let range_query = arena.create_token(EnumToken::MediaRangeQueryTokenType, TokenValue::None, None);
                                                            arena.set_token_list(arena.field_list(range_query, TokenField::L), &arena.field_tokens(lc, TokenField::R));
                                                            arena.set_token_list(arena.field_list(range_query, TokenField::Val), &[name]);
                                                            arena.set_token_list(arena.field_list(range_query, TokenField::Op), &[left_op]);
                                                            arena.set_token_list(arena.field_list(range_query, TokenField::Op2), &[right_op]);
                                                            arena.set_token_list(arena.field_list(range_query, TokenField::R), &arena.field_tokens(rc, TokenField::R));
                                                            arena.span_location(range_query, lc, rc);
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
            return (false, errors);
        }
        trim_array(arena, &mut tokens);
        *part = tokens;
    }
    stream.clear();
    for part in &parts {
        stream.extend_from_slice(part);
    }
    let success = !has_errors
        || stream.iter().any(|&x| !matches!(t(x), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType | EnumToken::CommaTokenType));
    (success, errors)
}
