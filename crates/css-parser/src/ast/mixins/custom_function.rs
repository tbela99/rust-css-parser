//! Port of `src/ast/mixins/custom-function.ts`: a custom function call (`<dashed-function>`) is replaced by the
//! result of the function, like var() is replaced by the value of a custom property
//! (<https://drafts.csswg.org/css-mixins-1/#defining-custom-functions>).
//!
//! The registry is shared by the scopes (`Rc`): its counters and sets are interior mutable.

use super::variables::{
    declaration_value, first_token, has_reference, is_at_rule, is_custom_property, matches_type, parameter_syntax, parse_arguments, parse_parameters,
    resolve, resolve_definitions, rule_properties, substitute, Definition, Entry, Found, Parameter, RuleScope, Substituted, VariableEntry,
    VariableScope,
};
use crate::ast::arena::AstArena;
use crate::ast::types::{EnumAstNodeStatus, EnumToken};
use crate::options::ParserOptions;
use crate::parser::declaration::JsMap;
use crate::parser::parse::parse_declarations;
use std::cell::{Cell, RefCell};
use crate::utils::fx::{HashMap, HashSet};
use std::rc::{Rc, Weak};

/// maximum number of nested custom function calls
const MAX_DEPTH: usize = 32;

/// custom function definition
#[derive(Clone)]
pub struct FunctionDefinition {
    pub node: u32,
    pub name: String,
    pub parameters: Vec<Parameter>,
    /// syntax of the return type: `returns <length>`. `None` when the result is not typed
    pub returns: Option<String>,
    /// the function body contains conditional rules: the function is not evaluated
    pub conditional_body: bool,
    /// local variables: the custom properties declared in the function body. the last declaration wins
    pub locals: Rc<JsMap<Vec<u32>>>,
    /// result tokens: the last `result` descriptor. `None` when the function has no result
    pub result: Option<Vec<u32>>,
}

impl Definition for FunctionDefinition {
    fn node(&self) -> u32 {
        self.node
    }
}

/// variables visible from the body of a custom function
struct FunctionScope {
    locals: RefCell<HashMap<String, Entry>>,
    args: RefCell<HashMap<String, Entry>>,
    /// scope of the call: the calling function, or the element
    parent: Rc<dyn VariableScope>,
    functions: Rc<FunctionRegistry>,
}

impl VariableScope for FunctionScope {
    fn lookup(&self, name: &str, descendant: bool) -> Found {
        let found = self.locals.borrow().get(name).cloned().or_else(|| self.args.borrow().get(name).cloned());
        if let Some(entry) = found {
            return Found::Entry(entry);
        }
        self.parent.lookup(name, descendant)
    }

    fn functions(&self) -> Option<Rc<FunctionRegistry>> {
        Some(self.functions.clone())
    }
}

/// parse a custom function definition. `None` if the prelude is not valid
///
/// ```css
/// @function --name(--a <length>: 1px) returns <length> { result: calc(2 * var(--a)) }
/// ```
pub fn parse_function(arena: &AstArena, node: u32) -> Option<FunctionDefinition> {
    let tokens = arena.tokens(node);
    let reference = first_token(arena, &tokens).filter(|&r| arena.data_types.get(r) == EnumToken::CustomFunctionTokenType)?;
    let parameters = parse_parameters(arena, reference)?;
    // [ returns <css-type> ]?
    let returns = tokens
        .iter()
        .position(|&t| arena.data_types.get(t) == EnumToken::IdenTokenType && arena.string_equals_ignore_case(arena.string_value.get(t), "returns"))
        .and_then(|index| parameter_syntax(arena, &tokens[index + 1..]));
    let mut definition = FunctionDefinition {
        node,
        name: arena.resolve_string(arena.string_value.get(reference)).to_string(),
        parameters,
        returns,
        conditional_body: false,
        locals: Rc::new(JsMap::new()),
        result: None,
    };
    let mut locals: JsMap<Vec<u32>> = JsMap::new();
    let mut child = arena.first_child.get(node);
    while child != 0 {
        if arena.data_types.get(child) == EnumToken::AtRuleNodeType {
            definition.conditional_body = true;
        } else if is_custom_property(arena, child) {
            locals.set(arena.resolve_string(arena.name.get(child)).to_string(), declaration_value(arena, child));
        } else if arena.data_types.get(child) == EnumToken::DeclarationNodeType && arena.string_equals(arena.name.get(child), "result") {
            definition.result = Some(declaration_value(arena, child));
        }
        child = arena.next_sibling.get(child);
    }
    definition.locals = Rc::new(locals);
    Some(definition)
}

