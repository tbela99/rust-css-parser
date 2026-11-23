//! Port of `src/ast/mixins/variables.ts`: the variables of the mixins and of the custom functions. The var()
//! functions that reference the private variables (mixin arguments, `@private`, local variables of a custom function,
//! ...) are replaced by a copy of the variable value (<https://drafts.csswg.org/css-mixins-1/>).
//!
//! The scopes are trait objects (`VariableScope`), the variables are shared cells. A variable refers to the scope its
//! value is resolved in with a weak reference: the scopes own their variables (no reference cycle), the scopes are
//! alive while their variables are resolved.

use super::custom_function::FunctionRegistry;
use crate::ast::arena::{AstArena, TokenValue};
use crate::ast::types::{EnumAstNodeStatus, EnumToken};
use crate::options::{ConvertColor, ParserOptions};
use crate::parser::declaration::JsMap;
use crate::parser::parse::parse_declarations;
use crate::parser::source::SourceFile;
use crate::printer::render::render_value;
use crate::validation::match_::{create_validation_context, match_all_syntaxes, Syntax};
use crate::validation::parser::parse::parse_syntax;
use std::cell::RefCell;
use crate::utils::fx::{HashMap, HashSet};
use std::rc::{Rc, Weak};

/// a private variable (`VariableEntry`)
#[derive(Default)]
pub struct VariableEntry {
    /// value tokens. `None` when the value is already resolved (see `text`)
    pub tokens: Option<Vec<u32>>,
    /// resolved value: `None` when it is not resolved yet (`undefined`), `Some(None)` when the value is invalid
    pub text: Option<Option<String>>,
    /// resolved value in the descendants of the element the mixin is applied to
    pub descendant_text: Option<Option<String>>,
    /// the value is being resolved: dependency cycle detection
    pub resolving: bool,
    /// scope used to resolve the var() functions of the value
    pub scope: Option<Weak<dyn VariableScope>>,
}

/// a shared variable
pub type Entry = Rc<RefCell<VariableEntry>>;

impl VariableEntry {
    /// a resolved value (`{ tokens: null, text, scope: null }`)
    pub fn resolved(text: Option<String>) -> Entry {
        Rc::new(RefCell::new(VariableEntry { text: Some(text), ..Default::default() }))
    }

    /// a value resolved in a scope (`{ tokens, scope }`)
    pub fn unresolved(tokens: Vec<u32>, scope: &Rc<dyn VariableScope>) -> Entry {
        Rc::new(RefCell::new(VariableEntry { tokens: Some(tokens), scope: Some(Rc::downgrade(scope)), ..Default::default() }))
    }
}

/// result of [`VariableScope::lookup`]
pub enum Found {
    /// the variable entry
    Entry(Entry),
    /// a private variable that is not visible (`UNDEFINED`): the fallback of var() is used
    Undefined,
    /// not a private variable (`null`): the var() function is evaluated by the browser
    Null,
}

/// variables visible from a value
pub trait VariableScope {
    /// find a variable. `descendant`: the variable is referenced by a descendant of the element a mixin is applied to
    fn lookup(&self, name: &str, descendant: bool) -> Found;

    /// the custom function calls are evaluated. `None` when they are not evaluated
    fn functions(&self) -> Option<Rc<FunctionRegistry>>;

    /// the variables of the call site that are visible from the body of a custom function. the scope itself when `None`
    fn dynamic(&self) -> Option<Rc<dyn VariableScope>> {
        None
    }

    /// the variables visible from the arguments of a custom function call. the scope itself when `None`
    fn arguments(&self) -> Option<Rc<dyn VariableScope>> {
        None
    }
}

/// custom properties by name
pub type Properties = Rc<HashMap<String, Entry>>;

/// variables of a style rule: the custom properties declared by the rule
pub struct RuleScope {
    pub properties: Properties,
    pub functions: Option<Rc<FunctionRegistry>>,
    pub dynamic: Option<Rc<dyn VariableScope>>,
}

impl RuleScope {
    pub fn new(properties: Properties, functions: Option<Rc<FunctionRegistry>>, dynamic: Option<Rc<dyn VariableScope>>) -> Rc<dyn VariableScope> {
        Rc::new(RuleScope { properties, functions, dynamic })
    }
}

