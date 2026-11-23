//! Port of `src/syntax/color/p3.ts`.

use super::srgb::{lsrgb2srgbvalues, srgb2lsrgbvalues, xyz2srgb};
use super::utils::matrix::multiply_matrices;
use super::xyz::srgb2xyz;

pub fn p32srgbvalues(r: f64, g: f64, b: f64, alpha: Option<f64>) -> Vec<f64> {
    let values = p32lp3(r, g, b, None);
    let values = lp32xyz(values[0], values[1], values[2], None);
    xyz2srgb(values[0], values[1], values[2], alpha)
}

pub fn srgb2p3values(r: f64, g: f64, b: f64, alpha: Option<f64>) -> Vec<f64> {
    let values = srgb2xyz(r, g, b, None);
    let values = xyz2lp3(values[0], values[1], values[2], None);
    let mut values = lp32p3(values[0], values[1], values[2], None);
    if let Some(alpha) = alpha.filter(|&a| a < 1.0) {
        values.push(alpha);
    }
    values
}

pub fn srgb2lp3values(r: f64, g: f64, b: f64, alpha: Option<f64>) -> Vec<f64> {
    let values = srgb2xyz(r, g, b, None);
    let mut values = xyz2lp3(values[0], values[1], values[2], None);
    if let Some(alpha) = alpha.filter(|&a| a < 1.0) {
        values.push(alpha);
    }
    values
}

pub fn lp32srgbvalues(r: f64, g: f64, b: f64, alpha: Option<f64>) -> Vec<f64> {
    let values = lp32xyz(r, g, b, None);
    xyz2srgb(values[0], values[1], values[2], alpha)
}

/// same as sRGB
pub fn p32lp3(r: f64, g: f64, b: f64, alpha: Option<f64>) -> Vec<f64> {
    srgb2lsrgbvalues(r, g, b, alpha)
}

/// same as sRGB
pub fn lp32p3(r: f64, g: f64, b: f64, alpha: Option<f64>) -> Vec<f64> {
    lsrgb2srgbvalues(r, g, b, alpha)
}

pub fn lp32xyz(r: f64, g: f64, b: f64, alpha: Option<f64>) -> Vec<f64> {
    const M: [[f64; 3]; 3] = [
        [608311.0 / 1250200.0, 189793.0 / 714400.0, 198249.0 / 1000160.0],
        [35783.0 / 156275.0, 247089.0 / 357200.0, 198249.0 / 2500400.0],
        [0.0, 32229.0 / 714400.0, 5220557.0 / 5000800.0],
    ];
    let mut result = multiply_matrices(&M, [r, g, b]);
    if let Some(alpha) = alpha.filter(|&a| a != 1.0) {
        result.push(alpha);
    }
    result
}

pub fn xyz2lp3(x: f64, y: f64, z: f64, alpha: Option<f64>) -> Vec<f64> {
    const M: [[f64; 3]; 3] = [
        [446124.0 / 178915.0, -333277.0 / 357830.0, -72051.0 / 178915.0],
        [-14852.0 / 17905.0, 63121.0 / 35810.0, 423.0 / 17905.0],
        [11844.0 / 330415.0, -50337.0 / 660830.0, 316169.0 / 330415.0],
    ];
    let mut result = multiply_matrices(&M, [x, y, z]);
    if let Some(alpha) = alpha.filter(|&a| a != 1.0) {
        result.push(alpha);
    }
    result
}
