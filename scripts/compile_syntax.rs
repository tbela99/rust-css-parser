//! Port of `scripts/compile-syntax.ts` (`npm run syntax-compile`): the syntaxes of the validation data (declarations,
//! types and functions) are parsed once, at build time, by the library parser, the references (`<length>`, `<'width'>`,
//! `<rgb()>`) are resolved, and the matchers are written as a static table of nodes
//! (`crates/css-parser/src/validation/compiled/syntaxes.rs`) matched by `validation::compiled::runtime`. the structure
//! of a matcher mirrors the syntax tree matched by the interpreter: a compiled matcher answers "valid", or defers to
//! the interpreter, which reports the errors.
//!
//! the problems found in the syntax file are reported in `crates/css-parser/src/data/syntax-report.json`:
//! - unresolved: a type that is neither defined nor built in: the definitions that reference it are not compiled. a type
//!   that is not allowed (see [`ALLOWED_UNRESOLVED`]) fails the build
//! - unsupported: a syntax construct the compiler does not know: the definition is validated by the interpreter

use rust_css_parser::validation::compiled::builtins::{is_builtin_type, is_raw_type};
use rust_css_parser::validation::compiled::runtime::leaf_kind;
use rust_css_parser::validation::first::is_nullable;
use rust_css_parser::validation::parser::parse::{parse_syntax, trim_syntax_array};
use rust_css_parser::validation::parser::typedef::ValidationTokenEnum as V;
use rust_css_parser::validation::parser::types::{MatchMax, ValidationToken};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::rc::Rc;

type Syntax = Rc<ValidationToken>;

/// the types that may be referenced without a definition: they are validated by the interpreter. the build fails when
/// another type is unresolved: a definition was renamed or removed upstream (webref), fix it in scripts/local-patch.json
const ALLOWED_UNRESOLVED: &[&str] = &[
    "any-value",
    "at-keyword-token",
    "boolean-expr-group",
    "charset",
    "declaration-rule-list",
    "dimension-token",
    "group-rule-body",
    "number-token",
    "qualified-rule-list",
    "rule-list",
    "string-token",
    "style-feature-value",
    "stylesheet",
    "url-token",
    "when-condition",
    "anchored-feature",
    // a hex color is a color token: <color> is built in (match_property())
    "hex-color",
];

const GROUPS: [&str; 3] = ["declarations", "syntaxes", "types"];

enum Error {
    Unsupported(String),
    Unresolved(String),
}

struct Compiler {
    config: Value,
    /// the compiled definitions: `group/name`, by index
    keys: Vec<String>,
    indexes: HashMap<String, u32>,
    /// the nodes (rust expressions), deduplicated
    nodes: Vec<String>,
    node_indexes: HashMap<String, u32>,
}

impl Compiler {
    fn group(&self, group: &str) -> &Map<String, Value> {
        self.config[group].as_object().expect("group")
    }

    /// the definition of a group, the aliases are followed (see `lookup_node()`)
    fn lookup(&self, group: &str, key: &str) -> Option<(String, String)> {
        let entries = self.group(group);
        if let Some(mut node) = entries.get(key) {
            let mut key = key.to_string();
            let mut i = 0;
            while let Some(alias) = node.get("alias").and_then(Value::as_str) {
                if i >= 4 {
                    break;
                }
                key = alias.to_string();
                node = entries.get(alias)?;
                i += 1;
            }
            return if node.get("alias").is_some_and(|a| !a.is_null()) { None } else { Some((group.to_string(), key)) };
        }
        if group == "syntaxes" && self.group("types").contains_key(key) {
            return self.lookup("types", key);
        }
        None
    }

    /// a vendor prefixed name falls back to the standard name (see `find_node()`)
    fn resolve(&self, group: &str, key: &str) -> Option<(String, String)> {
        if let Some(found) = self.lookup(group, key) {
            return Some(found);
        }
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
        self.lookup(group, &format!("{}{}", at, tail))
    }

