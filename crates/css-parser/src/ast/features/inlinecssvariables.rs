//! Port of `src/ast/features/inlinecssvariables.ts`.

use super::r#type::FeatureWalkMode;
use super::{FeatureResult, MinifyFeature, SharedFeature};
use crate::ast::arena::{AstArena, StrOrId, TokenField, TokenValue};
use crate::ast::minify::{split_rule, MinifyContext};
use crate::ast::mixins::utils::is_in_template;
use crate::ast::types::{EnumAstNodeStatus, EnumToken};
use crate::ast::walk::{ValueWalker, WalkerValueOptions};
use crate::options::{ConvertColor, ParserOptions};
use crate::printer::render::render_value;
use crate::syntax::constants::MATH_FUNCS;
use std::cell::RefCell;
use std::rc::Rc;

/// a custom property that may be inlined (`VariableScopeInfo`)
pub struct VariableScopeInfo {
    pub replaceable: bool,
    /// the rule that declares the property
    pub parent: u32,
    pub node: u32,
    /// detached copies of the value tokens
    pub values: Vec<u32>,
}

/// the custom properties by name, insertion ordered (`context.variableScope`)
pub type VariableScope = Rc<RefCell<crate::parser::declaration::JsMap<VariableScopeInfo>>>;

/// inline a binary expression into a parens token: (l op r)
fn inline_expression(arena: &AstArena, token: u32) -> Vec<u32> {
    if arena.data_types.get(token) != EnumToken::BinaryExpressionTokenType {
        return vec![token];
    }
    let mut chi = inline_expression(arena, arena.field(token, TokenField::L));
    chi.push(arena.create_token(arena.numeric_value.get(token) as u8, TokenValue::None, None));
    chi.extend(inline_expression(arena, arena.field(token, TokenField::R)));
    vec![arena.create_list(EnumToken::ParensTokenType, Some(&chi))]
}

fn replace(arena: &AstArena, node: u32, variable_scope: &VariableScope) {
    let mut walker = ValueWalker::of_node(arena, node, WalkerValueOptions::default(), false);
    while let Some(value) = walker.next(arena) {
        // the parent of a detached token is the declaration
        let parent_value = match arena.get_value_parent(value) {
            0 => node,
            p => p,
        };
        if arena.data_types.get(value) == EnumToken::BinaryExpressionTokenType && parent_value != node && arena.parents.get(value) == parent_value {
            arena.replace_token(value, &inline_expression(arena, value));
        }
    }
    let mut walker = ValueWalker::of_node(arena, node, WalkerValueOptions::default(), false);
    while let Some(value) = walker.next(arena) {
        let typ = arena.data_types.get(value);
        if typ == EnumToken::MathFunctionTokenType || (typ == EnumToken::WildCardFunctionTokenType && arena.string_equals(arena.string_value.get(value), "var")) {
            let first = arena.first_token.get(value);
            if first != 0 && arena.next_sibling.get(first) == 0 && arena.data_types.get(first) == EnumToken::DashedIdenTokenType {
                let scope = variable_scope.borrow();
                let info = arena.with_string(arena.string_value.get(first), |name| scope.get(name).map(|i| (i.replaceable, i.node)));
                // the variable declaration may have been released
                if let Some((true, declaration)) = info {
                    if arena.name.get(declaration) != 0 && arena.parents.get(value) != 0 {
                        let tokens: Vec<u32> = arena.tokens(declaration).into_iter().map(|t| arena.clone_node_deep(t, true, None)).collect();
                        drop(scope);
                        arena.replace_token(value, &tokens);
                    }
                }
            }
        }
    }
}

pub struct InlineCssVariablesFeature;

const ACCEPT: [u8; 2] = [EnumToken::RuleNodeType, EnumToken::AtRuleNodeType];

