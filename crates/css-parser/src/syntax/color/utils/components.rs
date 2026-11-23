//! Port of `src/syntax/color/utils/components.ts`.

use crate::ast::arena::{AstArena, TokenValue};
use crate::ast::types::{ColorType, EnumToken};
use crate::parser::utils::text::equals_ignore_case;
use crate::syntax::color::hex::expand_hex_value;
use crate::syntax::constants::color_name_to_hex;

/// `/([a-fA-F0-9]{2})/g` matches
fn hex_pairs(value: &str) -> Vec<&str> {
    let bytes = value.as_bytes();
    let mut result = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i].is_ascii_hexdigit() && bytes[i + 1].is_ascii_hexdigit() {
            result.push(&value[i..i + 2]);
            i += 2;
        } else {
            i += 1;
        }
    }
    result
}

/// components of a color: the number tokens of a hex color, the value tokens of a color function. `None`: the color
/// cannot be computed (`currentcolor`, `var()`)
pub fn get_color_components(arena: &AstArena, token: u32) -> Option<Vec<u32>> {
    let kin = arena.color_type.get(token);
    if kin == ColorType::HEX || kin == ColorType::LIT {
        let val = arena.resolve_string(arena.string_value.get(token));
        if equals_ignore_case("currentcolor", &val) {
            return None;
        }
        // typescript throws when the name is not a color name
        let value = expand_hex_value(if kin == ColorType::LIT { color_name_to_hex(&val.to_lowercase())? } else { &val });
        let pairs = hex_pairs(value.get(1..).unwrap_or(""));
        if pairs.is_empty() {
            return None;
        }
        return Some(
            pairs
                .iter()
                .enumerate()
                .map(|(index, t)| {
                    let v = u8::from_str_radix(t, 16).unwrap() as f64;
                    arena.create_token(EnumToken::Number, TokenValue::Num(if index < 3 { v } else { v / 255.0 }), None)
                })
                .collect(),
        );
    }
    let mut result = Vec::new();
    let mut child = arena.first_token.get(token);
    while child != 0 {
        let typ = arena.data_types.get(child);
        if !matches!(
            typ,
            EnumToken::LiteralTokenType | EnumToken::CommentTokenType | EnumToken::CommaTokenType | EnumToken::WhitespaceTokenType
        ) {
            if matches!(typ, EnumToken::FunctionTokenType | EnumToken::WildCardFunctionTokenType | EnumToken::MathFunctionTokenType)
                && arena.with_string(arena.string_value.get(child), |name| equals_ignore_case("var", name))
            {
                return None;
            }
            if typ == EnumToken::ColorTokenType && equals_ignore_case("currentcolor", &arena.resolve_string(arena.string_value.get(child))) {
                return None;
            }
            result.push(child);
        }
        child = arena.next_sibling.get(child);
    }
    Some(result)
}
