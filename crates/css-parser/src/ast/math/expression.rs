//! Port of `src/ast/math/expression.ts`.
//!
//! `Math[name.toUpperCase()]` lookups of the typescript are `math_constant()`: `undefined` is `None`.

use crate::ast::arena::{is_numeric_token_type, AstArena, TokenField, TokenValue};
use crate::ast::math::math::{compute, rem, NumOrFraction};
use crate::ast::types::EnumToken;
use crate::parser::utils::text::equals_ignore_case;
use crate::syntax::constants::MATH_FUNCS;
use crate::utils::number::{js_parse_number, js_round, to_fixed};

/// functions that return a number (`numbers`)
const NUMBERS: [&str; 6] = ["sin", "cos", "asin", "acos", "sign", "tan"];

/// `Math[name.toUpperCase()]` when it is a number
fn math_constant(name: &str) -> Option<f64> {
    use std::f64::consts as c;
    match name.to_uppercase().as_str() {
        "E" => Some(c::E),
        "LN10" => Some(c::LN_10),
        "LN2" => Some(c::LN_2),
        "LOG10E" => Some(c::LOG10_E),
        "LOG2E" => Some(c::LOG2_E),
        "PI" => Some(c::PI),
        "SQRT1_2" => Some(c::FRAC_1_SQRT_2),
        "SQRT2" => Some(c::SQRT_2),
        _ => None,
    }
}

/// `Math.sign()`
fn js_sign(v: f64) -> f64 {
    if v.is_nan() || v == 0.0 {
        v
    } else {
        v.signum()
    }
}

/// `Math[name](val)`
fn math_function(name: &str, v: f64) -> f64 {
    match name {
        "sin" => v.sin(),
        "cos" => v.cos(),
        "tan" => v.tan(),
        "asin" => v.asin(),
        "acos" => v.acos(),
        "atan" => v.atan(),
        "abs" => v.abs(),
        "sign" => js_sign(v),
        "sqrt" => v.sqrt(),
        "exp" => v.exp(),
        _ => f64::NAN,
    }
}

fn str_of(arena: &AstArena, token: u32) -> crate::ast::strings::SharedStr {
    arena.resolve_string(arena.string_value.get(token))
}

fn is_math_func(name: &str) -> bool {
    MATH_FUNCS.contains(&name)
}

/// numeric value of a token: a number or a fraction
fn numeric_value(arena: &AstArena, token: u32) -> NumOrFraction {
    let fraction = arena.get_fraction(token);
    if fraction != 0 {
        return NumOrFraction::Fraction(
            arena.numeric_value.get(arena.first_child.get(fraction)),
            arena.numeric_value.get(arena.last_child.get(fraction)),
        );
    }
    NumOrFraction::Num(get_value(arena, token).unwrap_or(f64::NAN))
}

/// set the numeric value of a token: a number or a fraction
fn set_numeric_value(arena: &AstArena, token: u32, val: NumOrFraction) {
    match val {
        NumOrFraction::Num(v) => {
            arena.set_fraction(token, None);
            arena.numeric_value.set(token, v);
        }
        NumOrFraction::Fraction(n, d) => {
            let fraction = arena.create_fraction(
                arena.create_token(EnumToken::NumberTokenType, TokenValue::Num(n), None),
                arena.create_token(EnumToken::NumberTokenType, TokenValue::Num(d), None),
            );
            arena.set_fraction(token, Some(fraction));
        }
    }
}

/// allocate a token with the location of another token
fn token_at(arena: &AstArena, typ: u8, location: u32, val: TokenValue, unit: Option<&str>) -> u32 {
    let token = arena.create_token(typ, val, unit);
    arena.copy_location(token, location);
    token
}

/// evaluate an array of tokens. the tokens are cloned: the result does not share any token with the input
pub fn evaluate(arena: &AstArena, tokens: &[u32]) -> Vec<u32> {
    let cloned: Vec<u32> = tokens.iter().map(|&token| arena.clone_node_deep(token, true, None)).collect();
    evaluate_tokens(arena, cloned)
}

