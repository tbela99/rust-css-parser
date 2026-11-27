//! Port of `src/parser/utils/selector.ts`.

use crate::ast::arena::{AstArena, StrOrId, TokenValue};
use crate::ast::strings::{StringBuilder, StringDerived};
use crate::utils::pool::PooledVec;
use crate::ast::types::{EnumAstNodeStatus, EnumToken};
use crate::options::ParserOptions;
use crate::parser::utils::text::equals_ignore_case;
use crate::printer::render::render_value;
use crate::syntax::constants::{is_function_def_type, tokens_func_def_map, COMBINATORS, PSEUDO_ELEMENTS};
use crate::syntax::syntax::{is_hash, is_ident};
use crate::types::ErrorDescription;
use crate::validation::config::{get_parsed_syntax, get_syntax_rule};
use crate::validation::match_::{create_validation_context, match_all_syntaxes, match_selector_syntax, match_simple_selector, selector_function_syntax, trim_array};
use crate::validation::parser::typedef::{ValidationSyntaxGroupEnum, ValidationTokenEnum};
use crate::validation::utils::list::split_token_list_comma;
use std::cell::RefCell;

/// `trimWhiteSpace` of `parse.ts`
pub fn is_trim_white_space(t: u8) -> bool {
    matches!(
        t,
        EnumToken::CommentTokenType
            | EnumToken::GtTokenType
            | EnumToken::GteTokenType
            | EnumToken::LtTokenType
            | EnumToken::LteTokenType
            | EnumToken::ColumnCombinatorTokenType
    )
}

/// `/^(([+-]?[0-9]*)?n)?([+-]?[0-9]+)?$/`: groups 2 and 3
fn nth_groups(value: &str) -> Option<(Option<String>, Option<String>)> {
    let parse_int = |s: &str| -> bool {
        let r = s.strip_prefix(['+', '-']).unwrap_or(s);
        !r.is_empty() && r.bytes().all(|c| c.is_ascii_digit())
    };
    if let Some(n) = value.find('n') {
        let prefix = &value[..n];
        let digits = prefix.strip_prefix(['+', '-']).unwrap_or(prefix);
        if digits.bytes().all(|c| c.is_ascii_digit()) {
            let rest = &value[n + 1..];
            if rest.is_empty() {
                return Some((Some(prefix.to_string()), None));
            }
            if parse_int(rest) {
                return Some((Some(prefix.to_string()), Some(rest.to_string())));
            }
        }
    }
    if value.is_empty() {
        return Some((None, None));
    }
    if parse_int(value) {
        return Some((None, Some(value.to_string())));
    }
    None
}

fn js_sign(n: f64) -> f64 {
    if n > 0.0 {
        1.0
    } else if n < 0.0 {
        -1.0
    } else {
        n
    }
}

