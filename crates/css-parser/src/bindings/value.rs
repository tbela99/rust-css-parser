//! Values exchanged with javascript and their binary encoding (shared by the N-API and the WASM bindings).
//!
//! Little endian. Each value starts with a tag:
//!
//! | tag | value |
//! |-----|-------|
//! | 0 | undefined |
//! | 1 | null |
//! | 2 / 3 | false / true |
//! | 4 | number: f64 |
//! | 5 | string: u32 byte length, utf-8 bytes |
//! | 6 | bytes (`ArrayBuffer`, `Uint8Array`): u32 length, bytes |
//! | 7 | array: u32 count, values |
//! | 8 | object: u32 count, (string key, value) pairs |
//! | 9 | function: u32 callback id, string name |
//! | 10 | arena: u32 handle |
//! | 11 | node (`AstArenaNode`): u32 arena handle, u32 id |
//! | 12 | promise: u32 id |
//! | 13 | error: string message |
//! | 14 | output buffer: u32 id, u32 length (a rust buffer given to javascript, see `take_output()`) |

use crate::host::{HostDef, HostValue};
use crate::plain::{Plain, PlainObject};
use std::rc::Rc;

thread_local! {
    static UNDEFINED: &'static Val = Box::leak(Box::new(Val::Undefined));
}

/// a reference to `undefined`
pub fn undefined() -> &'static Val {
    UNDEFINED.with(|u| *u)
}

/// a javascript value
#[derive(Clone, Debug, Default)]
pub enum Val {
    #[default]
    Undefined,
    Null,
    Bool(bool),
    Num(f64),
    Str(Rc<str>),
    Bytes(Rc<[u8]>),
    Array(Vec<Val>),
    Object(Vec<(Rc<str>, Val)>),
    Fn(u32, Rc<str>),
    Arena(u32),
    Node(u32, u32),
    Promise(u32),
    Error(Rc<str>),
    /// a buffer owned by the output registry: id, length of the content
    Output(u32, u32),
}

impl Val {
    pub fn str(s: &str) -> Val {
        Val::Str(Rc::from(s))
    }

    pub fn object(entries: Vec<(&str, Val)>) -> Val {
        Val::Object(entries.into_iter().map(|(k, v)| (Rc::from(k), v)).collect())
    }

    pub fn is_nullish(&self) -> bool {
        matches!(self, Val::Undefined | Val::Null)
    }

    pub fn get(&self, key: &str) -> &Val {
        match self {
            Val::Object(entries) => entries.iter().find(|(k, _)| &**k == key).map_or(undefined(), |(_, v)| v),
            _ => undefined(),
        }
    }

    pub fn num(&self) -> f64 {
        match self {
            Val::Num(n) => *n,
            Val::Bool(b) => *b as u8 as f64,
            Val::Null => 0.0,
            _ => f64::NAN,
        }
    }

    pub fn u32(&self) -> u32 {
        let n = self.num();
        if n.is_finite() && n >= 0.0 {
            n as u32
        } else {
            0
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Val::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            Val::Undefined | Val::Null => false,
            Val::Bool(b) => *b,
            Val::Num(n) => *n != 0.0 && !n.is_nan(),
            Val::Str(s) => !s.is_empty(),
            _ => true,
        }
    }

    /// a list of numbers (node ids)
    pub fn ids(&self) -> Vec<u32> {
        match self {
            Val::Array(items) => items.iter().map(|v| match v {
                Val::Node(_, id) => *id,
                v => v.u32(),
            }).collect(),
            Val::Node(_, id) => vec![*id],
            Val::Num(_) => vec![self.u32()],
            _ => Vec::new(),
        }
    }

