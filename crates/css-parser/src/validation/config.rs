//! Port of `src/validation/config.ts`.

use super::json::config_json;
use super::parser::parse::{parse_syntax, trim_syntax_array};
use super::parser::typedef::{ValidationSyntaxGroupEnum, ValidationTokenEnum as V};
use super::parser::types::ValidationToken;
use std::cell::RefCell;
use crate::utils::fx::HashMap;
use std::rc::Rc;

/// parsed syntax rule of an at-rule (or a group)
#[derive(Clone, Debug)]
pub struct ValidationSyntaxRule {
    pub optional_body: bool,
    pub accept_any_declaration: bool,
    pub accept_any_rule: bool,
    pub prelude: Rc<Vec<Rc<ValidationToken>>>,
    pub block: Option<Rc<Vec<Rc<ValidationToken>>>>,
    pub property_descriptors: Option<Rc<HashMap<String, Rc<Vec<Rc<ValidationToken>>>>>>,
}

impl ValidationSyntaxRule {
    pub fn get_prelude_rules(&self) -> Option<Rc<Vec<Rc<ValidationToken>>>> {
        Some(self.prelude.clone())
    }

    pub fn get_block_rules(&self) -> Option<Rc<Vec<Rc<ValidationToken>>>> {
        self.block.clone().filter(|b| !b.is_empty())
    }

    pub fn get_property_descriptors(&self) -> Option<Rc<HashMap<String, Rc<Vec<Rc<ValidationToken>>>>>> {
        self.property_descriptors.clone()
    }
}

/// the validation configuration
pub fn get_syntax_config() -> &'static serde_json::Value {
    config_json()
}

/// find a syntax definition in a group. aliases use the syntax of the standard property. a `syntaxes` lookup falls
/// back to the `types` group
fn lookup_node(group: ValidationSyntaxGroupEnum, key: &str) -> Option<&'static serde_json::Value> {
    let config = config_json();
    let entries = &config[group.as_str()];
    if let Some(node) = entries.get(key) {
        let mut node = node;
        let mut i = 0;
        while let Some(alias) = node.get("alias").and_then(|a| a.as_str()) {
            if i >= 4 {
                break;
            }
            node = entries.get(alias)?;
            i += 1;
        }
        return if node.get("alias").is_some_and(|a| !a.is_null()) { None } else { Some(node) };
    }
    if group == ValidationSyntaxGroupEnum::Syntaxes {
        return config["types"].get(key);
    }
    None
}

/// number of the syntax groups (`ValidationSyntaxGroupEnum`)
const GROUPS: usize = 9;

thread_local! {
    static DEFINED_SYNTAXES: RefCell<HashMap<String, bool>> = RefCell::new(HashMap::default());
    /// parsed syntaxes, by syntax node: the names that share a definition share the parsed syntax
    static PARSED_SYNTAXES: RefCell<HashMap<usize, Rc<Vec<Rc<ValidationToken>>>>> = RefCell::new(HashMap::default());
    /// parsed syntaxes by group (`group as usize`) and name: the lookups borrow the name, it is copied once
    static PARSED_BY_KEY: RefCell<Vec<HashMap<Box<str>, Option<Rc<Vec<Rc<ValidationToken>>>>>>> =
        RefCell::new((0..GROUPS).map(|_| HashMap::default()).collect());
    static SYNTAX_RULES: RefCell<Vec<HashMap<Box<str>, Option<Rc<ValidationSyntaxRule>>>>> =
        RefCell::new((0..GROUPS).map(|_| HashMap::default()).collect());
}

thread_local! {
    /// the definition node of a parsed syntax (by the address of its tokens), see [`syntax_key`]
    static SYNTAX_NODES: RefCell<HashMap<usize, usize>> = RefCell::new(HashMap::default());
    /// the key of the definitions that are compiled (`group/name`), by definition node, see [`syntax_key`]
    static NODE_KEYS: HashMap<usize, String> = {
        let config = config_json();
        let mut keys = HashMap::default();
        for group in ["declarations", "syntaxes", "types"] {
            if let Some(entries) = config[group].as_object() {
                for (key, value) in entries {
                    keys.insert(value as *const serde_json::Value as usize, format!("{}/{}", group, key));
                }
            }
        }
        keys
    };
}

/// the key of the definition of a parsed syntax (`declarations/width`, `types/color`), see `scripts/compile_syntax.rs`.
/// None if the syntax is not the syntax of a declaration, a type or a function (a descriptor, ...)
pub fn syntax_key(syntaxes: &[Rc<ValidationToken>]) -> Option<String> {
    let node = SYNTAX_NODES.with(|n| n.borrow().get(&(syntaxes.as_ptr() as usize)).copied())?;
    NODE_KEYS.with(|k| k.get(&node).cloned())
}

/// true if a type or a function syntax is defined
pub fn has_syntax(key: &str) -> bool {
    if let Some(defined) = DEFINED_SYNTAXES.with(|d| d.borrow().get(key).copied()) {
        return defined;
    }
    let defined = lookup_node(ValidationSyntaxGroupEnum::Syntaxes, key).is_some();
    DEFINED_SYNTAXES.with(|d| d.borrow_mut().insert(key.to_string(), defined));
    defined
}

/// the raw syntax of a type or a function
pub fn get_syntax(key: &str) -> Option<&'static str> {
    lookup_node(ValidationSyntaxGroupEnum::Syntaxes, key).and_then(|n| n["syntax"].as_str())
}