/// minify the an+b expression of a pseudo class function
fn minify_nth_expression(arena: &AstArena, func: u32) {
    let t = |x: u32| arena.data_types.get(x);
    // pooled buffers: no allocation per selector function
    let mut chi = PooledVec::new();
    chi.extend(arena.token_iter(func));
    let mut list = PooledVec::new();
    let mut index: isize = 0;
    while (index as usize) < chi.len() {
        let c = chi[index as usize];
        if matches!(t(c), EnumToken::CommentTokenType | EnumToken::WhitespaceTokenType) {
            index += 1;
            continue;
        }
        if t(c) == EnumToken::IdenTokenType && arena.string_equals_ignore_case(arena.string_value.get(c), "of") {
            index -= 1;
            break;
        }
        list.push(c);
        index += 1;
    }
    let splice_start = |chi: &mut Vec<u32>, index: isize, items: &[u32]| {
        let n = index.clamp(0, chi.len() as isize) as usize;
        chi.splice(0..n, items.iter().copied());
    };
    if list.len() == 2 {
        if t(list[1]) == EnumToken::NumberTokenType {
            if arena.numeric_value.get(list[1]) == 0.0 {
                list.truncate(1);
                if t(list[0]) == EnumToken::DimensionTokenType && arena.numeric_value.get(list[0]) == -2.0 {
                    arena.numeric_value.set(list[0], 2.0);
                }
            } else {
                let sign = js_sign(arena.numeric_value.get(list[1]));
                arena.numeric_value.set(list[1], arena.numeric_value.get(list[1]) * sign);
                let literal = arena.create_token(EnumToken::LiteralTokenType, TokenValue::Str(if sign > 0.0 { "+" } else { "-" }), None);
                list.insert(1, literal);
            }
        }
        if list.len() == 3
            && t(list[2]) == EnumToken::NumberTokenType
            && t(list[0]) == EnumToken::DimensionTokenType
            && (arena.numeric_value.get(list[0]) == 2.0 || arena.numeric_value.get(list[0]) == -2.0)
        {
            let name = if arena.numeric_value.get(list[2]) == 1.0 {
                Some("odd")
            } else if arena.numeric_value.get(list[2]) == 0.0 {
                Some("even")
            } else {
                None
            };
            if let Some(name) = name {
                let ident = arena.create_token(EnumToken::IdenTokenType, TokenValue::Str(name), None);
                arena.copy_location(ident, list[0]);
                list.splice(0..3, [ident]);
            }
        }
        splice_start(&mut chi, index, &list);
    }
    if list.len() == 1 && t(list[0]) == EnumToken::IdenTokenType && arena.string_equals_ignore_case(arena.string_value.get(list[0]), "-n") {
        arena.string_value.set(list[0], arena.intern_string("n"));
    }
    if list.len() == 3
        && t(list[0]) == EnumToken::IdenTokenType
        && arena.with_string(arena.string_value.get(list[0]), |v| matches!(v, "n" | "-n" | "+n"))
        && t(list[1]) == EnumToken::NextSiblingCombinatorTokenType
        && t(list[2]) == EnumToken::NumberTokenType
        && arena.numeric_value.get(list[2]) == 0.0
    {
        arena.string_value.set(list[0], arena.intern_string("n"));
        splice_start(&mut chi, index, &[list[0]]);
        arena.set_token_list(func, &chi);
        return;
    }
    let token = chi.iter().copied().find(|&x| !matches!(t(x), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType));
    if let Some(token) = token {
        if matches!(t(token), EnumToken::IdenTokenType | EnumToken::LiteralTokenType) {
            let val = arena.resolve_string(arena.string_value.get(token));
            if t(token) == EnumToken::IdenTokenType && (&*val == "odd" || &*val == "even") {
                if &*val == "even" {
                    arena.reset_token(token, EnumToken::DimensionTokenType, TokenValue::Num(2.0), Some("n"));
                }
            } else if let Some((g2, g3)) = nth_groups(&val) {
                // `matches[2] === "" ? 1 : matches[2] === "-" ? -1 : +matches[2]` (undefined -> NaN)
                let a1 = match g2.as_deref() {
                    Some("") => 1.0,
                    Some("-") => -1.0,
                    Some(s) => crate::utils::number::js_parse_number(s),
                    None => f64::NAN,
                };
                let b1 = g3.map_or(f64::NAN, |s| crate::utils::number::js_parse_number(&s));
                if b1 == 0.0 {
                    if a1.abs() == 1.0 {
                        arena.reset_token(token, EnumToken::IdenTokenType, TokenValue::Str("n"), None);
                    } else {
                        arena.reset_token(token, EnumToken::DimensionTokenType, TokenValue::Num(a1 * js_sign(a1)), Some("n"));
                    }
                }
            }
        } else if t(token) == EnumToken::DimensionTokenType {
            let index = chi.iter().position(|&x| x == token).unwrap();
            let mut num: Option<u32> = None;
            let mut i = index + 1;
            while i < chi.len() {
                if matches!(t(chi[i]), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType) {
                    i += 1;
                    continue;
                }
                if t(chi[i]) == EnumToken::NumberTokenType {
                    num = Some(chi[i]);
                    break;
                }
                i += 1;
            }
            if let Some(num) = num {
                if arena.numeric_value.get(num) == 0.0 {
                    let end = (i + 1).min(chi.len());
                    chi.drain(index + 1..end);
                    if arena.numeric_value.get(token) < 0.0 {
                        arena.numeric_value.set(token, arena.numeric_value.get(token) * -1.0);
                    }
                } else if arena.numeric_value.get(token) == 2.0 && arena.numeric_value.get(num).abs() == 1.0 {
                    arena.reset_token(token, EnumToken::IdenTokenType, TokenValue::Str("odd"), None);
                    let end = (i + 1).min(chi.len());
                    chi.drain(index + 1..end);
                }
            }
        }
    }
    arena.set_token_list(func, &chi);
}

