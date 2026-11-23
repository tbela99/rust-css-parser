//! Port of `src/syntax/color/a98rgb.ts`.

use super::srgb::xyz2srgb;
use super::utils::matrix::multiply_matrices;
use super::xyz::srgb2xyz;

fn suffix(a: Option<f64>) -> Option<f64> {
    a.filter(|&a| a != 1.0)
}

pub fn a98rgb2srgbvalues(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    let values = a98rgb2la98(r, g, b, None);
    let values = la98rgb2xyz(values[0], values[1], values[2], None);
    let mut values = xyz2srgb(values[0], values[1], values[2], None);
    if let Some(a) = a.filter(|&a| a < 1.0) {
        values.push(a);
    }
    values
}

pub fn srgb2a98values(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    let values = srgb2xyz(r, g, b, None);
    let values = xyz2la98rgb(values[0], values[1], values[2], None);
    let mut values = la98rgb2a98rgb(values[0], values[1], values[2], None);
    if let Some(a) = a.filter(|&a| a < 1.0) {
        values.push(a);
    }
    values
}

fn gamma(r: f64, g: f64, b: f64, a: Option<f64>, exponent: f64) -> Vec<f64> {
    let mut values: Vec<f64> = [r, g, b]
        .iter()
        .map(|&val| {
            let sign = if val < 0.0 { -1.0 } else { 1.0 };
            sign * val.abs().powf(exponent)
        })
        .collect();
    values.extend(suffix(a));
    values
}

fn a98rgb2la98(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    gamma(r, g, b, a, 563.0 / 256.0)
}

fn la98rgb2a98rgb(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    gamma(r, g, b, a, 256.0 / 563.0)
}

fn la98rgb2xyz(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    const M: [[f64; 3]; 3] = [
        [573536.0 / 994567.0, 263643.0 / 1420810.0, 187206.0 / 994567.0],
        [591459.0 / 1989134.0, 6239551.0 / 9945670.0, 374412.0 / 4972835.0],
        [53769.0 / 1989134.0, 351524.0 / 4972835.0, 4929758.0 / 4972835.0],
    ];
    let mut values = multiply_matrices(&M, [r, g, b]);
    values.extend(suffix(a));
    values
}

fn xyz2la98rgb(x: f64, y: f64, z: f64, a: Option<f64>) -> Vec<f64> {
    const M: [[f64; 3]; 3] = [
        [1829569.0 / 896150.0, -506331.0 / 896150.0, -308931.0 / 896150.0],
        [-851781.0 / 878810.0, 1648619.0 / 878810.0, 36519.0 / 878810.0],
        [16779.0 / 1248040.0, -147721.0 / 1248040.0, 1266979.0 / 1248040.0],
    ];
    let mut values = multiply_matrices(&M, [x, y, z]);
    values.extend(suffix(a));
    values
}
