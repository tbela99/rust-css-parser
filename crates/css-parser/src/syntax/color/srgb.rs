//! Port of `src/syntax/color/srgb.ts` (from <https://www.w3.org/TR/css-color-4/#color-conversion-code>).

use super::color::{color2srgbvalues, get_angle, get_number};
use super::hex::expand_hex_value;
use super::hwb::hwbvalues;
use super::lab::{get_lab_components, lab_to_srgb, lchvalues2labvalues};
use super::lch::get_lch_components;
use super::oklab::{get_oklab_components, oklab_to_srgb};
use super::oklch::get_oklch_components;
use super::utils::components::get_color_components;
use super::utils::values::{at, js_sign_or_one, opt};
use super::xyz::xyz_to_lin_srgb;
use crate::ast::arena::AstArena;
use crate::ast::types::{ColorType, EnumToken};
use crate::syntax::constants::color_name_to_hex;
use crate::syntax::syntax::parse_color;
use crate::utils::number::{js_parse_number, to_fixed};

/// srgb values of a color
pub fn srgbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let mut token = token;
    if arena.data_types.get(token) == EnumToken::IdenTokenType {
        token = parse_color(arena, token);
    }
    match arena.color_type.get(token) {
        ColorType::LIT | ColorType::HEX => Some(hex2srgbvalues(arena, token)),
        ColorType::RGB => rgb2srgb(arena, token),
        ColorType::HSL => hsl2srgb(arena, token),
        ColorType::HWB => hwb2srgbvalues(arena, token),
        ColorType::LAB => lab2srgbvalues(arena, token),
        ColorType::LCH => lch2srgbvalues(arena, token),
        ColorType::OKLAB => oklab2srgbvalues(arena, token),
        ColorType::OKLCH => oklch2srgbvalues(arena, token),
        ColorType::COLOR => color2srgbvalues(arena, token),
        _ => None,
    }
}

fn is_none(arena: &AstArena, t: u32) -> bool {
    arena.data_types.get(t) == EnumToken::IdenTokenType && &*arena.resolve_string(arena.string_value.get(t)) == "none"
}

pub fn rgb2srgb(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(
        get_color_components(arena, token)?
            .iter()
            .enumerate()
            .map(|(index, &t)| {
                if index == 3 {
                    if is_none(arena, t) {
                        1.0
                    } else {
                        get_number(arena, t)
                    }
                } else {
                    (if arena.data_types.get(t) == EnumToken::PercentageTokenType { 255.0 } else { 1.0 }) * get_number(arena, t) / 255.0
                }
            })
            .collect(),
    )
}

pub fn rgb2srgbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(
        get_color_components(arena, token)?
            .iter()
            .enumerate()
            .map(|(index, &t)| if index == 3 { get_number(arena, t) } else { get_number(arena, t) / 255.0 })
            .collect(),
    )
}

/// `COLORS_NAMES[name]` of a named color or the value of a hex color, expanded
pub(super) fn hex_value(arena: &AstArena, token: u32) -> String {
    let val = arena.resolve_string(arena.string_value.get(token));
    if arena.color_type.get(token) == ColorType::LIT {
        // typescript: `undefined` when the name is not a color name
        expand_hex_value(color_name_to_hex(&val.to_lowercase()).unwrap_or(""))
    } else {
        expand_hex_value(&val)
    }
}

/// `parseInt(value.slice(i, i + 2), 16)`
pub(super) fn parse_hex_pair(value: &str, i: usize) -> f64 {
    let s: String = value.chars().skip(i).take(2).collect();
    let digits: String = s.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    if digits.is_empty() {
        f64::NAN
    } else {
        u32::from_str_radix(&digits, 16).unwrap() as f64
    }
}

pub fn hex2srgbvalues(arena: &AstArena, token: u32) -> Vec<f64> {
    let value = hex_value(arena, token);
    let len = value.chars().count();
    let mut rgb = Vec::new();
    let mut i = 1;
    while i < len {
        rgb.push(parse_hex_pair(&value, i) / 255.0);
        i += 2;
    }
    if rgb.len() == 4 {
        rgb[3] = js_parse_number(&to_fixed(rgb[3], 2));
    }
    rgb
}

