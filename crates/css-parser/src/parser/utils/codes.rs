//! Port of `src/parser/utils/codes.ts`.
//!
//! Hashing and comparison of strings and of ranges of code units, without creating strings.
//! The values of the tokens are ranges of the code units of the source: a string and a range with the same content
//! have the same hash (30 bits).

use std::rc::Rc;

/// a code unit: a byte of an ascii input or an utf-16 code unit
pub trait Unit: Copy + Into<u32> + 'static {}
impl Unit for u8 {}
impl Unit for u16 {}

/// code units of the css source: the bytes of an ascii input, the utf-16 code units otherwise.
/// shared (reference counted): the tokenizer, the string table and the sources reference the same buffer
#[derive(Clone, Debug)]
pub enum CodeUnits {
    Bytes(Rc<[u8]>),
    Utf16(Rc<[u16]>),
}

impl CodeUnits {
    /// code units of a string: the bytes when the string is ascii, the utf-16 code units otherwise
    pub fn from_str(text: &str) -> Self {
        if text.is_ascii() {
            CodeUnits::Bytes(Rc::from(text.as_bytes()))
        } else {
            CodeUnits::Utf16(text.encode_utf16().collect::<Vec<u16>>().into())
        }
    }

    /// code units of utf-16 code units: bytes when every unit is ascii
    pub fn from_utf16(units: &[u16]) -> Self {
        if units.iter().all(|&u| u < 0x80) {
            CodeUnits::Bytes(units.iter().map(|&u| u as u8).collect::<Vec<u8>>().into())
        } else {
            CodeUnits::Utf16(Rc::from(units))
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        match self {
            CodeUnits::Bytes(b) => b.len(),
            CodeUnits::Utf16(u) => u.len(),
        }
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// code unit at an index (`codes[i]`). out of range reads return `0xffffffff` (`undefined` in typescript)
    #[inline]
    pub fn at(&self, index: usize) -> u32 {
        match self {
            CodeUnits::Bytes(b) => b.get(index).map_or(u32::MAX, |&c| c as u32),
            CodeUnits::Utf16(u) => u.get(index).map_or(u32::MAX, |&c| c as u32),
        }
    }

    /// the text of a range. ascii ranges are borrowed: no copy
    pub fn slice(&self, start: usize, end: usize) -> std::borrow::Cow<'_, str> {
        match self {
            // ascii bytes are valid utf-8
            CodeUnits::Bytes(b) => std::borrow::Cow::Borrowed(unsafe { std::str::from_utf8_unchecked(&b[start..end]) }),
            CodeUnits::Utf16(u) => std::borrow::Cow::Owned(String::from_utf16_lossy(&u[start..end])),
        }
    }

    pub fn hash_range(&self, start: usize, end: usize) -> i32 {
        match self {
            CodeUnits::Bytes(b) => hash_range(b, start, end),
            CodeUnits::Utf16(u) => hash_range(u, start, end),
        }
    }

    pub fn hash_range_lower(&self, start: usize, end: usize) -> i32 {
        match self {
            CodeUnits::Bytes(b) => hash_range_lower(b, start, end),
            CodeUnits::Utf16(u) => hash_range_lower(u, start, end),
        }
    }
}

/// FNV-1a offset basis
const FNV_OFFSET: u32 = 0x811c9dc5;
/// FNV-1a prime
const FNV_PRIME: u32 = 0x01000193;
/// the hashes are 30 bits integers
const HASH_MASK: u32 = 0x3fffffff;
/// KELVIN SIGN: lowercased to `k` by `toLowerCase()`
const KELVIN_SIGN: u32 = 0x212a;

/// hash of a string (case sensitive), over its utf-16 code units. same value as [`hash_range`]
pub fn hash_string(str: &str) -> i32 {
    let mut hash = FNV_OFFSET;
    for code in str.encode_utf16() {
        hash = (hash ^ code as u32).wrapping_mul(FNV_PRIME);
    }
    (hash & HASH_MASK) as i32
}

/// hash of utf-16 code units (case sensitive)
pub fn hash_units<T: Unit>(codes: &[T]) -> i32 {
    hash_range(codes, 0, codes.len())
}

/// hash of a range of code units (case sensitive). same value as [`hash_string`] for the same code units
#[inline]
pub fn hash_range<T: Unit>(codes: &[T], start: usize, end: usize) -> i32 {
    let mut hash = FNV_OFFSET;
    for &code in &codes[start..end] {
        hash = (hash ^ code.into()).wrapping_mul(FNV_PRIME);
    }
    (hash & HASH_MASK) as i32
}

/// lowercase ascii code of a code unit: the ascii uppercase letters and the kelvin sign are lowercased. -1 for the
/// other non ascii code units
#[inline]
pub fn lower_ascii(code: u32) -> i32 {
    if (0x41..=0x5a).contains(&code) {
        return (code + 32) as i32;
    }
    if code > 0x7f {
        return if code == KELVIN_SIGN { 0x6b } else { -1 };
    }
    code as i32
}

/// hash of a range of code units, ignoring the case. -1 when the range contains a non ascii character that cannot
/// match an ascii key
pub fn hash_range_lower<T: Unit>(codes: &[T], start: usize, end: usize) -> i32 {
    let mut hash = FNV_OFFSET;
    for &code in &codes[start..end] {
        let code = lower_ascii(code.into());
        if code == -1 {
            return -1;
        }
        hash = (hash ^ code as u32).wrapping_mul(FNV_PRIME);
    }
    (hash & HASH_MASK) as i32
}

/// hash of a string ignoring the case, see [`hash_range_lower`]
pub fn hash_string_lower(str: &str) -> i32 {
    let mut hash = FNV_OFFSET;
    for code in str.encode_utf16() {
        let code = lower_ascii(code as u32);
        if code == -1 {
            return -1;
        }
        hash = (hash ^ code as u32).wrapping_mul(FNV_PRIME);
    }
    (hash & HASH_MASK) as i32
}