/// local variable of a custom function. `initial`: the value of the parameter with the same name; `inherit` /
/// `unset`: the value of the variable with the same name in the calling context
fn local_variable(
    arena: &AstArena,
    name: &str,
    tokens: &[u32],
    values: &HashMap<String, Entry>,
    context: &Rc<dyn VariableScope>,
    scope: &Rc<dyn VariableScope>,
    descendant: bool,
) -> Entry {
    if tokens.len() == 1 && arena.data_types.get(tokens[0]) == EnumToken::IdenTokenType {
        let value = arena.string_value.get(tokens[0]);
        if arena.string_equals_ignore_case(value, "initial") {
            return values.get(name).cloned().unwrap_or_else(|| VariableEntry::resolved(None));
        }
        if arena.string_equals_ignore_case(value, "inherit") || arena.string_equals_ignore_case(value, "unset") {
            return match context.lookup(name, descendant) {
                Found::Undefined => VariableEntry::resolved(None),
                Found::Entry(entry) => entry,
                Found::Null => VariableEntry::resolved(Some(format!("var({})", name))),
            };
        }
    }
    VariableEntry::unresolved(tokens.to_vec(), scope)
}

/// evaluates the custom function calls (`FunctionRegistry`)
pub struct FunctionRegistry {
    /// the function definitions that have been evaluated
    pub evaluated: RefCell<HashSet<u32>>,
    /// the function definitions referenced by the calls that have not been evaluated: they are kept
    pub pending: RefCell<HashSet<u32>>,
    /// number of evaluated calls
    pub count: Cell<usize>,
    /// number of calls that are not evaluated
    pub failed: Cell<usize>,
    /// the calls are located in a custom property declaration
    pub custom_property: Cell<bool>,
    /// location of the calls: the declaration or the `@apply` rule that contains them
    pub site: Cell<u32>,
    /// functions being evaluated
    stack: RefCell<Vec<String>>,
    /// the functions that call themselves, directly or through other functions
    recursive: RefCell<HashMap<String, bool>>,
    /// function definitions by name, in document order
    pub definitions: JsMap<Vec<FunctionDefinition>>,
    options: ParserOptions,
    this: Weak<FunctionRegistry>,
}

impl FunctionRegistry {
    pub fn new(definitions: JsMap<Vec<FunctionDefinition>>, options: &ParserOptions) -> Rc<Self> {
        Rc::new_cyclic(|this| FunctionRegistry {
            evaluated: RefCell::new(HashSet::default()),
            pending: RefCell::new(HashSet::default()),
            count: Cell::new(0),
            failed: Cell::new(0),
            custom_property: Cell::new(false),
            site: Cell::new(0),
            stack: RefCell::new(Vec::new()),
            recursive: RefCell::new(HashMap::default()),
            definitions,
            options: options.clone(),
            this: this.clone(),
        })
    }

