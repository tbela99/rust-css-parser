//! Port of `src/ast/transform/skew.ts`: `from = from × skew`, angles in radian.

use super::utils::{multiply_in_place, IDENTITY_MATRIX};
use super::Matrix;

pub fn skew_x(x: f64, from: &mut Matrix) {
    let mut matrix = IDENTITY_MATRIX;
    matrix[4] = x.tan();
    multiply_in_place(from, &matrix);
}

pub fn skew_y(y: f64, from: &mut Matrix) {
    let mut matrix = IDENTITY_MATRIX;
    matrix[1] = y.tan();
    multiply_in_place(from, &matrix);
}

/// `y`: ignored if undefined
pub fn skew(x: f64, y: Option<f64>, from: &mut Matrix) {
    let mut matrix = IDENTITY_MATRIX;
    matrix[4] = x.tan();
    if let Some(y) = y {
        matrix[1] = y.tan();
    }
    multiply_in_place(from, &matrix);
}
