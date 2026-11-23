//! Port of `src/validation/first.ts`: first token sets of the validation syntaxes.
//!
//! The typescript caches are `WeakMap`s keyed by the syntax objects. The rust caches are keyed by the address of the
//! shared syntax tokens / sequences: an entry keeps the syntax alive (the address is not reused) and a sequence entry
//! is checked element by element (`Rc::ptr_eq`).

use super::config::{get_parsed_syntax, get_syntax_config};
use super::parser::typedef::{ValidationSyntaxGroupEnum, ValidationTokenEnum as V};
use super::parser::types::{ValidationToken, VVal};
use crate::ast::arena::AstArena;
use crate::ast::types::EnumToken;
use crate::parser::utils::text::{equals_ignore_case, lowercase};
use crate::syntax::constants::{is_function_def_type, is_function_type, FUNC_LIKE, TOKENS_FUNC_DEF_MAP};
use std::cell::RefCell;
use crate::utils::fx::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::OnceLock;

/// set of token types (bitset)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TypeSet([u64; 4]);

impl TypeSet {
    #[inline]
    pub fn add(&mut self, t: u8) {
        self.0[(t >> 6) as usize] |= 1 << (t & 63);
    }

    #[inline]
    pub fn has(&self, t: u8) -> bool {
        self.0[(t >> 6) as usize] & (1 << (t & 63)) != 0
    }

    fn merge(&mut self, other: &TypeSet) {
        for i in 0..4 {
            self.0[i] |= other.0[i];
        }
    }
}

/// the first tokens of a syntax
#[derive(Clone, Debug, Default)]
pub struct FirstSet {
    pub any: bool,
    pub nullable: bool,
    pub types: TypeSet,
    pub keywords: HashSet<String>,
    pub functions: HashSet<String>,
}

fn any_token() -> Rc<FirstSet> {
    thread_local! {
        static ANY: Rc<FirstSet> = Rc::new(FirstSet { any: true, nullable: true, ..Default::default() });
    }
    ANY.with(|a| a.clone())
}

fn merge(target: &mut FirstSet, source: &FirstSet) {
    target.any |= source.any;
    target.types.merge(&source.types);
    target.keywords.extend(source.keywords.iter().cloned());
    target.functions.extend(source.functions.iter().cloned());
}

/// key of a sequence: the address and the length of its elements
fn seq_key(syntaxes: &[Rc<ValidationToken>]) -> (usize, usize) {
    (syntaxes.as_ptr() as usize, syntaxes.len())
}

fn same_seq(a: &[Rc<ValidationToken>], b: &[Rc<ValidationToken>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| Rc::ptr_eq(x, y))
}

struct Caches {
    nullable: HashMap<usize, (Rc<ValidationToken>, bool)>,
    tokens: HashMap<usize, (Rc<ValidationToken>, Rc<FirstSet>)>,
    sequences: HashMap<(usize, usize), (Vec<Rc<ValidationToken>>, Rc<FirstSet>)>,
    computing: HashSet<(usize, usize)>,
    alternatives: HashMap<(usize, usize), (Vec<Vec<Rc<ValidationToken>>>, Rc<RefCell<AlternativeIndex>>)>,
}

thread_local! {
    static CACHES: RefCell<Caches> = RefCell::new(Caches {
        nullable: HashMap::default(),
        tokens: HashMap::default(),
        sequences: HashMap::default(),
        computing: HashSet::default(),
        alternatives: HashMap::default(),
    });
}

fn match_min_is_zero(syntax: &ValidationToken) -> bool {
    syntax.match_.as_ref().is_some_and(|m| m.min.val_num() == 0.0)
}

