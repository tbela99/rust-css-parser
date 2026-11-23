//! Port of `src/parser/declaration/list.ts`.

use super::map::PropertyMap;
use super::set::PropertySet;
use super::{flatten, has_key, DeclEntry, JsMap};
use crate::ast::arena::{AstArena, TokenValue};
use crate::ast::cache::StructuralMemo;
use crate::ast::types::{EnumAstNodeStatus, EnumToken};
use crate::options::{ParserOptions, RemoveDuplicateDeclarations};
use crate::parser::parse::parse_string;
use crate::parser::utils::config::{config_index, ConfigIndex};
use crate::parser::utils::text::{equals_ignore_case, lowercase};
use crate::validation::config::get_parsed_syntax;
use crate::validation::match_::{is_valid_declaration_value, Syntax};
use crate::validation::parser::typedef::ValidationSyntaxGroupEnum;
use serde_json::Value;
use std::cell::RefCell;
use crate::utils::fx::HashMap;
use std::rc::Rc;
use crate::ast::strings::SharedStr;

/// results shared by the property lists of a minification, valid for the arena they were computed for: the keys of
/// the declarations that are not merged by name (identical declarations have the same key) and the tokens of the
/// default values (each value is parsed once, the tokens are cloned)
pub struct DeclarationMemo {
    /// key by structure. same equality as the structural signature
    keys: StructuralMemo<Rc<str>>,
    /// parsed values: detached tokens that are cloned, never attached to the tree
    parsed: HashMap<String, Vec<u32>>,
}

impl Default for DeclarationMemo {
    fn default() -> Self {
        Self { keys: StructuralMemo::new(true), parsed: HashMap::default() }
    }
}

impl DeclarationMemo {
    /// tokens of a value: the value is parsed once, the tokens are cloned
    pub fn parse(&mut self, arena: &AstArena, value: &str) -> Vec<u32> {
        let template = self.parsed.entry(value.to_string()).or_insert_with(|| parse_string(arena, value));
        template.iter().map(|&t| arena.clone_node_deep(t, true, None)).collect()
    }

    /// unique key of the structure of a node: identical nodes have the same key
    pub fn key(&mut self, arena: &AstArena, node: u32) -> Rc<str> {
        let slot = self.keys.token(arena, node);
        if let Some(key) = self.keys.get(slot) {
            return key.clone();
        }
        // not a property name
        let key: Rc<str> = Rc::from(format!("\0{}", slot));
        self.keys.set(slot, key.clone());
        key
    }
}

/// true if the value of a declaration contains var(), env() or if()
fn has_wild_card_function(arena: &AstArena, declaration: u32) -> bool {
    arena
        .tokens(declaration)
        .iter()
        .any(|&t| matches!(arena.data_types.get(t), EnumToken::WildCardFunctionTokenType | EnumToken::WildCardFunctionTokenDefType))
}

/// a list of declarations: computes the shorthands and removes the duplicate declarations
pub struct PropertyList<'o> {
    options: &'o ParserOptions,
    declarations: JsMap<DeclEntry>,
    memo: Rc<RefCell<DeclarationMemo>>,
    /// syntaxes of the descriptors of the at-rule that contains the declarations (`result` in `@function`)
    descriptors: Option<Rc<HashMap<String, Rc<Vec<Syntax>>>>>,
}

impl<'o> PropertyList<'o> {
    pub fn new(options: &'o ParserOptions, memo: Option<Rc<RefCell<DeclarationMemo>>>, descriptors: Option<Rc<HashMap<String, Rc<Vec<Syntax>>>>>) -> Self {
        Self { options, declarations: JsMap::new(), memo: memo.unwrap_or_default(), descriptors }
    }

    fn key(&self, arena: &AstArena, node: u32) -> Rc<str> {
        self.memo.borrow_mut().key(arena, node)
    }

    /// keep the declaration by key: a previous identical declaration is replaced (moved to the end)
    fn set_by_key(&mut self, arena: &AstArena, declaration: u32) {
        let key = self.key(arena, declaration);
        self.declarations.delete(&key);
        self.declarations.set(key, DeclEntry::Node(declaration));
    }

