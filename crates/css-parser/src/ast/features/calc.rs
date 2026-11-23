//! Port of `src/ast/features/calc.ts`.

use super::r#type::FeatureWalkMode;
use super::{FeatureResult, MinifyFeature, SharedFeature};
use crate::ast::arena::{is_numeric_token_type, AstArena, TokenField};
use crate::ast::math::expression::evaluate;
use crate::ast::minify::MinifyContext;
use crate::ast::mixins::utils::is_in_template;
use crate::ast::types::{EnumAstNodeStatus, EnumToken};
use crate::ast::walk::{ValueWalker, WalkerValueOptions};
use crate::options::ParserOptions;
use crate::syntax::constants::{is_function_type, MATH_FUNCS};
use std::cell::RefCell;
use crate::utils::fx::HashSet;
use std::rc::Rc;

pub struct ComputeCalcExpressionFeature;

const ACCEPT: [u8; 2] = [EnumToken::RuleNodeType, EnumToken::AtRuleNodeType];

impl ComputeCalcExpressionFeature {
    pub fn register(options: &ParserOptions, features: &mut Vec<SharedFeature>) {
        if options.compute_calc_expression == Some(true) {
            features.push(Rc::new(RefCell::new(ComputeCalcExpressionFeature)));
        }
    }
}

impl MinifyFeature for ComputeCalcExpressionFeature {
    fn accept(&self) -> Option<&[u8]> {
        Some(&ACCEPT)
    }

    fn ordering(&self) -> i32 {
        3
    }

    fn process_mode(&self) -> u32 {
        FeatureWalkMode::Post
    }

    fn run(&mut self, arena: &AstArena, ast: u32, _options: &ParserOptions, _parent: u32, _context: &mut MinifyContext, _mode: u32) -> FeatureResult {
        // the mixin and custom function bodies are templates: the expressions are computed once they are used
        if arena.flags.get(ast) & EnumAstNodeStatus::HasChildren == 0 || is_in_template(arena, ast) {
            return FeatureResult::None;
        }
        let t = |x: u32| arena.data_types.get(x);
        let s = |x: u32| arena.resolve_string(arena.string_value.get(x));
        let mut node = arena.first_child.get(ast);
        while node != 0 {
            if t(node) == EnumToken::DeclarationNodeType && has_expression(arena, node) {
                // function tokens already evaluated, created on demand
                let mut set: Option<HashSet<u32>> = None;
                let mut walker = ValueWalker::of_node(arena, node, WalkerValueOptions::default(), false);
                while let Some(value) = walker.next(arena) {
                    // the parent of a detached token is the declaration
                    let parent = match arena.get_value_parent(value) {
                        0 => node,
                        p => p,
                    };
                    if t(parent) == EnumToken::BinaryExpressionTokenType {
                        continue;
                    }
                    let typ = t(value);
                    if typ == EnumToken::BinaryExpressionTokenType {
                        let result = evaluate(arena, &[value]);
                        if !result.is_empty() {
                            arena.replace_token(value, &result);
                        }
                        continue;
                    }
                    if !is_function_type(typ) {
                        continue;
                    }
                    let set = set.get_or_insert_with(HashSet::default);
                    if !set.insert(value) {
                        continue;
                    }
                    let name = s(value);
                    let should_evaluate = typ == EnumToken::MathFunctionTokenType || (typ == EnumToken::FunctionTokenType && MATH_FUNCS.contains(&name.as_str()));
                    if !should_evaluate {
                        continue;
                    }
                    let cp: Vec<u32> = if name != "calc" { vec![value] } else { arena.tokens(value) };
                    let values = evaluate(arena, &cp);
                    // fix a + -b into a - b
                    let mut inner = ValueWalker::of_tokens(&values, WalkerValueOptions::default(), false);
                    while let Some(v) = inner.next(arena) {
                        if t(v) == EnumToken::BinaryExpressionTokenType {
                            let r = if arena.get_child_at(v, TokenField::R).is_none() { 0 } else { arena.field_tokens(v, TokenField::R).first().copied().unwrap_or(0) };
                            if arena.numeric_value.get(v) as u8 == EnumToken::Add && is_numeric_token_type(t(r)) && arena.numeric_value.get(r) < 0.0 {
                                arena.numeric_value.set(v, EnumToken::Sub as f64);
                                arena.numeric_value.set(r, -arena.numeric_value.get(r));
                            }
                        }
                    }
                    // the value is replaced by the computed tokens or wrapped into a calc() function
                    if values.len() == 1 && t(values[0]) != EnumToken::BinaryExpressionTokenType {
                        let parent_type = t(parent);
                        let v0 = values[0];
                        let wrap = !(parent_type == EnumToken::MathFunctionTokenType || (parent_type == EnumToken::FunctionTokenType && s(parent) == "calc"))
                            && (!is_numeric_token_type(t(v0)) || arena.has_fraction(v0))
                            && !(t(v0) == EnumToken::MathFunctionTokenType || (t(v0) == EnumToken::FunctionTokenType && MATH_FUNCS.contains(&s(v0).as_str())));
                        let replacement = if wrap { create_calc(arena, value, &values) } else { v0 };
                        arena.replace_token(value, &[replacement]);
                    } else {
                        let calc = create_calc(arena, value, &values);
                        arena.replace_token(value, &[calc]);
                    }
                }
            }
            node = arena.next_sibling.get(node);
        }
        FeatureResult::None
    }
}

/// wrap tokens into a calc() function
fn create_calc(arena: &AstArena, source: u32, values: &[u32]) -> u32 {
    let calc = arena.create_function(EnumToken::MathFunctionTokenType, "calc", Some(values));
    arena.copy_location(calc, source);
    calc
}

/// true if a value token of the declaration is a function, an expression or has children
fn has_expression(arena: &AstArena, node: u32) -> bool {
    let mut token = arena.first_token.get(node);
    while token != 0 {
        let typ = arena.data_types.get(token);
        if typ == EnumToken::BinaryExpressionTokenType || is_function_type(typ) || arena.first_token.get(token) != 0 || arena.first_child.get(token) != 0 {
            return true;
        }
        token = arena.next_sibling.get(token);
    }
    false
}
