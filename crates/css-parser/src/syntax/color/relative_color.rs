//! Port of `src/syntax/color/relative-color.ts`.
//!
//! The typescript records (`Record<RelativeColorTypes, Token>`) are insertion ordered: `Vec<(key, token)>`. A missing
//! token (`undefined`) is 0.

use super::color::{convert_color, get_number};
use super::utils::components::get_color_components;
use crate::ast::arena::{AstArena, TokenValue};
use crate::ast::math::expression::{evaluate, evaluate_func};
use crate::ast::types::{ColorType, EnumToken};
use crate::ast::walk::{ValueWalker, WalkerValueOptions};
use crate::parser::utils::text::equals_ignore_case;
use crate::syntax::constants::{color_range, COLORS_FUNC, COLOR_FUNC_COLOR_SPACE, MATH_FUNCS};

/// ordered record of the relative color components
pub type Components = Vec<(String, u32)>;

/// `typeof Math[name.toUpperCase()] === "number"`
fn is_math_constant(name: &str) -> bool {
    matches!(name.to_uppercase().as_str(), "E" | "LN10" | "LN2" | "LOG10E" | "LOG2E" | "PI" | "SQRT1_2" | "SQRT2")
}

/// `Math[name.toUpperCase()]` (`NaN` for `undefined`)
fn math_constant(name: &str) -> f64 {
    use std::f64::consts as c;
    match name.to_uppercase().as_str() {
        "E" => c::E,
        "LN10" => c::LN_10,
        "LN2" => c::LN_2,
        "LOG10E" => c::LOG10_E,
        "LOG2E" => c::LOG2_E,
        "PI" => c::PI,
        "SQRT1_2" => c::FRAC_1_SQRT_2,
        "SQRT2" => c::SQRT_2,
        _ => f64::NAN,
    }
}

const KEYWORDS: [&str; 9] = ["in", "hue", "from", "alpha", "none", "shorter", "longer", "increasing", "decreasing"];

fn record_get(record: &Components, key: &str) -> Option<u32> {
    record.iter().find(|(k, _)| k == key).map(|(_, v)| *v)
}

/// parse the components of a relative color
#[allow(clippy::too_many_arguments)]
pub fn parse_relative_color_components(
    arena: &AstArena,
    relative_keys: &str,
    original: u32,
    r_exp: u32,
    g_exp: u32,
    b_exp: u32,
    a_exp: u32,
) -> Option<Components> {
    let s = |t: u32| arena.resolve_string(arena.string_value.get(t)).to_string();
    // colorFuncColorSpace x,y,z or r,g,b
    let names: String = if relative_keys.starts_with("xyz") {
        "xyz".into()
    } else if ["srgb", "srgb-linear", "display-p3", "a98-rgb", "prophoto-rgb", "rec2020", "rgb"].iter().any(|t| equals_ignore_case(t, relative_keys)) {
        "rgb".into()
    } else {
        let chars: Vec<char> = relative_keys.chars().collect();
        chars[chars.len().saturating_sub(3)..].iter().collect()
    };
    let names: Vec<String> = names.chars().map(|c| c.to_string()).collect();
    let mut all_components: Vec<u32> = if a_exp == 0 { vec![r_exp, g_exp, b_exp] } else { vec![r_exp, g_exp, b_exp, a_exp] };
    if let Some(components) = get_color_components(arena, original) {
        all_components.extend(components);
    }
    let valid_key = |val: &str| names.iter().any(|n| n == val);
    // ensure all components are valid for the color space
    for &component in &all_components {
        if component == 0 {
            continue;
        }
        if arena.data_types.get(component) == EnumToken::IdenTokenType {
            let val = s(component);
            if !is_math_constant(&val)
                && !KEYWORDS.iter().any(|k| equals_ignore_case(&val, k))
                && !COLORS_FUNC.iter().any(|t| equals_ignore_case(t, &val))
                && !COLOR_FUNC_COLOR_SPACE.contains(&val.as_str())
                && !valid_key(&val)
            {
                return None;
            }
            continue;
        }
        if arena.data_types.get(component) == EnumToken::MathFunctionTokenType && equals_ignore_case("calc", &s(component)) {
            let tokens = arena.tokens(component);
            let mut walker = ValueWalker::of_tokens(&tokens, WalkerValueOptions::default(), false);
            while let Some(value) = walker.next(arena) {
                if arena.data_types.get(value) == EnumToken::IdenTokenType {
                    let val = s(value).to_lowercase();
                    if !is_math_constant(&val)
                        && !KEYWORDS.contains(&val.as_str())
                        && !COLORS_FUNC.contains(&val.as_str())
                        && !COLOR_FUNC_COLOR_SPACE.contains(&val.as_str())
                        && !valid_key(&val)
                    {
                        return None;
                    }
                }
            }
        }
    }
    let to = ColorType::value(&relative_keys.to_uppercase().replace('-', "_")).unwrap_or(ColorType::NONE);
    let converted = convert_color(arena, original, to)?;
    let children: Vec<u32> = arena
        .tokens(converted)
        .into_iter()
        .filter(|&t| !matches!(arena.data_types.get(t), EnumToken::WhitespaceTokenType | EnumToken::LiteralTokenType | EnumToken::CommentTokenType))
        .collect();
    let children: &[u32] = if arena.color_type.get(converted) == ColorType::COLOR { children.get(1..).unwrap_or(&[]) } else { &children };
    let child = |i: usize| children.get(i).copied().unwrap_or(0);
    let (r, g, b, alpha) = (child(0), child(1), child(2), child(3));
    let is_none = |t: u32| arena.data_types.get(t) == EnumToken::IdenTokenType && s(t) == "none";
    let values: Components = vec![
        (names[0].clone(), get_value(arena, r, Some(converted), &names[0])),
        (names[1].clone(), get_value(arena, g, Some(converted), &names[1])),
        (names[2].clone(), get_value(arena, b, Some(converted), &names[2])),
        (
            "alpha".into(),
            if alpha == 0 {
                number_token(arena, 1.0, b)
            } else if is_none(alpha) {
                number_token(arena, 0.0, alpha)
            } else if arena.data_types.get(alpha) == EnumToken::PercentageTokenType {
                number_token(arena, get_number(arena, alpha), alpha)
            } else {
                alpha
            },
        ),
    ];
    let keys: Components = vec![
        (names[0].clone(), get_value(arena, r_exp, Some(converted), &names[0])),
        (names[1].clone(), get_value(arena, g_exp, Some(converted), &names[1])),
        (names[2].clone(), get_value(arena, b_exp, Some(converted), &names[2])),
        (
            "alpha".into(),
            get_value(
                arena,
                if a_exp == 0 {
                    number_token(arena, 1.0, b_exp)
                } else if is_none(a_exp) {
                    number_token(arena, 0.0, a_exp)
                } else {
                    a_exp
                },
                None,
                "",
            ),
        ),
    ];
    let mut result = compute_component_value(arena, keys, &values);
    if relative_keys == "hsl" {
        to_percentage(arena, record_get(&result, "s").unwrap_or(0));
        to_percentage(arena, record_get(&result, "l").unwrap_or(0));
    } else if relative_keys == "hwb" {
        to_percentage(arena, record_get(&result, "w").unwrap_or(0));
        to_percentage(arena, record_get(&result, "b").unwrap_or(0));
    }
    let alpha = record_get(&result, "alpha").unwrap_or(0);
    if arena.data_types.get(alpha) == EnumToken::NumberTokenType && arena.numeric_value.get(alpha) == 1.0 {
        result.retain(|(k, _)| k != "alpha");
    }
    Some(result)
}