/// xyz d65 input
pub fn xyz2srgb(x: f64, y: f64, z: f64, alpha: Option<f64>) -> Vec<f64> {
    let values = xyz_to_lin_srgb(x, y, z, None);
    lsrgb2srgbvalues(at(&values, 0), at(&values, 1), at(&values, 2), alpha)
}

pub fn hwb2srgbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    // the components cannot be computed: var(), ...
    let v = hwbvalues(arena, token)?;
    let mut rgb = hwb2srgb(v.h, v.w, v.b, None);
    if let Some(a) = v.a {
        if a != 1.0 {
            rgb.push(a);
        }
    }
    Some(rgb)
}

/// convert hwb to sRGB
pub fn hwb2srgb(h: f64, w: f64, b: f64, alpha: Option<f64>) -> Vec<f64> {
    use crate::utils::number::js_round;
    let w_norm = w / 100.0;
    let b_norm = b / 100.0;
    if w_norm + b_norm >= 1.0 {
        let gray = w_norm / (w_norm + b_norm);
        let g = js_round(gray * 10000.0) / 10000.0;
        return vec![g, g, g];
    }
    let h_prime = (h % 360.0) / 60.0;
    let x = 1.0 - ((h_prime % 2.0) - 1.0).abs();
    let (r_chroma, g_chroma, b_chroma) = if (0.0..1.0).contains(&h_prime) {
        (1.0, x, 0.0)
    } else if (1.0..2.0).contains(&h_prime) {
        (x, 1.0, 0.0)
    } else if (2.0..3.0).contains(&h_prime) {
        (0.0, 1.0, x)
    } else if (3.0..4.0).contains(&h_prime) {
        (0.0, x, 1.0)
    } else if (4.0..5.0).contains(&h_prime) {
        (x, 0.0, 1.0)
    } else if (5.0..=6.0).contains(&h_prime) {
        (1.0, 0.0, x)
    } else {
        (0.0, 0.0, 0.0)
    };
    let factor = 1.0 - w_norm - b_norm;
    let mut srgb = vec![r_chroma * factor + w_norm, g_chroma * factor + w_norm, b_chroma * factor + w_norm];
    if let Some(alpha) = alpha {
        if alpha != 1.0 {
            srgb.push(alpha);
        }
    }
    srgb
}

pub fn hsl2srgb(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let v = hslvalues(arena, token)?;
    // hue in degrees
    Some(hslvalues2srgbvalues(v.h, v.s, v.l, v.a))
}

pub fn cmyk2srgbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let components = get_color_components(arena, token)?;
    let n = |i: usize| components.get(i).map_or(f64::NAN, |&t| get_number(arena, t));
    let (c, m, y, k) = (n(0), n(1), n(2), n(3));
    let mul = 1.0 - k;
    // `Math.min(1, v)`
    let min1 = |v: f64| if v.is_nan() { f64::NAN } else { v.min(1.0) };
    let mut rgb = vec![1.0 - min1(c * mul + k), 1.0 - min1(m * mul + k), 1.0 - min1(y * mul + k)];
    if components.len() == 5 {
        rgb.push(get_number(arena, components[4]));
    }
    Some(rgb)
}

pub fn oklab2srgbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let values = get_oklab_components(arena, token).unwrap_or_default();
    if values.len() < 3 {
        return None;
    }
    let mut rgb = oklab_to_srgb(values[0], values[1], values[2]);
    if let Some(alpha) = opt(&values, 3) {
        if alpha != 1.0 {
            rgb.push(alpha);
        }
    }
    Some(rgb)
}

pub fn oklch2srgbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let values = get_oklch_components(arena, token).unwrap_or_default();
    if values.len() < 3 {
        return None;
    }
    let lab = lchvalues2labvalues(values[0], values[1], values[2], None);
    let mut rgb = oklab_to_srgb(lab[0], lab[1], lab[2]);
    // `alpha !== 1`: the alpha is always defined (`getOKLCHComponents()`)
    let alpha = opt(&values, 3);
    if alpha != Some(1.0) {
        rgb.push(alpha.unwrap_or(f64::NAN));
    }
    Some(rgb)
}