impl InlineCssVariablesFeature {
    pub fn register(options: &ParserOptions, features: &mut Vec<SharedFeature>) {
        if options.inline_css_variables == Some(true) {
            features.push(Rc::new(RefCell::new(InlineCssVariablesFeature)));
        }
    }
}

impl MinifyFeature for InlineCssVariablesFeature {
    fn accept(&self) -> Option<&[u8]> {
        Some(&ACCEPT)
    }

    fn ordering(&self) -> i32 {
        2
    }

    fn process_mode(&self) -> u32 {
        FeatureWalkMode::Post
    }

    fn run(&mut self, arena: &AstArena, ast: u32, _options: &ParserOptions, parent: u32, context: &mut MinifyContext, _mode: u32) -> FeatureResult {
        // the variables of a mixin or a custom function body are private variables
        if arena.flags.get(ast) & EnumAstNodeStatus::HasChildren == 0 || is_in_template(arena, ast) {
            return FeatureResult::None;
        }
        let variable_scope: VariableScope = context.get_or_insert_with("variableScope", VariableScope::default).clone();
        // [':root', 'html']
        let is_root = arena.data_types.get(parent) == EnumToken::StyleSheetNodeType && arena.data_types.get(ast) == EnumToken::RuleNodeType && {
            let raw = arena.raw.borrow().get(&ast).cloned().unwrap_or_else(|| std::rc::Rc::new(crate::ast::arena::RawSelector::from(split_rule(&arena.resolve_string(arena.name.get(ast))))));
            raw.iter().any(|segment| segment.iter().any(|s| s == ":root" || s == "html"))
        };
        let mut node = arena.first_child.get(ast);
        while node != 0 {
            if arena.data_types.get(node) == EnumToken::DeclarationNodeType {
                if arena.string_starts_with(arena.name.get(node), "--") {
                    // css variable: the last declaration wins
                    let name = arena.resolve_string(arena.name.get(node));
                    let values = arena.tokens(node).into_iter().map(|t| arena.clone_node_deep(t, true, None)).collect();
                    variable_scope.borrow_mut().set(name, VariableScopeInfo { replaceable: is_root, parent: ast, node, values });
                    let mut recursive = false;
                    let mut walker = ValueWalker::of_node(arena, node, WalkerValueOptions::default(), false);
                    while let Some(value) = walker.next(arena) {
                        let typ = arena.data_types.get(value);
                        if typ == EnumToken::MathFunctionTokenDefType
                            || (typ == EnumToken::WildCardFunctionTokenType
                                && arena.with_string(arena.string_value.get(value), |v| MATH_FUNCS.contains(&v) || v == "var"))
                        {
                            recursive = true;
                            break;
                        }
                    }
                    if recursive {
                        replace(arena, node, &variable_scope);
                    }
                } else {
                    replace(arena, node, &variable_scope);
                }
            }
            node = arena.next_sibling.get(node);
        }
        FeatureResult::None
    }

    fn has_cleanup(&self) -> bool {
        true
    }

    fn cleanup(&mut self, arena: &AstArena, _ast: u32, _options: &ParserOptions, context: &mut MinifyContext, _mode: u32) {
        let Some(variable_scope) = context.get_mut::<VariableScope>("variableScope").cloned() else {
            return;
        };
        thread_local! {
            static OPTIONS: ParserOptions = ParserOptions { convert_color: Some(ConvertColor::Bool(false)), ..Default::default() };
        }
        for info in variable_scope.borrow().values() {
            // drop the declarations from :root{}
            if info.replaceable && arena.parents.get(info.node) == info.parent {
                let mut text = format!("/* {}: ", arena.resolve_string(arena.name.get(info.node)));
                OPTIONS.with(|o| {
                    for &t in &info.values {
                        text.push_str(&render_value(arena, t, o));
                    }
                });
                text.push_str(" */");
                let comment = arena.create_comment(EnumToken::CommentTokenType, StrOrId::Str(&text));
                arena.replace_child(info.node, &[comment]);
                arena.detach(info.node);
            }
        }
    }
}
