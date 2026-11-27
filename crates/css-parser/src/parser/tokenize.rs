//! Port of `src/parser/tokenize.ts`.
//!
//! The tokenizer reads the code units of the css source (ascii bytes or utf-16 code units, see
//! [`crate::parser::input`]): the values of the tokens are ranges of the code units, no string is created.

use crate::ast::arena::AstArena;
use crate::ast::strings::StringSource;
use crate::ast::types::{ColorType, EnumToken};
use crate::parser::input::{to_code_units, CssInput};
use crate::parser::source::SourceFile;
use crate::parser::utils::canonical::{canonical_end, canonical_start, register_canonical};
use crate::parser::utils::codes::{lower_ascii, CodeUnits};
use crate::syntax::constants::{
    validation_units, COLORS_FUNC, CONTAINER_FUNC, GRID_TEMPLATE_FUNC, IMAGE_FUNC, MATH_FUNCS, PSEUDO_ELEMENTS, SUPPORT_FUNC,
    TIMELINE_FUNC, TIMING_FUNC, TRANSFORM_FUNCTIONS, URL_FUNC, WHEN_ELSE_FUNC, WILD_CARD_FUNCS,
};
use crate::syntax::syntax::{
    is_digit, is_ident_codepoint, is_ident_start, is_letter, is_new_line, is_non_printable, is_white_space, ANGLE_UNITS,
    DIMENSION_UNITS, FLEX_UNITS, FREQUENCY_UNITS, RESOLUTION_UNITS, TIME_UNITS,
};
use crate::utils::fx::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

/// end of the stream (`undefined` in typescript)
const EOF: u32 = u32::MAX;

/// decode the escape sequences of a range of code units: a backslash followed by a new line, or by 1 to 6
/// hexadecimal digits, and an optional space
pub fn decode_escapes(codes: &CodeUnits, start: usize, end: usize, output: &mut Vec<u16>) {
    output.clear();
    let mut i = start;
    while i < end {
        let code = codes.at(i);
        if code != 0x5c || i + 1 >= end {
            output.push(code as u16);
            i += 1;
            continue;
        }
        let mut next = codes.at(i + 1);
        if matches!(next, 0xa | 0xb | 0xc | 0xd | 0x2028 | 0x2029) {
            i += 2;
        } else if is_hex_digit(next) {
            let mut codepoint: u32 = 0;
            let mut k = i + 1;
            while k < end && k < i + 7 && is_hex_digit({
                next = codes.at(k);
                next
            }) {
                codepoint = codepoint * 16 + if next <= 0x39 { next - 0x30 } else { (next | 0x20) - 0x57 };
                k += 1;
            }
            i = k;
            if codepoint == 0 || (0xd800..=0xdfff).contains(&codepoint) || codepoint > 0x10ffff {
                output.push(0xfffd);
            } else if codepoint > 0xffff {
                let c = codepoint - 0x10000;
                output.push((0xd800 + (c >> 10)) as u16);
                output.push((0xdc00 + (c & 0x3ff)) as u16);
            } else {
                output.push(codepoint as u16);
            }
        } else {
            // not an escape sequence: the backslash is kept, the next character is read again
            output.push(code as u16);
            i += 1;
            continue;
        }
        // an optional space ends the escape sequence
        if i < end && codes.at(i) == 0x20 {
            i += 1;
        }
    }
}

/// table of ascii keys, looked up by a range of code units ignoring the case. no string is created: the range is
/// lowercased into a stack buffer
struct HashTable<T> {
    map: HashMap<Box<[u8]>, T>,
}

impl<T> HashTable<T> {
    fn new() -> Self {
        Self { map: HashMap::default() }
    }

    fn get(&self, codes: &CodeUnits, start: usize, end: usize) -> Option<&T> {
        let mut buffer = [0u8; 64];
        let length = end.checked_sub(start)?;
        if length > buffer.len() {
            return None;
        }
        for (k, slot) in buffer[..length].iter_mut().enumerate() {
            let code = lower_ascii(codes.at(start + k));
            if code < 0 {
                return None;
            }
            *slot = code as u8;
        }
        self.map.get(&buffer[..length])
    }

    fn get_key(&self, key: &str) -> Option<&T> {
        self.map.get(key.to_ascii_lowercase().as_bytes())
    }

    fn get_key_mut(&mut self, key: &str) -> Option<&mut T> {
        self.map.get_mut(key.to_ascii_lowercase().as_bytes())
    }

    fn set(&mut self, key: &str, value: T) {
        self.map.insert(key.to_ascii_lowercase().into_bytes().into_boxed_slice(), value);
    }
}

/// symbols, units and function names: their token type. units and function names: their canonical spelling by
/// token type
struct Tables {
    symbols: HashTable<u8>,
    items: HashTable<Vec<(u8, u32)>>,
}

fn tables() -> &'static Tables {
    // the canonical strings are thread local: the tables are built once per thread
    thread_local! {
        static TABLES: &'static Tables = Box::leak(Box::new(build_tables()));
    }
    TABLES.with(|t| *t)
}

fn build_tables() -> Tables {
    // insertion ordered definitions: a later definition of the same key replaces the value, like a `Map`
    let mut definitions: Vec<(String, u8)> = Vec::new();
    let define = |key: String, typ: u8, defs: &mut Vec<(String, u8)>| {
        if let Some(entry) = defs.iter_mut().find(|(k, _)| *k == key) {
            entry.1 = typ;
        } else {
            defs.push((key, typ));
        }
    };
    let mut items: HashTable<Vec<(u8, u32)>> = HashTable::new();
    for (key, typ) in [
        ("+", EnumToken::Plus),
        ("=", EnumToken::DelimTokenType),
        ("|", EnumToken::PipeTokenType),
        ("||", EnumToken::ColumnCombinatorTokenType),
        ("|=", EnumToken::DashMatchTokenType),
        ("&", EnumToken::NestingSelectorTokenType),
        ("*", EnumToken::Star),
        ("*=", EnumToken::ContainMatchTokenType),
        ("~", EnumToken::Tilda),
        ("~=", EnumToken::IncludeMatchTokenType),
        ("^=", EnumToken::StartMatchTokenType),
        ("$=", EnumToken::EndMatchTokenType),
        (",", EnumToken::Comma),
        (":", EnumToken::ColonTokenType),
        ("::", EnumToken::DoubleColonTokenType),
        (";", EnumToken::SemiColonTokenType),
        ("(", EnumToken::StartParensTokenType),
        (")", EnumToken::EndParensTokenType),
        ("[", EnumToken::AttrStartTokenType),
        ("]", EnumToken::AttrEndTokenType),
        ("{", EnumToken::BlockStartTokenType),
        ("}", EnumToken::BlockEndTokenType),
        ("<=", EnumToken::LteTokenType),
        (">", EnumToken::GtTokenType),
        (">=", EnumToken::GteTokenType),
        (" ", EnumToken::Whitespace),
        ("\t", EnumToken::Whitespace),
        ("\r", EnumToken::Whitespace),
        ("\n", EnumToken::Whitespace),
        ("\x0c", EnumToken::Whitespace),
    ] {
        define(key.to_string(), typ, &mut definitions);
    }
    let assign = |entries: &[&str], typ: u8, suffix: &str, lowercase: bool, defs: &mut Vec<(String, u8)>, items: &mut HashTable<Vec<(u8, u32)>>| {
        for &entry in entries {
            let key = format!("{}{}", if lowercase { entry.to_lowercase() } else { entry.to_string() }, suffix);
            define(key, typ, defs);
            if items.get_key(entry).is_none() {
                items.set(entry, Vec::new());
            }
            let list = items.get_key_mut(entry).unwrap();
            // the first item wins
            if !list.iter().any(|(t, _)| *t == typ) {
                list.push((typ, register_canonical(entry)));
            }
        }
    };
    assign(&FLEX_UNITS, EnumToken::FlexTokenType, "", false, &mut definitions, &mut items);
    assign(&DIMENSION_UNITS, EnumToken::LengthTokenType, "", false, &mut definitions, &mut items);
    assign(&RESOLUTION_UNITS, EnumToken::ResolutionTokenType, "", false, &mut definitions, &mut items);
    assign(&ANGLE_UNITS, EnumToken::AngleTokenType, "", false, &mut definitions, &mut items);
    assign(&TIME_UNITS, EnumToken::TimeTokenType, "", false, &mut definitions, &mut items);
    assign(&FREQUENCY_UNITS, EnumToken::FrequencyTokenType, "", false, &mut definitions, &mut items);
    // the units of the other dimensions are rendered in their canonical spelling: `6DB` is rendered `6dB`
    for unit in validation_units() {
        if items.get_key(unit).is_none() {
            items.set(unit, Vec::new());
        }
        let list = items.get_key_mut(unit).unwrap();
        if !list.iter().any(|(t, _)| *t == EnumToken::DimensionTokenType) {
            list.push((EnumToken::DimensionTokenType, register_canonical(unit)));
        }
    }
    assign(&PSEUDO_ELEMENTS, EnumToken::PseudoElementTokenType, "", false, &mut definitions, &mut items);
    assign(&CONTAINER_FUNC, EnumToken::ContainerFunctionTokenDefType, "(", false, &mut definitions, &mut items);
    assign(&URL_FUNC, EnumToken::UrlFunctionTokenDefType, "(", false, &mut definitions, &mut items);
    assign(&GRID_TEMPLATE_FUNC, EnumToken::GridTemplateFuncTokenDefType, "(", false, &mut definitions, &mut items);
    assign(&IMAGE_FUNC, EnumToken::ImageFunctionTokenDefType, "(", false, &mut definitions, &mut items);
    assign(&TIMELINE_FUNC, EnumToken::TimelineFunctionTokenDefType, "(", false, &mut definitions, &mut items);
    assign(&SUPPORT_FUNC, EnumToken::SupportsFunctionTokenDefType, "(", false, &mut definitions, &mut items);
    assign(&TIMING_FUNC, EnumToken::TimingFunctionTokenDefType, "(", false, &mut definitions, &mut items);
    assign(&COLORS_FUNC, EnumToken::ColorFunctionTokenDefType, "(", false, &mut definitions, &mut items);
    assign(&MATH_FUNCS, EnumToken::MathFunctionTokenDefType, "(", false, &mut definitions, &mut items);
    assign(&TRANSFORM_FUNCTIONS, EnumToken::TransformFunctionTokenDefType, "(", true, &mut definitions, &mut items);
    assign(&WHEN_ELSE_FUNC, EnumToken::WhenElseFunctionTokenDefType, "(", false, &mut definitions, &mut items);
    assign(&WILD_CARD_FUNCS, EnumToken::WildCardFunctionTokenDefType, "(", false, &mut definitions, &mut items);
    let mut symbols = HashTable::new();
    for (key, typ) in definitions {
        symbols.set(&key, typ);
    }
    Tables { symbols, items }
}