    /// add a declaration
    pub fn add_declaration(&mut self, arena: &AstArena, declaration: u32) -> &mut Self {
        if arena.data_types.get(declaration) != EnumToken::DeclarationNodeType {
            let key = self.key(arena, declaration);
            self.declarations.set(key, DeclEntry::Node(declaration));
            return self;
        }
        // the interned name, shared: no copy
        let name: SharedStr = arena.resolve_string(arena.name.get(declaration));
        let flags = arena.flags.get(declaration);
        // @function / result
        if flags & EnumAstNodeStatus::Unknown != 0 && equals_ignore_case("result", &name) {
            self.declarations.set(name, DeclEntry::Node(declaration));
            return self;
        }
        let keep_duplicates = match &self.options.remove_duplicate_declarations {
            Some(RemoveDuplicateDeclarations::Names(names)) => names.iter().any(|n| **n == *name),
            Some(RemoveDuplicateDeclarations::Bool(b)) => !b,
            None => true,
        };
        if flags & (EnumAstNodeStatus::Invalid | EnumAstNodeStatus::Unknown | EnumAstNodeStatus::ValidationFailed) != 0
            || equals_ignore_case("composes", &name)
            || keep_duplicates
        {
            self.set_by_key(arena, declaration);
            return self;
        }
        if self.options.compute_shorthand != Some(true) {
            self.declarations.set(name, DeclEntry::Node(declaration));
            return self;
        }
        if arena.flags.get(declaration) & EnumAstNodeStatus::Unvalidated != 0 {
            let lower = lowercase(&name);
            let syntax_rules: Option<Rc<Vec<Syntax>>> =
                self.descriptors.as_ref().and_then(|d| d.get(&*lower).cloned()).or_else(|| get_parsed_syntax(ValidationSyntaxGroupEnum::Declarations, &lower));
            if let Some(rules) = syntax_rules {
                let state = if is_valid_declaration_value(arena, &rules, declaration, self.options) {
                    EnumAstNodeStatus::Validated
                } else {
                    EnumAstNodeStatus::ValidationFailed
                };
                arena.flags.set(declaration, arena.flags.get(declaration) | (state & !EnumAstNodeStatus::Unvalidated));
            }
        }
        // do not compute shorthand for invalid declarations
        if arena.flags.get(declaration) & EnumAstNodeStatus::Validated == 0 {
            self.set_by_key_no_delete(arena, declaration);
            return self;
        }
        let property_name = name.clone();
        let index = config_index();
        let (short_hand_type, shorthand): (&str, Option<&'static Value>) = if let Some(p) = index.properties.get(&*property_name) {
            if has_key(p, "map") {
                ("map", p.get("map"))
            } else {
                ("set", p.get("shorthand"))
            }
        } else if let Some(m) = index.map.get(&*property_name) {
            ("map", m.get("shorthand"))
        } else if let Some(p) = index.property.get(&*property_name) {
            ("single", Some(*p))
        } else {
            ("", None)
        };
        let shorthand_name: Option<&'static str> = shorthand.and_then(|s| s.as_str());
        // the value of a shorthand that contains var(), env() or if() cannot be expanded
        if shorthand_name == Some(&*name) && (short_hand_type == "map" || short_hand_type == "set") && has_wild_card_function(arena, declaration) {
            self.set_by_key_no_delete(arena, declaration);
            return self;
        }
        if let Some(sh) = shorthand_name {
            if let Some(existing) = self.declarations.get(sh) {
                let existing: Option<u32> = match existing {
                    DeclEntry::Node(n) => Some(*n),
                    DeclEntry::Set(s) => s.borrow().declarations.get(&name).copied(),
                    DeclEntry::Map(m) => match m.borrow().declarations.get(&name) {
                        Some(DeclEntry::Node(n)) => Some(*n),
                        _ => None,
                    },
                };
                if let Some(existing) = existing {
                    if arena.flags.get(existing) & EnumAstNodeStatus::ImportantDeclaration != 0
                        && arena.flags.get(declaration) & EnumAstNodeStatus::ImportantDeclaration == 0
                    {
                        return self;
                    }
                }
            }
        }
        match short_hand_type {
            "map" => {
                let shorthand = shorthand_name.unwrap_or("");
                let map_name = index.map.get(&*property_name).and_then(|m| m.get("map")).and_then(|m| m.as_str()).or_else(|| {
                    index.map.get(shorthand).and_then(|s| s.get("properties")).and_then(|p| p.get(&*property_name)).and_then(|p| p.get("map")).and_then(|m| m.as_str())
                });
                let memo = Some(self.memo.clone());
                match map_name {
                    Some(map_name) => {
                        if !self.declarations.has(map_name) {
                            self.declarations.set(SharedStr::from_static(map_name), DeclEntry::Map(Rc::new(RefCell::new(PropertyMap::new(ConfigIndex::get(&index.map, map_name), memo.clone())))));
                        }
                        let Some(DeclEntry::Map(owner)) = self.declarations.get(map_name).cloned() else {
                            return self;
                        };
                        if !owner.borrow().declarations.has(shorthand) {
                            owner.borrow_mut().declarations.set(SharedStr::from_static(shorthand), DeclEntry::Map(Rc::new(RefCell::new(PropertyMap::new(ConfigIndex::get(&index.map, shorthand), memo)))));
                        }
                        let target = owner.borrow().declarations.get(shorthand).cloned();
                        if let Some(DeclEntry::Map(target)) = target {
                            target.borrow_mut().add(arena, declaration);
                        }
                    }
                    None => {
                        if !self.declarations.has(shorthand) {
                            self.declarations.set(SharedStr::from_static(shorthand), DeclEntry::Map(Rc::new(RefCell::new(PropertyMap::new(ConfigIndex::get(&index.map, shorthand), memo)))));
                        }
                        if let Some(DeclEntry::Map(target)) = self.declarations.get(shorthand).cloned() {
                            target.borrow_mut().add(arena, declaration);
                        }
                    }
                }
            }
            "set" => {
                let shorthand = shorthand_name.unwrap_or("");
                if !self.declarations.has(shorthand) {
                    self.declarations.set(SharedStr::from_static(shorthand), DeclEntry::Set(Rc::new(RefCell::new(PropertySet::new(ConfigIndex::get(&index.properties, shorthand))))));
                }
                if let Some(DeclEntry::Set(set)) = self.declarations.get(shorthand).cloned() {
                    set.borrow_mut().add(arena, declaration);
                }
            }
            _ => {
                if let Some(mapping) = shorthand.filter(|s| !s.is_null()) {
                    self.map_values(arena, declaration, mapping);
                }
                // a previous declaration with the same name takes precedence when it is !important and the new one is not
                if let Some(DeclEntry::Node(existing)) = self.declarations.get(&*property_name) {
                    if arena.flags.get(*existing) & EnumAstNodeStatus::ImportantDeclaration != 0
                        && arena.flags.get(declaration) & EnumAstNodeStatus::ImportantDeclaration == 0
                    {
                        return self;
                    }
                }
                self.declarations.set(property_name, DeclEntry::Node(declaration));
            }
        }
        self
    }

