//! Port of `src/ast/transform/rotate.ts`: `from = from × rotation`, angles in radian.

use super::utils::{multiply_in_place, IDENTITY_MATRIX};
use super::Matrix;

pub fn rotate3d(angle: f64, x: f64, y: f64, z: f64, from: &mut Matrix) {
    let mut matrix = IDENTITY_MATRIX;
    let sc = (angle / 2.0).sin() * (angle / 2.0).cos();
    let sq = (angle / 2.0).sin() * (angle / 2.0).sin();
    let norm = (x * x + y * y + z * z).sqrt();
    let unit = if norm == 0.0 { 0.0 } else { 1.0 / norm };
    let (x, y, z) = (x * unit, y * unit, z * unit);
    matrix[0] = 1.0 - 2.0 * (y * y + z * z) * sq;
    matrix[1] = 2.0 * (x * y * sq + z * sc);
    matrix[2] = 2.0 * (x * z * sq - y * sc);
    matrix[4] = 2.0 * (x * y * sq - z * sc);
    matrix[5] = 1.0 - 2.0 * (x * x + z * z) * sq;
    matrix[6] = 2.0 * (y * z * sq + x * sc);
    matrix[8] = 2.0 * (x * z * sq + y * sc);
    matrix[9] = 2.0 * (y * z * sq - x * sc);
    matrix[10] = 1.0 - 2.0 * (x * x + y * y) * sq;
    multiply_in_place(from, &matrix);
}

pub fn rotate(angle: f64, from: &mut Matrix) {
    let mut matrix = IDENTITY_MATRIX;
    matrix[0] = angle.cos();
    matrix[1] = angle.sin();
    matrix[4] = -angle.sin();
    matrix[5] = angle.cos();
    multiply_in_place(from, &matrix);
}

pub use rotate as rotate_z;
