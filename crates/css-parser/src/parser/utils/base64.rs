//! Port of `src/parser/utils/base64.ts`.

use base64::Engine;

/// base64 of the utf-8 encoding of a string
pub fn to_base64(value: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
}
