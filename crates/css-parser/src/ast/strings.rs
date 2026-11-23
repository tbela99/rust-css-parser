//! Port of `src/ast/strings.ts`.
//!
//! String table of an arena. An interned string is a range of a text:
//!
//! - a range of the code units of a source file: the values of the tokens read by the tokenizer. the table stores
//!   the source index and the start and end offsets, no string is created. the code units buffer is shared with the
//!   tokenizer (reference counted): it is not copied
//! - a range of the canonical text: the spelling of the known units and function names
//! - a range of the pool: the code units of the strings that are not a range of a source
//! - a string: the strings interned by [`StringTable::intern`]
//!
//! Strings are created when they are read ([`StringTable::get`]), once per id, as shared `Rc<str>`.

use crate::parser::utils::canonical::{canonical_code_at, with_canonical_text};
use crate::parser::utils::codes::{hash_range, hash_string, CodeUnits};
use std::any::Any;
use crate::utils::fx::HashMap;
use std::rc::Rc;

/// where the code units of an interned string are stored. a value that is not negative is the index of a source
#[allow(non_snake_case)]
pub mod StringSource {
    /// the code units are copied to the pool
    pub const Pool: i32 = -1;
    /// range of the canonical text
    pub const Canonical: i32 = -2;
    /// a string
    pub const Value: i32 = -3;
}

/// boolean properties of the strings computed once per string id. 4 flags at most
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StringFlag {
    /// a named color, a system color or `currentcolor` / `transparent`, ignoring the case
    Color = 0,
    /// a rule selector that is not modified by the minifier
    SimpleSelector = 1,
    /// a rule selector that contains a vendor prefixed pseudo class
    VendorPrefix = 2,
}

/// values derived from the strings computed once per string id
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum StringDerived {
    FunctionSyntax = 0,
    SelectorFunctionSyntax = 1,
    TransformKind = 2,
}

/// copy of a string table, see [`StringTable::export`]
#[derive(Clone, Debug, Default)]
pub struct StringTableData {
    pub count: u32,
    pub hashes: Vec<i32>,
    pub sources: Vec<i32>,
    pub starts: Vec<i32>,
    pub ends: Vec<i32>,
    /// code units of each source, by source index
    pub texts: Vec<Option<CodeUnits>>,
    pub pool: Vec<u16>,
    /// the strings of the `StringSource::Value` entries, by id
    pub values: Vec<Option<Rc<str>>>,
}

