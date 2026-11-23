//! Port of `src/validation/parser/parse.ts`: tokenize, parse and render css value definition syntaxes (webref).
//!
//! The typescript parser mutates token objects that are referenced by both the token list and the stack
//! (`Object.assign(token, ...)`, `tokens.lastIndexOf(stack.at(-1))`). The port builds the tokens in a vector: a token
//! is identified by its index, then the result is frozen into shared [`ValidationToken`]s.

use super::typedef::ValidationTokenEnum as V;
use super::types::{MatchMax, Position, VVal, ValidationMatch, ValidationRange, ValidationToken};
use crate::ast::types::EnumToken;
use crate::syntax::syntax::{is_ident, is_ident_start, is_pseudo, parse_dimension};
use std::rc::Rc;

/// symbol of one character
fn symbol_type(code: u32) -> Option<u8> {
    Some(match code {
        0x7c => V::Pipe,
        0x2a => V::Star,
        0x2c => V::Comma,
        0x2f => V::Separator,
        0x2b => V::Plus,
        0x3b => V::SemiColon,
        0x28 => V::OpenParenthesis,
        0x29 => V::CloseParenthesis,
        0x5b => V::OpenBracket,
        0x5d => V::CloseBracket,
        0x7b => V::OpenCurlyBrace,
        0x7d => V::CloseCurlyBrace,
        0x3f => V::QuestionMark,
        0x21 => V::Exclamation,
        0x23 => V::HashMark,
        0x3c => V::LessThan,
        0x3e => V::GreaterThan,
        0x20 | 0x09 | 0x0d | 0x0a | 0x0c => V::Whitespace,
        0x221e => V::InfinityToken,
        _ => return None,
    })
}

/// symbol made of the character at index and the next one. at the end of the string, a single character symbol
fn pair_symbol_type(syntax: &[u16], index: usize) -> Option<u8> {
    if index + 1 >= syntax.len() {
        return symbol_type(syntax[index] as u32);
    }
    match (syntax[index], syntax[index + 1]) {
        (0x26, 0x26) => Some(V::Ampersand),
        (0x2020, 0x20) => Some(V::DisallowWhitespace),
        (0x7c, 0x7c) => Some(V::Column),
        _ => None,
    }
}

fn is_digits(str: &str) -> bool {
    !str.is_empty() && str.bytes().all(|b| b.is_ascii_digit())
}

/// token built by the parser: the children are indices of the parser tokens
#[derive(Clone, Debug, Default)]
struct Node {
    t: ValidationToken,
    chi: Option<Vec<usize>>,
    chi_list: Option<Vec<Vec<usize>>>,
    l: Option<Vec<usize>>,
    r: Option<Vec<usize>>,
    match_: Option<(usize, NodeMax)>,
    range: Option<(Option<usize>, Option<usize>)>,
}

#[derive(Clone, Copy, Debug)]
enum NodeMax {
    Token(usize),
    Infinity,
}

/// the tokens of the parser
struct Nodes(Vec<Node>);

impl Nodes {
    fn push(&mut self, t: ValidationToken) -> usize {
        self.0.push(Node { t, ..Default::default() });
        self.0.len() - 1
    }

    fn typ(&self, i: usize) -> u8 {
        self.0[i].t.typ
    }

    fn freeze(&self, i: usize) -> Rc<ValidationToken> {
        let node = &self.0[i];
        let mut t = node.t.clone();
        let list = |v: &Vec<usize>| v.iter().map(|&c| self.freeze(c)).collect::<Vec<_>>();
        if let Some(chi) = &node.chi {
            t.chi = Some(list(chi));
        }
        if let Some(chi) = &node.chi_list {
            t.chi_list = Some(chi.iter().map(list).collect());
        }
        if let Some(l) = &node.l {
            t.l = Some(list(l));
        }
        if let Some(r) = &node.r {
            t.r = Some(list(r));
        }
        if let Some((min, max)) = node.match_ {
            t.match_ = Some(ValidationMatch {
                min: self.freeze(min),
                max: match max {
                    NodeMax::Token(m) => MatchMax::Token(self.freeze(m)),
                    NodeMax::Infinity => MatchMax::Infinity,
                },
            });
        }
        if let Some((min, max)) = node.range {
            t.range = Some(ValidationRange { min: min.map(|m| self.freeze(m)), max: max.map(|m| self.freeze(m)) });
        }
        Rc::new(t)
    }
}