/// do not capture the value (`hintsEnum`)
pub fn is_hint(typ: u8) -> bool {
    matches!(
        typ,
        EnumToken::CommaTokenType
            | EnumToken::ImportantTokenType
            | EnumToken::SemiColonTokenType
            | EnumToken::BlockStartTokenType
            | EnumToken::BlockEndTokenType
            | EnumToken::StartParensTokenType
            | EnumToken::EndParensTokenType
            | EnumToken::ColonTokenType
            | EnumToken::EOFTokenType
    )
}

/// field of the token the value of a token is stored in
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenValueField {
    None = 0,
    Number = 1,
    Name = 2,
    Value = 3,
    Unit = 4,
}

/// where the string value of a token is read from
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenValueKind {
    Source = 0,
    Canonical = 1,
    Escaped = 2,
}

#[allow(non_snake_case, dead_code)]
mod TokenMap {
    pub const EXCLAMATION: u32 = 33;
    pub const QUESTION: u32 = 63;
    pub const SLASH: u32 = 47;
    pub const LOWERTHAN: u32 = 60;
    pub const HASH: u32 = 35;
    pub const REVERSE_SOLIDUS: u32 = 92;
    pub const DOUBLE_QUOTE: u32 = 34;
    pub const SINGLE_QUOTE: u32 = 39;
    pub const DOT: u32 = 46;
    pub const AT: u32 = 64;
    pub const PIPE: u32 = 124;
    pub const EQUALS: u32 = 61;
    pub const AMPERSAND: u32 = 38;
    pub const STAR: u32 = 42;
    pub const TILDA: u32 = 126;
    pub const CARET: u32 = 94;
    pub const DOLLAR: u32 = 36;
    pub const COMMA: u32 = 44;
    pub const COLON: u32 = 58;
    pub const SEMICOLON: u32 = 59;
    pub const LEFT_PARENTHESIS: u32 = 40;
    pub const RIGHT_PARENTHESIS: u32 = 41;
    pub const LEFT_BRACKETS: u32 = 91;
    pub const RIGHT_BRACKETS: u32 = 93;
    pub const LEFT_BRACE: u32 = 123;
    pub const RIGHT_BRACE: u32 = 125;
    pub const PLUS: u32 = 43;
    pub const MINUS: u32 = 45;
    pub const GREATERTHAN: u32 = 62;
    pub const PERCENTAGE: u32 = 37;
}

use TokenMap::*;

/// value of a number: the longest prefix of the range that is a number, like `parseFloat()`. the prefix is copied to a
/// stack buffer and parsed by the correctly rounded rust parser (same result as `parseFloat()`)
pub fn parse_number(codes: &CodeUnits, start: usize, end: usize) -> f64 {
    parse_short_number(codes, start, end).unwrap_or_else(|| parse_long_number(codes, start, end))
}

/// powers of ten represented exactly by a f64
const POWERS_OF_TEN: [f64; 16] = [1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15];

/// a number of 15 digits or less, without exponent: the digits are an integer represented exactly by a f64, divided by
/// a power of ten represented exactly. the division is correctly rounded: the result is the result of `parseFloat`.
/// `None` for the other numbers
fn parse_short_number(codes: &CodeUnits, start: usize, end: usize) -> Option<f64> {
    let mut i = start;
    let mut code = codes.at(i);
    let negative = code == 0x2d;
    if negative || code == 0x2b {
        i += 1;
        code = codes.at(i);
    }
    let mut mantissa: u64 = 0;
    let mut digits = 0usize;
    let mut decimals = 0usize;
    let mut dot = false;
    while i < end {
        if (0x30..=0x39).contains(&code) {
            mantissa = mantissa * 10 + (code - 0x30) as u64;
            digits += 1;
            if dot {
                decimals += 1;
            }
        } else if code == 0x2e && !dot {
            dot = true;
        } else {
            break;
        }
        i += 1;
        code = codes.at(i);
    }
    if digits == 0 || digits > 15 || (i < end && (code == 0x65 || code == 0x45)) {
        return None;
    }
    let value = mantissa as f64 / POWERS_OF_TEN[decimals];
    Some(if negative { -value } else { value })
}

/// see [`parse_number`]: the numbers that are not read by [`parse_short_number`]
fn parse_long_number(codes: &CodeUnits, start: usize, end: usize) -> f64 {
    let mut buffer = [0u8; 128];
    let mut heap: Vec<u8> = Vec::new();
    let mut len = 0usize;
    let push = |b: u8, buffer: &mut [u8; 128], heap: &mut Vec<u8>, len: &mut usize| {
        if *len < 128 {
            buffer[*len] = b;
        } else {
            if heap.is_empty() {
                heap.extend_from_slice(&buffer[..]);
            }
            heap.push(b);
        }
        *len += 1;
    };
    let mut i = start;
    let mut code = codes.at(i);
    if code == 0x2d || code == 0x2b {
        push(code as u8, &mut buffer, &mut heap, &mut len);
        i += 1;
        code = codes.at(i);
    }
    let mut count = 0;
    let mut dot = false;
    while i < end {
        if (0x30..=0x39).contains(&code) {
            push(code as u8, &mut buffer, &mut heap, &mut len);
            count += 1;
        } else if code == 0x2e && !dot {
            dot = true;
            push(b'.', &mut buffer, &mut heap, &mut len);
        } else {
            break;
        }
        i += 1;
        code = codes.at(i);
    }
    if count == 0 {
        return f64::NAN;
    }
    if i < end && (code == 0x65 || code == 0x45) {
        let mut k = i + 1;
        let mut exponent: Vec<u8> = vec![b'e'];
        if k < end && (codes.at(k) == 0x2d || codes.at(k) == 0x2b) {
            exponent.push(codes.at(k) as u8);
            k += 1;
        }
        if k < end && is_digit(codes.at(k)) {
            let mut digits = 0;
            while k < end && is_digit(codes.at(k)) {
                // larger exponents are Infinity or 0
                if digits < 7 {
                    exponent.push(codes.at(k) as u8);
                }
                digits += 1;
                k += 1;
            }
            for b in exponent {
                push(b, &mut buffer, &mut heap, &mut len);
            }
        }
    }
    let text = if heap.is_empty() { &buffer[..len] } else { &heap[..] };
    // ascii digits: valid utf-8. rust rejects a trailing dot ("1.") but parseFloat accepts it
    let mut s = unsafe { std::str::from_utf8_unchecked(text) };
    let trimmed;
    if s.contains(".e") || s.contains(".E") || s.ends_with('.') {
        trimmed = s.replacen(".e", "e", 1).replacen(".E", "E", 1).trim_end_matches('.').to_string();
        s = &trimmed;
    }
    s.parse::<f64>().unwrap_or(f64::NAN)
}

