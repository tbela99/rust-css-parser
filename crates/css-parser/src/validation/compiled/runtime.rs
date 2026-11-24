//! Port of `src/validation/compiled/runtime.ts`: the matching engine of the compiled syntaxes.
//!
//! The syntaxes of the validation data are compiled at build time (`npm run syntax-compile`, `scripts/compile_syntax.rs`)
//! into a static table of [`Node`]s ([`super::syntaxes`]). A node matches the tokens of a value from a position and
//! returns the position after the matched tokens, or [`FAIL`]. The positions are indexes in the significant tokens of
//! the value (the whitespace and the comments are skipped, like the interpreter does). `Err(Bail)` aborts the match:
//! the interpreter validates the value.
//!
//! A compiled matcher mirrors the syntax tree matched by the interpreter ([`crate::validation::match_`]). Its "valid"
//! answer is final, its "invalid" answer is not: the interpreter validates the value again and reports the errors.

use super::builtins::is_raw_type;
use super::syntaxes::{KEYS, NODES, ROOTS};
use crate::ast::arena::AstArena;
use crate::ast::types::EnumToken;
use crate::options::ParserOptions;
use crate::parser::utils::text::{equals_ignore_case, lowercase};
use crate::syntax::constants::{is_function_def_type, is_function_type, FUNC_LIKE};
use crate::validation::match_::{is_css_wide_keyword, is_func_type, Context, ContextData, Syntax};
use crate::validation::match_property::match_property;
use crate::validation::parser::typedef::ValidationTokenEnum as V;
use crate::validation::parser::types::{VVal, ValidationToken};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// a syntax node. the children are indexes in [`NODES`], the definitions indexes in [`ROOTS`]
#[derive(Debug)]
pub enum Node {
    /// an empty syntax
    Empty,
    /// a definition the compiler could not compile: the interpreter validates it
    Interpreted,
    /// a sequence: the items, nullable\[i\]: the items from i are nullable
    Seq(&'static [u32], &'static [bool]),
    /// alternatives: the one that matches the most tokens
    Alt(&'static [u32]),
    /// a keyword (lowercase), ignoring the case
    Kw(&'static str),
    /// alternative keywords (lowercase, sorted)
    Kws(&'static [&'static str]),
    /// an optional syntax: skipped when it does not match
    Opt(u32),
    /// `*` (zero allowed) or `+`
    Rep(u32, bool),
    /// `{min,max}`
    Occ(u32, u32, f64),
    /// `#{min,max}`, the number of commas of an item
    List(u32, u32, f64, u32),
    /// a type validated by `match_property()`, the kind of its fast path (see [`leaf_kind`])
    Leaf(&'static str, u8),
    /// a type with a range: the type, min, max, the unit of a dimension range, optional
    Rng(u32, f64, f64, Option<&'static str>, bool),
    /// a reference to a definition
    Ref(u32),
    /// a function (lowercase name) and its arguments
    Fn(&'static str, u32),
    /// a function definition `<rgb()>`: the definition of the function syntax
    FnDef(&'static str, Option<u32>),
    /// a literal number or dimension: the value, the token type name of the unit, the unit
    Num(f64, Option<&'static str>, Option<&'static str>),
    /// a quoted literal: `'+'`
    Str(&'static str),
    Comma,
    Colon,
    Slash,
    OpenParens,
    CloseParens,
    OpenCurly,
    /// `<bg-layer>#? , <final-bg-layer>`: `[ <bg-layer> , ]*`
    OptList(u32),
    /// `||` (any) or `&&` (all): the operands, nullable operands
    Unordered(&'static [u32], &'static [bool], bool),
}

/// no match
pub const FAIL: i32 = -1;

/// the value must be validated by the interpreter
#[derive(Debug)]
pub struct Bail;

type Result<T> = std::result::Result<T, Bail>;

/// the maximum depth of the references being matched: deeper, the interpreter validates the value
const MAX_DEPTH: u32 = 256;

/// the fast path of the numeric types: the tokens that are neither idents (a css wide keyword is accepted by the types)
/// nor math functions are matched without `match_property()`. 0: no fast path
pub fn leaf_kind(name: &str) -> u8 {
    match name {
        "number" => 1,
        "integer" => 2,
        "zero" => 3,
        "length" => 4,
        "percentage" => 5,
        "length-percentage" => 6,
        "angle" => 7,
        "angle-percentage" => 8,
        "time" => 9,
        "flex" => 10,
        "resolution" => 11,
        "frequency" => 12,
        "string" => 13,
        "dashed-ident" | "custom-property-name" => 14,
        _ => 0,
    }
}

/// the fast path of a numeric type, see [`leaf_kind`]
fn fast_leaf(arena: &AstArena, kind: u8, token: u32, t: u8) -> bool {
    let zero = || t == EnumToken::NumberTokenType && arena.numeric_value.get(token) == 0.0;
    match kind {
        1 => t == EnumToken::NumberTokenType,
        2 => t == EnumToken::NumberTokenType && arena.numeric_value.get(token).fract() == 0.0 && arena.numeric_value.get(token).is_finite(),
        3 => zero(),
        4 => t == EnumToken::LengthTokenType || t == EnumToken::UserDefinedDimensionTokenType || zero(),
        5 => t == EnumToken::PercentageTokenType,
        6 => t == EnumToken::PercentageTokenType || t == EnumToken::LengthTokenType || t == EnumToken::UserDefinedDimensionTokenType || zero(),
        7 => t == EnumToken::AngleTokenType || t == EnumToken::UserDefinedDimensionTokenType,
        8 => t == EnumToken::AngleTokenType || t == EnumToken::UserDefinedDimensionTokenType || t == EnumToken::PercentageTokenType,
        9 => t == EnumToken::TimeTokenType || t == EnumToken::UserDefinedDimensionTokenType,
        10 => t == EnumToken::FlexTokenType || t == EnumToken::UserDefinedDimensionTokenType,
        11 => t == EnumToken::ResolutionTokenType,
        12 => t == EnumToken::FrequencyTokenType,
        13 => t == EnumToken::StringTokenType,
        14 => t == EnumToken::DashedIdenTokenType,
        _ => false,
    }
}

thread_local! {
    /// the property syntax passed to `match_property()`, by type name
    static PROPERTIES: RefCell<HashMap<&'static str, Syntax>> = RefCell::new(HashMap::new());
    /// the definitions, by key (`group/name`)
    static DEFINITIONS: HashMap<&'static str, u32> = KEYS.iter().enumerate().map(|(i, k)| (*k, i as u32)).collect();
}

/// the property syntax of a type, see [`PROPERTIES`]
fn property(name: &'static str) -> Syntax {
    PROPERTIES.with(|p| {
        p.borrow_mut()
            .entry(name)
            .or_insert_with(|| {
                let mut token = ValidationToken::new(V::PropertyType);
                token.val = VVal::Str(Rc::from(name));
                Rc::new(token)
            })
            .clone()
    })
}

/// the compiled definition of a key (`declarations/width`), None if it is not compiled
pub fn compiled_definition(key: &str) -> Option<u32> {
    DEFINITIONS.with(|d| d.get(key).copied())
}

/// the significant tokens: the whitespace and the comments are skipped, a trailing `!important` is removed (see
/// `create_validation_context()`)
fn significant_tokens(arena: &AstArena, tokens: &[u32]) -> Vec<u32> {
    let mut result: Vec<u32> = tokens
        .iter()
        .copied()
        .filter(|&t| !matches!(arena.data_types.get(t), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType | EnumToken::CDOCOMMTokenType))
        .collect();
    if result.last().is_some_and(|&t| arena.data_types.get(t) == EnumToken::ImportantTokenType) {
        result.pop();
    }
    result
}

/// the value being matched
struct Matcher<'a> {
    arena: &'a AstArena,
    /// the significant tokens of the value
    toks: Vec<u32>,
    /// the end of the tokens being matched: the items of a list are matched up to the next comma
    len: usize,
    options: &'a ParserOptions,
    depth: u32,
    /// the context passed to `match_property()` and the length of its tokens
    leaf: Option<(Context, usize)>,
}

impl<'a> Matcher<'a> {
    fn new(arena: &'a AstArena, toks: Vec<u32>, options: &'a ParserOptions, depth: u32) -> Self {
        let len = toks.len();
        Matcher { arena, toks, len, options, depth, leaf: None }
    }

    #[inline]
    fn typ(&self, p: i32) -> u8 {
        self.arena.data_types.get(self.toks[p as usize])
    }

    #[inline]
    fn end(&self) -> i32 {
        self.len as i32
    }

    /// the lowercase value of an ident token at a position, compared with a keyword
    fn ident_is(&self, p: i32, f: impl FnOnce(&str) -> bool) -> bool {
        let token = self.toks[p as usize];
        self.arena.data_types.get(token) == EnumToken::IdenTokenType && self.arena.with_string(self.arena.string_value.get(token), |v| f(&lowercase(v)))
    }

    /// the lowercase name of a function token
    fn name_is(&self, p: i32, name: &str) -> bool {
        let token = self.toks[p as usize];
        self.arena.with_string(self.arena.string_value.get(token), |v| lowercase(v) == name)
    }

    /// match a node from a position
    fn m(&mut self, node: u32, p: i32) -> Result<i32> {
        match &NODES[node as usize] {
            Node::Empty => Ok(p),
            Node::Interpreted => Err(Bail),
            Node::Seq(items, nullable) => {
                let mut p = p;
                for (i, &item) in items.iter().enumerate() {
                    if p >= self.end() {
                        return Ok(if nullable[i] { p } else { FAIL });
                    }
                    p = self.m(item, p)?;
                    if p < 0 {
                        return Ok(FAIL);
                    }
                }
                Ok(p)
            }
            Node::Alt(alternatives) => {
                let mut best = FAIL;
                for &alternative in alternatives.iter() {
                    let result = self.m(alternative, p)?;
                    if result > best {
                        best = result;
                        if result >= self.end() {
                            break;
                        }
                    }
                }
                Ok(best)
            }
            Node::Kw(value) => Ok(if self.ident_is(p, |v| v == *value) { p + 1 } else { FAIL }),
            Node::Kws(values) => Ok(if self.ident_is(p, |v| values.binary_search(&v).is_ok()) { p + 1 } else { FAIL }),
            Node::Opt(inner) => {
                let result = self.m(*inner, p)?;
                Ok(if result < 0 { p } else { result })
            }
            Node::Rep(item, zero) => {
                let mut p = p;
                let mut matched = false;
                loop {
                    let result = self.m(*item, p)?;
                    if result < 0 {
                        break;
                    }
                    matched = true;
                    if result >= self.end() {
                        p = self.end();
                        break;
                    }
                    // no token matched
                    if result == p {
                        break;
                    }
                    p = result;
                }
                Ok(if matched || *zero { p } else { FAIL })
            }
            Node::Occ(item, min, max) => {
                let mut p = p;
                let mut count: u32 = 0;
                loop {
                    let result = self.m(*item, p)?;
                    if result < 0 {
                        break;
                    }
                    count += 1;
                    if result >= self.end() {
                        p = self.end();
                        break;
                    }
                    if result == p {
                        break;
                    }
                    p = result;
                    if count as f64 == *max || p >= self.end() {
                        break;
                    }
                }
                Ok(if count == 0 || count < *min || count as f64 > *max { FAIL } else { p })
            }
            Node::List(item, min, max, commas) => {
                let len = self.len;
                let mut p = p;
                let mut count: u32 = 0;
                loop {
                    let end = self.item_end(p, *commas);
                    // Bail aborts the whole match: the length is not restored
                    self.len = end as usize;
                    let result = self.m(*item, p)?;
                    self.len = len;
                    if result < 0 {
                        break;
                    }
                    // the item does not match the tokens before the comma: the list ends with this item
                    if result < end {
                        if result > p {
                            count += 1;
                            p = result;
                        }
                        break;
                    }
                    count += 1;
                    if count as f64 == *max {
                        p = end;
                        break;
                    }
                    // the comma
                    p = if end < len as i32 { end + 1 } else { end };
                    if p >= len as i32 {
                        break;
                    }
                }
                Ok(if count == 0 || count < *min { FAIL } else { p })
            }
            Node::Leaf(name, kind) => self.leaf(name, *kind, p),
            Node::Rng(inner, min, max, unit, optional) => {
                let result = self.m(*inner, p)?;
                if result < 0 {
                    return Ok(if *optional { p } else { FAIL });
                }
                Ok(if self.in_range(self.toks[p as usize], *min, *max, *unit) { result } else { FAIL })
            }
            Node::Ref(definition) => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err(Bail);
                }
                let result = self.m(ROOTS[*definition as usize], p)?;
                self.depth -= 1;
                Ok(result)
            }
            Node::Fn(name, args) => {
                let t = self.typ(p);
                if (!is_function_def_type(t) && !FUNC_LIKE.contains(&t) && !is_func_type(t)) || !self.name_is(p, name) {
                    return Ok(FAIL);
                }
                if is_function_def_type(t) {
                    let close = self.closing_parens(p);
                    return Ok(if close != -1 && self.match_range(*args, p + 1, close)? { close + 1 } else { FAIL });
                }
                let toks = significant_tokens(self.arena, &self.arena.tokens(self.toks[p as usize]));
                let length = toks.len() as i32;
                let mut sub = Matcher::new(self.arena, toks, self.options, self.depth);
                Ok(if sub.m(*args, 0)? == length { p + 1 } else { FAIL })
            }
            Node::FnDef(name, definition) => {
                let Some(definition) = definition else {
                    return Ok(FAIL);
                };
                if !self.name_is(p, name) {
                    return Ok(FAIL);
                }
                let root = ROOTS[*definition as usize];
                let t = self.typ(p);
                if is_function_def_type(t) {
                    let close = self.closing_parens(p);
                    return Ok(if close != -1 && self.match_range(root, p, close + 1)? { close + 1 } else { FAIL });
                }
                if !is_function_type(t) {
                    return Ok(FAIL);
                }
                let mut sub = Matcher::new(self.arena, vec![self.toks[p as usize]], self.options, self.depth);
                Ok(if sub.m(root, 0)? == 1 { p + 1 } else { FAIL })
            }
            Node::Num(value, unit, unit_text) => {
                let token = self.toks[p as usize];
                if self.arena.has_fraction(token) || self.arena.numeric_value.get(token) != *value {
                    return Ok(FAIL);
                }
                let t = self.arena.data_types.get(token);
                Ok(match unit {
                    None => {
                        if t == EnumToken::NumberTokenType {
                            p + 1
                        } else {
                            FAIL
                        }
                    }
                    Some(unit) => {
                        if Some(t) == EnumToken::value(unit)
                            && self.arena.with_string(self.arena.unit.get(token), |u| equals_ignore_case(u, unit_text.unwrap_or("")))
                        {
                            p + 1
                        } else {
                            FAIL
                        }
                    }
                })
            }
            Node::Str(value) => {
                let token = self.toks[p as usize];
                Ok(match self.arena.data_types.get(token) {
                    EnumToken::Add if *value == "+" => p + 1,
                    EnumToken::Sub if *value == "-" => p + 1,
                    EnumToken::LiteralTokenType if self.arena.with_string(self.arena.string_value.get(token), |v| v == *value) => p + 1,
                    EnumToken::Plus => p + 1,
                    _ => FAIL,
                })
            }
            Node::Comma => Ok(self.token_type(p, EnumToken::CommaTokenType)),
            Node::Colon => Ok(self.token_type(p, EnumToken::ColonTokenType)),
            Node::OpenParens => Ok(self.token_type(p, EnumToken::StartParensTokenType)),
            Node::CloseParens => Ok(self.token_type(p, EnumToken::EndParensTokenType)),
            Node::OpenCurly => Ok(self.token_type(p, EnumToken::BlockStartTokenType)),
            Node::Slash => {
                let token = self.toks[p as usize];
                Ok(
                    if self.arena.data_types.get(token) == EnumToken::LiteralTokenType && self.arena.with_string(self.arena.string_value.get(token), |v| v == "/") {
                        p + 1
                    } else {
                        FAIL
                    },
                )
            }
            Node::OptList(group) => {
                let mut p = p;
                while p < self.end() {
                    let result = self.m(*group, p)?;
                    if result <= p {
                        break;
                    }
                    p = result;
                }
                Ok(p)
            }
            Node::Unordered(operands, nullable, all) => {
                let size = self.len + 1;
                // the position after an operand, by operand and position. -2: not matched yet
                let mut memo: Vec<i32> = vec![-2; operands.len() * size];
                let remaining: Vec<usize> = (0..operands.len()).collect();
                let best = self.search(operands, nullable, *all, &remaining, p, &mut memo, size)?;
                if best >= 0 {
                    return Ok(best);
                }
                Ok(if *all && nullable.iter().all(|n| *n) { p } else { FAIL })
            }
        }
    }

    #[inline]
    fn token_type(&self, p: i32, t: u8) -> i32 {
        if self.typ(p) == t {
            p + 1
        } else {
            FAIL
        }
    }

    /// the position after the operands that match the most tokens, -1 if no operand matches
    #[allow(clippy::too_many_arguments)]
    fn search(&mut self, operands: &[u32], nullable: &[bool], all: bool, remaining: &[usize], p: i32, memo: &mut Vec<i32>, size: usize) -> Result<i32> {
        let mut best: i32 = -1;
        for k in 0..remaining.len() {
            let operand = remaining[k];
            let key = operand * size + p as usize;
            let mut next = memo[key];
            if next == -2 {
                next = if p < self.end() { self.m(operands[operand], p)? } else { FAIL };
                // the operand must match a token
                if next <= p {
                    next = -1;
                }
                memo[key] = next;
            }
            if next < 0 {
                continue;
            }
            let rest: Vec<usize> = remaining[..k].iter().chain(remaining[k + 1..].iter()).copied().collect();
            let complete = !all || rest.iter().all(|&i| nullable[i]);
            if next < self.end() && !rest.is_empty() {
                let deeper = self.search(operands, nullable, all, &rest, next, memo, size)?;
                if deeper >= 0 {
                    if deeper >= self.end() {
                        return Ok(deeper);
                    }
                    if deeper > best {
                        best = deeper;
                    }
                }
            }
            if complete {
                if next >= self.end() {
                    return Ok(next);
                }
                if next > best {
                    best = next;
                }
            }
        }
        Ok(best)
    }

    /// the end of a comma separated item: the position of the comma that follows the item
    fn item_end(&self, p: i32, mut commas: u32) -> i32 {
        let mut depth = 0i32;
        for i in p..self.end() {
            let t = self.typ(i);
            if t == EnumToken::StartParensTokenType || is_function_def_type(t) {
                depth += 1;
            } else if t == EnumToken::EndParensTokenType {
                depth -= 1;
            } else if depth == 0 && t == EnumToken::CommaTokenType {
                if commas == 0 {
                    return i;
                }
                commas -= 1;
            }
        }
        self.end()
    }

    /// the position of the parenthesis that closes the function at a position: the arguments of a function follow the
    /// function token (see `peek_range()`). -1 if the function is not closed
    fn closing_parens(&self, p: i32) -> i32 {
        let mut depth = 0i32;
        for i in p..self.end() {
            let t = self.typ(i);
            if t == EnumToken::StartParensTokenType || is_function_def_type(t) {
                depth += 1;
            } else if t == EnumToken::EndParensTokenType {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
        }
        -1
    }

    /// true if a node matches the tokens from p to end (excluded)
    fn match_range(&mut self, node: u32, p: i32, end: i32) -> Result<bool> {
        let len = self.len;
        // Bail aborts the whole match: the length is not restored
        self.len = end as usize;
        let result = self.m(node, p)?;
        self.len = len;
        Ok(result == end)
    }

    /// a type validated by `match_property()`
    fn leaf(&mut self, name: &'static str, kind: u8, p: i32) -> Result<i32> {
        let token = self.toks[p as usize];
        let t = self.arena.data_types.get(token);
        if kind != 0 && t != EnumToken::IdenTokenType && t != EnumToken::MathFunctionTokenType && t != EnumToken::MathFunctionTokenDefType {
            return Ok(if fast_leaf(self.arena, kind, token, t) { p + 1 } else { FAIL });
        }
        if is_raw_type(name) {
            return Err(Bail);
        }
        let context = match &self.leaf {
            Some((context, len)) if *len == self.len => {
                context.borrow_mut().index = p as isize - 1;
                context.clone()
            }
            _ => {
                let context: Context = Rc::new(RefCell::new(ContextData::from_tokens(self.toks[..self.len].to_vec(), p as isize - 1)));
                self.leaf = Some((context.clone(), self.len));
                context
            }
        };
        let result = match_property(self.arena, &property(name), &context, self.options);
        if !result.success {
            return Ok(FAIL);
        }
        let index = context.borrow().index;
        Ok(if index >= self.len as isize { self.end() } else { index as i32 + 1 })
    }

    /// see `is_in_range()`
    fn in_range(&self, token: u32, min: f64, max: f64, unit: Option<&str>) -> bool {
        let t = self.arena.data_types.get(token);
        if matches!(t, EnumToken::MathFunctionTokenType | EnumToken::MathFunctionTokenDefType | EnumToken::WildCardFunctionTokenType | EnumToken::WildCardFunctionTokenDefType) {
            return true;
        }
        let value = self.arena.numeric_value.get(token);
        let mut success = !self.arena.has_fraction(token) && value >= min;
        if let Some(unit) = unit {
            success = Some(t) == EnumToken::value(unit);
        }
        success && value <= max
    }
}

/// match the tokens of a value with a compiled definition
///
/// returns true if the value is valid, false if the interpreter must validate it
pub fn match_compiled(arena: &AstArena, definition: u32, tokens: &[u32], options: &ParserOptions) -> bool {
    let toks = significant_tokens(arena, tokens);
    // a css wide keyword (see `match_syntax()`)
    if toks.len() == 1
        && arena.data_types.get(toks[0]) == EnumToken::IdenTokenType
        && arena.with_string(arena.string_value.get(toks[0]), is_css_wide_keyword)
    {
        return true;
    }
    let length = toks.len() as i32;
    let mut matcher = Matcher::new(arena, toks, options, 0);
    matches!(matcher.m(ROOTS[definition as usize], 0), Ok(result) if result == length)
}
