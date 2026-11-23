//! Port of `src/syntax/color/alpha.ts`.

use super::color::convert_color;
use super::utils::color_type::get_color_type;
use super::utils::components::get_color_components;
use super::utils::make_color::make_color;
use crate::ast::arena::{AstArena, TokenValue};
use crate::ast::math::expression::evaluate;
use crate::ast::types::{ColorType, EnumToken};
use crate::ast::walk::{ValueWalker, WalkerValueOptions};
use crate::parser::utils::text::equals_ignore_case;

/// `alpha(color / alpha)`: the color with another alpha
pub fn alpha(arena: &AstArena, color: u32, alpha: Option<u32>) -> Option<u32> {
    let Some(mut alpha) = alpha else {
        return Some(color);
    };
    let mut color = color;
    if arena.color_type.get(color) == ColorType::DEVICE_CMYK {
        return None;
    }
    let mut components = get_color_components(arena, color);
    let s = |t: u32| arena.resolve_string(arena.string_value.get(t));
    if arena.data_types.get(alpha) == EnumToken::MathFunctionTokenType {
        let tokens = arena.tokens(alpha);
        let mut walker = ValueWalker::of_tokens(&tokens, WalkerValueOptions::default(), false);
        while let Some(value) = walker.next(arena) {
            if arena.data_types.get(value) == EnumToken::IdenTokenType && equals_ignore_case(&s(value), "alpha") {
                match components.as_ref().and_then(|c| c.get(3)).copied().filter(|&c| c != 0) {
                    Some(c) => arena.copy_token(value, c),
                    None => arena.reset_token(value, EnumToken::NumberTokenType, TokenValue::Num(1.0), None),
                }
            }
        }
        let result = evaluate(arena, &[alpha]);
        if result.len() == 1 {
            alpha = result[0];
        }
    }
    let typ = arena.data_types.get(alpha);
    if typ != EnumToken::IdenTokenType && typ != EnumToken::NumberTokenType && typ != EnumToken::PercentageTokenType {
        return None;
    }
    if typ == EnumToken::IdenTokenType {
        let v = s(alpha);
        if equals_ignore_case(&v, "NaN") || equals_ignore_case(&v, "Infinity") || equals_ignore_case(&v, "-Infinity") {
            return None;
        }
    }
    if arena.color_type.get(color) == ColorType::COLOR_MIX || &*arena.resolve_string(arena.unit.get(color)) == "rel" {
        color = convert_color(arena, color, get_color_type(arena, color).unwrap_or(ColorType::NONE))?;
        components = get_color_components(arena, color);
    }
    let components = components?;
    if arena.data_types.get(alpha) == EnumToken::IdenTokenType && equals_ignore_case(&s(alpha), "none") {
        arena.reset_token(alpha, EnumToken::NumberTokenType, TokenValue::Num(0.0), None);
    }
    make_color(arena, arena.color_type.get(color), &components, Some(alpha))
}