    /// the index of a definition, allocated on first reference
    fn index_of(&mut self, group: &str, key: &str) -> u32 {
        let name = format!("{}/{}", group, key);
        if let Some(index) = self.indexes.get(&name) {
            return *index;
        }
        let index = self.keys.len() as u32;
        self.keys.push(name.clone());
        self.indexes.insert(name, index);
        index
    }

    /// a node, deduplicated
    fn node(&mut self, expression: String) -> u32 {
        if let Some(index) = self.node_indexes.get(&expression) {
            return *index;
        }
        let index = self.nodes.len() as u32;
        self.nodes.push(expression.clone());
        self.node_indexes.insert(expression, index);
        index
    }

    /// a sequence of syntaxes. the whitespace is skipped
    fn sequence(&mut self, syntaxes: &[Syntax]) -> Result<u32, Error> {
        let items: Vec<Syntax> = syntaxes.iter().filter(|s| s.typ != V::Whitespace).cloned().collect();
        if items.is_empty() {
            return Ok(self.node("Empty".into()));
        }
        let nullable: Vec<bool> = (0..items.len()).map(|i| items[i..].iter().all(is_nullable)).collect();
        let mut indexes = Vec::new();
        for item in &items {
            indexes.push(self.element(item)?);
        }
        Ok(self.node(format!("Seq(&{:?}, &{:?})", indexes, nullable)))
    }

    /// a copy of a syntax without some of its multipliers
    fn strip(syntax: &Syntax, f: impl FnOnce(&mut ValidationToken)) -> Syntax {
        let mut token = (**syntax).clone();
        f(&mut token);
        Rc::new(token)
    }

    /// a syntax with its multipliers, in the order of `match_syntax()`
    fn element(&mut self, syntax: &Syntax) -> Result<u32, Error> {
        let optional = (syntax.is_optional() || syntax.is_repeatable()) && !syntax.is_mandatatory_group();
        let wrap = |this: &mut Self, index: u32| -> u32 { if optional { this.node(format!("Opt({})", index)) } else { index } };

        if syntax.is_repeatable_at_least_once() || syntax.is_repeatable() {
            let rest = Self::strip(syntax, |t| {
                t.is_repeatable = None;
                t.is_optional = None;
                t.is_mandatatory_group = None;
                t.is_repeatable_at_least_once = None;
            });
            let inner = self.sequence(&[rest])?;
            let index = self.node(format!("Rep({}, {})", inner, syntax.is_repeatable()));
            return Ok(wrap(self, index));
        }

        if syntax.is_list() {
            let rest = Self::strip(syntax, |t| {
                t.is_list = None;
                t.match_ = None;
                t.is_optional = None;
            });
            let commas = if rest.typ == V::Bracket { rest.children().iter().filter(|t| t.typ == V::Comma).count() } else { 0 };
            let (min, max) = Self::bounds(syntax);
            let inner = self.sequence(&[rest])?;
            let index = self.node(format!("List({}, {}, {}, {})", inner, min, max, commas));
            return Ok(wrap(self, index));
        }

        if syntax.match_.is_some() {
            let (min, max) = Self::bounds(syntax);
            let rest = Self::strip(syntax, |t| t.match_ = None);
            let inner = self.sequence(&[rest])?;
            let index = self.node(format!("Occ({}, {}, {})", inner, min, max));
            return Ok(wrap(self, index));
        }

        let base = self.base(syntax)?;

        match syntax.typ {
            V::PropertyType => {
                if let Some(range) = &syntax.range {
                    // the range is checked after the match: a value out of range fails, even if the syntax is optional
                    let number = |t: &Syntax| -> String { float(t.val_num()) };
                    let min = range.min.as_ref().map_or("f64::NEG_INFINITY".to_string(), number);
                    let max = range.max.as_ref().map_or("f64::INFINITY".to_string(), number);
                    let unit = range.min.as_ref().filter(|m| m.typ == V::Dimension).and_then(|m| m.unit.clone());
                    return Ok(self.node(format!("Rng({}, {}, {}, {:?}, {})", base, min, max, unit.as_deref(), optional)));
                }
                Ok(wrap(self, base))
            }
            // the syntaxes skipped when they do not match (see match_syntax())
            V::Colon | V::Keyword | V::DeclarationType | V::StringToken | V::Bracket | V::Number | V::Dimension => Ok(wrap(self, base)),
            _ => Ok(base),
        }
    }

