//! Port of `src/syntax/color/rec2020.ts`.

use super::srgb::xyz2srgb;
use super::utils::matrix::multiply_matrices;
use super::xyz::srgb2xyz;

const ALPHA: f64 = 1.09929682680944;
const BETA: f64 = 0.018053968510807;

fn suffix(a: Option<f64>) -> Option<f64> {
    a.filter(|&a| a != 1.0)
}

pub fn rec20202srgb(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    let values = rec20202lrec2020(r, g, b, None);
    let values = lrec20202xyz(values[0], values[1], values[2], None);
    xyz2srgb(values[0], values[1], values[2], a)
}

pub fn srgb2rec2020values(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    let values = srgb2xyz(r, g, b, None);
    let values = xyz2lrec2020(values[0], values[1], values[2], None);
    lrec20202rec2020(values[0], values[1], values[2], a)
}

fn rec20202lrec2020(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    let mut values: Vec<f64> = [r, g, b]
        .iter()
        .map(|&val| {
            let sign = if val < 0.0 { -1.0 } else { 1.0 };
            let abs = val.abs();
            if abs < BETA * 4.5 {
                return val / 4.5;
            }
            sign * ((abs + ALPHA - 1.0) / ALPHA).powf(1.0 / 0.45)
        })
        .collect();
    values.extend(suffix(a));
    values
}

fn lrec20202rec2020(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    let mut values: Vec<f64> = [r, g, b]
        .iter()
        .map(|&val| {
            let sign = if val < 0.0 { -1.0 } else { 1.0 };
            let abs = val.abs();
            if abs > BETA {
                return sign * (ALPHA * abs.powf(0.45) - (ALPHA - 1.0));
            }
            4.5 * val
        })
        .collect();
    values.extend(suffix(a));
    values
}

fn lrec20202xyz(r: f64, g: f64, b: f64, a: Option<f64>) -> Vec<f64> {
    const M: [[f64; 3]; 3] = [
        [63426534.0 / 99577255.0, 20160776.0 / 139408157.0, 47086771.0 / 278816314.0],
        [26158966.0 / 99577255.0, 472592308.0 / 697040785.0, 8267143.0 / 139408157.0],
        [0.0, 19567812.0 / 697040785.0, 295819943.0 / 278816314.0],
    ];
    let mut values = multiply_matrices(&M, [r, g, b]);
    values.extend(suffix(a));
    values
}

fn xyz2lrec2020(x: f64, y: f64, z: f64, a: Option<f64>) -> Vec<f64> {
    const M: [[f64; 3]; 3] = [
        [30757411.0 / 17917100.0, -6372589.0 / 17917100.0, -4539589.0 / 17917100.0],
        [-19765991.0 / 29648200.0, 47925759.0 / 29648200.0, 467509.0 / 29648200.0],
        [792561.0 / 44930125.0, -1921689.0 / 44930125.0, 42328811.0 / 44930125.0],
    ];
    let mut values = multiply_matrices(&M, [x, y, z]);
    values.extend(suffix(a));
    values
}