/// evaluate an array of tokens. the tokens are moved into the result
fn evaluate_tokens(arena: &AstArena, tokens: Vec<u32>) -> Vec<u32> {
    let t = |x: u32| arena.data_types.get(x);
    if tokens.len() == 1
        && matches!(t(tokens[0]), EnumToken::MathFunctionTokenType | EnumToken::FunctionTokenType)
        && is_math_func(&str_of(arena, tokens[0]))
    {
        let mut chi: Vec<Vec<u32>> = Vec::new();
        let mut tk = arena.first_token.get(tokens[0]);
        while tk != 0 {
            let typ = t(tk);
            if chi.is_empty() || typ == EnumToken::CommaTokenType {
                chi.push(Vec::new());
            }
            if typ != EnumToken::WhitespaceTokenType && typ != EnumToken::CommaTokenType {
                chi.last_mut().unwrap().push(tk);
            }
            tk = arena.next_sibling.get(tk);
        }
        let mut children: Vec<u32> = Vec::new();
        for group in chi {
            if !children.is_empty() {
                children.push(arena.create_token(EnumToken::CommaTokenType, TokenValue::None, None));
            }
            children.extend(evaluate_tokens(arena, group));
        }
        arena.set_token_list(tokens[0], &children);
        let Some(result) = evaluate_func(arena, tokens[0]) else {
            return tokens;
        };
        if t(result[0]) == EnumToken::MathFunctionTokenType && &*str_of(arena, result[0]) == "calc" && arena.token_count(result[0]) == 1 {
            return arena.tokens(result[0]);
        }
        return result;
    }
    let expression = build_expression(arena, &tokens);
    let nodes = inline_expression(arena, evaluate_expression(arena, expression));
    if nodes.len() <= 1 {
        if nodes.len() == 1 {
            if t(nodes[0]) == EnumToken::BinaryExpressionTokenType {
                return inline_expression(arena, nodes[0]);
            }
            if t(nodes[0]) == EnumToken::IdenTokenType {
                if let Some(val) = math_constant(&str_of(arena, nodes[0])) {
                    arena.reset_token(nodes[0], EnumToken::NumberTokenType, TokenValue::Num(val), None);
                    return nodes;
                }
            }
        }
        return nodes;
    }
    // insertion ordered `Map<EnumToken, Token[]>`
    let mut map: Vec<(u8, Vec<u32>)> = Vec::new();
    let mut i = 0usize;
    while i < nodes.len() {
        let mut token = nodes[i];
        let mut typ = t(token);
        if typ == EnumToken::Add || typ == EnumToken::Plus {
            i += 1;
            continue;
        }
        if typ == EnumToken::Sub {
            let next = nodes.get(i + 1).copied().unwrap_or(0);
            if !is_scalar_token(arena, next) {
                token = arena.create_list(EnumToken::ListToken, Some(&[nodes[i], next]));
                arena.span_location(token, nodes[i], next);
            } else {
                let minus_one = token_at(arena, EnumToken::NumberTokenType, next, TokenValue::Num(-1.0), None);
                token = do_evaluate(arena, next, minus_one, EnumToken::Mul);
            }
            i += 1;
            typ = t(token);
        }
        match map.iter_mut().find(|(k, _)| *k == typ) {
            Some((_, values)) => values.push(token),
            None => map.push((typ, vec![token])),
        }
        i += 1;
    }
    let mut result: Vec<u32> = Vec::new();
    for (key, values) in map {
        let mut token = values[0];
        for &curr in &values[1..] {
            token = do_evaluate(arena, token, curr, EnumToken::Add);
        }
        if t(token) != EnumToken::BinaryExpressionTokenType && is_numeric_token_type(t(token)) && arena.numeric_value.get(token) < 0.0 {
            arena.numeric_value.set(token, -arena.numeric_value.get(token));
            result.push(token_at(arena, EnumToken::Sub, token, TokenValue::None, None));
            result.push(token);
            continue;
        }
        if !result.is_empty() && key != EnumToken::ListToken {
            if is_numeric_token_type(t(token)) && arena.numeric_value.get(token) == 0.0 {
                continue;
            }
            result.push(token_at(arena, EnumToken::Add, token, TokenValue::None, None));
        }
        result.push(token);
    }
    result
}