/// build a token from a buffered string
fn get_token_type(token: &str, loc: Position) -> ValidationToken {
    let mut t = ValidationToken { loc, ..Default::default() };
    if token == "<an+b>" {
        t.typ = V::PropertyType;
        t.val = VVal::Str(Rc::from("an+b"));
        return t;
    }
    let first = token.chars().next().map_or(0, |c| c as u32);
    if first == 34 || first == 39 {
        t.typ = V::StringToken;
        t.val = VVal::Str(Rc::from(token));
        return t;
    }
    if is_digits(token) || (first == 45 && is_digits(&token[1..])) {
        t.typ = V::Number;
        t.val = VVal::Num(crate::utils::number::js_parse_number(token));
        return t;
    }
    if let Some(dimension) = parse_dimension(token) {
        t.typ = V::Dimension;
        t.unit = EnumToken::name(dimension.typ).map(Rc::from);
        t.val = VVal::Num(dimension.val);
        t.unit_text = Some(Rc::from(dimension.unit.as_str()));
        return t;
    }
    if is_pseudo(token) {
        t.typ = V::PseudoClassToken;
        t.val = VVal::Str(Rc::from(token));
        return t;
    }
    if first == 64 && is_ident(&token[1..]) {
        t.typ = V::AtRule;
        t.val = VVal::Str(Rc::from(&token[1..]));
        return t;
    }
    t.typ = V::Keyword;
    t.val = VVal::Str(Rc::from(token));
    t
}

/// trim whitespace tokens, and the whitespace around commas
pub fn trim_syntax_array(tokens: &mut Vec<Rc<ValidationToken>>) {
    trim_by(tokens, |t| t.typ);
}

fn trim_by<T>(tokens: &mut Vec<T>, typ: impl Fn(&T) -> u8) {
    while tokens.first().is_some_and(|t| typ(t) == V::Whitespace) {
        tokens.remove(0);
    }
    while tokens.last().is_some_and(|t| typ(t) == V::Whitespace) {
        tokens.pop();
    }
    let mut i: isize = 0;
    while (i as usize) < tokens.len() {
        let iu = i as usize;
        if typ(&tokens[iu]) == V::Comma {
            if iu + 1 < tokens.len() && typ(&tokens[iu + 1]) == V::Whitespace {
                tokens.remove(iu + 1);
            }
            if iu > 0 && typ(&tokens[iu - 1]) == V::Whitespace {
                i -= 1;
                tokens.remove(i as usize);
            }
        }
        i += 1;
    }
}

fn trim(nodes: &Nodes, mut tokens: Vec<usize>) -> Vec<usize> {
    trim_by(&mut tokens, |&t| nodes.typ(t));
    tokens
}

/// `array.splice(start, deleteCount)`
fn splice(v: &mut Vec<usize>, start: isize, count: isize) -> Vec<usize> {
    let len = v.len() as isize;
    let start = if start < 0 { (len + start).max(0) } else { start.min(len) } as usize;
    let count = count.clamp(0, len - start as isize) as usize;
    v.drain(start..start + count).collect()
}

/// `array.at(i)` with negative indices
fn at(v: &[usize], i: isize) -> Option<usize> {
    let len = v.len() as isize;
    let i = if i < 0 { len + i } else { i };
    if i < 0 || i >= len {
        None
    } else {
        Some(v[i as usize])
    }
}

/// `array.lastIndexOf(value)`, -1 when the value is undefined
fn last_index_of(v: &[usize], value: Option<usize>) -> isize {
    value.and_then(|value| v.iter().rposition(|&x| x == value)).map_or(-1, |i| i as isize)
}

fn get(v: &[usize], i: isize) -> Option<usize> {
    if i < 0 {
        None
    } else {
        v.get(i as usize).copied()
    }
}