    /// result of a custom function call: the result that replaces the call, `Unchanged` when the call is not evaluated
    pub fn call(&self, arena: &AstArena, token: u32, caller: &Rc<dyn VariableScope>, descendant: bool) -> Substituted {
        let name = arena.resolve_string(arena.string_value.get(token));
        // the function is not defined in the stylesheet
        let Some(all) = self.definitions.get(&name) else {
            return Substituted::Unchanged;
        };
        // the functions visible from the call
        let definitions = resolve_definitions(arena, Some(all), self.site.get());
        let definition = if definitions.len() == 1 { Some(&definitions[0]) } else { None };
        let unusable = match definition {
            None => true,
            Some(d) => {
                d.conditional_body
                    || self.stack.borrow().len() >= MAX_DEPTH
                    || self.is_recursive(arena, &name, &mut Vec::new())
                    || (self.custom_property.get() && (d.returns.is_some() || d.parameters.iter().any(|p| p.syntax.is_some())))
            }
        };
        if unusable {
            // the visible definitions are kept
            for visible in &definitions {
                self.pending.borrow_mut().insert(visible.node);
            }
            self.failed.set(self.failed.get() + 1);
            return Substituted::Unchanged;
        }
        let definition = definition.unwrap();
        let failed = self.failed.get();
        let mut result = self.evaluate(arena, definition, token, caller, descendant);
        // a call of the function body is not evaluated: the function is not evaluated
        if result.is_some() && self.failed.get() != failed {
            result = None;
        }
        match result {
            None => {
                self.pending.borrow_mut().insert(definition.node);
                self.failed.set(self.failed.get() + 1);
                Substituted::Unchanged
            }
            Some(result) => {
                self.evaluated.borrow_mut().insert(definition.node);
                self.count.set(self.count.get() + 1);
                Substituted::Value(result)
            }
        }
    }

    /// evaluate a call. `None` when the arguments do not match the parameters or the result is not valid
    fn evaluate(&self, arena: &AstArena, definition: &FunctionDefinition, token: u32, caller: &Rc<dyn VariableScope>, descendant: bool) -> Option<String> {
        let args = parse_arguments(arena, token);
        if args.len() > definition.parameters.len() {
            return None;
        }
        // the variables of the call site: the custom properties of the rule, or the variables of the calling function
        let context: Rc<dyn VariableScope> = caller.dynamic().unwrap_or_else(|| caller.clone());
        let argument_scope: Rc<dyn VariableScope> = caller.arguments().unwrap_or_else(|| caller.clone());
        let function_scope = Rc::new(FunctionScope {
            locals: RefCell::new(HashMap::default()),
            args: RefCell::new(HashMap::default()),
            parent: context.clone(),
            functions: self.this.upgrade().expect("registry"),
        });
        let scope: Rc<dyn VariableScope> = function_scope.clone();
        for (index, parameter) in definition.parameters.iter().enumerate() {
            let entry = if index < args.len() {
                // the arguments are evaluated in the scope of the call
                VariableEntry::resolved(Some(substitute(arena, &args[index], &argument_scope, descendant)?))
            } else if let Some(default) = &parameter.default_value {
                VariableEntry::unresolved(default.clone(), &scope)
            } else {
                return None;
            };
            function_scope.args.borrow_mut().insert(parameter.name.clone(), entry);
        }
        for (name, tokens) in definition.locals.iter() {
            let values = function_scope.args.borrow().clone();
            let entry = local_variable(arena, name, tokens, &values, &context, &scope, descendant);
            function_scope.locals.borrow_mut().insert(name.to_string(), entry);
        }
        self.stack.borrow_mut().push(definition.name.clone());
        let result = (|| {
            for parameter in &definition.parameters {
                let entry = function_scope.args.borrow().get(&parameter.name).cloned()?;
                let value = resolve(arena, &entry, descendant)?;
                if !matches_type(arena, parameter.syntax.as_deref(), &value, &self.options) {
                    return None;
                }
            }
            // no result: guaranteed-invalid value. the call is left as is
            let tokens = definition.result.as_ref()?;
            let result = substitute(arena, tokens, &scope, descendant)?;
            // a result that does not match the return type: the call is left as is
            if !matches_type(arena, definition.returns.as_deref(), &result, &self.options) {
                return None;
            }
            Some(result)
        })();
        self.stack.borrow_mut().pop();
        result
    }