impl VariableScope for RuleScope {
    fn lookup(&self, name: &str, _descendant: bool) -> Found {
        match self.properties.get(name) {
            Some(entry) => Found::Entry(entry.clone()),
            None => Found::Null,
        }
    }

    fn functions(&self) -> Option<Rc<FunctionRegistry>> {
        self.functions.clone()
    }

    fn dynamic(&self) -> Option<Rc<dyn VariableScope>> {
        self.dynamic.clone()
    }

    /// the arguments see the variables of the call site
    fn arguments(&self) -> Option<Rc<dyn VariableScope>> {
        self.dynamic.clone()
    }
}

/// at-rules that may contain mixin and custom function definitions
pub const SCOPE_RULES: [&str; 6] = ["media", "supports", "container", "layer", "when", "else"];

/// mixin or custom function parameter
#[derive(Clone, Debug)]
pub struct Parameter {
    pub name: String,
    /// syntax of the parameter type: `<color>`, `<length> | auto`. `None` when the parameter is not typed
    pub syntax: Option<String>,
    /// default value tokens
    pub default_value: Option<Vec<u32>>,
}

/// true if the token is a whitespace or a comment
pub fn is_blank(arena: &AstArena, token: u32) -> bool {
    matches!(arena.data_types.get(token), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType)
}

pub fn is_comment(arena: &AstArena, token: u32) -> bool {
    arena.data_types.get(token) == EnumToken::CommentTokenType
}

/// remove the leading and trailing whitespace and comments
pub fn trim<'t>(arena: &AstArena, tokens: &'t [u32]) -> &'t [u32] {
    let mut start = 0;
    let mut end = tokens.len();
    while start < end && is_blank(arena, tokens[start]) {
        start += 1;
    }
    while end > start && is_blank(arena, tokens[end - 1]) {
        end -= 1;
    }
    &tokens[start..end]
}

/// remove the curly braces that wrap a comma containing value: `{a, b}` -> `a, b`
pub fn unwrap(arena: &AstArena, tokens: &[u32]) -> Vec<u32> {
    let t = |x: u32| arena.data_types.get(x);
    // the curly braces of a declaration value: `--fn({a, b})`
    if tokens.len() == 1 && t(tokens[0]) == EnumToken::WrappedValuesTokenType {
        return trim(arena, &arena.tokens(tokens[0])).to_vec();
    }
    if tokens.len() < 2 || t(tokens[0]) != EnumToken::BlockStartTokenType || t(tokens[tokens.len() - 1]) != EnumToken::BlockEndTokenType {
        return tokens.to_vec();
    }
    let mut depth = 0;
    for (i, &token) in tokens.iter().enumerate() {
        if t(token) == EnumToken::BlockStartTokenType {
            depth += 1;
        } else if t(token) == EnumToken::BlockEndTokenType {
            depth -= 1;
            // the first brace is closed before the end: `{a} b {c}`
            if depth == 0 && i < tokens.len() - 1 {
                return tokens.to_vec();
            }
        }
    }
    trim(arena, &tokens[1..tokens.len() - 1]).to_vec()
}

/// split a list of tokens on the commas that are not wrapped in curly braces. the comments are ignored
pub fn split_arguments(arena: &AstArena, tokens: &[u32]) -> Vec<Vec<u32>> {
    let mut result: Vec<Vec<u32>> = vec![Vec::new()];
    let mut depth = 0;
    for &token in tokens {
        if is_comment(arena, token) {
            continue;
        }
        let typ = arena.data_types.get(token);
        if typ == EnumToken::BlockStartTokenType {
            depth += 1;
        } else if typ == EnumToken::BlockEndTokenType && depth > 0 {
            depth -= 1;
        } else if typ == EnumToken::CommaTokenType && depth == 0 {
            result.push(Vec::new());
            continue;
        }
        result.last_mut().unwrap().push(token);
    }
    result.into_iter().map(|part| trim(arena, &part).to_vec()).collect()
}

