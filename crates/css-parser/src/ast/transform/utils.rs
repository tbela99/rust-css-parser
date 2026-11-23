//! Port of `src/ast/transform/utils.ts`: matrix operations of the transform functions.

use super::{DecomposedMatrix3D, Matrix};
use crate::syntax::constants::epsilon;
use crate::utils::number::{js_round, js_to_precision};

/// the identity matrix
pub const IDENTITY_MATRIX: Matrix = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

pub fn identity() -> Matrix {
    IDENTITY_MATRIX
}

pub fn set_identity(matrix: &mut Matrix) -> &mut Matrix {
    *matrix = IDENTITY_MATRIX;
    matrix
}

/// true if the values are equal (`===`) to the values of the identity matrix
pub fn is_identity(matrix: &Matrix) -> bool {
    matrix.iter().zip(IDENTITY_MATRIX.iter()).all(|(a, b)| a == b)
}

pub fn multiply(a: &Matrix, b: &Matrix) -> Matrix {
    let mut result = [0.0; 16];
    multiply_into(a, b, &mut result);
    result
}

/// `result = a × b` (the operands are read before the result is written)
pub fn multiply_into(a: &Matrix, b: &Matrix, result: &mut Matrix) {
    let a = *a;
    let b = *b;
    for column in (0..16).step_by(4) {
        let (b0, b1, b2, b3) = (b[column], b[column + 1], b[column + 2], b[column + 3]);
        result[column] = a[0] * b0 + a[4] * b1 + a[8] * b2 + a[12] * b3;
        result[column + 1] = a[1] * b0 + a[5] * b1 + a[9] * b2 + a[13] * b3;
        result[column + 2] = a[2] * b0 + a[6] * b1 + a[10] * b2 + a[14] * b3;
        result[column + 3] = a[3] * b0 + a[7] * b1 + a[11] * b2 + a[15] * b3;
    }
}

/// `matrix = matrix × b`
pub fn multiply_in_place(matrix: &mut Matrix, b: &Matrix) {
    let a = *matrix;
    multiply_into(&a, b, matrix);
}

/// inverse of a matrix, used to extract the perspective (the typescript indexing is kept: the augmented rows are
/// written with a stride of 8 and read with a stride of 4)
fn inverse(matrix: &Matrix) -> Option<[f64; 32]> {
    let mut augmented = [0.0f64; 32];
    for row in 0..4 {
        for j in 0..4 {
            augmented[row * 8 + j] = matrix[row * 4 + j];
            augmented[row * 8 + 4 + j] = if row == j { 1.0 } else { 0.0 };
        }
    }
    for col in 0..4 {
        let mut max_row = col;
        let mut max_val = augmented[col * 4 + col].abs();
        for row in col + 1..4 {
            let val = augmented[row * 4 + col].abs();
            if val > max_val {
                max_val = val;
                max_row = row;
            }
        }
        if max_val < epsilon {
            return None;
        }
        if max_row != col {
            augmented.swap(col, max_row);
        }
        let pivot = augmented[col * 4 + col];
        for j in 0..8 {
            augmented[col * 4 + j] /= pivot;
        }
        for row in 0..4 {
            if row != col {
                let factor = augmented[row * 4 + col];
                for j in 0..8 {
                    augmented[row * 4 + j] -= factor * augmented[col * 4 + j];
                }
            }
        }
    }
    Some(augmented)
}

/// `+value.toPrecision(6)`
pub fn to_precision6(value: f64) -> f64 {
    js_to_precision(value, 6)
}

/// round a number to 6 significant digits, or to the nearest integer when it is closer than `epsilon`
pub fn round(number: f64) -> f64 {
    let rounded = js_round(number);
    if (rounded - number).abs() <= epsilon {
        rounded
    } else {
        to_precision6(number)
    }
}

