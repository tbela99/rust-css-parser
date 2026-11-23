//! Port of `src/ast/transform/perspective.ts`: `from = from × perspective`.

use super::utils::{multiply_in_place, IDENTITY_MATRIX};
use super::Matrix;

/// `x`: distance, `None` for `none`
pub fn perspective(x: Option<f64>, from: &mut Matrix) {
    let mut matrix = IDENTITY_MATRIX;
    matrix[11] = match x {
        None => 0.0,
        Some(x) if x == 0.0 => f64::NEG_INFINITY,
        Some(x) => -1.0 / x,
    };
    multiply_in_place(from, &matrix);
}
