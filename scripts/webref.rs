//! Port of `scripts/webref.ts` (`npm run syntax-update`): the syntax file of the validation (`crates/css-parser/src/data/validation.json`)
//! generated from the W3C webref css definitions, the csstree patch, mdn-data and the local patch
//! (`scripts/local-patch.json`).
//!
//! `npm run syntax-update` (`cargo run --release --manifest-path scripts/Cargo.toml --bin webref`)
//!
//! Environment: `CSS_DATA_OUTPUT_DIR` (default `crates/css-parser/src/data`), `CSS_SYNTAX_CHANGES_FILE` (default `SYNTAX-CHANGELOG.md`).
//!
//! The javascript semantics that shape the output are kept: the insertion order of the object keys, the sort of the
//! keys with `localeCompare()` (see [`locale_compare`]) or by utf-16 code units (`Array.prototype.sort()`), the
//! `JSON.stringify()` formatting and the string concatenation of missing values (`undefined`).

use regex::{NoExpand, Regex};
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;

// syntax definitions of the W3C Webref repository: the consolidated css definitions of the @webref/css package
// (https://github.com/w3c/webref/tree/main/packages/css), pulled from the remote url. the definitions of the levels of a
// spec and their extensions are merged (the latest level is used). a new version of the package may introduce breaking
// changes: the version is updated manually
const WEBREF_URL: &str = "https://cdn.jsdelivr.net/npm/@webref/css@8.7.8/css.json";
// the csstree patch, written for mdn-data. the version is pinned: the generated syntax file must not change when a new
// version is published, it is updated manually
const PATCH_URL: &str = "https://cdn.jsdelivr.net/npm/css-tree@3.2.1/data/patch.json";
const LOCAL_PATCH_URL: &str = "scripts/local-patch.json";
const LANGUAGES_URL: &str = "https://cdn.jsdelivr.net/npm/@iso-639/data/1/en.json";
// the legacy data source (mdn-data, deprecated), used to find the origin of the definitions of the syntax files
// generated before the migration to webref, the legacy syntax of the vendor prefixed aliases and the definitions of the
// non standard properties. the version is pinned
const LEGACY_URL: &str = "https://cdn.jsdelivr.net/npm/mdn-data@2.37.2/css/";
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:180.0) Gecko/20100101 Firefox/151.0";

// the order of the origins in the summary
const ORIGIN_LABELS: [&str; 6] = ["legacy mdn-data", "mdn-data patch", "local-patch.json", "removed from webref", "iso-639", "unknown origin"];

/// groups of definitions of the syntax file
const GROUPS: [&str; 8] = ["declarations", "types", "selectors", "atRules", "descriptors", "mediaFeatures", "units", "languages"];

/// url of the file that provides each definition, by group and name. the types and the functions are named `<color>`
/// and `abs()`, the descriptors `@font-face/src`
#[derive(Clone, Default)]
struct Origins(HashMap<&'static str, HashMap<String, String>>);

impl Origins {
    fn new() -> Self {
        Origins(GROUPS.iter().map(|g| (*g, HashMap::new())).collect())
    }

    fn group(&mut self, group: &str) -> &mut HashMap<String, String> {
        let key = GROUPS.iter().find(|g| **g == group).copied().expect("origin group");
        self.0.get_mut(key).unwrap()
    }

    fn get(&self, group: &str, name: &str) -> Option<&String> {
        self.0.get(group).and_then(|g| g.get(name))
    }
}

/// scope of a definition: the definition it is nested in. the types are named without their angle brackets
#[derive(Clone, Debug)]
struct Scope {
    group: String,
    name: String,
    at_rule: Option<String>,
}

struct ScopedDefinition {
    // `<name>` for a type, `name()` for a function
    name: String,
    syntax: String,
    scope: Scope,
    // the url of the file that provides it
    url: String,
}

/// changes of a group of definitions
#[derive(Default)]
struct GroupChanges {
    added: Vec<String>,
    removed: Vec<String>,
    changed: Vec<(String, String, String)>,
}

type Result<T> = std::result::Result<T, String>;

// ---- javascript semantics ------------------------------------------------------------------------------------------

/// `String.prototype.localeCompare()` of the ICU root collation for the definition names: the characters are compared
/// by their primary weight (punctuation < symbols < digits < letters, the letters ignoring the case), then by case
/// (lowercase first)
fn locale_compare(a: &str, b: &str) -> Ordering {
    const ORDER: &str = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$0123456789abcdefghijklmnopqrstuvwxyz";
    let primary = |c: char| -> u32 {
        let lower = c.to_ascii_lowercase();
        match ORDER.find(lower) {
            Some(i) if lower.is_ascii() => i as u32,
            _ => 0x100 + c as u32,
        }
    };
    let pa: Vec<u32> = a.chars().map(primary).collect();
    let pb: Vec<u32> = b.chars().map(primary).collect();
    pa.cmp(&pb).then_with(|| {
        // tertiary: lowercase before uppercase
        let ta: Vec<bool> = a.chars().map(|c| c.is_ascii_uppercase()).collect();
        let tb: Vec<bool> = b.chars().map(|c| c.is_ascii_uppercase()).collect();
        ta.cmp(&tb)
    })
}

/// `Array.prototype.sort()` order: utf-16 code units
fn code_unit_compare(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// a value converted to a string by a concatenation (`undefined`)
fn concat_str(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) => "null".to_string(),
        Some(other) => other.to_string(),
    }
}

/// a property that is neither `null` nor `undefined`
fn present<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).filter(|v| !v.is_null())
}

fn present_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    present(value, key).and_then(|v| v.as_str())
}

fn entries(value: Option<&Value>) -> Vec<(String, Value)> {
    match value {
        Some(Value::Object(map)) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => Vec::new(),
    }
}

fn keys(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Object(map)) => map.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

/// `{ syntax: value.syntax }`: an undefined syntax is not serialized
fn syntax_object(syntax: Option<&Value>) -> Value {
    let mut map = Map::new();
    if let Some(syntax) = syntax {
        map.insert("syntax".into(), syntax.clone());
    }
    Value::Object(map)
}

fn syntax_value(syntax: &str) -> Value {
    syntax_object(Some(&Value::String(syntax.to_string())))
}

/// `JSON.stringify(value, null, 1)`
fn stringify_pretty(value: &Value) -> String {
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    serde::Serialize::serialize(value, &mut serializer).expect("json");
    String::from_utf8(out).expect("utf-8")
}

/// `new Date().toISOString()`
fn iso_date() -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let ms = now.as_millis() as i64;
    let (days, rem) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    // civil date from the days since the epoch
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as i64;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year,
        month,
        day,
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000
    )
}

/// fetch a json file
fn fetch_json(url: &str) -> Result<Value> {
    let response = match ureq::get(url).set("User-Agent", USER_AGENT).call() {
        Ok(response) => response,
        Err(ureq::Error::Status(status, response)) => return Err(format!("{}: {} {}", url, status, response.status_text())),
        Err(error) => return Err(format!("{}: {}", url, error)),
    };
    let mut body = String::new();
    response.into_reader().read_to_string(&mut body).map_err(|e| format!("{}: {}", url, e))?;
    serde_json::from_str(&body).map_err(|e| format!("{}: {}", url, e))
}

