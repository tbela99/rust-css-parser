//! Port of `src/validation/match.ts` (`match` is a rust keyword: the module is `match_`).
//!
//! Matches value tokens of the arena against the parsed value definition syntaxes.
//!
//! - a [`Context`] is a shared mutable cursor (`Rc<RefCell<..>>`, like the typescript context objects that are passed
//!   around and advanced in place). `slice()` shares the token buffer (offset), it does not copy the tokens
//! - syntax identity (typescript object identity) is the `Rc` address; the stacks keep the syntaxes alive

use super::config::{get_parsed_syntax, get_syntax_config};
use super::first::{is_nullable, matching_alternatives, may_start_with};
use super::parser::typedef::{MediaFeatureType, ValidationSyntaxGroupEnum, ValidationTokenEnum as V};
use super::parser::types::{MatchMax, ValidationToken};
use crate::ast::arena::AstArena;
use crate::ast::strings::StringDerived;
use crate::ast::types::EnumToken;
use crate::options::ParserOptions;
use crate::parser::utils::text::equals_ignore_case;
use crate::syntax::constants::{is_function_def_type, TOKENS_FUNC_DEF_MAP};
use crate::types::{ErrorAction, ErrorDescription, ErrorSyntax};
use std::cell::RefCell;
use crate::utils::fx::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;

pub type Syntax = Rc<ValidationToken>;

/// css wide keywords (`all` syntax)
fn all_values() -> &'static [String] {
    static VALUES: OnceLock<Vec<String>> = OnceLock::new();
    VALUES.get_or_init(|| {
        get_syntax_config()["declarations"]["all"]["syntax"]
            .as_str()
            .unwrap_or("")
            .split(|c: char| c.is_whitespace() || c == '|')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    })
}

/// true if the value is a css wide keyword, ignoring the case
pub fn is_css_wide_keyword(value: &str) -> bool {
    all_values().iter().any(|v| equals_ignore_case(v, value))
}

pub const SELECTOR_NODE_TYPES: [u8; 5] = [
    EnumToken::CommaTokenType,
    EnumToken::ColumnCombinatorTokenType,
    EnumToken::ChildCombinatorTokenType,
    EnumToken::NextSiblingCombinatorTokenType,
    EnumToken::SubsequentSiblingCombinatorTokenType,
];

fn trim_whitespace_before(t: u8) -> bool {
    SELECTOR_NODE_TYPES.contains(&t)
        || matches!(
            t,
            EnumToken::DelimTokenType
                | EnumToken::DashMatchTokenType
                | EnumToken::IncludeMatchTokenType
                | EnumToken::ContainMatchTokenType
                | EnumToken::StartMatchTokenType
                | EnumToken::EndMatchTokenType
                | EnumToken::AttrEndTokenType
        )
}

fn trim_whitespace_after(t: u8) -> bool {
    SELECTOR_NODE_TYPES.contains(&t)
        || matches!(
            t,
            EnumToken::DelimTokenType
                | EnumToken::DashMatchTokenType
                | EnumToken::IncludeMatchTokenType
                | EnumToken::ContainMatchTokenType
                | EnumToken::StartMatchTokenType
                | EnumToken::EndMatchTokenType
                | EnumToken::AttrStartTokenType
        )
}

/// `funcTypes`: the function token types
pub fn is_func_type(t: u8) -> bool {
    TOKENS_FUNC_DEF_MAP.iter().any(|(_, v)| *v == t) || t == EnumToken::FunctionTokenType || t == EnumToken::PseudoClassFuncTokenType
}

/// trim leading and trailing whitespace
pub fn trim_array(arena: &AstArena, tokens: &mut Vec<u32>) {
    let start = tokens.iter().position(|&t| arena.data_types.get(t) != EnumToken::WhitespaceTokenType).unwrap_or(tokens.len());
    tokens.drain(..start);
    while tokens.last().is_some_and(|&t| arena.data_types.get(t) == EnumToken::WhitespaceTokenType) {
        tokens.pop();
    }
}

/// `ValidationMediaFeature`
pub fn get_mf_info(feature_name: &str) -> Option<&'static serde_json::Value> {
    get_syntax_config()["mediaFeatures"].get(feature_name.to_lowercase())
}

/// result of [`is_mf_value`]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MFValue {
    pub valid: bool,
    pub success: bool,
    pub is_value_allowed: Option<bool>,
}

/// validate a media feature value
pub fn is_mf_value(arena: &AstArena, feature_name: &str, tokens: &[u32], is_mf_range: bool) -> MFValue {
    let tokens: Vec<u32> = tokens
        .iter()
        .copied()
        .filter(|&t| !matches!(arena.data_types.get(t), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType))
        .collect();
    if feature_name.starts_with("--") || (is_mf_range && (feature_name.starts_with("min-") || feature_name.starts_with("max-"))) {
        return MFValue { valid: true, success: false, is_value_allowed: Some(false) };
    }
    let Some(feature) = get_mf_info(feature_name) else {
        return MFValue { valid: false, success: false, is_value_allowed: None };
    };
    let t = |i: usize| tokens.get(i).map_or(0, |&x| arena.data_types.get(x));
    let one = tokens.len() == 1;
    let success = match feature["type"].as_str().unwrap_or("") {
        MediaFeatureType::BooleanType => {
            one && t(0) == EnumToken::NumberTokenType && matches!(arena.numeric_value.get(tokens[0]), v if v == 0.0 || v == 1.0)
        }
        MediaFeatureType::KeywordType => {
            one && t(0) == EnumToken::IdenTokenType && {
                let v = arena.resolve_string(arena.string_value.get(tokens[0])).to_lowercase();
                feature["values"].as_array().is_some_and(|a| a.iter().any(|x| x.as_str() == Some(v.as_str())))
            }
        }
        MediaFeatureType::LengthType => {
            one && (t(0) == EnumToken::LengthTokenType
                || (t(0) == EnumToken::NumberTokenType && arena.numeric_value.get(tokens[0]) == 0.0 && !arena.has_fraction(tokens[0])))
        }
        MediaFeatureType::IntergerType => {
            one && t(0) == EnumToken::NumberTokenType && !crate::utils::number::js_number_to_string(arena.numeric_value.get(tokens[0])).contains('.')
        }
        MediaFeatureType::NumberType => one && t(0) == EnumToken::NumberTokenType && !arena.has_fraction(tokens[0]),
        MediaFeatureType::StringType => one && t(0) == EnumToken::StringTokenType,
        MediaFeatureType::ResolutionType => one && t(0) == EnumToken::ResolutionTokenType,
        MediaFeatureType::RatioType => {
            (one && t(0) == EnumToken::NumberTokenType)
                || (tokens.len() == 3
                    && t(0) == EnumToken::NumberTokenType
                    && !arena.has_fraction(tokens[0])
                    && t(1) == EnumToken::LiteralTokenType
                    && arena.string_equals(arena.string_value.get(tokens[1]), "/")
                    && t(2) == EnumToken::NumberTokenType
                    && !arena.has_fraction(tokens[2]))
        }
        _ => true,
    };
    MFValue { valid: true, success, is_value_allowed: None }
}

// ---------------------------------------------------------------------------------------------------------------------
// validation context
// ---------------------------------------------------------------------------------------------------------------------

/// cursor over the tokens being validated
#[derive(Clone, Debug)]
pub struct ContextData {
    buffer: Rc<[u32]>,
    start: usize,
    pub index: isize,
}

