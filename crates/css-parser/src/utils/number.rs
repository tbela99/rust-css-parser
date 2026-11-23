//! javascript number formatting (`Number.prototype.toString()`), used wherever the typescript code converts a number
//! to a string (`"" + n`, template literals).

/// `String(n)`: shortest round-trip representation, javascript exponent rules
pub fn js_number_to_string(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    if n == 0.0 {
        return "0".into();
    }
    // shortest round trip digits and exponent from rust's `{:e}` formatting
    let formatted = format!("{:e}", n);
    let (mantissa, exponent) = formatted.split_once('e').unwrap();
    let exponent: i32 = exponent.parse().unwrap();
    let negative = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(|c| c.is_ascii_digit()).collect();
    let k = digits.len() as i32;
    // decimal point position: n = 0.digits * 10^(exponent + 1)
    let point = exponent + 1;
    let mut result = String::new();
    if negative {
        result.push('-');
    }
    if k <= point && point <= 21 {
        result.push_str(&digits);
        for _ in 0..(point - k) {
            result.push('0');
        }
    } else if 0 < point && point <= 21 {
        result.push_str(&digits[..point as usize]);
        result.push('.');
        result.push_str(&digits[point as usize..]);
    } else if -6 < point && point <= 0 {
        result.push_str("0.");
        for _ in 0..(-point) {
            result.push('0');
        }
        result.push_str(&digits);
    } else {
        let e = point - 1;
        result.push_str(&digits[..1]);
        if k > 1 {
            result.push('.');
            result.push_str(&digits[1..]);
        }
        result.push('e');
        result.push(if e >= 0 { '+' } else { '-' });
        result.push_str(&e.abs().to_string());
    }
    result
}

/// `Number(str)` / unary `+str` for css numeric strings
pub fn js_parse_number(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    if t.len() > 2 && (t.starts_with("0x") || t.starts_with("0X")) {
        return i64::from_str_radix(&t[2..], 16).map_or(f64::NAN, |v| v as f64);
    }
    if t.chars().all(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E')) {
        // rust rejects "1." and ".5"? it accepts both; it rejects "+.5"? accepted as well
        t.parse::<f64>().unwrap_or(f64::NAN)
    } else {
        f64::NAN
    }
}

/// `n.toFixed(digits)`
pub fn to_fixed(n: f64, digits: usize) -> String {
    if !n.is_finite() {
        return js_number_to_string(n);
    }
    // javascript picks the larger n on exact ties (half away from zero for the magnitude), rust rounds half to even
    let wide = format!("{:.*}", digits + 30, n.abs());
    let cut = wide.len() - 30;
    let tail = &wide[cut..];
    let magnitude = if tail.starts_with('5') && tail[1..].bytes().all(|b| b == b'0') {
        let bumped = n.abs() + 0.5 * 10f64.powi(-(digits as i32));
        let s = format!("{:.*}", digits + 30, bumped);
        s[..s.len() - 30].trim_end_matches('.').to_string()
    } else {
        format!("{:.*}", digits, n.abs())
    };
    // `(-0.0001).toFixed(2)` is "-0.00" in javascript
    if n < 0.0 {
        format!("-{}", magnitude)
    } else {
        magnitude
    }
}

/// `Math.round()`: half up (towards +inf), `-0` for the values in [-0.5, 0)
pub fn js_round(n: f64) -> f64 {
    if !n.is_finite() {
        return n;
    }
    let floor = n.floor();
    let r = if n - floor >= 0.5 { floor + 1.0 } else { floor };
    if r == 0.0 && n < 0.0 {
        -0.0
    } else {
        r
    }
}

/// `String(n)` for an integer or a float
pub fn num_to_str(n: f64) -> String {
    js_number_to_string(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_to_string() {
        for (n, s) in [(1.0, "1"), (0.1, "0.1"), (1e21, "1e+21"), (1e-7, "1e-7"), (123.456, "123.456"), (-0.5, "-0.5"), (1e20, "100000000000000000000"), (0.000001, "0.000001")] {
            assert_eq!(js_number_to_string(n), s);
        }
    }

    #[test]
    fn fixed() {
        assert_eq!(to_fixed(2.5, 0), "3");
        assert_eq!(to_fixed(1.005, 2), "1.00");
        assert_eq!(to_fixed(0.125, 2), "0.13");
        assert_eq!(to_fixed(-1.5, 0), "-2");
        assert_eq!(to_fixed(1.23456, 3), "1.235");
    }
}

/// `+value.toPrecision(precision)`: the exact decimal value rounded half up to `precision` significant digits, parsed
pub fn js_to_precision(value: f64, precision: usize) -> f64 {
    if !value.is_finite() || value == 0.0 {
        return value;
    }
    // exact decimal digits: the digit after the last kept digit decides (half up on the exact value)
    let s = format!("{:.*e}", precision + 40, value.abs());
    let (mantissa, exponent) = s.split_once('e').unwrap();
    let mut exponent: i32 = exponent.parse().unwrap();
    let digits: Vec<u8> = mantissa.bytes().filter(|b| b.is_ascii_digit()).map(|b| b - b'0').collect();
    let mut kept: Vec<u8> = digits[..precision].to_vec();
    if digits[precision] >= 5 {
        let mut i = precision;
        loop {
            if i == 0 {
                kept.insert(0, 1);
                kept.pop();
                exponent += 1;
                break;
            }
            i -= 1;
            if kept[i] == 9 {
                kept[i] = 0;
            } else {
                kept[i] += 1;
                break;
            }
        }
    }
    let text: String = kept.iter().map(|d| (b'0' + d) as char).collect();
    let parsed: f64 = format!("{}.{}e{}", &text[..1], &text[1..], exponent).parse().unwrap();
    if value < 0.0 {
        -parsed
    } else {
        parsed
    }
}