// ---- generator -----------------------------------------------------------------------------------------------------

struct Generator {
    json: Value,
    origins: Origins,
    patches: Value,
    local_patch: Value,
    mdn_properties: Value,
    // the mdn-data files, by file name, see fetch_mdn()
    mdn_files: RefCell<HashMap<String, Result<Value>>>,
    // functions and types scoped to another definition (e.g. <filter-function> -> blur(), clip -> rect())
    scoped_definitions: Vec<ScopedDefinition>,
    // renamed definitions, by origin group: old name -> new name. they are not added to the syntax file
    renamed: HashMap<String, Map<String, Value>>,
    // the vendor prefixed legacy name aliases of webref: alias -> standard property
    aliases: Map<String, Value>,
    // the names of the webref types, with or without a syntax
    webref_types: HashSet<String>,
    // the renamed properties the parser does not rename, see check_renamed_properties()
    renamed_missing: HashSet<String>,
    // the origins of the definitions of the legacy syntax files, see legacy_origins()
    legacy: Option<Origins>,
}

impl Generator {
    /// fetch a mdn-data file, once
    fn fetch_mdn(&self, file: &str) -> Result<Value> {
        if let Some(result) = self.mdn_files.borrow().get(file) {
            return result.clone();
        }
        let result = fetch_json(&format!("{}{}", LEGACY_URL, file));
        self.mdn_files.borrow_mut().insert(file.to_string(), result.clone());
        result
    }

    fn group(&mut self, group: &str) -> &mut Map<String, Value> {
        self.json.get_mut(group).and_then(|v| v.as_object_mut()).expect("json group")
    }

    fn group_ref(&self, group: &str) -> &Map<String, Value> {
        self.json.get(group).and_then(|v| v.as_object()).expect("json group")
    }

    /// `json[group][name]?.syntax`
    fn syntax_of(&self, group: &str, name: &str) -> Option<&Value> {
        self.group_ref(group).get(name).and_then(|d| present(d, "syntax"))
    }

    /// add or replace a webref definition
    fn define(&mut self, group: &str, name: &str, url: &str, add: impl FnOnce(&mut Self)) {
        self.origins.group(group).insert(origin_name(group, name), url.to_string());
        add(self);
    }

    fn skipped(&self, name: &str) -> bool {
        self.local_patch.get("skipped").and_then(|s| s.as_array()).is_some_and(|list| list.iter().filter_map(|t| t.as_str()).any(|t| name.starts_with(t)))
    }

    /// the parser renames the properties listed in renamed_standard_properties (src/syntax/syntax.rs), which is updated
    /// manually. report the renamed properties it does not rename or renames to another property, and the vendor
    /// prefixed properties it renames: the vendor prefixed aliases are not renamed
    fn check_renamed_properties(&mut self) {
        let file = repository_root().join("crates/css-parser/src/syntax/syntax.rs");
        let source = std::fs::read_to_string(file).unwrap_or_default();
        let body = source
            .find("fn renamed_standard_properties")
            .map(|start| {
                let rest = &source[start..];
                &rest[..rest.find("\n}\n").unwrap_or(rest.len())]
            })
            .unwrap_or("");
        let pair = Regex::new(r#""([^"]+)"\s*=>\s*Some\(\s*"([^"]+)"\s*\)"#).unwrap();
        let known: Vec<(String, String)> = pair.captures_iter(body).map(|c| (c[1].to_string(), c[2].to_string())).collect();
        let renamed = self.renamed.get("declarations").cloned().unwrap_or_default();

        for (name, target) in &renamed {
            let target = target.as_str().unwrap_or("");
            match known.iter().find(|(k, _)| k == name) {
                None => {
                    eprintln!(">> renamed property missing from renamedStandardProperties >> {} >> {}", name, target);
                    self.renamed_missing.insert(name.clone());
                }
                Some((_, known_target)) if known_target != target => {
                    eprintln!(">> renamed property with another target in renamedStandardProperties >> {} >> {} >> {}", name, target, known_target);
                }
                _ => {}
            }
        }

        let prefixed = Regex::new(r"^-[a-z]+-").unwrap();
        for (name, target) in &known {
            if prefixed.is_match(name) {
                eprintln!(">> vendor prefixed property renamed by renamedStandardProperties >> {} >> {}", name, target);
            }
        }
    }

    /// syntax of the definition a scoped definition is nested in
    fn get_scope_syntax(&self, scope: &Scope) -> Option<String> {
        if scope.group == "descriptors" {
            let at_rule = self.group_ref("atRules").get(scope.at_rule.as_deref().unwrap_or(""))?;
            return present(at_rule.get("descriptors")?, &scope.name).map(|v| v.as_str().map_or_else(|| "[object Object]".to_string(), |s| s.to_string()));
        }
        self.syntax_of(&scope.group, &scope.name).map(|v| concat_str(Some(v)))
    }

    /// update the syntax of the definition a scoped definition is nested in
    fn set_scope_syntax(&mut self, scope: &Scope, syntax: String) {
        if scope.group == "descriptors" {
            let at_rule = scope.at_rule.clone().unwrap_or_default();
            self.group("atRules")[&at_rule]["descriptors"][&scope.name] = Value::String(syntax);
        } else {
            let group = scope.group.clone();
            self.group(&group)[&scope.name]["syntax"] = Value::String(syntax);
        }
    }

    /// true if the definition the scoped definition is nested in references it
    fn can_inline(&self, definition: &ScopedDefinition) -> bool {
        self.get_scope_syntax(&definition.scope).is_some_and(|syntax| reference_pattern(&definition.name).is_match(&syntax))
    }

    /// replace the reference to a scoped definition by its syntax in the definition it is nested in:
    /// `clip: <rect()> | auto` becomes `clip: rect( <top>, <right>, <bottom>, <left> ) | auto`
    fn inline(&mut self, definition: &ScopedDefinition) -> bool {
        if !self.can_inline(definition) {
            return false;
        }
        let syntax = if !definition.name.starts_with('<') && is_single_function(&definition.name, &definition.syntax) {
            definition.syntax.trim().to_string()
        } else {
            format!("[ {} ]", definition.syntax.trim())
        };
        let current = self.get_scope_syntax(&definition.scope).unwrap_or_default();
        let replaced = reference_pattern(&definition.name).replace_all(&current, NoExpand(&syntax)).into_owned();
        self.set_scope_syntax(&definition.scope, replaced);
        true
    }

    /// a function or a type may be defined differently for different scopes (rect() for clip and rect() for
    /// <basic-shape>). the validation syntaxes have a single definition per name: the unscoped definition, or the
    /// first scoped definition that is not referenced by its scope. the other definitions are inlined in the syntax of
    /// their scope, as a function or a bracketed group
    fn resolve_scoped_definitions(&mut self) {
        let scoped = std::mem::take(&mut self.scoped_definitions);
        let mut by_name: Vec<(String, Vec<&ScopedDefinition>)> = Vec::new();

        for definition in &scoped {
            // nested in itself
            let scope_name = if definition.scope.group == "types" { format!("<{}>", definition.scope.name) } else { definition.scope.name.clone() };
            if scope_name == definition.name {
                continue;
            }
            match by_name.iter_mut().find(|(n, _)| *n == definition.name) {
                Some((_, list)) => list.push(definition),
                None => by_name.push((definition.name.clone(), vec![definition])),
            }
        }

        for (name, definitions) in &by_name {
            let group = if name.starts_with('<') { "types" } else { "syntaxes" };
            let key = if name.starts_with('<') { name[1..name.len() - 1].to_string() } else { name.clone() };
            let mut global = self.syntax_of(group, &key).map(|v| concat_str(Some(v)));

            if global.is_none() {
                let definition = definitions.iter().find(|d| !self.can_inline(d)).unwrap_or(&definitions[0]);
                global = Some(definition.syntax.clone());
                self.group(group).insert(key.clone(), syntax_value(&definition.syntax));
                self.origins.group("types").insert(origin_name("types", name), definition.url.clone());
            }

            for definition in definitions {
                if Some(&definition.syntax) == global.as_ref() {
                    continue;
                }
                // the scope already contains the syntax: `width: ... | fit-content(<length-percentage [0,∞]>)`
                if self.get_scope_syntax(&definition.scope).is_some_and(|s| s.contains(&definition.syntax)) {
                    continue;
                }
                if self.inline(definition) {
                    eprintln!(">> inlining scoped definition >> {} >> {} {}", name, definition.scope.group, definition.scope.name);
                } else {
                    eprintln!(">> ignoring scoped definition >> {} >> {} {} >> {}", name, definition.scope.group, definition.scope.name, definition.syntax);
                }
            }
        }
    }

    /// find a type syntax: the patches use the type names without the angle brackets. the group that defines it
    fn find_type_syntax(&self, key: &str) -> Option<&'static str> {
        if self.group_ref("syntaxes").get(key).is_some_and(|v| !v.is_null()) {
            Some("syntaxes")
        } else if self.group_ref("types").get(key).is_some_and(|v| !v.is_null()) {
            Some("types")
        } else {
            None
        }
    }