/// evaluate arithmetic operation
fn do_evaluate(arena: &AstArena, l: u32, r: u32, op: u8) -> u32 {
    let mut l = l;
    let mut r = r;
    let default_return = |l: u32, r: u32| {
        let token = arena.create_binary_expression(op, l, r);
        arena.span_location(token, l, r);
        token
    };
    let t = |x: u32| arena.data_types.get(x);
    let mut l_type = t(l);
    let mut r_type = t(r);
    if !is_scalar_token(arena, l)
        || !is_scalar_token(arena, r)
        || (l_type == r_type && arena.unit.get(l) != 0 && arena.unit.get(r) != 0 && arena.unit.get(l) != arena.unit.get(r))
    {
        return default_return(l, r);
    }
    if l_type == EnumToken::IdenTokenType {
        let val = str_of(arena, l);
        if equals_ignore_case(&val, "Infinity") {
            l_type = EnumToken::InfinityTokenType;
            arena.data_types.set(l, l_type);
        } else if equals_ignore_case(&val, "-Infinity") {
            l_type = EnumToken::NegativeInfinityTokenType;
            arena.data_types.set(l, l_type);
        } else if equals_ignore_case(&val, "NaN") {
            l_type = EnumToken::NaNTokenType;
            arena.data_types.set(l, l_type);
        }
    }
    if r_type == EnumToken::IdenTokenType {
        let val = str_of(arena, r);
        if equals_ignore_case(&val, "Infinity") {
            r_type = EnumToken::InfinityTokenType;
            arena.data_types.set(r, r_type);
        } else if equals_ignore_case(&val, "-Infinity") {
            r_type = EnumToken::NegativeInfinityTokenType;
            arena.data_types.set(r, r_type);
        }
    }
    // https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Values/calc-keyword#notes
    if l_type == EnumToken::NaNTokenType || r_type == EnumToken::NaNTokenType {
        return compute_nan(arena, l, r, &default_return);
    }
    if matches!(l_type, EnumToken::InfinityTokenType | EnumToken::NegativeInfinityTokenType)
        || matches!(r_type, EnumToken::InfinityTokenType | EnumToken::NegativeInfinityTokenType)
    {
        return compute_infinity(arena, l, r, op, &default_return);
    }
    let is_function = |x: u8| x == EnumToken::FunctionTokenType || x == EnumToken::MathFunctionTokenType;
    if is_function(l_type) {
        match evaluate_func(arena, l) {
            None => return default_return(l, r),
            Some(val) => {
                if val.len() == 1 && is_function(t(val[0])) {
                    return default_return(l, r);
                }
                if val.len() == 1 {
                    l = val[0];
                    l_type = t(l);
                }
            }
        }
    }
    if is_function(r_type) {
        match evaluate_func(arena, r) {
            None => return default_return(l, r),
            Some(val) => {
                if val.len() == 1 && is_function(t(val[0])) {
                    return default_return(l, r);
                }
                if val.len() == 1 {
                    r = val[0];
                    r_type = t(r);
                }
            }
        }
    }
    if (op == EnumToken::Add || op == EnumToken::Plus || op == EnumToken::Sub) && l_type != r_type {
        return default_return(l, r);
    }
    let typ = if l_type == EnumToken::NumberTokenType || l_type == EnumToken::IdenTokenType {
        r_type
    } else if r_type == EnumToken::NumberTokenType {
        l_type
    } else if l_type == EnumToken::PercentageTokenType {
        r_type
    } else {
        l_type
    };
    let mut v1 = numeric_value(arena, l);
    let mut v2 = numeric_value(arena, r);
    if op == EnumToken::Mul && l_type != EnumToken::NumberTokenType && r_type != EnumToken::NumberTokenType {
        if let (NumOrFraction::Num(v), EnumToken::PercentageTokenType) = (v1, l_type) {
            v1 = NumOrFraction::Fraction(v, 100.0);
        } else if let (NumOrFraction::Num(v), EnumToken::PercentageTokenType) = (v2, r_type) {
            v2 = NumOrFraction::Fraction(v, 100.0);
        }
    }
    // `v == 0`: a fraction (array) is never equal to 0
    let is_zero = |v: NumOrFraction| matches!(v, NumOrFraction::Num(n) if n == 0.0);
    let as_number = |v: NumOrFraction| match v {
        NumOrFraction::Num(n) => n,
        NumOrFraction::Fraction(..) => f64::NAN,
    };
    if op == EnumToken::Div && is_zero(v2) {
        if is_zero(v1) {
            arena.reset_token(l, EnumToken::IdenTokenType, TokenValue::Str("NaN"), None);
            return l;
        }
        let sign = js_sign(as_number(v1)) * js_sign(as_number(v2));
        // `Object.is(sign, -0) ? -1 : 1`
        let sign = if sign == 0.0 && sign.is_sign_negative() { -1.0 } else { 1.0 };
        arena.reset_token(l, if l_type == EnumToken::IdenTokenType { EnumToken::NumberTokenType } else { l_type }, TokenValue::Num(sign), None);
        return default_return(l, r);
    }
    let val = compute(v1, v2, op);
    // the result inherits the unit of the dimension operand
    let base = if l_type == EnumToken::NumberTokenType || l_type == EnumToken::IdenTokenType { r } else { l };
    let token = arena.create_token(if typ == EnumToken::IdenTokenType { EnumToken::NumberTokenType } else { typ }, TokenValue::None, None);
    arena.unit.set(token, arena.unit.get(base));
    set_numeric_value(arena, token, val);
    arena.span_location(token, l, r);
    token
}