/// the code units of a string, borrowed (see [`StringTable::with_units`]): ascii bytes or utf-16 code units
#[derive(Clone, Copy, Debug)]
pub enum Units<'a> {
    Bytes(&'a [u8]),
    Utf16(&'a [u16]),
}

/// builds a string from code units, strings and interned strings, then interns it: no string is created
#[derive(Default)]
pub struct StringBuilder {
    pub codes: Vec<u16>,
}

impl StringBuilder {
    pub fn new() -> Self {
        Self { codes: Vec::with_capacity(256) }
    }

    /// drop the code units
    pub fn reset(&mut self) -> &mut Self {
        self.codes.clear();
        self
    }

    pub fn len(&self) -> usize {
        self.codes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.codes.is_empty()
    }

    /// append the code units of an interned string
    pub fn append_id(&mut self, table: &StringTable, id: u32) -> &mut Self {
        table.copy_into(id, &mut self.codes);
        self
    }

    /// append a string
    pub fn append_string(&mut self, str: &str) -> &mut Self {
        self.codes.extend(str.encode_utf16());
        self
    }

    /// append a code unit
    pub fn append_code(&mut self, code: u16) -> &mut Self {
        self.codes.push(code);
        self
    }

    /// intern the string
    pub fn intern(&self, table: &mut StringTable) -> u32 {
        table.intern_units(&self.codes)
    }
}

/// a string shared without a copy: a shared string, or a range of the bytes of an ascii source (the source buffer is
/// shared). it derefs to `&str`, a clone increments a reference count
#[derive(Clone)]
pub struct SharedStr(SharedRepr);

#[derive(Clone)]
enum SharedRepr {
    Shared(Rc<str>),
    /// a static string (the configuration): no copy, no reference count
    Static(&'static str),
    /// a range of an ascii buffer: valid utf-8 at any boundary
    Ascii { buffer: Rc<[u8]>, start: u32, end: u32 },
}

impl SharedStr {
    /// a range of an ascii buffer, without a copy
    pub fn ascii(buffer: &Rc<[u8]>, start: usize, end: usize) -> Self {
        debug_assert!(buffer[start..end].is_ascii());
        SharedStr(SharedRepr::Ascii { buffer: buffer.clone(), start: start as u32, end: end as u32 })
    }

    pub fn as_str(&self) -> &str {
        self
    }

    /// a static string, without a copy
    pub fn from_static(s: &'static str) -> Self {
        SharedStr(SharedRepr::Static(s))
    }
}

impl std::ops::Deref for SharedStr {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        match &self.0 {
            SharedRepr::Shared(s) => s,
            SharedRepr::Static(s) => s,
            // SAFETY: the buffer is ascii (see `SharedStr::ascii()`)
            SharedRepr::Ascii { buffer, start, end } => unsafe { std::str::from_utf8_unchecked(&buffer[*start as usize..*end as usize]) },
        }
    }
}

impl AsRef<str> for SharedStr {
    fn as_ref(&self) -> &str {
        self
    }
}

impl std::borrow::Borrow<str> for SharedStr {
    fn borrow(&self) -> &str {
        self
    }
}

impl Default for SharedStr {
    fn default() -> Self {
        SharedStr(SharedRepr::Shared(Rc::from("")))
    }
}

impl From<Rc<str>> for SharedStr {
    fn from(s: Rc<str>) -> Self {
        SharedStr(SharedRepr::Shared(s))
    }
}

impl From<&str> for SharedStr {
    fn from(s: &str) -> Self {
        SharedStr(SharedRepr::Shared(Rc::from(s)))
    }
}

impl From<String> for SharedStr {
    fn from(s: String) -> Self {
        SharedStr(SharedRepr::Shared(Rc::from(s)))
    }
}

impl From<SharedStr> for Rc<str> {
    /// a shared string is returned as is, a range is copied
    fn from(s: SharedStr) -> Self {
        match s.0 {
            SharedRepr::Shared(s) => s,
            SharedRepr::Static(_) | SharedRepr::Ascii { .. } => Rc::from(&*s),
        }
    }
}

impl std::fmt::Display for SharedStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self)
    }
}

impl std::fmt::Debug for SharedStr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}

impl PartialEq for SharedStr {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for SharedStr {}

impl PartialEq<str> for SharedStr {
    fn eq(&self, other: &str) -> bool {
        &**self == other
    }
}

impl PartialEq<&str> for SharedStr {
    fn eq(&self, other: &&str) -> bool {
        &**self == *other
    }
}

impl std::hash::Hash for SharedStr {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (**self).hash(state)
    }
}

impl PartialOrd for SharedStr {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SharedStr {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (**self).cmp(&**other)
    }
}

/// interned strings of an arena
pub struct StringTable {
    /// number of strings
    pub count: u32,
    /// strings by id: the strings that have been read and the `StringSource::Value` strings
    cache: Vec<Option<SharedStr>>,
    /// utf-16 code units of the non ascii `StringSource::Value` strings
    value_units: HashMap<u32, Rc<[u16]>>,
    /// identity of the ids: changed when the ids change (see [`StringTable::compact`])
    pub epoch: u64,
    hashes: Vec<i32>,
    sources: Vec<i32>,
    starts: Vec<i32>,
    ends: Vec<i32>,
    /// hash table: string id + 1 by slot, 0 when the slot is empty (open addressing, linear probing)
    slots: Vec<i32>,
    /// code units of the sources, by source index
    texts: Vec<Option<CodeUnits>>,
    derived_values: Vec<Option<HashMap<u32, Box<dyn Any>>>>,
    flags: Vec<u8>,
    /// id of the lowercase string + 1, by id
    lowers: Vec<i32>,
    builder: StringBuilder,
    /// code units of the strings that are not a range of a source
    pool: Vec<u16>,
}

