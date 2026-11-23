//! Port of `src/parser/declaration/map.ts`.

use super::list::DeclarationMemo;
use super::set::PropertySet;
use super::{config_token, config_type, flatten, has_key, match_config_token, str_list, DeclEntry, JsMap};
use crate::ast::arena::{AstArena, StrOrId, TokenValue};
use crate::ast::types::EnumToken;
use crate::options::ParserOptions;
use crate::parser::parse::parse_string;
use crate::parser::utils::config::get_config;
use crate::parser::utils::eq::eq_token;
use crate::parser::utils::type_::match_type;
use crate::printer::render::render_value;
use serde_json::Value;
use std::cell::RefCell;
use crate::utils::fx::HashMap;
use std::rc::Rc;

/// tokens of a longhand and their rendered values (`TokenMap`)
struct TokenMap {
    t: Vec<u32>,
    value: Vec<String>,
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

fn non_null(v: Option<&Value>) -> Option<&Value> {
    v.filter(|v| !v.is_null())
}

thread_local! {
    /// the required count and the pattern of each map configuration, by configuration address: the configuration is
    /// static, they are computed once
    static MAP_SETTINGS: RefCell<HashMap<usize, (usize, Rc<[Rc<str>]>)>> = RefCell::new(HashMap::default());
}

/// the number of required properties and the pattern of a map configuration
fn map_settings(config: &'static Value) -> (usize, Rc<[Rc<str>]>) {
    let key = config as *const Value as usize;
    if let Some(settings) = MAP_SETTINGS.with(|m| m.borrow().get(&key).cloned()) {
        return settings;
    }
    let values: Vec<&Value> = config["properties"].as_object().map(|o| o.values().collect()).unwrap_or_default();
    let required = values.iter().filter(|v| truthy(v.get("required"))).count();
    let required_count = if required == 0 { values.len() } else { required };
    let pattern: Rc<[Rc<str>]> = config["pattern"].as_str().unwrap_or("").split(char::is_whitespace).map(Rc::from).collect();
    MAP_SETTINGS.with(|m| m.borrow_mut().insert(key, (required_count, pattern.clone())));
    (required_count, pattern)
}

/// map of declarations that can be merged into a single shorthand declaration
pub struct PropertyMap {
    pub declarations: JsMap<DeclEntry>,
    pub config: &'static Value,
    required_count: usize,
    pattern: Rc<[Rc<str>]>,
    /// longhand property name of the matched tokens
    pub property_names: HashMap<u32, Rc<str>>,
    /// results shared by the lists of a minification: the default values are parsed once
    memo: Option<Rc<RefCell<DeclarationMemo>>>,
}

impl PropertyMap {
    pub fn new(config: &'static Value, memo: Option<Rc<RefCell<DeclarationMemo>>>) -> Self {
        let (required_count, pattern) = map_settings(config);
        Self { declarations: JsMap::new(), config, required_count, pattern, property_names: HashMap::default(), memo }
    }

    fn shorthand(&self) -> &'static str {
        self.config["shorthand"].as_str().unwrap_or("")
    }