/// split the unicode ranges of a selector: `u+a` is the element `u` followed by the element `a`
fn split_unicode_ranges(arena: &AstArena, tokens: &mut Vec<u32>) {
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];
        if arena.data_types.get(token) == EnumToken::UniCodeRange {
            let text = arena.resolve_string(arena.string_value.get(token));
            let first: String = text.chars().take(1).collect();
            let rest: String = text.chars().skip(2).collect();
            let split = [
                arena.create_token(EnumToken::IdenTokenType, TokenValue::Str(&first), None),
                arena.create_token(EnumToken::Plus, TokenValue::Str("+"), None),
                arena.create_token(if is_ident(&rest) { EnumToken::IdenTokenType } else { EnumToken::LiteralTokenType }, TokenValue::Str(&rest), None),
            ];
            for &t in &split {
                arena.copy_location(t, token);
            }
            tokens.splice(i..i + 1, split);
            i += split.len() - 1;
        } else if arena.first_token.get(token) != 0 {
            let mut children = arena.tokens(token);
            if children.iter().any(|&c| arena.data_types.get(c) == EnumToken::UniCodeRange || arena.first_token.get(c) != 0) {
                let length = children.len();
                split_unicode_ranges(arena, &mut children);
                if children.len() != length {
                    arena.set_token_list(token, &children);
                }
            }
        }
        i += 1;
    }
}

fn source_of(arena: &AstArena, token: u32) -> u32 {
    arena.sources.borrow().get(arena.get_source_id(token) as usize).copied().unwrap_or(0)
}

fn locate(arena: &AstArena, node: u32, tokens: &[u32]) {
    let (Some(&first), Some(&last)) = (tokens.first(), tokens.last()) else {
        // typescript: `arena.getSourceId(undefined)` reads index 0
        arena.set_location(node, Some(source_of(arena, 0)), Some(arena.get_span_start(0)), Some(arena.get_span_end(0)));
        return;
    };
    arena.set_location(node, Some(source_of(arena, first)), Some(arena.get_span_start(first)), Some(arena.get_span_end(last)));
}

/// true if the token is a keyframe selector: `from`, `to` or a percentage between 0 and 100
fn is_keyframe_selector_token(arena: &AstArena, token: u32) -> bool {
    match arena.data_types.get(token) {
        EnumToken::PercentageTokenType => (0.0..=100.0).contains(&arena.numeric_value.get(token)),
        EnumToken::IdenTokenType => {
            let id = arena.string_value.get(token);
            let code = |k: usize| arena.string_code_at(id, k) | 0x20;
            // `from` and `to`, case-insensitive
            match arena.string_length(id) {
                4 => code(0) == 0x66 && code(1) == 0x72 && code(2) == 0x6f && code(3) == 0x6d,
                2 => code(0) == 0x74 && code(1) == 0x6f,
                _ => false,
            }
        }
        _ => false,
    }
}

/// true if the selector of a keyframe is a list of `from`, `to` and percentages between 0 and 100, separated by commas
/// and whitespace: it matches the `keyframe-selectors` syntax
fn is_simple_keyframe_selector(arena: &AstArena, tokens: &[u32]) -> bool {
    // a keyframe selector is expected
    let mut selector = true;
    for &token in tokens {
        let typ = arena.data_types.get(token);
        if typ == EnumToken::WhitespaceTokenType {
            continue;
        }
        if selector {
            if !is_keyframe_selector_token(arena, token) {
                return false;
            }
        } else if typ != EnumToken::CommaTokenType {
            return false;
        }
        selector = !selector;
    }
    !selector
}

