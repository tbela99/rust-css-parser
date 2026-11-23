//! Port of `src/ast/features/index.ts` and of the `MinifyFeature` interface.
//!
//! A feature is registered (`static register(options)`) into the feature list of a minification when its option is
//! set: the list holds shared instances (`Rc<RefCell<..>>`), a feature keeps its state between the nodes.

pub mod calc;
#[path = "if_.rs"]
pub mod r#if;
pub mod inlinecssvariables;
pub mod mixin;
pub mod prefix;
pub mod shorthand;
pub mod transform;
pub mod r#type;

use crate::ast::arena::AstArena;
use crate::ast::minify::MinifyContext;
use crate::options::ParserOptions;
use std::cell::RefCell;
use std::rc::Rc;

/// result of [`MinifyFeature::run`]: the replacement of the node
pub enum FeatureResult {
    /// `null`: the node is kept
    None,
    Node(u32),
    Nodes(Vec<u32>),
}

/// minification feature (`MinifyFeature`)
pub trait MinifyFeature {
    /// accepted node types, `None`: all
    fn accept(&self) -> Option<&[u8]> {
        None
    }
    fn ordering(&self) -> i32;
    fn process_mode(&self) -> u32;
    fn run(&mut self, arena: &AstArena, ast: u32, options: &ParserOptions, parent: u32, context: &mut MinifyContext, mode: u32) -> FeatureResult;
    /// true if the feature has a `cleanup()` method
    fn has_cleanup(&self) -> bool {
        false
    }
    fn cleanup(&mut self, _arena: &AstArena, _ast: u32, _options: &ParserOptions, _context: &mut MinifyContext, _mode: u32) {}
}

/// a registered feature
pub type SharedFeature = Rc<RefCell<dyn MinifyFeature>>;

/// register the features enabled by the options, in the order of the exports of `features/index.ts` (the classes
/// have no static ordering: the typescript sort keeps that order).
pub fn register_features(options: &ParserOptions, features: &mut Vec<SharedFeature>) {
    prefix::ComputePrefixFeature::register(options, features);
    mixin::EvaluateMixinFeature::register(options, features);
    inlinecssvariables::InlineCssVariablesFeature::register(options, features);
    shorthand::ComputeShorthandFeature::register(options, features);
    calc::ComputeCalcExpressionFeature::register(options, features);
    transform::TransformCssFeature::register(options, features);
    r#if::ExpandIfFeature::register(options, features);
}