    /// host definition (options, visitors): the functions are callbacks
    pub fn to_def(&self) -> HostDef {
        match self {
            Val::Undefined | Val::Null | Val::Error(_) | Val::Promise(_) | Val::Output(..) => HostDef::Null,
            Val::Bool(b) => HostDef::Bool(*b),
            Val::Num(n) => HostDef::Number(*n),
            Val::Str(s) => HostDef::Str(s.clone()),
            Val::Bytes(_) => HostDef::Null,
            Val::Array(items) => HostDef::Array(items.iter().map(|v| v.to_def()).collect()),
            Val::Object(entries) => HostDef::Object(entries.iter().filter(|(_, v)| !matches!(v, Val::Undefined)).map(|(k, v)| (k.clone(), v.to_def())).collect()),
            Val::Fn(id, name) => HostDef::Fn { id: *id, name: name.clone() },
            Val::Arena(h) => HostDef::Number(*h as f64),
            Val::Node(_, id) => HostDef::Number(*id as f64),
        }
    }

    /// value returned by a callback
    pub fn to_host(&self) -> HostValue {
        match self {
            Val::Undefined => HostValue::Undefined,
            Val::Null => HostValue::Null,
            Val::Bool(b) => HostValue::Bool(*b),
            Val::Num(n) => HostValue::Number(*n),
            Val::Str(s) => HostValue::Str(s.clone()),
            Val::Bytes(b) => HostValue::Bytes(b.clone()),
            Val::Array(items) => HostValue::Array(items.iter().map(|v| v.to_host()).collect()),
            Val::Object(_) => HostValue::Plain(self.to_plain()),
            Val::Node(arena, id) => HostValue::ForeignNode(*arena, *id),
            Val::Promise(id) => HostValue::Promise(*id),
            Val::Fn(..) | Val::Arena(_) | Val::Error(_) | Val::Output(..) => HostValue::Undefined,
        }
    }

    pub fn to_plain(&self) -> Plain {
        match self {
            Val::Undefined | Val::Fn(..) | Val::Promise(_) | Val::Error(_) | Val::Output(..) => Plain::Undefined,
            Val::Null => Plain::Null,
            Val::Bool(b) => Plain::Bool(*b),
            Val::Num(n) => Plain::Number(*n),
            Val::Str(s) => Plain::String(s.clone()),
            Val::Bytes(b) => Plain::String(Rc::from(String::from_utf8_lossy(b).as_ref())),
            Val::Array(items) => Plain::Array(items.iter().map(|v| v.to_plain()).collect()),
            Val::Object(entries) => {
                let mut o = PlainObject::new();
                for (k, v) in entries {
                    o.set(k.to_string(), v.to_plain());
                }
                Plain::Object(o)
            }
            Val::Arena(h) => Plain::Number(*h as f64),
            Val::Node(_, id) => Plain::Number(*id as f64),
        }
    }

    pub fn from_plain(plain: &Plain) -> Val {
        match plain {
            Plain::Undefined => Val::Undefined,
            Plain::Null => Val::Null,
            Plain::Bool(b) => Val::Bool(*b),
            Plain::Number(n) => Val::Num(*n),
            Plain::String(s) => Val::Str(s.clone()),
            Plain::Array(items) => Val::Array(items.iter().map(Val::from_plain).collect()),
            Plain::Object(o) => Val::Object(o.0.iter().map(|(k, v)| (Rc::from(k.as_ref()), Val::from_plain(v))).collect()),
        }
    }

    pub fn from_host(value: &HostValue, arena: u32) -> Val {
        match value {
            HostValue::Undefined => Val::Undefined,
            HostValue::Null => Val::Null,
            HostValue::Bool(b) => Val::Bool(*b),
            HostValue::Number(n) => Val::Num(*n),
            HostValue::Str(s) => Val::Str(s.clone()),
            HostValue::Bytes(b) => Val::Bytes(b.clone()),
            HostValue::ForeignNode(a, id) => Val::Node(*a, *id),
            HostValue::Plain(p) => Val::from_plain(p),
            HostValue::Array(items) => Val::Array(items.iter().map(|v| Val::from_host(v, arena)).collect()),
            HostValue::Promise(id) => Val::Promise(*id),
        }
    }

