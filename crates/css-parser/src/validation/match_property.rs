//! Port of `matchProperty()` (the built-in types) and `matchRepeatableSyntax()` of `src/validation/match.ts`.

use super::config::{get_parsed_syntax, get_syntax, get_syntax_config, has_syntax};
use super::match_::{
    create_validation_context, is_css_wide_keyword, match_selector_syntax, Context, ContextExt, Syntax, ValidationMatch, function_syntax,
};
use super::match_syntax::{match_syntax, peek_feature_tokens};
use super::parser::typedef::ValidationSyntaxGroupEnum;
use crate::ast::arena::{AstArena, TokenField};
use crate::ast::types::{ColorType, EnumToken};
use crate::options::ParserOptions;
use crate::parser::utils::text::equals_ignore_case;
use crate::syntax::constants::{is_function_def_type, mFGT, mFLT};
use crate::syntax::syntax::is_color;
use crate::types::ErrorDescription;
use std::cell::RefCell;
use crate::utils::fx::{HashMap, HashSet};
use std::rc::Rc;

fn ok(context: &Context, token: Option<u32>) -> ValidationMatch {
    ValidationMatch::new(true, true, token, Some(context.clone()), None, Vec::new())
}

fn ko(context: &Context, token: Option<u32>, property: &Syntax) -> ValidationMatch {
    ValidationMatch::new(false, true, token, Some(context.clone()), Some(property.clone()), Vec::new())
}

/// first item of a function syntax (`name()`): the arguments are its children, borrowed by the callers
fn function_head(syntax: Option<Rc<Vec<Syntax>>>) -> Option<Syntax> {
    syntax.and_then(|s| s.first().cloned())
}

fn is_integer(n: f64) -> bool {
    n.is_finite() && n.fract() == 0.0
}

/// `/^(([+-]?[0-9]*)?n)?([+-]?[0-9]+)?$/`: None when it does not match, Some(has the third group)
fn match_an_plus_b(value: &str) -> Option<bool> {
    let b = value.as_bytes();
    // try the `...n` prefix (greedy), then the optional integer
    let rest_after_n = |start: usize| -> Option<bool> {
        let r = &b[start..];
        if r.is_empty() {
            return Some(false);
        }
        let mut i = 0;
        if matches!(r[0], b'+' | b'-') {
            i = 1;
        }
        if i < r.len() && r[i..].iter().all(|c| c.is_ascii_digit()) {
            Some(true)
        } else {
            None
        }
    };
    if let Some(n) = b.iter().position(|&c| c == b'n') {
        let prefix = &b[..n];
        let mut i = 0;
        if !prefix.is_empty() && matches!(prefix[0], b'+' | b'-') {
            i = 1;
        }
        if prefix[i..].iter().all(|c| c.is_ascii_digit()) {
            if let Some(g3) = rest_after_n(n + 1) {
                return Some(g3);
            }
        }
    }
    rest_after_n(0)
}

thread_local! {
    static PROPERTY_KEYWORD_VALUES: RefCell<HashMap<String, Rc<HashSet<String>>>> = RefCell::new(HashMap::default());
}