    /// `declarations.set(memo.key(declaration), declaration)`
    fn set_by_key_no_delete(&mut self, arena: &AstArena, declaration: u32) {
        let key = self.key(arena, declaration);
        self.declarations.set(key, DeclEntry::Node(declaration));
    }

    /// map the values of a single property (`mapValues()`)
    fn map_values(&mut self, arena: &AstArena, declaration: u32, mapping: &Value) {
        let declaration_values = arena.tokens(declaration);
        let mut values: Option<Vec<u32>> = None;
        if let Some(patterns_list) = mapping.get("pattern").and_then(|p| p.as_array()) {
            for patterns in patterns_list {
                // insertion ordered set
                let mut set: Vec<u32> = declaration_values
                    .iter()
                    .copied()
                    .filter(|&v| !matches!(arena.data_types.get(v), EnumToken::CommentTokenType | EnumToken::WhitespaceTokenType))
                    .collect();
                let mut val: Vec<u32> = Vec::new();
                for pattern in super::str_list(patterns) {
                    if pattern == "<length>" {
                        // falls through to the default case
                        set.retain(|&v| {
                            let matched = arena.data_types.get(v) == EnumToken::LengthTokenType
                                || (arena.data_types.get(v) == EnumToken::NumberTokenType && arena.numeric_value.get(v) == 0.0);
                            if matched {
                                val.push(v);
                            }
                            !matched
                        });
                    }
                    set.retain(|&v| {
                        if arena.data_types.get(v) != EnumToken::IdenTokenType {
                            return true;
                        }
                        let value = arena.resolve_string(arena.string_value.get(v)).to_lowercase();
                        let matched = value == pattern || delimited_match(&value, pattern);
                        if matched {
                            val.push(v);
                        }
                        !matched
                    });
                }
                if set.is_empty() {
                    let mut list: Vec<u32> = Vec::new();
                    for curr in val {
                        if !list.is_empty() {
                            list.push(arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None));
                        }
                        list.push(curr);
                    }
                    values = Some(list);
                    break;
                }
            }
        }
        for &val in &declaration_values {
            if arena.data_types.get(val) == EnumToken::IdenTokenType {
                let name = arena.resolve_string(arena.string_value.get(val)).to_lowercase();
                if let Some(m) = mapping.get("mapping").and_then(|m| m.get(&name)).filter(|m| !m.is_null()) {
                    let typ = m.get("typ").and_then(|t| t.as_str()).and_then(EnumToken::value).unwrap_or(0);
                    let v = m.get("val");
                    let value = match v {
                        Some(Value::String(s)) => TokenValue::Str(s),
                        Some(Value::Number(n)) => TokenValue::Num(n.as_f64().unwrap_or(0.0)),
                        _ => TokenValue::None,
                    };
                    arena.reset_token(val, typ, value, None);
                }
            }
        }
        if let Some(values) = values {
            arena.set_token_list(declaration, &values);
        }
    }

    /// the declarations of the list, the shorthands computed (`[Symbol.iterator]()`)
    pub fn iterate(&mut self, arena: &AstArena) -> Vec<u32> {
        flatten(arena, self.declarations.values())
    }
}

/// `new RegExp("(^|\\|)" + pattern + "(\\||$)").test(value)`. the pattern is not grouped: `a|b|c` is the alternation
/// of `(^|\|)a`, `b` and `c(\||$)`
fn delimited_match(value: &str, pattern: &str) -> bool {
    let alternatives: Vec<&str> = pattern.split('|').collect();
    let last = alternatives.len() - 1;
    alternatives.iter().enumerate().any(|(i, alt)| {
        let starts = i == 0;
        let ends = i == last;
        value.match_indices(alt).any(|(at, _)| {
            let end = at + alt.len();
            let before = !starts || at == 0 || value.as_bytes()[at - 1] == b'|';
            let after = !ends || end == value.len() || value.as_bytes()[end] == b'|';
            before && after
        }) || (alt.is_empty() && (!starts || !ends || value.is_empty() || value.contains('|')))
    })
}