/// `typeof Math[name.toUpperCase()] === "string"`: never true
fn is_math_string(_name: &str) -> bool {
    false
}

fn compute_nan(arena: &AstArena, l: u32, r: u32, default_return: &dyn Fn(u32, u32) -> u32) -> u32 {
    let l_type = arena.data_types.get(l);
    let r_type = arena.data_types.get(r);
    if l_type == EnumToken::NaNTokenType {
        if r_type == EnumToken::NumberTokenType || (r_type == EnumToken::IdenTokenType && is_math_string(&str_of(arena, r))) {
            return l;
        }
        // NaN Infinity e pi ...
        if matches!(r_type, EnumToken::IdenTokenType | EnumToken::InfinityTokenType | EnumToken::NegativeInfinityTokenType) {
            return l;
        }
        if is_numeric_token_type(r_type) {
            set_numeric_value(arena, r, NumOrFraction::Num(1.0));
        }
        return if r_type == EnumToken::NaNTokenType { l } else { default_return(l, r) };
    }
    if r_type == EnumToken::NaNTokenType {
        if l_type == EnumToken::NumberTokenType || (l_type == EnumToken::IdenTokenType && is_math_string(&str_of(arena, l))) {
            return r;
        }
        if is_numeric_token_type(l_type) {
            set_numeric_value(arena, l, NumOrFraction::Num(1.0));
        }
    }
    default_return(l, r)
}

