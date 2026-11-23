//! Port of `src/ast/transform/minify.ts`.

use super::compute::{compute_transform, transform_kind, TransformKind};
use super::utils::{decompose, multiply_in_place, round, to_zero_value, IDENTITY_MATRIX};
use super::{DecomposedMatrix3D, Matrix};
use crate::ast::arena::{AstArena, TokenValue};
use crate::ast::types::EnumToken;
use crate::syntax::constants::epsilon;

fn px(arena: &AstArena, value: f64) -> u32 {
    arena.create_token(EnumToken::LengthTokenType, TokenValue::Num(value), Some("px"))
}

fn deg(arena: &AstArena, value: f64) -> u32 {
    arena.create_token(EnumToken::AngleTokenType, TokenValue::Num(value), Some("deg"))
}

fn num(arena: &AstArena, value: f64) -> u32 {
    arena.create_token(EnumToken::NumberTokenType, TokenValue::Num(value), None)
}

fn comma(arena: &AstArena) -> u32 {
    arena.create_token(EnumToken::CommaTokenType, TokenValue::None, None)
}

fn function(arena: &AstArena, name: &str, tokens: &[u32]) -> u32 {
    arena.create_function(EnumToken::TransformFunctionTokenType, name, Some(tokens))
}

/// the shortest transform functions of a matrix: translate → rotate → skew → scale. `None` if the matrix cannot be
/// decomposed
pub fn minify(arena: &AstArena, matrix: &Matrix) -> Option<Vec<u32>> {
    let mut d = DecomposedMatrix3D { scale_x: 1.0, scale_y: 1.0, scale_z: 1.0, perspective_w: 1.0, ..Default::default() };
    if !decompose(matrix, &mut d) {
        return None;
    }
    let mut result: Vec<u32> = Vec::new();
    if !(round(d.translate_x) == 0.0 && round(d.translate_y) == 0.0 && round(d.translate_z) == 0.0) {
        let x = round(d.translate_x) != 0.0;
        let y = round(d.translate_y) != 0.0;
        let z = round(d.translate_z) != 0.0;
        if x as u8 + y as u8 + z as u8 == 1 {
            if x {
                result.push(function(arena, "translate", &[px(arena, round(d.translate_x))]));
            } else {
                result.push(function(arena, if y { "translateY" } else { "translateZ" }, &[px(arena, round(if y { d.translate_y } else { d.translate_z }))]));
            }
        } else if z {
            let tx = if d.translate_x == 0.0 { num(arena, 0.0) } else { px(arena, round(d.translate_x)) };
            let c1 = comma(arena);
            let ty = if d.translate_y == 0.0 { num(arena, 0.0) } else { px(arena, round(d.translate_y)) };
            let c2 = comma(arena);
            let tz = px(arena, round(d.translate_z));
            result.push(function(arena, "translate3d", &[tx, c1, ty, c2, tz]));
        } else {
            let tx = px(arena, round(d.translate_x));
            let c = comma(arena);
            let ty = px(arena, round(d.translate_y));
            result.push(function(arena, "translate", &[tx, c, ty]));
        }
    }
    if round(d.rotate_angle) != 0.0 {
        let (x, y, z, angle) = (d.rotate_x, d.rotate_y, d.rotate_z, d.rotate_angle);
        if y == 0.0 && z == 0.0 {
            result.push(function(arena, "rotateX", &[deg(arena, round(angle))]));
        } else if x == 0.0 && z == 0.0 {
            result.push(function(arena, "rotateY", &[deg(arena, round(angle))]));
        } else if x == 0.0 && y == 0.0 {
            result.push(function(arena, "rotate", &[deg(arena, round(angle))]));
        } else {
            let tokens = [num(arena, round(x)), comma(arena), num(arena, round(y)), comma(arena), num(arena, round(z)), comma(arena), deg(arena, round(angle))];
            result.push(function(arena, "rotate3d", &tokens));
        }
    }
    if !(round(d.skew_xy) == 0.0 && round(d.skew_xz) == 0.0) {
        // a single angle
        let single = round(d.skew_xz) == 0.0;
        let skew_x = round(d.skew_xy.atan() * 180.0 / std::f64::consts::PI);
        let skew_y = round(d.skew_xz.atan() * 180.0 / std::f64::consts::PI);
        if single {
            result.push(function(arena, "skew", &[deg(arena, round(skew_x))]));
        } else {
            let tokens = [deg(arena, round(skew_x)), comma(arena), deg(arena, round(skew_y))];
            result.push(function(arena, "skew", &tokens));
        }
    }
    if !(round(d.scale_x) == 1.0 && round(d.scale_y) == 1.0 && round(d.scale_z) == 1.0) {
        let sx = to_zero_value(d.scale_x);
        let sy = to_zero_value(d.scale_y);
        let sz = to_zero_value(d.scale_z);
        let (x, y, z) = (sx != 1.0, sy != 1.0, sz != 1.0);
        if x as u8 + y as u8 + z as u8 == 1 {
            let name = if x { "scaleX" } else if y { "scaleY" } else { "scaleZ" };
            result.push(function(arena, name, &[num(arena, round(if x { sx } else if y { sy } else { sz }))]));
        } else if !z {
            let tokens = [num(arena, round(sx)), comma(arena), num(arena, round(sy))];
            result.push(function(arena, "scale", &tokens));
        } else {
            let tokens = [num(arena, round(sx)), comma(arena), num(arena, round(sy)), comma(arena), num(arena, round(sz))];
            result.push(function(arena, "scale3d", &tokens));
        }
    }
    // identity
    if result.is_empty() || (result.len() == 1 && eq_matrix(arena, &IDENTITY_MATRIX, &result)) {
        result.clear();
        result.push(arena.create_token(EnumToken::IdenTokenType, TokenValue::Str("none"), None));
    }
    Some(result)
}

