//! Port of `src/ast/transform/translate.ts`: `from = from × translation`.

use super::utils::{multiply_in_place, IDENTITY_MATRIX};
use super::Matrix;

fn apply(from: &mut Matrix, set: impl FnOnce(&mut Matrix)) {
    let mut matrix = IDENTITY_MATRIX;
    set(&mut matrix);
    multiply_in_place(from, &matrix);
}

pub fn translate_x(x: f64, from: &mut Matrix) {
    apply(from, |m| m[12] = x);
}

pub fn translate_y(y: f64, from: &mut Matrix) {
    apply(from, |m| m[13] = y);
}

pub fn translate_z(z: f64, from: &mut Matrix) {
    apply(from, |m| m[14] = z);
}

/// `y`: 0 if undefined
pub fn translate(x: f64, y: Option<f64>, from: &mut Matrix) {
    apply(from, |m| {
        m[12] = x;
        m[13] = y.unwrap_or(0.0);
    });
}

/// `y`, `z`: NaN if undefined
pub fn translate3d(x: f64, y: Option<f64>, z: Option<f64>, from: &mut Matrix) {
    apply(from, |m| {
        m[12] = x;
        m[13] = y.unwrap_or(f64::NAN);
        m[14] = z.unwrap_or(f64::NAN);
    });
}