/// shared validation context (`ValidationContext`)
pub type Context = Rc<RefCell<ContextData>>;

fn is_skipped(arena: &AstArena, token: u32) -> bool {
    matches!(arena.data_types.get(token), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType | EnumToken::CDOCOMMTokenType)
}

impl ContextData {
    fn new(tokens: Vec<u32>, index: isize) -> Self {
        Self { buffer: Rc::from(tokens), start: 0, index }
    }

    /// a context over tokens, at an index (the compiled matchers, see `validation::compiled::runtime`)
    pub(crate) fn from_tokens(tokens: Vec<u32>, index: isize) -> Self {
        Self::new(tokens, index)
    }

    /// the tokens of the context
    #[inline]
    pub fn tokens(&self) -> &[u32] {
        &self.buffer[self.start..]
    }

    #[inline]
    fn get(&self, index: isize) -> Option<u32> {
        if index < 0 {
            None
        } else {
            self.tokens().get(index as usize).copied()
        }
    }

    /// `current()`
    pub fn current(&self) -> Option<u32> {
        if self.index < 0 {
            None
        } else {
            self.get(self.index)
        }
    }

    /// `peek(offset)`: the next significant token
    pub fn peek(&self, arena: &AstArena, mut offset: isize) -> Option<u32> {
        let len = self.tokens().len() as isize;
        let mut index = self.index + 1;
        let mut token = self.get(index);
        while offset >= 0 && index < len {
            while token.is_some_and(|t| is_skipped(arena, t)) {
                index += 1;
                token = self.get(index);
            }
            if token.is_none() || offset == 0 {
                return token;
            }
            offset -= 1;
            index += 1;
            token = self.get(index);
        }
        token
    }

    /// `peekRange(close, counter)`: the tokens until the closing token (balanced parentheses and braces)
    pub fn peek_range(&self, arena: &AstArena, close: u8, mut counter: i32) -> Vec<u32> {
        let tokens = self.tokens();
        let len = tokens.len() as isize;
        let mut index = self.index;
        let mut match_count = 0i32;
        let mut block_count = 0i32;
        let mut result = Vec::new();
        while index + 1 < len && arena.data_types.get(tokens[(index + 1) as usize]) == EnumToken::WhitespaceTokenType {
            index += 1;
        }
        while index + 1 < len {
            index += 1;
            let token = tokens[index as usize];
            let t = arena.data_types.get(token);
            if t == EnumToken::StartParensTokenType || is_function_def_type(t) {
                match_count += 1;
            } else if t == EnumToken::EndParensTokenType {
                match_count -= 1;
            } else if t == EnumToken::BlockStartTokenType {
                block_count += 1;
            } else if t == EnumToken::BlockEndTokenType && block_count > 0 {
                block_count -= 1;
            }
            if match_count >= 0 {
                result.push(token);
            }
            if match_count == 0 && block_count == 0 && close != EnumToken::EndParensTokenType && t == close {
                counter -= 1;
            }
            if match_count <= 0 && counter <= 0 {
                break;
            }
        }
        result
    }

    /// `split(split)`: the remaining tokens split by a token type (balanced parentheses)
    pub fn split(&self, arena: &AstArena, split: u8) -> Vec<Vec<u32>> {
        let tokens = self.tokens();
        let mut index = self.index;
        let mut match_count = 0i32;
        let mut result: Vec<Vec<u32>> = vec![Vec::new()];
        while index + 1 < tokens.len() as isize {
            index += 1;
            let token = tokens[index as usize];
            let t = arena.data_types.get(token);
            if t == EnumToken::StartParensTokenType || is_function_def_type(t) {
                match_count += 1;
            } else if t == EnumToken::EndParensTokenType {
                match_count -= 1;
            }
            result.last_mut().unwrap().push(token);
            if match_count == 0 && t == split {
                result.push(Vec::new());
            }
        }
        result
    }

    pub fn get_remaining_tokens(&self) -> Vec<u32> {
        let start = (self.index + 1).max(0) as usize;
        self.tokens().get(start..).map_or_else(Vec::new, |s| s.to_vec())
    }

    pub fn end(&mut self) {
        self.index = self.tokens().len() as isize + 1;
    }

    /// `next()`: the next significant token
    pub fn next(&mut self, arena: &AstArena) -> Option<u32> {
        self.index += 1;
        let mut token = self.get(self.index);
        while token.is_some_and(|t| {
            matches!(
                arena.data_types.get(t),
                EnumToken::WhitespaceTokenType
                    | EnumToken::CommentTokenType
                    | EnumToken::CDOCOMMTokenType
                    | EnumToken::BadCdoTokenType
                    | EnumToken::BadCommentTokenType
                    | EnumToken::BadStringTokenType
            )
        }) {
            self.index += 1;
            token = self.get(self.index);
        }
        token
    }

    /// `slice()`: a new context over the tokens from the current one (shared buffer)
    pub fn slice(&self) -> ContextData {
        if self.index == -1 {
            ContextData { buffer: self.buffer.clone(), start: self.start, index: -1 }
        } else {
            let start = (self.start as isize + self.index).clamp(0, self.buffer.len() as isize) as usize;
            ContextData { buffer: self.buffer.clone(), start, index: 0 }
        }
    }

    /// `update(token)`: move to a token
    pub fn update(&mut self, token: Option<u32>) {
        if let Some(token) = token {
            if let Some(index) = self.tokens().iter().position(|&t| t == token) {
                self.index = index as isize;
            }
        }
    }

    /// `done()`: no significant token left
    pub fn done(&self, arena: &AstArena) -> bool {
        let tokens = self.tokens();
        let mut index = self.index + 1;
        while index < tokens.len() as isize {
            if matches!(arena.data_types.get(tokens[index as usize]), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType) {
                index += 1;
            } else {
                return false;
            }
        }
        true
    }
}

/// context helpers on the shared handle
pub trait ContextExt {
    fn current(&self) -> Option<u32>;
    fn peek(&self, arena: &AstArena) -> Option<u32>;
    fn next_token(&self, arena: &AstArena) -> Option<u32>;
    fn done(&self, arena: &AstArena) -> bool;
    fn end(&self);
    fn update(&self, token: Option<u32>);
    fn slice(&self) -> Context;
    fn tokens(&self) -> Vec<u32>;
    fn index(&self) -> isize;
}

impl ContextExt for Context {
    fn current(&self) -> Option<u32> {
        self.borrow().current()
    }

    fn peek(&self, arena: &AstArena) -> Option<u32> {
        self.borrow().peek(arena, 0)
    }

    fn next_token(&self, arena: &AstArena) -> Option<u32> {
        self.borrow_mut().next(arena)
    }

    fn done(&self, arena: &AstArena) -> bool {
        self.borrow().done(arena)
    }

    fn end(&self) {
        self.borrow_mut().end();
    }

    fn update(&self, token: Option<u32>) {
        self.borrow_mut().update(token);
    }

    fn slice(&self) -> Context {
        Rc::new(RefCell::new(self.borrow().slice()))
    }

    fn tokens(&self) -> Vec<u32> {
        self.borrow().tokens().to_vec()
    }

    fn index(&self) -> isize {
        self.borrow().index
    }
}