/// first token that is not a whitespace or a comment
pub fn first_token(arena: &AstArena, tokens: &[u32]) -> Option<u32> {
    tokens.iter().copied().find(|&t| !is_blank(arena, t))
}

/// true if the token is a var() function
pub fn is_var(arena: &AstArena, token: u32) -> bool {
    arena.data_types.get(token) == EnumToken::WildCardFunctionTokenType && arena.string_equals_ignore_case(arena.string_value.get(token), "var")
}

/// true if a token or one of its descendants is a var() function, or a custom function call when `functions` is set
pub fn has_reference(arena: &AstArena, token: u32, functions: bool) -> bool {
    if is_var(arena, token) || (functions && arena.data_types.get(token) == EnumToken::CustomFunctionTokenType) {
        return true;
    }
    if arena.first_token.get(token) == 0 && arena.first_child.get(token) == 0 {
        return false;
    }
    arena.sub_tokens(token).into_iter().any(|child| has_reference(arena, child, functions))
}

thread_local! {
    /// render options of the tokens: the values are parsed again, the colors are not converted
    static RENDER_OPTIONS: ParserOptions = ParserOptions { convert_color: Some(ConvertColor::Bool(false)), ..Default::default() };
}

/// render tokens
pub fn render(arena: &AstArena, tokens: &[u32]) -> String {
    RENDER_OPTIONS.with(|options| {
        let mut result = String::new();
        for &token in tokens {
            result.push_str(&render_value(arena, token, options));
        }
        result
    })
}

/// render tokens, the var() functions that reference a private variable replaced by a copy of the variable value, the
/// custom function calls replaced by their result when the scope evaluates them. `None` when a variable is invalid
/// and var() has no fallback: the value is not valid
pub fn substitute(arena: &AstArena, tokens: &[u32], scope: &Rc<dyn VariableScope>, descendant: bool) -> Option<String> {
    let functions = scope.functions().is_some();
    if !tokens.iter().any(|&t| has_reference(arena, t, functions)) {
        return Some(render(arena, tokens));
    }
    // the copies of the tokens are modified: the var() functions are replaced by their value
    let copies: Vec<u32> = tokens.iter().map(|&t| arena.clone_node_deep(t, true, None)).collect();
    let list = arena.create_list(EnumToken::ListToken, Some(&copies));
    if !replace_variables(arena, list, scope, descendant) {
        return None;
    }
    Some(render(arena, &arena.tokens(list)))
}

/// replace the var() functions that reference a private variable with literal tokens holding the value. false when
/// the value is not valid
pub fn replace_variables(arena: &AstArena, parent: u32, scope: &Rc<dyn VariableScope>, descendant: bool) -> bool {
    let literal = |value: &str| arena.create_token(EnumToken::LiteralTokenType, TokenValue::Str(value), None);
    for token in arena.sub_tokens(parent) {
        if is_var(arena, token) {
            match substitute_variable(arena, token, scope, descendant) {
                Substituted::Invalid => return false,
                Substituted::Value(value) => arena.replace_token(token, &[literal(&value)]),
                Substituted::Unchanged => {}
            }
        } else if let (Some(functions), EnumToken::CustomFunctionTokenType) = (scope.functions(), arena.data_types.get(token)) {
            // custom function call: `--fn(1px)`
            match functions.call(arena, token, scope, descendant) {
                Substituted::Invalid => return false,
                Substituted::Value(value) => arena.replace_token(token, &[literal(&value)]),
                // the call is not evaluated: its arguments may reference variables
                Substituted::Unchanged => {
                    if !replace_variables(arena, token, scope, descendant) {
                        return false;
                    }
                }
            }
        } else if (arena.first_token.get(token) != 0 || arena.first_child.get(token) != 0) && !replace_variables(arena, token, scope, descendant) {
            return false;
        }
    }
    true
}

/// value of a var() function or of a custom function call
pub enum Substituted {
    /// the value that replaces the function
    Value(String),
    /// `undefined`: the function is not modified
    Unchanged,
    /// `null`: the value is invalid
    Invalid,
}

impl From<Option<String>> for Substituted {
    fn from(value: Option<String>) -> Self {
        match value {
            Some(v) => Substituted::Value(v),
            None => Substituted::Invalid,
        }
    }
}

