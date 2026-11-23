//! Port of `src/syntax/color/xyz.ts`.

use super::srgb::srgb2lsrgbvalues;
use super::utils::matrix::multiply_matrices;
use super::xyzd50::xyz_d65_to_d50;

#[allow(non_snake_case)]
pub fn xyz_to_lin_srgb(x: f64, y: f64, z: f64, alpha: Option<f64>) -> Vec<f64> {
    const M: [[f64; 3]; 3] = [
        [12831.0 / 3959.0, -329.0 / 214.0, -1974.0 / 3959.0],
        [-851781.0 / 878810.0, 1648619.0 / 878810.0, 36519.0 / 878810.0],
        [705.0 / 12673.0, -2585.0 / 12673.0, 705.0 / 667.0],
    ];
    let mut result = multiply_matrices(&M, [x, y, z]);
    if let Some(alpha) = alpha.filter(|&a| a != 1.0) {
        result.push(alpha);
    }
    result
}

pub fn xyz_d50_to_d65(x: f64, y: f64, z: f64) -> Vec<f64> {
    const M: [[f64; 3]; 3] = [
        [0.9554734527042182, -0.023098536874261423, 0.0632593086610217],
        [-0.028369706963208136, 1.0099954580058226, 0.021041398966943008],
        [0.012314001688319899, -0.020507696433477912, 1.3303659366080753],
    ];
    multiply_matrices(&M, [x, y, z])
}

pub fn srgb2xyz(r: f64, g: f64, b: f64, alpha: Option<f64>) -> Vec<f64> {
    let l = srgb2lsrgbvalues(r, g, b, None);
    let (r, g, b) = (l[0], l[1], l[2]);
    let mut rgb = vec![
        0.4123907992659595 * r + 0.35758433938387796 * g + 0.1804807884018343 * b,
        0.21263900587151036 * r + 0.7151686787677559 * g + 0.07219231536073371 * b,
        0.01933081871559185 * r + 0.11919477979462599 * g + 0.9505321522496606 * b,
    ];
    if let Some(alpha) = alpha.filter(|&a| a != 1.0) {
        rgb.push(alpha);
    }
    rgb
}

#[allow(non_snake_case)]
pub fn srgb2xyz_d65(r: f64, g: f64, b: f64, alpha: Option<f64>) -> Vec<f64> {
    let values = srgb2xyz(r, g, b, None);
    let mut rgb = xyz_d65_to_d50(values[0], values[1], values[2], None);
    if let Some(alpha) = alpha.filter(|&a| a != 1.0) {
        rgb.push(alpha);
    }
    rgb
}
