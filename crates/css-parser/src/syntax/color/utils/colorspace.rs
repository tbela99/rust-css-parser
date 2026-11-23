//! Port of `src/syntax/color/utils/colorspace.ts`.

use super::components::get_color_components;
use crate::ast::arena::AstArena;
use crate::parser::utils::text::equals_ignore_case;

/// color space of a color
pub fn get_color_space(arena: &AstArena, color: u32) -> Option<String> {
    let components = get_color_components(arena, color)?;
    if components.len() < 3 {
        return None;
    }
    let s = |t: u32| arena.resolve_string(arena.string_value.get(t)).to_string();
    let color_name = s(color);
    let name = if equals_ignore_case("color", &color_name) {
        if equals_ignore_case("from", &s(components[0])) {
            s(components[2])
        } else {
            s(components[0])
        }
    } else if equals_ignore_case("color-mix", &color_name) {
        if equals_ignore_case("in", &s(components[0])) {
            s(components[1])
        } else {
            return Some("oklab".into());
        }
    } else {
        color_name
    };
    Some(if name == "device-cmyk" {
        "cmyk".into()
    } else if name.starts_with("xyz") {
        "xyz".into()
    } else if ["srgb", "srgb-linear", "display-p3", "a98-rgb", "prophoto-rgb", "rec2020", "rgb"].iter().any(|t| equals_ignore_case(t, &name)) {
        "rgb".into()
    } else {
        // `name.slice(-3)`
        let chars: Vec<char> = name.chars().collect();
        chars[chars.len().saturating_sub(3)..].iter().collect()
    })
}