fn compute_infinity(arena: &AstArena, l: u32, r: u32, op: u8, default_return: &dyn Fn(u32, u32) -> u32) -> u32 {
    let l_type = arena.data_types.get(l);
    let r_type = arena.data_types.get(r);
    let is_infinity = |x: u8| x == EnumToken::InfinityTokenType || x == EnumToken::NegativeInfinityTokenType;
    if is_infinity(l_type) {
        if is_infinity(r_type) {
            // Infinity / Infinity Infinity - Infinity
            if op == EnumToken::Div || (op == EnumToken::Add && l_type != r_type) || (op == EnumToken::Sub && l_type == r_type) {
                arena.data_types.set(l, EnumToken::NaNTokenType);
                return l;
            }
            if l_type != r_type {
                arena.data_types.set(l, EnumToken::NegativeInfinityTokenType);
            }
            return l;
        }
        let value = get_value(arena, r).unwrap_or(f64::NAN);
        let sign = js_sign(value);
        let is_number = r_type == EnumToken::NumberTokenType || r_type == EnumToken::IdenTokenType;
        // Infinity * 0
        if value == 0.0 && op == EnumToken::Mul {
            arena.data_types.set(l, EnumToken::NaNTokenType);
            return if is_number { l } else { default_return(l, r) };
        }
        if is_number {
            let v = arena.numeric_value.get(r);
            arena.numeric_value.set(r, v / (v * sign));
        }
        if op == EnumToken::Div {
            if is_number {
                arena.reset_token(r, EnumToken::NumberTokenType, TokenValue::Num(if sign < 0.0 { -0.0 } else { 0.0 }), None);
                return r;
            }
            return default_return(l, r);
        }
        if sign == -1.0 {
            arena.data_types.set(
                l,
                if l_type == EnumToken::InfinityTokenType { EnumToken::NegativeInfinityTokenType } else { EnumToken::InfinityTokenType },
            );
        }
        return if is_number { l } else { default_return(l, r) };
    }
    if is_infinity(r_type) {
        let value = get_value(arena, l).unwrap_or(f64::NAN);
        let sign = js_sign(value);
        let is_number = l_type == EnumToken::NumberTokenType || l_type == EnumToken::IdenTokenType;
        if is_number {
            arena.reset_token(l, EnumToken::NumberTokenType, TokenValue::Num(sign), None);
        }
        if value == 0.0 && op == EnumToken::Mul {
            arena.data_types.set(l, EnumToken::NaNTokenType);
            return l;
        }
        if op == EnumToken::Div {
            if is_number {
                arena.reset_token(l, EnumToken::NumberTokenType, TokenValue::Num(if sign < 0.0 { -0.0 } else { 0.0 }), None);
                return l;
            }
            return default_return(l, r);
        }
        return if is_number || value == 0.0 { r } else { default_return(l, r) };
    }
    default_return(l, r)
}

/// numeric value of a scalar token. `None`: `undefined` (an identifier that is not a `Math` constant)
fn get_value(arena: &AstArena, t: u32) -> Option<f64> {
    let typ = arena.data_types.get(t);
    if typ == EnumToken::IdenTokenType {
        return math_constant(&str_of(arena, t));
    }
    if typ == EnumToken::FractionTokenType {
        return Some(arena.numeric_value.get(arena.first_child.get(t)) / arena.numeric_value.get(arena.last_child.get(t)));
    }
    if !is_numeric_token_type(typ) {
        // not a number
        return Some(f64::NAN);
    }
    let fraction = arena.get_fraction(t);
    if fraction != 0 {
        return Some(arena.numeric_value.get(arena.first_child.get(fraction)) / arena.numeric_value.get(arena.last_child.get(fraction)));
    }
    Some(arena.numeric_value.get(t))
}

/// convert an angle token to radians (in place)
fn to_radians(arena: &AstArena, token: u32) -> u32 {
    let typ = arena.data_types.get(token);
    if typ == EnumToken::NumberTokenType {
        arena.data_types.set(token, EnumToken::AngleTokenType);
        arena.set_unit_string(token, Some("rad"));
    } else if typ == EnumToken::AngleTokenType {
        let factor = match &*arena.resolve_string(arena.unit.get(token)) {
            "deg" => Some(std::f64::consts::PI / 180.0),
            "grad" => Some(std::f64::consts::PI / 200.0),
            "turn" => Some(2.0 * std::f64::consts::PI),
            _ => None,
        };
        if let Some(factor) = factor {
            arena.numeric_value.set(token, arena.numeric_value.get(token) * factor);
            arena.set_unit_string(token, Some("rad"));
        }
    }
    token
}