#[inline]
fn is_hex_digit(codepoint: u32) -> bool {
    (0x30..=0x39).contains(&codepoint) || (0x41..=0x46).contains(&codepoint) || (0x61..=0x66).contains(&codepoint)
}

/// true if the character may follow a numeric token
#[inline]
fn is_numeric_token_end(codepoint: u32, is_dimension: bool) -> bool {
    codepoint == EOF
        || is_white_space(codepoint)
        || codepoint == LEFT_BRACE
        || codepoint == RIGHT_PARENTHESIS
        || codepoint == SEMICOLON
        || codepoint == EXCLAMATION
        || codepoint == RIGHT_BRACE
        || codepoint == SLASH
        || codepoint == STAR
        || codepoint == COMMA
        || (is_dimension && codepoint == PLUS)
}

/// the css being tokenized (`ParseInfo`)
#[derive(Debug)]
pub struct ParseInfo {
    pub codes: CodeUnits,
    pub length: usize,
    pub source: Rc<SourceFile>,
    pub offset: usize,
    pub time: f64,
    pub position: usize,
    pub current_position: usize,
}

impl ParseInfo {
    #[inline(always)]
    fn at(&self, index: usize) -> u32 {
        if index < self.length {
            self.codes.at(index)
        } else {
            EOF
        }
    }

    #[inline(always)]
    fn cur(&self) -> u32 {
        self.at(self.current_position - self.offset)
    }

    fn push_line_start(&self, position: usize) {
        self.source.line_starts.borrow_mut().line_starts.push(position as u32);
    }
}

/// the parse info of a css input
pub fn create_parse_info(input: Option<&CssInput>, source: Rc<SourceFile>) -> ParseInfo {
    let mut parse_info = ParseInfo {
        codes: CodeUnits::Bytes(Rc::from(&[][..])),
        length: 0,
        source,
        offset: 0,
        time: 0.0,
        position: 0,
        current_position: 0,
    };
    Tokenizer::load(&mut parse_info, input);
    parse_info
}

/// a chunk of a stream
pub enum StreamChunk {
    Bytes(Rc<[u8]>),
    Text(Rc<str>),
}

/// decode the longest prefix of complete utf-8 characters: the invalid sequences are replaced with U+FFFD, like
/// `TextDecoder`. returns the length of the decoded prefix: the bytes that follow start a character that is not
/// complete
fn decode_utf8_prefix(bytes: &[u8], out: &mut String) -> usize {
    let mut i = 0;
    while i < bytes.len() {
        match std::str::from_utf8(&bytes[i..]) {
            Ok(text) => {
                out.push_str(text);
                return bytes.len();
            }
            Err(error) => {
                let valid = error.valid_up_to();
                // SAFETY: the prefix is valid utf-8
                out.push_str(unsafe { std::str::from_utf8_unchecked(&bytes[i..i + valid]) });
                match error.error_len() {
                    // a character that continues in the next chunk
                    None => return i + valid,
                    Some(length) => {
                        out.push('\u{fffd}');
                        i += valid + length;
                    }
                }
            }
        }
    }
    i
}

/// a buffer of at least `required` units that starts with `units`: the capacity doubles
fn grow<T: Copy>(units: &[T], required: usize, zero: T) -> Rc<[T]> {
    let mut capacity = units.len().max(1 << 16);
    while capacity < required {
        capacity *= 2;
    }
    let mut buffer = Vec::with_capacity(capacity);
    buffer.extend_from_slice(units);
    buffer.resize(capacity, zero);
    Rc::from(buffer)
}

/// write the utf-16 code units of a text at an offset
fn write_utf16(buffer: &mut [u16], offset: usize, text: &str) {
    for (i, unit) in text.encode_utf16().enumerate() {
        buffer[offset + i] = unit;
    }
}

/// tokenizer
pub struct Tokenizer {
    pub typ: Option<u8>,
    pub kin: Option<u8>,
    pub num: f64,
    pub field: TokenValueField,
    pub value_kind: TokenValueKind,
    pub value_start: usize,
    pub value_end: usize,
    pub src_id: Option<u32>,
    pub sta: Option<usize>,
    pub end: Option<usize>,
    pub bytes_in: Option<usize>,
    pub slice: Option<usize>,
    ident_non_ascii: bool,
    hint: Option<u8>,
    state: Option<u8>,
    pub parse_info: ParseInfo,
    decoded: Vec<u16>,
    /// the input is a stream that is not read to its end, see [`Tokenizer::next`] and [`Tokenizer::append`]
    stream: Option<StreamState>,
    /// the last call of [`Tokenizer::next`] reached the end of the code units read from the stream: the token may
    /// continue in the next chunk. no token is returned, a chunk must be appended before `next()` is called again
    pub waiting: bool,
    /// the size of the css of a stream, in bytes, when it is known, see [`Tokenizer::streaming`]
    expected_length: usize,
}

/// the state of a stream read by the tokenizer
struct StreamState {
    /// the bytes of a character split between two chunks
    pending: Vec<u8>,
    /// no chunk has been decoded yet: a leading byte order mark is removed
    first: bool,
}

/// number of code units the tokenizer may read after the end of a token (escape sequences, unicode ranges, `<!--`,
/// ...): a token read from a stream is complete when the stream has more code units after it
const STREAM_LOOKAHEAD: usize = 32;

impl Tokenizer {
    pub fn new(parse_info: ParseInfo) -> Self {
        Self {
            typ: None,
            kin: None,
            num: 0.0,
            field: TokenValueField::None,
            value_kind: TokenValueKind::Source,
            value_start: 0,
            value_end: 0,
            src_id: None,
            sta: None,
            end: None,
            bytes_in: None,
            slice: None,
            ident_non_ascii: false,
            hint: None,
            state: None,
            parse_info,
            decoded: Vec::new(),
            stream: None,
            waiting: false,
            expected_length: 0,
        }
    }

    /// tokenizer of a stream: the chunks are appended with [`Tokenizer::append`] when the tokenizer is waiting.
    /// `size`: the size of the css in bytes when it is known, 0 otherwise. the code units are allocated once: the size
    /// in bytes is not less than the number of utf-16 code units
    pub fn streaming(parse_info: ParseInfo, size: usize) -> Self {
        let mut tokenizer = Self::new(parse_info);
        tokenizer.parse_info.codes = CodeUnits::Bytes(if size > 0 { Rc::from(vec![0u8; size]) } else { Rc::from(&[][..]) });
        tokenizer.parse_info.length = 0;
        tokenizer.stream = Some(StreamState { pending: Vec::new(), first: true });
        tokenizer.expected_length = size;
        tokenizer
    }

    /// the size of the css of a stream, in bytes, when it is known. 0: not known, or not a stream
    pub fn expected_length(&self) -> usize {
        self.expected_length
    }

    /// true while the input is a stream that is not read to its end
    pub fn is_streaming(&self) -> bool {
        self.stream.is_some()
    }

