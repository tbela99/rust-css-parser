//! Port of `src/parser/utils/text.ts`.

use std::borrow::Cow;

/// `fontSize` -> `font-size`
pub fn dasherize(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + 4);
    for c in value.chars() {
        if c.is_ascii_uppercase() {
            result.push('-');
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

/// `font-size` -> `fontSize`
pub fn camelize(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut result = String::with_capacity(value.len());
    let mut i = 0;
    while i < value.len() {
        if bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_lowercase() {
            result.push(bytes[i + 1].to_ascii_uppercase() as char);
            i += 2;
        } else {
            let c = value[i..].chars().next().unwrap();
            result.push(c);
            i += c.len_utf8();
        }
    }
    result
}

/// lowercase string, like `String.prototype.toLowerCase()`. the string is borrowed when it has no uppercase letter
pub fn lowercase(value: &str) -> Cow<'_, str> {
    if value.bytes().any(|b| b.is_ascii_uppercase() || b > 0x7f) {
        Cow::Owned(to_lower_case(value))
    } else {
        Cow::Borrowed(value)
    }
}

/// `String.prototype.toLowerCase()`
pub fn to_lower_case(value: &str) -> String {
    if value.is_ascii() {
        value.to_ascii_lowercase()
    } else {
        value.to_lowercase()
    }
}

/// compare ignoring the case of the ascii letters
pub fn equals_ignore_case(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.eq_ignore_ascii_case(b)
}
