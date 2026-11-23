//! Plain (javascript like) values: the plain objects of `toPlainObject()` / `fromObject()`, visitor results, json
//! data. Objects keep the insertion order of their keys, numbers are `f64` (`NaN`, `-0` and the infinities included).

use std::borrow::Cow;
use std::rc::Rc;

pub type Key = Cow<'static, str>;

/// shared `undefined` (no reference count is ever touched: `Undefined` holds no data)
struct SyncPlain(Plain);
unsafe impl Sync for SyncPlain {}
static UNDEFINED: SyncPlain = SyncPlain(Plain::Undefined);

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Plain {
    #[default]
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(Rc<str>),
    Array(Vec<Plain>),
    Object(PlainObject),
}

/// object with ordered keys
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PlainObject(pub Vec<(Key, Plain)>);

impl PlainObject {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn get(&self, key: &str) -> Option<&Plain> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Plain> {
        self.0.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// set a key (replaced in place when it exists, appended otherwise)
    pub fn set(&mut self, key: impl Into<Key>, value: impl Into<Plain>) {
        let key = key.into();
        let value = value.into();
        if let Some(slot) = self.get_mut(&key) {
            *slot = value;
        } else {
            self.0.push((key, value));
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<Plain> {
        let index = self.0.iter().position(|(k, _)| k == key)?;
        Some(self.0.remove(index).1)
    }

    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(k, _)| k.as_ref())
    }
}

impl Plain {
    pub fn object() -> Self {
        Plain::Object(PlainObject::new())
    }

    /// `value == null`
    pub fn is_nullish(&self) -> bool {
        matches!(self, Plain::Undefined | Plain::Null)
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Plain::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Plain::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Plain::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Plain>> {
        match self {
            Plain::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&PlainObject> {
        match self {
            Plain::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut PlainObject> {
        match self {
            Plain::Object(o) => Some(o),
            _ => None,
        }
    }

    /// property of an object (`undefined` otherwise)
    pub fn get(&self, key: &str) -> &Plain {
        self.as_object().and_then(|o| o.get(key)).unwrap_or(&UNDEFINED.0)
    }

    /// javascript truthiness
    pub fn truthy(&self) -> bool {
        match self {
            Plain::Undefined | Plain::Null => false,
            Plain::Bool(b) => *b,
            Plain::Number(n) => *n != 0.0 && !n.is_nan(),
            Plain::String(s) => !s.is_empty(),
            _ => true,
        }
    }

    /// `JSON.stringify()`
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out);
        out
    }

    fn write_json(&self, out: &mut String) {
        match self {
            Plain::Undefined | Plain::Null => out.push_str("null"),
            Plain::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Plain::Number(n) => {
                if n.is_finite() {
                    out.push_str(&crate::utils::number::js_number_to_string(*n))
                } else {
                    out.push_str("null")
                }
            }
            Plain::String(s) => write_json_string(s, out),
            Plain::Array(a) => {
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    if matches!(v, Plain::Undefined) {
                        out.push_str("null");
                    } else {
                        v.write_json(out);
                    }
                }
                out.push(']');
            }
            Plain::Object(o) => {
                out.push('{');
                let mut first = true;
                for (k, v) in &o.0 {
                    if matches!(v, Plain::Undefined) {
                        continue;
                    }
                    if !first {
                        out.push(',');
                    }
                    first = false;
                    write_json_string(k, out);
                    out.push(':');
                    v.write_json(out);
                }
                out.push('}');
            }
        }
    }

    /// convert a json value
    pub fn from_json(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Plain::Null,
            serde_json::Value::Bool(b) => Plain::Bool(*b),
            serde_json::Value::Number(n) => Plain::Number(n.as_f64().unwrap_or(f64::NAN)),
            serde_json::Value::String(s) => Plain::String(Rc::from(s.as_str())),
            serde_json::Value::Array(a) => Plain::Array(a.iter().map(Plain::from_json).collect()),
            serde_json::Value::Object(o) => {
                Plain::Object(PlainObject(o.iter().map(|(k, v)| (Cow::Owned(k.clone()), Plain::from_json(v))).collect()))
            }
        }
    }
}

fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

impl From<f64> for Plain {
    fn from(n: f64) -> Self {
        Plain::Number(n)
    }
}

impl From<u32> for Plain {
    fn from(n: u32) -> Self {
        Plain::Number(n as f64)
    }
}

impl From<i32> for Plain {
    fn from(n: i32) -> Self {
        Plain::Number(n as f64)
    }
}

impl From<u8> for Plain {
    fn from(n: u8) -> Self {
        Plain::Number(n as f64)
    }
}

impl From<bool> for Plain {
    fn from(b: bool) -> Self {
        Plain::Bool(b)
    }
}

impl From<&str> for Plain {
    fn from(s: &str) -> Self {
        Plain::String(Rc::from(s))
    }
}

impl From<String> for Plain {
    fn from(s: String) -> Self {
        Plain::String(Rc::from(s))
    }
}

impl From<crate::ast::strings::SharedStr> for Plain {
    fn from(s: crate::ast::strings::SharedStr) -> Self {
        Plain::String(s.into())
    }
}

impl From<Rc<str>> for Plain {
    fn from(s: Rc<str>) -> Self {
        Plain::String(s)
    }
}

impl From<Vec<Plain>> for Plain {
    fn from(a: Vec<Plain>) -> Self {
        Plain::Array(a)
    }
}

impl From<PlainObject> for Plain {
    fn from(o: PlainObject) -> Self {
        Plain::Object(o)
    }
}

impl<T: Into<Plain>> From<Option<T>> for Plain {
    fn from(o: Option<T>) -> Self {
        o.map_or(Plain::Null, Into::into)
    }
}

impl Plain {
    /// plain value of a host definition (the functions are `undefined`)
    pub fn from_host_def(def: &crate::host::HostDef) -> Plain {
        use crate::host::HostDef;
        match def {
            HostDef::Null => Plain::Null,
            HostDef::Bool(b) => Plain::Bool(*b),
            HostDef::Number(n) => Plain::Number(*n),
            HostDef::Str(s) => Plain::String(s.clone()),
            HostDef::Fn { .. } => Plain::Undefined,
            HostDef::Array(items) => Plain::Array(items.iter().map(Plain::from_host_def).collect()),
            HostDef::Object(entries) => {
                let mut o = PlainObject::new();
                for (k, v) in entries {
                    o.set(k.to_string(), Plain::from_host_def(v));
                }
                Plain::Object(o)
            }
        }
    }
}