/// true if a list of transform functions is equivalent to a matrix (the values differ by at most `epsilon`)
pub fn eq_matrix(arena: &AstArena, data: &Matrix, b: &[u32]) -> bool {
    let mut mat = IDENTITY_MATRIX;
    for &token in b {
        let mut function_matrix = IDENTITY_MATRIX;
        if !compute_transform(arena, token, &mut function_matrix) {
            return false;
        }
        multiply_in_place(&mut mat, &function_matrix);
    }
    // `Math.abs(a - b) > epsilon`: NaN differences are not greater
    !(0..16).any(|i| (mat[i] - data[i]).abs() > epsilon)
}

/// shorter form of a transform function: `translate3d(0, 10px, 0)` → `translateY(10px)`, `scale3d(2, 1, 1)` →
/// `scaleX(2)`, `skewX()` → `skew()`. the percentages of the scale functions are converted to numbers
pub fn minify_transform_functions(arena: &AstArena, transform: u32) -> u32 {
    use TransformKind as K;
    let kind = transform_kind(arena, arena.string_value.get(transform)) & K::KindMask;
    if kind == K::SkewX {
        arena.string_value.set(transform, arena.intern_string("skew"));
        return transform;
    }
    if !matches!(kind, K::Translate | K::Translate3d | K::Scale | K::Scale3d) {
        return transform;
    }
    let is_scale = kind == K::Scale || kind == K::Scale3d;
    let (mut first, mut second, mut third) = (0u32, 0u32, 0u32);
    let mut count = 0;
    for token in arena.token_iter(transform).collect::<Vec<_>>() {
        let typ = arena.data_types.get(token);
        if matches!(typ, EnumToken::CommentTokenType | EnumToken::WhitespaceTokenType | EnumToken::CommaTokenType) {
            continue;
        }
        if !matches!(typ, EnumToken::NumberTokenType | EnumToken::LengthTokenType | EnumToken::AngleTokenType | EnumToken::PercentageTokenType) {
            return transform;
        }
        if typ == EnumToken::PercentageTokenType && is_scale && !arena.has_fraction(token) {
            arena.data_types.set(token, EnumToken::NumberTokenType);
            arena.numeric_value.set(token, arena.numeric_value.get(token) / 100.0);
        }
        match count {
            0 => first = token,
            1 => second = token,
            2 => third = token,
            _ => {}
        }
        count += 1;
    }
    if count == 0 || ((kind == K::Translate || kind == K::Scale) && count > 2) {
        return transform;
    }
    let ignored = if is_scale { 1.0 } else { 0.0 };
    let x = arena.numeric_value.get(first) != ignored;
    let y = count > 1 && arena.numeric_value.get(second) != ignored;
    let z = count > 2 && arena.numeric_value.get(third) != ignored;
    let size = x as u8 + y as u8 + z as u8;
    if size == 2 && z {
        return transform;
    }
    let pick = if x { first } else if y { second } else { third };
    match (is_scale, size) {
        (false, 0) => function(arena, "translate", &[num(arena, 0.0)]),
        (false, 1) => function(arena, if x { "translate" } else if y { "translateY" } else { "translateZ" }, &[pick]),
        (false, 2) => {
            let c = comma(arena);
            function(arena, "translate", &[first, c, second])
        }
        (true, 0) => function(arena, "scale", &[num(arena, 1.0)]),
        (true, 1) => function(arena, if x { "scaleX" } else if y { "scaleY" } else { "scaleZ" }, &[pick]),
        (true, 2) => {
            let c = comma(arena);
            function(arena, "scale", &[first, c, second])
        }
        _ => transform,
    }
}