    /// apply a patch. the patch is the origin of the definitions it adds or replaces
    fn apply_patches(&mut self, replace: bool, url: &str, patch: &Value) {
        for (key, value) in entries(patch.get("atrules")) {
            let key = if key.starts_with('@') { key } else { format!("@{}", key) };

            if self.skipped(&key[1..]) {
                eprintln!(">> skipping at-rule >> {}", key);
                continue;
            }

            let existing = self.group_ref("atRules").get(&key).cloned();
            if replace || existing.is_none() || existing.as_ref().is_some_and(|e| present(e, "syntax").is_none()) {
                eprintln!(">> adding at-rule >> {}", key);
                let mut definition = syntax_object(value.get("syntax"));
                self.origins.group("atRules").insert(key.clone(), url.to_string());

                if let Some(descriptors) = value.get("descriptors") {
                    definition["descriptors"] = descriptors.clone();
                    for descriptor in keys(Some(descriptors)) {
                        self.origins.group("descriptors").insert(format!("{}/{}", key, descriptor), url.to_string());
                    }
                }

                self.group("atRules").insert(key, definition);
            }
        }

        for (key, value) in entries(patch.get("properties")) {
            if self.skipped(&key) {
                eprintln!(">> skipping property >> {}", key);
                continue;
            }

            // the vendor prefixed aliases are not replaced: the patch provides their legacy syntax, which extends the
            // syntax of the standard property, see apply_legacy_syntaxes()
            if self.aliases.contains_key(&key) {
                continue;
            }

            if !self.group_ref("declarations").contains_key(&key) {
                eprintln!(">> adding declarations >> {}", key);
                self.group("declarations").insert(key.clone(), syntax_object(value.get("syntax")));
                self.origins.group("declarations").insert(key, url.to_string());
            } else if is_extension(&value) {
                eprintln!(">> extending declaration syntax >> {}", key);
                let declaration = self.group("declarations").get_mut(&key).unwrap();
                let syntax = format!("{} {}", concat_str(declaration.get("syntax")), concat_str(value.get("syntax")));
                declaration["syntax"] = Value::String(syntax);
            } else if replace {
                eprintln!(">> replacing declaration syntax >> {}", key);
                let syntax = format!(" {}", concat_str(value.get("syntax")));
                self.group("declarations").insert(key.clone(), syntax_value(&syntax));
                self.origins.group("declarations").insert(key, url.to_string());
            } else {
                eprintln!(">> keeping webref declaration syntax >> {}", key);
            }
        }

        for (key, value) in entries(patch.get("types")) {
            if self.skipped(&key) {
                eprintln!(">> skipping type >> {}", key);
                continue;
            }

            // webref types are stored without their angle brackets in json.types
            match self.find_type_syntax(&key) {
                None => {
                    eprintln!(">> adding type syntax >> {}", key);
                    self.group("syntaxes").insert(key.clone(), syntax_object(value.get("syntax")));
                    self.origins.group("types").insert(origin_name("types", &key), url.to_string());
                }
                Some(group) if is_extension(&value) => {
                    eprintln!(">> extending type syntax >> {} >> {}", key, value);
                    let definition = self.group(group).get_mut(&key).unwrap();
                    let syntax = format!("{} {}", concat_str(definition.get("syntax")), concat_str(value.get("syntax")));
                    definition["syntax"] = Value::String(syntax);
                }
                Some(group) if replace => {
                    eprintln!(">> replacing type syntax >> {} >> {}", key, value);
                    let definition = self.group(group).get_mut(&key).unwrap();
                    match value.get("syntax") {
                        Some(syntax) => definition["syntax"] = syntax.clone(),
                        None => {
                            definition.as_object_mut().map(|o| o.remove("syntax"));
                        }
                    }
                    self.origins.group("types").insert(origin_name("types", &key), url.to_string());
                }
                Some(_) => eprintln!(">> keeping webref type syntax >> {}", key),
            }
        }

        if let Some(selectors) = patch.get("selectors") {
            for (key, value) in entries(Some(selectors)) {
                // webref does not provide a syntax for some selectors, or provides a prose one: `:nth-child(An+B [of S]?)`
                if replace || !self.group_ref("selectors").contains_key(&key) {
                    eprintln!(">> adding selector syntax >> {}", key);
                    self.group("selectors").insert(key.clone(), syntax_object(value.get("syntax")));
                    self.origins.group("selectors").insert(key, url.to_string());
                }
            }
        }

        if let Some(media_features) = patch.get("mediaFeatures") {
            for (_, config) in entries(Some(media_features)) {
                for (category, features) in entries(Some(&config)) {
                    for (name, mut feature) in entries(Some(&features)) {
                        // Object.assign(feature, { category })
                        if let Value::Object(map) = &mut feature {
                            map.insert("category".into(), Value::String(category.clone()));
                        }
                        self.group("mediaFeatures").insert(name.clone(), feature);
                        self.origins.group("mediaFeatures").insert(name, url.to_string());
                    }
                }
            }
        }
    }