    /// append a chunk of the stream: utf-8 bytes or text. `None` ends the stream. the code units stay bytes while the
    /// css is ascii, they are utf-16 code units otherwise. the buffer of the code units doubles when it is full: the
    /// code units read so far are not copied for each chunk. the buffer is written in place when the tokenizer holds
    /// the only reference to it: the caller drops its other references first (the source text of the arena)
    pub fn append(&mut self, chunk: Option<StreamChunk>) {
        self.waiting = false;
        let Some(state) = self.stream.as_mut() else {
            return;
        };
        let mut text = String::new();
        match chunk {
            Some(StreamChunk::Text(t)) => text.push_str(&t),
            Some(StreamChunk::Bytes(bytes)) => {
                state.pending.extend_from_slice(&bytes);
                let pending = std::mem::take(&mut state.pending);
                let rest = decode_utf8_prefix(&pending, &mut text);
                state.pending = pending[rest..].to_vec();
                // TextDecoder removes the byte order mark
                if state.first && !text.is_empty() {
                    if let Some(stripped) = text.strip_prefix('\u{feff}') {
                        text = stripped.to_string();
                    }
                }
                if !text.is_empty() {
                    state.first = false;
                }
            }
            None => {
                // a character that is not complete at the end of the stream
                if !state.pending.is_empty() {
                    text.push('\u{fffd}');
                }
                self.stream = None;
            }
        }
        if !text.is_empty() {
            self.append_text(&text);
        }
        if self.stream.is_none() {
            self.finish_stream();
        }
    }

    /// append the code units of a text to the code units of the stream
    fn append_text(&mut self, text: &str) {
        // the size of the stream when it is known: the buffer of utf-16 code units is allocated once
        let expected = self.expected_length;
        let pi = &mut self.parse_info;
        let length = pi.length;
        let ascii = text.is_ascii() && matches!(pi.codes, CodeUnits::Bytes(_));
        let required = length + if ascii { text.len() } else { text.encode_utf16().count() };
        let codes = std::mem::replace(&mut pi.codes, CodeUnits::Bytes(Rc::from(&[][..])));
        pi.codes = match codes {
            CodeUnits::Bytes(mut bytes) if ascii => {
                if required > bytes.len() || Rc::get_mut(&mut bytes).is_none() {
                    bytes = grow(&bytes[..length], required, 0u8);
                }
                Rc::get_mut(&mut bytes).expect("unique buffer")[length..required].copy_from_slice(text.as_bytes());
                CodeUnits::Bytes(bytes)
            }
            CodeUnits::Bytes(bytes) => {
                // the css is not ascii: the code units are utf-16 code units from now on
                let widened: Vec<u16> = bytes[..length].iter().map(|&b| b as u16).collect();
                let mut units = grow(&widened, required.max(expected), 0u16);
                write_utf16(Rc::get_mut(&mut units).expect("unique buffer"), length, text);
                CodeUnits::Utf16(units)
            }
            CodeUnits::Utf16(mut units) => {
                if required > units.len() || Rc::get_mut(&mut units).is_none() {
                    units = grow(&units[..length], required, 0u16);
                }
                write_utf16(Rc::get_mut(&mut units).expect("unique buffer"), length, text);
                CodeUnits::Utf16(units)
            }
        };
        pi.length = required;
    }

    /// the end of the stream: the code units are trimmed to their length, the text of the source file is set
    fn finish_stream(&mut self) {
        let pi = &mut self.parse_info;
        let length = pi.length;
        let (codes, text): (CodeUnits, Rc<str>) = match &pi.codes {
            CodeUnits::Bytes(b) => {
                let bytes: Rc<[u8]> = Rc::from(&b[..length]);
                // ascii: the bytes are the text
                let text: Rc<str> = Rc::from(std::str::from_utf8(&bytes).expect("ascii"));
                (CodeUnits::Bytes(bytes), text)
            }
            CodeUnits::Utf16(u) => (CodeUnits::Utf16(Rc::from(&u[..length])), Rc::from(String::from_utf16_lossy(&u[..length]))),
        };
        pi.codes = codes;
        *pi.source.content.borrow_mut() = text;
    }

    /// tokenizer of a css input with a new anonymous source
    pub fn from_input(input: &CssInput) -> Self {
        let source = SourceFile::new(Rc::from(""), Vec::new(), Some(Rc::from("")));
        Self::new(create_parse_info(Some(input), source))
    }

    /// set the css read by the tokenizer. the text of the css is the content of the source file
    pub fn load(parse_info: &mut ParseInfo, input: Option<&CssInput>) {
        let units = to_code_units(input);
        parse_info.length = units.codes.len();
        parse_info.codes = units.codes;
        *parse_info.source.content.borrow_mut() = units.text;
    }

    /// id of the source file being tokenized
    pub fn source_id(&self) -> u32 {
        self.parse_info.source.id
    }

    /// length of the css input, in code units
    pub fn stream_length(&self) -> usize {
        self.parse_info.length
    }

    /// the source file
    pub fn source(&self) -> Rc<SourceFile> {
        self.parse_info.source.clone()
    }

    fn consume_string(&mut self) -> &mut Self {
        let quote = self.advance_code();
        let mut decode_segments = false;
        loop {
            let pi = &self.parse_info;
            let mut char_code = pi.cur();
            if char_code == EOF {
                break;
            }
            if char_code == REVERSE_SOLIDUS {
                let p = pi.current_position - pi.offset;
                if char_code == pi.at(p + 1) {
                    self.advance(2);
                    continue;
                }
                char_code = pi.at(p + 1);
                if matches!(char_code, 0xa | 0xb | 0xc | 0xd | 0x2028 | 0x2029) {
                    self.advance(2);
                    decode_segments = true;
                    continue;
                }
                let escape_length = self.escape_sequence_length();
                if escape_length > 0 {
                    let pi = &self.parse_info;
                    let length = escape_length + 1 + if is_white_space(pi.cur()) { 1 } else { 0 };
                    decode_segments = true;
                    self.advance(length);
                    continue;
                }
                self.advance(2);
                continue;
            }
            if char_code == quote {
                self.advance_code();
                return self.make_token(Some(EnumToken::StringTokenType), None, decode_segments);
            }
            if is_new_line(char_code) {
                self.advance_code();
                return self.make_token(Some(EnumToken::BadStringTokenType), None, false);
            }
            self.advance_code();
        }
        self.make_token(Some(EnumToken::StringTokenType), None, decode_segments)
    }

    fn consume_string_as_url_token(&mut self) -> &mut Self {
        let quote = self.advance_code();
        let mut decode_segments = false;
        loop {
            let pi = &self.parse_info;
            let mut char_code = pi.cur();
            if char_code == EOF {
                break;
            }
            if char_code == REVERSE_SOLIDUS {
                let p = pi.current_position - pi.offset;
                if char_code == pi.at(p + 1) {
                    self.advance(2);
                    continue;
                }
                let escape_length = self.escape_sequence_length();
                if escape_length > 0 {
                    let length = escape_length + 1 + if is_white_space(self.parse_info.cur()) { 1 } else { 0 };
                    decode_segments = true;
                    self.advance(length);
                    continue;
                }
                self.advance(2);
                continue;
            }
            if char_code == quote {
                self.advance_code();
                let pi = &self.parse_info;
                let k = 1;
                let end = pi.length - pi.offset;
                let position = pi.current_position - pi.offset;
                let mut k = k;
                while position + k < end {
                    char_code = pi.at(position);
                    if char_code == EOF {
                        self.advance(k);
                        return self.make_token(Some(EnumToken::BadUrlTokenType), None, false);
                    }
                    if is_white_space(char_code) {
                        k += 1;
                        continue;
                    }
                    if char_code != RIGHT_PARENTHESIS {
                        self.advance(k);
                        return self.make_token(Some(EnumToken::BadUrlTokenType), None, false);
                    }
                    break;
                }
                return self.make_token(Some(EnumToken::StringTokenType), None, decode_segments);
            }
            if is_new_line(char_code) {
                self.advance_code();
                loop {
                    let c = self.parse_info.cur();
                    if c == EOF {
                        break;
                    }
                    if c == REVERSE_SOLIDUS {
                        self.advance(2);
                        continue;
                    }
                    if c == RIGHT_PARENTHESIS {
                        return self.make_token(Some(EnumToken::BadUrlTokenType), None, false);
                    }
                    self.advance_code();
                }
                return self.make_token(Some(EnumToken::BadStringTokenType), None, false);
            }
            self.advance_code();
        }
        self.make_token(Some(EnumToken::BadUrlTokenType), None, false)
    }