/// true if the syntax may match no token
pub fn is_nullable(syntax: &Rc<ValidationToken>) -> bool {
    if syntax.typ == V::Whitespace {
        return true;
    }
    if syntax.is_mandatatory_group() {
        return false;
    }
    if syntax.is_optional() || syntax.is_repeatable() || match_min_is_zero(syntax) {
        return true;
    }
    let key = Rc::as_ptr(syntax) as usize;
    if let Some(v) = CACHES.with(|c| c.borrow().nullable.get(&key).map(|e| e.1)) {
        return v;
    }
    let nullable = match syntax.typ {
        V::Bracket => syntax.children().iter().all(is_nullable),
        V::PipeToken => syntax.alternatives().iter().any(|s| s.iter().all(is_nullable)),
        V::AmpersandToken => {
            syntax.l.as_deref().unwrap_or(&[]).iter().all(is_nullable) && syntax.r.as_deref().unwrap_or(&[]).iter().all(is_nullable)
        }
        _ => false,
    };
    CACHES.with(|c| c.borrow_mut().nullable.insert(key, (syntax.clone(), nullable)));
    nullable
}

/// the types of the function tokens: the types whose name is a key of the sets
fn function_token_types() -> &'static TypeSet {
    static SET: OnceLock<TypeSet> = OnceLock::new();
    SET.get_or_init(|| {
        let mut set = TypeSet::default();
        for v in 0..=255u8 {
            if EnumToken::name(v).is_none() {
                continue;
            }
            let func_type = TOKENS_FUNC_DEF_MAP.iter().any(|(_, t)| *t == v)
                || v == EnumToken::FunctionTokenType
                || v == EnumToken::PseudoClassFuncTokenType;
            if is_function_def_type(v) || is_function_type(v) || FUNC_LIKE.contains(&v) || func_type {
                set.add(v);
            }
        }
        set
    })
}

fn function_def_types() -> Vec<u8> {
    (0..=255u8).filter(|&v| EnumToken::name(v).is_some() && is_function_def_type(v)).collect()
}

/// the types of the tokens accepted by the built-in types of the matcher. None: any token. Err: not a built-in type
fn builtin_type(name: &str) -> Result<Option<Vec<u8>>, ()> {
    use EnumToken as E;
    let color = || {
        let mut v = vec![E::ColorTokenType, E::IdenTokenType, E::FunctionTokenType];
        v.extend(function_def_types());
        v
    };
    Ok(Some(match name {
        "dashed-ident" | "extension-name" | "custom-property-name" => vec![E::DashedIdenTokenType],
        "namespace-prefix" | "ident" | "ident-token" | "custom-ident" | "counter-name" | "counter-style-name" => {
            vec![E::IdenTokenType, E::DashedIdenTokenType]
        }
        "unicode-range-token" => vec![E::UniCodeRange],
        "hash-token" => vec![E::HashTokenType, E::ColorTokenType],
        "string" => vec![E::StringTokenType],
        "wq-name" => vec![E::IdenTokenType],
        "number" | "zero" | "integer" => vec![E::NumberTokenType],
        "dimension" => vec![
            E::DimensionTokenType,
            E::AngleTokenType,
            E::LengthTokenType,
            E::PercentageTokenType,
            E::ResolutionTokenType,
            E::TimeTokenType,
            E::FrequencyTokenType,
            E::UserDefinedDimensionTokenType,
            E::FlexTokenType,
        ],
        "angle" => vec![E::AngleTokenType, E::UserDefinedDimensionTokenType],
        "angle-percentage" => vec![E::AngleTokenType, E::UserDefinedDimensionTokenType, E::PercentageTokenType],
        "percentage" => vec![E::PercentageTokenType],
        "length-percentage" => vec![E::PercentageTokenType, E::LengthTokenType, E::UserDefinedDimensionTokenType, E::NumberTokenType],
        "flex" => vec![E::FlexTokenType, E::UserDefinedDimensionTokenType],
        "length" => vec![E::LengthTokenType, E::UserDefinedDimensionTokenType, E::NumberTokenType],
        "resolution" => vec![E::ResolutionTokenType],
        "frequency" => vec![E::FrequencyTokenType],
        "decibel" | "semitones" => vec![E::DimensionTokenType],
        "time" => vec![E::TimeTokenType, E::UserDefinedDimensionTokenType],
        "quote" | "display-box" | "display-inside" | "display-internal" | "display-outside" | "display-legacy"
        | "content-position" | "mf-name" | "language-code" => vec![E::IdenTokenType],
        "url" => vec![E::UrlFunctionTokenDefType, E::UrlFunctionTokenType],
        "color" | "color-function" => color(),
        "function-token" => function_def_types(),
        "dashed-function" => vec![E::CustomFunctionTokenDefType, E::CustomFunctionTokenType],
        "block-contents" => vec![E::StartParensTokenType],
        "css-type" => vec![E::FunctionTokenDefType, E::FunctionTokenType, E::CssTypeTokenType],
        "declaration-value" | "calc-sum" | "an+b" | "any-value" | "type-selector" | "compound-selector" | "complex-selector"
        | "subclass-selector" | "attribute-selector" | "complex-selector-unit" | "pseudo-compound-selector"
        | "forgiving-relative-selector-list" | "complex-selector-list" | "size-feature" | "style-feature" => return Ok(None),
        _ => return Err(()),
    }))
}

