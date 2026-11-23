//! Port of `src/syntax/color/rgb.ts`.

use super::color::{color2srgbvalues, minmax};
use super::srgb::{cmyk2srgbvalues, hex_value, hslvalues, hslvalues2srgbvalues, hwb2srgbvalues, lab2srgbvalues, lch2srgbvalues, oklab2srgbvalues, oklch2srgbvalues, parse_hex_pair};
use super::tokens::{number, percentage};
use super::utils::values::at;
use crate::ast::arena::AstArena;
use crate::ast::types::ColorType;
use crate::utils::number::{js_parse_number, js_round, to_fixed};

pub fn srgb2rgb(value: f64) -> f64 {
    minmax(js_round(value * 255.0), 0.0, 255.0)
}

macro_rules! to_token {
    ($name:ident, $values:ident) => {
        pub fn $name(arena: &AstArena, token: u32) -> Option<u32> {
            let values = $values(arena, token)?;
            Some(rgb2rgb_token(arena, &values))
        }
    };
}

pub fn hex2rgb_token(arena: &AstArena, token: u32) -> Option<u32> {
    Some(rgb2rgb_token(arena, &hex2rgbvalues(arena, token)))
}

to_token!(hsl2rgb_token, hsl2rgbvalues);
to_token!(hwb2rgb_token, hwb2rgbvalues);
to_token!(cmyk2rgb_token, cmyk2rgbvalues);
to_token!(oklab2rgb_token, oklab2rgbvalues);
to_token!(oklch2rgb_token, oklch2rgbvalues);
to_token!(lab2rgb_token, lab2rgbvalues);
to_token!(lch2rgb_token, lch2rgbvalues);

pub fn color2rgb_token(arena: &AstArena, token: u32) -> Option<u32> {
    let values = color2srgbvalues(arena, token)?;
    Some(rgb2rgb_token(arena, &to_rgb(values)))
}

fn rgb2rgb_token(arena: &AstArena, values: &[f64]) -> u32 {
    let mut chi = vec![number(arena, at(values, 0)), number(arena, at(values, 1)), number(arena, at(values, 2))];
    if values.len() == 4 {
        chi.push(percentage(arena, values[3] * 100.0));
    }
    arena.create_color("rgb", ColorType::RGB, Some(&chi))
}

/// `values.map((t, index) => index === 3 ? t : srgb2rgb(t))`
fn to_rgb(values: Vec<f64>) -> Vec<f64> {
    values.into_iter().enumerate().map(|(index, t)| if index == 3 { t } else { srgb2rgb(t) }).collect()
}

pub fn hex2rgbvalues(arena: &AstArena, token: u32) -> Vec<f64> {
    let value = hex_value(arena, token);
    let len = value.chars().count();
    let mut rgb = Vec::new();
    let mut i = 1;
    while i < len {
        rgb.push(parse_hex_pair(&value, i));
        i += 2;
    }
    if rgb.len() == 4 {
        if rgb[3] == 255.0 {
            rgb.pop();
        } else {
            rgb[3] = js_parse_number(&to_fixed(rgb[3] / 255.0, 2));
        }
    }
    rgb
}

pub fn hwb2rgbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_rgb(hwb2srgbvalues(arena, token)?))
}

pub fn hsl2rgbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let v = hslvalues(arena, token)?;
    let mut values: Vec<f64> = hslvalues2srgbvalues(v.h, v.s, v.l, None).into_iter().map(|t| minmax(js_round(t * 255.0), 0.0, 255.0)).collect();
    if let Some(a) = v.a.filter(|&a| a != 1.0) {
        values.push(a);
    }
    Some(values)
}

pub fn hsl2srgbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let v = hslvalues(arena, token)?;
    let mut values = hslvalues2srgbvalues(v.h, v.s, v.l, None);
    if let Some(a) = v.a.filter(|&a| a != 1.0) {
        values.push(a);
    }
    Some(values)
}

pub fn cmyk2rgbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_rgb(cmyk2srgbvalues(arena, token)?))
}

pub fn oklab2rgbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_rgb(oklab2srgbvalues(arena, token)?))
}

pub fn oklch2rgbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_rgb(oklch2srgbvalues(arena, token)?))
}

pub fn lab2rgbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_rgb(lab2srgbvalues(arena, token)?))
}

pub fn lch2rgbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_rgb(lch2srgbvalues(arena, token)?))
}