    /// consume a unicode range: `U+0025-00FF`, `U+4??`
    fn consume_unicode_range_token(&self) -> usize {
        let pi = &self.parse_info;
        let start = pi.current_position - pi.offset;
        let mut position = start + 2;
        let mut count = 0;
        let mut wildcards = 0;
        if pi.at(start + 1) != PLUS {
            return 0;
        }
        while count < 6 {
            let codepoint = pi.at(position);
            if wildcards == 0 && is_hex_digit(codepoint) {
                count += 1;
                position += 1;
            } else if codepoint == QUESTION {
                wildcards += 1;
                count += 1;
                position += 1;
            } else {
                break;
            }
        }
        if count == 0 {
            return 0;
        }
        if wildcards == 0 && pi.at(position) == MINUS && is_hex_digit(pi.at(position + 1)) {
            position += 1;
            count = 0;
            while count < 6 && is_hex_digit(pi.at(position)) {
                count += 1;
                position += 1;
            }
        }
        let codepoint = pi.at(position);
        if codepoint == EOF
            || is_white_space(codepoint)
            || codepoint == COMMA
            || codepoint == SEMICOLON
            || codepoint == RIGHT_BRACE
            || codepoint == RIGHT_PARENTHESIS
            || codepoint == EXCLAMATION
        {
            return position - start;
        }
        0
    }

    /// consume a number, a dimension or a percentage
    fn consume_numeric_token(&mut self) -> usize {
        self.slice = None;
        self.hint = None;
        let pi = &self.parse_info;
        let offset = pi.current_position - pi.offset;
        let mut position = offset;
        let mut has_digits = false;
        let mut codepoint = pi.at(position);
        if codepoint == PLUS || codepoint == MINUS {
            position += 1;
        }
        while is_digit(pi.at(position)) {
            position += 1;
            has_digits = true;
        }
        if pi.at(position) == DOT && is_digit(pi.at(position + 1)) {
            position += 1;
            while is_digit(pi.at(position)) {
                position += 1;
            }
            has_digits = true;
        }
        if !has_digits {
            return 0;
        }
        codepoint = pi.at(position);
        if codepoint == 0x45 || codepoint == 0x65 {
            let mut k = position + 1;
            if pi.at(k) == PLUS || pi.at(k) == MINUS {
                k += 1;
            }
            if is_digit(pi.at(k)) {
                position = k;
                while is_digit(pi.at(position)) {
                    position += 1;
                }
            }
        }
        codepoint = pi.at(position);
        if codepoint == PERCENTAGE {
            let slice = position;
            position += 1;
            if !is_numeric_token_end(pi.at(position), false) {
                return 0;
            }
            self.slice = Some(slice);
            self.hint = Some(EnumToken::PercentageTokenType);
            return position - offset;
        }
        if is_letter(codepoint) || (codepoint == MINUS && pi.at(position + 1) == MINUS) {
            let slice = position;
            while position < pi.length {
                position += 1;
                codepoint = pi.at(position);
                if !is_letter(codepoint) && codepoint != MINUS {
                    break;
                }
            }
            if !is_numeric_token_end(pi.at(position), true) {
                return 0;
            }
            let hint = get_symbol_hint(pi, slice, position).unwrap_or(EnumToken::DimensionTokenType);
            self.slice = Some(slice);
            self.hint = Some(hint);
            return position - offset;
        }
        if is_numeric_token_end(codepoint, false) {
            position - offset
        } else {
            0
        }
    }

    fn consume_ident_token(&mut self) -> usize {
        let pi = &self.parse_info;
        let mut position = pi.current_position - pi.offset;
        let offset = position;
        self.ident_non_ascii = false;
        let mut codepoint = pi.at(position);
        if !is_ident_start(codepoint) && codepoint != MINUS {
            return 0;
        }
        if codepoint == MINUS {
            position += 1;
            codepoint = pi.at(position);
            if !is_ident_start(codepoint) && codepoint != MINUS {
                return 0;
            }
        }
        loop {
            codepoint = pi.at(position);
            if codepoint == EOF {
                break;
            }
            if codepoint == REVERSE_SOLIDUS {
                codepoint = pi.at(position + 1);
                if codepoint == EOF {
                    return 0;
                }
                if matches!(codepoint, 0xa | 0xb | 0xc | 0xd | 0x2028 | 0x2029) {
                    return 0;
                }
                position += 2;
                continue;
            }
            if codepoint == 0x2d || is_ident_codepoint(codepoint) {
                if codepoint > 0x7f {
                    self.ident_non_ascii = true;
                }
                position += 1;
            } else {
                match codepoint {
                    COLON | LEFT_BRACE | RIGHT_BRACE | LEFT_PARENTHESIS | RIGHT_PARENTHESIS | LEFT_BRACKETS | RIGHT_BRACKETS
                    | SEMICOLON | EXCLAMATION | SLASH | HASH | STAR | EQUALS | TILDA | PIPE | CARET | DOLLAR | COMMA
                    | GREATERTHAN | DOT | PLUS => return position - offset,
                    _ => {}
                }
                if is_white_space(codepoint) {
                    return position - offset;
                }
                return 0;
            }
        }
        position - offset
    }

    fn consume_color(&self) -> usize {
        let pi = &self.parse_info;
        let mut position = pi.current_position - pi.offset;
        let offset = position;
        if pi.at(position) != HASH {
            return 0;
        }
        position += 1;
        let mut count = 0;
        while is_hex_digit(pi.at(position)) {
            position += 1;
            count += 1;
        }
        if count != 3 && count != 4 && count != 6 && count != 8 {
            return 0;
        }
        let codepoint = pi.at(position);
        if codepoint == EOF
            || is_white_space(codepoint)
            || codepoint == RIGHT_PARENTHESIS
            || codepoint == SEMICOLON
            || codepoint == RIGHT_BRACE
            || codepoint == COMMA
        {
            return position - offset;
        }
        0
    }

    fn consume_url_token(&mut self, end_position: usize) -> &mut Self {
        while is_white_space(self.parse_info.cur()) {
            self.advance_code();
        }
        let mut char_code = self.parse_info.cur();
        if char_code == DOUBLE_QUOTE || char_code == SINGLE_QUOTE {
            return self.consume_string_as_url_token();
        }
        loop {
            self.advance_code();
            char_code = self.parse_info.cur();
            if !(char_code != RIGHT_PARENTHESIS && !is_white_space(char_code) && self.parse_info.current_position < end_position) {
                break;
            }
        }
        if char_code != RIGHT_PARENTHESIS {
            let mut k = 1;
            let pi = &self.parse_info;
            while k < end_position {
                char_code = pi.at(pi.current_position - pi.offset + k);
                if is_white_space(char_code) {
                    k += 1;
                    continue;
                }
                break;
            }
            if char_code == EOF || (char_code != RIGHT_PARENTHESIS && !is_ident_start(char_code)) {
                self.advance(k);
                return self.make_token(Some(EnumToken::BadUrlTokenType), None, false);
            }
        }
        let typ = if self.parse_info.cur() == EOF || !self.is_url_token() {
            EnumToken::BadUrlTokenType
        } else {
            EnumToken::UrlTokenTokenType
        };
        self.make_token(Some(typ), None, false)
    }

