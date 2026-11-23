//! Port of `src/ast/transform/matrix.ts`.

use super::utils::{is_2d_matrix, is_identity};
use super::Matrix;
use crate::ast::arena::{AstArena, TokenValue};
use crate::ast::types::EnumToken;

/// set the values of a `matrix()` (6 values) or of a `matrix3d()` (16 values) function
pub fn set_matrix_values(values: &[f64], count: usize, matrix: &mut Matrix) {
    if count == 6 {
        *matrix = [values[0], values[1], 0.0, 0.0, values[2], values[3], 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, values[4], values[5], 0.0, 1.0];
    } else {
        matrix.copy_from_slice(&values[..16]);
    }
}

/// the tokens of a matrix: `none`, `matrix()` if it is a 2d matrix, `matrix3d()` otherwise
pub fn serialize(arena: &AstArena, matrix: &Matrix) -> u32 {
    if is_identity(matrix) {
        return arena.create_token(EnumToken::IdenTokenType, TokenValue::Str("none"), None);
    }
    // https://drafts.csswg.org/css-transforms-2/#two-dimensional-subset
    let is_2d = is_2d_matrix(matrix);
    let count = if is_2d { 6 } else { 16 };
    let mut tokens: Vec<u32> = Vec::with_capacity(count * 2 - 1);
    for i in 0..count {
        if i > 0 {
            tokens.push(arena.create_token(EnumToken::CommaTokenType, TokenValue::None, None));
        }
        // matrix(a, b, c, d, e, f): values 0, 1, 4, 5, 12, 13
        let index = if is_2d {
            if i < 2 {
                i
            } else if i < 4 {
                i + 2
            } else {
                i + 8
            }
        } else {
            i
        };
        tokens.push(arena.create_token(EnumToken::NumberTokenType, TokenValue::Num(matrix[index]), None));
    }
    arena.create_function(EnumToken::TransformFunctionTokenType, if is_2d { "matrix" } else { "matrix3d" }, Some(&tokens))
}
