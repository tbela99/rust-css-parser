//! Port of `src/validation/parser/types.d.ts`: validation tokens (parsed value definition syntax).

use std::rc::Rc;

/// position of a token in the syntax (`[LOC]`)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Position {
    pub ind: i64,
    pub lin: i64,
    pub col: i64,
}

/// value of a validation token (`val`)
#[derive(Clone, Debug, Default, PartialEq)]
pub enum VVal {
    #[default]
    None,
    Str(Rc<str>),
    Num(f64),
    /// declaration definition: the value token
    Token(Rc<ValidationToken>),
}

/// maximum of a `match` (`{min,max}`): a number token, `null` or `Number.POSITIVE_INFINITY`
#[derive(Clone, Debug, PartialEq)]
pub enum MatchMax {
    Null,
    Token(Rc<ValidationToken>),
    Infinity,
}

/// `{min,max}` occurrences
#[derive(Clone, Debug, PartialEq)]
pub struct ValidationMatch {
    pub min: Rc<ValidationToken>,
    pub max: MatchMax,
}

/// `[min,max]` value range
#[derive(Clone, Debug, PartialEq)]
pub struct ValidationRange {
    pub min: Option<Rc<ValidationToken>>,
    pub max: Option<Rc<ValidationToken>>,
}

/// validation token. the optional fields are the fields of the typescript token objects
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ValidationToken {
    pub typ: u8,
    pub loc: Position,
    pub is_list: Option<bool>,
    pub text: Option<Rc<str>>,
    pub is_repeatable: Option<bool>,
    pub is_repeatable_at_least_once: Option<bool>,
    pub is_optional: Option<bool>,
    pub is_mandatatory_group: Option<bool>,
    pub match_: Option<ValidationMatch>,
    pub range: Option<ValidationRange>,
    pub val: VVal,
    pub nam: Option<Rc<str>>,
    /// `EnumToken` name of a dimension (`unit`)
    pub unit: Option<Rc<str>>,
    pub unit_text: Option<Rc<str>>,
    /// children (functions, brackets, optional groups, ...)
    pub chi: Option<Vec<Rc<ValidationToken>>>,
    /// alternatives of a `|` / `||` token
    pub chi_list: Option<Vec<Vec<Rc<ValidationToken>>>>,
    pub l: Option<Vec<Rc<ValidationToken>>>,
    pub r: Option<Vec<Rc<ValidationToken>>>,
    pub prelude: Option<Vec<Rc<ValidationToken>>>,
}

impl ValidationToken {
    pub fn new(typ: u8) -> Self {
        Self { typ, ..Default::default() }
    }

    /// string value
    pub fn val_str(&self) -> &str {
        match &self.val {
            VVal::Str(s) => s,
            _ => "",
        }
    }

    /// numeric value (NaN when not a number)
    pub fn val_num(&self) -> f64 {
        match &self.val {
            VVal::Num(n) => *n,
            _ => f64::NAN,
        }
    }

    pub fn is_optional(&self) -> bool {
        self.is_optional == Some(true)
    }

    pub fn is_list(&self) -> bool {
        self.is_list == Some(true)
    }

    pub fn is_repeatable(&self) -> bool {
        self.is_repeatable == Some(true)
    }

    pub fn is_repeatable_at_least_once(&self) -> bool {
        self.is_repeatable_at_least_once == Some(true)
    }

    pub fn is_mandatatory_group(&self) -> bool {
        self.is_mandatatory_group == Some(true)
    }

    /// children (empty when none)
    pub fn children(&self) -> &[Rc<ValidationToken>] {
        self.chi.as_deref().unwrap_or(&[])
    }

    /// alternatives (empty when none)
    pub fn alternatives(&self) -> &[Vec<Rc<ValidationToken>>] {
        self.chi_list.as_deref().unwrap_or(&[])
    }
}

impl ValidationToken {
    /// plain object of the token: the string keyed properties of the typescript object (`[LOC]` is a symbol)
    pub fn to_plain(&self) -> crate::plain::Plain {
        use crate::plain::{Plain, PlainObject};
        let list = |tokens: &[Rc<ValidationToken>]| Plain::Array(tokens.iter().map(|t| t.to_plain()).collect());
        let mut o = PlainObject::new();
        o.set("typ", self.typ);
        match &self.val {
            VVal::None => {}
            VVal::Str(s) => o.set("val", s.clone()),
            VVal::Num(n) => o.set("val", *n),
            VVal::Token(t) => o.set("val", t.to_plain()),
        }
        if let Some(nam) = &self.nam {
            o.set("nam", nam.clone());
        }
        if let Some(unit) = &self.unit {
            o.set("unit", unit.clone());
        }
        if let Some(unit_text) = &self.unit_text {
            o.set("unitText", unit_text.clone());
        }
        if let Some(text) = &self.text {
            o.set("text", text.clone());
        }
        if let Some(chi) = &self.chi {
            o.set("chi", list(chi));
        }
        if let Some(chi) = &self.chi_list {
            o.set("chi", Plain::Array(chi.iter().map(|c| list(c)).collect()));
        }
        if let Some(l) = &self.l {
            o.set("l", list(l));
        }
        if let Some(r) = &self.r {
            o.set("r", list(r));
        }
        if let Some(prelude) = &self.prelude {
            o.set("prelude", list(prelude));
        }
        for (key, value) in [
            ("isList", self.is_list),
            ("isRepeatable", self.is_repeatable),
            ("isRepeatableAtLeastOnce", self.is_repeatable_at_least_once),
            ("isOptional", self.is_optional),
            ("isMandatatoryGroup", self.is_mandatatory_group),
        ] {
            if let Some(v) = value {
                o.set(key, v);
            }
        }
        if let Some(m) = &self.match_ {
            let mut mo = PlainObject::new();
            mo.set("min", m.min.to_plain());
            mo.set(
                "max",
                match &m.max {
                    MatchMax::Null => Plain::Null,
                    MatchMax::Token(t) => t.to_plain(),
                    MatchMax::Infinity => Plain::Number(f64::INFINITY),
                },
            );
            o.set("match", mo);
        }
        if let Some(r) = &self.range {
            let mut ro = PlainObject::new();
            ro.set("min", r.min.as_ref().map_or(Plain::Null, |t| t.to_plain()));
            ro.set("max", r.max.as_ref().map_or(Plain::Null, |t| t.to_plain()));
            o.set("range", ro);
        }
        Plain::Object(o)
    }
}