    /// encode the value at the end of a buffer
    pub fn encode(&self, out: &mut Vec<u8>) {
        fn string(out: &mut Vec<u8>, s: &str) {
            out.extend_from_slice(&(s.len() as u32).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        match self {
            Val::Undefined => out.push(0),
            Val::Null => out.push(1),
            Val::Bool(b) => out.push(2 + *b as u8),
            Val::Num(n) => {
                out.push(4);
                out.extend_from_slice(&n.to_le_bytes());
            }
            Val::Str(s) => {
                out.push(5);
                string(out, s);
            }
            Val::Bytes(b) => {
                out.push(6);
                out.extend_from_slice(&(b.len() as u32).to_le_bytes());
                out.extend_from_slice(b);
            }
            Val::Array(items) => {
                out.push(7);
                out.extend_from_slice(&(items.len() as u32).to_le_bytes());
                for item in items {
                    item.encode(out);
                }
            }
            Val::Object(entries) => {
                out.push(8);
                out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
                for (k, v) in entries {
                    string(out, k);
                    v.encode(out);
                }
            }
            Val::Fn(id, name) => {
                out.push(9);
                out.extend_from_slice(&id.to_le_bytes());
                string(out, name);
            }
            Val::Arena(h) => {
                out.push(10);
                out.extend_from_slice(&h.to_le_bytes());
            }
            Val::Node(a, id) => {
                out.push(11);
                out.extend_from_slice(&a.to_le_bytes());
                out.extend_from_slice(&id.to_le_bytes());
            }
            Val::Promise(id) => {
                out.push(12);
                out.extend_from_slice(&id.to_le_bytes());
            }
            Val::Error(message) => {
                out.push(13);
                string(out, message);
            }
            Val::Output(id, len) => {
                out.push(14);
                out.extend_from_slice(&id.to_le_bytes());
                out.extend_from_slice(&len.to_le_bytes());
            }
        }
    }

    /// decode a value. `pos` is moved after it
    pub fn decode(bytes: &[u8], pos: &mut usize) -> Val {
        fn u32_at(bytes: &[u8], pos: &mut usize) -> u32 {
            let v = u32::from_le_bytes(bytes[*pos..*pos + 4].try_into().unwrap());
            *pos += 4;
            v
        }
        fn string(bytes: &[u8], pos: &mut usize) -> Rc<str> {
            let len = u32_at(bytes, pos) as usize;
            let s: Rc<str> = Rc::from(std::str::from_utf8(&bytes[*pos..*pos + len]).unwrap_or(""));
            *pos += len;
            s
        }
        let tag = bytes[*pos];
        *pos += 1;
        match tag {
            0 => Val::Undefined,
            1 => Val::Null,
            2 => Val::Bool(false),
            3 => Val::Bool(true),
            4 => {
                let v = f64::from_le_bytes(bytes[*pos..*pos + 8].try_into().unwrap());
                *pos += 8;
                Val::Num(v)
            }
            5 => Val::Str(string(bytes, pos)),
            6 => {
                let len = u32_at(bytes, pos) as usize;
                let b: Rc<[u8]> = Rc::from(&bytes[*pos..*pos + len]);
                *pos += len;
                Val::Bytes(b)
            }
            7 => {
                let n = u32_at(bytes, pos);
                Val::Array((0..n).map(|_| Val::decode(bytes, pos)).collect())
            }
            8 => {
                let n = u32_at(bytes, pos);
                Val::Object((0..n).map(|_| (string(bytes, pos), Val::decode(bytes, pos))).collect())
            }
            9 => {
                let id = u32_at(bytes, pos);
                Val::Fn(id, string(bytes, pos))
            }
            10 => Val::Arena(u32_at(bytes, pos)),
            11 => {
                let a = u32_at(bytes, pos);
                Val::Node(a, u32_at(bytes, pos))
            }
            12 => Val::Promise(u32_at(bytes, pos)),
            13 => Val::Error(string(bytes, pos)),
            14 => {
                let id = u32_at(bytes, pos);
                Val::Output(id, u32_at(bytes, pos))
            }
            _ => Val::Undefined,
        }
    }
}
