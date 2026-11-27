//! Port of `src/syntax/color/hsl.ts`.

use super::color::{color2srgbvalues, get_number};
use super::hwb::hwbvalues;
use super::rgb::{cmyk2rgbvalues, lab2rgbvalues, lch2rgbvalues};
use super::srgb::{hex2srgbvalues, oklab2srgbvalues, oklch2srgbvalues};
use super::tokens::{number, percentage, slash};
use super::utils::components::get_color_components;
use super::utils::values::{at, js_max, js_min, opt};
use crate::ast::arena::AstArena;
use crate::ast::types::{ColorType, EnumToken};

macro_rules! to_token {
    ($name:ident, $values:ident) => {
        pub fn $name(arena: &AstArena, token: u32) -> Option<u32> {
            let values = $values(arena, token)?;
            Some(hsl_token(arena, values))
        }
    };
}

pub fn hex2hsl_token(arena: &AstArena, token: u32) -> Option<u32> {
    let values = hex2srgbvalues(arena, token);
    Some(hsl_token(arena, srgb2hsl(&values)))
}

to_token!(rgb2hsl_token, rgb2hslvalues);
to_token!(hwb2hsl_token, hwb2hslvalues);
to_token!(cmyk2hsl_token, cmyk2hslvalues);
to_token!(oklab2hsl_token, oklab2hslvalues);
to_token!(oklch2hsl_token, oklch2hslvalues);
to_token!(lab2hsl_token, lab2hslvalues);
to_token!(lch2hsl_token, lch2hslvalues);

pub fn color2hsl_token(arena: &AstArena, token: u32) -> Option<u32> {
    let values = color2srgbvalues(arena, token)?;
    Some(hsl_token(arena, srgb2hsl(&values)))
}

fn srgb2hsl(values: &[f64]) -> Vec<f64> {
    srgb2hslvalues(at(values, 0), at(values, 1), at(values, 2), opt(values, 3))
}

fn hsl_token(arena: &AstArena, mut values: Vec<f64>) -> u32 {
    if values.get(1) == Some(&0.0) {
        values[0] = 0.0;
    }
    if matches!(values.get(2), Some(&v) if v == 1.0 || v == 0.0) {
        values[0] = 0.0;
        values[1] = 0.0;
    }
    let mut chi = vec![number(arena, at(&values, 0)), percentage(arena, at(&values, 1)), percentage(arena, at(&values, 2))];
    if values.len() == 4 && values[3] != 1.0 {
        chi.push(slash(arena));
        chi.push(percentage(arena, values[3] * 100.0));
    }
    arena.create_color("hsl", ColorType::HSL, Some(&chi))
}

/// value of a red, green or blue component of an `rgb()` color, from 0 to 255: `100%` is 255
fn rgb_channel(arena: &AstArena, token: u32) -> f64 {
    (if arena.data_types.get(token) == EnumToken::PercentageTokenType { 255.0 } else { 1.0 }) * get_number(arena, token)
}

pub fn rgb2hslvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let chi = get_color_components(arena, token)?;
    if chi.len() < 3 {
        return None;
    }
    let r = rgb_channel(arena, chi[0]);
    let g = rgb_channel(arena, chi[1]);
    let b = rgb_channel(arena, chi[2]);
    let a = if chi.len() == 4 { Some(get_number(arena, chi[3])) } else { None };
    let a = a.filter(|&a| a != 1.0);
    Some(rgbvalues2hslvalues(r, g, b, a))
}

pub fn cmyk2hslvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let values = cmyk2rgbvalues(arena, token)?;
    Some(rgbvalues2hslvalues(at(&values, 0), at(&values, 1), at(&values, 2), opt(&values, 3)))
}

pub fn hwb2hslvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let v = hwbvalues(arena, token)?;
    let w_norm = v.w / 100.0;
    let b_norm = v.b / 100.0;
    let hue = ((v.h % 360.0) + 360.0) % 360.0;
    if w_norm + b_norm >= 1.0 {
        let gray_lightness = w_norm / (w_norm + b_norm);
        return Some(vec![hue, 0.0, gray_lightness * 100.0]);
    }
    let lightness = (1.0 - b_norm + w_norm) / 2.0;
    let mut saturation = 0.0;
    if lightness > 0.0 && lightness < 1.0 {
        let chroma = 1.0 - w_norm - b_norm;
        saturation = chroma / (1.0 - (2.0 * lightness - 1.0).abs());
    }
    let mut hsl = vec![hue, saturation * 100.0, lightness * 100.0];
    if let Some(a) = v.a.filter(|&a| a != 1.0) {
        hsl.push(a);
    }
    Some(hsl)
}

pub fn lab2hslvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let values = lab2rgbvalues(arena, token)?;
    Some(rgbvalues2hslvalues(at(&values, 0), at(&values, 1), at(&values, 2), opt(&values, 3)))
}

pub fn lch2hslvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let values = lch2rgbvalues(arena, token)?;
    Some(rgbvalues2hslvalues(at(&values, 0), at(&values, 1), at(&values, 2), opt(&values, 3)))
}

pub fn oklab2hslvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(srgb2hsl(&oklab2srgbvalues(arena, token)?))
}

pub fn oklch2hslvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(srgb2hsl(&oklch2srgbvalues(arena, token)?))
}

pub fn rgbvalues2hslvalues(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    srgb2hslvalues(r / 255.0, g / 255.0, b / 255.0, a)
}

pub fn srgb2hslvalues(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    let max = js_max(&[r, g, b]);
    let min = js_min(&[r, g, b]);
    let delta = max - min;
    let l = (max + min) / 2.0;
    let mut h = 0.0;
    let mut s = 0.0;
    if delta != 0.0 {
        s = delta / (1.0 - (2.0 * l - 1.0).abs());
        if max == r {
            h = 60.0 * (((g - b) / delta) % 6.0);
        } else if max == g {
            h = 60.0 * ((b - r) / delta + 2.0);
        } else if max == b {
            h = 60.0 * ((r - g) / delta + 4.0);
        }
    }
    h = ((h % 360.0) + 360.0) % 360.0;
    let mut hsl = vec![h, s * 100.0, l * 100.0];
    if let Some(a) = a.filter(|&a| a < 1.0) {
        hsl.push(a);
    }
    hsl
}
