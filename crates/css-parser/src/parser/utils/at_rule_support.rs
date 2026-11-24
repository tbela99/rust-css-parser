//! Port of `src/parser/utils/at-rule-support.ts`.

use crate::ast::arena::{AstArena, TokenField, TokenValue};
use crate::ast::types::EnumToken;
use crate::options::ParserOptions;
use crate::parser::utils::declaration::parse_declaration;
use crate::syntax::constants::{is_function_def_type, tokens_func_def_map, PSEUDO_ELEMENTS};
use crate::types::ErrorDescription;
use crate::validation::config::get_syntax_config;
use crate::validation::match_::{create_validation_context, match_all_syntaxes, selector_function_syntax, trim_array, Syntax};

fn index_of(v: &[u32], value: Option<u32>) -> isize {
    value.and_then(|value| v.iter().position(|&x| x == value)).map_or(-1, |i| i as isize)
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

fn trim_white_space(t: u8) -> bool {
    matches!(
        t,
        EnumToken::GtTokenType
            | EnumToken::ChildCombinatorTokenType
            | EnumToken::NextSiblingCombinatorTokenType
            | EnumToken::SubsequentSiblingCombinatorTokenType
            | EnumToken::ColumnCombinatorTokenType
            | EnumToken::UniversalSelectorTokenType
    )
}

/// parse the condition of `@supports`
pub fn parse_at_rule_support_syntax(arena: &AstArena, stream: &mut Vec<u32>, context: u32, options: &ParserOptions) -> (bool, Vec<ErrorDescription>) {
    let t = |x: u32| arena.data_types.get(x);
    let ty = |v: &Vec<u32>, i: usize| v.get(i).map(|&x| t(x));
    let mut tokens: Vec<u32> = Vec::new();
    let mut stack: Vec<u32> = Vec::new();
    let mut expect_and_or = false;
    let mut errors: Vec<ErrorDescription> = Vec::new();
    // scopes: (has and, has or)
    let mut scopes: Vec<(bool, bool)> = vec![(false, false)];
    let mut i = 0usize;
    while i < stream.len() {
        let current = stream[i];
        tokens.push(current);
        if t(current) == EnumToken::ColonTokenType {
            if ty(stream, i + 1) == Some(EnumToken::IdenTokenType) {
                let next = stream[i + 1];
                let val = arena.resolve_string(arena.string_value.get(next));
                arena.data_types.set(
                    current,
                    if PSEUDO_ELEMENTS.contains(&&*val) { EnumToken::PseudoElementTokenType } else { EnumToken::PseudoClassTokenType },
                );
                arena.string_value.set(current, arena.intern_string(&format!(":{}", val)));
                arena.set_span(current, arena.get_source_id(current), arena.get_span_start(current), arena.get_span_end(next));
                stream.remove(i + 1);
                i += 1;
                continue;
            }
            if ty(stream, i + 1) == Some(EnumToken::FunctionTokenDefType) {
                let next = stream[i + 1];
                let val = format!(":{}", arena.resolve_string(arena.string_value.get(next)));
                arena.data_types.set(
                    current,
                    if get_syntax_config()["selectors"].get(format!("{}()", val)).is_some() {
                        EnumToken::PseudoClassFunctionTokenDefType
                    } else {
                        t(next)
                    },
                );
                arena.string_value.set(current, arena.intern_string(&val));
                stack.push(current);
                arena.set_span(current, arena.get_source_id(current), arena.get_span_start(current), arena.get_span_end(next));
                stream.remove(i + 1);
                i += 1;
                continue;
            }
        }
        if trim_white_space(t(current)) {
            if tokens.len() >= 2 && t(tokens[tokens.len() - 2]) == EnumToken::WhitespaceTokenType {
                tokens.remove(tokens.len() - 2);
            }
            if ty(stream, i + 1) == Some(EnumToken::WhitespaceTokenType) {
                i += 2;
                continue;
            }
        }
        if expect_and_or {
            let mut k = i;
            while k < stream.len() && matches!(t(stream[k]), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType) {
                tokens.push(stream[k]);
                k += 1;
            }
            expect_and_or = false;
        }
        if t(current) == EnumToken::StartParensTokenType || is_function_def_type(t(current)) {
            scopes.push((false, false));
            stack.push(current);
            i += 1;
            continue;
        }
        let top = |stack: &Vec<u32>| stack.last().map(|&s| t(s));
        match t(current) {
            EnumToken::EndParensTokenType => 'case: {
                if top(&stack) == Some(EnumToken::ColonTokenType) {
                    let index = index_of(&tokens, if stack.len() >= 2 { Some(stack[stack.len() - 2]) } else { None });
                    let count = tokens.len() as isize - index - 2;
                    let mut slice = splice_trimmed(arena, &mut tokens, index + 1, count);
                    let declaration_options = ParserOptions { validation: Some(false), ..options.clone() };
                    let declaration = parse_declaration(arena, &mut slice, Some(context), &declaration_options, &mut errors);
                    let at = ((index + 1).max(0) as usize).min(tokens.len());
                    tokens.insert(at, declaration);
                    stack.pop();
                }
                if top(&stack) == Some(EnumToken::StartParensTokenType) {
                    let parens = *stack.last().unwrap();
                    let index = index_of(&tokens, Some(parens));
                    let count = tokens.len() as isize - index - 2;
                    let slice = splice_trimmed(arena, &mut tokens, index + 1, count);
                    arena.data_types.set(parens, EnumToken::ParensTokenType);
                    arena.set_token_list(parens, &slice);
                    arena.span_location(parens, parens, current);
                    stack.pop();
                    tokens.pop();
                    scopes.pop();
                    expect_and_or = true;
                } else if top(&stack).is_some_and(is_function_def_type) {
                    let func = *stack.last().unwrap();
                    let index = index_of(&tokens, Some(func));
                    arena.data_types.set(func, tokens_func_def_map(t(func)).unwrap_or(0));
                    let count = tokens.len() as isize - index - 2;
                    let slice = splice_trimmed(arena, &mut tokens, index + 1, count);
                    arena.set_token_list(func, &slice);
                    arena.span_location(func, func, current);
                    if t(tokens[index as usize]) == EnumToken::PseudoClassFuncTokenType {
                        // looked up once per function name, the children of the syntax are borrowed
                        let syntax = selector_function_syntax(arena, tokens[index as usize]);
                        let chi: Option<&[Syntax]> = syntax.as_ref().and_then(|s| s.first()).map(|f| f.children());
                        let result = match_all_syntaxes(arena, chi, &create_validation_context(arena, &arena.tokens(tokens[index as usize])), options);
                        if !result.success {
                            return (false, result.errors);
                        }
                        stack.pop();
                        tokens.pop();
                        break 'case;
                    }
                    stack.pop();
                    tokens.pop();
                    scopes.pop();
                }
                if top(&stack) == Some(EnumToken::NotTokenType) {
                    let not = *stack.last().unwrap();
                    let index = index_of(&tokens, Some(not));
                    let unary = arena.create_token(EnumToken::SupportsQueryUnaryConditionTokenType, TokenValue::None, None);
                    // typescript: `i - index - 1` mixes the index of the stream and of the tokens
                    let slice = splice_trimmed(arena, &mut tokens, index + 1, i as isize - index - 1);
                    arena.set_token_list(arena.field_list(unary, TokenField::R), &slice);
                    arena.set_token_list(arena.field_list(unary, TokenField::L), &[not]);
                    arena.span_location(unary, not, current);
                    tokens[index as usize] = unary;
                    stack.pop();
                }
                if matches!(top(&stack), Some(EnumToken::AndTokenType) | Some(EnumToken::OrTokenType)) {
                    let op = *stack.last().unwrap();
                    let index = index_of(&tokens, Some(op));
                    let index2 = if stack.len() > 1 { index_of(&tokens, Some(stack[stack.len() - 2])) + 1 } else { 0 };
                    let mut left: Vec<u32> = tokens[(index2.max(0) as usize).min(index.max(0) as usize)..index.max(0) as usize].to_vec();
                    trim_array(arena, &mut left);
                    let mut right: Vec<u32> = tokens[((index + 1).max(0) as usize).min(tokens.len())..].to_vec();
                    trim_array(arena, &mut right);
                    let condition = arena.create_token(EnumToken::SupportsQueryConditionTokenType, TokenValue::None, None);
                    arena.set_token_list(arena.field_list(condition, TokenField::L), &left);
                    arena.set_token_list(arena.field_list(condition, TokenField::R), &right);
                    arena.set_token_list(arena.field_list(condition, TokenField::Op), &[op]);
                    arena.span_location(condition, op, current);
                    let index2 = index2.max(0) as usize;
                    if index2 < tokens.len() {
                        tokens[index2] = condition;
                    } else {
                        tokens.push(condition);
                    }
                    tokens.truncate(index2 + 1);
                    stack.pop();
                }
            }
            EnumToken::ColonTokenType => stack.push(current),
            EnumToken::IdenTokenType => {
                let val = arena.resolve_string(arena.string_value.get(current)).to_lowercase();
                if val == "not" {
                    stack.push(current);
                    arena.data_types.set(current, EnumToken::NotTokenType);
                } else if val == "and" || val == "or" {
                    if val == "or" && scopes.len() == 1 && (scopes[0].0 || scopes[0].1) {
                        let file: String = options.source.as_ref().and_then(|s| s.file.as_deref().map(String::from)).unwrap_or_default();
                        let (line, column) =
                            options.source.as_ref().map_or((0, 0), |s| s.line_starts.borrow().get_offsets(arena.get_span_start(current)));
                        let mut error = ErrorDescription::drop(format!("<or> is not allowed outside of a parenthesis at {}:{}:{}", file, line, column));
                        error.location = Some((Some(file.as_str().into()), line, column));
                        return (false, vec![error]);
                    }
                    let op = if val == "or" { EnumToken::OrTokenType } else { EnumToken::AndTokenType };
                    arena.data_types.set(current, op);
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
            _ => {}
        }
        i += 1;
    }
    trim_array(arena, &mut tokens);
    *stream = tokens;
    (true, errors)
}