    /// add the mdn-data definitions of the properties and of the selectors that webref does not define: the non
    /// standard properties (`box-orient`, `font-smooth`, `-webkit-tap-highlight-color`) and the non standard pseudo
    /// elements (`::-webkit-slider-thumb`). the renamed properties and the vendor prefixed aliases are not added. the
    /// types referenced by the added definitions that are not defined are added from mdn-data. they are added before the
    /// patches: the csstree patch and the local patch extend or replace them
    fn add_legacy_definitions(&mut self) -> Result<()> {
        let selectors = self.fetch_mdn("selectors.json")?;
        let syntaxes = self.fetch_mdn("syntaxes.json")?;
        let mut added: Vec<String> = Vec::new();
        let renamed = self.renamed.get("declarations").cloned().unwrap_or_default();

        for (name, property) in entries(Some(&self.mdn_properties.clone())) {
            let Some(syntax) = present(&property, "syntax") else {
                continue;
            };
            if name.starts_with("--") || self.group_ref("declarations").contains_key(&name) || self.aliases.contains_key(&name) || renamed.contains_key(&name) {
                continue;
            }
            eprintln!(">> adding mdn-data declaration >> {}", name);
            self.group("declarations").insert(name.clone(), syntax_object(Some(syntax)));
            self.origins.group("declarations").insert(name, format!("{}properties.json", LEGACY_URL));
            added.push(concat_str(Some(syntax)));
        }

        for (name, selector) in entries(Some(&selectors)) {
            // the other entries describe the selectors (`Type selectors`)
            let Some(syntax) = present(&selector, "syntax") else {
                continue;
            };
            if !name.starts_with(':') || self.group_ref("selectors").contains_key(&name) {
                continue;
            }
            eprintln!(">> adding mdn-data selector >> {}", name);
            self.group("selectors").insert(name.clone(), syntax_object(Some(syntax)));
            self.origins.group("selectors").insert(name, format!("{}selectors.json", LEGACY_URL));
        }

        // the types referenced by the added definitions, and by the types they reference
        while let Some(syntax) = added.pop() {
            for reference in self.undefined_references(&syntax) {
                let name = &reference[1..reference.len() - 1];
                let Some(syntax) = syntaxes.get(name).and_then(|s| present(s, "syntax")) else {
                    continue;
                };
                if name.starts_with('\'') {
                    continue;
                }
                eprintln!(">> adding mdn-data type syntax >> {}", name);
                self.group("syntaxes").insert(name.to_string(), syntax_object(Some(syntax)));
                self.origins.group("types").insert(origin_name("types", name), format!("{}syntaxes.json", LEGACY_URL));
                added.push(concat_str(Some(syntax)));
            }
        }

        Ok(())
    }

    /// the references of a syntax that are not defined: `<'mask-clip'>`, `<composite-style>`, `<rect()>`. the webref
    /// types without a syntax (`<length>`, `<string>`) and the tokens (`<ident-token>`) are defined
    fn undefined_references(&self, syntax: &str) -> Vec<String> {
        let mut references: Vec<String> = Vec::new();
        let mut add = |r: String| {
            if !references.contains(&r) {
                references.push(r);
            }
        };

        for c in Regex::new(r"<'([^']+)'>").unwrap().captures_iter(syntax) {
            if self.syntax_of("declarations", &c[1]).is_none() {
                add(c[0].to_string());
            }
        }

        for c in Regex::new(r"<([a-zA-Z0-9-]+(?:\(\))?)(?:\s*\[[^\]]*\])?>").unwrap().captures_iter(syntax) {
            let name = &c[1];
            if !name.ends_with("-token") && !self.webref_types.contains(name) && self.find_type_syntax(name).is_none() {
                add(format!("<{}>", name));
            }
        }

        references
    }

    /// the vendor prefixed legacy name aliases of webref are not renamed: they use the syntax of the standard property.
    /// when mdn-data, the csstree patch or the local patch provide a legacy syntax, the alias gets its own syntax: the
    /// syntax of the standard property extended with the legacy syntax.
    ///
    /// `-webkit-mask-clip: <'mask-clip'> | [ <coord-box> | no-clip | border | padding | content | text ]#`
    ///
    /// the sources are applied in the order of the mdn-data pipeline: mdn-data, then the csstree patch, then the local
    /// patch. a patch replaces the legacy syntax of the previous sources, or extends it when its syntax starts with `|`
    fn apply_legacy_syntaxes(&mut self) {
        let mdn_properties_url = format!("{}properties.json", LEGACY_URL);
        let aliases = self.aliases.clone();
        let sources = [(PATCH_URL.to_string(), self.patches.clone()), (LOCAL_PATCH_URL.to_string(), self.local_patch.clone())];

        for (name, standard) in &aliases {
            let standard = standard.as_str().unwrap_or("");
            if self.group_ref("declarations").get(name).and_then(|d| present(d, "alias")).is_none() {
                continue;
            }

            let standard_syntaxes: Vec<Option<String>> = vec![
                self.syntax_of("declarations", standard).map(|v| concat_str(Some(v))),
                self.mdn_properties.get(standard).and_then(|p| present(p, "syntax")).map(|v| concat_str(Some(v))),
            ];

            // the legacy syntax, replaced by the next sources
            let mut legacy: Option<String> = None;
            // the extensions of the legacy syntax
            let mut extensions: Vec<String> = Vec::new();
            // the sources of the legacy syntax and of its extensions
            let mut urls: Vec<String> = Vec::new();

            let mdn_syntax = self.mdn_properties.get(name).and_then(|p| present(p, "syntax")).map(|v| concat_str(Some(v)));
            if is_legacy_syntax(mdn_syntax.as_deref(), &standard_syntaxes) {
                legacy = mdn_syntax;
                urls = vec![mdn_properties_url.clone()];
            }

            for (url, patch) in &sources {
                let Some(value) = patch.get("properties").and_then(|p| p.get(name)) else {
                    continue;
                };
                let Some(syntax) = present_str(value, "syntax") else {
                    continue;
                };

                if self.skipped(name) {
                    eprintln!(">> skipping property >> {}", name);
                    continue;
                }

                if is_extension(value) {
                    // `| <legacy>`: an alternative of the syntax
                    extensions.push(Regex::new(r"^\|\s*").unwrap().replace(syntax.trim(), "").into_owned());
                    urls.push(url.clone());
                } else if is_legacy_syntax(Some(syntax), &standard_syntaxes) {
                    legacy = Some(syntax.trim().to_string());
                    extensions.clear();
                    urls = vec![url.clone()];
                } else {
                    // the patch provides the syntax of the standard property: there is no legacy syntax
                    eprintln!(">> no legacy syntax for the vendor prefixed alias >> {} >> {}", name, origin_label(url));
                    legacy = None;
                    extensions.clear();
                    urls.clear();
                }
            }

            if legacy.is_none() && extensions.is_empty() {
                eprintln!(">> vendor prefixed alias >> {} >> {}", name, standard);
                continue;
            }

            let mut parts = vec![format!("<'{}'>", standard)];
            parts.extend(legacy.map(|l| l.trim().to_string()));
            parts.extend(extensions);
            let syntax = parts.join(" | ");

            let mut labels: Vec<&str> = Vec::new();
            for url in &urls {
                let label = origin_label(url);
                if !labels.contains(&label) {
                    labels.push(label);
                }
            }
            eprintln!(">> legacy syntax of a vendor prefixed alias >> {} >> {}", name, labels.join(", "));

            self.group("declarations").insert(name.clone(), syntax_value(&syntax));
            self.origins.group("declarations").insert(name.clone(), urls.last().cloned().unwrap_or_default());

            for reference in self.undefined_references(&syntax) {
                eprintln!(">> undefined reference in the legacy syntax of a vendor prefixed alias >> {} >> {}", name, reference);
            }
        }
    }