/// parse a selector: a rule or a keyframes rule
pub fn parse_selector(arena: &AstArena, tokens: &mut Vec<u32>, context: Option<u32>, options: &ParserOptions, errors: &mut Vec<ErrorDescription>) -> u32 {
    let t = |x: u32| arena.data_types.get(x);
    let context_type = context.map(t);
    split_unicode_ranges(arena, tokens);
    if context_type == Some(EnumToken::KeyframesAtRuleNodeType) {
        // `from`, `to` and the percentages are valid: the syntax is not matched
        let (success, result_errors) = if is_simple_keyframe_selector(arena, tokens) {
            (true, Vec::new())
        } else {
            let syntax = get_parsed_syntax(ValidationSyntaxGroupEnum::Syntaxes, "keyframe-selectors");
            let result = match_all_syntaxes(arena, syntax.as_deref().map(|v| v.as_slice()), &create_validation_context(arena, tokens), options);
            (result.success, result.errors)
        };
        let mut parts = split_token_list_comma(arena, tokens);
        for part in parts.iter_mut() {
            trim_array(arena, part);
            if options.minify == Some(true) {
                let filtered: Vec<u32> = part.iter().copied().filter(|&x| !matches!(t(x), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType)).collect();
                if filtered.len() == 1 {
                    let f = filtered[0];
                    if t(f) == EnumToken::IdenTokenType && arena.with_string(arena.string_value.get(f), |v| v.eq_ignore_ascii_case("from")) {
                        arena.reset_token(f, EnumToken::PercentageTokenType, TokenValue::Num(0.0), None);
                    } else if t(f) == EnumToken::PercentageTokenType && arena.numeric_value.get(f) == 100.0 {
                        arena.reset_token(f, EnumToken::IdenTokenType, TokenValue::Str("to"), None);
                    }
                    *part = filtered;
                }
            }
        }
        let mut joined: Vec<u32> = Vec::new();
        for part in &parts {
            if !joined.is_empty() {
                joined.push(arena.create_token(EnumToken::CommaTokenType, TokenValue::None, None));
            }
            joined.extend_from_slice(part);
        }
        *tokens = joined;
        trim_array(arena, tokens);
        let render_options = ParserOptions { minify: Some(false), ..Default::default() };
        let mut seen: Vec<String> = Vec::new();
        for part in split_token_list_comma(arena, tokens) {
            let s: String = part.iter().map(|&x| render_value(arena, x, &render_options)).collect();
            if !seen.contains(&s) {
                seen.push(s);
            }
        }
        let sel = seen.join(",");
        let node = arena.create_rule(
            EnumToken::KeyframesRuleNodeType,
            StrOrId::Str(&sel),
            if tokens.is_empty() { None } else { Some(tokens) },
            if success { EnumAstNodeStatus::Validated } else { EnumAstNodeStatus::Invalid },
            Some(result_errors),
        );
        locate(arena, node, tokens);
        return node;
    }
    let mut stack = PooledVec::new();
    let mut allowed = true;
    let mut nested = false;
    if context_type != Some(EnumToken::StyleSheetNodeType) && context_type != Some(EnumToken::RuleNodeType) {
        allowed = false;
        if context_type == Some(EnumToken::AtRuleNodeType) {
            // computed once per at-rule name: the name is not formatted for each selector
            allowed = arena.string_derived(arena.name.get(context.unwrap()), StringDerived::AtRuleAcceptsRules, |id| {
                let rule = get_syntax_rule(ValidationSyntaxGroupEnum::AtRules, &format!("@{}", arena.resolve_string(id)));
                rule.as_ref().is_some_and(|r| r.accept_any_rule)
                    || rule.and_then(|r| r.get_block_rules()).is_some_and(|rules| {
                        rules.iter().any(|r| r.typ == ValidationTokenEnum::PropertyType && matches!(r.val_str(), "block-contents" | "rule-list"))
                    })
            });
        }
    }
    let mut parent = context.unwrap_or(0);
    while parent > 0 {
        if t(parent) == EnumToken::AtRuleNodeType && arena.string_equals(arena.name.get(parent), "media") {
            parent = arena.parents.get(parent);
            continue;
        }
        nested = t(parent) == EnumToken::RuleNodeType || (t(parent) == EnumToken::AtRuleNodeType && arena.string_equals(arena.name.get(parent), "mixin"));
        if nested {
            break;
        }
        parent = arena.parents.get(parent);
    }
    // a pooled buffer: it is swapped with the tokens, the buffer of the tokens returns to the pool
    let mut all = PooledVec::new();
    all.reserve(tokens.len());
    let mut read = 0;
    while read < tokens.len() {
        let token = tokens[read];
        let next = tokens.get(read + 1).copied();
        let nt = next.map(t);
        if t(token) == EnumToken::ColonTokenType {
            if nt == Some(EnumToken::IdenTokenType) {
                let next = next.unwrap();
                arena.data_types.set(token, EnumToken::PseudoElementTokenType);
                let id = with_builder(|b| {
                    b.reset().append_code(0x3a);
                    arena.append_string(b, arena.string_value.get(next));
                    arena.intern_builder(b)
                });
                arena.string_value.set(token, id);
                arena.span_location(token, token, next);
                all.push(token);
                read += 2;
                continue;
            }
            if nt == Some(EnumToken::FunctionTokenDefType) {
                let next = next.unwrap();
                let id = with_builder(|b| {
                    b.reset().append_code(0x3a);
                    arena.append_string(b, arena.string_value.get(next));
                    arena.intern_builder(b)
                });
                arena.string_value.set(token, id);
                // `:name()` is a selector function: looked up once per name
                arena.data_types.set(
                    token,
                    if selector_function_syntax(arena, token).is_some() { EnumToken::PseudoClassFunctionTokenDefType } else { t(next) },
                );
                arena.span_location(token, token, next);
                all.push(token);
                read += 2;
                continue;
            }
        }
        if t(token) == EnumToken::DoubleColonTokenType {
            if nt == Some(EnumToken::IdenTokenType) {
                let next = next.unwrap();
                arena.data_types.set(token, EnumToken::PseudoClassTokenType);
                let single = is_single_colon_pseudo_element(arena, arena.string_value.get(next));
                let id = with_builder(|b| {
                    b.reset().append_code(0x3a);
                    if !single {
                        b.append_code(0x3a);
                    }
                    arena.append_string(b, arena.string_value.get(next));
                    arena.intern_builder(b)
                });
                arena.string_value.set(token, id);
                arena.span_location(token, token, next);
                all.push(token);
                read += 2;
                continue;
            }
            if nt == Some(EnumToken::FunctionTokenDefType) {
                let next = next.unwrap();
                let id = with_builder(|b| {
                    b.reset().append_code(0x3a).append_code(0x3a);
                    arena.append_string(b, arena.string_value.get(next));
                    arena.intern_builder(b)
                });
                arena.string_value.set(token, id);
                arena.data_types.set(
                    token,
                    if selector_function_syntax(arena, token).is_some() { EnumToken::PseudoClassFunctionTokenDefType } else { EnumToken::FunctionTokenDefType },
                );
                arena.span_location(token, token, next);
                all.push(token);
                read += 2;
                continue;
            }
        }
        if t(token) == EnumToken::ColorTokenType {
            if arena.with_string(arena.string_value.get(token), is_hash) {
                arena.data_types.set(token, EnumToken::HashTokenType);
                all.push(token);
                read += 1;
                continue;
            }
            let sel = render_selector(arena, tokens);
            let node = arena.create_rule(
                EnumToken::RuleNodeType,
                StrOrId::Id(sel),
                Some(tokens),
                EnumAstNodeStatus::Invalid,
                Some(vec![ErrorDescription::drop("invalid hash id").with_node(token)]),
            );
            locate(arena, node, tokens);
            return node;
        }
        all.push(token);
        read += 1;
    }
    std::mem::swap(tokens, &mut *all);
    drop(all);
    // a selector made of simple selectors is valid, it has no attribute selector and no function
    let simple = match_simple_selector(arena, tokens);
    let (success, result_errors) = if simple { (true, Vec::new()) } else { match_selector_syntax(arena, tokens, errors, options, nested) };
    trim_array(arena, tokens);
    if success && !simple {
        let mut i: isize = 0;
        while (i as usize) < tokens.len() {
            let token = tokens[i as usize];
            match t(token) {
                EnumToken::AttrStartTokenType | EnumToken::PseudoClassFunctionTokenDefType => stack.push(token),
                EnumToken::AttrEndTokenType => {
                    if stack.last().is_some_and(|&s| t(s) == EnumToken::AttrStartTokenType) {
                        let attr = *stack.last().unwrap();
                        let index = tokens.iter().position(|&x| x == attr).unwrap() as isize;
                        arena.span_location(attr, attr, token);
                        tokens.remove(i as usize);
                        arena.data_types.set(attr, EnumToken::AttrTokenType);
                        arena.string_value.set(attr, 0);
                        // the tokens are read in place, then removed
                        arena.set_token_list(attr, &tokens[(index + 1) as usize..i as usize]);
                        tokens.drain((index + 1) as usize..i as usize);
                        i = index;
                        stack.pop();
                    }
                }
                EnumToken::EndParensTokenType => {
                    if stack.last().is_some_and(|&s| t(s) == EnumToken::PseudoClassFunctionTokenDefType) {
                        let func = *stack.last().unwrap();
                        let index = tokens.iter().position(|&x| x == func).unwrap() as isize;
                        arena.span_location(func, func, token);
                        tokens.remove(i as usize);
                        if is_function_def_type(t(func)) {
                            arena.data_types.set(func, tokens_func_def_map(t(func)).unwrap_or(0));
                            // the trimmed range is read in place, then the tokens are removed
                            let range = &tokens[(index + 1) as usize..i as usize];
                            let ws = |x: &u32| t(*x) == EnumToken::WhitespaceTokenType;
                            let start = range.iter().position(|x| !ws(x)).unwrap_or(range.len());
                            let end = range.iter().rposition(|x| !ws(x)).map_or(start, |e| e + 1);
                            arena.set_token_list(func, &range[start..end]);
                            tokens.drain((index + 1) as usize..i as usize);
                        }
                        if options.minify == Some(true) {
                            let is_nth = arena.with_string(arena.string_value.get(func), |n| {
                                matches!(n, ":nth-child" | ":nth-last-child" | ":nth-of-type" | ":nth-last-of-type")
                            });
                            if is_nth {
                                minify_nth_expression(arena, func);
                            }
                        }
                        i = index;
                        stack.pop();
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    let sel = if simple { render_simple_selector(arena, tokens) } else { render_selector(arena, tokens) };
    let state = if success && allowed {
        EnumAstNodeStatus::Validated
    } else if allowed {
        EnumAstNodeStatus::Invalid
    } else {
        EnumAstNodeStatus::Disallowed
    };
    let node = arena.create_rule(EnumToken::RuleNodeType, StrOrId::Id(sel), Some(tokens), state, Some(if success { Vec::new() } else { result_errors }));
    locate(arena, node, tokens);
    node
}

/// buffers of the selector rendering, reused
struct Buffers {
    pseudo: StringBuilder,
    part: StringBuilder,
    selector: StringBuilder,
    part_offsets: Vec<usize>,
    selector_offsets: Vec<usize>,
}

thread_local! {
    static BUFFERS: RefCell<Buffers> = RefCell::new(Buffers {
        pseudo: StringBuilder::new(),
        part: StringBuilder::new(),
        selector: StringBuilder::new(),
        part_offsets: Vec::with_capacity(64),
        selector_offsets: Vec::with_capacity(16),
    });
}

fn with_builder<R>(f: impl FnOnce(&mut StringBuilder) -> R) -> R {
    let mut builder = BUFFERS.with(|b| std::mem::take(&mut b.borrow_mut().pseudo));
    let result = f(&mut builder);
    BUFFERS.with(|b| b.borrow_mut().pseudo = builder);
    result
}

/// true if ":" + the string is a pseudo element with a single colon syntax (`:before`)
fn is_single_colon_pseudo_element(arena: &AstArena, id: u32) -> bool {
    let length = arena.string_length(id);
    PSEUDO_ELEMENTS.iter().any(|pseudo| {
        let bytes = pseudo.as_bytes();
        bytes.len() == length + 1 && bytes[0] == b':' && (0..length).all(|k| arena.string_code_at(id, k) == bytes[k + 1] as u32)
    })
}

/// true if the string of a token is a combinator
fn is_combinator_string(arena: &AstArena, token: Option<u32>) -> bool {
    let Some(token) = token else {
        return false;
    };
    let id = arena.string_value.get(token);
    let length = arena.string_length(id);
    if length == 0 || length > 2 {
        return false;
    }
    COMBINATORS.iter().any(|c| arena.string_equals(id, c))
}

/// append the rendering of a selector token to the builder
fn append_selector_token(arena: &AstArena, token: u32, builder: &mut StringBuilder) {
    if arena.first_token.get(token) == 0 && arena.first_child.get(token) == 0 {
        let code = match arena.data_types.get(token) {
            EnumToken::HashTokenType
            | EnumToken::IdenTokenType
            | EnumToken::LiteralTokenType
            | EnumToken::DashedIdenTokenType
            | EnumToken::ClassSelectorTokenType
            | EnumToken::PseudoClassTokenType => {
                arena.append_string(builder, arena.string_value.get(token));
                return;
            }
            EnumToken::WhitespaceTokenType | EnumToken::DescendantCombinatorTokenType => Some(0x20),
            EnumToken::CommaTokenType => Some(0x2c),
            EnumToken::Star | EnumToken::UniversalSelectorTokenType => Some(0x2a),
            EnumToken::NestingSelectorTokenType => Some(0x26),
            EnumToken::GtTokenType | EnumToken::ChildCombinatorTokenType => Some(0x3e),
            EnumToken::Plus | EnumToken::NextSiblingCombinatorTokenType => Some(0x2b),
            EnumToken::Tilda | EnumToken::SubsequentSiblingCombinatorTokenType => Some(0x7e),
            _ => None,
        };
        if let Some(code) = code {
            builder.append_code(code);
            return;
        }
    }
    let options = ParserOptions { minify: Some(false), ..Default::default() };
    builder.append_string(&render_value(arena, token, &options));
}

fn is_part_code(b: &Buffers, index: isize, count: usize, code: u16) -> bool {
    if index < 0 || index as usize >= count {
        return false;
    }
    let i = index as usize;
    let start = b.part_offsets[i * 2];
    b.part_offsets[i * 2 + 1] - start == 1 && b.part.codes[start] == code
}

fn is_subclass_part(b: &Buffers, index: usize, count: usize) -> bool {
    if index >= count || b.part_offsets[index * 2 + 1] == b.part_offsets[index * 2] {
        return false;
    }
    let code = b.part.codes[b.part_offsets[index * 2]];
    matches!(code, 0x2e | 0x23 | 0x3a | 0x5b)
}

fn is_type_part(b: &Buffers, index: usize, count: usize) -> bool {
    if index >= count {
        return false;
    }
    let (start, end) = (b.part_offsets[index * 2], b.part_offsets[index * 2 + 1]);
    let code = b.part.codes.get(start).copied().unwrap_or(0);
    if is_part_code(b, index as isize + 1, count, 0x7c) {
        return false;
    }
    if !((0x61..=0x7a).contains(&code) || (0x41..=0x5a).contains(&code) || code == 0x5f || code >= 0x80) {
        return false;
    }
    let c = &b.part.codes;
    !(end - start == 4 && (c[start] | 0x20) == 0x68 && (c[start + 1] | 0x20) == 0x74 && (c[start + 2] | 0x20) == 0x6d && (c[start + 3] | 0x20) == 0x6c)
}

/// add the current selector to the selectors of the rule when it is not already present
fn add_selector_codes(b: &mut Buffers, mut count: usize, selectors: usize) -> usize {
    let mut i = 0;
    while i + 1 < count {
        if is_part_code(b, i as isize, count, 0x2a) && !is_part_code(b, i as isize - 1, count, 0x7c) && is_subclass_part(b, i + 1, count) {
            b.part_offsets.copy_within((i + 1) * 2..count * 2, i * 2);
            count -= 1;
        }
        i += 1;
    }
    if count > 2 && is_part_code(b, 0, count, 0x2a) && is_part_code(b, 1, count, 0x20) && is_type_part(b, 2, count) {
        b.part_offsets.copy_within(4..count * 2, 0);
        count -= 2;
    }
    let start = b.selector.len() + if selectors > 0 { 1 } else { 0 };
    if selectors > 0 {
        b.selector.append_code(0x2c);
    }
    for i in 0..count {
        let (ts, te) = (b.part_offsets[i * 2], b.part_offsets[i * 2 + 1]);
        for k in ts..te {
            let c = b.part.codes[k];
            b.selector.append_code(c);
        }
    }
    let end = b.selector.len();
    for k in 0..selectors {
        let (s, e) = (b.selector_offsets[k * 2], b.selector_offsets[k * 2 + 1]);
        if e - s == end - start && b.selector.codes[s..e] == b.selector.codes[start..end] {
            b.selector.codes.truncate(if selectors > 0 { start - 1 } else { start });
            return selectors;
        }
    }
    if b.selector_offsets.len() < (selectors + 1) * 2 {
        b.selector_offsets.resize((selectors + 1) * 2, 0);
    }
    b.selector_offsets[selectors * 2] = start;
    b.selector_offsets[selectors * 2 + 1] = end;
    selectors + 1
}

/// selector of a rule made of simple selectors (see [`match_simple_selector`]), without the leading and the trailing
/// whitespace: the string of a single token is used as is, a compound or a complex selector is built without
/// [`render_selector`]. a selector list is rendered with [`render_selector`]: the duplicate selectors are removed
fn render_simple_selector(arena: &AstArena, tokens: &[u32]) -> u32 {
    // rendered as their string by append_selector_token()
    if tokens.len() == 1 && arena.data_types.get(tokens[0]) != EnumToken::PseudoElementTokenType {
        return arena.string_value.get(tokens[0]);
    }
    if tokens.iter().any(|&x| arena.data_types.get(x) == EnumToken::CommaTokenType) {
        return render_selector(arena, tokens);
    }
    let mut selector = BUFFERS.with(|b| std::mem::take(&mut b.borrow_mut().selector));
    selector.reset();
    for &token in tokens {
        append_selector_token(arena, token, &mut selector);
    }
    let id = arena.intern_builder(&selector);
    BUFFERS.with(|b| b.borrow_mut().selector = selector);
    id
}

/// selector of a rule: the distinct comma separated selectors, interned. no string is created
pub fn render_selector(arena: &AstArena, tokens: &[u32]) -> u32 {
    let mut b = BUFFERS.with(|b| {
        let mut buffers = b.borrow_mut();
        Buffers {
            pseudo: std::mem::take(&mut buffers.pseudo),
            part: std::mem::take(&mut buffers.part),
            selector: std::mem::take(&mut buffers.selector),
            part_offsets: std::mem::take(&mut buffers.part_offsets),
            selector_offsets: std::mem::take(&mut buffers.selector_offsets),
        }
    });
    let mut count = 0usize;
    let mut selectors = 0usize;
    b.part.reset();
    b.selector.reset();
    b.part_offsets.clear();
    let typ = |i: isize| if i < 0 { None } else { tokens.get(i as usize).map(|&x| arena.data_types.get(x)) };
    for (index, &curr) in tokens.iter().enumerate() {
        let i = index as isize;
        let t = arena.data_types.get(curr);
        if t == EnumToken::WhitespaceTokenType
            && (typ(i - 1).is_some_and(is_trim_white_space)
                || typ(i + 1).is_some_and(is_trim_white_space)
                || is_combinator_string(arena, if index > 0 { Some(tokens[index - 1]) } else { None })
                || is_combinator_string(arena, tokens.get(index + 1).copied()))
        {
            continue;
        }
        if t == EnumToken::CommaTokenType {
            selectors = add_selector_codes(&mut b, count, selectors);
            count = 0;
            b.part.reset();
            b.part_offsets.clear();
            continue;
        }
        let start = b.part.len();
        let mut part = std::mem::take(&mut b.part);
        append_selector_token(arena, curr, &mut part);
        b.part = part;
        if b.part.len() - start == 1 && b.part.codes[start] == 0x2c {
            b.part.codes.truncate(start);
            selectors = add_selector_codes(&mut b, count, selectors);
            count = 0;
            b.part.reset();
            b.part_offsets.clear();
            continue;
        }
        if b.part_offsets.len() < (count + 1) * 2 {
            b.part_offsets.resize((count + 1) * 2, 0);
        }
        b.part_offsets[count * 2] = start;
        b.part_offsets[count * 2 + 1] = b.part.len();
        count += 1;
    }
    add_selector_codes(&mut b, count, selectors);
    let id = arena.intern_builder(&b.selector);
    BUFFERS.with(|buffers| {
        let mut buffers = buffers.borrow_mut();
        buffers.pseudo = b.pseudo;
        buffers.part = b.part;
        buffers.selector = b.selector;
        buffers.part_offsets = b.part_offsets;
        buffers.selector_offsets = b.selector_offsets;
    });
    id
}

#[allow(dead_code)]
fn _unused(a: &str, b: &str) -> bool {
    equals_ignore_case(a, b)
}