/// `Math.hypot()` (v8 algorithm: scaled, compensated sum)
pub fn js_hypot(values: &[f64]) -> f64 {
    let mut max = 0.0f64;
    let mut nan = false;
    for &v in values {
        let abs = v.abs();
        if abs.is_nan() {
            nan = true;
        } else if abs > max {
            max = abs;
        }
    }
    if max == f64::INFINITY {
        return f64::INFINITY;
    }
    if nan {
        return f64::NAN;
    }
    if max == 0.0 {
        return 0.0;
    }
    let mut sum = 0.0f64;
    let mut compensation = 0.0f64;
    for &v in values {
        let n = v.abs() / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

/// decompose a matrix: translate → rotate → skew → scale. false if the matrix cannot be decomposed
pub fn decompose(original: &Matrix, result: &mut DecomposedMatrix3D) -> bool {
    if original[15] == 0.0 {
        return false;
    }
    let mut matrix = *original;
    // `matrix[i] /= matrix[15]`: the last value is divided last
    for i in 0..16 {
        matrix[i] /= matrix[15];
    }
    result.perspective_x = 0.0;
    result.perspective_y = 0.0;
    result.perspective_z = 0.0;
    result.perspective_w = 1.0;
    if matrix[3] != 0.0 || matrix[7] != 0.0 || matrix[11] != 0.0 {
        let (r0, r1, r2, r3) = (matrix[3], matrix[7], matrix[11], matrix[15]);
        let Some(inverted) = inverse(original) else {
            return false;
        };
        result.perspective_x = r0 * inverted[0] + r1 * inverted[4] + r2 * inverted[8] + r3 * inverted[12];
        result.perspective_y = r0 * inverted[1] + r1 * inverted[5] + r2 * inverted[9] + r3 * inverted[13];
        result.perspective_z = r0 * inverted[2] + r1 * inverted[6] + r2 * inverted[10] + r3 * inverted[14];
        result.perspective_w = r0 * inverted[3] + r1 * inverted[7] + r2 * inverted[11] + r3 * inverted[15];
        matrix[3] = 0.0;
        matrix[7] = 0.0;
        matrix[11] = 0.0;
        matrix[15] = 1.0;
    }
    result.translate_x = matrix[12];
    result.translate_y = matrix[13];
    result.translate_z = matrix[14];
    let (row00, row01, row02) = (matrix[0], matrix[1], matrix[2]);
    let (row10, row11, row12) = (matrix[4], matrix[5], matrix[6]);
    let (row20, row21, row22) = (matrix[8], matrix[9], matrix[10]);
    let cross0 = row11 * row22 - row12 * row21;
    let cross1 = row12 * row20 - row10 * row22;
    let cross2 = row10 * row21 - row11 * row20;
    let scale_x = js_hypot(&[row00, row01, row02]);
    let norm = (row00 * row00 + row01 * row01 + row02 * row02).sqrt();
    let n = |v: f64, norm: f64| if norm == 0.0 { 0.0 } else { v / norm };
    let (row0_norm0, row0_norm1, row0_norm2) = (n(row00, norm), n(row01, norm), n(row02, norm));
    let skew_xy = row0_norm0 * row10 + row0_norm1 * row11 + row0_norm2 * row12;
    let row1_proj0 = row10 - skew_xy * row0_norm0;
    let row1_proj1 = row11 - skew_xy * row0_norm1;
    let row1_proj2 = row12 - skew_xy * row0_norm2;
    let scale_y = js_hypot(&[row1_proj0, row1_proj1, row1_proj2]);
    let norm = (row1_proj0 * row1_proj0 + row1_proj1 * row1_proj1 + row1_proj2 * row1_proj2).sqrt();
    let (row1_norm0, row1_norm1, row1_norm2) = (n(row1_proj0, norm), n(row1_proj1, norm), n(row1_proj2, norm));
    let skew_xz = row0_norm0 * row20 + row0_norm1 * row21 + row0_norm2 * row22;
    let skew_yz = row1_norm0 * row20 + row1_norm1 * row21 + row1_norm2 * row22;
    let row2_proj0 = row20 - skew_xz * row0_norm0 - skew_yz * row1_norm0;
    let row2_proj1 = row21 - skew_xz * row0_norm1 - skew_yz * row1_norm1;
    let row2_proj2 = row22 - skew_xz * row0_norm2 - skew_yz * row1_norm2;
    let norm = (row2_proj0 * row2_proj0 + row2_proj1 * row2_proj1 + row2_proj2 * row2_proj2).sqrt();
    let (row2_norm0, row2_norm1, row2_norm2) = (n(row2_proj0, norm), n(row2_proj1, norm), n(row2_proj2, norm));
    let determinant = row00 * cross0 + row01 * cross1 + row02 * cross2;
    let scale_z = js_hypot(&[row2_proj0, row2_proj1, row2_proj2]) * if determinant < 0.0 { -1.0 } else { 1.0 };
    let (r00, r01, r02) = (row0_norm0, row1_norm0, row2_norm0);
    let (r10, r11, r12) = (row0_norm1, row1_norm1, row2_norm1);
    let (r20, r21, r22) = (row0_norm2, row1_norm2, row2_norm2);
    let trace = r00 + r11 + r22;
    let (qw, mut qx, mut qy, mut qz);
    if trace > 0.0 {
        let s = 0.5 / (trace + 1.0).sqrt();
        qw = 0.25 / s;
        qx = (r21 - r12) * s;
        qy = (r02 - r20) * s;
        qz = (r10 - r01) * s;
    } else if r00 > r11 && r00 > r22 {
        let s = 2.0 * (1.0 + r00 - r11 - r22).sqrt();
        qw = (r21 - r12) / s;
        qx = 0.25 * s;
        qy = (r01 + r10) / s;
        qz = (r02 + r20) / s;
    } else if r11 > r22 {
        let s = 2.0 * (1.0 + r11 - r00 - r22).sqrt();
        qw = (r02 - r20) / s;
        qx = (r01 + r10) / s;
        qy = 0.25 * s;
        qz = (r12 + r21) / s;
    } else {
        let s = 2.0 * (1.0 + r22 - r00 - r11).sqrt();
        qw = (r10 - r01) / s;
        qx = (r02 + r20) / s;
        qy = (r12 + r21) / s;
        qz = 0.25 * s;
    }
    qx = to_zero_value(qx);
    qy = to_zero_value(qy);
    qz = to_zero_value(qz);
    // smallest non zero absolute value of the axis components
    let mut q = 0.0f64;
    for abs in [qx.abs(), qy.abs(), qz.abs()] {
        if q == 0.0 || (abs > 0.0 && abs < q) {
            q = abs;
        }
    }
    if q > 0.0 {
        qx /= q;
        qy /= q;
        qz /= q;
    }
    result.rotate_x = qx;
    result.rotate_y = qy;
    result.rotate_z = qz;
    // `Object.is(qw, +0)`
    result.rotate_angle = if qw == 0.0 && qw.is_sign_positive() { 0.0 } else { 2.0 * qw.acos() * 180.0 / std::f64::consts::PI };
    result.scale_x = scale_x;
    result.scale_y = scale_y;
    result.scale_z = scale_z;
    result.skew_xy = skew_xy;
    result.skew_xz = skew_xz;
    result.skew_yz = skew_yz;
    true
}

/// a value close to 0 (`epsilon`) is set to 0, the other values are rounded to 6 significant digits
pub fn to_zero_value(value: f64) -> f64 {
    if value.abs() <= epsilon {
        return 0.0;
    }
    if value.fract() == 0.0 && value < 1e6 && value > -1e6 {
        return value;
    }
    to_precision6(value)
}

pub fn to_zero(matrix: &mut Matrix) -> &mut Matrix {
    for v in matrix.iter_mut() {
        *v = to_zero_value(*v);
    }
    matrix
}

/// <https://drafts.csswg.org/css-transforms-1/#2d-matrix>
pub fn is_2d_matrix(m: &Matrix) -> bool {
    m[2] == 0.0 && m[3] == 0.0 && m[6] == 0.0 && m[7] == 0.0 && m[8] == 0.0 && m[9] == 0.0 && m[11] == 0.0 && m[14] == 0.0 && m[10] == 1.0 && m[15] == 1.0
}