/// `^(@?)(-[a-zA-Z]+)-(.*?)$`: the name without its vendor prefix
fn strip_vendor_prefix(key: &str) -> Option<String> {
    let (at, rest) = match key.strip_prefix('@') {
        Some(rest) => ("@", rest),
        None => ("", key),
    };
    let rest = rest.strip_prefix('-')?;
    let letters = rest.bytes().take_while(|b| b.is_ascii_alphabetic()).count();
    if letters == 0 {
        return None;
    }
    let tail = rest[letters..].strip_prefix('-')?;
    Some(format!("{}{}", at, tail))
}

fn find_node(group: ValidationSyntaxGroupEnum, key: &str) -> Option<&'static serde_json::Value> {
    if let Some(node) = lookup_node(group, key) {
        return Some(node);
    }
    let mut key = key.to_string();
    if key.starts_with('@') || key.starts_with('-') {
        if let Some(stripped) = strip_vendor_prefix(&key) {
            key = stripped;
        }
    }
    lookup_node(group, &key)
}

/// parsed syntax of a definition, shared by the names that share the definition
pub fn get_parsed_syntax(group: ValidationSyntaxGroupEnum, key: &str) -> Option<Rc<Vec<Rc<ValidationToken>>>> {
    if let Some(cached) = PARSED_BY_KEY.with(|c| c.borrow()[group as usize].get(key).cloned()) {
        return cached;
    }
    let result = find_node(group, key).map(|node| {
        let id = node as *const serde_json::Value as usize;
        if let Some(parsed) = PARSED_SYNTAXES.with(|p| p.borrow().get(&id).cloned()) {
            return parsed;
        }
        let mut tokens = parse_syntax(node["syntax"].as_str().unwrap_or(""));
        trim_syntax_array(&mut tokens);
        let parsed = Rc::new(tokens);
        PARSED_SYNTAXES.with(|p| p.borrow_mut().insert(id, parsed.clone()));
        SYNTAX_NODES.with(|n| n.borrow_mut().insert(parsed.as_ptr() as usize, id));
        parsed
    });
    PARSED_BY_KEY.with(|c| c.borrow_mut()[group as usize].insert(Box::from(key), result.clone()));
    result
}

/// syntax rule of an at-rule: its prelude, its block, its descriptors
pub fn get_syntax_rule(group: ValidationSyntaxGroupEnum, key: &str) -> Option<Rc<ValidationSyntaxRule>> {
    if let Some(cached) = SYNTAX_RULES.with(|c| c.borrow()[group as usize].get(key).cloned()) {
        return cached;
    }
    let result = compute_syntax_rule(group, key).map(Rc::new);
    SYNTAX_RULES.with(|c| c.borrow_mut()[group as usize].insert(Box::from(key), result.clone()));
    result
}

fn compute_syntax_rule(group: ValidationSyntaxGroupEnum, key: &str) -> Option<ValidationSyntaxRule> {
    let node = find_node(group, key)?;
    let syntax_rules = get_parsed_syntax(group, key)?;
    let mut block_start: Option<usize> = None;
    let mut block_end: Option<usize> = None;
    for (i, t) in syntax_rules.iter().enumerate() {
        if t.typ == V::OpenCurlyBrace || t.typ == V::SemiColon {
            block_start = Some(i);
            break;
        }
    }
    if block_start.is_some() {
        for i in (0..syntax_rules.len()).rev() {
            if syntax_rules[i].typ == V::CloseCurlyBrace {
                block_end = Some(i);
                break;
            }
        }
    }
    // `slice(blockStart + 1, blockEnd)`: blockEnd -1 slices to the last but one token
    let mut block: Option<Vec<Rc<ValidationToken>>> = block_start.map(|start| {
        let end = match block_end {
            Some(end) => end,
            None => syntax_rules.len().saturating_sub(1),
        };
        let mut b = if start + 1 <= end { syntax_rules[start + 1..end].to_vec() } else { Vec::new() };
        trim_syntax_array(&mut b);
        b
    });
    let mut prelude: Vec<Rc<ValidationToken>> = match block_start {
        None => syntax_rules.to_vec(),
        Some(start) => syntax_rules[..start].to_vec(),
    };
    trim_syntax_array(&mut prelude);
    let property_descriptors = node.get("descriptors").and_then(|d| d.as_object()).map(|descriptors| {
        let mut map = HashMap::default();
        for (key, value) in descriptors {
            let syntax = value.as_str().or_else(|| value["syntax"].as_str()).unwrap_or("");
            map.insert(key.clone(), Rc::new(parse_syntax(syntax)));
        }
        Rc::new(map)
    });
    let mut optional_body = false;
    if block.as_ref().is_none_or(|b| b.is_empty()) && !prelude.is_empty() {
        let mut k = prelude.len() as isize - 1;
        if prelude[k as usize].typ == V::SemiColon {
            k -= 1;
        }
        if k >= 0 && prelude[k as usize].typ == V::Bracket {
            let chi = prelude[k as usize].children();
            if chi.first().is_some_and(|c| c.typ == V::OpenCurlyBrace) && chi.last().is_some_and(|c| c.typ == V::CloseCurlyBrace) {
                block = Some(chi[1..chi.len() - 1].to_vec());
                optional_body = prelude[k as usize].is_optional();
            }
        }
    }
    let syntax = node["syntax"].as_str().unwrap_or("");
    Some(ValidationSyntaxRule {
        optional_body,
        accept_any_declaration: syntax.contains("<declaration-list>"),
        accept_any_rule: syntax.contains("<group-rule-body>") || syntax.contains("<stylesheet>"),
        prelude: Rc::new(prelude),
        block: block.map(Rc::new),
        property_descriptors,
    })
}