    fn props(&self, property: &str) -> &'static Value {
        &self.config["properties"][property]
    }

    /// tokens of a value: parsed once when a memo is shared
    fn parse(&self, arena: &AstArena, value: &str) -> Vec<u32> {
        match &self.memo {
            None => parse_string(arena, value),
            Some(memo) => memo.borrow_mut().parse(arena, value),
        }
    }

    pub fn add(&mut self, arena: &AstArena, declaration: u32) -> &mut Self {
        let nam = arena.resolve_string(arena.name.get(declaration));
        let shorthand = self.shorthand();
        if nam == shorthand {
            self.declarations.clear();
            self.declarations.set(nam, DeclEntry::Node(declaration));
            self.match_types(arena, declaration);
            return self;
        }
        let separator = non_null(self.config.get("separator"));
        // expand the shorthand
        if let Some(DeclEntry::Node(existing)) = self.declarations.get(shorthand).cloned() {
            let mut tokens: JsMap<Vec<Vec<u32>>> = JsMap::new();
            let mut values: Vec<u32> = Vec::new();
            let mut val: Vec<Vec<u32>> = vec![Vec::new()];
            for curr in arena.tokens(existing) {
                if match_config_token(arena, curr, separator) {
                    val.push(Vec::new());
                    continue;
                }
                val.last_mut().unwrap().push(curr);
            }
            for current in 0..val.len() {
                let mut acc = std::mem::take(&mut val[current]);
                for property in self.pattern.clone().iter() {
                    self.expand_property(arena, &mut acc, &property, current, &mut tokens);
                }
                values.extend(acc);
            }
            if values.is_empty() {
                let mut declarations: JsMap<DeclEntry> = JsMap::new();
                for (name, groups) in tokens.iter() {
                    let mut list: Vec<u32> = Vec::new();
                    for group in groups {
                        if !list.is_empty() {
                            list.push(config_token(arena, separator));
                        }
                        for &token in group {
                            list.push(self.take(arena, token));
                        }
                    }
                    let node = arena.create_declaration(EnumToken::DeclarationNodeType, StrOrId::Str(name), Some(&list), 0, None);
                    declarations.set(name, DeclEntry::Node(node));
                }
                self.declarations = declarations;
            }
        }
        let properties_config = &get_config()["properties"];
        match non_null(properties_config.get(&*nam)) {
            Some(config) => {
                let property = config["shorthand"].as_str().unwrap_or("").to_string();
                let value = self.declarations.get(&property).cloned();
                let set = match value {
                    Some(DeclEntry::Set(set)) => set,
                    other => {
                        let set = Rc::new(RefCell::new(PropertySet::new(&properties_config[property.as_str()])));
                        self.declarations.set(property.clone(), DeclEntry::Set(set.clone()));
                        if let Some(DeclEntry::Node(v)) = other {
                            set.borrow_mut().add(arena, v);
                        }
                        set
                    }
                };
                set.borrow_mut().add(arena, declaration);
            }
            None => self.declarations.set(nam, DeclEntry::Node(declaration)),
        }
        self
    }

    /// match the tokens of a value group with a property of the pattern (the `pattern.reduce()` callback)
    fn expand_property(&mut self, arena: &AstArena, acc: &mut Vec<u32>, property: &str, current: usize, tokens: &mut JsMap<Vec<Vec<u32>>>) {
        let props = self.props(property);
        let has_prefix = has_key(props, "prefix");
        let filled = |tokens: &JsMap<Vec<Vec<u32>>>| tokens.get(property).is_some_and(|t| t.len() > current);
        let mut i: isize = 0;
        while (i as usize) < acc.len() {
            let token = acc[i as usize];
            if matches!(arena.data_types.get(token), EnumToken::CommentTokenType | EnumToken::WhitespaceTokenType) {
                acc.remove(i as usize);
                continue;
            }
            if self.property_names.get(&token).map(|n| &**n) == Some(property) || match_type(arena, token, props) {
                if has_prefix {
                    if let Some(previous) = non_null(props.get("previous")).and_then(|p| p.as_str()) {
                        if !tokens.has(previous) {
                            return;
                        }
                    }
                }
                match tokens.get_mut(property) {
                    None => tokens.set(property, vec![vec![token]]),
                    Some(t) => {
                        if current == t.len() {
                            t.push(vec![token]);
                        } else if let Some(group) = t.get_mut(current) {
                            group.push(arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None));
                            group.push(token);
                        }
                    }
                }
                acc.remove(i as usize);
                i -= 1;
                if has_prefix && i >= 0 && match_config_token(arena, acc[i as usize], props.get("prefix")) {
                    acc.remove(i as usize);
                    i -= 1;
                }
                if truthy(props.get("multiple")) {
                    i += 1;
                    continue;
                }
                return;
            } else if filled(tokens) {
                return;
            }
            i += 1;
        }
        if filled(tokens) {
            return;
        }
        // default
        let defaults = str_list(&props["default"]);
        if let Some(default) = defaults.first() {
            let parsed = self.parse(arena, default);
            match tokens.get_mut(property) {
                None => tokens.set(property, vec![parsed]),
                Some(t) => {
                    if current == t.len() {
                        t.push(parsed);
                    } else if let Some(group) = t.get_mut(current) {
                        group.push(arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None));
                        group.extend(parsed);
                    }
                }
            }
        }
    }

    /// the declarations: the merged shorthand when the map is complete (`[Symbol.iterator]()`)
    pub fn iterate(&mut self, arena: &AstArena) -> Vec<u32> {
        let properties_mapping = &self.config["properties"];
        let mapping_entries: Vec<(&'static str, &'static Value)> =
            properties_mapping.as_object().map(|o| o.iter().map(|(k, v)| (k.as_str(), v)).collect()).unwrap_or_default();
        let patterns: Vec<&str> = self.config["pattern"].as_str().unwrap_or("").split(' ').collect();
        let shorthand = self.shorthand();
        let has_mapping = self.declarations.values().any(|v| matches!(v, DeclEntry::Map(_)));
        let mut declarations: JsMap<DeclEntry> = self.declarations.clone();
        if has_mapping {
            let mut mapped: JsMap<DeclEntry> = JsMap::new();
            for value in self.declarations.values().cloned().collect::<Vec<_>>() {
                match value {
                    DeclEntry::Map(m) => {
                        let (entries, m_shorthand, m_config) = {
                            let mb = m.borrow();
                            (mb.declarations.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<Vec<_>>(), mb.shorthand(), mb.config)
                        };
                        for (k, v) in entries {
                            if k == m_shorthand {
                                let DeclEntry::Node(v) = v else {
                                    continue;
                                };
                                let values = arena.tokens(v);
                                let keys: Vec<&str> = m_config["properties"].as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
                                for key in keys {
                                    let clone = arena.clone_node_deep(v, false, None);
                                    mapped.set(key, DeclEntry::Node(clone));
                                    let mut mapped_values: Vec<u32> = Vec::new();
                                    for &value in &values {
                                        let is_key = m.borrow().property_names.get(&value).map(|n| &**n) == Some(key);
                                        if is_key {
                                            if !mapped_values.is_empty() {
                                                mapped_values.push(arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None));
                                            }
                                            let taken = m.borrow_mut().take(arena, value);
                                            mapped_values.push(taken);
                                        }
                                    }
                                    if mapped_values.is_empty() {
                                        let default = m_config["properties"][key]["default"][0].as_str().unwrap_or("");
                                        mapped_values.extend(self.parse(arena, default));
                                    }
                                    arena.set_token_list(clone, &mapped_values);
                                }
                            } else {
                                mapped.set(k, v);
                            }
                        }
                    }
                    // a property set has no name
                    DeclEntry::Set(_) => mapped.set("undefined", value),
                    DeclEntry::Node(n) => mapped.set(arena.resolve_string(arena.name.get(n)).to_string(), value),
                }
            }
            if patterns.len() == mapped.len() {
                declarations = JsMap::new();
                for key in &patterns {
                    if let Some(v) = mapped.get(key) {
                        declarations.set(*key, v.clone());
                    }
                }
            }
        }
        // the loop variable is read after the loop (`this.propertyNames.get(val) === property`)
        let mut property: Option<&str> = None;
        let mut required_count = 0usize;
        let mut is_shorthand = true;
        for &(key, cfg) in &mapping_entries {
            property = Some(key);
            if truthy(cfg.get("required")) {
                match declarations.get(key) {
                    None => {
                        is_shorthand = false;
                        break;
                    }
                    Some(DeclEntry::Set(s)) if !s.borrow().is_shorthand() => {
                        is_shorthand = false;
                        break;
                    }
                    Some(_) => required_count += 1,
                }
            }
        }
        if required_count == 0 {
            required_count = declarations.len();
        }
        let entries: Vec<DeclEntry> = declarations.values().cloned().collect();
        if !is_shorthand || required_count < self.required_count {
            if is_shorthand && declarations.has(shorthand) {
                let mut cache: HashMap<u32, String> = HashMap::default();
                let values = flatten(arena, &entries);
                let mut is_important = false;
                let mut filtered: Vec<u32> = Vec::new();
                for &declaration in &values {
                    let dec = self.remove_defaults_of(arena, declaration, &mut cache);
                    for t in arena.tokens(dec) {
                        let typ = arena.data_types.get(t);
                        if typ == EnumToken::ImportantTokenType {
                            is_important = true;
                        }
                        if filtered.is_empty() && typ != EnumToken::WhitespaceTokenType && typ != EnumToken::ImportantTokenType {
                            filtered.push(dec);
                        }
                    }
                }
                let defaults = str_list(&self.config["default"]);
                if filtered.is_empty() && !defaults.is_empty() {
                    let parsed = self.parse(arena, defaults[0]);
                    filtered.push(arena.create_declaration(EnumToken::DeclarationNodeType, StrOrId::Str(shorthand), Some(&parsed), 0, None));
                    if is_important {
                        arena.append_token(filtered[0], arena.create_token(EnumToken::ImportantTokenType, TokenValue::None, None));
                    }
                }
                return if filtered.is_empty() { values } else { filtered };
            }
            for entry in &entries {
                let DeclEntry::Node(declaration) = entry else {
                    continue;
                };
                let nam = arena.resolve_string(arena.name.get(*declaration));
                let config = if nam == shorthand { self.config } else { non_null(properties_mapping.get(&*nam)).unwrap_or(self.config) };
                let Some(mapping) = config.get("mapping") else {
                    continue;
                };
                let declaration_values = arena.tokens(*declaration);
                for (key, val) in mapping.as_object().into_iter().flatten() {
                    let keys = self.parse(arena, key);
                    if keys.len() != declaration_values.len() {
                        continue;
                    }
                    if keys.iter().zip(&declaration_values).all(|(&k, &v)| eq_token(arena, k, v)) {
                        let parsed = self.parse(arena, val.as_str().unwrap_or(""));
                        arena.set_token_list(*declaration, &parsed);
                        break;
                    }
                }
            }
            return flatten(arena, &entries);
        }
        let mut count = 0usize;
        let separator = non_null(self.config.get("separator"));
        let mut tokens: JsMap<Vec<Vec<u32>>> = JsMap::new();
        for &(name, props) in &mapping_entries {
            let Some(entry) = declarations.get(name).cloned() else {
                continue;
            };
            let mut current = 0usize;
            let decls = flatten(arena, &[entry]);
            for declaration in decls {
                for val in arena.tokens(declaration) {
                    if match_config_token(arena, val, separator) {
                        current += 1;
                        if let Some(t) = tokens.get_mut(name) {
                            if t.len() == current {
                                t.push(Vec::new());
                            }
                        }
                        continue;
                    }
                    let val_type = arena.data_types.get(val);
                    if val_type == EnumToken::WhitespaceTokenType || val_type == EnumToken::CommentTokenType {
                        continue;
                    }
                    if truthy(props.get("multiple")) {
                        if let Some(sep) = non_null(props.get("separator")) {
                            if config_type(sep) == val_type
                                && (val_type == EnumToken::CommaTokenType
                                    || sep.get("val").and_then(|v| v.as_str()).is_some_and(|v| arena.string_equals(arena.string_value.get(val), v)))
                            {
                                continue;
                            }
                        }
                    }
                    let matched = match_type(arena, val, props);
                    if is_shorthand {
                        is_shorthand = matched;
                    }
                    if self.property_names.get(&val).map(|n| &**n) == property || matched {
                        if !tokens.has(name) {
                            tokens.set(name, vec![Vec::new()]);
                        }
                        // is default value
                        if let Some(group) = tokens.get_mut(name).unwrap().get_mut(current) {
                            group.push(val);
                        }
                    } else {
                        break;
                    }
                }
            }
            if count == 0 {
                count = current;
            }
        }
        // grid-template
        if shorthand == "grid-template" && patterns.len() == tokens.len() {
            let first = |p: &str| tokens.get(p).and_then(|t| t.first()).cloned().unwrap_or_default();
            let (t0, t1, t2) = (first(patterns[0]), first(patterns[1]), first(patterns.get(2).copied().unwrap_or("")));
            let (k, l) = (t0.len(), t1.len());
            let j = k.min(l);
            let mut result: Vec<u32> = Vec::new();
            for i in 0..j {
                result.push(t0[i]);
                result.push(t1[i]);
            }
            result.extend_from_slice(&t0[j..]);
            result.extend_from_slice(&t1[j..]);
            let prefix = non_null(self.config["properties"].get(patterns.get(2).copied().unwrap_or("")).and_then(|p| p.get("prefix")));
            result.push(config_token(arena, prefix));
            for (index, &t) in t2.iter().enumerate() {
                if index > 0 {
                    result.push(arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None));
                }
                result.push(t);
            }
            let Some(DeclEntry::Node(source)) = declarations.get(patterns[0]).cloned() else {
                return flatten(arena, &entries);
            };
            let declaration = arena.clone_node_deep(source, false, None);
            arena.name.set(declaration, arena.intern_string(shorthand));
            let taken: Vec<u32> = result.into_iter().map(|t| self.take(arena, t)).collect();
            arena.set_token_list(declaration, &taken);
            return vec![declaration];
        }
        count += 1;
        let missing_required = mapping_entries.iter().any(|(name, cfg)| truthy(cfg.get("required")) && !tokens.has(name));
        let counts_differ = !tokens.values().all(|v| {
            v.iter().filter(|t| t.len() != 1 || arena.data_types.get(t[0]) != EnumToken::CommentTokenType).count() == count
        });
        if !is_shorthand || missing_required || counts_differ {
            return flatten(arena, &entries);
        }
        if has_mapping {
            if patterns.len() == tokens.len() {
                let Some(DeclEntry::Node(source)) = declarations.get(patterns[0]).cloned() else {
                    return flatten(arena, &entries);
                };
                let declaration = arena.clone_node_deep(source, false, None);
                let mut declaration_values = arena.tokens(declaration);
                arena.name.set(declaration, arena.intern_string(shorthand));
                for pattern in &patterns {
                    if !declaration_values.is_empty() {
                        declaration_values.push(config_token(arena, separator));
                    }
                    let groups = tokens.get(pattern).cloned().unwrap_or_default();
                    for (gi, group) in groups.iter().enumerate() {
                        if gi > 0 {
                            declaration_values.push(arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None));
                        }
                        for (ti, &t) in group.iter().enumerate() {
                            if ti > 0 {
                                declaration_values.push(arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None));
                            }
                            declaration_values.push(self.take(arena, t));
                        }
                    }
                }
                arena.set_token_list(declaration, &declaration_values);
                return vec![declaration];
            }
            return flatten(arena, &entries);
        }
        let values = self.shorthand_values(arena, &tokens, required_count, separator);
        vec![arena.create_declaration(EnumToken::DeclarationNodeType, StrOrId::Str(shorthand), Some(&values), 0, None)]
    }

    /// the value tokens of the shorthand: the default values are removed
    fn shorthand_values(&mut self, arena: &AstArena, tokens: &JsMap<Vec<Vec<u32>>>, required_count: usize, separator: Option<&Value>) -> Vec<u32> {
        let properties_mapping = &self.config["properties"];
        let global_properties = &get_config()["properties"];
        let ws = |arena: &AstArena| arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None);
        let mut acc: Vec<Vec<u32>> = Vec::new();
        for (name, groups) in tokens.iter() {
            let props = &properties_mapping[name];
            let defaults = str_list(&props["default"]);
            let required = truthy(props.get("required"));
            for (i, group) in groups.iter().enumerate() {
                if acc.len() == i {
                    acc.push(Vec::new());
                }
                let mut values: Vec<u32> = group.clone();
                let rendered = group.iter().map(|&t| render_value_plain(arena, t) + " ").collect::<String>();
                if defaults.contains(&rendered.trim_end()) && !required {
                    continue;
                }
                // remove default values
                let mut do_filter_default = true;
                if has_key(global_properties, name) {
                    for &v in &values {
                        let typ = arena.data_types.get(v);
                        if !matches!(typ, EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType | EnumToken::IdenTokenType)
                            || (typ == EnumToken::IdenTokenType && !defaults.contains(&&*arena.resolve_string(arena.string_value.get(v))))
                        {
                            do_filter_default = false;
                            break;
                        }
                    }
                }
                let filtered: Vec<u32> = values
                    .iter()
                    .copied()
                    .filter(|&val| {
                        let typ = arena.data_types.get(val);
                        if typ == EnumToken::WhitespaceTokenType || typ == EnumToken::CommentTokenType {
                            return false;
                        }
                        !do_filter_default || !(typ == EnumToken::IdenTokenType && defaults.contains(&&*arena.resolve_string(arena.string_value.get(val))))
                    })
                    .collect();
                if !filtered.is_empty() || !(self.required_count == required_count && required) {
                    values = filtered;
                }
                if values.is_empty() {
                    continue;
                }
                if let Some(mapping) = props.get("mapping") {
                    let constraints = props.get("constraints");
                    let within = match constraints {
                        None => true,
                        Some(c) if !has_key(c, "max") => true,
                        Some(c) => c["mapping"]["max"].as_f64().is_some_and(|max| (values.len() as f64) <= max),
                    };
                    if within {
                        let mut k = values.len();
                        while k > 0 {
                            k -= 1;
                            if arena.data_types.get(values[k]) == EnumToken::IdenTokenType {
                                let key = arena.resolve_string(arena.string_value.get(values[k]));
                                if let Some(replacement) = mapping.get(&*key) {
                                    let parsed = self.parse(arena, replacement.as_str().unwrap_or(""));
                                    values.splice(k..k + 1, parsed);
                                }
                            }
                        }
                    }
                }
                if has_key(props, "prefix") {
                    acc[i].push(config_token(arena, props.get("prefix")));
                } else if !acc[i].is_empty() {
                    acc[i].push(ws(arena));
                }
                let value_separator = non_null(props.get("separator"));
                for (index, &v) in values.iter().enumerate() {
                    if index > 0 {
                        acc[i].push(config_token(arena, value_separator));
                    }
                    let taken = self.take(arena, v);
                    acc[i].push(taken);
                }
            }
        }
        let defaults = str_list(&self.config["default"]);
        let mut values: Vec<u32> = Vec::new();
        for mut curr in acc {
            if !values.is_empty() {
                values.push(config_token(arena, separator));
            }
            if curr.is_empty() && !defaults.is_empty() {
                for (index, t) in self.parse(arena, defaults[0]).into_iter().enumerate() {
                    if index > 0 {
                        curr.push(ws(arena));
                    }
                    curr.push(t);
                }
            }
            values.extend(curr);
        }
        if let Some(mapping) = non_null(self.config.get("mapping")) {
            thread_local! {
                static OPTIONS: ParserOptions = ParserOptions { remove_comments: Some(true), minify: Some(true), ..Default::default() };
            }
            let val: String = OPTIONS.with(|o| values.iter().map(|&t| render_value(arena, t, o)).collect());
            if let Some(replacement) = mapping.get(&val) {
                let typ = if val.starts_with('"') || val.starts_with('\'') { EnumToken::StringTokenType } else { EnumToken::IdenTokenType };
                values = vec![arena.create_token(typ, TokenValue::Str(replacement.as_str().unwrap_or("")), None)];
            }
        }
        if values.len() == 1 && arena.string_value.get(values[0]) != 0 {
            let lower = arena.resolve_string(arena.string_value.get(values[0])).to_lowercase();
            if defaults.contains(&lower.as_str()) && defaults.first() != Some(&lower.as_str()) {
                values = self.parse(arena, defaults[0]);
            }
        }
        values
    }

    /// detach a token from its declaration: tokens are cloned when they are still attached to a declaration
    pub fn take(&mut self, arena: &AstArena, token: u32) -> u32 {
        if arena.parents.get(token) == 0 {
            return token;
        }
        let clone = arena.clone_node_deep(token, true, None);
        if let Some(name) = self.property_names.get(&token).cloned() {
            self.property_names.insert(clone, name);
        }
        clone
    }

    fn match_types(&mut self, arena: &AstArena, declaration: u32) {
        let patterns = self.pattern.clone();
        let mut declaration_values = arena.tokens(declaration);
        let mut values = declaration_values.clone();
        // insertion ordered: the remaining count of each pattern
        let mut map: JsMap<f64> = JsMap::new();
        for pattern in patterns.iter() {
            let mut j: isize = 0;
            while (j as usize) < values.len() {
                if !map.has(pattern) {
                    let max = self.config["properties"].get(&**pattern).and_then(|p| p.get("constraints")).and_then(|c| c.get("mapping")).and_then(|m| m.get("max"));
                    map.set(pattern.clone(), non_null(max).and_then(|m| m.as_f64()).unwrap_or(1.0));
                }
                let count = *map.get(pattern).unwrap();
                if count > 0.0 && match_type(arena, values[j as usize], self.props(pattern)) {
                    self.property_names.insert(values[j as usize], pattern.clone());
                    map.set(pattern.clone(), count - 1.0);
                    values.remove(j as usize);
                    j -= 1;
                }
                j += 1;
            }
        }
        if let Some(set) = non_null(self.config.get("set")).and_then(|s| s.as_object()) {
            let mut modified = false;
            for (key, val) in set {
                if !map.has(key) {
                    continue;
                }
                for v in str_list(val) {
                    // missing
                    if map.get(v) == Some(&1.0) {
                        let mut i = declaration_values.len();
                        while i > 0 {
                            i -= 1;
                            if self.property_names.get(&declaration_values[i]).map(|n| &**n) == Some(key.as_str()) {
                                let clone = arena.clone_node_deep(declaration_values[i], true, None);
                                self.property_names.insert(clone, Rc::from(v));
                                declaration_values.splice(i..i, [clone, arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None)]);
                                modified = true;
                            }
                        }
                    }
                }
            }
            if modified {
                arena.set_token_list(declaration, &declaration_values);
            }
        }
    }

    /// remove the default values of a shorthand declaration (the `removeDefaults()` closure of the iterator)
    fn remove_defaults_of(&mut self, arena: &AstArena, declaration: u32, cache: &mut HashMap<u32, String>) -> u32 {
        let sep = non_null(self.config.get("separator"));
        let typ = match sep {
            None => EnumToken::CommaTokenType,
            Some(s) => config_type(s),
        };
        let separator: &str = match sep {
            None => ",",
            Some(s) => s.get("val").and_then(|v| v.as_str()).unwrap_or(""),
        };
        self.match_types(arena, declaration);
        let mut map: JsMap<TokenMap> = JsMap::new();
        let mut value: Vec<u32> = Vec::new();
        let mut values: Vec<Vec<u32>> = Vec::new();
        for t in arena.tokens(declaration) {
            let rendered = cache.entry(t).or_insert_with(|| render_value_minified(arena, t)).clone();
            if arena.data_types.get(t) == typ && separator == rendered {
                self.remove_defaults(arena, &map, &mut value);
                values.push(std::mem::take(&mut value));
                map.clear();
                continue;
            }
            value.push(t);
            if let Some(property_name) = self.property_names.get(&t).cloned() {
                match map.get_mut(&property_name) {
                    None => map.set(property_name, TokenMap { t: vec![t], value: vec![rendered] }),
                    Some(v) => {
                        v.t.push(t);
                        v.value.push(rendered);
                    }
                }
            }
        }
        self.remove_defaults(arena, &map, &mut value);
        values.push(value);
        let mut result: Vec<u32> = Vec::new();
        for curr in values {
            if sep.is_some() && !result.is_empty() {
                result.push(config_token(arena, sep));
            }
            for cr in curr {
                if arena.data_types.get(cr) == EnumToken::WhitespaceTokenType && result.last().is_some_and(|&l| arena.data_types.get(l) == EnumToken::WhitespaceTokenType) {
                    continue;
                }
                result.push(cr);
            }
        }
        while result.last().is_some_and(|&l| arena.data_types.get(l) == EnumToken::WhitespaceTokenType) {
            result.pop();
        }
        while result.first().is_some_and(|&f| arena.data_types.get(f) == EnumToken::WhitespaceTokenType) {
            result.remove(0);
        }
        arena.set_token_list(declaration, &result);
        declaration
    }

    fn remove_defaults(&self, arena: &AstArena, map: &JsMap<TokenMap>, value: &mut Vec<u32>) {
        let shorthand_defaults = str_list(&self.config["default"]);
        for (key, val) in map.iter() {
            let Some(config) = non_null(self.config["properties"].get(key)) else {
                continue;
            };
            let v = val.value.join(" ");
            let defaults = str_list(&config["default"]);
            if !(defaults.contains(&v.as_str()) || (value.len() == 1 && shorthand_defaults.contains(&v.as_str()))) {
                continue;
            }
            let prefix = non_null(config.get("prefix"));
            // `for (const token of value)` while the array is modified: the iterator index moves on
            let mut idx = 0usize;
            while idx < value.len() {
                let token = value[idx];
                if val.t.contains(&token) {
                    let mut index = value.iter().position(|&x| x == token).unwrap();
                    value.remove(index);
                    if prefix.is_some() {
                        while index > 0 {
                            index -= 1;
                            if arena.data_types.get(value[index]) == EnumToken::WhitespaceTokenType {
                                continue;
                            }
                            if match_config_token(arena, value[index], prefix) {
                                value.remove(index);
                                break;
                            }
                        }
                    }
                }
                idx += 1;
            }
        }
    }
}

/// `renderValue(arena, token)` with the default options
fn render_value_plain(arena: &AstArena, token: u32) -> String {
    crate::printer::render::render_value_default(arena, token)
}

/// `renderValue(arena, token, { minify: true })`
fn render_value_minified(arena: &AstArena, token: u32) -> String {
    thread_local! {
        static OPTIONS: ParserOptions = ParserOptions { minify: Some(true), ..Default::default() };
    }
    OPTIONS.with(|o| render_value(arena, token, o))
}
