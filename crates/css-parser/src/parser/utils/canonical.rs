//! Port of `src/parser/utils/canonical.ts`.
//!
//! Canonical strings: the spellings of the known units and function names (`dB`, `translateX`, ...). They are
//! registered once and stored in a single text: the string table references ranges of this text, no string is created.

use std::cell::RefCell;
use crate::utils::fx::HashMap;

#[derive(Default)]
struct Canonical {
    /// offset of the registered strings in the text
    offsets: HashMap<String, u32>,
    /// registered strings, concatenated (the canonical strings are ascii: one code unit per byte)
    text: String,
}

thread_local! {
    static CANONICAL: RefCell<Canonical> = RefCell::new(Canonical::default());
}

/// register a canonical string
/// returns the range of the string in the canonical text, packed: `start << 8 | length`
pub fn register_canonical(str: &str) -> u32 {
    debug_assert!(str.is_ascii() && str.len() < 256);
    CANONICAL.with(|c| {
        let mut c = c.borrow_mut();
        let start = match c.offsets.get(str) {
            Some(&start) => start,
            None => {
                let start = c.text.len() as u32;
                c.offsets.insert(str.to_owned(), start);
                c.text.push_str(str);
                start
            }
        };
        (start << 8) | str.len() as u32
    })
}

/// start of a packed range
#[inline]
pub fn canonical_start(range: u32) -> u32 {
    range >> 8
}

/// end of a packed range
#[inline]
pub fn canonical_end(range: u32) -> u32 {
    (range >> 8) + (range & 0xff)
}

/// run a function with the canonical text (ascii: the bytes are the code units)
pub fn with_canonical_text<R>(f: impl FnOnce(&str) -> R) -> R {
    CANONICAL.with(|c| f(&c.borrow().text))
}

/// a range of the canonical text
pub fn canonical_slice(start: u32, end: u32) -> String {
    with_canonical_text(|t| t[start as usize..end as usize].to_owned())
}

/// code unit of the canonical text
pub fn canonical_code_at(offset: u32) -> u32 {
    with_canonical_text(|t| t.as_bytes().get(offset as usize).map_or(u32::MAX, |&b| b as u32))
}
