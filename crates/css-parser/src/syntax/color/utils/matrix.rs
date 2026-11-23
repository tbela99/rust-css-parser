//! Port of `src/syntax/color/utils/matrix.ts` (from <https://www.w3.org/TR/css-color-4/multiply-matrices.js>).

/// multiply a 3x3 matrix by a vector. a missing or `NaN` component of the vector counts as 0 (`col[i] || 0`)
pub fn multiply_matrices(a: &[[f64; 3]; 3], b: [f64; 3]) -> Vec<f64> {
    let b = b.map(|x| if x.is_nan() { 0.0 } else { x });
    a.iter().map(|row| row.iter().zip(b.iter()).fold(0.0, |acc, (c, v)| acc + c * v)).collect()
}