/// value of a var() function
pub fn substitute_variable(arena: &AstArena, token: u32, scope: &Rc<dyn VariableScope>, descendant: bool) -> Substituted {
    let children = arena.tokens(token);
    let Some(name_token) = first_token(arena, &children).filter(|&n| arena.data_types.get(n) == EnumToken::DashedIdenTokenType) else {
        return Substituted::Unchanged;
    };
    let name = arena.resolve_string(arena.string_value.get(name_token));
    let fallback: Option<&[u32]> = children
        .iter()
        .position(|&c| arena.data_types.get(c) == EnumToken::CommaTokenType)
        .map(|comma| trim(arena, &children[comma + 1..]));
    match scope.lookup(&name, descendant) {
        Found::Null => {
            // not a private variable: the fallback may reference private variables `var(--color, var(--private))`
            if let Some(fallback) = fallback {
                let functions = scope.functions().is_some();
                if fallback.iter().any(|&c| has_reference(arena, c, functions)) {
                    // an invalid fallback is equivalent to no fallback
                    return Substituted::Value(match substitute(arena, fallback, scope, descendant) {
                        None => format!("var({})", name),
                        Some(value) => format!("var({},{})", name, value),
                    });
                }
            }
            Substituted::Unchanged
        }
        found => {
            if let Found::Entry(entry) = found {
                if let Some(value) = resolve(arena, &entry, descendant) {
                    return Substituted::Value(value);
                }
            }
            // the variable is not defined or is invalid
            match fallback {
                None => Substituted::Invalid,
                Some(fallback) => substitute(arena, fallback, scope, descendant).into(),
            }
        }
    }
}

/// resolved value of a variable. `None` when the value is invalid
pub fn resolve(arena: &AstArena, entry: &Entry, descendant: bool) -> Option<String> {
    let (tokens, scope) = {
        let e = entry.borrow();
        match (&e.tokens, e.scope.as_ref().and_then(|s| s.upgrade())) {
            // the value is already resolved: the custom properties of the rule, the arguments
            (None, _) | (_, None) => return e.text.clone().flatten(),
            (Some(tokens), Some(scope)) => {
                let cached = if descendant { &e.descendant_text } else { &e.text };
                if let Some(value) = cached {
                    return value.clone();
                }
                // dependency cycle
                if e.resolving {
                    return None;
                }
                (tokens.clone(), scope)
            }
        }
    };
    entry.borrow_mut().resolving = true;
    let value = substitute(arena, &tokens, &scope, descendant);
    let mut e = entry.borrow_mut();
    e.resolving = false;
    if descendant {
        e.descendant_text = Some(value.clone());
    } else {
        e.text = Some(value.clone());
    }
    value
}

/// value tokens of a declaration, without `!important`
pub fn declaration_value(arena: &AstArena, declaration: u32) -> Vec<u32> {
    let all = arena.tokens(declaration);
    let tokens = trim(arena, &all);
    if tokens.last().is_some_and(|&l| arena.data_types.get(l) == EnumToken::ImportantTokenType) {
        return trim(arena, &tokens[..tokens.len() - 1]).to_vec();
    }
    tokens.to_vec()
}

/// true if the node is an at-rule with the given name
pub fn is_at_rule(arena: &AstArena, node: u32, name: &str) -> bool {
    arena.data_types.get(node) == EnumToken::AtRuleNodeType && arena.string_equals(arena.name.get(node), name)
}

/// true if the node is an at-rule with a block that may contain mixin definitions
pub fn is_scope_rule(arena: &AstArena, node: u32) -> bool {
    arena.data_types.get(node) == EnumToken::AtRuleNodeType
        && arena.flags.get(node) & EnumAstNodeStatus::HasChildren != 0
        && SCOPE_RULES.iter().any(|name| arena.string_equals(arena.name.get(node), name))
}

/// true if the node is a custom property declaration
pub fn is_custom_property(arena: &AstArena, node: u32) -> bool {
    arena.data_types.get(node) == EnumToken::DeclarationNodeType && arena.string_starts_with(arena.name.get(node), "--")
}