/// first tokens of a sequence of syntaxes
pub fn first_of_sequence(syntaxes: Option<&[Rc<ValidationToken>]>) -> Rc<FirstSet> {
    let Some(syntaxes) = syntaxes else {
        return any_token();
    };
    let key = seq_key(syntaxes);
    let cached = CACHES.with(|c| {
        let c = c.borrow();
        if let Some((keep, set)) = c.sequences.get(&key) {
            if same_seq(keep, syntaxes) {
                return Some(set.clone());
            }
        }
        if c.computing.contains(&key) {
            return Some(any_token());
        }
        None
    });
    if let Some(set) = cached {
        return set;
    }
    CACHES.with(|c| c.borrow_mut().computing.insert(key));
    let mut set = FirstSet { nullable: true, ..Default::default() };
    for syntax in syntaxes {
        let first = first_of(syntax);
        merge(&mut set, &first);
        if set.any || !first.nullable {
            set.nullable = false;
            break;
        }
    }
    let set = if set.any { any_token() } else { Rc::new(set) };
    CACHES.with(|c| {
        let mut c = c.borrow_mut();
        c.computing.remove(&key);
        c.sequences.insert(key, (syntaxes.to_vec(), set.clone()));
    });
    set
}

/// first tokens of a syntax token
pub fn first_of(syntax: &Rc<ValidationToken>) -> Rc<FirstSet> {
    let key = Rc::as_ptr(syntax) as usize;
    if let Some(set) = CACHES.with(|c| c.borrow().tokens.get(&key).map(|e| e.1.clone())) {
        return set;
    }
    let mut set = FirstSet::default();
    let mut nullable = false;
    match syntax.typ {
        V::Whitespace => nullable = true,
        V::Keyword => {
            set.keywords.insert(lowercase(syntax.val_str()).into_owned());
        }
        V::Comma => set.types.add(EnumToken::CommaTokenType),
        V::Colon => set.types.add(EnumToken::ColonTokenType),
        V::Separator => set.types.add(EnumToken::LiteralTokenType),
        V::OpenParenthesis => set.types.add(EnumToken::StartParensTokenType),
        V::CloseParenthesis => set.types.add(EnumToken::EndParensTokenType),
        V::OpenCurlyBrace => set.types.add(EnumToken::BlockStartTokenType),
        V::Number => set.types.add(EnumToken::NumberTokenType),
        V::Dimension => {
            // `EnumToken[unit]`: undefined for an unknown name (the set holds `undefined`, it accepts no token)
            if let Some(t) = syntax.unit.as_deref().and_then(EnumToken::value) {
                set.types.add(t);
            }
        }
        V::StringToken => {
            set.types.add(EnumToken::Add);
            set.types.add(EnumToken::Sub);
            set.types.add(EnumToken::LiteralTokenType);
            set.types.add(EnumToken::Plus);
        }
        V::Function | V::FunctionDefinition => {
            set.functions.insert(lowercase(syntax.val_str()).into_owned());
        }
        V::Bracket => {
            let first = first_of_sequence(Some(syntax.children()));
            merge(&mut set, &first);
            nullable = first.nullable;
        }
        V::OptionalGroupToken => {
            merge(&mut set, &first_of_sequence(Some(syntax.children())));
            nullable = true;
        }
        V::PipeToken => {
            for syntaxes in syntax.alternatives() {
                let first = first_of_sequence(Some(syntaxes));
                merge(&mut set, &first);
                nullable |= first.nullable;
            }
        }
        V::ColumnToken => {
            for syntaxes in syntax.alternatives() {
                merge(&mut set, &first_of_sequence(Some(syntaxes)));
            }
        }
        V::AmpersandToken => {
            nullable = true;
            for syntaxes in [syntax.l.as_deref().unwrap_or(&[]), syntax.r.as_deref().unwrap_or(&[])] {
                let first = first_of_sequence(Some(syntaxes));
                merge(&mut set, &first);
                nullable &= first.nullable;
            }
        }
        V::DeclarationType => {
            let parsed = get_parsed_syntax(ValidationSyntaxGroupEnum::Declarations, syntax.val_str());
            let first = first_of_sequence(parsed.as_deref().map(|v| v.as_slice()));
            merge(&mut set, &first);
            nullable = first.nullable;
        }
        V::PropertyType => {
            let name = syntax.val_str();
            match builtin_type(name) {
                Ok(None) => {
                    set.any = true;
                    nullable = true;
                }
                Ok(Some(types)) => {
                    for t in types {
                        set.types.add(t);
                    }
                }
                Err(()) => {
                    let parsed = get_parsed_syntax(ValidationSyntaxGroupEnum::Syntaxes, name);
                    let first = first_of_sequence(parsed.as_deref().map(|v| v.as_slice()));
                    merge(&mut set, &first);
                    nullable = first.nullable;
                }
            }
        }
        _ => {
            set.any = true;
            nullable = true;
        }
    }
    set.nullable = nullable || syntax.is_optional() || syntax.is_repeatable() || match_min_is_zero(syntax) || is_nullable(syntax);
    let set = Rc::new(set);
    CACHES.with(|c| c.borrow_mut().tokens.insert(key, (syntax.clone(), set.clone())));
    set
}