/// tokenize a syntax definition
pub fn tokenize_syntax(syntax: &str) -> Vec<ValidationToken> {
    let units: Vec<u16> = syntax.encode_utf16().collect();
    let length = units.len();
    let mut tokens: Vec<ValidationToken> = Vec::new();
    let (mut pos_ind, pos_lin, mut pos_col): (i64, i64, i64) = (0, 1, 0);
    let (mut cur_ind, cur_lin, mut cur_col): (i64, i64, i64) = (-1, 1, 0);
    let mut prefix = String::new();
    let mut buffer_start = 0usize;
    let mut buffer_end = 0usize;
    let mut i: isize = -1;
    let code_at = |k: usize| units.get(k).map_or(u32::MAX, |&c| c as u32);
    let slice = |a: usize, b: usize| String::from_utf16_lossy(&units[a..b]);

    macro_rules! buffer_length {
        () => {
            prefix.encode_utf16().count() + buffer_end - buffer_start
        };
    }
    macro_rules! buffer {
        () => {
            if prefix.is_empty() {
                slice(buffer_start, buffer_end)
            } else {
                format!("{}{}", prefix, slice(buffer_start, buffer_end))
            }
        };
    }
    macro_rules! append {
        ($index:expr) => {{
            let index = $index;
            if buffer_end == buffer_start {
                buffer_start = index;
            }
            buffer_end = index + 1;
        }};
    }
    macro_rules! clear {
        () => {{
            prefix.clear();
            buffer_start = 0;
            buffer_end = 0;
        }};
    }
    macro_rules! start_token {
        () => {{
            let loc = Position { ind: pos_ind, lin: pos_lin, col: pos_col };
            pos_ind = cur_ind;
            pos_col = cur_col;
            loc
        }};
    }

    loop {
        i += 1;
        if i as usize >= length {
            break;
        }
        let iu = i as usize;
        let code = units[iu] as u32;
        cur_col += 1;
        cur_ind += 1;
        if code == 60 && units[iu + 1..].starts_with(&"an+b>".encode_utf16().collect::<Vec<_>>()) {
            if buffer_length!() > 0 {
                prefix = buffer!();
                buffer_start = 0;
                buffer_end = 0;
            }
            tokens.push(get_token_type("<an+b>", start_token!()));
            cur_col += 5;
            cur_ind += 5;
            i += 5;
            continue;
        }
        if let Some(typ) = pair_symbol_type(&units, iu) {
            if iu + 1 < length {
                i += 1;
                cur_col += 1;
                cur_ind += 1;
            }
            let loc = start_token!();
            if buffer_length!() > 0 {
                let b = buffer!();
                tokens.push(get_token_type(&b, start_token!()));
                clear!();
            }
            tokens.push(ValidationToken { typ, loc, ..Default::default() });
            continue;
        }
        if code == 58 {
            if code_at(iu + 1) == 58 && (is_ident_start(code_at(iu + 2)) || (code_at(iu + 2) == 45 && is_ident_start(code_at(iu + 3)))) {
                append!(iu);
                i += 1;
                append!(i as usize);
                continue;
            }
            if is_ident_start(code_at(iu + 1)) || (code_at(iu + 1) == 45 && is_ident_start(code_at(iu + 2))) {
                append!(iu);
                continue;
            }
        }
        if let Some(typ) = symbol_type(code) {
            if typ == V::Whitespace {
                let mut k = iu;
                while k + 1 < length && symbol_type(units[k + 1] as u32) == Some(V::Whitespace) {
                    k += 1;
                }
                if k != iu {
                    cur_col += (k - iu) as i64;
                    cur_ind += (k - iu) as i64;
                    i = k as isize;
                }
            }
            let loc = start_token!();
            if buffer_length!() > 0 {
                let b = buffer!();
                tokens.push(get_token_type(&b, start_token!()));
                clear!();
            }
            tokens.push(ValidationToken { typ, loc, ..Default::default() });
            continue;
        }
        match code {
            58 => {
                let loc = start_token!();
                if buffer_length!() > 0 {
                    let b = buffer!();
                    tokens.push(get_token_type(&b, start_token!()));
                    clear!();
                }
                tokens.push(ValidationToken { typ: V::Colon, loc, ..Default::default() });
            }
            34 | 39 => {
                let quote = if buffer_length!() > 0 { buffer!().encode_utf16().next().map_or(code, |c| c as u32) } else { code };
                append!(i as usize);
                while (i as usize) + 1 < length {
                    i += 1;
                    append!(i as usize);
                    if units[i as usize] as u32 == quote {
                        break;
                    }
                }
                let str = buffer!();
                tokens.push(get_token_type(&str, start_token!()));
                cur_col += str.chars().count() as i64;
                cur_ind += str.encode_utf16().count() as i64;
                clear!();
            }
            _ => append!(i as usize),
        }
    }
    if buffer_length!() > 0 {
        let b = buffer!();
        tokens.push(get_token_type(&b, start_token!()));
    }
    tokens.push(ValidationToken { typ: V::EOF, loc: Position { ind: pos_ind, lin: pos_lin, col: pos_col }, ..Default::default() });
    let _ = cur_lin;
    tokens
}