/// evaluate a math function
pub fn evaluate_func(arena: &AstArena, token: u32) -> Option<Vec<u32>> {
    let t = |x: u32| arena.data_types.get(x);
    let mut values = arena.tokens(token);
    let name = str_of(arena, token).to_string();
    let value_or_nan = |x: u32| get_value(arena, x).unwrap_or(f64::NAN);
    match name.as_str() {
        "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "abs" | "sign" | "sqrt" | "exp" => {
            if matches!(name.as_str(), "sin" | "cos" | "tan") {
                for &v in &values {
                    to_radians(arena, v);
                }
            }
            let value = evaluate_tokens(arena, values);
            let first = value.first().copied().unwrap_or(0);
            let computed = math_function(&name, value_or_nan(first));
            Some(vec![if matches!(name.as_str(), "asin" | "acos" | "atan") {
                token_at(arena, EnumToken::AngleTokenType, first, TokenValue::Num(computed), Some("rad"))
            } else {
                token_at(arena, if NUMBERS.contains(&name.as_str()) { EnumToken::NumberTokenType } else { t(first) }, first, TokenValue::Num(computed), None)
            }])
        }
        "hypot" => {
            let chi: Vec<u32> = values
                .iter()
                .copied()
                .filter(|&x| !matches!(t(x), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType | EnumToken::CommaTokenType))
                .collect();
            let mut all: Vec<f64> = Vec::new();
            for &c in &chi {
                // `Number.isNaN(undefined)` is false
                let val = get_value(arena, c);
                if val.is_some_and(f64::is_nan) {
                    return None;
                }
                all.push(val.unwrap_or(f64::NAN));
            }
            let result = arena.clone_node_deep(chi.first().copied().unwrap_or(0), true, None);
            set_numeric_value(arena, result, NumOrFraction::Num(js_hypot(&all)));
            arena.copy_location(result, token);
            Some(vec![result])
        }
        "mod" | "atan2" | "pow" | "rem" => {
            let first = arena.first_token.get(token);
            let typ = if first == 0 { 0 } else { t(first) };
            let mut tk = if first == 0 { 0 } else { arena.next_sibling.get(first) };
            while tk != 0 {
                if !matches!(t(tk), EnumToken::WhitespaceTokenType | EnumToken::CommaTokenType | EnumToken::CommentTokenType) && t(tk) != typ {
                    return None;
                }
                tk = arena.next_sibling.get(tk);
            }
            let chi: Vec<u32> = values.iter().copied().filter(|&x| !matches!(t(x), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType)).collect();
            if name == "atan2" {
                for &c in &chi {
                    to_radians(arena, c);
                }
            }
            // https://developer.mozilla.org/en-US/docs/Web/CSS/mod
            let v1 = evaluate_tokens(arena, vec![chi.first().copied().unwrap_or(0)]);
            let v2 = evaluate_tokens(arena, vec![chi.get(2).copied().unwrap_or(0)]);
            let val1 = value_or_nan(v1[0]);
            let val2 = value_or_nan(v2[0]);
            let result = arena.clone_node_deep(v1[0], true, None);
            if name == "rem" {
                set_numeric_value(arena, result, NumOrFraction::Num(js_parse_number(&to_fixed(val1 % val2, rem(&[val1, val2])))));
                return Some(vec![result]);
            }
            arena.copy_location(result, token);
            if name == "pow" {
                set_numeric_value(arena, result, NumOrFraction::Num(val1.powf(val2)));
                return Some(vec![result]);
            }
            if name == "atan2" {
                arena.data_types.set(result, EnumToken::AngleTokenType);
                arena.set_unit_string(result, Some("rad"));
                set_numeric_value(arena, result, NumOrFraction::Num(val1.atan2(val2)));
                return Some(vec![result]);
            }
            set_numeric_value(arena, result, NumOrFraction::Num(if val2 == 0.0 { val1 } else { val1 - (val1 / val2).floor() * val2 }));
            Some(vec![result])
        }
        "clamp" => {
            arena.set_token_list(token, &values);
            Some(vec![token])
        }
        "log" | "round" | "min" | "max" => {
            let strategy = if name == "round" && !values.is_empty() && t(values[0]) == EnumToken::IdenTokenType {
                Some(str_of(arena, values.remove(0)).to_string())
            } else {
                None
            };
            let mut values_map: Vec<(String, Vec<u32>)> = Vec::new();
            for &curr in &values {
                if matches!(t(curr), EnumToken::CommaTokenType | EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType) {
                    continue;
                }
                // the argument is evaluated on a copy: the function is returned unchanged when it cannot be computed
                let result = evaluate_tokens(arena, vec![arena.clone_node_deep(curr, true, None)]);
                let first = result.first().copied().unwrap_or(0);
                let key = format!("{}{}", t(first), arena.resolve_string(arena.unit.get(first)));
                match values_map.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, v)) => v.push(first),
                    None => values_map.push((key, vec![first])),
                }
            }
            if values_map.len() == 1 {
                let values = &values_map[0].1;
                if name == "log" {
                    let val1 = value_or_nan(values[0]);
                    // `Math.log(null)` = `Math.log(0)`
                    let val2 = if values.len() == 2 { value_or_nan(values[1]) } else { 0.0 };
                    let result = arena.clone_node_deep(values[0], true, None);
                    set_numeric_value(arena, result, NumOrFraction::Num(val1.ln() / val2.ln()));
                    arena.copy_location(result, token);
                    return Some(vec![result]);
                }
                if name == "min" || name == "max" {
                    let mut val = value_or_nan(values[0]);
                    let mut ret = values[0];
                    for &curr in &values[1..] {
                        let val2 = value_or_nan(curr);
                        if val2 < val && name == "min" {
                            val = val2;
                            ret = curr;
                        } else if val2 > val && name == "max" {
                            val = val2;
                            ret = curr;
                        }
                    }
                    return Some(vec![ret]);
                }
                if name == "round" {
                    let mut val = value_or_nan(values[0]);
                    let val2 = value_or_nan(values.get(1).copied().unwrap_or(0));
                    match strategy.as_deref() {
                        None | Some("down") => val -= val % val2,
                        Some("to-zero") => val = (val / val2).trunc() * val2,
                        Some("nearest") => val = js_round(val / val2) * val2,
                        Some(_) => val = (val / val2).ceil() * val2,
                    }
                    let result = arena.clone_node_deep(values[0], true, None);
                    set_numeric_value(arena, result, NumOrFraction::Num(val));
                    arena.copy_location(result, token);
                    return Some(vec![result]);
                }
            }
            Some(vec![token])
        }
        _ => Some(vec![token]),
    }
}

