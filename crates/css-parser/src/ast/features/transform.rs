//! Port of `src/ast/features/transform.ts`.

use super::r#type::FeatureWalkMode;
use super::{FeatureResult, MinifyFeature, SharedFeature};
use crate::ast::arena::AstArena;
use crate::ast::cache::StructuralMemo;
use crate::ast::minify::MinifyContext;
use crate::ast::transform::compute::{compute, transform_kind, TransformKind};
use crate::ast::transform::minify::{eq_matrix, minify_transform_functions};
use crate::ast::types::{EnumAstNodeStatus, EnumToken};
use crate::options::ParserOptions;
use crate::printer::render::{filter_values, render_value_default};
use std::cell::RefCell;
use std::rc::Rc;

/// outcome of the minification of a transform value (`Outcome` / detached tokens of `TransformMemo`)
#[derive(Clone)]
enum Outcome {
    /// the matrix cannot be computed: the transform functions are minified one by one
    Failed,
    /// the transform functions minified one by one are the shortest form
    Functions,
    /// the shortest form reuses tokens of the value: it is computed for every declaration
    Uncached,
    /// the detached tokens of the shortest form, cloned for every declaration
    Tokens(Vec<u32>),
}

/// results of the minification of the transform values, shared by the rules of a minification: the matrix of a value
/// is computed once per distinct value (`TransformMemo`)
type TransformMemo = Rc<RefCell<StructuralMemo<Outcome>>>;

pub struct TransformCssFeature;

const ACCEPT: [u8; 3] = [EnumToken::RuleNodeType, EnumToken::AtRuleNodeType, EnumToken::KeyframesRuleNodeType];

impl TransformCssFeature {
    pub fn register(options: &ParserOptions, features: &mut Vec<SharedFeature>) {
        if options.compute_transform == Some(true) {
            features.push(Rc::new(RefCell::new(TransformCssFeature)));
        }
    }
}

/// length of the rendered tokens (utf-16 code units)
fn rendered_length(arena: &AstArena, tokens: &[u32]) -> usize {
    tokens.iter().map(|&t| render_value_default(arena, t).encode_utf16().count()).sum()
}

/// the candidate chosen as the shortest form
#[derive(PartialEq)]
enum Choice {
    Matrix,
    Children,
    Cumulative,
    Minified,
}

impl MinifyFeature for TransformCssFeature {
    fn accept(&self) -> Option<&[u8]> {
        Some(&ACCEPT)
    }

    fn ordering(&self) -> i32 {
        4
    }

    fn process_mode(&self) -> u32 {
        FeatureWalkMode::Post
    }

    fn run(&mut self, arena: &AstArena, ast: u32, _options: &ParserOptions, _parent: u32, context: &mut MinifyContext, _mode: u32) -> FeatureResult {
        if arena.flags.get(ast) & EnumAstNodeStatus::HasChildren == 0 {
            return FeatureResult::None;
        }
        let mut memo: Option<TransformMemo> = None;
        let mut node = arena.first_child.get(ast);
        while node != 0 {
            let next = arena.next_sibling.get(node);
            if arena.flags.get(node) & (EnumAstNodeStatus::Invalid | EnumAstNodeStatus::ValidationFailed) != 0
                || arena.data_types.get(node) != EnumToken::DeclarationNodeType
                || transform_kind(arena, arena.name.get(node)) & TransformKind::KindMask != TransformKind::Property
            {
                node = next;
                continue;
            }
            // created with the first transform declaration of the minification
            let memo = memo.get_or_insert_with(|| context.get_or_insert_with("transforms", || Rc::new(RefCell::new(StructuralMemo::<Outcome>::default()))).clone());
            let slot = memo.borrow_mut().list(arena, arena.first_token.get(node), 0);
            let outcome = memo.borrow().get(slot).cloned();
            if let Some(Outcome::Tokens(template)) = &outcome {
                let tokens: Vec<u32> = template.iter().map(|&t| arena.clone_node_deep(t, true, None)).collect();
                arena.set_token_list(node, &tokens);
                node = arena.next_sibling.get(node);
                continue;
            }
            let mut children: Vec<u32> = arena
                .tokens(node)
                .into_iter()
                .map(|child| if arena.data_types.get(child) == EnumToken::TransformFunctionTokenType { minify_transform_functions(arena, child) } else { child })
                .collect();
            match outcome {
                Some(Outcome::Failed) => {
                    arena.set_token_list(node, &children);
                    node = arena.next_sibling.get(node);
                    continue;
                }
                Some(Outcome::Functions) => {
                    filter_values(arena, &mut children);
                    arena.set_token_list(node, &children);
                    node = arena.next_sibling.get(node);
                    continue;
                }
                _ => {}
            }
            let Some(computed) = compute(arena, &children) else {
                memo.borrow_mut().set(slot, Outcome::Failed);
                arena.set_token_list(node, &children);
                node = arena.next_sibling.get(node);
                continue;
            };
            // the shortest form: the functions minified one by one, the cumulative list, the minified matrix
            let length = render_value_default(arena, computed.matrix).encode_utf16().count();
            let mut result = Choice::Matrix;
            // `filterValues()` filters the functions in place
            filter_values(arena, &mut children);
            if rendered_length(arena, &children) < length {
                result = Choice::Children;
            }
            if eq_matrix(arena, &computed.values, &computed.cumulative) && rendered_length(arena, &computed.cumulative) < length {
                result = Choice::Cumulative;
            }
            if eq_matrix(arena, &computed.values, &computed.minified) && rendered_length(arena, &computed.minified) < length {
                result = Choice::Minified;
            }
            let tokens: Vec<u32> = match result {
                Choice::Matrix => vec![computed.matrix],
                Choice::Children => children,
                Choice::Cumulative => computed.cumulative.clone(),
                Choice::Minified => computed.minified.clone(),
            };
            let outcome = match result {
                Choice::Children => Outcome::Functions,
                Choice::Cumulative if !computed.created => Outcome::Uncached,
                // detached copies: the tokens of the declaration may be modified by the next features
                _ => Outcome::Tokens(tokens.iter().map(|&t| arena.clone_node_deep(t, true, None)).collect()),
            };
            memo.borrow_mut().set(slot, outcome);
            arena.set_token_list(node, &tokens);
            node = arena.next_sibling.get(node);
        }
        FeatureResult::None
    }
}