/// create a validation context: the comments are removed, the whitespace and `!important` are trimmed
pub fn create_validation_context(arena: &AstArena, tokens: &[u32]) -> Context {
    let t = |x: u32| arena.data_types.get(x);
    if !tokens.iter().any(|&x| t(x) == EnumToken::CommentTokenType) {
        // no comment: the trimmed range is copied once into the shared buffer of the context
        let mut start = 0;
        let mut end = tokens.len();
        while start < end && t(tokens[start]) == EnumToken::WhitespaceTokenType {
            start += 1;
        }
        while end > start && t(tokens[end - 1]) == EnumToken::WhitespaceTokenType {
            end -= 1;
        }
        if end > start && t(tokens[end - 1]) == EnumToken::ImportantTokenType {
            end -= 1;
            while end > start && t(tokens[end - 1]) == EnumToken::WhitespaceTokenType {
                end -= 1;
            }
        }
        return Rc::new(RefCell::new(ContextData { buffer: Rc::from(&tokens[start..end]), start: 0, index: -1 }));
    }
    let mut filtered: Vec<u32> = tokens.iter().copied().filter(|&t| arena.data_types.get(t) != EnumToken::CommentTokenType).collect();
    trim_array(arena, &mut filtered);
    if filtered.last().is_some_and(|&t| arena.data_types.get(t) == EnumToken::ImportantTokenType) {
        filtered.pop();
        trim_array(arena, &mut filtered);
    }
    Rc::new(RefCell::new(ContextData::new(filtered, -1)))
}

// ---------------------------------------------------------------------------------------------------------------------
// results
// ---------------------------------------------------------------------------------------------------------------------

/// result of a match (`ValidationMatch`)
#[derive(Clone, Debug)]
pub struct ValidationMatch {
    pub success: bool,
    pub valid: bool,
    pub token: Option<u32>,
    pub context: Option<Context>,
    pub syntax_token: Option<Syntax>,
    pub errors: Vec<ErrorDescription>,
}

impl ValidationMatch {
    pub fn new(success: bool, valid: bool, token: Option<u32>, context: Option<Context>, syntax_token: Option<Syntax>, errors: Vec<ErrorDescription>) -> Self {
        Self { success, valid, token, context, syntax_token, errors }
    }

    /// the context of the result (contexts are always set, except for the cached results)
    pub fn ctx(&self) -> &Context {
        self.context.as_ref().expect("validation context")
    }
}

/// `[file, line, column]` of a token
pub fn token_location(arena: &AstArena, token: u32, options: &ParserOptions) -> Option<(Option<Rc<str>>, u32, u32)> {
    let source = options.source.as_ref()?;
    let (line, column) = source.line_starts.borrow().get_offsets(arena.get_span_start(token));
    Some((source.file.clone(), line, column))
}

/// a "drop" error of a token, with its location
pub fn drop_error(arena: &AstArena, message: String, token: u32, options: &ParserOptions) -> ErrorDescription {
    let mut error = ErrorDescription::drop(message).with_node(token);
    error.location = token_location(arena, token, options);
    error
}

/// `EnumToken[type]`
pub fn token_type_name(t: u8) -> &'static str {
    EnumToken::name(t).unwrap_or("undefined")
}

// ---------------------------------------------------------------------------------------------------------------------
// selectors
// ---------------------------------------------------------------------------------------------------------------------