    /// the origins of the definitions of a syntax file generated before the migration to webref: mdn-data, then the
    /// csstree patch (mdn-data patch), then the local patch
    fn legacy_origins(&mut self) -> &Origins {
        if self.legacy.is_none() {
            let mut result = Origins::new();
            let add = |result: &mut Origins, group: &str, names: Vec<String>, url: &str| {
                for name in names {
                    result.group(group).entry(origin_name(group, &name)).or_insert_with(|| url.to_string());
                }
            };
            let mdn = |file: &str| -> Value {
                match self.fetch_mdn(file) {
                    Ok(Value::Null) => Value::Object(Map::new()),
                    Ok(value) => value,
                    Err(error) => {
                        eprintln!(">> legacy mdn-data not available >> {}", error);
                        Value::Object(Map::new())
                    }
                }
            };
            let at_rules = |result: &mut Origins, url: &str, definitions: &Value| {
                for (name, definition) in entries(Some(definitions)) {
                    let key = if name.starts_with('@') { name } else { format!("@{}", name) };
                    add(result, "atRules", vec![key.clone()], url);
                    add(result, "descriptors", keys(definition.get("descriptors")).into_iter().map(|d| format!("{}/{}", key, d)).collect(), url);
                }
            };

            add(&mut result, "declarations", keys(Some(&mdn("properties.json"))), &format!("{}properties.json", LEGACY_URL));
            add(&mut result, "types", keys(Some(&mdn("syntaxes.json"))), &format!("{}syntaxes.json", LEGACY_URL));
            add(&mut result, "types", keys(Some(&mdn("functions.json"))), &format!("{}functions.json", LEGACY_URL));
            add(&mut result, "selectors", keys(Some(&mdn("selectors.json"))), &format!("{}selectors.json", LEGACY_URL));
            at_rules(&mut result, &format!("{}at-rules.json", LEGACY_URL), &mdn("at-rules.json"));
            add(&mut result, "units", keys(Some(&mdn("units.json"))), &format!("{}units.json", LEGACY_URL));

            for (url, patch) in [(PATCH_URL, &self.patches), (LOCAL_PATCH_URL, &self.local_patch)] {
                add(&mut result, "declarations", keys(patch.get("properties")), url);
                add(&mut result, "types", keys(patch.get("types")), url);
                add(&mut result, "selectors", keys(patch.get("selectors")), url);
                at_rules(&mut result, url, patch.get("atrules").unwrap_or(&Value::Null));
                add(&mut result, "units", keys(patch.get("units")), url);

                for (_, config) in entries(patch.get("mediaFeatures")) {
                    for (_, features) in entries(Some(&config)) {
                        add(&mut result, "mediaFeatures", keys(Some(&features)), url);
                    }
                }
            }

            let languages: Vec<String> = self.json["languages"].as_array().map(|l| l.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
            add(&mut result, "languages", languages, LANGUAGES_URL);
            self.legacy = Some(result);
        }
        self.legacy.as_ref().unwrap()
    }

    /// the removed definitions of a group, grouped by origin: (label, url, names)
    fn group_by_origin(&mut self, group: &str, names: &[String], previous_origins: &Origins) -> Vec<(&'static str, String, Vec<String>)> {
        let mut by_url: Vec<(String, Vec<String>)> = Vec::new();

        for name in names {
            let url = match previous_origins.get(group, name).filter(|u| !u.is_empty()) {
                Some(url) => url.clone(),
                None => self.legacy_origins().get(group, name).filter(|u| !u.is_empty()).cloned().unwrap_or_default(),
            };
            match by_url.iter_mut().find(|(u, _)| *u == url) {
                Some((_, list)) => list.push(name.clone()),
                None => by_url.push((url, vec![name.clone()])),
            }
        }

        let mut result: Vec<(&'static str, String, Vec<String>)> = by_url.into_iter().map(|(url, names)| (origin_label(&url), url, names)).collect();
        let index = |label: &str| ORIGIN_LABELS.iter().position(|l| *l == label).map_or(-1, |i| i as i64);
        result.sort_by(|a, b| index(a.0).cmp(&index(b.0)).then_with(|| locale_compare(&a.1, &b.1)));
        result
    }

    /// markdown summary of the changes between the previous and the new syntax file
    fn summarize_changes(&mut self, previous: Option<&Value>, current: &Value, previous_origins: &Origins) -> String {
        let mut lines: Vec<String> = vec![format!("## Syntax changes - {}", iso_date()), String::new()];

        lines.push(format!("source: {}", WEBREF_URL));
        lines.push(String::new());

        let Some(previous) = previous else {
            lines.push(format!(
                "no previous syntax file: {} declarations, {} types and functions, {} selectors, {} at-rules, {} units",
                keys(current.get("declarations")).len(),
                value_syntaxes_of(current).len(),
                keys(current.get("selectors")).len(),
                keys(current.get("atRules")).len(),
                current.get("units").and_then(|u| u.as_array()).map_or(0, |u| u.len())
            ));
            return lines.join("\n");
        };

        let titles: [(&str, &str); 8] = [
            ("declarations", "declarations"),
            ("types", "types and functions"),
            ("selectors", "selectors"),
            ("atRules", "at-rules"),
            ("descriptors", "at-rule descriptors"),
            ("mediaFeatures", "media features"),
            ("units", "units"),
            ("languages", "languages"),
        ];

        let before = groups_of(previous);
        let after = groups_of(current);
        let groups: Vec<(&str, GroupChanges, bool)> = titles
            .iter()
            .map(|(group, _)| (*group, compare_definitions(&before[*group], &after[*group]), *group != "units" && *group != "languages"))
            .collect();
        let title = |group: &str| titles.iter().find(|(g, _)| *g == group).unwrap().1;
        let all_renamed = self.renamed.clone();
        let renamed_of = |group: &str| all_renamed.get(group).cloned().unwrap_or_default();

        let mut counts: Vec<String> = Vec::new();

        for (group, changes, _) in &groups {
            if changes.added.len() + changes.removed.len() + changes.changed.len() > 0 {
                let renamed = renamed_of(group);
                let renamed_count = changes.removed.iter().filter(|name| renamed.contains_key(*name)).count();
                counts.push(format!(
                    "- {}: {} added, {} removed{}, {} changed",
                    title(group),
                    changes.added.len(),
                    changes.removed.len(),
                    if renamed_count > 0 { format!(" ({} renamed)", renamed_count) } else { String::new() },
                    changes.changed.len()
                ));
            }
        }

        if counts.is_empty() {
            lines.push("no changes".into());
            return lines.join("\n");
        }

        lines.extend(counts);

        for (group, changes, show_syntax) in &groups {
            if changes.added.len() + changes.removed.len() + changes.changed.len() == 0 {
                continue;
            }

            lines.push(String::new());
            lines.push(format!("### {}", title(group)));

            if !changes.added.is_empty() {
                lines.push(String::new());
                lines.push(format!("added: {}", changes.added.iter().map(|n| code(n)).collect::<Vec<_>>().join(", ")));
            }

            if !changes.removed.is_empty() {
                // the removed definitions, grouped by origin. the renamed definitions are reported with their new name
                lines.push(String::new());
                lines.push("removed:".into());
                lines.push(String::new());
                let renamed = renamed_of(group);

                for (label, url, names) in self.group_by_origin(group, &changes.removed, previous_origins) {
                    let names: Vec<String> = names
                        .iter()
                        .map(|name| match renamed.get(name) {
                            Some(target) => format!(
                                "{} (renamed: {}{}",
                                code(name),
                                code(&concat_str(Some(target))),
                                if *group == "declarations" && self.renamed_missing.contains(name) { ", missing from renamedStandardProperties)" } else { ")" }
                            ),
                            None => code(name),
                        })
                        .collect();
                    lines.push(format!("- {}{}: {}", label, if url.is_empty() { String::new() } else { format!(" ({})", url) }, names.join(", ")));
                }
            }

            if !changes.changed.is_empty() {
                lines.push(String::new());
                lines.push("changed:".into());
                lines.push(String::new());

                for (name, before, after) in &changes.changed {
                    lines.push(format!("- {}", code(name)));
                    if *show_syntax {
                        lines.push(format!("  - old: {}", code(before)));
                        lines.push(format!("  - new: {}", code(after)));
                    }
                }
            }
        }

        lines.join("\n")
    }
}

/// scope of a scoped definition, from the `for` key of webref: `clip`, `<basic-shape>`, `attr()`, `@function`,
/// `@font-face/src`, `:nth-child()`
fn scope_of(name: &str) -> Scope {
    let scope = |group: &str, name: &str| Scope { group: group.into(), name: name.into(), at_rule: None };
    if name.starts_with('<') {
        return scope("types", &name[1..name.len() - 1]);
    }
    if name.starts_with('@') {
        return match name.find('/') {
            None => scope("atRules", name),
            Some(index) => Scope { group: "descriptors".into(), name: name[index + 1..].into(), at_rule: Some(name[..index].into()) },
        };
    }
    if name.starts_with(':') {
        return scope("selectors", name);
    }
    if name.ends_with("()") {
        return scope("syntaxes", name);
    }
    scope("declarations", name)
}

/// reference of a type or a function in a syntax: `<content-list>`, `<rect()>`
fn reference_pattern(name: &str) -> Regex {
    let reference = if name.starts_with('<') { name.to_string() } else { format!("<{}>", name) };
    Regex::new(&regex::escape(&reference)).unwrap()
}

/// true if the syntax is a single function: `rect( <top>, <right>, <bottom>, <left> )`
fn is_single_function(name: &str, syntax: &str) -> bool {
    let syntax = syntax.trim();
    if !syntax.starts_with(&name[..name.len() - 1]) || !syntax.ends_with(')') {
        return false;
    }
    let units: Vec<u16> = syntax.encode_utf16().collect();
    let mut depth = 0;
    for (i, &unit) in units.iter().enumerate().skip(name.encode_utf16().count() - 2) {
        if unit == b'(' as u16 {
            depth += 1;
        } else if unit == b')' as u16 {
            depth -= 1;
            if depth == 0 {
                return i == units.len() - 1;
            }
        }
    }
    false
}

/// true if a patch extends a definition instead of replacing it: its syntax starts with `|` or its comment starts with
/// `extend`
fn is_extension(value: &Value) -> bool {
    value.get("comment").and_then(|c| c.as_str()).is_some_and(|c| c.starts_with("extend")) || concat_str(value.get("syntax")).trim_start().starts_with('|')
}

/// syntax without the insignificant whitespace: the patches may add leading spaces
fn normalize_syntax(syntax: Option<&str>) -> String {
    syntax.unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// true if the syntax of a vendor prefixed alias is a legacy syntax: it differs from the syntax of the standard property
fn is_legacy_syntax(syntax: Option<&str>, standard_syntaxes: &[Option<String>]) -> bool {
    let normalized = normalize_syntax(syntax);
    !normalized.is_empty() && standard_syntaxes.iter().all(|s| s.as_deref().is_none_or(|s| normalize_syntax(Some(s)) != normalized))
}

/// name of a definition in its group: the types stored in json.syntaxes are named without their angle brackets
fn origin_name(group: &str, name: &str) -> String {
    if group == "types" && !name.ends_with("()") && !name.starts_with('<') {
        format!("<{}>", name)
    } else {
        name.to_string()
    }
}

/// description of an origin
fn origin_label(url: &str) -> &'static str {
    if url.is_empty() {
        return "unknown origin";
    }
    // the other versions of mdn-data and of the csstree patch: the origins saved by the previous versions of this script
    if url.starts_with(LEGACY_URL) || url.contains("/mdn-data@") || url.contains("/mdn/data/") {
        return "legacy mdn-data";
    }
    if url == PATCH_URL || url.contains("/css-tree@") || url.contains("/csstree/csstree/") {
        return "mdn-data patch";
    }
    if url == LOCAL_PATCH_URL {
        return "local-patch.json";
    }
    if url == LANGUAGES_URL {
        return "iso-639";
    }
    // the other origins are webref files
    "removed from webref"
}

/// the keys sorted with `localeCompare()`, `syntax` first, recursively (not the arrays)
fn sort_keys(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|(a, _), (b, _)| {
                if *a == "syntax" {
                    Ordering::Less
                } else if *b == "syntax" {
                    Ordering::Greater
                } else {
                    locale_compare(a, b)
                }
            });
            Value::Object(entries.into_iter().map(|(k, v)| (k.clone(), if v.is_object() { sort_keys(v) } else { v.clone() })).collect())
        }
        other => other.clone(),
    }
}