thread_local! {
    static EPOCH: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

fn next_epoch() -> u64 {
    EPOCH.with(|e| {
        let v = e.get();
        e.set(v + 1);
        v
    })
}

impl Default for StringTable {
    fn default() -> Self {
        Self::new()
    }
}

impl StringTable {
    pub fn new() -> Self {
        Self {
            count: 0,
            cache: Vec::new(),
            value_units: HashMap::default(),
            epoch: next_epoch(),
            hashes: vec![0; 256],
            sources: vec![0; 256],
            starts: vec![0; 256],
            ends: vec![0; 256],
            slots: vec![0; 512],
            texts: Vec::new(),
            derived_values: Vec::new(),
            flags: Vec::new(),
            lowers: Vec::new(),
            builder: StringBuilder::new(),
            pool: Vec::with_capacity(1024),
        }
    }

    /// set the code units of a source: the strings of the source are ranges of it. the buffer is shared, not copied
    pub fn set_text(&mut self, source: usize, codes: CodeUnits) {
        if self.texts.len() <= source {
            self.texts.resize(source + 1, None);
        }
        self.texts[source] = Some(codes);
    }

    /// kept for parity with typescript: the code units are the text, there is nothing to release
    pub fn release_codes(&mut self, _source: usize) {}

    /// code units of a source
    pub fn text(&self, source: usize) -> Option<&CodeUnits> {
        self.texts.get(source).and_then(|t| t.as_ref())
    }

    #[inline]
    fn find<F: Fn(&Self, u32) -> bool>(&self, hash: i32, eq: F) -> Result<u32, usize> {
        let mask = self.slots.len() - 1;
        let mut slot = (hash as usize) & mask;
        loop {
            let entry = self.slots[slot];
            if entry == 0 {
                return Err(slot);
            }
            let id = (entry - 1) as u32;
            if self.hashes[id as usize] == hash && eq(self, id) {
                return Ok(id);
            }
            slot = (slot + 1) & mask;
        }
    }

    /// intern a string
    pub fn intern(&mut self, str: &str) -> u32 {
        let hash = hash_string(str);
        match self.find(hash, |t, id| t.equals_string(id, str)) {
            Ok(id) => id,
            Err(slot) => {
                let len = if str.is_ascii() { str.len() } else { str.encode_utf16().count() };
                let id = self.add(hash, slot, StringSource::Value, 0, len as i32);
                if !str.is_ascii() {
                    self.value_units.insert(id, str.encode_utf16().collect::<Vec<u16>>().into());
                }
                self.set_cache(id, Rc::from(str));
                id
            }
        }
    }

    /// intern a shared string, without copying it
    /// intern a shared string: a shared `Rc<str>` is kept as is, a range of a source is copied
    pub fn intern_shared(&mut self, str: &SharedStr) -> u32 {
        match &str.0 {
            SharedRepr::Shared(rc) => self.intern_rc(rc),
            SharedRepr::Static(_) | SharedRepr::Ascii { .. } => self.intern(str),
        }
    }

    pub fn intern_rc(&mut self, str: &Rc<str>) -> u32 {
        let hash = hash_string(str);
        match self.find(hash, |t, id| t.equals_string(id, str)) {
            Ok(id) => id,
            Err(slot) => {
                let len = if str.is_ascii() { str.len() } else { str.encode_utf16().count() };
                let id = self.add(hash, slot, StringSource::Value, 0, len as i32);
                if !str.is_ascii() {
                    self.value_units.insert(id, str.encode_utf16().collect::<Vec<u16>>().into());
                }
                self.set_cache(id, str.clone());
                id
            }
        }
    }

    /// intern utf-16 code units (copied to the pool when they are not interned yet)
    pub fn intern_units(&mut self, codes: &[u16]) -> u32 {
        let hash = hash_range(codes, 0, codes.len());
        match self.find(hash, |t, id| t.equals_units(id, codes)) {
            Ok(id) => id,
            Err(slot) => {
                let offset = self.pool.len() as i32;
                self.pool.extend_from_slice(codes);
                self.add(hash, slot, StringSource::Pool, offset, offset + codes.len() as i32)
            }
        }
    }

    /// intern a range of the code units of a source (`source >= 0`, the range is kept) or of the canonical text
    /// (`StringSource::Canonical`, `codes` is the canonical text). use [`StringTable::intern_units`] for the pool
    pub fn intern_range(&mut self, codes: &CodeUnits, start: usize, end: usize, source: i32) -> u32 {
        let hash = codes.hash_range(start, end);
        match self.find(hash, |t, id| t.equals_code_range(id, codes, start, end)) {
            Ok(id) => id,
            Err(slot) => {
                if source == StringSource::Pool {
                    let offset = self.pool.len() as i32;
                    for i in start..end {
                        self.pool.push(codes.at(i) as u16);
                    }
                    self.add(hash, slot, StringSource::Pool, offset, offset + (end - start) as i32)
                } else {
                    self.add(hash, slot, source, start as i32, end as i32)
                }
            }
        }
    }

    /// intern a range of the canonical text
    pub fn intern_canonical(&mut self, start: u32, end: u32) -> u32 {
        let (hash, found) = with_canonical_text(|text| {
            let bytes = &text.as_bytes()[start as usize..end as usize];
            let hash = hash_range(bytes, 0, bytes.len());
            (hash, self.find(hash, |t, id| t.equals_bytes(id, bytes)))
        });
        match found {
            Ok(id) => id,
            Err(slot) => self.add(hash, slot, StringSource::Canonical, start as i32, end as i32),
        }
    }

    /// id of a string, -1 when the string is not interned
    pub fn lookup(&self, str: &str) -> i32 {
        match self.find(hash_string(str), |t, id| t.equals_string(id, str)) {
            Ok(id) => id as i32,
            Err(_) => -1,
        }
    }

    fn set_cache(&mut self, id: u32, value: impl Into<SharedStr>) {
        let value = value.into();
        let id = id as usize;
        if self.cache.len() <= id {
            self.cache.resize(id + 1, None);
        }
        self.cache[id] = Some(value);
    }

    /// the string of an id. the string is created the first time it is read
    pub fn get(&mut self, id: u32) -> SharedStr {
        if let Some(Some(str)) = self.cache.get(id as usize) {
            return str.clone();
        }
        if id >= self.count {
            return SharedStr::default();
        }
        let i = id as usize;
        let (start, end) = (self.starts[i] as usize, self.ends[i] as usize);
        // an ascii range of a source shares the source buffer: no copy
        let str: SharedStr = match (self.sources[i], self.texts.get(self.sources[i].max(0) as usize)) {
            (source, Some(Some(CodeUnits::Bytes(bytes)))) if source >= 0 => SharedStr::ascii(bytes, start, end),
            _ => SharedStr::from(self.peek(id)),
        };
        self.set_cache(id, str.clone());
        str
    }

    /// the string of an id, without caching it
    pub fn peek(&self, id: u32) -> String {
        if let Some(Some(str)) = self.cache.get(id as usize) {
            return str.to_string();
        }
        if id >= self.count {
            return String::new();
        }
        let i = id as usize;
        let (start, end) = (self.starts[i] as usize, self.ends[i] as usize);
        match self.sources[i] {
            StringSource::Pool => String::from_utf16_lossy(&self.pool[start..end]),
            StringSource::Canonical => with_canonical_text(|t| t[start..end].to_owned()),
            source => self.texts[source as usize].as_ref().map_or(String::new(), |c| c.slice(start, end).into_owned()),
        }
    }

    /// run a function with the string of an id, without creating it when it is an ascii range of a source
    pub fn with_str<R>(&self, id: u32, f: impl FnOnce(&str) -> R) -> R {
        if let Some(Some(str)) = self.cache.get(id as usize) {
            return f(str);
        }
        if id < self.count {
            let i = id as usize;
            let source = self.sources[i];
            if source >= 0 {
                if let Some(CodeUnits::Bytes(bytes)) = self.texts.get(source as usize).and_then(|t| t.as_ref()) {
                    // ascii: valid utf-8
                    return f(unsafe { std::str::from_utf8_unchecked(&bytes[self.starts[i] as usize..self.ends[i] as usize]) });
                }
            } else if source == StringSource::Canonical {
                let (s, e) = (self.starts[i] as usize, self.ends[i] as usize);
                return with_canonical_text(|t| f(&t[s..e]));
            }
        }
        f(&self.peek(id))
    }

    /// length of a string, in utf-16 code units
    #[inline]
    pub fn length(&self, id: u32) -> usize {
        if id >= self.count {
            return 0;
        }
        (self.ends[id as usize] - self.starts[id as usize]) as usize
    }

    /// utf-16 code unit of a string
    pub fn code_at(&self, id: u32, index: usize) -> u32 {
        let i = id as usize;
        let source = self.sources[i];
        let offset = self.starts[i] as usize + index;
        if source >= 0 {
            return self.texts[source as usize].as_ref().map_or(u32::MAX, |c| c.at(offset));
        }
        match source {
            StringSource::Pool => self.pool.get(offset).map_or(u32::MAX, |&c| c as u32),
            StringSource::Canonical => canonical_code_at(offset as u32),
            _ => {
                if let Some(units) = self.value_units.get(&id) {
                    units.get(index).map_or(u32::MAX, |&c| c as u32)
                } else {
                    self.cache[i].as_ref().and_then(|s| s.as_bytes().get(index)).map_or(u32::MAX, |&b| b as u32)
                }
            }
        }
    }

    /// the code units of a string, borrowed from where they are stored (the source text, the pool, the canonical text,
    /// the value): no copy
    pub fn with_units<R>(&self, id: u32, f: impl FnOnce(Units<'_>) -> R) -> R {
        if id >= self.count {
            return f(Units::Bytes(&[]));
        }
        let i = id as usize;
        let (start, end) = (self.starts[i] as usize, self.ends[i] as usize);
        match self.sources[i] {
            StringSource::Pool => f(Units::Utf16(&self.pool[start..end])),
            StringSource::Canonical => with_canonical_text(|t| f(Units::Bytes(&t.as_bytes()[start..end]))),
            StringSource::Value => match self.value_units.get(&id) {
                Some(units) => f(Units::Utf16(units)),
                None => f(Units::Bytes(self.cache.get(i).and_then(|c| c.as_ref()).map_or(&[][..], |s| s.as_bytes()))),
            },
            source => match self.texts.get(source as usize).and_then(|t| t.as_ref()) {
                Some(CodeUnits::Bytes(b)) => f(Units::Bytes(&b[start..end])),
                Some(CodeUnits::Utf16(u)) => f(Units::Utf16(&u[start..end])),
                None => f(Units::Bytes(&[])),
            },
        }
    }

    /// append the code units of a string to a buffer. the string is not created
    pub fn copy_into(&self, id: u32, target: &mut Vec<u16>) -> usize {
        let length = self.length(id);
        target.reserve(length);
        let i = id as usize;
        let start = self.starts[i] as usize;
        match self.sources[i] {
            StringSource::Pool => target.extend_from_slice(&self.pool[start..start + length]),
            StringSource::Canonical => with_canonical_text(|t| target.extend(t.as_bytes()[start..start + length].iter().map(|&b| b as u16))),
            StringSource::Value => {
                if let Some(units) = self.value_units.get(&id) {
                    target.extend_from_slice(units);
                } else if let Some(Some(s)) = self.cache.get(i) {
                    target.extend(s.bytes().map(|b| b as u16));
                }
            }
            source => match self.texts[source as usize].as_ref() {
                Some(CodeUnits::Bytes(b)) => target.extend(b[start..start + length].iter().map(|&b| b as u16)),
                Some(CodeUnits::Utf16(u)) => target.extend_from_slice(&u[start..start + length]),
                None => {}
            },
        }
        length
    }

    /// a boolean property of a string, computed once per id from the string
    pub fn flag(&mut self, id: u32, flag: StringFlag, compute: impl FnOnce(&str) -> bool) -> bool {
        if let Some(value) = self.cached_flag(id, flag) {
            return value;
        }
        let value = self.with_str(id, compute);
        self.set_flag(id, flag, value);
        value
    }

    /// the string of an id when it has been created (`cache[id]`)
    pub fn cached(&self, id: u32) -> Option<SharedStr> {
        self.cache.get(id as usize).cloned().flatten()
    }

    /// cached value of a flag, if it is computed
    pub fn cached_flag(&self, id: u32, flag: StringFlag) -> Option<bool> {
        let value = (self.flags.get(id as usize).copied().unwrap_or(0) >> (flag as u8 * 2)) & 3;
        if value == 0 {
            None
        } else {
            Some(value == 3)
        }
    }

    /// store the value of a flag
    pub fn set_flag(&mut self, id: u32, flag: StringFlag, value: bool) {
        if id as usize >= self.flags.len() {
            let size = (self.count.max(id + 1) * 2) as usize;
            self.flags.resize(size, 0);
        }
        self.flags[id as usize] |= (if value { 3 } else { 1 }) << (flag as u8 * 2);
    }

    /// a value derived from a string, if it is computed
    pub fn derived<T: Clone + 'static>(&self, id: u32, slot: StringDerived) -> Option<T> {
        self.derived_values
            .get(slot as usize)
            .and_then(|m| m.as_ref())
            .and_then(|m| m.get(&id))
            .and_then(|v| v.downcast_ref::<T>())
            .cloned()
    }

    /// store a value derived from a string
    pub fn set_derived<T: 'static>(&mut self, id: u32, slot: StringDerived, value: T) {
        let slot = slot as usize;
        if self.derived_values.len() <= slot {
            self.derived_values.resize_with(slot + 1, || None);
        }
        self.derived_values[slot].get_or_insert_with(HashMap::default).insert(id, Box::new(value));
    }

    /// append the bytes of a string to a buffer when it is ascii. returns the number of bytes, -1 when the string is
    /// not ascii (nothing is appended)
    pub fn copy_ascii(&self, id: u32, target: &mut Vec<u8>) -> isize {
        self.with_units(id, |units| match units {
            // a byte slice is ascii (the non ascii texts are stored as utf-16 code units)
            Units::Bytes(bytes) => {
                target.extend_from_slice(bytes);
                bytes.len() as isize
            }
            Units::Utf16(units) => {
                if units.iter().any(|&u| u > 0x7f) {
                    return -1;
                }
                target.extend(units.iter().map(|&u| u as u8));
                units.len() as isize
            }
        })
    }

    /// index of the first occurrence of a code unit, -1 when it is not found
    pub fn index_of_code(&self, id: u32, code: u32) -> isize {
        self.with_units(id, |units| match units {
            Units::Bytes(bytes) => {
                if code > 0x7f {
                    return -1;
                }
                bytes.iter().position(|&b| b as u32 == code).map_or(-1, |i| i as isize)
            }
            Units::Utf16(units) => units.iter().position(|&u| u as u32 == code).map_or(-1, |i| i as isize),
        })
    }

    /// true if the string of an id equals a string. the string is not created
    pub fn equals(&self, id: u32, str: &str) -> bool {
        if id >= self.count {
            return false;
        }
        self.equals_string(id, str)
    }

    /// id of the lowercase string: the id itself when the string has no uppercase letter
    pub fn lower(&mut self, id: u32) -> u32 {
        if let Some(&l) = self.lowers.get(id as usize) {
            if l != 0 {
                return (l - 1) as u32;
            }
        }
        let length = self.length(id);
        let mut result = id;
        for i in 0..length {
            let code = self.code_at(id, i);
            if code > 0x7f {
                let str = self.get(id);
                let lower = str.to_lowercase();
                result = if *lower == *str { id } else { self.intern(&lower) };
                break;
            }
            if (0x41..=0x5a).contains(&code) {
                let mut builder = std::mem::take(&mut self.builder);
                builder.reset().append_id(self, id);
                for c in builder.codes[i..].iter_mut() {
                    if (0x41..=0x5a).contains(c) {
                        *c += 32;
                    }
                }
                result = builder.intern(self);
                self.builder = builder;
                break;
            }
        }
        if id as usize >= self.lowers.len() {
            self.lowers.resize((self.count.max(id + 1) * 2) as usize, 0);
        }
        self.lowers[id as usize] = result as i32 + 1;
        result
    }

    /// true if the string of an id starts with a string
    pub fn starts_with(&self, id: u32, str: &str) -> bool {
        if id >= self.count || self.length(id) < str.len() {
            return false;
        }
        self.with_units(id, |units| match units {
            Units::Bytes(bytes) => bytes.starts_with(str.as_bytes()),
            Units::Utf16(units) => str.encode_utf16().zip(units.iter()).all(|(c, &u)| c == u) && units.len() >= str.encode_utf16().count(),
        })
    }

    /// true if the string of an id equals a string, ignoring the case of the ascii letters
    pub fn equals_ignore_case(&self, id: u32, str: &str) -> bool {
        if id >= self.count {
            return false;
        }
        let lower = |c: u32| if (65..=90).contains(&c) { c + 32 } else { c };
        self.with_units(id, |units| match units {
            Units::Bytes(bytes) => bytes.eq_ignore_ascii_case(str.as_bytes()),
            Units::Utf16(units) => units.len() == str.encode_utf16().count() && str.encode_utf16().zip(units.iter()).all(|(c, &u)| lower(c as u32) == lower(u as u32)),
        })
    }

    /// drop the strings that are not used and renumber the others. returns the new id of each string
    pub fn compact(&mut self, used: &[u8]) -> Vec<u32> {
        let previous_count = self.count as usize;
        let mut mapping = vec![0u32; previous_count];
        let mut count = 0u32;
        for id in 0..previous_count {
            if used.get(id).copied().unwrap_or(0) != 0 {
                mapping[id] = count;
                count += 1;
            }
        }
        let hashes = std::mem::take(&mut self.hashes);
        let sources = std::mem::take(&mut self.sources);
        let starts = std::mem::take(&mut self.starts);
        let ends = std::mem::take(&mut self.ends);
        let cache = std::mem::take(&mut self.cache);
        let mut value_units = std::mem::take(&mut self.value_units);
        let previous_pool = std::mem::take(&mut self.pool);
        let mut pool = Vec::with_capacity(previous_pool.len().max(1024));
        self.reset(count);
        for id in 0..previous_count {
            if used.get(id).copied().unwrap_or(0) == 0 {
                continue;
            }
            let target = mapping[id] as usize;
            let (mut start, mut end) = (starts[id], ends[id]);
            if sources[id] == StringSource::Pool {
                let offset = pool.len() as i32;
                pool.extend_from_slice(&previous_pool[start as usize..end as usize]);
                end = offset + end - start;
                start = offset;
            }
            self.hashes[target] = hashes[id];
            self.sources[target] = sources[id];
            self.starts[target] = start;
            self.ends[target] = end;
            if let Some(Some(c)) = cache.get(id) {
                self.set_cache(target as u32, c.clone());
            }
            if let Some(units) = value_units.remove(&(id as u32)) {
                self.value_units.insert(target as u32, units);
            }
        }
        self.pool = pool;
        self.rehash();
        mapping
    }

    /// copy of the table
    pub fn export(&self) -> StringTableData {
        let count = self.count as usize;
        StringTableData {
            count: self.count,
            hashes: self.hashes[..count].to_vec(),
            sources: self.sources[..count].to_vec(),
            starts: self.starts[..count].to_vec(),
            ends: self.ends[..count].to_vec(),
            texts: self.texts.clone(),
            pool: self.pool.clone(),
            values: (0..count)
                .map(|id| if self.sources[id] == StringSource::Value { self.cache.get(id).cloned().flatten().map(Rc::from) } else { None })
                .collect(),
        }
    }

    /// initialize the table from an exported copy
    pub fn import(&mut self, data: &StringTableData) {
        let count = data.count as usize;
        self.reset(data.count);
        self.hashes[..count].copy_from_slice(&data.hashes[..count]);
        self.sources[..count].copy_from_slice(&data.sources[..count]);
        self.starts[..count].copy_from_slice(&data.starts[..count]);
        self.ends[..count].copy_from_slice(&data.ends[..count]);
        self.texts = data.texts.clone();
        self.pool = data.pool.clone();
        for id in 0..count {
            if data.sources[id] == StringSource::Value {
                if let Some(Some(v)) = data.values.get(id) {
                    if !v.is_ascii() {
                        self.value_units.insert(id as u32, v.encode_utf16().collect::<Vec<u16>>().into());
                    }
                    self.set_cache(id as u32, v.clone());
                }
            }
        }
        self.rehash();
    }

    fn add(&mut self, hash: i32, slot: usize, source: i32, start: i32, end: i32) -> u32 {
        let id = self.count as usize;
        self.count += 1;
        if id == self.hashes.len() {
            self.grow(id * 2);
        }
        self.hashes[id] = hash;
        self.sources[id] = source;
        self.starts[id] = start;
        self.ends[id] = end;
        self.slots[slot] = id as i32 + 1;
        if self.count as usize * 2 > self.slots.len() {
            self.rehash();
        }
        id as u32
    }

    fn equals_units(&self, id: u32, codes: &[u16]) -> bool {
        self.length(id) == codes.len() && codes.iter().enumerate().all(|(i, &c)| self.code_at(id, i) == c as u32)
    }

    fn equals_bytes(&self, id: u32, codes: &[u8]) -> bool {
        self.length(id) == codes.len() && codes.iter().enumerate().all(|(i, &c)| self.code_at(id, i) == c as u32)
    }

    fn equals_code_range(&self, id: u32, codes: &CodeUnits, start: usize, end: usize) -> bool {
        let length = end - start;
        if self.length(id) != length {
            return false;
        }
        let i = id as usize;
        // same buffer, same range
        let source = self.sources[i];
        if source >= 0 && self.starts[i] as usize == start {
            if let Some(text) = self.texts[source as usize].as_ref() {
                let same = match (text, codes) {
                    (CodeUnits::Bytes(a), CodeUnits::Bytes(b)) => Rc::ptr_eq(a, b),
                    (CodeUnits::Utf16(a), CodeUnits::Utf16(b)) => Rc::ptr_eq(a, b),
                    _ => false,
                };
                if same {
                    return true;
                }
            }
        }
        // the units of the string are borrowed once and compared as slices (a memcmp when the widths match)
        self.with_units(id, |units| match (units, codes) {
            (Units::Bytes(a), CodeUnits::Bytes(b)) => a == &b[start..end],
            (Units::Utf16(a), CodeUnits::Utf16(b)) => a == &b[start..end],
            (Units::Bytes(a), CodeUnits::Utf16(b)) => a.iter().zip(&b[start..end]).all(|(&x, &y)| x as u16 == y),
            (Units::Utf16(a), CodeUnits::Bytes(b)) => a.iter().zip(&b[start..end]).all(|(&x, &y)| x == y as u16),
        })
    }

    fn equals_string(&self, id: u32, str: &str) -> bool {
        let i = id as usize;
        if let Some(Some(cached)) = self.cache.get(i) {
            return **cached == *str;
        }
        if self.sources[i] == StringSource::Value {
            return false;
        }
        self.with_units(id, |units| match units {
            Units::Bytes(bytes) => bytes == str.as_bytes(),
            Units::Utf16(units) => units.len() == str.encode_utf16().count() && str.encode_utf16().zip(units.iter()).all(|(c, &u)| c == u),
        })
    }

    fn grow(&mut self, size: usize) {
        self.hashes.resize(size, 0);
        self.sources.resize(size, 0);
        self.starts.resize(size, 0);
        self.ends.resize(size, 0);
    }

    /// drop the strings, allocate the columns for `count` strings. the ids change
    fn reset(&mut self, count: u32) {
        let size = (count as usize).max(256);
        self.count = count;
        self.cache = Vec::new();
        self.value_units = HashMap::default();
        self.epoch = next_epoch();
        self.lowers = Vec::new();
        self.flags = Vec::new();
        self.derived_values = Vec::new();
        self.hashes = vec![0; size];
        self.sources = vec![0; size];
        self.starts = vec![0; size];
        self.ends = vec![0; size];
    }

    fn rehash(&mut self) {
        let mut size = 512;
        while size < self.count as usize * 4 {
            size *= 2;
        }
        let mut slots = vec![0i32; size];
        let mask = size - 1;
        for id in 0..self.count as usize {
            let mut slot = (self.hashes[id] as usize) & mask;
            while slots[slot] != 0 {
                slot = (slot + 1) & mask;
            }
            slots[slot] = id as i32 + 1;
        }
        self.slots = slots;
    }
}