/// the css wide keywords, accepted by every syntax
fn css_wide_keywords() -> &'static [String] {
    static KEYWORDS: OnceLock<Vec<String>> = OnceLock::new();
    KEYWORDS.get_or_init(|| {
        get_syntax_config()["declarations"]["all"]["syntax"]
            .as_str()
            .unwrap_or("")
            .split(|c: char| c.is_whitespace() || c == '|')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    })
}

/// the token may match a syntax that does not accept its type
fn matches_any_syntax(arena: &AstArena, token: u32, typ: u8) -> bool {
    match typ {
        EnumToken::WildCardFunctionTokenType
        | EnumToken::WildCardFunctionTokenDefType
        | EnumToken::MathFunctionTokenType
        | EnumToken::MathFunctionTokenDefType
        // a custom function call is substituted by its result, like var()
        | EnumToken::CustomFunctionTokenType
        | EnumToken::CustomFunctionTokenDefType => true,
        EnumToken::IdenTokenType => arena.with_string(arena.string_value.get(token), |value| {
            css_wide_keywords().iter().any(|k| equals_ignore_case(k, value))
        }),
        _ => false,
    }
}

/// true if a set may start with a token
fn accepts(set: &FirstSet, typ: u8, name: Option<&str>) -> bool {
    if set.any || set.types.has(typ) {
        return true;
    }
    match name {
        None => false,
        Some(name) => {
            if typ == EnumToken::IdenTokenType {
                set.keywords.contains(name)
            } else {
                set.functions.contains(name)
            }
        }
    }
}