/// custom properties declared by the children of a node: the `!important` declarations override the other
/// declarations, the last declaration wins. the declarations of the inner nodes override the outer nodes
pub fn collect_properties(arena: &AstArena, node: u32, result: &mut HashMap<String, Entry>) {
    let mut important: HashSet<String> = HashSet::default();
    let mut properties: JsMap<Entry> = JsMap::new();
    let mut child = arena.first_child.get(node);
    while child != 0 {
        if is_custom_property(arena, child) {
            let name = arena.resolve_string(arena.name.get(child)).to_string();
            let is_important = arena.flags.get(child) & EnumAstNodeStatus::ImportantDeclaration != 0;
            if !(important.contains(&name) && !is_important) {
                if is_important {
                    important.insert(name.clone());
                }
                properties.set(name, VariableEntry::resolved(Some(render(arena, &declaration_value(arena, child)))));
            }
        }
        child = arena.next_sibling.get(child);
    }
    for (name, entry) in properties.iter() {
        result.entry(name.to_string()).or_insert_with(|| entry.clone());
    }
}

/// syntax of a parameter type: `<color>` -> `<color>`, `type(<length> | auto)` -> `<length> | auto`. `None` when the
/// parameter is not typed or accepts any value
pub fn parameter_syntax(arena: &AstArena, tokens: &[u32]) -> Option<String> {
    let filtered: Vec<u32> = tokens.iter().copied().filter(|&t| !is_comment(arena, t)).collect();
    let typ = trim(arena, &filtered);
    if typ.is_empty() {
        return None;
    }
    let syntax = if typ.len() == 1 && arena.data_types.get(typ[0]) == EnumToken::FunctionTokenType && arena.string_equals_ignore_case(arena.string_value.get(typ[0]), "type") {
        render(arena, trim(arena, &arena.tokens(typ[0]))).trim().to_string()
    } else {
        render(arena, typ).trim().to_string()
    };
    if syntax.is_empty() || syntax == "*" {
        None
    } else {
        Some(syntax)
    }
}