/// numeric value token: percentages are scaled to the range of the component
fn get_value(arena: &AstArena, t: u32, converted: Option<u32>, component: &str) -> u32 {
    if arena.data_types.get(t) == EnumToken::PercentageTokenType {
        let mut value = get_number(arena, t);
        if let Some(converted) = converted {
            let color_space = ColorType::name(arena.color_type.get(converted)).unwrap_or("").to_lowercase();
            if let Some(range) = color_range(&color_space, component) {
                value *= range[1];
            }
        }
        return number_token(arena, value, t);
    }
    t
}

fn compute_component_value(arena: &AstArena, mut expr: Components, values: &Components) -> Components {
    let s = |t: u32| arena.resolve_string(arena.string_value.get(t)).to_string();
    for object in [values, &expr] {
        if let Some(h) = record_get(object, "h") {
            // normalize hue
            let mut walker = ValueWalker::of_tokens(&[h], WalkerValueOptions::default(), false);
            while let Some(k) = walker.next(arena) {
                if arena.data_types.get(k) == EnumToken::AngleTokenType && &*arena.resolve_string(arena.unit.get(k)) == "deg" {
                    arena.data_types.set(k, EnumToken::NumberTokenType);
                }
            }
        }
    }
    for index in 0..expr.len() {
        let exp = expr[index].1;
        let typ = arena.data_types.get(exp);
        if matches!(typ, EnumToken::NumberTokenType | EnumToken::PercentageTokenType | EnumToken::AngleTokenType | EnumToken::LengthTokenType) {
            continue;
        }
        let is_math = typ == EnumToken::MathFunctionTokenType || (typ == EnumToken::FunctionTokenType && MATH_FUNCS.contains(&s(exp).as_str()));
        if typ == EnumToken::IdenTokenType && record_get(values, &s(exp)).is_some() {
            expr[index].1 = record_get(values, &s(exp)).unwrap();
        } else if is_math {
            let tokens = arena.tokens(exp);
            let mut walker = ValueWalker::of_tokens(&tokens, WalkerValueOptions::default(), false);
            while let Some(value) = walker.next(arena) {
                if arena.data_types.get(value) == EnumToken::IdenTokenType {
                    let name = s(value);
                    // replace the identifier by its value
                    match record_get(values, &name).filter(|&r| r != 0) {
                        Some(replacement) => arena.copy_token(value, replacement),
                        None => arena.reset_token(value, EnumToken::NumberTokenType, TokenValue::Num(math_constant(&name)), None),
                    }
                }
            }
            let result = if s(exp) != "calc" { evaluate_func(arena, exp).unwrap_or_default() } else { evaluate(arena, &arena.tokens(exp)) };
            if result.len() == 1 && arena.data_types.get(result[0]) != EnumToken::BinaryExpressionTokenType {
                expr[index].1 = result[0];
            }
        }
    }
    expr
}

/// allocate a number token with the location of another token
fn number_token(arena: &AstArena, val: f64, location: u32) -> u32 {
    let token = arena.create_token(EnumToken::NumberTokenType, TokenValue::Num(val), None);
    if location != 0 {
        arena.copy_location(token, location);
    }
    token
}

/// convert a number token to a percentage (in place)
fn to_percentage(arena: &AstArena, token: u32) {
    if token == 0 {
        return;
    }
    arena.numeric_value.set(token, get_number(arena, token) * 100.0);
    arena.data_types.set(token, EnumToken::PercentageTokenType);
}