    /// the bounds of `{min,max}`: (min, max)
    fn bounds(syntax: &Syntax) -> (u32, String) {
        match &syntax.match_ {
            None => (0, "f64::INFINITY".into()),
            Some(m) => {
                let min = if m.min.val_num().is_finite() { m.min.val_num() as u32 } else { 0 };
                let max = match &m.max {
                    MatchMax::Token(t) if t.val_num().is_finite() => float(t.val_num()),
                    _ => "f64::INFINITY".into(),
                };
                (min, max)
            }
        }
    }

    /// a reference to a definition
    fn reference(&mut self, group: &str, key: &str) -> Result<u32, Error> {
        match self.resolve(group, key) {
            None => Err(Error::Unresolved(if group == "declarations" { format!("'{}'", key) } else { key.to_string() })),
            Some((group, key)) => {
                let index = self.index_of(&group, &key);
                Ok(self.node(format!("Ref({})", index)))
            }
        }
    }

    /// a syntax without its multipliers
    fn base(&mut self, syntax: &Syntax) -> Result<u32, Error> {
        match syntax.typ {
            V::Keyword => Ok(self.node(format!("Kw({:?})", syntax.val_str().to_lowercase()))),
            V::PipeToken => {
                let alternatives: Vec<Vec<Syntax>> =
                    syntax.alternatives().iter().map(|a| a.iter().filter(|t| t.typ != V::Whitespace).cloned().collect()).collect();
                // the alternatives are keywords: a lookup
                if alternatives.iter().all(|a| {
                    a.len() == 1
                        && a[0].typ == V::Keyword
                        && !a[0].is_optional()
                        && !a[0].is_repeatable()
                        && !a[0].is_repeatable_at_least_once()
                        && !a[0].is_list()
                        && a[0].match_.is_none()
                        && a[0].range.is_none()
                }) {
                    let mut keywords: Vec<String> = alternatives.iter().map(|a| a[0].val_str().to_lowercase()).collect();
                    keywords.sort();
                    keywords.dedup();
                    return Ok(self.node(format!("Kws(&{:?})", keywords)));
                }
                let mut indexes = Vec::new();
                for alternative in &alternatives {
                    indexes.push(self.sequence(alternative)?);
                }
                Ok(self.node(format!("Alt(&{:?})", indexes)))
            }
            V::Bracket => self.sequence(syntax.children()),
            V::ColumnToken => {
                let operands: Vec<Vec<Syntax>> = syntax.alternatives().to_vec();
                self.unordered(&operands, false)
            }
            V::AmpersandToken => {
                let mut operands = Vec::new();
                ampersand_operands(syntax, &mut operands);
                self.unordered(&operands, true)
            }
            V::PropertyType => {
                let name = syntax.val_str();
                if is_builtin_type(name) {
                    if is_raw_type(name) {
                        return Ok(self.node("Interpreted".into()));
                    }
                    return Ok(self.node(format!("Leaf({:?}, {})", name, leaf_kind(name))));
                }
                self.reference("syntaxes", name)
            }
            V::DeclarationType => self.reference("declarations", syntax.val_str()),
            V::Function => {
                let args = self.sequence(syntax.children())?;
                Ok(self.node(format!("Fn({:?}, {})", syntax.val_str().to_lowercase(), args)))
            }
            V::FunctionDefinition => {
                let name = syntax.val_str();
                let definition = match self.resolve("syntaxes", &format!("{}()", name)) {
                    None => "None".to_string(),
                    Some((group, key)) => format!("Some({})", self.index_of(&group, &key)),
                };
                Ok(self.node(format!("FnDef({:?}, {})", name.to_lowercase(), definition)))
            }
            V::Number => Ok(self.node(format!("Num({}, None, None)", float(syntax.val_num())))),
            V::Dimension => Ok(self.node(format!("Num({}, {:?}, {:?})", float(syntax.val_num()), syntax.unit.as_deref(), syntax.unit_text.as_deref()))),
            V::StringToken => {
                let value = syntax.val_str();
                let inner = if value.len() >= 2 { &value[1..value.len() - 1] } else { "" };
                Ok(self.node(format!("Str({:?})", inner)))
            }
            V::Comma => Ok(self.node("Comma".into())),
            V::Colon => Ok(self.node("Colon".into())),
            V::Separator => Ok(self.node("Slash".into())),
            V::OpenParenthesis => Ok(self.node("OpenParens".into())),
            V::CloseParenthesis => Ok(self.node("CloseParens".into())),
            V::OpenCurlyBrace => Ok(self.node("OpenCurly".into())),
            V::OptionalGroupToken => {
                let chi = syntax.children();
                // `<bg-layer>#? , <final-bg-layer>`: `[ <bg-layer> , ]*` (see match_syntax())
                if chi.len() == 1 && chi[0].is_list() {
                    let item = Self::strip(&chi[0], |t| {
                        t.is_list = None;
                        t.is_optional = None;
                        t.match_ = None;
                    });
                    let group = self.sequence(&[item, Rc::new(ValidationToken::new(V::Comma))])?;
                    return Ok(self.node(format!("OptList({})", group)));
                }
                let group = self.sequence(chi)?;
                Ok(self.node(format!("Opt({})", group)))
            }
            typ => Err(Error::Unsupported(V::name(typ).unwrap_or("unknown").to_string())),
        }
    }

