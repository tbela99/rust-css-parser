//! Port of `src/syntax/color/utils/color-type.ts`.

use crate::ast::arena::AstArena;
use crate::ast::types::ColorType;

/// color type of the color components. `None`: unknown (`undefined` / `null`)
pub fn get_color_type(arena: &AstArena, color: u32) -> Option<u8> {
    if arena.color_type.get(color) == ColorType::COLOR_MIX || &*arena.resolve_string(arena.unit.get(color)) == "rel" {
        let token = arena.get_token_at(color, 2)?;
        return if arena.color_type.get(color) == ColorType::COLOR_MIX {
            // `ColorType[val.toUpperCase().replace("-", "_")]`: the first "-" only
            ColorType::value(&arena.resolve_string(arena.string_value.get(token)).to_uppercase().replacen('-', "_", 1))
        } else {
            get_color_type(arena, token)
        };
    }
    Some(arena.color_type.get(color))
}
