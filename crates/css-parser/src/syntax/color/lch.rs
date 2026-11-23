//! Port of `src/syntax/color/lch.ts`.

use super::color::{color2srgbvalues, get_angle, get_number};
use super::lab::{get_lab_components, hex2labvalues, hsl2labvalues, hwb2labvalues, oklab2labvalues, oklch2labvalues, rgb2labvalues, srgb2labvalues, xyz2lab};
use super::srgb::cmyk2srgbvalues;
use super::tokens::numbers_with_alpha;
use super::utils::components::get_color_components;
use super::utils::values::{at, opt};
use crate::ast::arena::AstArena;
use crate::ast::types::{ColorType, EnumToken};
use crate::syntax::constants::epsilon;

macro_rules! to_token {
    ($name:ident, $values:ident) => {
        pub fn $name(arena: &AstArena, token: u32) -> Option<u32> {
            let values = $values(arena, token)?;
            Some(lch_token(arena, &values))
        }
    };
}

to_token!(hex2lch_token, hex2lchvalues);
to_token!(rgb2lch_token, rgb2lchvalues);
to_token!(hsl2lch_token, hsl2lchvalues);
to_token!(hwb2lch_token, hwb2lchvalues);
to_token!(cmyk2lch_token, cmyk2lchvalues);
to_token!(lab2lch_token, lab2lchvalues);
to_token!(oklab2lch_token, oklab2lchvalues);
to_token!(oklch2lch_token, oklch2lchvalues);
to_token!(color2lch_token, color2lchvalues);

fn lch_token(arena: &AstArena, values: &[f64]) -> u32 {
    let chi = numbers_with_alpha(arena, values, values.len() == 4);
    arena.create_color("lch", ColorType::LCH, Some(&chi))
}

fn lab2lch(values: &[f64]) -> Vec<f64> {
    labvalues2lchvalues(at(values, 0), at(values, 1), at(values, 2), opt(values, 3))
}

pub fn hex2lchvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(lab2lch(&hex2labvalues(arena, token)?))
}

pub fn rgb2lchvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(lab2lch(&rgb2labvalues(arena, token)?))
}

pub fn hsl2lchvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(lab2lch(&hsl2labvalues(arena, token)?))
}

pub fn hwb2lchvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(lab2lch(&hwb2labvalues(arena, token)?))
}

pub fn lab2lchvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(lab2lch(&get_lab_components(arena, token)?))
}

pub fn srgb2lch(r: f64, g: f64, blue: f64, alpha: Option<f64>) -> Vec<f64> {
    lab2lch(&srgb2labvalues(r, g, blue, alpha))
}

pub fn oklab2lchvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(lab2lch(&oklab2labvalues(arena, token)?))
}

pub fn cmyk2lchvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let values = cmyk2srgbvalues(arena, token)?;
    Some(srgb2lch(at(&values, 0), at(&values, 1), at(&values, 2), opt(&values, 3)))
}

pub fn oklch2lchvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(lab2lch(&oklch2labvalues(arena, token)?))
}

pub fn color2lchvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let values = color2srgbvalues(arena, token)?;
    Some(srgb2lch(at(&values, 0), at(&values, 1), at(&values, 2), opt(&values, 3)))
}

pub fn labvalues2lchvalues(l: f64, a: f64, b: f64, alpha: Option<f64>) -> Vec<f64> {
    let c = (a * a + b * b).sqrt();
    let mut h = b.atan2(a) * 180.0 / std::f64::consts::PI;
    if h >= -epsilon && h <= epsilon {
        h = 0.0;
    } else if h < 0.0 {
        h += 360.0;
    }
    match alpha {
        None => vec![l, c, h],
        Some(alpha) => vec![l, c, h, alpha],
    }
}

pub fn xyz2lchvalues(x: f64, y: f64, z: f64, alpha: Option<f64>) -> Vec<f64> {
    let values = xyz2lab(x, y, z, None);
    let mut lch = labvalues2lchvalues(values[0], values[1], values[2], None);
    if let Some(alpha) = alpha.filter(|&a| a != 1.0) {
        lch.push(alpha);
    }
    lch
}

pub fn get_lch_components(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let components = get_color_components(arena, token)?;
    for &c in &components {
        if !matches!(
            arena.data_types.get(c),
            EnumToken::NumberTokenType | EnumToken::PercentageTokenType | EnumToken::AngleTokenType | EnumToken::IdenTokenType
        ) {
            return None;
        }
    }
    let scaled = |i: usize, factor: f64| {
        components.get(i).map_or(f64::NAN, |&t| {
            get_number(arena, t) * if arena.data_types.get(t) == EnumToken::PercentageTokenType { factor } else { 1.0 }
        })
    };
    let l = scaled(0, 100.0);
    let c = scaled(1, 150.0);
    let h = components.get(2).map_or(f64::NAN, |&t| get_angle(arena, t)) * 360.0;
    let alpha = components.get(3).map_or(1.0, |&t| get_number(arena, t));
    Some(vec![l, c, h, alpha])
}