    /// `||` and `&&`: the operands and their nullable flags
    fn unordered(&mut self, operands: &[Vec<Syntax>], all: bool) -> Result<u32, Error> {
        let nullable: Vec<bool> = operands.iter().map(|o| o.iter().all(is_nullable)).collect();
        let mut indexes = Vec::new();
        for operand in operands {
            indexes.push(self.sequence(operand)?);
        }
        Ok(self.node(format!("Unordered(&{:?}, &{:?}, {})", indexes, nullable, all)))
    }
}

/// a float literal
fn float(value: f64) -> String {
    if value.is_nan() {
        "f64::NAN".into()
    } else if value == f64::INFINITY {
        "f64::INFINITY".into()
    } else if value == f64::NEG_INFINITY {
        "f64::NEG_INFINITY".into()
    } else {
        format!("{:?}", value)
    }
}

/// the operands of chained ampersands, see `ampersand_operands()`
fn ampersand_operands(syntax: &Syntax, operands: &mut Vec<Vec<Syntax>>) {
    for side in [syntax.l.as_deref().unwrap_or(&[]), syntax.r.as_deref().unwrap_or(&[])] {
        let tokens: Vec<&Syntax> = side.iter().filter(|t| t.typ != V::Whitespace).collect();
        if tokens.len() == 1
            && tokens[0].typ == V::AmpersandToken
            && !tokens[0].is_optional()
            && !tokens[0].is_repeatable()
            && !tokens[0].is_repeatable_at_least_once()
            && !tokens[0].is_list()
            && tokens[0].match_.is_none()
        {
            ampersand_operands(tokens[0], operands);
        } else {
            operands.push(side.to_vec());
        }
    }
}

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().expect("repository root").to_path_buf();
    let data = root.join("crates/css-parser/src/data");
    let output_file = root.join("crates/css-parser/src/validation/compiled/syntaxes.rs");
    let report_file = data.join("syntax-report.json");
    let config: Value = serde_json::from_str(&std::fs::read_to_string(data.join("validation.json")).expect("validation.json")).expect("json");

    let mut compiler = Compiler { config, keys: Vec::new(), indexes: HashMap::new(), nodes: Vec::new(), node_indexes: HashMap::new() };

    // the definitions are compiled in a stable order: the declarations, the functions and the types, sorted by name. the
    // referenced definitions get their index on first reference
    for group in GROUPS {
        let mut keys: Vec<String> =
            compiler.group(group).iter().filter(|(_, v)| v.get("alias").is_none_or(Value::is_null)).map(|(k, _)| k.clone()).collect();
        // `Array.prototype.sort()`: by utf-16 code units
        keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        for key in keys {
            compiler.index_of(group, &key);
        }
    }

    let mut roots: Vec<u32> = Vec::new();
    let mut unresolved: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut unsupported: BTreeMap<String, String> = BTreeMap::new();
    let mut index = 0;

    while index < compiler.keys.len() {
        let name = compiler.keys[index].clone();
        let (group, key) = name.split_once('/').expect("key");
        let syntax = compiler.group(group).get(key).and_then(|n| n.get("syntax")).and_then(Value::as_str).map(str::to_string);
        // the nodes allocated by a definition that is not compiled are kept: they are not referenced
        let result = match syntax {
            None => Err(Error::Unsupported("no syntax".into())),
            Some(syntax) => {
                let mut tokens = parse_syntax(&syntax);
                trim_syntax_array(&mut tokens);
                compiler.sequence(&tokens)
            }
        };
        let root = match result {
            Ok(root) => root,
            Err(error) => {
                match error {
                    Error::Unresolved(t) => unresolved.entry(t).or_default().push(name.clone()),
                    Error::Unsupported(construct) => {
                        unsupported.insert(name.clone(), construct);
                    }
                }
                // validated by the interpreter
                compiler.node("Interpreted".into())
            }
        };
        roots.push(root);
        index += 1;
    }

    let mut source = String::new();
    source.push_str("// generated by scripts/compile_syntax.rs from crates/css-parser/src/data/validation.json: do not edit, run `npm run syntax-compile`\n");
    source.push_str("use super::runtime::Node;\nuse super::runtime::Node::*;\n\n");
    source.push_str("/// the compiled definitions, `group/name`, by index\n");
    source.push_str(&format!("pub static KEYS: &[&str] = &{:?};\n\n", compiler.keys));
    source.push_str("/// the root node of the definitions, by index\n");
    source.push_str(&format!("pub static ROOTS: &[u32] = &{:?};\n\n", roots));
    source.push_str("/// the nodes\npub static NODES: &[Node] = &[\n");
    for (i, node) in compiler.nodes.iter().enumerate() {
        source.push_str(&format!("    /* {} */ {},\n", i, node));
    }
    source.push_str("];\n");
    std::fs::write(&output_file, source).expect("syntaxes.rs");

    for names in unresolved.values_mut() {
        names.sort();
    }

    let report = serde_json::json!({ "unresolved": unresolved, "unsupported": unsupported });
    // `JSON.stringify(report, null, 1)`
    let mut buffer = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut buffer, formatter);
    serde::Serialize::serialize(&report, &mut serializer).expect("report");
    std::fs::write(&report_file, String::from_utf8(buffer).expect("utf8") + "\n").expect("syntax-report.json");

    let failed: usize = unsupported.len() + unresolved.values().map(Vec::len).sum::<usize>();
    println!(">> {}/{} definitions compiled, see {}", compiler.keys.len() - failed, compiler.keys.len(), output_file.display());

    for (name, construct) in &unsupported {
        eprintln!(">> not compiled (unsupported {}) >> {}", construct, name);
    }

    let mut errors = 0;
    for (t, names) in &unresolved {
        let allowed = ALLOWED_UNRESOLVED.contains(&t.trim_matches('\''));
        eprintln!(">> {} <{}> >> {}", if allowed { "unresolved" } else { "ERROR unresolved" }, t, names.join(", "));
        if !allowed {
            errors += 1;
        }
    }

    if errors > 0 {
        eprintln!(
            ">> {} unresolved type(s): a definition was renamed or removed upstream. define it in scripts/local-patch.json, or allow it in scripts/compile_syntax.rs",
            errors
        );
        std::process::exit(1);
    }
}