// ---- summary -------------------------------------------------------------------------------------------------------

/// compare the definitions of a group, by name
fn compare_definitions(previous: &[(String, String)], current: &[(String, String)]) -> GroupChanges {
    let mut changes = GroupChanges::default();
    let previous_map: HashMap<&str, &str> = previous.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let current_map: HashMap<&str, &str> = current.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    for (name, syntax) in current {
        match previous_map.get(name.as_str()) {
            None => changes.added.push(name.clone()),
            Some(before) if before != syntax => changes.changed.push((name.clone(), before.to_string(), syntax.clone())),
            _ => {}
        }
    }

    for (name, _) in previous {
        if !current_map.contains_key(name.as_str()) {
            changes.removed.push(name.clone());
        }
    }

    changes.added.sort_by(|a, b| code_unit_compare(a, b));
    changes.removed.sort_by(|a, b| code_unit_compare(a, b));
    changes.changed.sort_by(|a, b| locale_compare(&a.0, &b.0));
    changes
}

/// an ordered record: assigning an existing key keeps its position
fn set(record: &mut Vec<(String, String)>, key: String, value: String) {
    match record.iter_mut().find(|(k, _)| *k == key) {
        Some(entry) => entry.1 = value,
        None => record.push((key, value)),
    }
}

/// the syntaxes of a group, by name
fn syntaxes_of(group: Option<&Value>) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for (name, definition) in entries(group) {
        // an alias is reported with the name of the standard property
        let value = match present(&definition, "alias") {
            Some(alias) => format!("alias of {}", concat_str(Some(alias))),
            None => normalize_syntax(definition.get("syntax").and_then(|s| s.as_str())),
        };
        set(&mut result, name, value);
    }
    result
}