/// match a selector: the tokens are checked and normalized in place (combinators, whitespace)
pub fn match_selector_syntax(
    arena: &AstArena,
    stream: &mut Vec<u32>,
    errors: &mut Vec<ErrorDescription>,
    options: &ParserOptions,
    nested: bool,
) -> (bool, Vec<ErrorDescription>) {
    let fail = |message: String, token: u32| (false, vec![drop_error(arena, message, token, options)]);
    // pooled buffers: the tokens are swapped with the stream, the buffer of the stream returns to the pool
    let mut stack = crate::utils::pool::PooledVec::new();
    let mut tokens = crate::utils::pool::PooledVec::new();
    tokens.reserve(stream.len());
    let mut i = 0usize;
    let mut success = true;
    let t = |x: u32| arena.data_types.get(x);
    let top = |stack: &Vec<u32>| stack.last().map(|&x| t(x));
    while i < stream.len() && matches!(t(stream[i]), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType) {
        i += 1;
    }
    if i < stream.len() {
        match t(stream[i]) {
            EnumToken::Plus
            | EnumToken::Tilda
            | EnumToken::GtTokenType
            | EnumToken::ColumnCombinatorTokenType
            | EnumToken::ChildCombinatorTokenType
            | EnumToken::NextSiblingCombinatorTokenType
            | EnumToken::SubsequentSiblingCombinatorTokenType => {
                if !nested {
                    return fail(format!("Unexpected token {}", token_type_name(t(stream[i]))), stream[i]);
                }
            }
            _ => {}
        }
    }
    while i < stream.len() {
        let token = stream[i];
        if t(token) == EnumToken::Star {
            arena.data_types.set(token, EnumToken::UniversalSelectorTokenType);
        }
        tokens.push(token);
        if is_function_def_type(t(token)) {
            if top(&stack).is_some_and(|x| SELECTOR_NODE_TYPES.contains(&x)) {
                stack.pop();
            }
            stack.push(token);
            i += 1;
            continue;
        }
        if top(&stack).is_some_and(|x| SELECTOR_NODE_TYPES.contains(&x))
            && !SELECTOR_NODE_TYPES.contains(&t(token))
            && !matches!(t(token), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType | EnumToken::CDOCOMMTokenType)
        {
            stack.pop();
        }
        match t(token) {
            EnumToken::PseudoClassFuncTokenType => {
                let syntax = selector_function_syntax(arena, token);
                // the children of the syntax are borrowed: the parsed syntax is shared (Rc)
                let chi: &[Syntax] = syntax.as_ref().and_then(|s| s.first()).map_or(&[], |f| f.children());
                let result = match_all_syntaxes(arena, Some(chi), &create_validation_context(arena, &arena.tokens(token)), options);
                if !result.success {
                    success = false;
                    errors.extend(result.errors);
                }
            }
            EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType | EnumToken::CDOCOMMTokenType => {}
            EnumToken::NestingSelectorTokenType => {
                if !nested && options.nested_rule != Some(true) {
                    return fail("Nesting selector is not allowed".into(), token);
                }
            }
            EnumToken::Plus => {
                arena.data_types.set(token, EnumToken::NextSiblingCombinatorTokenType);
                stack.push(token);
            }
            EnumToken::Tilda => arena.data_types.set(token, EnumToken::SubsequentSiblingCombinatorTokenType),
            EnumToken::GtTokenType => arena.data_types.set(token, EnumToken::ChildCombinatorTokenType),
            EnumToken::UniversalSelectorTokenType => {}
            EnumToken::ColumnCombinatorTokenType
            | EnumToken::ChildCombinatorTokenType
            | EnumToken::DescendantCombinatorTokenType
            | EnumToken::NextSiblingCombinatorTokenType
            | EnumToken::SubsequentSiblingCombinatorTokenType => {
                if stream.get(i + 1).is_some_and(|&n| t(n) == EnumToken::WhitespaceTokenType) {
                    i += 1;
                }
                if top(&stack) == Some(EnumToken::UniversalSelectorTokenType) {
                    stack.pop();
                }
                if top(&stack).is_some_and(|x| SELECTOR_NODE_TYPES.contains(&x)) {
                    return fail(format!("Unexpected combinator {}", token_type_name(t(token))), token);
                }
                stack.push(token);
            }
            EnumToken::CommaTokenType => {
                if top(&stack) == Some(EnumToken::UniversalSelectorTokenType) {
                    stack.pop();
                }
                stack.push(token);
            }
            EnumToken::PipeTokenType
            | EnumToken::DelimTokenType
            | EnumToken::StringTokenType
            | EnumToken::IncludeMatchTokenType
            | EnumToken::ContainMatchTokenType
            | EnumToken::StartMatchTokenType
            | EnumToken::EndMatchTokenType
            | EnumToken::DashMatchTokenType
            | EnumToken::IdenTokenType
            | EnumToken::HashTokenType
            | EnumToken::PseudoElementTokenType
            | EnumToken::PseudoClassTokenType
            | EnumToken::ClassSelectorTokenType => {}
            EnumToken::AttrStartTokenType => stack.push(token),
            EnumToken::AttrEndTokenType => {
                if top(&stack) == Some(EnumToken::UniversalSelectorTokenType) {
                    stack.pop();
                }
                if top(&stack) != Some(EnumToken::AttrStartTokenType) {
                    return fail(format!("Unexpected token {}", token_type_name(t(token))), token);
                }
                let k = tokens.len() - 1;
                let index = tokens.iter().position(|&x| Some(x) == stack.last().copied()).map_or(-1, |p| p as isize);
                let mut slice: Vec<u32> = Vec::new();
                for n in (index + 1) as usize..k {
                    if matches!(t(tokens[n]), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType | EnumToken::CDOCOMMTokenType) {
                        continue;
                    }
                    slice.push(tokens[n]);
                }
                let ty = |s: &Vec<u32>, i: usize| s.get(i).map(|&x| t(x));
                if ty(&slice, 1) == Some(EnumToken::PipeTokenType) {
                    slice.remove(0);
                }
                if ty(&slice, 0) == Some(EnumToken::PipeTokenType) {
                    slice.remove(0);
                }
                if slice.len() == 1 {
                    stack.pop();
                } else {
                    for _ in 0..2 {
                        if !slice.is_empty() {
                            slice.remove(0);
                        }
                    }
                    if ty(&slice, 0) == Some(EnumToken::StringTokenType) {
                        let value = arena.resolve_string(arena.string_value.get(slice[0]));
                        let inner: String = {
                            let chars: Vec<char> = value.chars().collect();
                            if chars.len() >= 2 { chars[1..chars.len() - 1].iter().collect() } else { String::new() }
                        };
                        if !inner.is_empty() && inner.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
                            arena.string_value.set(slice[0], arena.intern_string(&inner));
                            arena.data_types.set(slice[0], EnumToken::IdenTokenType);
                        }
                    }
                    if !slice.is_empty() {
                        slice.remove(0);
                    }
                    if slice.is_empty() {
                        stack.pop();
                    } else {
                        let s0 = slice[0];
                        if t(s0) != EnumToken::IdenTokenType
                            || (!arena.string_equals(arena.string_value.get(s0), "i") && !arena.string_equals(arena.string_value.get(s0), "s"))
                        {
                            return fail(format!("Unexpected token {}", token_type_name(t(s0))), s0);
                        }
                        stack.pop();
                    }
                }
            }
            EnumToken::StartParensTokenType => {
                return fail(format!("Unexpected token {}", token_type_name(t(token))), token);
            }
            EnumToken::EndParensTokenType => {
                if top(&stack) == Some(EnumToken::UniversalSelectorTokenType) {
                    stack.pop();
                }
                if matches!(top(&stack), Some(EnumToken::PseudoClassFunctionTokenDefType) | Some(EnumToken::PseudoElementTokenType)) {
                    let func = *stack.last().unwrap();
                    let index = tokens.iter().position(|&x| x == func).map_or(-1, |p| p as isize);
                    // looked up once per function name (`:name()`), the children of the syntax are borrowed
                    let syntax = selector_function_syntax(arena, func);
                    let chi: &[Syntax] = syntax.as_ref().and_then(|s| s.first()).map_or(&[], |f| f.children());
                    let end = tokens.len().saturating_sub(1);
                    let start = ((index + 1) as usize).min(end);
                    let result = match_all_syntaxes(arena, Some(chi), &create_validation_context(arena, &tokens[start..end]), options);
                    if !result.success {
                        success = false;
                        errors.extend(result.errors);
                    }
                    stack.pop();
                } else {
                    return fail(format!("Unexpected token {}", token_type_name(t(token))), token);
                }
            }
            EnumToken::NumberTokenType | EnumToken::LiteralTokenType | EnumToken::DimensionTokenType
                if top(&stack) == Some(EnumToken::PseudoClassFunctionTokenDefType) => {}
            _ => {
                return fail(format!("Unsupported selector token {}", token_type_name(t(token))), token);
            }
        }
        let len = tokens.len();
        if t(token) == EnumToken::WhitespaceTokenType
            && len >= 2
            && trim_whitespace_after(t(tokens[len - 2]))
            && t(tokens[len - 1]) == EnumToken::WhitespaceTokenType
        {
            tokens.pop();
        } else if trim_whitespace_before(t(token)) && len >= 2 && t(tokens[len - 2]) == EnumToken::WhitespaceTokenType {
            tokens.remove(len - 2);
        }
        i += 1;
    }
    if top(&stack) == Some(EnumToken::UniversalSelectorTokenType) {
        stack.pop();
    }
    if let Some(&last) = stack.last() {
        return fail(format!("Unmatched token {}", token_type_name(t(last))), last);
    }
    // the buffer of the stream is replaced: no copy
    std::mem::swap(stream, &mut *tokens);
    // typescript returns the array of the caller (`{ success, errors }`): the node errors of an invalid rule are the
    // errors of the parse so far. the callers only read them when the selector is not valid: the list is not copied for
    // a valid selector (a copy per selector is quadratic with the number of errors of the stylesheet)
    (success, if success { Vec::new() } else { errors.clone() })
}

// ---------------------------------------------------------------------------------------------------------------------
// declaration validation cache
// ---------------------------------------------------------------------------------------------------------------------

/// error of a failed validation bound to the position of its tokens in the value
#[derive(Clone, Debug)]
struct CachedError {
    error: ErrorDescription,
    node: isize,
    location: isize,
}

#[derive(Clone, Debug)]
struct CachedValidation {
    offset: usize,
    length: usize,
    valid: bool,
    errors: Option<Vec<CachedError>>,
}

#[derive(Default)]
struct ValidationCache {
    by_syntax: HashMap<(usize, usize), (Vec<Syntax>, HashMap<i32, Vec<CachedValidation>>)>,
    pool: Vec<f64>,
}

struct EncodingState {
    caches: HashMap<u64, ValidationCache>,
    epochs: Vec<u64>,
    encoding: Vec<f64>,
    hash: i32,
    /// the encoded value contains a token the compiled matchers do not handle (var(), a custom function call, ...): the
    /// interpreter validates it, see [`compiled_verdict`]
    bail: bool,
}

thread_local! {
    static ENCODING: RefCell<EncodingState> = RefCell::new(EncodingState { caches: HashMap::default(), epochs: Vec::new(), encoding: Vec::with_capacity(512), hash: 0, bail: false });
}