/// parse a css value definition syntax (webref)
pub fn parse_syntax(syntax: &str) -> Vec<Rc<ValidationToken>> {
    let mut nodes = Nodes(Vec::new());
    let mut stack: Vec<usize> = Vec::new();
    let mut tokens: Vec<usize> = Vec::new();
    let top = |stack: &Vec<usize>| stack.last().copied();

    for token in tokenize_syntax(syntax) {
        let token = nodes.push(token);
        tokens.push(token);
        let len = |tokens: &Vec<usize>| tokens.len() as isize;
        match nodes.typ(token) {
            V::EOF => {
                if let Some(s) = top(&stack) {
                    let typ = nodes.typ(s);
                    if typ == V::PipeToken || typ == V::AmpersandToken || typ == V::ColumnToken {
                        let index = last_index_of(&tokens, Some(s));
                        let n = len(&tokens) - index - 2;
                        let part = splice(&mut tokens, index + 1, n);
                        let part = trim(&nodes, part);
                        if typ == V::AmpersandToken {
                            nodes.0[s].r = Some(part);
                        } else {
                            nodes.0[s].chi_list.get_or_insert_with(Vec::new).push(part);
                        }
                        stack.pop();
                    }
                }
                tokens.pop();
            }
            V::Pipe => {
                while let Some(s) = top(&stack).filter(|&s| matches!(nodes.typ(s), V::AmpersandToken | V::ColumnToken)) {
                    let index = last_index_of(&tokens, Some(s));
                    let n = len(&tokens) - index - 2;
                    let part = trim(&nodes, splice(&mut tokens, index + 1, n));
                    if nodes.typ(s) == V::AmpersandToken {
                        nodes.0[s].r = Some(part);
                    } else {
                        nodes.0[s].chi_list.get_or_insert_with(Vec::new).push(part);
                    }
                    stack.pop();
                }
                let index = if stack.is_empty() { -1 } else { last_index_of(&tokens, top(&stack)) };
                if top(&stack).map(|s| nodes.typ(s)) != Some(V::PipeToken) {
                    let n = len(&tokens) - index - 2;
                    let part = trim(&nodes, splice(&mut tokens, index + 1, n));
                    nodes.0[token].t.typ = V::PipeToken;
                    nodes.0[token].chi_list = Some(vec![part]);
                    stack.push(token);
                    continue;
                }
                tokens.pop();
                let n = len(&tokens) - index - 1;
                let part = trim(&nodes, splice(&mut tokens, index + 1, n));
                let s = top(&stack).unwrap();
                nodes.0[s].chi_list.get_or_insert_with(Vec::new).push(part);
            }
            V::Ampersand => {
                let index = if stack.is_empty() { -1 } else { last_index_of(&tokens, top(&stack)) };
                if top(&stack).map(|s| nodes.typ(s)) == Some(V::AmpersandToken) {
                    let s = top(&stack).unwrap();
                    let n = len(&tokens) - index - 2;
                    let part = trim(&nodes, splice(&mut tokens, index + 1, n));
                    nodes.0[s].r = Some(part);
                    stack.pop();
                    let l = splice(&mut tokens, index, 1);
                    nodes.0[token].t.typ = V::AmpersandToken;
                    nodes.0[token].l = Some(l);
                    nodes.0[token].r = Some(Vec::new());
                    stack.push(token);
                } else {
                    let n = len(&tokens) - index - 2;
                    let part = trim(&nodes, splice(&mut tokens, index + 1, n));
                    nodes.0[token].t.typ = V::AmpersandToken;
                    nodes.0[token].l = Some(part);
                    nodes.0[token].r = Some(Vec::new());
                    stack.push(token);
                }
            }
            V::Column => {
                let index = if stack.is_empty() { -1 } else { last_index_of(&tokens, top(&stack)) };
                if top(&stack).map(|s| nodes.typ(s)) == Some(V::ColumnToken) {
                    let s = top(&stack).unwrap();
                    let n = len(&tokens) - index - 2;
                    let part = trim(&nodes, splice(&mut tokens, index + 1, n));
                    nodes.0[s].chi_list.get_or_insert_with(Vec::new).push(part);
                    tokens.pop();
                } else {
                    let part = if index != -1 {
                        let n = len(&tokens) - index - 2;
                        trim(&nodes, splice(&mut tokens, index + 1, n))
                    } else {
                        let n = len(&tokens) - 2;
                        trim(&nodes, splice(&mut tokens, 0, n))
                    };
                    nodes.0[token].t.typ = V::ColumnToken;
                    nodes.0[token].chi_list = Some(vec![part]);
                    stack.push(token);
                }
            }
            V::OpenParenthesis => {
                if let Some(prev) = at(&tokens, -2) {
                    let typ = nodes.typ(prev);
                    if typ == V::Keyword || typ == V::PseudoClassToken {
                        nodes.0[prev].t.typ = if typ == V::Keyword { V::Function } else { V::PseudoClassFunctionToken };
                        nodes.0[prev].chi = Some(Vec::new());
                        stack.push(prev);
                        tokens.pop();
                        continue;
                    }
                }
                stack.push(token);
            }
            V::CloseParenthesis => {
                if let Some(s) = top(&stack).filter(|&s| nodes.typ(s) == V::PipeToken) {
                    let index = last_index_of(&tokens, Some(s));
                    let n = len(&tokens) - index - 2;
                    let part = trim(&nodes, splice(&mut tokens, index + 1, n));
                    nodes.0[s].chi_list.get_or_insert_with(Vec::new).push(part);
                    stack.pop();
                }
                let s = top(&stack);
                if s.is_some_and(|s| {
                    nodes.typ(s) == V::OpenParenthesis || (nodes.typ(s) == V::PropertyType && nodes.0[s].t.val_str() == "function-token")
                }) {
                    stack.pop();
                } else if let Some(s) = s.filter(|&s| nodes.typ(s) == V::AmpersandToken) {
                    let index = last_index_of(&tokens, Some(s));
                    let n = len(&tokens) - index - 2;
                    let part = trim(&nodes, splice(&mut tokens, index + 1, n));
                    nodes.0[s].r = Some(part);
                    stack.pop();
                }
                if let Some(s) = top(&stack).filter(|&s| matches!(nodes.typ(s), V::Function | V::PseudoClassFunctionToken)) {
                    let index = last_index_of(&tokens, Some(s));
                    tokens.pop();
                    let n = len(&tokens);
                    let part = trim(&nodes, splice(&mut tokens, index + 1, n));
                    nodes.0[s].chi = Some(part);
                    stack.pop();
                }
            }
            V::GreaterThan => {
                let n = len(&tokens);
                if let (Some(lt), Some(prev)) = (get(&tokens, n - 3), get(&tokens, n - 2)) {
                    if nodes.typ(lt) == V::LessThan {
                        match nodes.typ(prev) {
                            V::Keyword => {
                                let val = nodes.0[prev].t.val.clone();
                                nodes.0[lt].t.typ = V::PropertyType;
                                nodes.0[lt].t.val = val;
                                if nodes.0[prev].range.is_some() {
                                    nodes.0[lt].range = nodes.0[prev].range;
                                }
                                tokens.truncate(tokens.len() - 2);
                                let last = *tokens.last().unwrap();
                                if nodes.0[last].t.val_str() == "function-token" {
                                    stack.push(last);
                                }
                            }
                            V::StringToken => {
                                let val = nodes.0[prev].t.val_str().to_string();
                                let inner: String = {
                                    let chars: Vec<char> = val.chars().collect();
                                    if chars.len() >= 2 { chars[1..chars.len() - 1].iter().collect() } else { String::new() }
                                };
                                nodes.0[lt].t.typ = V::DeclarationType;
                                nodes.0[lt].t.val = VVal::Str(Rc::from(inner));
                                tokens.truncate(tokens.len() - 2);
                            }
                            V::Function => {
                                let val = nodes.0[prev].t.val.clone();
                                nodes.0[lt].t.typ = V::FunctionDefinition;
                                nodes.0[lt].t.val = val;
                                tokens.truncate(tokens.len() - 2);
                            }
                            _ => {}
                        }
                    }
                }
            }
            V::OpenBracket | V::OpenCurlyBrace => stack.push(token),
            V::CloseBracket => {
                loop {
                    let Some(s) = top(&stack) else { break };
                    let typ = nodes.typ(s);
                    if typ == V::AmpersandToken || typ == V::ColumnToken || typ == V::PipeToken {
                        let index = last_index_of(&tokens, Some(s));
                        let n = len(&tokens) - index - 2;
                        let part = trim(&nodes, splice(&mut tokens, index + 1, n));
                        if typ == V::AmpersandToken {
                            nodes.0[s].r = Some(part);
                        } else {
                            nodes.0[s].chi_list.get_or_insert_with(Vec::new).push(part);
                        }
                        stack.pop();
                        continue;
                    }
                    break;
                }
                let current = top(&stack);
                let index = last_index_of(&tokens, current);
                let t = |k: isize| get(&tokens, index + k).map(|x| nodes.typ(x));
                let is_negative_infinity = t(1) == Some(V::Keyword)
                    && get(&tokens, index + 1).is_some_and(|x| nodes.0[x].t.val_str() == "-")
                    && t(2) == Some(V::InfinityToken);
                let name_before = matches!(t(-2), Some(V::Keyword) | Some(V::StringToken)) && t(-1) == Some(V::Whitespace);
                if is_negative_infinity
                    && name_before
                    && t(3) == Some(V::Comma)
                    && matches!(t(4), Some(V::Number) | Some(V::Dimension) | Some(V::InfinityToken))
                    && t(5) == Some(V::CloseBracket)
                {
                    stack.pop();
                    let name = get(&tokens, index - 2).unwrap();
                    let max = if t(4) == Some(V::InfinityToken) { None } else { get(&tokens, index + 4) };
                    nodes.0[name].range = Some((None, max));
                    splice(&mut tokens, index - 1, 8);
                    continue;
                }
                if name_before
                    && matches!(t(1), Some(V::Number) | Some(V::Dimension))
                    && t(2) == Some(V::Comma)
                    && (t(3) == t(1) || t(3) == Some(V::InfinityToken))
                    && t(4) == Some(V::CloseBracket)
                {
                    stack.pop();
                    let name = get(&tokens, index - 2).unwrap();
                    let min = get(&tokens, index + 1);
                    let max = if t(3) == t(1) { get(&tokens, index + 3) } else { None };
                    nodes.0[name].range = Some((min, max));
                    splice(&mut tokens, index - 1, 7);
                    continue;
                }
                stack.pop();
                let start_index = last_index_of(&tokens, current);
                let end_index = tokens.iter().position(|&x| x == token).map_or(-1, |i| i as isize);
                let chi: Vec<usize> = if start_index + 1 <= end_index && end_index >= 0 {
                    tokens[(start_index + 1) as usize..end_index as usize].to_vec()
                } else {
                    Vec::new()
                };
                let chi = trim(&nodes, chi);
                if let Some(c) = current {
                    nodes.0[c].t.typ = V::Bracket;
                    nodes.0[c].chi = Some(chi);
                }
                let n = len(&tokens);
                splice(&mut tokens, start_index + 1, n);
            }
            V::CloseCurlyBrace => {
                loop {
                    let Some(s) = top(&stack) else { break };
                    let typ = nodes.typ(s);
                    if typ == V::ColumnToken || typ == V::PipeToken {
                        let index = last_index_of(&tokens, Some(s));
                        let n = len(&tokens) - index - 2;
                        let part = trim(&nodes, splice(&mut tokens, index + 1, n));
                        nodes.0[s].chi_list.get_or_insert_with(Vec::new).push(part);
                        stack.pop();
                        continue;
                    }
                    break;
                }
                let index = last_index_of(&tokens, top(&stack));
                let end = tokens.len().saturating_sub(1);
                let start = (index + 1).max(0) as usize;
                let slice: Vec<usize> = if start < end { tokens[start..end].to_vec() } else { Vec::new() };
                if (slice.len() == 1 || slice.len() == 3) && nodes.typ(slice[0]) == V::Number {
                    if let Some(target) = get(&tokens, index - 1) {
                        let max = if slice.len() == 3 {
                            if nodes.typ(slice[2]) == V::InfinityToken {
                                NodeMax::Infinity
                            } else {
                                NodeMax::Token(slice[2])
                            }
                        } else {
                            NodeMax::Token(slice[0])
                        };
                        nodes.0[target].match_ = Some((slice[0], max));
                    }
                    stack.pop();
                    tokens.truncate(index.max(0) as usize);
                    continue;
                }
                stack.pop();
            }
            V::Star | V::HashMark | V::Exclamation | V::Plus => {
                let typ = nodes.typ(token);
                tokens.pop();
                if let Some(&last) = tokens.last() {
                    let t = &mut nodes.0[last].t;
                    match typ {
                        V::Star => t.is_repeatable = Some(true),
                        V::HashMark => t.is_list = Some(true),
                        V::Exclamation => t.is_mandatatory_group = Some(true),
                        _ => t.is_repeatable_at_least_once = Some(true),
                    }
                }
            }
            V::QuestionMark => {
                tokens.pop();
                let is_group = at(&tokens, -3).is_some_and(|t| nodes.typ(t) == V::Comma);
                if is_group {
                    let slice_index = tokens.len() - 3;
                    let chi = trim(&nodes, tokens[slice_index..].to_vec());
                    tokens.truncate(slice_index);
                    let group = nodes.push(ValidationToken { typ: V::OptionalGroupToken, is_optional: Some(true), ..Default::default() });
                    nodes.0[group].chi = Some(chi);
                    tokens.push(group);
                } else if let Some(&last) = tokens.last() {
                    nodes.0[last].t.is_optional = Some(true);
                }
            }
            V::Comma => {
                let before = at(&tokens, -2).map(|t| nodes.typ(t));
                let prev = at(&tokens, -3);
                if before == Some(V::Whitespace)
                    && prev.is_some_and(|p| nodes.0[p].t.is_optional() && nodes.typ(p) != V::OptionalGroupToken)
                {
                    let slice_index = tokens.len() - 3;
                    if nodes.0[tokens[slice_index]].t.is_list() {
                        tokens.pop();
                    }
                    let members: Vec<usize> = tokens[slice_index..].to_vec();
                    for &m in &members {
                        nodes.0[m].t.is_optional = None;
                    }
                    let chi = trim(&nodes, members);
                    tokens.truncate(slice_index);
                    let group = nodes.push(ValidationToken { typ: V::OptionalGroupToken, is_optional: Some(true), ..Default::default() });
                    nodes.0[group].chi = Some(chi);
                    tokens.push(group);
                }
            }
            _ => {}
        }
    }
    if let Some(s) = top(&stack).filter(|&s| nodes.typ(s) == V::PipeToken) {
        let index = last_index_of(&tokens, Some(s));
        let n = tokens.len() as isize - index - 1;
        let part = trim(&nodes, splice(&mut tokens, index + 1, n));
        nodes.0[s].chi_list.get_or_insert_with(Vec::new).push(part);
    }
    let tokens = trim(&nodes, tokens);
    tokens.into_iter().map(|t| nodes.freeze(t)).collect()
}

