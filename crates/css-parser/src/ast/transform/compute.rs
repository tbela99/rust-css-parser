//! Port of `src/ast/transform/compute.ts`: computation of the matrix of a transform list.

use super::matrix::{serialize, set_matrix_values};
use super::minify::minify;
use super::perspective::perspective;
use super::rotate::{rotate, rotate3d};
use super::scale::{scale, scale3d, scale_x, scale_y, scale_z};
use super::skew::{skew, skew_x, skew_y};
use super::translate::{translate, translate3d, translate_x, translate_y, translate_z};
use super::utils::{multiply_in_place, to_zero, IDENTITY_MATRIX};
use super::Matrix;
use crate::ast::arena::{AstArena, TokenValue};
use crate::ast::strings::StringDerived;
use crate::ast::types::EnumToken;
use crate::syntax::color::color::{get_angle, get_number};
use crate::syntax::syntax::length2_px;

/// kind of a transform function name, and family of the name (`TransformKind`)
#[allow(non_snake_case, non_upper_case_globals)]
pub mod TransformKind {
    pub const None: u32 = 0;
    pub const Translate: u32 = 1;
    pub const TranslateX: u32 = 2;
    pub const TranslateY: u32 = 3;
    pub const TranslateZ: u32 = 4;
    pub const Translate3d: u32 = 5;
    pub const Rotate: u32 = 6;
    pub const RotateX: u32 = 7;
    pub const RotateY: u32 = 8;
    pub const RotateZ: u32 = 9;
    pub const Rotate3d: u32 = 10;
    pub const Scale: u32 = 11;
    pub const ScaleX: u32 = 12;
    pub const ScaleY: u32 = 13;
    pub const ScaleZ: u32 = 14;
    pub const Scale3d: u32 = 15;
    pub const Skew: u32 = 16;
    pub const SkewX: u32 = 17;
    pub const SkewY: u32 = 18;
    pub const Perspective: u32 = 19;
    pub const Matrix: u32 = 20;
    pub const Matrix3d: u32 = 21;
    /// the name of the `transform` property or of a prefixed version (`-webkit-transform`)
    pub const Property: u32 = 22;
    pub const KindMask: u32 = 0xff;
    pub const TranslateFamily: u32 = 1 << 8;
    pub const RotateFamily: u32 = 2 << 8;
    pub const ScaleFamily: u32 = 3 << 8;
    pub const FamilyMask: u32 = 3 << 8;
}

fn kind_of_name(name: &str) -> u32 {
    use TransformKind as K;
    match name {
        "translate" => K::Translate,
        "translateX" => K::TranslateX,
        "translateY" => K::TranslateY,
        "translateZ" => K::TranslateZ,
        "translate3d" => K::Translate3d,
        "rotate" => K::Rotate,
        "rotateX" => K::RotateX,
        "rotateY" => K::RotateY,
        "rotateZ" => K::RotateZ,
        "rotate3d" => K::Rotate3d,
        "scale" => K::Scale,
        "scaleX" => K::ScaleX,
        "scaleY" => K::ScaleY,
        "scaleZ" => K::ScaleZ,
        "scale3d" => K::Scale3d,
        "skew" => K::Skew,
        "skewX" => K::SkewX,
        "skewY" => K::SkewY,
        "perspective" => K::Perspective,
        "matrix" => K::Matrix,
        "matrix3d" => K::Matrix3d,
        _ => K::None,
    }
}

/// `/^(-[a-z]+-)?transform$/`
fn is_transform_property(name: &str) -> bool {
    let Some(prefix) = name.strip_suffix("transform") else {
        return false;
    };
    prefix.is_empty() || (prefix.len() > 2 && prefix.starts_with('-') && prefix.ends_with('-') && prefix[1..prefix.len() - 1].bytes().all(|b| b.is_ascii_lowercase()))
}

fn compute_transform_kind(name: &str) -> u32 {
    let mut kind = kind_of_name(name);
    if kind == TransformKind::None && is_transform_property(name) {
        return TransformKind::Property;
    }
    if name.starts_with("translate") {
        kind |= TransformKind::TranslateFamily;
    } else if name.starts_with("rotate") {
        kind |= TransformKind::RotateFamily;
    } else if name.starts_with("scale") {
        kind |= TransformKind::ScaleFamily;
    }
    kind
}

/// the kind and the family of a string, computed once per interned string
pub fn transform_kind(arena: &AstArena, id: u32) -> u32 {
    arena.string_derived(id, StringDerived::TransformKind, |id| arena.with_string(id, compute_transform_kind))
}

