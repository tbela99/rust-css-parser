//! Port of `src/syntax/color/lab.ts`.

use super::color::{color2srgbvalues, get_number};
use super::lch::get_lch_components;
use super::oklab::{get_oklab_components, oklab_to_xyz};
use super::oklch::oklch2srgbvalues_opt as oklch2srgbvalues;
use super::srgb::{cmyk2srgbvalues, hex2srgbvalues, hsl2srgb, hwb2srgbvalues, rgb2srgb, xyz2srgb};
use super::tokens::numbers_with_alpha;
use super::utils::components::get_color_components;
use super::utils::values::{at, opt};
use super::xyz::{srgb2xyz_d65, xyz_d50_to_d65};
use super::xyzd50::xyz_d65_to_d50;
use crate::ast::arena::AstArena;
use crate::ast::types::{ColorType, EnumToken};
use crate::syntax::constants::{e, k, D50};

macro_rules! to_token {
    ($name:ident, $values:ident) => {
        pub fn $name(arena: &AstArena, token: u32) -> Option<u32> {
            let values = $values(arena, token)?;
            Some(lab_token(arena, &values))
        }
    };
}

to_token!(hex2lab_token, hex2labvalues);
to_token!(rgb2lab_token, rgb2labvalues);
to_token!(hsl2lab_token, hsl2labvalues);
to_token!(hwb2lab_token, hwb2labvalues);
to_token!(cmyk2lab_token, cmyk2labvalues);
to_token!(lch2lab_token, lch2labvalues);
to_token!(oklab2lab_token, oklab2labvalues);
to_token!(oklch2lab_token, oklch2labvalues);
to_token!(color2lab_token, color2labvalues);

fn lab_token(arena: &AstArena, values: &[f64]) -> u32 {
    let chi = numbers_with_alpha(arena, values, values.len() == 4);
    arena.create_color("lab", ColorType::LAB, Some(&chi))
}

fn srgb2lab(values: &[f64]) -> Vec<f64> {
    srgb2labvalues(at(values, 0), at(values, 1), at(values, 2), opt(values, 3))
}

pub fn hex2labvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(srgb2lab(&hex2srgbvalues(arena, token)))
}

pub fn rgb2labvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(srgb2lab(&rgb2srgb(arena, token)?))
}

pub fn cmyk2labvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(srgb2lab(&cmyk2srgbvalues(arena, token)?))
}

pub fn hsl2labvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(srgb2lab(&hsl2srgb(arena, token)?))
}

pub fn hwb2labvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(srgb2lab(&hwb2srgbvalues(arena, token)?))
}

pub fn lch2labvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let values = get_lch_components(arena, token)?;
    Some(lchvalues2labvalues(at(&values, 0), at(&values, 1), at(&values, 2), opt(&values, 3)))
}

pub fn oklab2labvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let values = get_oklab_components(arena, token)?;
    let values = oklab_to_xyz(at(&values, 0), at(&values, 1), at(&values, 2), opt(&values, 3));
    let values = xyz_d65_to_d50(at(&values, 0), at(&values, 1), at(&values, 2), opt(&values, 3));
    Some(xyz2lab(at(&values, 0), at(&values, 1), at(&values, 2), opt(&values, 3)))
}

pub fn oklch2labvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(srgb2lab(&oklch2srgbvalues(arena, token)?))
}

pub fn color2labvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(srgb2lab(&color2srgbvalues(arena, token)?))
}

pub fn srgb2labvalues(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    let xyz = srgb2xyz_d65(r, g, b, None);
    let mut result = xyz2lab(xyz[0], xyz[1], xyz[2], None);
    if r == b && b == g {
        result[1] = 0.0;
        result[2] = 0.0;
    }
    if let Some(a) = a {
        result.push(a);
    }
    result
}

pub fn xyz2lab(x: f64, y: f64, z: f64, a: Option<f64>) -> Vec<f64> {
    // the xyz values are relative to D50
    let f: Vec<f64> = [x, y, z]
        .iter()
        .enumerate()
        .map(|(i, &value)| value / D50[i])
        .map(|value| if value > e { value.cbrt() } else { (k * value + 16.0) / 116.0 })
        .collect();
    let mut result = vec![116.0 * f[1] - 16.0, 500.0 * (f[0] - f[1]), 200.0 * (f[1] - f[2])];
    if let Some(a) = a.filter(|&a| a != 1.0) {
        result.push(a);
    }
    result
}

pub fn lchvalues2labvalues(l: f64, c: f64, h: f64, a: Option<f64>) -> Vec<f64> {
    let mut result = vec![l, c * (h * std::f64::consts::PI / 180.0).cos(), c * (h * std::f64::consts::PI / 180.0).sin()];
    if let Some(a) = a {
        result.push(a);
    }
    result
}

pub fn get_lab_components(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
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
    let a = scaled(1, 125.0);
    let b = scaled(2, 125.0);
    let alpha = components.get(3).map_or(1.0, |&t| get_number(arena, t));
    let mut result = vec![l, a, b];
    if alpha != 1.0 {
        result.push(alpha);
    }
    Some(result)
}

/// D50 LAB to sRGB
#[allow(non_snake_case)]
pub fn lab_to_srgb(l: f64, a: f64, b: f64) -> Vec<f64> {
    let xyz_d50 = lab_to_xyz(l, a, b);
    let xyz_d65 = xyz_d50_to_d65(xyz_d50[0], xyz_d50[1], xyz_d50[2]);
    xyz2srgb(xyz_d65[0], xyz_d65[1], xyz_d65[2], None)
}

/// Lab to D50-adapted XYZ
pub fn lab_to_xyz(l: f64, a: f64, b: f64) -> Vec<f64> {
    let k_: f64 = 24389.0 / 27.0; // 29^3/3^3
    let e_: f64 = 216.0 / 24389.0; // 6^3/29^3
    let f1 = (l + 16.0) / 116.0;
    let f0 = a / 500.0 + f1;
    let f2 = f1 - b / 200.0;
    let xyz = [
        if f0.powf(3.0) > e_ { f0.powf(3.0) } else { (116.0 * f0 - 16.0) / k_ },
        if l > k_ * e_ { ((l + 16.0) / 116.0).powf(3.0) } else { l / k_ },
        if f2.powf(3.0) > e_ { f2.powf(3.0) } else { (116.0 * f2 - 16.0) / k_ },
    ];
    xyz.iter().enumerate().map(|(i, value)| value * D50[i]).collect()
}
