//! Port of `src/syntax/color/utils/make-color.ts`.

use super::values::hex2;
use crate::ast::arena::{AstArena, TokenValue};
use crate::ast::types::{ColorType, EnumToken};
use crate::parser::utils::text::equals_ignore_case;
use crate::syntax::color::color::{get_number, minmax};
use crate::utils::number::js_round;

/// create a color token
pub fn make_color(arena: &AstArena, kind: u8, components: &[u32], alpha: Option<u32>) -> Option<u32> {
    if components.len() < 3 {
        return None;
    }
    let alpha_value = match alpha {
        None => 1.0,
        Some(alpha) => {
            if arena.data_types.get(alpha) == EnumToken::IdenTokenType && equals_ignore_case(&arena.resolve_string(arena.string_value.get(alpha)), "none") {
                0.0
            } else {
                minmax(get_number(arena, alpha), 0.0, 1.0)
            }
        }
    };
    // the components may belong to another color token: a token belongs to one token list
    let mut components: Vec<u32> = components[..3]
        .iter()
        .map(|&c| if arena.parents.get(c) == 0 { c } else { arena.clone_node_deep(c, true, None) })
        .collect();
    if alpha_value != 1.0 {
        components.push(arena.create_token(EnumToken::NumberTokenType, TokenValue::Num(alpha_value), None));
    }
    if kind == ColorType::LIT || kind == ColorType::HEX {
        let mut val = String::from("#");
        for (index, &token) in components.iter().enumerate() {
            let mut value = get_number(arena, token);
            if index == 3 {
                value *= 255.0;
            }
            val.push_str(&hex2(js_round(value)));
        }
        return Some(arena.create_color(&val, ColorType::HEX, None));
    }
    let has_alpha = components.len() == 4;
    let mut i = components.len();
    while i > 1 {
        i -= 1;
        components.insert(i, arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None));
    }
    if has_alpha {
        let len = components.len();
        components[len - 2] = arena.create_token(EnumToken::LiteralTokenType, TokenValue::Str("/"), None);
    }
    let name = ColorType::name(kind).unwrap_or("").to_lowercase();
    Some(arena.create_color(&name, kind, Some(&components)))
}
