//! Port of `src/syntax/color/hex.ts`.

use crate::syntax::constants::{color_name_to_hex, hex_to_color_name};

/// shortest representation of a hex color or a named color
pub fn reduce_hex_value(value: &str) -> String {
    let mut value = value.to_string();
    if !value.starts_with('#') {
        if let Some(hex) = color_name_to_hex(&value.to_lowercase()) {
            value = hex.to_string();
        }
    }
    let named_color = hex_to_color_name(&expand_hex_value(&value));
    let c: Vec<char> = value.chars().collect();
    if c.len() == 7 {
        if c[1] == c[2] && c[3] == c[4] && c[5] == c[6] {
            value = format!("#{}{}{}", c[1], c[3], c[5]);
        }
    } else if c.len() == 9 {
        if c[1] == c[2] && c[3] == c[4] && c[5] == c[6] && c[7] == c[8] {
            value = format!("#{}{}{}{}", c[1], c[3], c[5], if c[7] == 'f' { String::new() } else { c[7].to_string() });
        }
        if value.ends_with("ff") {
            value.truncate(value.len() - 2);
        }
    }
    match named_color {
        Some(n) if n.len() <= value.len() => n.to_string(),
        _ => value,
    }
}

/// `#abc` -> `#aabbcc`, `#abcd` -> `#aabbccdd`
pub fn expand_hex_value(value: &str) -> String {
    let c: Vec<char> = value.chars().collect();
    match c.len() {
        4 => format!("#{0}{0}{1}{1}{2}{2}", c[1], c[2], c[3]),
        5 => format!("#{0}{0}{1}{1}{2}{2}{3}{3}", c[1], c[2], c[3], c[4]),
        _ => value.to_string(),
    }
}

use super::color::{color2srgbvalues, get_number};
use super::rgb::{cmyk2rgbvalues, hsl2rgbvalues, hwb2rgbvalues, srgb2rgb};
use super::srgb::{lab2srgbvalues, lch2srgbvalues, oklab2srgbvalues, oklch2srgbvalues};
use super::utils::components::get_color_components;
use super::utils::values::hex2;
use crate::ast::arena::AstArena;
use crate::ast::types::{ColorType, EnumToken};
use crate::utils::number::js_round;

/// `acc + value.toString(16).padStart(2, "0")`
fn to_hex_string(values: &[f64], prefix: &str) -> String {
    values.iter().fold(prefix.to_string(), |acc, &v| acc + &hex2(v))
}

fn from_string(arena: &AstArena, value: Option<String>) -> Option<u32> {
    Some(hex_token(arena, &value?))
}

pub fn rgb2hex_token(arena: &AstArena, token: u32) -> Option<u32> {
    from_string(arena, rgb2hexvalues(arena, token))
}

pub fn hsl2hex_token(arena: &AstArena, token: u32) -> Option<u32> {
    from_string(arena, hsl2hexvalues(arena, token))
}

pub fn cmyk2hex_token(arena: &AstArena, token: u32) -> Option<u32> {
    from_string(arena, cmyk2hexvalues(arena, token))
}

pub fn hwb2hex_token(arena: &AstArena, token: u32) -> Option<u32> {
    from_string(arena, hwb2hexvalues(arena, token))
}

pub fn color2hex_token(arena: &AstArena, token: u32) -> Option<u32> {
    let value = color2srgbvalues(arena, token)?;
    let rgb: Vec<f64> = value.iter().map(|&curr| srgb2rgb(curr + f64::EPSILON)).collect();
    Some(hex_token(arena, &to_hex_string(&rgb, "#")))
}

fn srgb_hex_token(arena: &AstArena, value: Option<Vec<f64>>) -> Option<u32> {
    let rgb: Vec<f64> = value?.iter().map(|&curr| srgb2rgb(curr)).collect();
    Some(hex_token(arena, &to_hex_string(&rgb, "#")))
}

pub fn oklab2hex_token(arena: &AstArena, token: u32) -> Option<u32> {
    srgb_hex_token(arena, oklab2srgbvalues(arena, token))
}

pub fn oklch2hex_token(arena: &AstArena, token: u32) -> Option<u32> {
    srgb_hex_token(arena, oklch2srgbvalues(arena, token))
}

pub fn lab2hex_token(arena: &AstArena, token: u32) -> Option<u32> {
    srgb_hex_token(arena, lab2srgbvalues(arena, token))
}

pub fn lch2hex_token(arena: &AstArena, token: u32) -> Option<u32> {
    srgb_hex_token(arena, lch2srgbvalues(arena, token))
}

fn hex_token(arena: &AstArena, value: &str) -> u32 {
    let value = reduce_hex_value(value);
    let kin = if value.starts_with('#') { ColorType::HEX } else { ColorType::LIT };
    arena.create_color(&value, kin, None)
}

pub fn rgb2hexvalues(arena: &AstArena, token: u32) -> Option<String> {
    let components = get_color_components(arena, token)?;
    if components.len() < 3 {
        return None;
    }
    let mut value = String::from("#");
    for &t in &components[..3] {
        if arena.data_types.get(t) == EnumToken::Iden && &*arena.resolve_string(arena.string_value.get(t)) == "none" {
            value.push_str("00");
        } else {
            let factor = if arena.data_types.get(t) == EnumToken::PercentageTokenType { 255.0 } else { 1.0 };
            value.push_str(&hex2(js_round(get_number(arena, t) * factor)));
        }
    }
    if components.len() == 4 {
        let v = get_number(arena, components[3]);
        if v < 1.0 {
            value.push_str(&hex2(js_round(255.0 * v)));
        }
    }
    Some(value)
}

fn rgb_values_hex(t: Option<Vec<f64>>, prefix: &str) -> Option<String> {
    let mut t = t?;
    if t.len() == 4 {
        t[3] = srgb2rgb(t[3]);
    }
    Some(to_hex_string(&t, prefix))
}

pub fn hsl2hexvalues(arena: &AstArena, token: u32) -> Option<String> {
    rgb_values_hex(hsl2rgbvalues(arena, token), "#")
}

pub fn hwb2hexvalues(arena: &AstArena, token: u32) -> Option<String> {
    rgb_values_hex(hwb2rgbvalues(arena, token), "#")
}

pub fn cmyk2hexvalues(arena: &AstArena, token: u32) -> Option<String> {
    rgb_values_hex(cmyk2rgbvalues(arena, token), "#")
}