/// `value | 0`
fn to_int32(value: f64) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    let v = value.trunc() % 4294967296.0;
    (if v < 0.0 { v + 4294967296.0 } else { v }) as u32 as i32
}

fn encode(state: &mut EncodingState, value: f64) {
    state.encoding.push(value);
    let v = if value.fract() == 0.0 && value.is_finite() { to_int32(value) } else { to_int32(value * 4096.0) };
    state.hash = (state.hash ^ v).wrapping_mul(0x01000193);
}

fn encode_token(arena: &AstArena, state: &mut EncodingState, token: u32) {
    let typ = arena.data_types.get(token);
    if matches!(
        typ,
        EnumToken::WildCardFunctionTokenType
            | EnumToken::WildCardFunctionTokenDefType
            | EnumToken::CustomFunctionTokenType
            | EnumToken::CustomFunctionTokenDefType
            | EnumToken::BadCdoTokenType
            | EnumToken::BadCommentTokenType
            | EnumToken::BadStringTokenType
    ) {
        state.bail = true;
    }
    encode(state, typ as f64);
    encode(state, arena.name.get(token) as f64);
    encode(state, if typ == EnumToken::StringTokenType { 0.0 } else { arena.string_value.get(token) as f64 });
    encode(state, arena.numeric_value.get(token));
    encode(state, arena.unit.get(token) as f64);
    encode(state, arena.color_type.get(token) as f64);
    if arena.first_token.get(token) != 0 {
        encode(state, -1.0);
        for child in arena.token_iter(token) {
            encode_token(arena, state, child);
        }
        encode(state, -2.0);
    }
    if arena.first_child.get(token) != 0 {
        encode(state, -3.0);
        for child in arena.child_iter(token) {
            encode_token(arena, state, child);
        }
        encode(state, -4.0);
    }
}

/// encode the value tokens: returns the encoding hash
fn encode_value(arena: &AstArena, tokens: &[u32]) {
    ENCODING.with(|e| {
        let mut state = e.borrow_mut();
        state.encoding.clear();
        state.hash = 0x811c9dc5u32 as i32;
        state.bail = false;
        for &t in tokens {
            encode_token(arena, &mut state, t);
        }
    });
}

fn syntaxes_key(syntaxes: &[Syntax]) -> (usize, usize) {
    (syntaxes.as_ptr() as usize, syntaxes.len())
}

/// the cache of the arena strings (by string epoch). the old epochs are dropped
fn cache_of(state: &mut EncodingState, epoch: u64) -> &mut ValidationCache {
    if !state.caches.contains_key(&epoch) {
        state.epochs.push(epoch);
        if state.epochs.len() > 16 {
            let old = state.epochs.remove(0);
            state.caches.remove(&old);
        }
    }
    state.caches.entry(epoch).or_default()
}

/// the cached validation of the current encoding for the syntax: (index in the bucket, validation)
fn find_validation(arena: &AstArena, syntaxes: &[Syntax]) -> Option<CachedValidation> {
    ENCODING.with(|e| {
        let state = e.borrow();
        let cache = state.caches.get(&arena.string_epoch())?;
        let (keep, by_hash) = cache.by_syntax.get(&syntaxes_key(syntaxes))?;
        if keep.len() != syntaxes.len() || !keep.iter().zip(syntaxes).all(|(a, b)| Rc::ptr_eq(a, b)) {
            return None;
        }
        let bucket = by_hash.get(&state.hash)?;
        bucket
            .iter()
            .find(|v| v.length == state.encoding.len() && cache.pool[v.offset..v.offset + v.length] == state.encoding[..])
            .cloned()
    })
}

/// copy the current encoding to the pool of the cache: (offset, length, hash)
fn store_encoding(arena: &AstArena) -> (usize, usize, i32) {
    ENCODING.with(|e| {
        let mut state = e.borrow_mut();
        let encoding = std::mem::take(&mut state.encoding);
        let hash = state.hash;
        let cache = cache_of(&mut state, arena.string_epoch());
        let offset = cache.pool.len();
        cache.pool.extend_from_slice(&encoding);
        state.encoding = encoding;
        (offset, state.encoding.len(), hash)
    })
}

fn cache_validation(arena: &AstArena, syntaxes: &[Syntax], offset: usize, length: usize, hash: i32, valid: bool, errors: Option<Vec<CachedError>>) {
    ENCODING.with(|e| {
        let mut state = e.borrow_mut();
        let cache = cache_of(&mut state, arena.string_epoch());
        let entry = cache.by_syntax.entry(syntaxes_key(syntaxes)).or_insert_with(|| (syntaxes.to_vec(), HashMap::default()));
        if !(entry.0.len() == syntaxes.len() && entry.0.iter().zip(syntaxes).all(|(a, b)| Rc::ptr_eq(a, b))) {
            *entry = (syntaxes.to_vec(), HashMap::default());
        }
        entry.1.entry(hash).or_default().push(CachedValidation { offset, length, valid, errors });
    });
}

/// record the errors of a failure cached without its errors
fn set_cached_errors(arena: &AstArena, syntaxes: &[Syntax], offset: usize, errors: Option<Vec<CachedError>>) {
    ENCODING.with(|e| {
        let mut state = e.borrow_mut();
        let cache = cache_of(&mut state, arena.string_epoch());
        if let Some((_, by_hash)) = cache.by_syntax.get_mut(&syntaxes_key(syntaxes)) {
            for bucket in by_hash.values_mut() {
                for v in bucket.iter_mut() {
                    if v.offset == offset {
                        v.errors = errors;
                        return;
                    }
                }
            }
        }
    });
}

fn collect_tokens(arena: &AstArena, token: u32, result: &mut Vec<u32>) {
    result.push(token);
    for child in arena.token_iter(token) {
        collect_tokens(arena, child, result);
    }
    for child in arena.child_iter(token) {
        collect_tokens(arena, child, result);
    }
}

fn value_tokens(arena: &AstArena, tokens: &[u32]) -> Vec<u32> {
    let mut result = Vec::new();
    for &t in tokens {
        collect_tokens(arena, t, &mut result);
    }
    result
}

fn bind_errors(arena: &AstArena, tokens: &[u32], errors: &[ErrorDescription], options: &ParserOptions) -> Option<Vec<CachedError>> {
    let mut result = Vec::new();
    for error in errors {
        let mut node: isize = -1;
        let mut location: isize = -1;
        if let Some(n) = error.node {
            node = tokens.iter().position(|&t| t == n)? as isize;
        }
        if let Some(loc) = &error.location {
            if node != -1 && token_location(arena, tokens[node as usize], options).as_ref() == Some(loc) {
                location = node;
            } else {
                location = tokens.iter().position(|&t| token_location(arena, t, options).as_ref() == Some(loc))? as isize;
            }
        }
        result.push(CachedError { error: error.clone(), node, location });
    }
    Some(result)
}

fn rebuild_errors(arena: &AstArena, tokens: &[u32], errors: &[CachedError], options: &ParserOptions) -> Vec<ErrorDescription> {
    errors
        .iter()
        .map(|e| {
            let mut error = e.error.clone();
            if e.node != -1 {
                error.node = Some(tokens[e.node as usize]);
            }
            if e.location != -1 {
                error.location = token_location(arena, tokens[e.location as usize], options);
            }
            error
        })
        .collect()
}