/// matches a built-in type (`<length>`, `<color>`, ...) or the syntax of a type
pub fn match_property(arena: &AstArena, property: &Syntax, context: &Context, options: &ParserOptions) -> ValidationMatch {
    let mut success = false;
    let token = context.peek(arena);
    let typ = |t: Option<u32>| t.map_or(0u8, |t| arena.data_types.get(t));
    let t = typ(token);
    let val = property.val_str();
    let str_of = |tk: u32| arena.resolve_string(arena.string_value.get(tk));
    let mut check_calc = (t == EnumToken::MathFunctionTokenDefType || t == EnumToken::MathFunctionTokenType)
        && ["number", "zero", "integer", "percentage", "dimension", "length-percentage", "length", "time", "calc-sum", "calc-product"]
            .contains(&val);
    if check_calc && !["number", "zero", "integer", "percentage", "length-percentage"].contains(&val) {
        check_calc = token.is_some_and(|tk| arena.string_equals(arena.string_value.get(tk), "calc"));
    }
    if check_calc {
        let tk = token.unwrap();
        // looked up once per function name
        let head = function_head(function_syntax(arena, tk));
        let syntax = head.as_ref().map(|f| f.children());
        if t == EnumToken::MathFunctionTokenType {
            let r = match_syntax(arena, syntax, &create_validation_context(arena, &arena.tokens(tk)), options);
            if r.success && r.ctx().done(arena) {
                context.next_token(arena);
                return ok(context, token);
            }
        } else {
            let range = context.borrow().peek_range(arena, EnumToken::EndParensTokenType, 0);
            let inner: &[u32] = if range.len() >= 2 { &range[1..range.len() - 1] } else { &[] };
            let r = match_syntax(arena, syntax, &create_validation_context(arena, inner), options);
            if r.success {
                context.update(range.last().copied());
                return ok(context, range.last().copied());
            }
        }
        return ko(context, context.peek(arena), property);
    }
    match val {
        "declaration-value" => {
            let mut expect_token = true;
            let mut consumed = 0;
            let mut stack: Vec<u32> = Vec::new();
            while let Some(tk) = context.peek(arena) {
                let tt = arena.data_types.get(tk);
                if tt == EnumToken::StartParensTokenType || is_function_def_type(tt) {
                    stack.push(tk);
                }
                expect_token = false;
                if tt == EnumToken::CommaTokenType {
                    expect_token = true;
                }
                if tt == EnumToken::EndParensTokenType {
                    if stack.pop().is_none() {
                        expect_token = false;
                        break;
                    }
                }
                context.next_token(arena);
                consumed += 1;
            }
            success = consumed > 0 && !expect_token;
            if success {
                return ValidationMatch::new(true, true, None, Some(context.clone()), Some(property.clone()), Vec::new());
            }
        }
        "calc-sum" => {
            let calc_sum = get_parsed_syntax(ValidationSyntaxGroupEnum::Syntaxes, "calc-sum");
            if typ(token) == EnumToken::BinaryExpressionTokenType {
                let tk = token.unwrap();
                let mut r = match_syntax(arena, calc_sum.as_deref().map(|v| v.as_slice()), &create_validation_context(arena, &[arena.field(tk, TokenField::L)]), options);
                if r.success {
                    r = match_syntax(arena, calc_sum.as_deref().map(|v| v.as_slice()), &create_validation_context(arena, &[arena.field(tk, TokenField::R)]), options);
                }
                success = r.success;
            } else {
                let r = match_syntax(arena, calc_sum.as_deref().map(|v| v.as_slice()), &context.slice(), options);
                success = r.success;
                if success {
                    if r.ctx().done(arena) {
                        context.end();
                    } else {
                        context.update(r.ctx().current());
                    }
                    return ok(context, None);
                }
            }
        }
        "an+b" => 'anb: {
            let tk = token;
            let tt = typ(tk);
            if tt == EnumToken::IdenTokenType && tk.is_some_and(|tk| matches!(&*str_of(tk), "even" | "odd")) {
                success = true;
                break 'anb;
            }
            if tt == EnumToken::NumberTokenType && is_integer(arena.numeric_value.get(tk.unwrap())) {
                success = true;
                break 'anb;
            }
            if tt == EnumToken::LiteralTokenType || tt == EnumToken::IdenTokenType {
                let m = match_an_plus_b(&str_of(tk.unwrap()));
                success = m.is_some();
                if m == Some(false) {
                    let sign = context.borrow().peek(arena, 1);
                    let st = typ(sign);
                    if st == EnumToken::NumberTokenType {
                        if !is_integer(arena.numeric_value.get(sign.unwrap())) {
                            success = false;
                            break 'anb;
                        }
                        context.update(sign);
                        return ValidationMatch::new(true, true, None, Some(context.clone()), Some(property.clone()), Vec::new());
                    }
                    if st == EnumToken::Add || st == EnumToken::NextSiblingCombinatorTokenType || st == EnumToken::Sub {
                        let next = context.borrow().peek(arena, 2);
                        success = typ(next) == EnumToken::NumberTokenType && is_integer(arena.numeric_value.get(next.unwrap()));
                        if success {
                            context.update(next);
                            return ValidationMatch::new(true, true, None, Some(context.clone()), Some(property.clone()), Vec::new());
                        }
                    }
                }
                break 'anb;
            }
            if tt == EnumToken::DimensionTokenType && arena.string_equals(arena.unit.get(tk.unwrap()), "n") {
                success = is_integer(arena.numeric_value.get(tk.unwrap()));
                let Some(next) = context.borrow().peek(arena, 1) else {
                    break 'anb;
                };
                if arena.data_types.get(next) == EnumToken::NumberTokenType {
                    if is_integer(arena.numeric_value.get(next)) {
                        context.next_token(arena);
                    } else {
                        success = false;
                    }
                    break 'anb;
                }
                if arena.data_types.get(next) == EnumToken::NextSiblingCombinatorTokenType {
                    arena.data_types.set(next, EnumToken::Add);
                }
                let next = context.borrow().peek(arena, 2);
                if typ(next) == EnumToken::NumberTokenType && is_integer(arena.numeric_value.get(next.unwrap())) {
                    success = true;
                    context.next_token(arena);
                    context.next_token(arena);
                }
            }
        }
        "quote" | "display-box" | "display-inside" | "display-internal" | "display-outside" | "display-legacy" | "content-position" => {
            let values = PROPERTY_KEYWORD_VALUES.with(|p| {
                p.borrow_mut()
                    .entry(val.to_string())
                    .or_insert_with(|| {
                        Rc::new(get_syntax(val).unwrap_or("").split(|c: char| c.is_whitespace() || c == '|').map(String::from).collect())
                    })
                    .clone()
            });
            success = t == EnumToken::IdenTokenType && values.contains(&str_of(token.unwrap()).to_lowercase());
        }
        "mf-name" => {
            success = t == EnumToken::IdenTokenType
                && get_syntax_config()["mediaFeatures"].get(str_of(token.unwrap()).to_lowercase()).is_some();
        }
        "dashed-ident" | "extension-name" | "custom-property-name" => success = t == EnumToken::DashedIdenTokenType,
        "namespace-prefix" | "ident" | "ident-token" | "custom-ident" | "counter-name" | "counter-style-name" => {
            success = t == EnumToken::IdenTokenType || t == EnumToken::DashedIdenTokenType;
            if success && t == EnumToken::IdenTokenType {
                let v = str_of(token.unwrap()).to_lowercase();
                success = v != "none" && !is_css_wide_keyword_exact(&v);
            }
        }
        "any-value" => {
            success = token.is_none()
                || [
                    EnumToken::String,
                    EnumToken::NumberTokenType,
                    EnumToken::DashedIdenTokenType,
                    EnumToken::IdenTokenType,
                    EnumToken::FunctionTokenType,
                    EnumToken::ImageFunctionTokenType,
                    EnumToken::UrlFunctionTokenType,
                    EnumToken::TimingFunctionTokenType,
                    EnumToken::TimelineFunctionTokenType,
                    EnumToken::TransformFunctionTokenType,
                ]
                .contains(&t);
        }
        "unicode-range-token" => success = t == EnumToken::UniCodeRange,
        "hash-token" => {
            success = t == EnumToken::HashTokenType || (t == EnumToken::ColorTokenType && arena.color_type.get(token.unwrap()) == ColorType::HEX);
        }
        "string" => success = t == EnumToken::StringTokenType,
        "wq-name" => {
            let success = t == EnumToken::IdenTokenType;
            let token = if success { context.next_token(arena) } else { context.peek(arena) };
            return ValidationMatch::new(success, true, token, Some(context.clone()), Some(property.clone()), Vec::new());
        }
        "number" => success = t == EnumToken::NumberTokenType,
        "zero" => success = t == EnumToken::NumberTokenType && arena.numeric_value.get(token.unwrap()) == 0.0,
        "integer" => success = t == EnumToken::NumberTokenType && is_integer(arena.numeric_value.get(token.unwrap())),
        "dimension" => {
            success = matches!(
                t,
                EnumToken::DimensionTokenType
                    | EnumToken::AngleTokenType
                    | EnumToken::LengthTokenType
                    | EnumToken::PercentageTokenType
                    | EnumToken::ResolutionTokenType
                    | EnumToken::TimeTokenType
                    | EnumToken::FrequencyTokenType
                    | EnumToken::UserDefinedDimensionTokenType
                    | EnumToken::FlexTokenType
            )
        }
        "angle" => success = t == EnumToken::AngleTokenType || t == EnumToken::UserDefinedDimensionTokenType,
        "angle-percentage" => {
            success = matches!(t, EnumToken::AngleTokenType | EnumToken::UserDefinedDimensionTokenType | EnumToken::PercentageTokenType)
        }
        "percentage" => success = token.is_some() && t == EnumToken::PercentageTokenType,
        "length-percentage" => {
            success = matches!(t, EnumToken::PercentageTokenType | EnumToken::LengthTokenType | EnumToken::UserDefinedDimensionTokenType)
                || (t == EnumToken::NumberTokenType && arena.numeric_value.get(token.unwrap()) == 0.0)
        }
        "flex" => success = t == EnumToken::FlexTokenType || t == EnumToken::UserDefinedDimensionTokenType,
        "length" => {
            success = matches!(t, EnumToken::LengthTokenType | EnumToken::UserDefinedDimensionTokenType)
                || (t == EnumToken::NumberTokenType && arena.numeric_value.get(token.unwrap()) == 0.0)
        }
        "resolution" => success = t == EnumToken::ResolutionTokenType,
        "frequency" => success = t == EnumToken::FrequencyTokenType,
        "decibel" | "semitones" => {
            success = t == EnumToken::DimensionTokenType
                && arena.with_string(arena.unit.get(token.unwrap()), |u| equals_ignore_case(u, if val == "decibel" { "dB" } else { "st" }));
        }
        "time" => success = token.is_some() && (t == EnumToken::TimeTokenType || t == EnumToken::UserDefinedDimensionTokenType),
        "color" | "color-function" => {
            let mut errors: Vec<ErrorDescription> = Vec::new();
            if token.is_some() && is_function_def_type(t) {
                let range = context.borrow().peek_range(arena, EnumToken::EndParensTokenType, 0);
                let mut new_range: Vec<u32> = range.iter().map(|&r| arena.clone_node_deep(r, true, None)).collect();
                let parse_options = ParserOptions { parse_color: Some(true), ..Default::default() };
                crate::parser::parse::parse_tokens(arena, &mut new_range, Some(&parse_options), Some(&mut errors));
                let success = new_range.len() == 1 && is_color(arena, new_range[0], Some(&mut errors));
                if success {
                    context.update(range.last().copied());
                }
                return ValidationMatch::new(success, true, context.peek(arena), Some(context.clone()), None, errors);
            }
            success = token.is_some_and(|tk| is_color(arena, tk, Some(&mut errors)));
            if !success {
                return ValidationMatch::new(false, false, token, Some(context.clone()), Some(property.clone()), errors);
            }
        }
        "type-selector" | "compound-selector" | "complex-selector" | "subclass-selector" | "attribute-selector"
        | "complex-selector-unit" | "pseudo-compound-selector" | "forgiving-relative-selector-list" | "complex-selector-list" => {
            let mut tokens = context.borrow().get_remaining_tokens();
            let forgiving = val == "forgiving-relative-selector-list";
            let (result_success, errors) = match_selector_syntax(arena, &mut tokens, &mut Vec::new(), options, val.starts_with("forgiving-relative-selector-list"));
            if result_success || forgiving {
                context.update(tokens.last().copied());
            }
            return ValidationMatch::new(result_success || forgiving, true, context.peek(arena), Some(context.clone()), Some(property.clone()), errors);
        }
        "function-token" => {
            if is_function_def_type(t) && token.is_some() {
                success = true;
            } else {
                return ko(context, context.peek(arena), property);
            }
        }
        "language-code" => {
            success = t == EnumToken::IdenTokenType && {
                let v = str_of(token.unwrap());
                get_syntax_config()["languages"]
                    .as_array()
                    .is_some_and(|a| a.iter().filter_map(|l| l.as_str()).any(|l| *v == *l || v.starts_with(&format!("{}-", l))))
            };
        }
        "size-feature" | "style-feature" => {
            let filtered = peek_feature_tokens(arena, context);
            let ft = |i: usize| arena.data_types.get(filtered[i]);
            let size = val == "size-feature";
            success = if filtered.len() == 1 {
                ft(0) == EnumToken::IdenTokenType || ft(0) == EnumToken::DashedIdenTokenType
            } else {
                filtered.len() > 2
                    && (mFLT.contains(&ft(1))
                        || mFGT.contains(&ft(1))
                        || ft(1) == EnumToken::ColonTokenType
                        || (size && ft(1) == EnumToken::DelimTokenType))
            };
            if success {
                context.update(filtered.last().copied());
            }
            let token = if success { None } else if size { filtered.get(1).copied().or_else(|| context.peek(arena)) } else { context.peek(arena) };
            return ValidationMatch::new(success, if size { true } else { success }, token, Some(context.clone()), Some(property.clone()), Vec::new());
        }
        "url" => 'url: {
            if t == EnumToken::UrlFunctionTokenDefType {
                let mut l = 0isize;
                let peek = |l: isize| context.borrow().peek(arena, l);
                let mut tk;
                loop {
                    l += 1;
                    tk = peek(l);
                    match tk {
                        Some(x) if matches!(arena.data_types.get(x), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType) => continue,
                        _ => break,
                    }
                }
                if tk.is_some_and(|x| matches!(arena.data_types.get(x), EnumToken::StringTokenType | EnumToken::IdenTokenType | EnumToken::UrlTokenTokenType)) {
                    loop {
                        l += 1;
                        tk = peek(l);
                        match tk {
                            Some(x) if matches!(arena.data_types.get(x), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType) => continue,
                            _ => break,
                        }
                    }
                    if typ(tk) == EnumToken::EndParensTokenType {
                        context.update(tk);
                        return ok(context, None);
                    }
                }
                break 'url;
            }
            if t != EnumToken::UrlFunctionTokenType {
                success = false;
                break 'url;
            }
            let url_tokens = arena.tokens(token.unwrap());
            if url_tokens.iter().any(|&u| !matches!(arena.data_types.get(u), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType)) {
                context.next_token(arena);
                return ok(context, None);
            }
        }
        "css-type" => {
            if (t == EnumToken::FunctionTokenDefType || t == EnumToken::FunctionTokenType)
                && arena.with_string(arena.string_value.get(token.unwrap()), |v| equals_ignore_case(v, "type"))
            {
                if t == EnumToken::FunctionTokenDefType {
                    let last = context.borrow().peek_range(arena, EnumToken::EndParensTokenType, 0).last().copied();
                    context.update(last);
                } else {
                    context.next_token(arena);
                }
                return ok(context, None);
            }
            success = t == EnumToken::CssTypeTokenType;
            if success {
                context.next_token(arena);
                let multiplier = {
                    let c = context.borrow();
                    let i = c.index + 1;
                    if i >= 0 { c.tokens().get(i as usize).copied() } else { None }
                };
                if let Some(m) = multiplier {
                    if arena.data_types.get(m) == EnumToken::Plus
                        || (arena.data_types.get(m) == EnumToken::LiteralTokenType && arena.string_equals(arena.string_value.get(m), "#"))
                    {
                        context.next_token(arena);
                    }
                }
                return ok(context, None);
            }
        }
        "dashed-function" => {
            success = t == EnumToken::CustomFunctionTokenDefType || t == EnumToken::CustomFunctionTokenType;
            if success {
                if t == EnumToken::CustomFunctionTokenDefType {
                    let last = context.borrow().peek_range(arena, EnumToken::EndParensTokenType, 0).last().copied();
                    context.update(last);
                } else {
                    context.next_token(arena);
                }
                return ok(context, token);
            }
        }
        "block-contents" => {
            if t == EnumToken::StartParensTokenType && token.is_some() {
                context.next_token(arena);
                return ok(context, None);
            }
            return ko(context, token, property);
        }
        _ => {
            if !has_syntax(val) {
                // typescript throws `Unexpected validation property`
                return ko(context, context.peek(arena), property);
            }
            let parsed = get_parsed_syntax(ValidationSyntaxGroupEnum::Syntaxes, val);
            return match_syntax(arena, parsed.as_deref().map(|v| v.as_slice()), context, options);
        }
    }
    let peeked = context.peek(arena);
    if !success && typ(peeked) == EnumToken::IdenTokenType && is_css_wide_keyword_exact(&str_of(peeked.unwrap())) {
        success = true;
    }
    if success {
        context.next_token(arena);
        return ok(context, None);
    }
    ko(context, context.peek(arena), property)
}

