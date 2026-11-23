//! Port of `src/ast/transform/scale.ts`: `from = from × scale`.

use super::utils::{multiply_in_place, IDENTITY_MATRIX};
use super::Matrix;

fn apply(from: &mut Matrix, x: f64, y: f64, z: f64) {
    let mut matrix = IDENTITY_MATRIX;
    matrix[0] = x;
    matrix[5] = y;
    matrix[10] = z;
    multiply_in_place(from, &matrix);
}

pub fn scale_x(x: f64, from: &mut Matrix) {
    apply(from, x, 1.0, 1.0);
}

pub fn scale_y(y: f64, from: &mut Matrix) {
    apply(from, 1.0, y, 1.0);
}

pub fn scale_z(z: f64, from: &mut Matrix) {
    apply(from, 1.0, 1.0, z);
}

pub fn scale(x: f64, y: f64, from: &mut Matrix) {
    apply(from, x, y, 1.0);
}

pub fn scale3d(x: f64, y: f64, z: f64, from: &mut Matrix) {
    apply(from, x, y, z);
}