/// lowercase name of an identifier or of a function token
fn token_name(arena: &AstArena, token: u32, typ: u8) -> Option<crate::ast::strings::SharedStr> {
    if typ == EnumToken::IdenTokenType || function_token_types().has(typ) {
        Some(arena.resolve_string(arena.lower_string_id(arena.string_value.get(token))))
    } else {
        None
    }
}

const MAX_NAMES: usize = 512;

/// the alternatives of a syntax indexed by their first tokens
pub struct AlternativeIndex {
    sets: Vec<Rc<FirstSet>>,
    filtered: bool,
    by_type: HashMap<u8, Option<Rc<Vec<usize>>>>,
    by_name: HashMap<u8, HashMap<crate::ast::strings::SharedStr, Option<Rc<Vec<usize>>>>>,
    names: usize,
}

impl AlternativeIndex {
    fn new(alternatives: &[Vec<Rc<ValidationToken>>]) -> Self {
        let sets: Vec<Rc<FirstSet>> = alternatives.iter().map(|s| first_of_sequence(Some(s))).collect();
        let filtered = sets.iter().any(|s| !s.any && !s.nullable);
        Self { sets, filtered, by_type: HashMap::default(), by_name: HashMap::default(), names: 0 }
    }

    fn lookup(&mut self, typ: u8, name: Option<crate::ast::strings::SharedStr>) -> Option<Rc<Vec<usize>>> {
        match name {
            None => {
                if let Some(r) = self.by_type.get(&typ) {
                    return r.clone();
                }
                let r = self.compute(typ, None);
                self.by_type.insert(typ, r.clone());
                r
            }
            Some(name) => {
                if let Some(r) = self.by_name.get(&typ).and_then(|m| m.get(&name)) {
                    return r.clone();
                }
                let r = self.compute(typ, Some(&name));
                if self.names < MAX_NAMES {
                    self.names += 1;
                    self.by_name.entry(typ).or_default().insert(name, r.clone());
                }
                r
            }
        }
    }

    fn compute(&self, typ: u8, name: Option<&str>) -> Option<Rc<Vec<usize>>> {
        let result: Vec<usize> = (0..self.sets.len()).filter(|&i| self.sets[i].nullable || accepts(&self.sets[i], typ, name)).collect();
        if result.len() == self.sets.len() {
            None
        } else {
            Some(Rc::new(result))
        }
    }
}

/// the alternatives of a syntax that may match the first token of a value, in order. None when all of them may match
pub fn matching_alternatives(arena: &AstArena, alternatives: &[Vec<Rc<ValidationToken>>], token: Option<u32>) -> Option<Rc<Vec<usize>>> {
    let key = (alternatives.as_ptr() as usize, alternatives.len());
    let index = CACHES.with(|c| {
        c.borrow().alternatives.get(&key).and_then(|(keep, index)| {
            (keep.len() == alternatives.len() && keep.iter().zip(alternatives).all(|(a, b)| same_seq(a, b))).then(|| index.clone())
        })
    });
    let index = match index {
        Some(index) => index,
        None => {
            let index = Rc::new(RefCell::new(AlternativeIndex::new(alternatives)));
            CACHES.with(|c| c.borrow_mut().alternatives.insert(key, (alternatives.to_vec(), index.clone())));
            index
        }
    };
    let token = token?;
    if !index.borrow().filtered {
        return None;
    }
    let typ = arena.data_types.get(token);
    if matches_any_syntax(arena, token, typ) {
        return None;
    }
    let name = token_name(arena, token, typ);
    let result = index.borrow_mut().lookup(typ, name);
    result
}

/// true if a sequence of syntaxes may match tokens starting with a token
pub fn may_start_with(arena: &AstArena, syntaxes: &[Rc<ValidationToken>], token: Option<u32>) -> bool {
    let Some(token) = token else {
        return true;
    };
    let set = first_of_sequence(Some(syntaxes));
    if set.any {
        return true;
    }
    let typ = arena.data_types.get(token);
    matches_any_syntax(arena, token, typ) || accepts(&set, typ, token_name(arena, token, typ).as_deref())
}

#[allow(dead_code)]
fn _unused(_: VVal) {}
