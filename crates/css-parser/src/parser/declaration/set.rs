//! Port of `src/parser/declaration/set.ts`.

use super::{str_list, JsMap};
use crate::ast::arena::{is_numeric_token_type, AstArena, StrOrId, TokenValue};
use crate::ast::types::{EnumAstNodeStatus, EnumToken};
use crate::parser::utils::eq::eq_token;
use crate::syntax::syntax::is_length;
use serde_json::Value;

fn dedup(arena: &AstArena, values: &mut [Vec<u32>]) {
    let t = |x: u32| arena.data_types.get(x);
    for value in values.iter_mut() {
        let mut i = value.len();
        while i > 1 {
            i -= 1;
            let tk = value[i];
            let k = value[if i == 1 { 0 } else { i % 2 }];
            if is_numeric_token_type(t(tk))
                && is_numeric_token_type(t(k))
                && arena.numeric_value.get(tk) == 0.0
                && arena.numeric_value.get(k) == 0.0
                && !arena.has_fraction(tk)
                && !arena.has_fraction(k)
                && ((t(tk) == EnumToken::NumberTokenType && is_length(arena, k))
                    || (t(k) == EnumToken::NumberTokenType && is_length(arena, tk))
                    || is_length(arena, k)
                    || is_length(arena, tk))
            {
                value.remove(i);
                continue;
            }
            if eq_token(arena, tk, k) {
                value.remove(i);
                continue;
            }
            break;
        }
    }
}

/// set of declarations that can be merged into a single shorthand declaration
pub struct PropertySet {
    config: &'static Value,
    pub declarations: JsMap<u32>,
}

impl PropertySet {
    pub fn new(config: &'static Value) -> Self {
        Self { config, declarations: JsMap::new() }
    }

    fn shorthand(&self) -> &'static str {
        self.config["shorthand"].as_str().unwrap_or("")
    }

    pub fn add(&mut self, arena: &AstArena, declaration: u32) -> &mut Self {
        let nam = arena.resolve_string(arena.name.get(declaration));
        let shorthand = self.shorthand();
        if nam == shorthand {
            self.declarations = JsMap::new();
        } else if let Some(&existing) = self.declarations.get(shorthand) {
            // expand the shorthand
            let types = str_list(&self.config["types"]);
            let keywords = str_list(&self.config["keywords"]);
            let separator = self.config.get("separator").and_then(|s| s.get("val")).and_then(|v| v.as_str());
            let mut is_valid = true;
            let mut tokens: Vec<Vec<u32>> = Vec::new();
            for token in arena.token_iter(existing) {
                let typ = arena.data_types.get(token);
                if types.iter().any(|t| EnumToken::value(t) == Some(typ))
                    || (typ == EnumToken::NumberTokenType
                        && arena.numeric_value.get(token) == 0.0
                        && (types.contains(&"Length") || types.contains(&"Angle") || types.contains(&"Dimension")))
                {
                    if tokens.is_empty() {
                        tokens.push(Vec::new());
                    }
                    tokens.last_mut().unwrap().push(token);
                    continue;
                }
                if typ != EnumToken::WhitespaceTokenType && typ != EnumToken::CommentTokenType {
                    if typ == EnumToken::IdenTokenType && keywords.contains(&&*arena.resolve_string(arena.string_value.get(token))) {
                        if tokens.is_empty() {
                            tokens.push(Vec::new());
                        }
                        tokens.last_mut().unwrap().push(token);
                    }
                    if typ == EnumToken::LiteralTokenType && separator.is_some_and(|s| arena.string_equals(arena.string_value.get(token), s)) {
                        tokens.push(Vec::new());
                        continue;
                    }
                    is_valid = false;
                    break;
                }
            }
            if is_valid && !tokens.is_empty() {
                self.declarations.delete(shorthand);
                let properties = str_list(&self.config["properties"]);
                for values in &tokens {
                    for (index, property) in properties.iter().enumerate() {
                        if !self.declarations.has(property) {
                            let node = arena.create_declaration(EnumToken::DeclarationNodeType, StrOrId::Str(property), Some(&[]), 0, None);
                            self.declarations.set(*property, node);
                        }
                        let mut index = index;
                        while index > 0 && index >= values.len() {
                            if index > 1 {
                                index %= 2;
                            } else {
                                index = 0;
                                break;
                            }
                        }
                        let node = *self.declarations.get(property).unwrap();
                        if arena.first_token.get(node) != 0 {
                            arena.append_token(node, arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None));
                        }
                        // typescript: `values[index]` may be undefined
                        if let Some(&value) = values.get(index) {
                            arena.append_token(node, arena.clone_node_deep(value, true, None));
                        }
                    }
                }
            }
            self.declarations.set(nam, declaration);
            return self;
        }
        self.declarations.set(nam, declaration);
        self
    }

    pub fn is_shorthand(&self) -> bool {
        if self.declarations.has(self.shorthand()) {
            return self.declarations.len() == 1;
        }
        self.config["properties"].as_array().map_or(0, |a| a.len()) == self.declarations.len()
    }

    /// the declarations: the merged shorthand when the set is complete (`[Symbol.iterator]()`)
    pub fn iterate(&mut self, arena: &AstArena) -> Vec<u32> {
        let values: Vec<u32> = self.declarations.values().copied().collect();
        let properties = str_list(&self.config["properties"]);
        // the !important declarations are not merged
        if !self.declarations.has(self.shorthand()) && values.iter().any(|&d| arena.flags.get(d) & EnumAstNodeStatus::ImportantDeclaration != 0) {
            return values;
        }
        if self.declarations.len() < properties.len() {
            if self.is_shorthand() {
                let val: Vec<u32> = arena
                    .tokens(values[0])
                    .into_iter()
                    .filter(|&c| !matches!(arena.data_types.get(c), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType))
                    .collect();
                let mut list: Vec<u32> = Vec::new();
                for curr in val {
                    if !list.is_empty() {
                        list.push(arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None));
                    }
                    list.push(curr);
                }
                arena.set_token_list(values[0], &list);
            }
            return values;
        }
        let mut groups: Vec<Vec<u32>> = Vec::new();
        for property in &properties {
            let mut index = 0;
            let tokens = self.declarations.get(property).map_or_else(Vec::new, |&id| arena.tokens(id));
            for token in tokens {
                if arena.data_types.get(token) == EnumToken::WhitespaceTokenType {
                    continue;
                }
                if groups.len() == index {
                    groups.push(Vec::new());
                }
                groups[index].push(token);
                index += 1;
            }
        }
        if self.config.get("valueSeparator").is_none_or(|v| v.is_null()) {
            dedup(arena, &mut groups);
        }
        let separator = self.config.get("separator").and_then(|s| s.get("val")).and_then(|v| v.as_str());
        let mut acc: Vec<u32> = Vec::new();
        for mut curr in groups {
            if curr.len() > 1 {
                let k = curr.len() * 2 - 1;
                let mut i = 1;
                while i < k {
                    curr.insert(i, arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None));
                    i += 2;
                }
            }
            if !acc.is_empty() {
                acc.push(arena.create_token(EnumToken::LiteralTokenType, separator.map_or(TokenValue::None, TokenValue::Str), None));
            }
            for token in curr {
                // tokens are shared with the original declarations
                acc.push(if arena.parents.get(token) != 0 { arena.clone_node_deep(token, true, None) } else { token });
            }
        }
        vec![arena.create_declaration(EnumToken::DeclarationNodeType, StrOrId::Str(self.shorthand()), Some(&acc), 0, None)]
    }
}
