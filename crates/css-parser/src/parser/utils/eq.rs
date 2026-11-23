//! Port of `src/parser/utils/eq.ts`.

use crate::ast::arena::AstArena;
use crate::plain::Plain;

/// deep equality of plain values (`eq()`)
pub fn eq(a: &Plain, b: &Plain) -> bool {
    match (a, b) {
        (Plain::Undefined | Plain::Null, _) | (_, Plain::Undefined | Plain::Null) => a.is_nullish() && b.is_nullish(),
        (Plain::Array(x), Plain::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| eq(a, b)),
        (Plain::Object(x), Plain::Object(y)) => {
            x.0.len() == y.0.len() && x.0.iter().all(|(k, v)| y.get(k).is_some_and(|w| eq(v, w)))
        }
        (Plain::Array(_), _) | (_, Plain::Array(_)) | (Plain::Object(_), _) | (_, Plain::Object(_)) => false,
        _ => a == b,
    }
}

/// structural equality of two tokens (the source location is ignored)
pub fn eq_token(arena: &AstArena, a: u32, b: u32) -> bool {
    if a == b {
        return true;
    }
    if arena.data_types.get(a) != arena.data_types.get(b)
        || arena.name.get(a) != arena.name.get(b)
        || arena.string_value.get(a) != arena.string_value.get(b)
        || arena.unit.get(a) != arena.unit.get(b)
        || arena.color_type.get(a) != arena.color_type.get(b)
    {
        return false;
    }
    let (x, y) = (arena.numeric_value.get(a), arena.numeric_value.get(b));
    // NaN values are equal
    if x != y && (!x.is_nan() || !y.is_nan()) {
        return false;
    }
    let (mut ta, mut tb) = (arena.first_token.get(a), arena.first_token.get(b));
    while ta != 0 && tb != 0 {
        if !eq_token(arena, ta, tb) {
            return false;
        }
        ta = arena.next_sibling.get(ta);
        tb = arena.next_sibling.get(tb);
    }
    if ta != tb {
        return false;
    }
    let (mut ta, mut tb) = (arena.first_child.get(a), arena.first_child.get(b));
    while ta != 0 && tb != 0 {
        if !eq_token(arena, ta, tb) {
            return false;
        }
        ta = arena.next_sibling.get(ta);
        tb = arena.next_sibling.get(tb);
    }
    ta == tb
}
