//! Port of `src/syntax/color/utils/distance.ts`.

use super::values::{at, opt};
use crate::ast::arena::AstArena;
use crate::ast::types::ColorType;
use crate::syntax::color::color::convert_color;
use crate::syntax::color::oklab::get_oklab_components;
use crate::syntax::constants::{colorDistancePrecision, colorPrecision};
use crate::syntax::syntax::to_precision_value;

/// distance between two colors in the oklab space (<https://drafts.csswg.org/css-color-4/#comparing-color-values>)
pub fn ok_lab_distance(arena: &AstArena, color1: u32, color2: u32) -> Option<f64> {
    let c1 = convert_color(arena, color1, ColorType::OKLAB)?;
    let c2 = convert_color(arena, color2, ColorType::OKLAB)?;
    let ok_lab1 = get_oklab_components(arena, c1)?;
    let ok_lab2 = get_oklab_components(arena, c2)?;
    let mut diff = vec![at(&ok_lab1, 0) - at(&ok_lab2, 0), at(&ok_lab1, 1) - at(&ok_lab2, 1), at(&ok_lab1, 2) - at(&ok_lab2, 2)];
    // include alpha
    if opt(&ok_lab1, 3).is_some() || opt(&ok_lab2, 3).is_some() {
        diff.push(opt(&ok_lab1, 3).unwrap_or(1.0) - opt(&ok_lab2, 3).unwrap_or(1.0));
    }
    let hypot = (diff[0] * diff[0] + diff[1] * diff[1] + diff[2] * diff[2] + opt(&diff, 3).unwrap_or(0.0).powi(2)).sqrt();
    Some(to_precision_value(hypot, colorPrecision))
}

/// true if two colors are close in the oklab space
pub fn is_ok_lab_close(arena: &AstArena, color1: u32, color2: u32) -> bool {
    is_ok_lab_close_with(arena, color1, color2, colorDistancePrecision)
}

/// [`is_ok_lab_close`] with a threshold
pub fn is_ok_lab_close_with(arena: &AstArena, color1: u32, color2: u32, threshold: f64) -> bool {
    let Some(c1) = convert_color(arena, color1, ColorType::OKLAB) else {
        return false;
    };
    let Some(c2) = convert_color(arena, color2, ColorType::OKLAB) else {
        return false;
    };
    let (Some(ok_lab1), Some(ok_lab2)) = (get_oklab_components(arena, c1), get_oklab_components(arena, c2)) else {
        return false;
    };
    (0..3).all(|i| !((at(&ok_lab1, i) - at(&ok_lab2, i)).abs() > threshold))
}
