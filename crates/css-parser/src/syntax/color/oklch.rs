//! Port of `src/syntax/color/oklch.ts`.

use super::color::{color2srgbvalues, get_angle, get_number};
use super::lch::labvalues2lchvalues;
use super::oklab::{get_oklab_components, hex2oklabvalues, hsl2oklabvalues, hwb2oklabvalues, lab2oklabvalues, lch2oklabvalues, rgb2oklabvalues, srgb2oklab};
use super::srgb::cmyk2srgbvalues;
use super::tokens::numbers_with_alpha;
use super::utils::components::get_color_components;
use super::utils::values::{at, opt};
use crate::ast::arena::AstArena;
use crate::ast::types::{ColorType, EnumToken};

pub use super::srgb::oklch2srgbvalues as oklch2srgbvalues_opt;

macro_rules! to_token {
    ($name:ident, $values:ident) => {
        pub fn $name(arena: &AstArena, token: u32) -> Option<u32> {
            let values = $values(arena, token)?;
            Some(oklch_token(arena, &values))
        }
    };
}

to_token!(hex2oklch_token, hex2oklchvalues);
to_token!(rgb2oklch_token, rgb2oklchvalues);
to_token!(hsl2oklch_token, hsl2oklchvalues);
to_token!(hwb2oklch_token, hwb2oklchvalues);
to_token!(cmyk2oklch_token, cmyk2oklchvalues);
to_token!(lab2oklch_token, lab2oklchvalues);
to_token!(oklab2oklch_token, oklab2oklchvalues);
to_token!(lch2oklch_token, lch2oklchvalues);

pub fn color2oklch_token(arena: &AstArena, token: u32) -> Option<u32> {
    let values = color2srgbvalues(arena, token)?;
    Some(oklch_token(arena, &srgb2oklch(at(&values, 0), at(&values, 1), at(&values, 2), opt(&values, 3))))
}

fn oklch_token(arena: &AstArena, values: &[f64]) -> u32 {
    let chi = numbers_with_alpha(arena, values, values.len() == 4);
    arena.create_color("oklch", ColorType::OKLCH, Some(&chi))
}

fn lab2lch(values: &[f64]) -> Vec<f64> {
    labvalues2lchvalues(at(values, 0), at(values, 1), at(values, 2), opt(values, 3))
}

pub fn hex2oklchvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(lab2lch(&hex2oklabvalues(arena, token)?))
}

pub fn rgb2oklchvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(lab2lch(&rgb2oklabvalues(arena, token)?))
}

pub fn hsl2oklchvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(lab2lch(&hsl2oklabvalues(arena, token)?))
}

/// typescript: `hwb2oklabvalues(...) as number[]` (a `null` result throws)
pub fn hwb2oklchvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(lab2lch(&hwb2oklabvalues(arena, token)?))
}

pub fn cmyk2oklchvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let values = cmyk2srgbvalues(arena, token)?;
    Some(srgb2oklch(at(&values, 0), at(&values, 1), at(&values, 2), opt(&values, 3)))
}

pub fn lab2oklchvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(lab2lch(&lab2oklabvalues(arena, token)?))
}

pub fn lch2oklchvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(lab2lch(&lch2oklabvalues(arena, token)?))
}

pub fn oklab2oklchvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    Some(lab2lch(&get_oklab_components(arena, token)?))
}

pub fn srgb2oklch(r: f64, g: f64, blue: f64, alpha: Option<f64>) -> Vec<f64> {
    lab2lch(&srgb2oklab(r, g, blue, alpha))
}

/// components of an oklch color: an empty array when a component is not a number, a percentage, an angle or an
/// identifier
pub fn get_oklch_components(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let components = get_color_components(arena, token)?;
    for &c in &components {
        if !matches!(
            arena.data_types.get(c),
            EnumToken::NumberTokenType | EnumToken::PercentageTokenType | EnumToken::AngleTokenType | EnumToken::IdenTokenType
        ) {
            return Some(Vec::new());
        }
    }
    let n = |i: usize| components.get(i).map_or(f64::NAN, |&t| get_number(arena, t));
    let l = n(0);
    let c = components.get(1).map_or(f64::NAN, |&t| {
        get_number(arena, t) * if arena.data_types.get(t) == EnumToken::PercentageTokenType { 0.4 } else { 1.0 }
    });
    let h = components.get(2).map_or(f64::NAN, |&t| get_angle(arena, t)) * 360.0;
    let alpha = match components.get(3) {
        None => 1.0,
        Some(&t) if arena.data_types.get(t) == EnumToken::IdenTokenType && &*arena.resolve_string(arena.string_value.get(t)) == "none" => 1.0,
        Some(&t) => get_number(arena, t),
    };
    Some(vec![l, c, h, alpha])
}