/// multiply a matrix in place by the matrix of a transform function. false if the matrix cannot be computed (it is
/// then partially updated)
pub fn compute_transform(arena: &AstArena, token: u32, matrix: &mut Matrix) -> bool {
    use TransformKind as K;
    if arena.data_types.get(token) != EnumToken::TransformFunctionTokenType {
        return false;
    }
    let kind = transform_kind(arena, arena.string_value.get(token)) & K::KindMask;
    let mut args = [0.0f64; 16];
    let mut count = 0usize;
    let t = |x: u32| arena.data_types.get(x);
    let children = arena.token_iter(token).collect::<Vec<_>>();
    match kind {
        K::Translate | K::TranslateX | K::TranslateY | K::TranslateZ | K::Translate3d => {
            for &child in &children {
                if matches!(t(child), EnumToken::CommentTokenType | EnumToken::CommaTokenType | EnumToken::WhitespaceTokenType) {
                    continue;
                }
                let Some(val) = length2_px(arena, child) else {
                    return false;
                };
                if count < 16 {
                    args[count] = val;
                }
                count += 1;
            }
            let max = if kind == K::Translate3d { 3 } else if kind == K::Translate { 2 } else { 1 };
            if count == 0 || count > max {
                return false;
            }
            match kind {
                K::TranslateX => translate_x(args[0], matrix),
                K::TranslateY => translate_y(args[0], matrix),
                K::TranslateZ => translate_z(args[0], matrix),
                K::Translate => translate(args[0], if count > 1 { Some(args[1]) } else { None }, matrix),
                _ => translate3d(args[0], if count > 1 { Some(args[1]) } else { None }, if count > 2 { Some(args[2]) } else { None }, matrix),
            }
            true
        }
        K::Rotate | K::RotateX | K::RotateY | K::RotateZ | K::Rotate3d => {
            let (mut x, mut y, mut z) = (0.0, 0.0, 0.0);
            let (mut first, mut second, mut third, mut last) = (0u32, 0u32, 0u32, 0u32);
            // the whitespace tokens are arguments
            for &child in &children {
                if matches!(t(child), EnumToken::CommentTokenType | EnumToken::CommaTokenType) {
                    continue;
                }
                match count {
                    0 => first = child,
                    1 => second = child,
                    2 => third = child,
                    _ => {}
                }
                last = child;
                count += 1;
            }
            if count != if kind == K::Rotate3d { 4 } else { 1 } {
                return false;
            }
            match kind {
                K::RotateX => x = 1.0,
                K::RotateY => y = 1.0,
                K::Rotate | K::RotateZ => z = 1.0,
                _ => {
                    x = get_number(arena, first);
                    y = get_number(arena, second);
                    z = get_number(arena, third);
                }
            }
            let angle = get_angle(arena, last);
            if x.is_nan() || y.is_nan() || z.is_nan() || angle.is_nan() {
                return false;
            }
            if kind == K::Rotate || kind == K::RotateZ {
                rotate(angle * 2.0 * std::f64::consts::PI, matrix);
            } else {
                rotate3d(angle * 2.0 * std::f64::consts::PI, x, y, z, matrix);
            }
            true
        }
        K::Scale | K::ScaleX | K::ScaleY | K::ScaleZ | K::Scale3d => {
            for &child in &children {
                let typ = t(child);
                if typ == EnumToken::CommentTokenType || typ == EnumToken::CommaTokenType {
                    continue;
                }
                // whitespace tokens are not allowed
                if typ != EnumToken::NumberTokenType && typ != EnumToken::PercentageTokenType {
                    return false;
                }
                if count < 16 {
                    args[count] = get_number(arena, child);
                }
                count += 1;
            }
            match kind {
                K::Scale3d => {
                    if count != 3 {
                        return false;
                    }
                    scale3d(args[0], args[1], args[2], matrix);
                }
                K::Scale => {
                    if count != 1 && count != 2 {
                        return false;
                    }
                    scale(args[0], if count > 1 { args[1] } else { args[0] }, matrix);
                }
                _ if count != 1 => return false,
                K::ScaleX => scale_x(args[0], matrix),
                K::ScaleY => scale_y(args[0], matrix),
                _ => scale_z(args[0], matrix),
            }
            true
        }
        K::Skew | K::SkewX | K::SkewY => {
            for &child in &children {
                if matches!(t(child), EnumToken::CommentTokenType | EnumToken::CommaTokenType | EnumToken::WhitespaceTokenType) {
                    continue;
                }
                if count < 16 {
                    args[count] = get_angle(arena, child) * 2.0 * std::f64::consts::PI;
                }
                count += 1;
            }
            if count == 0 || count > if kind == K::Skew { 2 } else { 1 } {
                return false;
            }
            match kind {
                K::Skew => skew(args[0], if count > 1 { Some(args[1]) } else { None }, matrix),
                K::SkewX => skew_x(args[0], matrix),
                _ => skew_y(args[0], matrix),
            }
            true
        }
        K::Perspective => {
            // None for `none`
            let mut distance: Option<f64> = None;
            for &child in &children {
                let typ = t(child);
                if typ == EnumToken::CommentTokenType || typ == EnumToken::WhitespaceTokenType {
                    continue;
                }
                count += 1;
                if typ == EnumToken::IdenTokenType && arena.string_equals(arena.string_value.get(child), "none") {
                    distance = None;
                    continue;
                }
                let Some(val) = length2_px(arena, child) else {
                    return false;
                };
                distance = Some(val);
            }
            if count != 1 {
                return false;
            }
            perspective(distance, matrix);
            true
        }
        K::Matrix | K::Matrix3d => {
            for &child in &children {
                if matches!(t(child), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType | EnumToken::CommaTokenType) {
                    continue;
                }
                if count < 16 {
                    args[count] = get_number(arena, child);
                }
                count += 1;
            }
            if count != if kind == K::Matrix { 6 } else { 16 } {
                return false;
            }
            let mut values = [0.0; 16];
            set_matrix_values(&args, count, &mut values);
            multiply_in_place(matrix, &values);
            true
        }
        _ => false,
    }
}

