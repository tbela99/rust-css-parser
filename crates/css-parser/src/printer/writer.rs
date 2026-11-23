//! Port of `src/printer/writer.ts`.
//!
//! Growable buffer the rendered css is written to, encoded as UTF-8. The bindings expose the written bytes to
//! javascript as a view (no copy) and decode them lazily.

use std::cell::Cell;

thread_local! {
    static WRITER_ID: Cell<u32> = const { Cell::new(0) };
}

/// a new writer id: a positive 31 bits integer
fn next_writer_id() -> u32 {
    WRITER_ID.with(|w| {
        let id = (w.get() % 0x7ffffffe) + 1;
        w.set(id);
        id
    })
}

/// default capacity of a writer, in bytes
pub const DEFAULT_CAPACITY: usize = 1 << 12;

/// growable utf-8 buffer
#[derive(Debug)]
pub struct Writer {
    /// the written bytes
    pub buffer: Vec<u8>,
    /// unique id of the content of the writer: it changes when the writer is reset
    pub id: u32,
    /// number of bytes written minus the number of UTF-16 code units of the written strings
    pub extra: i64,
}

impl Default for Writer {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

impl Writer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: Vec::with_capacity(if capacity > 0 { capacity } else { DEFAULT_CAPACITY }),
            id: next_writer_id(),
            extra: 0,
        }
    }

    /// number of bytes written
    #[inline]
    pub fn length(&self) -> usize {
        self.buffer.len()
    }

    /// write a string
    #[inline]
    pub fn write(&mut self, str: &str) {
        self.buffer.extend_from_slice(str.as_bytes());
        if !str.is_ascii() {
            self.extra += str.len() as i64 - str.encode_utf16().count() as i64;
        }
    }

    /// write an ascii character
    #[inline]
    pub fn write_byte(&mut self, code: u8) {
        self.buffer.push(code);
    }

    /// copy bytes already written to the end of the buffer
    pub fn copy(&mut self, start: usize, end: usize, extra: i64) {
        self.buffer.extend_from_within(start..end);
        self.extra += extra;
    }

    /// length of the written css once decoded, in UTF-16 code units
    pub fn code_length(&self) -> usize {
        (self.buffer.len() as i64 - self.extra) as usize
    }

    /// drop the written bytes: the writer is reused. it gets a new id
    pub fn reset(&mut self) {
        self.id = next_writer_id();
        self.buffer.clear();
        self.extra = 0;
    }

    /// drop the bytes written after a position
    pub fn truncate(&mut self, length: usize, extra: i64) {
        self.buffer.truncate(length);
        self.extra = extra;
    }

    /// last byte written, -1 when nothing is written
    pub fn last_byte(&self) -> i32 {
        self.buffer.last().map_or(-1, |&b| b as i32)
    }

    /// make room for `size` more bytes
    pub fn reserve(&mut self, size: usize) {
        self.buffer.reserve(size);
    }

    /// the written bytes
    pub fn bytes(&self) -> &[u8] {
        &self.buffer
    }

    /// the written css
    pub fn as_str(&self) -> &str {
        // only strings and ascii bytes are written
        std::str::from_utf8(&self.buffer).unwrap_or_default()
    }

    /// take the written bytes
    pub fn into_bytes(self) -> Vec<u8> {
        self.buffer
    }
}