/// the types and the functions, by name: `<color>`, `abs()`
fn value_syntaxes_of(file: &Value) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for (name, definition) in entries(file.get("types")) {
        let key = if name.starts_with('<') { name } else { format!("<{}>", name) };
        set(&mut result, key, normalize_syntax(definition.get("syntax").and_then(|s| s.as_str())));
    }
    for (name, definition) in entries(file.get("syntaxes")) {
        let key = if name.ends_with("()") || name.starts_with('<') { name } else { format!("<{}>", name) };
        set(&mut result, key, normalize_syntax(definition.get("syntax").and_then(|s| s.as_str())));
    }
    result
}

/// the at-rule descriptors, by at-rule and descriptor name: `@font-face/src`
fn descriptors_of(file: &Value) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for (at_rule, definition) in entries(file.get("atRules")) {
        for (name, descriptor) in entries(definition.get("descriptors")) {
            let syntax = match &descriptor {
                Value::String(s) => Some(s.as_str()),
                other => other.get("syntax").and_then(|s| s.as_str()),
            };
            set(&mut result, format!("{}/{}", at_rule, name), normalize_syntax(syntax));
        }
    }
    result
}

/// the values of a list, by value: units, languages
fn list_of(values: Option<&Value>) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for value in values.and_then(|v| v.as_array()).into_iter().flatten() {
        let value = concat_str(Some(value));
        set(&mut result, value.clone(), value);
    }
    result
}

/// the media features, by name. the keys of the objects are sorted
fn media_features_of(file: &Value) -> Vec<(String, String)> {
    fn stringify(value: &Value) -> String {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort_by(|a, b| code_unit_compare(a, b));
                format!("{{{}}}", keys.iter().map(|k| format!("{}:{}", Value::String((*k).clone()), stringify(&map[*k]))).collect::<Vec<_>>().join(","))
            }
            other => other.to_string(),
        }
    }
    entries(file.get("mediaFeatures")).into_iter().map(|(name, feature)| (name, stringify(&feature))).collect()
}

/// markdown code span
fn code(text: &str) -> String {
    if text.contains('`') {
        format!("`` {} ``", text)
    } else {
        format!("`{}`", text)
    }
}

/// the definitions of a syntax file, by group and name
fn groups_of(file: &Value) -> HashMap<&'static str, Vec<(String, String)>> {
    HashMap::from([
        ("declarations", syntaxes_of(file.get("declarations"))),
        ("types", value_syntaxes_of(file)),
        ("selectors", syntaxes_of(file.get("selectors"))),
        ("atRules", syntaxes_of(file.get("atRules"))),
        ("descriptors", descriptors_of(file)),
        ("mediaFeatures", media_features_of(file)),
        ("units", list_of(file.get("units"))),
        ("languages", list_of(file.get("languages"))),
    ])
}

/// the origins file: the names of the definitions, by group and url
fn serialize_origins(origins: &Origins) -> String {
    let mut result = Map::new();
    for group in GROUPS {
        let mut by_url: Vec<(String, Vec<String>)> = Vec::new();
        let mut names: Vec<(&String, &String)> = origins.0[group].iter().collect();
        // insertion order of collect_origins(): the names sorted
        names.sort_by(|a, b| code_unit_compare(a.0, b.0));
        for (name, url) in names {
            match by_url.iter_mut().find(|(u, _)| u == url) {
                Some((_, list)) => list.push(name.clone()),
                None => by_url.push((url.clone(), vec![name.clone()])),
            }
        }
        by_url.sort_by(|a, b| locale_compare(&a.0, &b.0));
        let mut urls = Map::new();
        for (url, mut names) in by_url {
            names.sort_by(|a, b| code_unit_compare(a, b));
            urls.insert(url, Value::Array(names.into_iter().map(Value::String).collect()));
        }
        result.insert(group.to_string(), Value::Object(urls));
    }
    stringify_pretty(&Value::Object(result)) + "\n"
}

/// parse the origins file, see serialize_origins()
fn parse_origins(content: &Value) -> Origins {
    let mut origins = Origins::new();
    for (group, urls) in entries(Some(content)) {
        if GROUPS.contains(&group.as_str()) {
            for (url, names) in entries(Some(&urls)) {
                for name in names.as_array().into_iter().flatten() {
                    origins.group(&group).insert(concat_str(Some(name)), url.clone());
                }
            }
        }
    }
    origins
}