/// multiply a matrix in place by the matrices of a list of transform functions
pub fn compute_matrix(arena: &AstArena, tokens: &[u32], start: usize, end: usize, matrix: &mut Matrix) -> bool {
    tokens[start..end].iter().all(|&t| compute_transform(arena, t, matrix))
}

/// result of [`compute`]
pub struct ComputedTransform {
    /// the values of the matrix of the transform list, rounded (see `to_zero()`)
    pub values: Matrix,
    /// the matrix of the transform list: `none`, `matrix()` or `matrix3d()`
    pub matrix: u32,
    /// the transform list, each run of translations, rotations or scales and each other function minified
    pub cumulative: Vec<u32>,
    /// the minified matrix
    pub minified: Vec<u32>,
    /// true if the tokens of the `cumulative` list are all new tokens
    pub created: bool,
}

/// compute the matrix of a transform list. `None` if the matrix cannot be computed
pub fn compute(arena: &AstArena, transform_list: &[u32]) -> Option<ComputedTransform> {
    use TransformKind as K;
    // all rotation arguments must be numbers, percentages or angles below one turn
    for &token in transform_list {
        let kind = transform_kind(arena, arena.string_value.get(token)) & K::KindMask;
        if matches!(kind, K::Rotate | K::RotateX | K::RotateY | K::RotateZ | K::Rotate3d) {
            for child in arena.token_iter(token) {
                let typ = arena.data_types.get(child);
                if typ == EnumToken::WhitespaceTokenType || typ == EnumToken::CommaTokenType {
                    continue;
                }
                if (typ != EnumToken::AngleTokenType && typ != EnumToken::NumberTokenType && typ != EnumToken::PercentageTokenType)
                    // angle >= 360deg, do not transform
                    || get_angle(arena, child).abs() >= 1.0
                {
                    return None;
                }
            }
        }
    }
    let (tokens, runs) = split_transform_list(arena, transform_list);
    let mut cumulative: Vec<u32> = Vec::new();
    let mut matrix: Matrix = IDENTITY_MATRIX;
    let mut empty = true;
    let mut created = true;
    for run in 0..runs.len().saturating_sub(1) {
        let (start, end) = (runs[run], runs[run + 1]);
        let mut run_matrix = IDENTITY_MATRIX;
        if !compute_matrix(arena, &tokens, start, end, &mut run_matrix) {
            return None;
        }
        multiply_in_place(&mut matrix, &run_matrix);
        match minify(arena, &run_matrix) {
            None => {
                created = false;
                empty = false;
                for &token in &tokens[start..end] {
                    push_transform(arena, &mut cumulative, token);
                }
            }
            Some(minified) => {
                empty = empty && minified.is_empty();
                for token in minified {
                    push_transform(arena, &mut cumulative, token);
                }
            }
        }
    }
    let serialized = serialize(arena, &matrix);
    if !empty && cumulative.is_empty() {
        cumulative.push(arena.create_token(EnumToken::IdenTokenType, TokenValue::Str("none"), None));
    }
    to_zero(&mut matrix);
    let matrix_token = serialize(arena, &matrix);
    let minified = minify(arena, &matrix).unwrap_or_else(|| vec![serialized]);
    Some(ComputedTransform { values: matrix, matrix: matrix_token, cumulative, minified, created })
}

/// append a token to a transform list. `none` is omitted
fn push_transform(arena: &AstArena, list: &mut Vec<u32>, token: u32) {
    if arena.data_types.get(token) != EnumToken::IdenTokenType || !arena.string_equals(arena.string_value.get(token), "none") {
        list.push(token);
    }
}

/// split a transform list into runs: the consecutive translations, rotations and scales are grouped, the other
/// tokens are alone. the comments and the whitespace tokens are ignored. `(tokens, run starts + end)`
fn split_transform_list(arena: &AstArena, transform_list: &[u32]) -> (Vec<u32>, Vec<usize>) {
    let mut pattern = 0u32;
    let mut tokens: Vec<u32> = Vec::new();
    let mut runs: Vec<usize> = Vec::new();
    for &token in transform_list {
        let typ = arena.data_types.get(token);
        if typ == EnumToken::CommentTokenType || typ == EnumToken::WhitespaceTokenType {
            continue;
        }
        if typ == EnumToken::TransformFunctionTokenType {
            let family = transform_kind(arena, arena.string_value.get(token)) & TransformKind::FamilyMask;
            if pattern != 0 && family == pattern {
                // continue the current run
                tokens.push(token);
                continue;
            }
            pattern = family;
        }
        runs.push(tokens.len());
        tokens.push(token);
    }
    runs.push(tokens.len());
    (tokens, runs)
}