/// h, s, l, a values of a hsl color
pub struct HslValues {
    pub h: f64,
    pub s: f64,
    pub l: f64,
    pub a: Option<f64>,
}

pub fn hslvalues(arena: &AstArena, token: u32) -> Option<HslValues> {
    let components = get_color_components(arena, token)?;
    let n = |i: usize| components.get(i).map_or(f64::NAN, |&t| get_number(arena, t));
    let typ = |i: usize| components.get(i).map_or(0, |&t| arena.data_types.get(t));
    let h = components.first().map_or(f64::NAN, |&t| get_angle(arena, t)) * 360.0;
    let mut s = n(1);
    let mut l = n(2);
    if typ(1) == EnumToken::PercentageTokenType {
        s *= 100.0;
    }
    if typ(2) == EnumToken::PercentageTokenType {
        l *= 100.0;
    }
    let a = if components.len() == 4 { Some(n(3)) } else { None };
    Some(HslValues { h, s, l, a })
}

pub fn hslvalues2srgbvalues(h: f64, s: f64, l: f64, a: Option<f64>) -> Vec<f64> {
    let s_norm = s / 100.0;
    let l_norm = l / 100.0;
    let chroma = (1.0 - (2.0 * l_norm - 1.0).abs()) * s_norm;
    let h_prime = h / 60.0;
    let x = chroma * (1.0 - ((h_prime % 2.0) - 1.0).abs());
    let (r, g, b) = if (0.0..1.0).contains(&h_prime) {
        (chroma, x, 0.0)
    } else if (1.0..2.0).contains(&h_prime) {
        (x, chroma, 0.0)
    } else if (2.0..3.0).contains(&h_prime) {
        (0.0, chroma, x)
    } else if (3.0..4.0).contains(&h_prime) {
        (0.0, x, chroma)
    } else if (4.0..5.0).contains(&h_prime) {
        (x, 0.0, chroma)
    } else if (5.0..=6.0).contains(&h_prime) {
        (chroma, 0.0, x)
    } else {
        (0.0, 0.0, 0.0)
    };
    let m = l_norm - chroma / 2.0;
    let mut values = vec![r + m, g + m, b + m];
    if let Some(a) = a {
        values.push(a);
    }
    // exact float decimals instead of multiplying by 255
    values
}

pub fn lab2srgbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let values = get_lab_components(arena, token).unwrap_or_default();
    if values.len() < 3 {
        return None;
    }
    let mut rgb = lab_to_srgb(values[0], values[1], values[2]);
    if let Some(alpha) = opt(&values, 3) {
        if alpha < 1.0 {
            rgb.push(alpha);
        }
    }
    Some(rgb)
}

pub fn lch2srgbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let components = get_lch_components(arena, token)?;
    let values = lchvalues2labvalues(at(&components, 0), at(&components, 1), at(&components, 2), opt(&components, 3));
    // https://www.w3.org/TR/css-color-4/#lab-to-lch
    let mut rgb = lab_to_srgb(values[0], values[1], values[2]);
    let alpha = opt(&values, 3);
    if alpha != Some(1.0) {
        rgb.push(alpha.unwrap_or(f64::NAN));
    }
    Some(rgb)
}

/// sRGB -> lRGB
pub fn srgb2lsrgbvalues(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    let mut rgb: Vec<f64> = [r, g, b]
        .iter()
        .map(|&val| {
            let abs = val.abs();
            if abs <= 0.04045 {
                return val / 12.92;
            }
            js_sign_or_one(val) * ((abs + 0.055) / 1.055).powf(2.4)
        })
        .collect();
    if let Some(a) = a {
        if a != 1.0 {
            rgb.push(a);
        }
    }
    rgb
}

pub fn lsrgb2srgbvalues(r: f64, g: f64, b: f64, alpha: Option<f64>) -> Vec<f64> {
    let mut rgb: Vec<f64> = [r, g, b]
        .iter()
        .map(|&val| {
            let abs = val.abs();
            if abs > 0.0031308 {
                return js_sign_or_one(val) * (1.055 * abs.powf(1.0 / 2.4) - 0.055);
            }
            12.92 * val
        })
        .collect();
    if let Some(alpha) = alpha {
        if alpha != 1.0 {
            rgb.push(alpha);
        }
    }
    rgb
}