thread_local! {
    /// number of declaration values validated (cache misses), see [`declaration_validations`]
    static DECLARATION_VALIDATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// number of declaration values validated on this thread: an identical value is validated once
pub fn declaration_validations() -> usize {
    DECLARATION_VALIDATIONS.with(|c| c.get())
}

fn count_validation() {
    DECLARATION_VALIDATIONS.with(|c| c.set(c.get() + 1));
}

// ---------------------------------------------------------------------------------------------------------------------
// compiled matchers
// ---------------------------------------------------------------------------------------------------------------------

/// the number of values validated by the compiled matchers, see [`compiled_stats`]
#[derive(Clone, Debug, Default)]
pub struct CompiledStats {
    /// valid: the answer of the compiled matcher is final
    pub valid: usize,
    /// deferred to the interpreter
    pub deferred: usize,
    /// deferred: the value contains var(), a custom function, ...
    pub bail: usize,
    /// deferred: the syntax is not compiled
    pub uncompiled: usize,
    /// deferred: the compiled matcher rejected the value, the interpreter accepted it (verified mode)
    pub rejected_valid: usize,
    /// (syntax, value) of the values rejected by a compiled matcher and accepted by the interpreter (verified mode)
    pub rejected_valid_samples: Vec<(String, String)>,
    /// (syntax, value) of the values accepted by a compiled matcher and rejected by the interpreter (verified mode)
    pub mismatches: Vec<(String, String)>,
}

thread_local! {
    /// 0: the compiled matchers are not used, 1: used (a valid answer is final), 2: verified by the interpreter
    static COMPILED_MODE: std::cell::Cell<u8> = const { std::cell::Cell::new(1) };
    static COMPILED_STATS: RefCell<CompiledStats> = RefCell::new(CompiledStats::default());
    /// the compiled definition of the syntaxes (by address), None when they are not compiled
    static COMPILED_DEFINITIONS: RefCell<HashMap<(usize, usize), Option<u32>>> = RefCell::new(HashMap::default());
}

/// use the compiled matchers (see [`crate::validation::compiled`]): 0 not used, 1 used, 2 verified by the interpreter
/// (see [`compiled_stats`])
pub fn set_compiled_validation(mode: u8) {
    COMPILED_MODE.with(|m| m.set(mode));
}

/// the number of values validated by the compiled matchers on this thread, and the disagreements with the interpreter
/// (verified mode, see [`set_compiled_validation`])
pub fn compiled_stats() -> CompiledStats {
    COMPILED_STATS.with(|s| s.borrow().clone())
}

/// the text of a value, for the reports
fn value_text(arena: &AstArena, tokens: &[u32]) -> String {
    tokens.iter().map(|&t| crate::printer::render::render_value_default(arena, t)).collect()
}

/// true if the value is valid according to the compiled matcher of the syntax. false: the interpreter must validate it
/// (the value may be valid). the value must be encoded (see [`encode_value`])
fn compiled_verdict(arena: &AstArena, syntaxes: &[Syntax], tokens: &[u32], options: &ParserOptions) -> bool {
    let mode = COMPILED_MODE.with(|m| m.get());
    if mode == 0 {
        return false;
    }
    if ENCODING.with(|e| e.borrow().bail) {
        COMPILED_STATS.with(|s| {
            let mut s = s.borrow_mut();
            s.deferred += 1;
            s.bail += 1;
        });
        return false;
    }
    let definition = COMPILED_DEFINITIONS.with(|c| {
        *c.borrow_mut()
            .entry(syntaxes_key(syntaxes))
            .or_insert_with(|| super::config::syntax_key(syntaxes).and_then(|key| super::compiled::runtime::compiled_definition(&key)))
    });
    let Some(definition) = definition else {
        COMPILED_STATS.with(|s| {
            let mut s = s.borrow_mut();
            s.deferred += 1;
            s.uncompiled += 1;
        });
        return false;
    };
    let valid = super::compiled::runtime::match_compiled(arena, definition, tokens, options);
    if mode == 2 {
        // the validation may encode other values
        let saved = ENCODING.with(|e| {
            let e = e.borrow();
            (e.encoding.clone(), e.hash, e.bail)
        });
        let interpreted = match_all_syntaxes(arena, Some(syntaxes), &create_validation_context(arena, tokens), options).success;
        ENCODING.with(|e| {
            let mut e = e.borrow_mut();
            e.encoding = saved.0;
            e.hash = saved.1;
            e.bail = saved.2;
        });
        if valid != interpreted {
            let sample = (super::config::syntax_key(syntaxes).unwrap_or_default(), value_text(arena, tokens));
            COMPILED_STATS.with(|s| {
                let mut s = s.borrow_mut();
                if valid {
                    s.mismatches.push(sample);
                } else {
                    s.rejected_valid += 1;
                    s.rejected_valid_samples.push(sample);
                }
            });
        }
    }
    COMPILED_STATS.with(|s| {
        let mut s = s.borrow_mut();
        if valid {
            s.valid += 1;
        } else {
            s.deferred += 1;
        }
    });
    valid
}

/// validate the value of a declaration. the successful and the failed validations are cached by value encoding
pub fn match_declaration_syntax(arena: &AstArena, syntaxes: &[Syntax], tokens: &[u32], options: &ParserOptions) -> ValidationMatch {
    encode_value(arena, tokens);
    let cached = find_validation(arena, syntaxes);
    if let Some(cached) = &cached {
        if cached.valid {
            return ValidationMatch::new(true, true, None, None, None, Vec::new());
        }
        if let Some(errors) = &cached.errors {
            return ValidationMatch::new(false, true, None, None, None, rebuild_errors(arena, &value_tokens(arena, tokens), errors, options));
        }
    }
    if cached.is_none() && compiled_verdict(arena, syntaxes, tokens, options) {
        let (offset, length, hash) = store_encoding(arena);
        count_validation();
        cache_validation(arena, syntaxes, offset, length, hash, true, None);
        return ValidationMatch::new(true, true, None, None, None, Vec::new());
    }
    // the encoding is stored before the validation, which may encode other values
    let (offset, length, hash) = store_encoding(arena);
    count_validation();
    let result = match_all_syntaxes(arena, Some(syntaxes), &create_validation_context(arena, tokens), options);
    if result.success {
        cache_validation(arena, syntaxes, offset, length, hash, true, None);
    } else {
        let errors = bind_errors(arena, &value_tokens(arena, tokens), &result.errors, options);
        if let Some(cached) = &cached {
            // failure cached without its errors
            set_cached_errors(arena, syntaxes, cached.offset, errors);
        } else {
            cache_validation(arena, syntaxes, offset, length, hash, false, errors);
        }
    }
    result
}

/// validate the value tokens of a declaration node. the validations are cached
pub fn is_valid_declaration_value(arena: &AstArena, syntaxes: &[Syntax], declaration: u32, options: &ParserOptions) -> bool {
    let tokens = arena.tokens(declaration);
    encode_value(arena, &tokens);
    if let Some(cached) = find_validation(arena, syntaxes) {
        return cached.valid;
    }
    if compiled_verdict(arena, syntaxes, &tokens, options) {
        let (offset, length, hash) = store_encoding(arena);
        count_validation();
        cache_validation(arena, syntaxes, offset, length, hash, true, None);
        return true;
    }
    let (offset, length, hash) = store_encoding(arena);
    count_validation();
    let valid = match_all_syntaxes(arena, Some(syntaxes), &create_validation_context(arena, &tokens), options).success;
    cache_validation(arena, syntaxes, offset, length, hash, valid, None);
    valid
}

// ---------------------------------------------------------------------------------------------------------------------
// matching
// ---------------------------------------------------------------------------------------------------------------------

struct Visited {
    entries: Vec<(u32, Syntax)>,
    base: usize,
}

thread_local! {
    /// the syntaxes being matched and the token they are matched at: a syntax matched again at the same token while
    /// it is being matched is a cyclic definition
    static VISITED: RefCell<Visited> = const { RefCell::new(Visited { entries: Vec::new(), base: 0 }) };
    /// the wildcard functions being matched with a syntax
    static WILDCARD_PROBES: RefCell<HashMap<u32, Syntax>> = RefCell::new(HashMap::default());
}

/// true if a syntax is being matched at a token
pub(crate) fn is_visited(token: u32, syntax: &Syntax) -> bool {
    VISITED.with(|v| {
        let v = v.borrow();
        v.entries[v.base..].iter().any(|(t, s)| *t == token && Rc::ptr_eq(s, syntax))
    })
}

/// record that a syntax is being matched at a token
pub(crate) fn enter_syntax(token: u32, syntax: &Syntax) -> usize {
    VISITED.with(|v| {
        let mut v = v.borrow_mut();
        v.entries.push((token, syntax.clone()));
        v.entries.len() - 1
    })
}

/// leave the syntaxes entered after a mark
pub(crate) fn leave_syntax(length: usize) {
    VISITED.with(|v| v.borrow_mut().entries.truncate(length));
}

/// mark a visited entry as matched (`visitedSyntaxes[i] = null`)
pub(crate) fn clear_visited(index: usize) {
    VISITED.with(|v| {
        if let Some(e) = v.borrow_mut().entries.get_mut(index) {
            e.0 = u32::MAX;
        }
    });
}

/// match all the syntaxes: the tokens must be consumed
pub fn match_all_syntaxes(arena: &AstArena, syntaxes: Option<&[Syntax]>, context: &Context, options: &ParserOptions) -> ValidationMatch {
    let (base, length) = VISITED.with(|v| {
        let mut v = v.borrow_mut();
        let state = (v.base, v.entries.len());
        v.base = v.entries.len();
        state
    });
    let result = match_syntax(arena, syntaxes, context, options);
    VISITED.with(|v| {
        let mut v = v.borrow_mut();
        v.base = base;
        v.entries.truncate(length);
    });
    if let Some(syntaxes) = syntaxes {
        if result.success {
            if let Some(syntax_token) = &result.syntax_token {
                if let Some(index) = syntaxes.iter().position(|s| Rc::ptr_eq(s, syntax_token)) {
                    let mandatory = find_mandatory_syntax(syntaxes, index);
                    if mandatory != -1 {
                        return ValidationMatch::new(
                            false,
                            result.valid,
                            result.token,
                            result.context.clone(),
                            Some(syntaxes[mandatory as usize].clone()),
                            result.errors,
                        );
                    }
                }
            }
        }
        if result.success && !result.ctx().done(arena) {
            let token = result.ctx().peek(arena);
            let mut errors = Vec::new();
            if let Some(token) = token {
                errors.push(drop_error(arena, "unexpected token".into(), token, options));
            }
            return ValidationMatch::new(false, result.valid, token, result.context.clone(), None, errors);
        }
    }
    let errors = if !result.success && result.errors.is_empty() {
        let location_token = result.token.or_else(|| context.borrow().tokens().last().copied());
        let mut error = ErrorDescription::drop("could not match syntax");
        error.node = result.token;
        error.syntax = result.syntax_token.clone().map(ErrorSyntax::Token);
        error.location = location_token.and_then(|t| token_location(arena, t, options));
        vec![error]
    } else {
        result.errors
    };
    ValidationMatch::new(
        result.success,
        result.valid,
        result.token,
        result.context,
        if !result.success { result.syntax_token } else { None },
        errors,
    )
}

/// number of the mandatory commas of a list item
fn count_item_commas(syntax: &ValidationToken) -> i32 {
    if syntax.typ == V::Bracket {
        syntax.children().iter().filter(|t| t.typ == V::Comma).count() as i32
    } else {
        0
    }
}

fn match_max_finite(syntax: &ValidationToken) -> Option<f64> {
    match syntax.match_.as_ref().map(|m| &m.max) {
        Some(MatchMax::Token(t)) if t.val_num().is_finite() => Some(t.val_num()),
        _ => None,
    }
}

/// matches a list (`#`) of a syntax
pub(crate) fn match_list_syntax(arena: &AstArena, syntax: &Syntax, context: &Context, options: &ParserOptions) -> ValidationMatch {
    let mut rest = (**syntax).clone();
    rest.is_list = None;
    rest.match_ = None;
    rest.is_optional = None;
    let rest = Rc::new(rest);
    let mut success = false;
    let mut result: Option<ValidationMatch> = None;
    let mut count = 0.0;
    let commas = count_item_commas(&rest);
    let rest_list = [rest.clone()];
    loop {
        let range = context.borrow().peek_range(arena, EnumToken::CommaTokenType, 1 + commas);
        let last_is_comma = range.last().is_some_and(|&t| arena.data_types.get(t) == EnumToken::CommaTokenType);
        let item: &[u32] = if last_is_comma { &range[..range.len() - 1] } else { &range };
        let tmp = match_syntax(arena, Some(&rest_list), &create_validation_context(arena, item), options);
        if tmp.success && !tmp.ctx().done(arena) {
            if let Some(current) = tmp.ctx().current() {
                count += 1.0;
                success = true;
                result = Some(tmp.clone());
                context.update(Some(current));
            }
            break;
        }
        let mut done = false;
        if tmp.success {
            count += 1.0;
            success = true;
            result = Some(tmp.clone());
            if let Some(max) = match_max_finite(syntax) {
                if count == max {
                    let target = if last_is_comma { range.len().checked_sub(2).map(|i| range[i]) } else { range.last().copied() };
                    context.update(target);
                    break;
                }
            }
            context.update(range.last().copied());
            done = context.done(arena);
        }
        if !(tmp.success && !done) {
            break;
        }
    }
    if result.is_some() {
        if let Some(m) = &syntax.match_ {
            let min = m.min.val_num();
            if !min.is_nan() && count < min {
                result = None;
            }
        }
    }
    match result {
        None => ValidationMatch::new(false, true, context.peek(arena), Some(context.clone()), Some(syntax.clone()), Vec::new()),
        Some(mut r) => {
            r.success = success;
            r.context = Some(context.clone());
            r.token = context.peek(arena);
            r
        }
    }
}

/// matches the occurrences (`{min,max}`) of a syntax
pub(crate) fn match_occurence_syntax(arena: &AstArena, syntax: &Syntax, context: &Context, options: &ParserOptions) -> ValidationMatch {
    let mut rest = (**syntax).clone();
    rest.match_ = None;
    let rest_list = [Rc::new(rest)];
    let mut result: Option<ValidationMatch> = None;
    let mut count = 0.0;
    loop {
        let tmp = match_syntax(arena, Some(&rest_list), &context.slice(), options);
        if tmp.success {
            count += 1.0;
            result = Some(tmp.clone());
            if tmp.ctx().done(arena) {
                context.end();
                break;
            }
            if tmp.ctx().current() == context.current() {
                break;
            }
            context.update(tmp.ctx().current());
            if let Some(max) = match_max_finite(syntax) {
                if count == max {
                    break;
                }
            }
        }
        if !(tmp.success && !context.done(arena)) {
            break;
        }
    }
    let out_of_range = syntax.match_.as_ref().is_some_and(|m| {
        count < m.min.val_num() || match_max_finite(syntax).is_some_and(|max| count > max)
    });
    match result {
        Some(r) if !out_of_range => r,
        _ => {
            let mut error = ErrorDescription::drop("could not match syntax");
            error.node = context.peek(arena);
            ValidationMatch::new(false, true, None, Some(context.clone()), None, vec![error])
        }
    }
}

/// index of the first syntax that must be matched, starting at index. -1 if the remaining syntaxes may match no token
pub(crate) fn find_mandatory_syntax(syntaxes: &[Syntax], index: usize) -> isize {
    (index..syntaxes.len()).find(|&i| !is_nullable(&syntaxes[i])).map_or(-1, |i| i as isize)
}

/// true if a token is a literal number or dimension of a syntax: `0`, `90deg`
pub(crate) fn is_literal_number(arena: &AstArena, token: u32, syntax: &ValidationToken) -> bool {
    if arena.has_fraction(token) || arena.numeric_value.get(token) != syntax.val_num() {
        return false;
    }
    if syntax.typ == V::Number {
        return arena.data_types.get(token) == EnumToken::NumberTokenType;
    }
    let typ = syntax.unit.as_deref().and_then(EnumToken::value);
    Some(arena.data_types.get(token)) == typ
        && arena.with_string(arena.unit.get(token), |u| equals_ignore_case(u, syntax.unit_text.as_deref().unwrap_or("")))
}

/// true if a numeric token is in the range of the syntax: `<integer [1,∞]>`
pub(crate) fn is_in_range(arena: &AstArena, token: u32, syntax: &ValidationToken) -> bool {
    let Some(range) = &syntax.range else {
        return true;
    };
    if matches!(
        arena.data_types.get(token),
        EnumToken::MathFunctionTokenType
            | EnumToken::MathFunctionTokenDefType
            | EnumToken::WildCardFunctionTokenType
            | EnumToken::WildCardFunctionTokenDefType
    ) {
        return true;
    }
    let value = arena.numeric_value.get(token);
    let mut success = !arena.has_fraction(token) && range.min.as_ref().is_none_or(|m| value >= m.val_num());
    if let Some(min) = &range.min {
        if min.typ == V::Dimension {
            success = Some(arena.data_types.get(token)) == min.unit.as_deref().and_then(EnumToken::value);
        }
    }
    if success {
        if let Some(max) = &range.max {
            success = value <= max.val_num();
        }
    }
    success
}

/// the validation syntax of a function token (`name()`): looked up once per function name
pub(crate) fn function_syntax(arena: &AstArena, token: u32) -> Option<Rc<Vec<Syntax>>> {
    arena.string_derived(arena.string_value.get(token), StringDerived::FunctionSyntax, |id| {
        get_parsed_syntax(ValidationSyntaxGroupEnum::Syntaxes, &format!("{}()", arena.resolve_string(id)))
    })
}

/// the validation syntax of a selector function token (`:name()`)
pub(crate) fn selector_function_syntax(arena: &AstArena, token: u32) -> Option<Rc<Vec<Syntax>>> {
    arena.string_derived(arena.string_value.get(token), StringDerived::SelectorFunctionSyntax, |id| {
        get_parsed_syntax(ValidationSyntaxGroupEnum::Selectors, &format!("{}()", arena.resolve_string(id)))
    })
}

/// var(), env() and if() may be substituted by any number of values. None: the syntax at index is skipped
#[allow(clippy::too_many_arguments)]
pub(crate) fn match_wild_card_function(
    arena: &AstArena,
    syntaxes: &[Syntax],
    index: usize,
    context: &Context,
    token: u32,
    end: Option<u32>,
    options: &ParserOptions,
) -> Option<ValidationMatch> {
    let mut errors: Vec<ErrorDescription> = Vec::new();
    let mut candidates: Vec<Context> = Vec::new();
    let syntax = &syntaxes[index];
    if !matches!(syntax.typ, V::Keyword | V::Comma | V::Separator | V::StringToken | V::OpenParenthesis | V::CloseParenthesis) {
        let previous = WILDCARD_PROBES.with(|w| w.borrow_mut().insert(token, syntax.clone()));
        let probe = match_syntax(arena, Some(std::slice::from_ref(syntax)), &context.slice(), options);
        WILDCARD_PROBES.with(|w| {
            let mut w = w.borrow_mut();
            match previous {
                None => w.remove(&token),
                Some(p) => w.insert(token, p),
            }
        });
        if probe.success {
            if probe.ctx().done(arena) {
                if find_mandatory_syntax(syntaxes, index + 1) == -1 {
                    context.end();
                    return Some(ValidationMatch::new(true, probe.valid, None, Some(context.clone()), None, probe.errors));
                }
            } else if probe.ctx().current().is_some() {
                candidates.push(probe.ctx().clone());
            }
        }
    }
    let next = context.slice();
    next.update(end);
    if next.done(arena) {
        context.end();
        return Some(ValidationMatch::new(true, true, None, Some(context.clone()), None, Vec::new()));
    }
    candidates.push(next);
    for candidate in &candidates {
        let rest = match_syntax(arena, Some(&syntaxes[index + 1..]), &candidate.slice(), options);
        if rest.success {
            if rest.ctx().done(arena) {
                context.end();
            } else {
                context.update(rest.ctx().current().or_else(|| candidate.current()));
            }
            return Some(ValidationMatch::new(true, rest.valid, rest.token, Some(context.clone()), None, rest.errors));
        }
        errors = rest.errors;
    }
    if is_nullable(syntax) {
        return None;
    }
    Some(ValidationMatch::new(false, true, Some(token), Some(context.clone()), Some(syntax.clone()), errors))
}

/// true if a wildcard function is being probed with a syntax
pub(crate) fn wild_card_probe(token: u32) -> Option<Syntax> {
    WILDCARD_PROBES.with(|w| w.borrow().get(&token).cloned())
}

/// the tokens are consumed: the match succeeds if the remaining syntaxes, starting at index, are optional
pub(crate) fn set_done_match(
    syntaxes: &[Syntax],
    index: usize,
    valid: bool,
    token: Option<u32>,
    context: &Context,
    errors: Vec<ErrorDescription>,
) -> ValidationMatch {
    let mandatory = find_mandatory_syntax(syntaxes, index);
    if mandatory != -1 {
        return ValidationMatch::new(false, valid, token, Some(context.clone()), Some(syntaxes[mandatory as usize].clone()), Vec::new());
    }
    ValidationMatch::new(true, valid, token, Some(context.clone()), syntaxes.get(index).cloned(), errors)
}

// matchSyntax, the unordered syntaxes and matchProperty: see match_syntax.rs
pub use super::match_syntax::{match_syntax, peek_feature_tokens};

#[allow(dead_code)]
fn _unused(_: ErrorAction, _: Option<&serde_json::Value>, _: &dyn Fn() -> bool) {}

#[allow(dead_code)]
fn _use_first(arena: &AstArena, s: &[Vec<Syntax>]) -> Option<Rc<Vec<usize>>> {
    let _ = may_start_with(arena, &[], None);
    matching_alternatives(arena, s, None)
}