/// the origins of the definitions of the syntax file
fn collect_origins(file: &Value, origins: &Origins) -> Origins {
    let mut result = Origins::new();
    let groups = groups_of(file);
    for group in GROUPS {
        let mut names: Vec<&String> = groups[group].iter().map(|(k, _)| k).collect();
        names.sort_by(|a, b| code_unit_compare(a, b));
        for name in names {
            let url = origins
                .get(group, name)
                .cloned()
                // a descriptor is provided by its at-rule
                .or_else(|| if group == "descriptors" { origins.get("atRules", &name[..name.find('/').unwrap_or(0)]).cloned() } else { None })
                .unwrap_or_default();
            if url.is_empty() {
                eprintln!(">> unknown origin >> {} >> {}", group, name);
            }
            result.group(group).insert(name.clone(), url);
        }
    }
    result
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{}", error);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let root = repository_root();
    let local_patch: Value = serde_json::from_str(&std::fs::read_to_string(root.join(LOCAL_PATCH_URL)).map_err(|e| format!("{}: {}", LOCAL_PATCH_URL, e))?)
        .map_err(|e| format!("{}: {}", LOCAL_PATCH_URL, e))?;

    let languages = keys(Some(&fetch_json(LANGUAGES_URL)?));
    let json = serde_json::json!({
        "declarations": {},
        "syntaxes": {},
        "selectors": {},
        "atRules": {},
        "units": [],
        "languages": languages,
        "mediaFeatures": {},
        "types": {},
    });

    let mut origins = Origins::new();
    for language in &languages {
        origins.group("languages").insert(language.clone(), LANGUAGES_URL.to_string());
    }

    let patches = fetch_json(PATCH_URL)?;
    let webref = fetch_json(WEBREF_URL)?;

    let mut generator = Generator {
        json,
        origins,
        patches,
        local_patch,
        mdn_properties: Value::Null,
        mdn_files: RefCell::new(HashMap::new()),
        scoped_definitions: Vec::new(),
        renamed: HashMap::from([("declarations".to_string(), Map::new())]),
        aliases: Map::new(),
        webref_types: webref.get("types").and_then(|t| t.as_array()).into_iter().flatten().filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(String::from)).collect(),
        renamed_missing: HashSet::new(),
        legacy: None,
    };

    // the mdn-data properties provide the legacy syntax of some vendor prefixed aliases: `-webkit-mask-clip`
    generator.mdn_properties = generator.fetch_mdn("properties.json")?;

    let list = |key: &str| webref.get(key).and_then(|v| v.as_array()).cloned().unwrap_or_default();

    for at_rule in list("atrules") {
        let name = concat_str(at_rule.get("name"));
        // the scoped at-rules (`@swash` for `@font-feature-values`) are stored with the other at-rules
        let Some(syntax) = present(&at_rule, "syntax").cloned() else {
            eprintln!(">> skipping at-rule without syntax >> {}", name);
            continue;
        };

        generator.define("atRules", &name, WEBREF_URL, |g| {
            g.group("atRules").insert(name.clone(), syntax_object(Some(&syntax)));
        });

        // @media descriptors are the media features, they are validated using json.mediaFeatures
        if name == "@media" {
            continue;
        }

        for descriptor in at_rule.get("descriptors").and_then(|d| d.as_array()).into_iter().flatten() {
            let Some(syntax) = present(descriptor, "syntax").cloned() else {
                continue;
            };
            let descriptor_name = concat_str(descriptor.get("name"));
            generator.define("descriptors", &format!("{}/{}", name, descriptor_name), WEBREF_URL, |g| {
                let at_rule = g.group("atRules").get_mut(&name).unwrap().as_object_mut().unwrap();
                at_rule.entry("descriptors").or_insert_with(|| Value::Object(Map::new()))[&descriptor_name] = syntax;
            });
        }
    }

    let prefixed = Regex::new(r"^-[a-z]+-").unwrap();

    for property in list("properties") {
        let name = concat_str(property.get("name"));

        if let Some(alias_of) = present(&property, "legacyAliasOf").cloned() {
            // the vendor prefixed legacy name aliases use the syntax of the standard property, they are not renamed:
            // `-webkit-align-content: { alias: "align-content" }`
            if prefixed.is_match(&name) {
                generator.define("declarations", &name, WEBREF_URL, |g| {
                    let mut map = Map::new();
                    map.insert("alias".into(), alias_of.clone());
                    g.group("declarations").insert(name.clone(), Value::Object(map));
                });
                generator.aliases.insert(name, alias_of);
                continue;
            }

            // the other legacy name aliases are renamed properties (`word-wrap` is a legacy name alias of
            // `overflow-wrap`): they are not added, the parser renames them (renamed_standard_properties)
            eprintln!(">> renamed property >> {} >> {}", name, concat_str(Some(&alias_of)));
            generator.renamed.get_mut("declarations").unwrap().insert(name, alias_of);
            continue;
        }

        let Some(syntax) = present(&property, "syntax").cloned() else {
            eprintln!(">> skipping property without syntax >> {}", name);
            continue;
        };

        generator.define("declarations", &name, WEBREF_URL, |g| {
            g.group("declarations").insert(name.clone(), syntax_object(Some(&syntax)));
        });
    }

    // webref does not provide a syntax for some selectors, or provides a prose one, see the patches
    for selector in list("selectors") {
        if let Some(syntax) = present(&selector, "syntax").cloned() {
            let name = concat_str(selector.get("name"));
            generator.define("selectors", &name, WEBREF_URL, |g| {
                g.group("selectors").insert(name.clone(), syntax_object(Some(&syntax)));
            });
        }
    }

    // the functions (`abs()`) are stored in json.syntaxes, the types (`color` for `<color>`) in json.types, without
    // their angle brackets
    for (definitions, is_type) in [(list("functions"), false), (list("types"), true)] {
        for definition in definitions {
            let Some(syntax) = present(&definition, "syntax").cloned() else {
                continue;
            };
            let definition_name = concat_str(definition.get("name"));
            let name = if is_type { format!("<{}>", definition_name) } else { definition_name.clone() };
            let scopes: Vec<String> = match definition.get("for") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(items)) => items.iter().map(|s| concat_str(Some(s))).collect(),
                Some(other) => vec![concat_str(Some(other))],
            };

            if scopes.is_empty() {
                generator.define("types", &name, WEBREF_URL, |g| {
                    g.group(if is_type { "types" } else { "syntaxes" }).insert(definition_name.clone(), syntax_object(Some(&syntax)));
                });
                continue;
            }

            for scope in scopes {
                generator.scoped_definitions.push(ScopedDefinition {
                    name: name.clone(),
                    syntax: concat_str(Some(&syntax)),
                    scope: scope_of(&scope),
                    url: WEBREF_URL.to_string(),
                });
            }
        }
    }

    generator.check_renamed_properties();
    generator.resolve_scoped_definitions();
    // the non standard properties and selectors that webref does not define keep their mdn-data definition
    generator.add_legacy_definitions()?;

    // @webref/css does not list the units: they are defined by the local patch
    for unit in keys(generator.local_patch.get("units")) {
        generator.origins.group("units").insert(unit, LOCAL_PATCH_URL.to_string());
    }

    let mut units: Vec<String> = generator.origins.group("units").keys().cloned().collect();
    // Object.keys(origins.units): insertion order, then sorted
    units.sort_by(|a, b| code_unit_compare(a, b));
    generator.json["units"] = Value::Array(units.into_iter().map(Value::String).collect());

    // the csstree patch was written for another data source: it adds the missing definitions and extends the existing
    // ones, but it does not replace the webref definitions. the local patch may replace them
    let patches = generator.patches.clone();
    generator.apply_patches(false, PATCH_URL, &patches);
    let local_patch = generator.local_patch.clone();
    generator.apply_patches(true, LOCAL_PATCH_URL, &local_patch);
    // the vendor prefixed aliases with a legacy syntax extend the standard syntax
    generator.apply_legacy_syntaxes();

    for (name, declaration) in entries(generator.json.get("declarations")) {
        if let Some(alias) = present(&declaration, "alias") {
            let alias = concat_str(Some(alias));
            if !generator.group_ref("declarations").contains_key(&alias) {
                eprintln!(">> alias of an undefined property >> {} >> {}", name, alias);
            }
        }
    }

    generator.json = sort_keys(&generator.json);

    let output_directory = std::env::var("CSS_DATA_OUTPUT_DIR").map(PathBuf::from).unwrap_or_else(|_| root.join("crates/css-parser/src/data"));
    let output_file = output_directory.join("validation.json");
    // the origin of the definitions of the syntax file, used to report the origin of the removed definitions
    let origins_file = output_directory.join("validation.origins.json");
    // the changes are appended to this file, they are ported manually to CHANGELOG.md
    let changes_file = std::env::var("CSS_SYNTAX_CHANGES_FILE").map(PathBuf::from).unwrap_or_else(|_| root.join("SYNTAX-CHANGELOG.md"));

    // the previous syntax file, compared with the new one
    let previous: Option<Value> = std::fs::read_to_string(&output_file).ok().and_then(|c| serde_json::from_str(&c).ok());
    let previous_origins: Origins = std::fs::read_to_string(&origins_file).ok().and_then(|c| serde_json::from_str(&c).ok()).map_or_else(Origins::new, |v| parse_origins(&v));

    let json = generator.json.clone();
    std::fs::write(&output_file, serde_json::to_string(&json).expect("json")).map_err(|e| format!("{}: {}", output_file.display(), e))?;
    std::fs::write(&origins_file, serialize_origins(&collect_origins(&json, &generator.origins))).map_err(|e| format!("{}: {}", origins_file.display(), e))?;

    let summary = generator.summarize_changes(previous.as_ref(), &json, &previous_origins);

    println!("{}", summary);

    if previous.is_some() && summary.contains("\n### ") {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&changes_file).map_err(|e| format!("{}: {}", changes_file.display(), e))?;
        write!(file, "{}\n\n", summary).map_err(|e| e.to_string())?;
        eprintln!(">> syntax changes appended to {}", changes_file.display());
    }

    Ok(())
}

/// the root of the repository (the parent of `scripts/`)
fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().expect("repository root").to_path_buf()
}
