//! Helpers of the color ports (no typescript counterpart): javascript number array semantics.
//!
//! The typescript conversions pass `values[3]` (possibly `undefined`) as the optional alpha argument: [`opt`] is that
//! access, [`at`] reads a component (`NaN` like `undefined` in arithmetic when it is missing).

/// `values[i]` as a number: `NaN` when missing
#[inline]
pub fn at(values: &[f64], i: usize) -> f64 {
    values.get(i).copied().unwrap_or(f64::NAN)
}

/// `values[i]` as an optional argument: `None` when missing (`undefined`)
#[inline]
pub fn opt(values: &[f64], i: usize) -> Option<f64> {
    values.get(i).copied()
}

/// `Math.max(...values)`: `NaN` when one of the values is `NaN`
pub fn js_max(values: &[f64]) -> f64 {
    let mut max = f64::NEG_INFINITY;
    for &v in values {
        if v.is_nan() {
            return f64::NAN;
        }
        // `Math.max(-0, 0)` is 0
        if v > max || (v == 0.0 && max == 0.0 && max.is_sign_negative()) {
            max = v;
        }
    }
    max
}

/// `Math.min(...values)`: `NaN` when one of the values is `NaN`
pub fn js_min(values: &[f64]) -> f64 {
    let mut min = f64::INFINITY;
    for &v in values {
        if v.is_nan() {
            return f64::NAN;
        }
        if v < min || (v == 0.0 && min == 0.0 && v.is_sign_negative()) {
            min = v;
        }
    }
    min
}

/// `Math.sign()`
pub fn js_sign(v: f64) -> f64 {
    if v.is_nan() || v == 0.0 {
        v
    } else {
        v.signum()
    }
}

/// `Math.sign(v) || 1`
pub fn js_sign_or_one(v: f64) -> f64 {
    let s = js_sign(v);
    if s == 0.0 || s.is_nan() {
        1.0
    } else {
        s
    }
}

/// `Number.prototype.toString(16)`
pub fn to_hex_string(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    let negative = v < 0.0;
    let abs = v.abs();
    let int = abs.trunc();
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if int < 9007199254740992.0 {
        out.push_str(&format!("{:x}", int as u64));
    } else {
        // large values: digits of the integer part
        let mut digits = Vec::new();
        let mut n = int;
        while n >= 1.0 {
            let d = (n % 16.0) as u32;
            digits.push(std::char::from_digit(d, 16).unwrap());
            n = (n / 16.0).trunc();
        }
        out.extend(digits.iter().rev());
    }
    let mut fraction = abs - int;
    if fraction > 0.0 {
        out.push('.');
        let mut count = 0;
        while fraction > 0.0 && count < 52 {
            fraction *= 16.0;
            let d = fraction.trunc();
            out.push(std::char::from_digit(d as u32, 16).unwrap());
            fraction -= d;
            count += 1;
        }
    }
    out
}

/// `value.toString(16).padStart(2, "0")`
pub fn hex2(v: f64) -> String {
    format!("{:0>2}", to_hex_string(v))
}