    /// set the current token: its type, its numeric value and the range of its string value. no string is created
    pub fn make_token(&mut self, hint: Option<u8>, slice: Option<usize>, decode_segments: bool) -> &mut Self {
        let pi = &self.parse_info;
        let start = pi.position - pi.offset;
        let end = pi.current_position - pi.offset;
        let mut canonical: i64 = -1;
        let mut value_start = start;
        let mut has_value = false;
        self.typ = None;
        self.kin = None;
        self.num = 0.0;
        self.field = TokenValueField::None;
        self.slice = None;
        self.hint = None;
        // `if (slice)`: 0 is falsy
        if let Some(s) = slice.filter(|&s| s != 0) {
            self.slice = Some(s);
        }
        if let Some(hint) = hint {
            let mut is_list = false;
            let mut has_unit = false;
            match hint {
                EnumToken::TransformFunctionTokenDefType
                | EnumToken::ColorFunctionTokenDefType
                | EnumToken::ContainerFunctionTokenDefType
                | EnumToken::UrlFunctionTokenDefType
                | EnumToken::GridTemplateFuncTokenDefType
                | EnumToken::ImageFunctionTokenDefType
                | EnumToken::TimelineFunctionTokenDefType
                | EnumToken::SupportsFunctionTokenDefType
                | EnumToken::TimingFunctionTokenDefType
                | EnumToken::MathFunctionTokenDefType
                | EnumToken::WhenElseFunctionTokenDefType
                | EnumToken::WildCardFunctionTokenDefType => is_list = true,
                EnumToken::FrequencyTokenType
                | EnumToken::ResolutionTokenType
                | EnumToken::LengthTokenType
                | EnumToken::FlexTokenType
                | EnumToken::AngleTokenType
                | EnumToken::TimeTokenType => {
                    is_list = true;
                    has_unit = true;
                }
                EnumToken::DimensionTokenType => has_unit = true,
                _ => {}
            }
            let hinted = is_hint(hint);
            if is_list {
                value_start = if has_unit { slice.unwrap_or(0) } else { start };
                canonical = find_item(hint, pi, value_start, end);
                has_value = canonical != -1;
            } else if hint == EnumToken::NumberTokenType || hint == EnumToken::WhitespaceTokenType {
                // the number is read below. the whitespace is rendered as a single space
            } else if !hinted {
                value_start = slice.unwrap_or(start);
                canonical = if hint == EnumToken::DimensionTokenType {
                    find_item(EnumToken::DimensionTokenType, pi, value_start, end)
                } else {
                    -1
                };
                has_value = true;
            }
            self.typ = Some(hint);
            if !hinted {
                if has_unit || hint == EnumToken::PercentageTokenType || hint == EnumToken::DimensionTokenType {
                    self.num = parse_number(&pi.codes, start, slice.unwrap_or(start));
                    if hint != EnumToken::PercentageTokenType {
                        self.field = if has_value { TokenValueField::Unit } else { TokenValueField::Number };
                        if canonical == -1 && pi.at(value_start) == 0x2d && pi.at(value_start + 1) == 0x2d {
                            self.typ = Some(EnumToken::UserDefinedDimensionTokenType);
                        }
                    } else {
                        self.field = TokenValueField::Number;
                    }
                } else if hint == EnumToken::NumberTokenType {
                    self.num = parse_number(&pi.codes, slice.unwrap_or(start), end);
                    self.field = TokenValueField::Number;
                } else if hint == EnumToken::AtRuleTokenType {
                    self.field = if has_value { TokenValueField::Name } else { TokenValueField::None };
                } else {
                    self.field = if has_value { TokenValueField::Value } else { TokenValueField::None };
                    if hint == EnumToken::ColorTokenType {
                        self.kin = Some(ColorType::HEX);
                    }
                }
            }
        } else if self.equals_ignore_case("!important") {
            self.typ = Some(EnumToken::ImportantTokenType);
        }
        if self.typ.is_none() {
            value_start = start;
            has_value = true;
            self.typ = Some(EnumToken::LiteralTokenType);
            self.field = TokenValueField::Value;
        }
        if has_value {
            if canonical != -1 {
                self.value_kind = TokenValueKind::Canonical;
                self.value_start = canonical_start(canonical as u32) as usize;
                self.value_end = canonical_end(canonical as u32) as usize;
            } else {
                self.value_kind = if decode_segments { TokenValueKind::Escaped } else { TokenValueKind::Source };
                self.value_start = value_start;
                self.value_end = end;
            }
        }
        let pi = &mut self.parse_info;
        self.src_id = Some(pi.source.id);
        self.sta = Some(pi.position);
        self.end = Some(pi.current_position);
        self.bytes_in = Some(pi.current_position);
        pi.position = pi.current_position;
        self
    }

    /// id of the string value of the current token in the arena. no string is created
    pub fn intern_value(&mut self, arena: &AstArena, source_index: i32) -> u32 {
        match self.value_kind {
            TokenValueKind::Canonical => arena.intern_canonical(self.value_start as u32, self.value_end as u32),
            TokenValueKind::Escaped => {
                let mut decoded = std::mem::take(&mut self.decoded);
                decode_escapes(&self.parse_info.codes, self.value_start, self.value_end, &mut decoded);
                let id = arena.intern_units(&decoded);
                self.decoded = decoded;
                id
            }
            TokenValueKind::Source => arena.intern_range(
                &self.parse_info.codes,
                self.value_start,
                self.value_end,
                if source_index < 0 { StringSource::Pool } else { source_index },
            ),
        }
    }

    /// the string value of the current token (decoded)
    pub fn value_string(&mut self) -> String {
        match self.value_kind {
            TokenValueKind::Canonical => crate::parser::utils::canonical::canonical_slice(self.value_start as u32, self.value_end as u32),
            TokenValueKind::Escaped => {
                let mut decoded = std::mem::take(&mut self.decoded);
                decode_escapes(&self.parse_info.codes, self.value_start, self.value_end, &mut decoded);
                let s = String::from_utf16_lossy(&decoded);
                self.decoded = decoded;
                s
            }
            TokenValueKind::Source => self.parse_info.codes.slice(self.value_start, self.value_end).into_owned(),
        }
    }

    /// content of the source being tokenized
    pub fn content(&self) -> Rc<str> {
        self.parse_info.source.content.borrow().clone()
    }

    /// code units of the source being tokenized
    pub fn codes(&self) -> &CodeUnits {
        &self.parse_info.codes
    }

    /// true if the text of the token being read equals the input, ignoring the case
    fn equals_ignore_case(&self, input: &str) -> bool {
        let pi = &self.parse_info;
        let position = pi.position - pi.offset;
        if pi.current_position - pi.position != input.len() {
            return false;
        }
        input.bytes().enumerate().all(|(i, b)| {
            let mut ca = pi.at(position + i);
            let mut cb = b as u32;
            if (65..=90).contains(&ca) {
                ca += 32;
            }
            if (65..=90).contains(&cb) {
                cb += 32;
            }
            ca == cb
        })
    }

    fn matches(&self, input: &str) -> bool {
        let pi = &self.parse_info;
        let position = pi.current_position - pi.offset;
        input.bytes().enumerate().all(|(i, b)| pi.at(position + i) == b as u32)
    }

    /// length of the escape sequence that follows the backslash at the current position
    fn escape_sequence_length(&self) -> usize {
        let pi = &self.parse_info;
        let position = pi.current_position - pi.offset;
        let limit = if pi.length.saturating_sub(position) < 7 { pi.length.saturating_sub(position) } else { 7 };
        let mut digits = 0;
        let mut length = 0;
        for i in 1..limit {
            let codepoint = pi.at(position + i);
            if codepoint == 0x20 {
                length += 1;
                break;
            }
            if !is_hex_digit(codepoint) {
                break;
            }
            digits += 1;
            length += 1;
        }
        if digits > 0 {
            length
        } else {
            0
        }
    }

    /// move forward by count characters and record the new lines
    fn advance(&mut self, count: usize) {
        let pi = &self.parse_info;
        let position = pi.current_position - pi.offset;
        let remaining = pi.length.saturating_sub(position);
        let length = count.min(remaining);
        for i in 0..length {
            let codepoint = pi.at(position + i);
            if matches!(codepoint, 0xa | 0xb | 0xc | 0xd | 0x2028 | 0x2029) && (codepoint != 0xd || pi.at(position + i + 1) != 0xa) {
                pi.push_line_start(position + pi.offset + i);
            }
        }
        self.parse_info.current_position += length;
    }

    /// move forward over characters that are not new lines
    #[inline]
    fn skip(&mut self, count: usize) {
        let pi = &mut self.parse_info;
        let remaining = pi.length.saturating_sub(pi.current_position - pi.offset);
        pi.current_position += count.min(remaining);
    }

    fn skip_ident(&mut self, count: usize) {
        if self.ident_non_ascii {
            self.advance(count);
        } else {
            self.skip(count);
        }
    }

    /// move forward by one character and return its code (EOF at the end of the stream)
    fn advance_code(&mut self) -> u32 {
        let pi = &self.parse_info;
        let position = pi.current_position - pi.offset;
        let codepoint = pi.at(position);
        if codepoint == EOF {
            return EOF;
        }
        if codepoint == 0xa
            || codepoint == 0xb
            || codepoint == 0xc
            || (codepoint == 0xd && pi.at(position + 1) != 0xa)
            || codepoint == 0x2028
            || codepoint == 0x2029
        {
            pi.push_line_start(position + pi.offset);
        }
        self.parse_info.current_position += 1;
        codepoint
    }

