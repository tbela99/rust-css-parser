//! Port of `src/ast/math/math.ts`.

use crate::ast::types::EnumToken;
use crate::syntax::syntax::minify_number;
use crate::utils::number::js_number_to_string;

/// a number or a fraction `[numerator, denominator]`
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NumOrFraction {
    Num(f64),
    Fraction(f64, f64),
}

/// greatest common divisor
pub fn gcd(x: f64, y: f64) -> f64 {
    let mut x = x.abs();
    let mut y = y.abs();
    if x == y {
        return x;
    }
    if y > x {
        std::mem::swap(&mut x, &mut y);
    }
    // `while (y)`: 0 and NaN are falsy
    while y != 0.0 && !y.is_nan() {
        let t = y;
        y = x % y;
        x = t;
    }
    x
}

/// compute an arithmetic operation. fractions are returned when shorter than the decimal value
pub fn compute(a: NumOrFraction, b: NumOrFraction, op: u8) -> NumOrFraction {
    if let (NumOrFraction::Num(a), NumOrFraction::Num(b)) = (a, b) {
        match op {
            EnumToken::Add => return NumOrFraction::Num(a + b),
            EnumToken::Sub => return NumOrFraction::Num(a - b),
            EnumToken::Mul => return NumOrFraction::Num(a * b),
            _ => {
                let r = simplify(a, b);
                if r.1 == 1.0 {
                    return NumOrFraction::Num(r.0);
                }
                let result = a / b;
                let r2 = format!("{}/{}", minify_number(r.0), minify_number(r.1));
                return if minify_number(result).len() < r2.len() { NumOrFraction::Num(result) } else { NumOrFraction::Fraction(r.0, r.1) };
            }
        }
    }
    let l1 = match a {
        NumOrFraction::Num(n) => (n, 1.0),
        NumOrFraction::Fraction(n, d) => (n, d),
    };
    let r1 = match b {
        NumOrFraction::Num(n) => (n, 1.0),
        NumOrFraction::Fraction(n, d) => (n, d),
    };
    let (l2, r2) = match op {
        EnumToken::Add => (l1.0 * r1.1 + l1.1 * r1.0, l1.1 * r1.1),
        EnumToken::Sub => (l1.0 * r1.1 - l1.1 * r1.0, l1.1 * r1.1),
        EnumToken::Mul => (l1.0 * r1.0, l1.1 * r1.1),
        _ => (l1.0 * r1.1, l1.1 * r1.0),
    };
    let a2 = simplify(l2, r2);
    if a2.1 == 1.0 {
        return NumOrFraction::Num(a2.0);
    }
    let result = a2.0 / a2.1;
    if minify_number(result).len() <= minify_number(a2.0).len() + 1 + minify_number(a2.1).len() {
        NumOrFraction::Num(result)
    } else {
        NumOrFraction::Fraction(a2.0, a2.1)
    }
}

/// the number of decimals of the non integer values, 0 when they are all integers
pub fn rem(a: &[f64]) -> usize {
    if a.iter().any(|i| !(i.is_finite() && i.fract() == 0.0)) {
        return a
            .iter()
            .map(|&b| js_number_to_string(b).split('.').nth(1).map_or(0, |d| d.len()))
            .fold(0, usize::max);
    }
    0
}

/// simplify a fraction
pub fn simplify(a: f64, b: f64) -> (f64, f64) {
    let g = gcd(a, b);
    if g > 1.0 {
        (a / g, b / g)
    } else {
        (a, b)
    }
}