/// the values that are resolved by the browser: `/(^|[^\w-])(var|env|...)\(/i`
fn is_unresolved_value(value: &str) -> bool {
    const NAMES: [&str; 28] = [
        "var", "env", "attr", "if", "calc", "min", "max", "clamp", "round", "mod", "rem", "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "pow", "sqrt",
        "hypot", "log", "exp", "abs", "sign", "calc-size", "progress", "random",
    ];
    let lower = value.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    NAMES.iter().any(|name| {
        let pattern = format!("{}(", name);
        lower.match_indices(&pattern).any(|(at, _)| at == 0 || !(bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_' || bytes[at - 1] == b'-'))
    })
}

thread_local! {
    /// parsed syntaxes of the parameter types
    static SYNTAXES: RefCell<HashMap<String, Option<Rc<Vec<Syntax>>>>> = RefCell::new(HashMap::default());
}

/// parse a syntax: `None` when it is not supported (the typescript parser throws)
fn parse_type_syntax(syntax: &str) -> Option<Rc<Vec<Syntax>>> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let parsed = std::panic::catch_unwind(|| parse_syntax(syntax)).ok();
    std::panic::set_hook(previous);
    parsed.filter(|p| !p.is_empty()).map(Rc::new)
}

/// true if a value matches the type of a parameter. a value that is resolved by the browser is not checked
pub fn matches_type(arena: &AstArena, syntax: Option<&str>, value: &str, options: &ParserOptions) -> bool {
    let Some(syntax) = syntax else {
        return true;
    };
    if is_unresolved_value(value) {
        return true;
    }
    let parsed = SYNTAXES.with(|s| s.borrow_mut().entry(syntax.to_string()).or_insert_with(|| parse_type_syntax(syntax)).clone());
    let Some(parsed) = parsed else {
        return true;
    };
    // the value is parsed as a declaration value: the math functions are parsed into expressions
    let (_, nodes) = parse_declarations(&format!("--value:{}", value), Some(arena.rc()));
    let Some(declaration) = nodes.into_iter().find(|&n| arena.data_types.get(n) == EnumToken::DeclarationNodeType) else {
        return false;
    };
    let tokens = arena.tokens(declaration);
    thread_local! {
        /// source of the validation errors of the parameter types
        static VALIDATION_SOURCE: Rc<SourceFile> = SourceFile::new(Rc::from(""), Vec::new(), Some(Rc::from("")));
    }
    let options = ParserOptions { source: Some(VALIDATION_SOURCE.with(|s| s.clone())), ..options.clone() };
    match_all_syntaxes(arena, Some(&parsed), &create_validation_context(arena, &tokens), &options).success
}

/// custom properties of the rule the `@apply` rule belongs to: the declarations of the nearest style rule and of the
/// at-rules between them. `None` when the `@apply` rule is not inside a style rule
pub fn rule_properties(arena: &AstArena, node: u32) -> Option<HashMap<String, Entry>> {
    let mut result: HashMap<String, Entry> = HashMap::default();
    let mut parent = arena.parents.get(node);
    while parent != 0 {
        match arena.data_types.get(parent) {
            EnumToken::AtRuleNodeType => collect_properties(arena, parent, &mut result),
            EnumToken::RuleNodeType => {
                collect_properties(arena, parent, &mut result);
                return Some(result);
            }
            _ => return None,
        }
        parent = arena.parents.get(parent);
    }
    None
}

/// parse the parameters of a mixin or a custom function: `--name(--a, --b <color>: red)`. `None` when the parameters
/// are not valid
pub fn parse_parameters(arena: &AstArena, reference: u32) -> Option<Vec<Parameter>> {
    let mut parameters: Vec<Parameter> = Vec::new();
    let all = arena.tokens(reference);
    let children = trim(arena, &all);
    if children.iter().all(|&t| is_blank(arena, t)) {
        return Some(parameters);
    }
    let mut list = split_arguments(arena, children);
    // trailing comma: `@mixin --name(--a, --b,)`
    if list.len() > 1 && list.last().unwrap().is_empty() {
        list.pop();
    }
    for parameter in list {
        if parameter.is_empty() || arena.data_types.get(parameter[0]) != EnumToken::DashedIdenTokenType {
            return None;
        }
        // <dashed-ident> <css-type>? [ : <default-value> ]?
        let colon = parameter.iter().position(|&t| arena.data_types.get(t) == EnumToken::ColonTokenType);
        let name = arena.resolve_string(arena.string_value.get(parameter[0])).to_string();
        // the parameters are unique
        if parameters.iter().any(|p| p.name == name) {
            return None;
        }
        let end = colon.unwrap_or(parameter.len());
        parameters.push(Parameter {
            name,
            syntax: parameter_syntax(arena, &parameter[1..end]),
            default_value: colon.map(|c| unwrap(arena, trim(arena, &parameter[c + 1..]))),
        });
    }
    Some(parameters)
}

/// arguments of a mixin or a custom function call: the comments and the whitespace are ignored, the curly braces are
/// removed
pub fn parse_arguments(arena: &AstArena, reference: u32) -> Vec<Vec<u32>> {
    let children = arena.tokens(reference);
    if children.iter().all(|&t| is_blank(arena, t)) {
        return Vec::new();
    }
    split_arguments(arena, &children).iter().map(|arg| unwrap(arena, arg)).collect()
}

/// a definition located in the tree
pub trait Definition {
    fn node(&self) -> u32;
}

/// the definitions visible from a reference: the definitions of the innermost scope that defines the name
pub fn resolve_definitions<T: Definition + Clone>(arena: &AstArena, definitions: Option<&Vec<T>>, site: u32) -> Vec<T> {
    let Some(definitions) = definitions.filter(|d| !d.is_empty()) else {
        return Vec::new();
    };
    let mut scope = arena.parents.get(site);
    while scope != 0 {
        let result: Vec<T> = definitions.iter().filter(|d| arena.parents.get(d.node()) == scope).cloned().collect();
        if !result.is_empty() {
            return result;
        }
        scope = arena.parents.get(scope);
    }
    Vec::new()
}

/// name of a mixin or a custom function definition: `@mixin --name(...)`, `@function --name(...)`
pub fn definition_name(arena: &AstArena, node: u32) -> Option<String> {
    first_token(arena, &arena.tokens(node)).map(|r| arena.resolve_string(arena.string_value.get(r)).to_string())
}

/// collect the mixin or custom function definitions of a stylesheet, a conditional group rule or a cascade layer.
/// the definitions that are not valid are collected in `invalid`
pub fn collect_definitions<T>(arena: &AstArena, node: u32, name: &str, parse: &dyn Fn(&AstArena, u32) -> Option<T>, definitions: &mut JsMap<Vec<T>>, invalid: &mut Vec<u32>) {
    let mut child = arena.first_child.get(node);
    while child != 0 {
        if is_at_rule(arena, child, name) {
            let key = definition_name(arena, child);
            match key.and_then(|k| parse(arena, child).map(|d| (k, d))) {
                None => invalid.push(child),
                Some((key, definition)) => match definitions.get_mut(&key) {
                    Some(list) => list.push(definition),
                    None => definitions.set(key, vec![definition]),
                },
            }
        } else if is_scope_rule(arena, child) {
            collect_definitions(arena, child, name, parse, definitions, invalid);
        }
        child = arena.next_sibling.get(child);
    }
}

/// the mixin or custom function at-rules of a stylesheet
pub fn collect_at_rules(arena: &AstArena, node: u32, name: &str, result: &mut Vec<u32>) {
    let mut child = arena.first_child.get(node);
    while child != 0 {
        if is_at_rule(arena, child, name) {
            result.push(child);
        } else if is_scope_rule(arena, child) {
            collect_at_rules(arena, child, name, result);
        }
        child = arena.next_sibling.get(child);
    }
}

/// the prelude of a definition in its comment: the comments are removed (`/\/\*[\s\S]*?\*\//g`), the whitespace after
/// `(` (`/\(\s+/g`) and before `)` and `,` (`/\s+([),])/g`) too
fn comment_prelude(prelude: &str) -> String {
    let mut without_comments = String::with_capacity(prelude.len());
    let mut rest = prelude;
    while let Some(start) = rest.find("/*") {
        match rest[start + 2..].find("*/") {
            Some(end) => {
                without_comments.push_str(&rest[..start]);
                rest = &rest[start + 2 + end + 2..];
            }
            None => break,
        }
    }
    without_comments.push_str(rest);
    let chars: Vec<char> = without_comments.chars().collect();
    let mut result = String::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '(' {
            result.push(c);
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            continue;
        }
        if c.is_whitespace() {
            let mut j = i;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == ')' || chars[j] == ',') {
                i = j;
                continue;
            }
        }
        result.push(c);
        i += 1;
    }
    result.trim().to_string()
}

