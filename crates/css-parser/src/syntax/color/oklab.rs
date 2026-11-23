//! Port of `src/syntax/color/oklab.ts`.

use super::color::{color2srgbvalues, get_number};
use super::lab::lchvalues2labvalues;
use super::oklch::get_oklch_components;
use super::srgb::{cmyk2srgbvalues, hex2srgbvalues, hsl2srgb, hwb2srgbvalues, lab2srgbvalues, lch2srgbvalues, lsrgb2srgbvalues, rgb2srgb, srgb2lsrgbvalues};
use super::tokens::{number, percentage, slash};
use super::utils::components::get_color_components;
use super::utils::matrix::multiply_matrices;
use super::utils::values::{at, opt};
use crate::ast::arena::AstArena;
use crate::ast::types::{ColorType, EnumToken};

macro_rules! to_token {
    ($name:ident, $values:ident) => {
        pub fn $name(arena: &AstArena, token: u32) -> Option<u32> {
            let values = $values(arena, token)?;
            Some(oklab_token(arena, &values))
        }
    };
}

to_token!(hex2oklab_token, hex2oklabvalues);
to_token!(rgb2oklab_token, rgb2oklabvalues);
to_token!(hsl2oklab_token, hsl2oklabvalues);
to_token!(hwb2oklab_token, hwb2oklabvalues);
to_token!(cmyk2oklab_token, cmyk2oklabvalues);
to_token!(lab2oklab_token, lab2oklabvalues);
to_token!(lch2oklab_token, lch2oklabvalues);
to_token!(oklch2oklab_token, oklch2oklabvalues);
to_token!(color2oklab_token, color2oklabvalues);

fn oklab_token(arena: &AstArena, values: &[f64]) -> u32 {
    let mut chi = vec![number(arena, at(values, 0)), number(arena, at(values, 1)), number(arena, at(values, 2))];
    if values.len() == 4 && values[3] != 1.0 {
        chi.push(slash(arena));
        chi.push(percentage(arena, values[3] * 100.0));
    }
    arena.create_color("oklab", ColorType::OKLAB, Some(&chi))
}

fn to_oklab(values: &[f64]) -> Vec<f64> {
    srgb2oklab(at(values, 0), at(values, 1), at(values, 2), opt(values, 3))
}

pub fn hex2oklabvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_oklab(&hex2srgbvalues(arena, token)))
}

pub fn rgb2oklabvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_oklab(&rgb2srgb(arena, token)?))
}

pub fn hsl2oklabvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_oklab(&hsl2srgb(arena, token)?))
}

pub fn hwb2oklabvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_oklab(&hwb2srgbvalues(arena, token)?))
}

pub fn cmyk2oklabvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_oklab(&cmyk2srgbvalues(arena, token)?))
}

pub fn lab2oklabvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_oklab(&lab2srgbvalues(arena, token)?))
}

pub fn lch2oklabvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_oklab(&lch2srgbvalues(arena, token)?))
}

pub fn oklch2oklabvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let values = get_oklch_components(arena, token)?;
    Some(lchvalues2labvalues(at(&values, 0), at(&values, 1), at(&values, 2), opt(&values, 3)))
}

fn color2oklabvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(to_oklab(&color2srgbvalues(arena, token)?))
}

pub fn srgb2oklab(r: f64, g: f64, blue: f64, alpha: Option<f64>) -> Vec<f64> {
    let lin = srgb2lsrgbvalues(r, g, blue, None);
    let (r, g, blue) = (lin[0], lin[1], lin[2]);
    let l_ = (0.41222147079999993 * r + 0.5363325363 * g + 0.0514459929 * blue).cbrt();
    let m_ = (0.2119034981999999 * r + 0.6806995450999999 * g + 0.1073969566 * blue).cbrt();
    let s_ = (0.08830246189999998 * r + 0.2817188376 * g + 0.6299787005000002 * blue).cbrt();
    let l = 0.2104542553 * l_ + 0.793617785 * m_ - 0.0040720468 * s_;
    let gray = r == g && g == blue;
    let a = if gray { 0.0 } else { 1.9779984951 * l_ - 2.428592205 * m_ + 0.4505937099 * s_ };
    let b = if gray { 0.0 } else { 0.0259040371 * l_ + 0.7827717662 * m_ - 0.808675766 * s_ };
    match alpha {
        Some(alpha) if alpha != 1.0 => vec![l, a, b, alpha],
        _ => vec![l, a, b],
    }
}

pub fn get_oklab_components(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let components = get_color_components(arena, token)?;
    if components.len() < 3 {
        return None;
    }
    let scaled = |i: usize, factor: f64| {
        let t = components[i];
        get_number(arena, t) * if arena.data_types.get(t) == EnumToken::PercentageTokenType { factor } else { 1.0 }
    };
    let l = get_number(arena, components[0]);
    let a = scaled(1, 0.4);
    let b = scaled(2, 0.4);
    let mut rgb = vec![l, a, b];
    if components.len() > 3 {
        let alpha = get_number(arena, components[3]);
        if alpha != 1.0 {
            rgb.push(alpha);
        }
    }
    Some(rgb)
}

#[allow(non_snake_case)]
pub fn oklab_to_xyz(l: f64, a: f64, b: f64, alpha: Option<f64>) -> Vec<f64> {
    const LMS_TO_XYZ: [[f64; 3]; 3] = [
        [1.2268798758459243, -0.5578149944602171, 0.2813910456659647],
        [-0.0405757452148008, 1.112286803280317, -0.0717110580655164],
        [-0.0763729366746601, -0.4214933324022432, 1.5869240198367816],
    ];
    const OKLAB_TO_LMS: [[f64; 3]; 3] = [
        [1.0, 0.3963377773761749, 0.2158037573099136],
        [1.0, -0.1055613458156586, -0.0638541728258133],
        [1.0, -0.0894841775298119, -1.2914855480194092],
    ];
    let lms = multiply_matrices(&OKLAB_TO_LMS, [l, a, b]);
    let mut xyz = multiply_matrices(&LMS_TO_XYZ, [lms[0].powf(3.0), lms[1].powf(3.0), lms[2].powf(3.0)]);
    if let Some(alpha) = alpha {
        xyz.push(alpha);
    }
    xyz
}

#[allow(non_snake_case)]
pub fn oklab_to_srgb(l: f64, a: f64, b: f64) -> Vec<f64> {
    let L = (l * 0.99999999845051981432 + 0.39633779217376785678 * a + 0.21580375806075880339 * b).powf(3.0);
    let M = (l * 1.0000000088817607767 - 0.1055613423236563494 * a - 0.063854174771705903402 * b).powf(3.0);
    let S = (l * 1.0000000546724109177 - 0.089484182094965759684 * a - 1.2914855378640917399 * b).powf(3.0);
    lsrgb2srgbvalues(
        4.076741661347994 * L - 3.307711590408193 * M + 0.230969928729428 * S,
        -1.2684380040921763 * L + 2.6097574006633715 * M - 0.3413193963102197 * S,
        -0.004196086541837188 * L - 0.7034186144594493 * M + 1.7076147009309444 * S,
        None,
    )
}
