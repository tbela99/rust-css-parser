//! Port of `src/syntax/color/cmyk.ts`.

use super::color::color2srgbvalues;
use super::rgb::hsl2srgbvalues;
use super::srgb::{hwb2srgbvalues, lab2srgbvalues, lch2srgbvalues, oklab2srgbvalues, oklch2srgbvalues, rgb2srgbvalues};
use super::tokens::{percentage, slash};
use super::utils::values::{at, js_max, opt};
use crate::ast::arena::AstArena;
use crate::ast::types::ColorType;

fn to_cmyk_token(arena: &AstArena, values: &[f64]) -> u32 {
    cmyktoken(arena, &srgb2cmykvalues(at(values, 0), at(values, 1), at(values, 2), opt(values, 3)))
}

/// conversion of the srgb values with at least 3 components
fn with3(arena: &AstArena, values: Option<Vec<f64>>) -> Option<u32> {
    let values = values?;
    if values.len() < 3 {
        return None;
    }
    Some(to_cmyk_token(arena, &values))
}

pub fn rgb2cmyk_token(arena: &AstArena, token: u32) -> Option<u32> {
    with3(arena, rgb2srgbvalues(arena, token))
}

pub fn hsl2cmyk_token(arena: &AstArena, token: u32) -> Option<u32> {
    Some(to_cmyk_token(arena, &hsl2srgbvalues(arena, token)?))
}

pub fn hwb2cmyk_token(arena: &AstArena, token: u32) -> Option<u32> {
    Some(to_cmyk_token(arena, &hwb2srgbvalues(arena, token)?))
}

pub fn lab2cmyk_token(arena: &AstArena, token: u32) -> Option<u32> {
    with3(arena, lab2srgbvalues(arena, token))
}

pub fn lch2cmyk_token(arena: &AstArena, token: u32) -> Option<u32> {
    with3(arena, lch2srgbvalues(arena, token))
}

pub fn oklab2cmyk(arena: &AstArena, token: u32) -> Option<u32> {
    with3(arena, oklab2srgbvalues(arena, token))
}

pub fn oklch2cmyk_token(arena: &AstArena, token: u32) -> Option<u32> {
    with3(arena, oklch2srgbvalues(arena, token))
}

pub fn color2cmyk_token(arena: &AstArena, token: u32) -> Option<u32> {
    Some(to_cmyk_token(arena, &color2srgbvalues(arena, token)?))
}

pub fn srgb2cmykvalues(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    let k = 1.0 - js_max(&[r, g, b]);
    let div = 1.0 - k;
    let c = if k == 1.0 { 0.0 } else { (1.0 - r - k) / div };
    let m = if k == 1.0 { 0.0 } else { (1.0 - g - k) / div };
    let y = if k == 1.0 { 0.0 } else { (1.0 - b - k) / div };
    let mut result = vec![c, m, y, k];
    if let Some(a) = a.filter(|&a| a < 1.0) {
        result.push(a);
    }
    result
}

fn cmyktoken(arena: &AstArena, values: &[f64]) -> u32 {
    let mut chi = Vec::new();
    for (index, v) in values.iter().enumerate() {
        if index == 4 {
            chi.push(slash(arena));
        }
        chi.push(percentage(arena, v * 100.0));
    }
    arena.create_color("device-cmyk", ColorType::DEVICE_CMYK, Some(&chi))
}
