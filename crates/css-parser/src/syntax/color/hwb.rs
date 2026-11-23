//! Port of `src/syntax/color/hwb.ts`.

use super::color::{color2srgbvalues, get_angle, get_number};
use super::srgb::{cmyk2srgbvalues, hslvalues, lab2srgbvalues, lch2srgbvalues, oklab2srgbvalues, oklch2srgbvalues};
use super::tokens::{number, percentage, slash};
use super::utils::components::get_color_components;
use super::utils::values::{at, js_max, js_min, opt};
use crate::ast::arena::AstArena;
use crate::ast::types::ColorType;

macro_rules! to_token {
    ($name:ident, $values:ident) => {
        pub fn $name(arena: &AstArena, token: u32) -> Option<u32> {
            let values = $values(arena, token)?;
            Some(hwb_token(arena, &values))
        }
    };
}

to_token!(rgb2hwb_token, rgb2hwbvalues_opt);
to_token!(hsl2hwb_token, hsl2hwbvalues);
to_token!(cmyk2hwb_token, cmyk2hwbvalues);
to_token!(oklab2hwb_token, oklab2hwbvalues);
to_token!(oklch2hwb_token, oklch2hwbvalues);
to_token!(lab2hwb_token, lab2hwbvalues);
to_token!(lch2hwb_token, lch2hwbvalues);
to_token!(color2hwb_token, color2hwbvalues);

pub fn hwb_token(arena: &AstArena, values: &[f64]) -> u32 {
    let mut chi = vec![number(arena, at(values, 0)), percentage(arena, at(values, 1)), percentage(arena, at(values, 2))];
    if values.len() == 4 {
        chi.push(slash(arena));
        chi.push(percentage(arena, values[3] * 100.0));
    }
    arena.create_color("hwb", ColorType::HWB, Some(&chi))
}

fn to_hwb(values: &[f64]) -> Vec<f64> {
    srgb2hwb(at(values, 0), at(values, 1), at(values, 2), opt(values, 3))
}

/// typescript: `getColorComponents(...)!` (a `null` result throws)
pub fn rgb2hwbvalues(arena: &AstArena, token: u32) -> Vec<f64> {
    let values: Vec<f64> = get_color_components(arena, token)
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(index, &t)| if index == 3 { get_number(arena, t) } else { get_number(arena, t) / 255.0 })
        .collect();
    to_hwb(&values)
}

fn rgb2hwbvalues_opt(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(rgb2hwbvalues(arena, token))
}

pub fn cmyk2hwbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_hwb(&cmyk2srgbvalues(arena, token)?))
}

pub fn hsl2hwbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let v = hslvalues(arena, token)?;
    Some(hslvalues2hwbvalues(v.h, v.s, v.l, v.a))
}

pub fn lab2hwbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_hwb(&lab2srgbvalues(arena, token)?))
}

pub fn lch2hwbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_hwb(&lch2srgbvalues(arena, token)?))
}

pub fn oklab2hwbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_hwb(&oklab2srgbvalues(arena, token)?))
}

pub fn oklch2hwbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_hwb(&oklch2srgbvalues(arena, token)?))
}

pub fn color2hwbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_hwb(&color2srgbvalues(arena, token)?))
}

/// h, w, b, a values of a hwb color
pub struct HwbValues {
    pub h: f64,
    pub w: f64,
    pub b: f64,
    pub a: Option<f64>,
}

pub fn hwbvalues(arena: &AstArena, token: u32) -> Option<HwbValues> {
    let components = get_color_components(arena, token)?;
    let n = |i: usize| components.get(i).map_or(f64::NAN, |&t| get_number(arena, t));
    let h = components.first().map_or(f64::NAN, |&t| get_angle(arena, t)) * 360.0;
    let w = n(1) * 100.0;
    let b = n(2) * 100.0;
    let a = if components.len() == 4 { Some(n(3)) } else { None };
    Some(HwbValues { h, w, b, a })
}

/// `Math.min(1, Math.max(0, v))`
fn clamp01(v: f64) -> f64 {
    js_min(&[1.0, js_max(&[0.0, v])])
}

pub fn srgb2hwb(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    let r_clamp = clamp01(r);
    let g_clamp = clamp01(g);
    let b_clamp = clamp01(b);
    let c_max = js_max(&[r_clamp, g_clamp, b_clamp]);
    let c_min = js_min(&[r_clamp, g_clamp, b_clamp]);
    let delta = c_max - c_min;
    let w = c_min;
    let bl = 1.0 - c_max;
    let mut h = 0.0;
    if delta != 0.0 {
        if c_max == r_clamp {
            h = 60.0 * (((g_clamp - b_clamp) / delta) % 6.0);
        } else if c_max == g_clamp {
            h = 60.0 * ((b_clamp - r_clamp) / delta + 2.0);
        } else if c_max == b_clamp {
            h = 60.0 * ((r_clamp - g_clamp) / delta + 4.0);
        }
        if h < 0.0 {
            h += 360.0;
        }
    }
    let mut result = vec![h, w * 100.0, bl * 100.0];
    if let Some(a) = a {
        result.push(a);
    }
    result
}

pub fn hslvalues2hwbvalues(h: f64, s: f64, l: f64, a: Option<f64>) -> Vec<f64> {
    let s_norm = s / 100.0;
    let l_norm = l / 100.0;
    let hue = ((h % 360.0) + 360.0) % 360.0;
    let v = l_norm + s_norm * js_min(&[l_norm, 1.0 - l_norm]);
    let chroma = 2.0 * (v - l_norm);
    let w = v - chroma;
    let b = 1.0 - v;
    let mut hwb = vec![hue, w * 100.0, b * 100.0];
    if let Some(a) = a.filter(|&a| a != 1.0) {
        hwb.push(a);
    }
    hwb
}
