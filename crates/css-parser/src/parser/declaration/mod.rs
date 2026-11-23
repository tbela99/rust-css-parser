//! Port of `src/parser/declaration/*`: shorthand computation and duplicate declaration removal.
//!
//! The typescript `Map`s are insertion ordered: [`JsMap`] (a key set again keeps its position, a deleted key set again
//! moves to the end). The shorthand configuration (`properties.json`) is read as json values, like the typescript
//! property accesses.

pub mod list;
pub mod map;
pub mod set;

use crate::ast::arena::{AstArena, TokenValue};
use crate::ast::types::EnumToken;
use serde_json::Value;

/// insertion ordered map with the semantics of the javascript `Map`
#[derive(Clone, Debug)]
pub struct JsMap<V> {
    /// the keys are shared strings: an interned name of the arena is stored without a copy
    entries: Vec<(SharedStr, V)>,
}

impl<V> Default for JsMap<V> {
    fn default() -> Self {
        Self { entries: Vec::new() }
    }
}

impl<V> JsMap<V> {
    pub fn new() -> Self {
        Self::default()
    }

    fn index(&self, key: &str) -> Option<usize> {
        self.entries.iter().position(|(k, _)| &**k == key)
    }

    pub fn has(&self, key: &str) -> bool {
        self.index(key).is_some()
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.index(key).map(|i| &self.entries[i].1)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.index(key).map(move |i| &mut self.entries[i].1)
    }

    /// set a value: an existing key keeps its position
    pub fn set(&mut self, key: impl AsRef<str> + Into<SharedStr>, value: V) {
        match self.index(key.as_ref()) {
            Some(i) => self.entries[i].1 = value,
            None => self.entries.push((key.into(), value)),
        }
    }

    pub fn delete(&mut self, key: &str) -> bool {
        match self.index(key) {
            Some(i) => {
                self.entries.remove(i);
                true
            }
            None => false,
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| &**k)
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.entries.iter().map(|(_, v)| v)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &V)> {
        self.entries.iter().map(|(k, v)| (&**k, v))
    }
}

/// `key in object`
pub(crate) fn has_key(v: &Value, key: &str) -> bool {
    v.as_object().is_some_and(|o| o.contains_key(key))
}

/// a string array of the configuration (empty when missing)
pub(crate) fn str_list(v: &Value) -> Vec<&str> {
    v.as_array().map(|a| a.iter().filter_map(|s| s.as_str()).collect()).unwrap_or_default()
}

/// `EnumToken[config.typ]`
pub(crate) fn config_type(config: &Value) -> u8 {
    config.get("typ").and_then(|t| t.as_str()).and_then(EnumToken::value).unwrap_or(0)
}

/// create a token from the shorthand configuration (a whitespace without configuration)
pub(crate) fn config_token(arena: &AstArena, config: Option<&Value>) -> u32 {
    match config.filter(|c| !c.is_null()) {
        None => arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None),
        Some(config) => {
            let val = config.get("val").and_then(|v| v.as_str());
            arena.create_token(config_type(config), val.map_or(TokenValue::None, TokenValue::Str), None)
        }
    }
}

/// true if the token matches the configuration token
pub(crate) fn match_config_token(arena: &AstArena, token: u32, config: Option<&Value>) -> bool {
    let Some(config) = config.filter(|c| !c.is_null()) else {
        return false;
    };
    arena.data_types.get(token) == config_type(config)
        && config.get("val").and_then(|v| v.as_str()).is_none_or(|val| arena.string_equals(arena.string_value.get(token), val))
}

use std::cell::RefCell;
use std::rc::Rc;
use crate::ast::strings::SharedStr;

/// a value of the declaration maps: a declaration node, or a set / map of declarations shared by reference
#[derive(Clone)]
pub enum DeclEntry {
    Node(u32),
    Set(Rc<RefCell<set::PropertySet>>),
    Map(Rc<RefCell<map::PropertyMap>>),
}

/// the declarations of entries: the sets and the maps are iterated (`[Symbol.iterator]()`, which may compute a
/// shorthand: each iteration runs it again, like the typescript iterators)
pub fn flatten<'a>(arena: &AstArena, entries: impl IntoIterator<Item = &'a DeclEntry>) -> Vec<u32> {
    let entries = entries.into_iter();
    let mut result = Vec::with_capacity(entries.size_hint().0);
    for entry in entries {
        match entry {
            DeclEntry::Node(n) => result.push(*n),
            DeclEntry::Set(s) => result.extend(s.borrow_mut().iterate(arena)),
            DeclEntry::Map(m) => result.extend(m.borrow_mut().iterate(arena)),
        }
    }
    result
}