/// render options of [`render_syntax`]
#[derive(Clone, Copy, Debug)]
pub struct RenderSyntaxOptions {
    pub minify: bool,
    pub indent: i64,
}

impl Default for RenderSyntaxOptions {
    fn default() -> Self {
        Self { minify: true, indent: 1 }
    }
}

fn render_list(tokens: &[Rc<ValidationToken>], options: &mut RenderSyntaxOptions) -> String {
    tokens.iter().map(|t| render_syntax(t, options)).collect()
}

/// render a validation token
pub fn render_syntax(token: &ValidationToken, options: &mut RenderSyntaxOptions) -> String {
    let glue = if options.minify { String::new() } else { " ".repeat(options.indent.max(0) as usize) };
    match token.typ {
        V::InfinityToken => "\u{221e}".into(),
        V::Root => render_list(token.children(), options),
        V::Whitespace => " ".into(),
        V::FunctionDefinition => format!("<{}()>", token.val_str()),
        V::HashMark => "#".into(),
        V::Colon => ":".into(),
        V::Pipe => format!("{glue}|{glue}"),
        V::Column => format!("{glue}||{glue}"),
        V::PipeToken => {
            let mut acc = String::new();
            for curr in token.alternatives() {
                if !acc.trim().is_empty() {
                    acc.push_str(&format!("{glue}|{glue}"));
                }
                acc.push_str(&render_list(curr, options));
            }
            acc
        }
        V::ColumnToken => {
            let sep = format!("{glue}||{glue}");
            let mut acc = String::new();
            for curr in token.alternatives() {
                if !acc.is_empty() {
                    acc.push_str(&sep);
                }
                acc.push_str(&render_list(curr, options));
            }
            acc
        }
        V::AmpersandToken => {
            let sep = format!("{glue}&&{glue}");
            format!(
                "{}{}{}",
                render_list(token.l.as_deref().unwrap_or(&[]), options),
                sep,
                render_list(token.r.as_deref().unwrap_or(&[]), options)
            )
        }
        V::Function | V::PseudoClassFunctionToken | V::Parens => {
            format!("{}({}){}", token.val_str(), render_list(token.children(), options), render_attributes(token))
        }
        V::Comma => format!(",{glue}"),
        V::Keyword => format!("{}{}", token.val_str(), render_attributes(token)),
        V::OpenBracket => format!("[{glue}"),
        V::Ampersand => format!("{glue}&&{glue}"),
        V::Plus => "+".into(),
        V::QuestionMark => "?".into(),
        V::Separator => "/".into(),
        V::Bracket => format!("[{glue}{}{glue}]{}", render_list(token.children(), options), render_attributes(token)),
        V::PropertyType | V::DeclarationType => {
            let q = if token.typ == V::DeclarationType { "'" } else { "" };
            let range = match &token.range {
                None => String::new(),
                Some(range) => format!(
                    " [{},{}]",
                    range.min.as_ref().map_or("-\u{221e}".into(), |m| render_syntax(m, &mut RenderSyntaxOptions::default())),
                    range.max.as_ref().map_or("\u{221e}".into(), |m| render_syntax(m, &mut RenderSyntaxOptions::default()))
                ),
            };
            format!("<{q}{}{q}{range}>{}", token.val_str(), render_attributes(token))
        }
        V::Number => crate::utils::number::js_number_to_string(token.val_num()),
        V::PseudoClassToken | V::StringToken => token.val_str().to_string(),
        V::SemiColon => ";".into(),
        V::AtRule => format!("@{}", token.val_str()),
        V::AtRuleDefinition => {
            let prelude = token.prelude.as_ref().map_or(String::new(), |p| format!(" {}", render_list(p, options)));
            let chi = token.chi.as_ref().map_or(String::new(), |c| format!(" {{\n{}", render_list(c, options)));
            // `.slice(1, -1)` of the children part
            let chi: String = {
                let chars: Vec<char> = chi.chars().collect();
                if chars.len() >= 2 { chars[1..chars.len() - 1].iter().collect() } else { String::new() }
            };
            format!("@{}{}{}\n}}", token.val_str(), prelude, chi)
        }
        V::Block => {
            let nl = if options.minify { "" } else { "\n" };
            format!("{{{nl}{}{nl}}}", render_list(token.children(), options))
        }
        V::DeclarationDefinitionToken => {
            let val = match &token.val {
                VVal::Token(t) => render_syntax(t, options),
                _ => String::new(),
            };
            format!("{}: {}", token.nam.as_deref().unwrap_or(""), val)
        }
        V::LessThan => "<".into(),
        V::GreaterThan => ">".into(),
        V::CloseParenthesis => ")".into(),
        V::OpenParenthesis => "(".into(),
        V::DisallowWhitespace => "\u{2020} ".into(),
        V::Dimension => format!("{}{}", crate::utils::number::js_number_to_string(token.val_num()), token.unit_text.as_deref().unwrap_or("")),
        V::OpenCurlyBrace => {
            options.indent += 1;
            format!("{{{}}}", if options.minify { "" } else { "\n" })
        }
        V::CloseCurlyBrace => {
            options.indent -= 1;
            format!("{}}}", if options.minify { "" } else { "\n" })
        }
        V::DeclarationNameToken => format!("{}:", token.val_str()),
        V::OptionalGroupToken => {
            let chi = token.children();
            let mut acc = String::new();
            for (index, curr) in chi.iter().enumerate() {
                if index == chi.len() - 1 {
                    if curr.typ == V::Comma {
                        acc.push_str("?,");
                    } else {
                        acc.push_str(&render_syntax(curr, options));
                        acc.push('?');
                    }
                } else {
                    acc.push_str(&render_syntax(curr, options));
                }
            }
            acc
        }
        // typescript throws "Unhandled token"
        _ => String::new(),
    }
}

/// `#?!*+{min,max}` suffixes
fn render_attributes(token: &ValidationToken) -> String {
    let mut result = String::new();
    if token.is_list() {
        result.push('#');
    }
    if token.is_optional() {
        result.push('?');
    }
    if token.is_mandatatory_group() {
        result.push('!');
    }
    if token.is_repeatable() {
        result.push('*');
    }
    if token.is_repeatable_at_least_once() {
        result.push('+');
    }
    if let Some(m) = &token.match_ {
        let min = render_syntax(&m.min, &mut RenderSyntaxOptions::default());
        match &m.max {
            MatchMax::Null => result.push_str(&format!("{{{}}}", min)),
            MatchMax::Token(max) if max.val_num() == m.min.val_num() || max.val_num().is_nan() => {
                result.push_str(&format!("{{{}}}", min))
            }
            MatchMax::Token(max) if max.val_num().is_finite() => {
                result.push_str(&format!("{{{},{}}}", min, render_syntax(max, &mut RenderSyntaxOptions::default())))
            }
            _ => result.push_str(&format!("{{{},\u{221e}}}", min)),
        }
    }
    result
}
