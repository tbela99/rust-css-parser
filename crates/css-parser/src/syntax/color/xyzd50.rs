//! Port of `src/syntax/color/xyzd50.ts`.

use super::lab::xyz2lab;
use super::lch::labvalues2lchvalues;
use super::srgb::lsrgb2srgbvalues;
use super::utils::matrix::multiply_matrices;
use super::xyz::xyz_d50_to_d65;

pub fn xyzd502lch(x: f64, y: f64, z: f64, alpha: Option<f64>) -> Vec<f64> {
    let values = xyz_d50_to_d65(x, y, z);
    let lab = xyz2lab(values[0], values[1], values[2], None);
    // L in range [0,100]. For use in CSS, add a percent
    labvalues2lchvalues(lab[0], lab[1], lab[2], alpha)
}

/// Bradford chromatic adaptation from D65 to D50 (see <https://github.com/LeaVerou/color.js/pull/354/files>)
pub fn xyz_d65_to_d50(x: f64, y: f64, z: f64, alpha: Option<f64>) -> Vec<f64> {
    const M: [[f64; 3]; 3] = [
        [1.0479297925449969, 0.022946870601609652, -0.05019226628920524],
        [0.02962780877005599, 0.9904344267538799, -0.017073799063418826],
        [-0.009243040646204504, 0.015055191490298152, 0.7518742814281371],
    ];
    let mut result = multiply_matrices(&M, [x, y, z]);
    if let Some(alpha) = alpha.filter(|&a| a != 1.0) {
        result.push(alpha);
    }
    result
}

pub fn xyzd502srgb(x: f64, y: f64, z: f64, alpha: Option<f64>) -> Vec<f64> {
    lsrgb2srgbvalues(
        x * 3.1341359569958707 - y * 1.6173863321612538 - 0.4906619460083532 * z,
        x * -0.978795502912089 + y * 1.916254567259524 + 0.03344273116131949 * z,
        x * 0.07195537988411677 - y * 0.2289768264158322 + 1.405386058324125 * z,
        alpha,
    )
}