/// `Math.hypot()`
fn js_hypot(values: &[f64]) -> f64 {
    if values.iter().any(|v| v.is_infinite()) {
        return f64::INFINITY;
    }
    if values.iter().any(|v| v.is_nan()) {
        return f64::NAN;
    }
    values.iter().fold(0.0f64, |acc, &v| acc.hypot(v))
}

/// convert BinaryExpression into an array
pub fn inline_expression(arena: &AstArena, token: u32) -> Vec<u32> {
    let mut result: Vec<u32> = Vec::new();
    if arena.data_types.get(token) == EnumToken::BinaryExpressionTokenType {
        let op = arena.numeric_value.get(token) as u8;
        if op == EnumToken::Mul || op == EnumToken::Div {
            result.push(token);
        } else {
            let l = arena.field(token, TokenField::L);
            let r = arena.field(token, TokenField::R);
            result.extend(inline_expression(arena, l));
            result.push(token_at(arena, op, token, TokenValue::None, None));
            result.extend(inline_expression(arena, r));
        }
    } else {
        result.push(token);
    }
    result
}

/// evaluate expression
fn evaluate_expression(arena: &AstArena, token: u32) -> u32 {
    if arena.data_types.get(token) != EnumToken::BinaryExpressionTokenType {
        return token;
    }
    let mut l = arena.field(token, TokenField::L);
    let mut r = arena.field(token, TokenField::R);
    if arena.data_types.get(r) == EnumToken::BinaryExpressionTokenType {
        r = evaluate_expression(arena, r);
    }
    if arena.data_types.get(l) == EnumToken::BinaryExpressionTokenType {
        l = evaluate_expression(arena, l);
    }
    do_evaluate(arena, l, r, arena.numeric_value.get(token) as u8)
}