    fn is_ident_token(&self) -> bool {
        let pi = &self.parse_info;
        let mut j = (pi.current_position - pi.offset) as isize;
        let mut i = (pi.position - pi.offset) as isize;
        j -= 1;
        let at = |i: isize| pi.at(i as usize);
        let mut codepoint = at(i);
        if codepoint == 0x2d {
            let next = at(i + 1);
            if next == EOF {
                return false;
            }
            if !is_ident_start(next) && next != 0x2d {
                return false;
            }
            codepoint = next;
            i += 1;
        }
        if codepoint != 0x2d && !is_ident_start(codepoint) {
            return false;
        }
        if codepoint == REVERSE_SOLIDUS {
            i += 1;
        }
        while i < j {
            i += 1;
            codepoint = at(i);
            if codepoint == REVERSE_SOLIDUS {
                i += 2;
                continue;
            }
            if codepoint != 0x2d && !is_ident_codepoint(codepoint) {
                return false;
            }
        }
        true
    }

    fn starts_with(&self, input: &str) -> bool {
        let pi = &self.parse_info;
        input.bytes().enumerate().all(|(i, b)| pi.at(pi.position - pi.offset + i) == b as u32)
    }

    fn is_url_token(&self) -> bool {
        let pi = &self.parse_info;
        let mut i = pi.position - pi.offset;
        loop {
            i += 1;
            if i >= pi.current_position {
                break;
            }
            let mut c = pi.at(i);
            if is_non_printable(c) || c == 0x27 || c == 0x22 || c == 0x28 || c == 0x29 {
                return false;
            }
            if c == REVERSE_SOLIDUS {
                i += 1;
                if i >= pi.current_position {
                    return false;
                }
                c = pi.at(i);
                if c == 0x6e || c == 0x72 || c == 0x66 {
                    return false;
                }
                continue;
            }
            if c == 0x20 || c == 0x09 {
                break;
            }
        }
        i == pi.current_position
    }

    pub fn done(&self) -> bool {
        self.typ == Some(EnumToken::EOF)
    }

    /// read the next token
    #[allow(clippy::should_implement_trait)]
    /// read the next token. when the input is a stream that is not read to its end, a token that ends near the end of
    /// the code units read so far may continue in the next chunk (an identifier, a number, a string, a comment, ...):
    /// the token is not returned, [`Tokenizer::waiting`] is true and a chunk must be appended before the next call
    pub fn next(&mut self) -> &mut Self {
        if self.stream.is_none() {
            return self.next_token();
        }
        let position = self.parse_info.position;
        let current_position = self.parse_info.current_position;
        let state = self.state;
        let line_count = self.parse_info.source.line_starts.borrow().line_starts.len();
        self.next_token();
        // the tokenizer reads at most a few code units after the end of a token: the token is complete when the
        // stream has more code units after it
        if self.parse_info.current_position + STREAM_LOOKAHEAD > self.parse_info.length {
            self.parse_info.position = position;
            self.parse_info.current_position = current_position;
            self.state = state;
            self.parse_info.source.line_starts.borrow_mut().line_starts.truncate(line_count);
            self.typ = None;
            self.waiting = true;
        }
        self
    }