/// `allValues.includes(value)`: case sensitive
fn is_css_wide_keyword_exact(value: &str) -> bool {
    let _ = is_css_wide_keyword;
    get_syntax_config()["declarations"]["all"]["syntax"]
        .as_str()
        .unwrap_or("")
        .split(|c: char| c.is_whitespace() || c == '|')
        .any(|v| v == value)
}

/// matches a repeatable syntax (`*`, `+`)
pub fn match_repeatable_syntax(arena: &AstArena, syntax: &Syntax, context: &Context, options: &ParserOptions) -> ValidationMatch {
    let mut rest = (**syntax).clone();
    let is_repeatable = rest.is_repeatable();
    rest.is_repeatable = None;
    rest.is_optional = None;
    rest.is_mandatatory_group = None;
    rest.is_repeatable_at_least_once = None;
    let rest = [Rc::new(rest)];
    let mut result: Option<ValidationMatch> = None;
    let mut success = is_repeatable;
    loop {
        let tmp = match_syntax(arena, Some(&rest), &context.slice(), options);
        if tmp.success {
            result = Some(tmp.clone());
            success = true;
            if tmp.ctx().done(arena) {
                context.end();
                break;
            }
            if tmp.ctx().current() == context.current() {
                break;
            }
            context.update(tmp.ctx().current());
        }
        if !(tmp.success && !context.done(arena)) {
            break;
        }
    }
    ValidationMatch::new(
        success,
        true,
        context.peek(arena),
        Some(context.clone()),
        Some(syntax.clone()),
        result.map(|r| r.errors).unwrap_or_default(),
    )
}