fn is_scalar_token(arena: &AstArena, token: u32) -> bool {
    let typ = arena.data_types.get(token);
    let iden = typ == EnumToken::IdenTokenType;
    arena.unit.get(token) != 0
        || typ == EnumToken::MathFunctionTokenType
        || (typ == EnumToken::FunctionTokenType && is_math_func(&str_of(arena, token)))
        || (iden && {
            let v = str_of(arena, token);
            math_constant(&v).is_some() || &*v == "NaN" || equals_ignore_case(&v, "Infinity") || equals_ignore_case(&v, "-Infinity")
        })
        || matches!(
            typ,
            EnumToken::NumberTokenType
                | EnumToken::FractionTokenType
                | EnumToken::PercentageTokenType
                | EnumToken::NaNTokenType
                | EnumToken::InfinityTokenType
                | EnumToken::NegativeInfinityTokenType
        )
}

/// generate a binary expression tree
pub fn build_expression(arena: &AstArena, tokens: &[u32]) -> u32 {
    let filtered: Vec<u32> = tokens.iter().copied().filter(|&t| arena.data_types.get(t) != EnumToken::WhitespaceTokenType).collect();
    let first = factor(arena, filtered, &["/", "*"]);
    factor(arena, first, &["+", "-"]).first().copied().unwrap_or(0)
}

fn get_arithmetic_operation(op: &str) -> u8 {
    match op {
        "+" => EnumToken::Add,
        "-" => EnumToken::Sub,
        "/" => EnumToken::Div,
        _ => EnumToken::Mul,
    }
}

/// generate a binary expression tree
fn factor_token(arena: &AstArena, token: u32) -> u32 {
    let typ = arena.data_types.get(token);
    if typ == EnumToken::ParensTokenType
        || (matches!(typ, EnumToken::MathFunctionTokenType | EnumToken::FunctionTokenType) && &*str_of(arena, token) == "calc")
    {
        return build_expression(arena, &arena.tokens(token));
    }
    token
}

/// generate a binary expression tree
fn factor(arena: &AstArena, mut tokens: Vec<u32>, ops: &[&str]) -> Vec<u32> {
    let op_list: Vec<u8> = ops.iter().map(|x| get_arithmetic_operation(x)).collect();
    if tokens.len() == 1 {
        return vec![factor_token(arena, tokens[0])];
    }
    let mut i: isize = 0;
    while (i as usize) < tokens.len() {
        let iu = i as usize;
        if arena.data_types.get(tokens[iu]) == EnumToken::ListToken {
            let inner = arena.tokens(tokens[iu]);
            tokens.splice(iu..iu + 1, inner);
            if iu >= tokens.len() {
                break;
            }
        }
        let typ = arena.data_types.get(tokens[iu]);
        let is_op = op_list.contains(&if typ == EnumToken::Plus { EnumToken::Add } else { typ });
        let literal_op = typ == EnumToken::LiteralTokenType && ops.contains(&&*str_of(arena, tokens[iu]));
        if is_op || typ == EnumToken::Star || literal_op {
            let op = if is_op {
                if typ == EnumToken::Plus {
                    EnumToken::Add
                } else {
                    typ
                }
            } else {
                get_arithmetic_operation(&str_of(arena, tokens[iu]))
            };
            let prev = if iu > 0 { tokens[iu - 1] } else { 0 };
            let next = tokens.get(iu + 1).copied().unwrap_or(0);
            let expression = arena.create_binary_expression(op, factor_token(arena, prev), factor_token(arena, next));
            arena.span_location(expression, prev, next);
            let start = iu.saturating_sub(1);
            let end = (iu + 2).min(tokens.len());
            tokens.splice(start..end, [expression]);
            i -= 1;
        }
        i += 1;
    }
    tokens
}