/// finalize the mixin or custom function definitions once they are evaluated: the evaluated definitions are replaced
/// with a comment unless `keep` is set, the definitions that are kept are reduced to their last definition in the same
/// scope, the definitions that are not valid are left as is
pub fn finalize_definitions(arena: &AstArena, ast: u32, name: &str, kept: &HashSet<u32>, keep: bool, invalid: &[u32]) {
    // the last definition in the same scope
    let mut last: HashMap<u32, HashMap<String, u32>> = HashMap::default();
    let mut nodes = Vec::new();
    collect_at_rules(arena, ast, name, &mut nodes);
    for node in nodes {
        let Some(key) = definition_name(arena, node) else {
            continue;
        };
        if invalid.contains(&node) {
            continue;
        }
        if !kept.contains(&node) {
            if !keep {
                // the comments of the prelude are removed: the comments are not nested
                let text = format!("/* @{} {} */", name, comment_prelude(&arena.resolve_string(arena.string_value.get(node))));
                let comment = arena.create_comment(EnumToken::CommentTokenType, crate::ast::arena::StrOrId::Str(&text));
                arena.copy_location(comment, node);
                arena.replace_child(node, &[comment]);
            }
            continue;
        }
        let scope = last.entry(arena.parents.get(node)).or_default();
        // the definition is defined only once in the same scope: the last definition wins
        if let Some(previous) = scope.insert(key, node) {
            arena.detach(previous);
        }
    }
}
