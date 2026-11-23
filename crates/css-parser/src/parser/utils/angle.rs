//! Port of `src/parser/utils/angle.ts`.

use crate::ast::arena::AstArena;

/// convert an angle to degrees (in place)
pub fn to_degrees(arena: &AstArena, angle: u32) -> u32 {
    let factor = arena.with_string(arena.unit.get(angle), |unit| match unit {
        "rad" => Some(180.0 / std::f64::consts::PI),
        "grad" => Some(0.9),
        "turn" => Some(360.0),
        _ => None,
    });
    if let Some(factor) = factor {
        arena.numeric_value.set(angle, arena.numeric_value.get(angle) * factor);
        arena.unit.set(angle, arena.intern_string("deg"));
    }
    angle
}