    /// read the next token from the code units of the input
    fn next_token(&mut self) -> &mut Self {
        let end_position = self.parse_info.length.saturating_sub(1);
        loop {
            let mut char_code = self.parse_info.cur();
            if char_code == EOF {
                break;
            }
            if self.state == Some(EnumToken::UrlFunctionTokenDefType) {
                self.state = None;
                return self.consume_url_token(end_position);
            }
            if self.parse_info.position == self.parse_info.current_position {
                if char_code == 0x55 || char_code == 0x75 {
                    let count = self.consume_unicode_range_token();
                    if count > 0 {
                        self.skip(count);
                        return self.make_token(Some(EnumToken::UniCodeRange), None, false);
                    }
                }
                if char_code == MINUS || char_code == PLUS || char_code == DOT || is_digit(char_code) {
                    let count = self.consume_numeric_token();
                    if count > 0 {
                        self.advance(count);
                        let hint = self.hint.unwrap_or(EnumToken::NumberTokenType);
                        let slice = self.slice;
                        return self.make_token(Some(hint), slice, false);
                    }
                }
                if is_ident_start(char_code) || char_code == MINUS {
                    let count = self.consume_ident_token();
                    if count > 0 {
                        self.skip_ident(count);
                        char_code = self.parse_info.cur();
                        if char_code != LEFT_PARENTHESIS {
                            let typ = if self.starts_with("--") { EnumToken::DashedIdenTokenType } else { EnumToken::IdenTokenType };
                            return self.make_token(Some(typ), None, false);
                        }
                    }
                }
                if char_code == AT {
                    self.skip(1);
                    char_code = self.parse_info.cur();
                    if char_code == MINUS || is_ident_start(self.parse_info.cur()) {
                        self.parse_info.position = self.parse_info.current_position;
                        let count = self.consume_ident_token();
                        if count > 0 {
                            self.skip_ident(count);
                            return self.make_token(Some(EnumToken::AtRuleTokenType), None, false);
                        }
                    }
                }
                if char_code == HASH {
                    let count = self.consume_color();
                    if count > 0 {
                        self.skip(count);
                        return self.make_token(Some(EnumToken::ColorTokenType), None, false);
                    }
                    self.skip(1);
                    let count = self.consume_ident_token();
                    if count > 0 {
                        self.skip_ident(count);
                        return self.make_token(Some(EnumToken::HashTokenType), None, false);
                    }
                }
            }
            let pending = self.parse_info.position < self.parse_info.current_position;
            match char_code {
                EQUALS => {
                    self.skip(1);
                    return self.make_token(Some(EnumToken::DelimTokenType), None, false);
                }
                PLUS => {
                    self.skip(1);
                    return self.make_token(Some(EnumToken::Plus), None, false);
                }
                MINUS => {
                    if self.parse_info.position == self.parse_info.current_position {
                        let pi = &self.parse_info;
                        let next = pi.at(pi.current_position - pi.offset + 1);
                        if is_white_space(next) {
                            self.skip(1);
                            return self.make_token(Some(EnumToken::Sub), None, false);
                        }
                    }
                    self.skip(1);
                }
                LEFT_BRACE => {
                    if pending {
                        return self.make_token(None, None, false);
                    }
                    self.skip(1);
                    return self.make_token(Some(EnumToken::BlockStartTokenType), None, false);
                }
                RIGHT_BRACE => {
                    if pending {
                        return self.make_token(None, None, false);
                    }
                    self.skip(1);
                    return self.make_token(Some(EnumToken::BlockEndTokenType), None, false);
                }
                LEFT_PARENTHESIS => {
                    if pending && self.is_ident_token() {
                        let pi = &self.parse_info;
                        let hint = if self.starts_with("--") {
                            EnumToken::CustomFunctionTokenDefType
                        } else {
                            get_symbol_hint(pi, pi.position - pi.offset, pi.current_position - pi.offset + 1)
                                .unwrap_or(EnumToken::FunctionTokenDefType)
                        };
                        self.make_token(Some(hint), None, false);
                        self.skip(1);
                        self.parse_info.position = self.parse_info.current_position;
                        if hint == EnumToken::UrlFunctionTokenDefType {
                            self.state = Some(hint);
                        }
                        return self;
                    }
                    self.skip(1);
                    return self.make_token(Some(EnumToken::StartParensTokenType), None, false);
                }
                RIGHT_PARENTHESIS => {
                    if pending {
                        return self.make_token(None, None, false);
                    }
                    self.skip(1);
                    return self.make_token(Some(EnumToken::EndParensTokenType), None, false);
                }
                LEFT_BRACKETS => {
                    self.skip(1);
                    return self.make_token(Some(EnumToken::AttrStartTokenType), None, false);
                }
                RIGHT_BRACKETS => {
                    self.skip(1);
                    return self.make_token(Some(EnumToken::AttrEndTokenType), None, false);
                }
                SEMICOLON => {
                    self.skip(1);
                    return self.make_token(Some(EnumToken::SemiColonTokenType), None, false);
                }
                COLON => {
                    self.skip(1);
                    if self.parse_info.cur() == COLON {
                        self.skip(1);
                        return self.make_token(Some(EnumToken::DoubleColonTokenType), None, false);
                    }
                    return self.make_token(Some(EnumToken::ColonTokenType), None, false);
                }
                0x9 | 0x20 | 0xa | 0xb | 0xc | 0xd | 0x2028 | 0x2029 => {
                    if pending {
                        return self.make_token(None, None, false);
                    }
                    self.advance_code();
                    let mut next = self.parse_info.cur();
                    while next == 0x20 || (0x9..=0xd).contains(&next) || next == 0x2028 || next == 0x2029 {
                        self.advance_code();
                        next = self.parse_info.cur();
                    }
                    return self.make_token(Some(EnumToken::WhitespaceTokenType), None, false);
                }
                COMMA => {
                    self.skip(1);
                    return self.make_token(Some(EnumToken::CommaTokenType), None, false);
                }
                DOLLAR => {
                    if self.matches("$=") {
                        self.skip(2);
                        return self.make_token(Some(EnumToken::EndMatchTokenType), None, false);
                    }
                    self.skip(1);
                }
                TILDA => {
                    if self.matches("~=") {
                        self.skip(2);
                        return self.make_token(Some(EnumToken::IncludeMatchTokenType), None, false);
                    }
                    self.skip(1);
                    return self.make_token(Some(EnumToken::Tilda), None, false);
                }
                CARET => {
                    if self.matches("^=") {
                        self.skip(2);
                        return self.make_token(Some(EnumToken::StartMatchTokenType), None, false);
                    }
                    self.skip(1);
                }
                STAR => {
                    if self.matches("*=") {
                        self.skip(2);
                        return self.make_token(Some(EnumToken::ContainMatchTokenType), None, false);
                    }
                    self.skip(1);
                    return self.make_token(Some(EnumToken::Star), None, false);
                }
                AMPERSAND => {
                    if pending {
                        return self.make_token(None, None, false);
                    }
                    self.skip(1);
                    return self.make_token(Some(EnumToken::NestingSelectorTokenType), None, false);
                }
                PIPE => {
                    if self.matches("||") {
                        self.skip(2);
                        return self.make_token(Some(EnumToken::ColumnCombinatorTokenType), None, false);
                    } else if self.matches("|=") {
                        self.skip(2);
                        return self.make_token(Some(EnumToken::DashMatchTokenType), None, false);
                    }
                    self.skip(1);
                    return self.make_token(Some(EnumToken::PipeTokenType), None, false);
                }
                EXCLAMATION => {
                    if pending {
                        return self.make_token(None, None, false);
                    }
                    if self.matches("!important") {
                        self.skip(10);
                        return self.make_token(Some(EnumToken::ImportantTokenType), None, false);
                    }
                    self.skip(1);
                }
                SLASH => {
                    if !self.matches("/*") {
                        self.skip(1);
                        let pi = &self.parse_info;
                        let hint = get_symbol_hint(pi, pi.position - pi.offset, pi.current_position - pi.offset);
                        return self.make_token(hint, None, false);
                    }
                    self.skip(2);
                    loop {
                        let c = self.advance_code();
                        if c == EOF {
                            break;
                        }
                        if c == STAR && self.matches("/") {
                            self.skip(1);
                            return self.make_token(Some(EnumToken::CommentTokenType), None, false);
                        }
                    }
                    if self.parse_info.position < self.parse_info.current_position {
                        return self.make_token(Some(EnumToken::BadCommentTokenType), None, false);
                    }
                }
                GREATERTHAN => {
                    if self.matches(">=") {
                        self.skip(2);
                        return self.make_token(Some(EnumToken::GteTokenType), None, false);
                    }
                    self.skip(1);
                    return self.make_token(Some(EnumToken::GtTokenType), None, false);
                }
                LOWERTHAN => {
                    if pending {
                        return self.make_token(None, None, false);
                    }
                    if self.matches("<=") {
                        self.skip(2);
                        return self.make_token(Some(EnumToken::LteTokenType), None, false);
                    }
                    self.skip(1);
                    if self.matches("!--") {
                        self.skip(3);
                        loop {
                            let c = self.advance_code();
                            if c == EOF {
                                break;
                            }
                            if c == MINUS && self.matches("->") {
                                break;
                            }
                        }
                        if self.parse_info.current_position >= end_position {
                            return self.make_token(Some(EnumToken::BadCdoTokenType), None, false);
                        }
                        self.skip(2);
                        return self.make_token(Some(EnumToken::CDOCOMMTokenType), None, false);
                    }
                    let pi = &self.parse_info;
                    let start_position = pi.position - pi.offset;
                    let mut offset = 1;
                    let mut c = pi.at(start_position + offset);
                    if is_letter(c) {
                        offset += 1;
                        c = pi.at(offset + start_position);
                        while offset < end_position && (c == MINUS || is_letter(c)) {
                            offset += 1;
                            c = pi.at(offset + start_position);
                        }
                        if offset > 1 && c == GREATERTHAN {
                            self.skip(offset);
                            return self.make_token(Some(EnumToken::CssTypeTokenType), None, false);
                        }
                    }
                }
                HASH => {
                    if pending {
                        return self.make_token(None, None, false);
                    }
                    self.skip(1);
                }
                REVERSE_SOLIDUS => {
                    self.advance(1);
                    if self.parse_info.cur() == EOF && self.parse_info.position < self.parse_info.current_position {
                        return self.make_token(None, None, false);
                    }
                    self.advance(1);
                }
                SINGLE_QUOTE | DOUBLE_QUOTE => {
                    if pending {
                        return self.make_token(None, None, false);
                    }
                    return self.consume_string();
                }
                DOT => {
                    let pi = &self.parse_info;
                    let codepoint = pi.at(pi.current_position - pi.offset + 1);
                    if is_ident_start(codepoint) || codepoint == MINUS {
                        self.skip(1);
                        let count = self.consume_ident_token();
                        if count > 0 {
                            self.skip_ident(count);
                            return self.make_token(Some(EnumToken::ClassSelectorTokenType), None, false);
                        }
                    }
                    self.advance_code();
                }
                _ => self.skip(1),
            }
        }
        if self.parse_info.position < self.parse_info.current_position {
            return self.make_token(None, None, false);
        }
        self.make_token(Some(EnumToken::EOFTokenType), None, false)
    }
}

/// token type of a symbol, unit or function name
fn get_symbol_hint(parse_info: &ParseInfo, start: usize, end: usize) -> Option<u8> {
    let end = end.min(parse_info.length);
    if end <= start {
        return None;
    }
    tables().symbols.get(&parse_info.codes, start, end).copied()
}

/// canonical spelling of a unit or a function name of the given token type, -1 when the name is not known
fn find_item(typ: u8, parse_info: &ParseInfo, start: usize, end: usize) -> i64 {
    if end <= start {
        return -1;
    }
    tables()
        .items
        .get(&parse_info.codes, start, end)
        .and_then(|items| items.iter().find(|(t, _)| *t == typ))
        .map_or(-1, |(_, c)| *c as i64)
}

#[allow(dead_code)]
static _UNUSED: OnceLock<()> = OnceLock::new();

#[cfg(test)]
mod tests {
    use super::*;

    /// the numbers read by `parse_short_number` have the value read by `parse_long_number`
    #[test]
    fn short_numbers() {
        let mut inputs: Vec<String> = [
            "0", "-0", "+0", "1", "-1", "+1", "1.", "-1.", ".5", "-.5", "+.5", "0.1", "0.2", "0.3", "1.5", "12.345",
            "123456789012345", "12345678901234.5", "0.000000000000001", "9.99999999999999", "33.333333333333",
            "1e3", "1.e3", "1.5e-3", "1em", "10px", "1.2.3", ".", "-", "+-1", "007", "0.30000000000000004",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        // pseudo random numbers
        let mut seed: u64 = 0x2545f4914f6cdd1d;
        for _ in 0..20000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let digits = (seed % 15 + 1) as usize;
            let mantissa = format!("{:015}", seed % 1_000_000_000_000_000);
            let mantissa = &mantissa[15 - digits..];
            let dot = (seed >> 20) as usize % (digits + 1);
            let sign = ["", "-", "+"][(seed >> 40) as usize % 3];
            inputs.push(format!("{}{}.{}", sign, &mantissa[..dot], &mantissa[dot..]));
            inputs.push(format!("{}{}", sign, mantissa));
        }
        for input in inputs {
            let codes = CodeUnits::Bytes(Rc::from(input.as_bytes()));
            let long = parse_long_number(&codes, 0, input.len());
            if let Some(short) = parse_short_number(&codes, 0, input.len()) {
                assert!(short.to_bits() == long.to_bits(), "{}: {} != {}", input, short, long);
            }
        }
    }
}
