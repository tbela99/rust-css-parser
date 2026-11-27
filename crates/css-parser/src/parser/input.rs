//! Port of `src/parser/input.ts`.
//!
//! css input: utf-8 bytes (the preferred input: the bindings pass the bytes of an `ArrayBuffer`) or a string. ascii
//! input is read from its bytes without copy: the same shared buffer is the text and the code units.

use crate::parser::utils::codes::CodeUnits;
use std::rc::Rc;

/// css input
#[derive(Clone, Debug)]
pub enum CssInput {
    Str(Rc<str>),
    Bytes(Rc<[u8]>),
}

impl From<&str> for CssInput {
    fn from(s: &str) -> Self {
        CssInput::Str(Rc::from(s))
    }
}

impl From<String> for CssInput {
    fn from(s: String) -> Self {
        CssInput::Str(Rc::from(s))
    }
}

impl From<Vec<u8>> for CssInput {
    fn from(b: Vec<u8>) -> Self {
        CssInput::Bytes(Rc::from(b))
    }
}

/// the css read by the tokenizer: the code units and the text
#[derive(Clone, Debug)]
pub struct CssCodeUnits {
    /// the utf-16 code units of the css. ascii css: the bytes of the input, without copy
    pub codes: CodeUnits,
    /// the text of the css
    pub text: Rc<str>,
}

/// view shared ascii / utf-8 bytes as a string without copy
pub fn rc_bytes_to_str(bytes: Rc<[u8]>) -> Result<Rc<str>, Rc<[u8]>> {
    if std::str::from_utf8(&bytes).is_err() {
        return Err(bytes);
    }
    // SAFETY: `str` and `[u8]` have the same layout and the bytes are valid utf-8
    Ok(unsafe { Rc::from_raw(Rc::into_raw(bytes) as *const str) })
}

/// view a shared string as bytes without copy
pub fn rc_str_to_bytes(text: Rc<str>) -> Rc<[u8]> {
    // SAFETY: `str` and `[u8]` have the same layout
    unsafe { Rc::from_raw(Rc::into_raw(text) as *const [u8]) }
}

/// true if the input is empty
pub fn is_empty_input(input: &CssInput) -> bool {
    match input {
        CssInput::Str(s) => s.is_empty(),
        CssInput::Bytes(b) => b.is_empty(),
    }
}

/// the utf-16 code units of a text. the ascii characters are copied as they are, the other characters are encoded: the
/// buffer is allocated once, `encode_utf16().collect()` grows it many times
fn utf16_units(text: &str) -> Rc<[u16]> {
    let bytes = text.as_bytes();
    let mut units: Vec<u16> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte < 0x80 {
            units.push(byte as u16);
            i += 1;
            continue;
        }
        // a character starts at i: the bytes are valid utf-8
        let c = text[i..].chars().next().expect("utf-8 character");
        let mut buffer = [0u16; 2];
        units.extend_from_slice(c.encode_utf16(&mut buffer));
        i += c.len_utf8();
    }
    units.into()
}

/// convert the css input to code units. an ascii input is read from its bytes: the code units are the bytes. the other
/// inputs are decoded (utf-8, without byte order mark) to utf-16 code units: the positions of the tokens are utf-16
/// positions, like the positions in a javascript string
pub fn to_code_units(input: Option<&CssInput>) -> CssCodeUnits {
    let empty = || CssCodeUnits { codes: CodeUnits::Bytes(Rc::from(&[][..])), text: Rc::from("") };
    match input {
        None => empty(),
        Some(CssInput::Str(text)) => {
            if text.is_empty() {
                return empty();
            }
            if text.is_ascii() {
                CssCodeUnits { codes: CodeUnits::Bytes(rc_str_to_bytes(text.clone())), text: text.clone() }
            } else {
                CssCodeUnits { codes: CodeUnits::Utf16(utf16_units(&text)), text: text.clone() }
            }
        }
        Some(CssInput::Bytes(bytes)) => {
            if bytes.is_empty() {
                return empty();
            }
            if bytes.is_ascii() {
                let text = rc_bytes_to_str(bytes.clone()).expect("ascii is utf-8");
                return CssCodeUnits { codes: CodeUnits::Bytes(bytes.clone()), text };
            }
            // TextDecoder: lossy utf-8, the byte order mark is removed
            let body = bytes.strip_prefix(&[0xef, 0xbb, 0xbf][..]).unwrap_or(bytes);
            let text: Rc<str> = Rc::from(String::from_utf8_lossy(body).as_ref());
            if text.is_ascii() {
                return CssCodeUnits { codes: CodeUnits::Bytes(rc_str_to_bytes(text.clone())), text };
            }
            CssCodeUnits { codes: CodeUnits::Utf16(utf16_units(&text)), text }
        }
    }
}