    /// true if a function calls itself, directly or through the functions it calls
    fn is_recursive(&self, arena: &AstArena, name: &str, stack: &mut Vec<String>) -> bool {
        if stack.iter().any(|s| s == name) {
            return true;
        }
        if let Some(&result) = self.recursive.borrow().get(name) {
            return result;
        }
        let mut result = false;
        stack.push(name.to_string());
        'definitions: for definition in self.definitions.get(name).map(|d| d.as_slice()).unwrap_or(&[]) {
            let mut values: Vec<&Vec<u32>> = definition.locals.values().collect();
            if let Some(result) = &definition.result {
                values.push(result);
            }
            for parameter in &definition.parameters {
                if let Some(default) = &parameter.default_value {
                    values.push(default);
                }
            }
            for tokens in values {
                for call in find_calls(arena, tokens) {
                    let callee = arena.resolve_string(arena.string_value.get(call));
                    if self.definitions.has(&callee) && self.is_recursive(arena, &callee, stack) {
                        result = true;
                        break 'definitions;
                    }
                }
            }
        }
        stack.pop();
        // a function that reaches one of the functions being checked is part of a cycle
        self.recursive.borrow_mut().insert(name.to_string(), result);
        result
    }
}

/// the custom function calls of a list of tokens and their descendants
pub fn find_calls(arena: &AstArena, tokens: &[u32]) -> Vec<u32> {
    let mut result = Vec::new();
    find_calls_into(arena, tokens, &mut result);
    result
}

fn find_calls_into(arena: &AstArena, tokens: &[u32], result: &mut Vec<u32>) {
    for &token in tokens {
        if arena.data_types.get(token) == EnumToken::CustomFunctionTokenType {
            result.push(token);
        }
        if arena.first_token.get(token) != 0 || arena.first_child.get(token) != 0 {
            find_calls_into(arena, &arena.sub_tokens(token), result);
        }
    }
}

/// replace the custom function calls of the declarations of a stylesheet with their result. the bodies of the mixins
/// and of the custom functions are evaluated when they are used
pub fn evaluate_functions(arena: &AstArena, ast: u32, registry: &Rc<FunctionRegistry>) {
    let mut stack: Vec<u32> = Vec::new();
    let mut child = arena.last_child.get(ast);
    while child != 0 {
        stack.push(child);
        child = arena.previous_sibling.get(child);
    }
    while let Some(node) = stack.pop() {
        if is_at_rule(arena, node, "mixin") || is_at_rule(arena, node, "function") {
            continue;
        }
        if arena.data_types.get(node) == EnumToken::DeclarationNodeType {
            evaluate_declaration(arena, node, registry);
            continue;
        }
        if arena.flags.get(node) & EnumAstNodeStatus::HasChildren != 0 {
            let mut child = arena.last_child.get(node);
            while child != 0 {
                stack.push(child);
                child = arena.previous_sibling.get(child);
            }
        }
    }
}

/// replace the custom function calls of a declaration with their result
fn evaluate_declaration(arena: &AstArena, declaration: u32, registry: &Rc<FunctionRegistry>) {
    let tokens = arena.tokens(declaration);
    if !tokens.iter().any(|&t| has_reference(arena, t, true)) {
        return;
    }
    // the var() functions of the declaration are not modified. the custom functions see the custom properties of the
    // rule: the call site
    let dynamic = RuleScope::new(Rc::new(rule_properties(arena, declaration).unwrap_or_default()), Some(registry.clone()), None);
    let scope = RuleScope::new(Rc::new(HashMap::default()), Some(registry.clone()), Some(dynamic));
    let count = registry.count.get();
    let failed = registry.failed.get();
    registry.custom_property.set(is_custom_property(arena, declaration));
    registry.site.set(declaration);
    let value = substitute(arena, &tokens, &scope, false);
    // no call has been evaluated, a call is not evaluated or the value is not valid: the declaration is left as is
    let Some(value) = value.filter(|_| registry.count.get() != count && registry.failed.get() == failed) else {
        return;
    };
    let css = format!("{}:{}", arena.resolve_string(arena.name.get(declaration)), value);
    let (_, nodes) = parse_declarations(&css, Some(arena.rc()));
    let nodes: Vec<u32> = nodes.into_iter().filter(|&n| arena.data_types.get(n) == EnumToken::DeclarationNodeType).collect();
    if nodes.len() == 1 {
        arena.copy_location(nodes[0], declaration);
        arena.replace_child(declaration, &[nodes[0]]);
    }
}
