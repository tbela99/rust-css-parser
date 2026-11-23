//! Port of `src/ast/features/shorthand.ts`.

use super::r#type::FeatureWalkMode;
use super::{FeatureResult, MinifyFeature, SharedFeature};
use crate::ast::arena::AstArena;
use crate::ast::minify::MinifyContext;
use crate::ast::mixins::utils::move_result;
use crate::ast::types::{EnumAstNodeStatus, EnumToken};
use crate::options::ParserOptions;
use crate::parser::declaration::list::{DeclarationMemo, PropertyList};
use crate::validation::config::get_syntax_rule;
use crate::validation::parser::typedef::ValidationSyntaxGroupEnum;
use std::cell::RefCell;
use std::rc::Rc;

pub struct ComputeShorthandFeature;

const ACCEPT: [u8; 3] = [EnumToken::RuleNodeType, EnumToken::AtRuleNodeType, EnumToken::KeyframesRuleNodeType];

impl ComputeShorthandFeature {
    pub fn register(options: &ParserOptions, features: &mut Vec<SharedFeature>) {
        if options.compute_shorthand == Some(true) {
            features.push(Rc::new(RefCell::new(ComputeShorthandFeature)));
        }
    }
}

impl MinifyFeature for ComputeShorthandFeature {
    fn accept(&self) -> Option<&[u8]> {
        Some(&ACCEPT)
    }

    fn ordering(&self) -> i32 {
        5
    }

    fn process_mode(&self) -> u32 {
        FeatureWalkMode::Post
    }

    fn run(&mut self, arena: &AstArena, ast: u32, options: &ParserOptions, _parent: u32, context: &mut MinifyContext, _mode: u32) -> FeatureResult {
        if arena.flags.get(ast) & EnumAstNodeStatus::HasChildren == 0 || arena.first_child.get(ast) == 0 {
            return FeatureResult::None;
        }
        let t = |x: u32| arena.data_types.get(x);
        // the keys of the declarations are shared by the rules of the minification
        let memo: Rc<RefCell<DeclarationMemo>> = context.get_or_insert_with("declarations", || Rc::new(RefCell::new(DeclarationMemo::default()))).clone();
        let mut do_not_sort = false;
        let mut descriptors = None;
        let mut is_function = false;
        if t(ast) == EnumToken::AtRuleNodeType {
            match &*arena.resolve_string(arena.name.get(ast)) {
                "function" => {
                    do_not_sort = true;
                    is_function = true;
                    descriptors = get_syntax_rule(ValidationSyntaxGroupEnum::AtRules, "@function").and_then(|r| r.get_property_descriptors());
                }
                "mixin" => do_not_sort = true,
                _ => {}
            }
        }
        // the children are processed by segments: an at-rule without a block or an `@apply` rule inserts declarations
        // at its position, the declarations are not moved across it
        let mut child = arena.first_child.get(ast);
        while child != 0 {
            let mut list = PropertyList::new(options, Some(memo.clone()), descriptors.clone());
            let mut has_declarations = false;
            // first node of the segment that is not moved into the property list
            let mut first = 0u32;
            // at-rule without a block that ends the segment
            let mut boundary = 0u32;
            while child != 0 {
                let mut last = child;
                if !do_not_sort {
                    // capture comments with the next node
                    while arena.next_sibling.get(last) != 0 && t(last) == EnumToken::CommentNodeType {
                        last = arena.next_sibling.get(last);
                    }
                }
                let next = arena.next_sibling.get(last);
                if t(last) == EnumToken::AtRuleNodeType
                    && (arena.flags.get(last) & EnumAstNodeStatus::HasChildren == 0 || arena.string_equals(arena.name.get(last), "apply"))
                {
                    boundary = child;
                    child = next;
                    break;
                }
                if do_not_sort || t(last) == EnumToken::DeclarationNodeType {
                    has_declarations = true;
                    let mut node = child;
                    while node != next {
                        let next_node = arena.next_sibling.get(node);
                        arena.detach(node);
                        list.add_declaration(arena, node);
                        node = next_node;
                    }
                } else if first == 0 {
                    first = child;
                }
                child = next;
            }
            if has_declarations {
                // the declarations are inserted before the rules of the segment
                let reference = if first != 0 { first } else { boundary };
                for node in list.iterate(arena) {
                    if reference == 0 {
                        arena.append_child(ast, node);
                    } else {
                        arena.insert_before(reference, &[node]);
                    }
                }
            }
        }
        // the deduplicated result keeps the position of the first result: it is moved after the conditional rules
        if is_function {
            move_result(arena, ast);
        }
        FeatureResult::Node(ast)
    }
}
