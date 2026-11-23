//! Port of `src/syntax/color/prophotorgb.ts`.

use super::utils::values::{js_sign, opt};
use super::xyz::srgb2xyz;
use super::xyzd50::{xyz_d65_to_d50, xyzd502srgb};

pub fn prophotorgb2srgbvalues(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    let values = prophotorgb2xyz50(r, g, b, None);
    xyzd502srgb(values[0], values[1], values[2], a)
}

pub fn srgb2prophotorgbvalues(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    let values = srgb2xyz(r, g, b, None);
    let values = xyz_d65_to_d50(values[0], values[1], values[2], None);
    let mut values = xyz50_to_prophotorgb(values[0], values[1], values[2], None);
    if let Some(a) = a.filter(|&a| a < 1.0) {
        values.push(a);
    }
    values
}

fn alpha_suffix(a: Option<f64>) -> Option<f64> {
    a.filter(|&a| a != 1.0)
}

fn prophotorgb2lin_prophoto(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    let mut values: Vec<f64> = [r, g, b]
        .iter()
        .map(|&v| {
            let abs = v.abs();
            if abs >= 16.0 / 512.0 {
                return js_sign(v) * abs.powf(1.8);
            }
            v / 16.0
        })
        .collect();
    values.extend(alpha_suffix(a));
    values
}

fn prophotorgb2xyz50(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    let lin = prophotorgb2lin_prophoto(r, g, b, a);
    let (r, g, b, a) = (lin[0], lin[1], lin[2], opt(&lin, 3));
    let mut xyz = vec![
        0.7977666449006423 * r + 0.1351812974005331 * g + 0.0313477341283922 * b,
        0.2880748288194013 * r + 0.7118352342418731 * g + 0.0000899369387256 * b,
        0.8251046025104602 * b,
    ];
    xyz.extend(alpha_suffix(a));
    xyz
}

fn xyz50_to_prophotorgb(x: f64, y: f64, z: f64, a: Option<f64>) -> Vec<f64> {
    gam_prophotorgb(
        x * 1.3457868816471585 - y * 0.2555720873797946 - 0.0511018649755453 * z,
        x * -0.5446307051249019 + y * 1.5082477428451466 + 0.0205274474364214 * z,
        1.2119675456389452 * z,
        a.filter(|&a| a != 1.0),
    )
}

fn gam_prophotorgbvalue(v: f64) -> f64 {
    let abs = v.abs();
    if abs >= 1.0 / 512.0 {
        return js_sign(v) * abs.powf(1.0 / 1.8);
    }
    16.0 * v
}

fn gam_prophotorgb(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    let mut values = vec![gam_prophotorgbvalue(r), gam_prophotorgbvalue(g), gam_prophotorgbvalue(b)];
    if let Some(a) = a.filter(|&a| a < 1.0) {
        values.push(a);
    }
    values
}
