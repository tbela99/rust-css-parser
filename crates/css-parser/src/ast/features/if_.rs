//! Port of `src/ast/features/if.ts`: `if()` conditions expanded into declarations and at-rules.

use super::r#type::FeatureWalkMode;
use super::{FeatureResult, MinifyFeature, SharedFeature};
use crate::ast::arena::{Arena, AstArena, StrOrId, TokenField};
use crate::ast::minify::MinifyContext;
use crate::ast::types::EnumToken;
use crate::options::{ConvertColor, ParserOptions};
use crate::parser::utils::eq::eq_token;
use crate::printer::render::render_value;
use crate::validation::match_::trim_array;
use std::cell::RefCell;
use crate::utils::fx::HashMap;
use std::rc::Rc;

/// true if the token is an if() function or an if condition
fn is_if_token(arena: &Arena, value: u32) -> bool {
    arena.data_types.get(value) == EnumToken::IfConditionTokenType
        || (arena.data_types.get(value) == EnumToken::WildCardFunctionTokenType && arena.string_equals_ignore_case(arena.string_value.get(value), "if"))
}

/// deep clone a declaration. the clone map is filled with the cloned value tokens
fn clone_declaration(arena: &AstArena, declaration: u32, node_map: &mut HashMap<u32, u32>) -> u32 {
    node_map.clear();
    arena.clone_node_deep(declaration, true, Some(node_map))
}

/// create an at-rule wrapping a declaration. the location is inherited from the declaration parent
fn create_at_rule(arena: &AstArena, declaration: u32, parent: u32, nam: &str, tokens: &[u32], options: &ParserOptions) -> u32 {
    let value: String = tokens.iter().map(|&t| render_value(arena, t, options)).collect();
    let at_rule = arena.create_at_rule(EnumToken::AtRuleNodeType, StrOrId::Str(nam), StrOrId::Str(&value), true, Some(tokens), 0, None);
    let parent = if arena.parents.get(declaration) != 0 { arena.parents.get(declaration) } else { parent };
    if parent != 0 {
        arena.copy_location(at_rule, parent);
    }
    at_rule
}

/// first token of the left side of an if condition that is not a comment or a whitespace
fn condition_left(arena: &AstArena, condition: u32) -> Option<u32> {
    arena
        .field_tokens(condition, TokenField::L)
        .into_iter()
        .find(|&t| !matches!(arena.data_types.get(t), EnumToken::CommentTokenType | EnumToken::WhitespaceTokenType))
}

/// value tokens of an if condition (right side), mapped to their clones. the trailing semicolon is removed
fn condition_value(arena: &AstArena, condition: u32, node_map: &HashMap<u32, u32>) -> Vec<u32> {
    let mut tokens = arena.field_tokens(condition, TokenField::R);
    if tokens.last().is_some_and(|&l| arena.data_types.get(l) == EnumToken::SemiColonTokenType) {
        tokens.pop();
        trim_array(arena, &mut tokens);
    }
    tokens.iter().map(|t| node_map.get(t).copied().unwrap_or(0)).collect()
}

/// `arena.replaceToken(target, ...tokens)`: an unmapped target (`undefined`) is ignored
fn replace(arena: &AstArena, target: u32, tokens: &[u32]) {
    if target != 0 {
        arena.replace_token(target, tokens);
    }
}

fn is_else(arena: &AstArena, token: u32) -> bool {
    arena.data_types.get(token) == EnumToken::IdenTokenType && arena.string_equals_ignore_case(arena.string_value.get(token), "else")
}

fn substitute_if_else_node(arena: &AstArena, declaration: u32, node: u32, wrapper: u32, parent_wrapper: u32, parent: u32) -> Vec<u32> {
    let mut result: Vec<u32> = Vec::new();
    let mut node_map: HashMap<u32, u32> = HashMap::default();
    // map a token or the declaration itself to its clone
    let mapped = |target: u32, cloned: u32, map: &HashMap<u32, u32>| if target == declaration { cloned } else { map.get(&target).copied().unwrap_or(0) };
    let typ = arena.data_types.get(node);
    if typ == EnumToken::IfElseConditionTokenType {
        let mut cloned = clone_declaration(arena, declaration, &mut node_map);
        let else_condition = arena.field(node, TokenField::R);
        // replace else: ... with the actual value
        if arena.data_types.get(else_condition) == EnumToken::IfConditionTokenType {
            if let Some(target) = condition_left(arena, else_condition).filter(|&t| is_else(arena, t)) {
                let _ = target;
                let value = condition_value(arena, else_condition, &node_map);
                replace(arena, mapped(wrapper, cloned, &node_map), &value);
                if parent_wrapper != declaration {
                    let siblings = arena.tokens(parent_wrapper);
                    if let Some(index) = siblings.iter().position(|&s| s == wrapper) {
                        let left = condition_left(arena, arena.field(node, TokenField::L));
                        for &sibling in &siblings[index + 1..] {
                            if arena.data_types.get(sibling) != arena.data_types.get(wrapper) {
                                continue;
                            }
                            for child in arena.tokens(sibling) {
                                if arena.data_types.get(child) != EnumToken::IfElseConditionTokenType {
                                    continue;
                                }
                                let left_side = condition_left(arena, arena.field(child, TokenField::L));
                                if let (Some(l), Some(ls)) = (left, left_side) {
                                    if eq_token(arena, l, ls) {
                                        let value = condition_value(arena, arena.field(child, TokenField::R), &node_map);
                                        replace(arena, mapped(sibling, cloned, &node_map), &value);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        result.push(cloned);
        cloned = clone_declaration(arena, declaration, &mut node_map);
        let l = mapped(arena.field(node, TokenField::L), cloned, &node_map);
        replace(arena, mapped(wrapper, cloned, &node_map), &[l]);
        result.push(cloned);
    } else if typ == EnumToken::IfConditionTokenType {
        let Some(left) = condition_left(arena, node) else {
            return result;
        };
        let left_type = arena.data_types.get(left);
        if is_else(arena, left) {
            let cloned = clone_declaration(arena, declaration, &mut node_map);
            let target = if wrapper == declaration { node } else { wrapper };
            let value = condition_value(arena, node, &node_map);
            replace(arena, mapped(target, cloned, &node_map), &value);
            result.push(cloned);
        } else if left_type == EnumToken::WhenElseFunctionTokenType {
            let name = arena.resolve_string(arena.string_value.get(left));
            let minify = name != "supports";
            let options = ParserOptions { minify: Some(minify), convert_color: Some(ConvertColor::Bool(minify)), ..Default::default() };
            let inner: Vec<u32> = arena.tokens(left).into_iter().map(|t| arena.clone_node_deep(t, true, None)).collect();
            let parens = arena.create_list(EnumToken::ParensTokenType, Some(&inner));
            let at_rule = create_at_rule(arena, declaration, parent, &name, &[parens], &options);
            let cloned = clone_declaration(arena, declaration, &mut node_map);
            let value = condition_value(arena, node, &node_map);
            replace(arena, mapped(node, cloned, &node_map), &value);
            arena.append_child_list(at_rule, &[cloned]);
            result.push(at_rule);
            process_node(arena, cloned, parent);
        } else if left_type == EnumToken::ContainerFunctionTokenType {
            let left_clone = arena.clone_node_deep(left, true, None);
            let at_rule = create_at_rule(arena, declaration, parent, "container", &[left_clone], &ParserOptions::default());
            let cloned = clone_declaration(arena, declaration, &mut node_map);
            let is_wild_card = wrapper != declaration && arena.data_types.get(wrapper) == EnumToken::WildCardFunctionTokenType;
            let value = condition_value(arena, node, &node_map);
            replace(arena, mapped(if is_wild_card { wrapper } else { node }, cloned, &node_map), &value);
            arena.append_child_list(at_rule, &[cloned]);
            result.push(at_rule);
            process_node(arena, cloned, parent);
        }
    }
    result
}

/// expand the if() conditions of a declaration into declarations / at-rules
fn process_node(arena: &AstArena, declaration_node: u32, parent: u32) -> Vec<u32> {
    let mut result: Vec<u32> = Vec::new();
    let mut stack: Vec<u32> = vec![declaration_node];
    // the generated nodes are detached: the parent gives the location of the generated at-rules
    let parent = if arena.parents.get(declaration_node) != 0 { arena.parents.get(declaration_node) } else { parent };
    let mut k = 0;
    while k < stack.len() {
        let ast_node = stack[k];
        k += 1;
        let Some((declaration, value)) = arena.find_by_value(ast_node, is_if_token) else {
            result.push(ast_node);
            continue;
        };
        // the parent of a detached token is the declaration
        let parent_wrapper = match arena.get_value_parent(value) {
            0 => declaration,
            p => p,
        };
        let grand_parent = if parent_wrapper == declaration {
            declaration
        } else {
            match arena.get_value_parent(parent_wrapper) {
                0 => declaration,
                p => p,
            }
        };
        if arena.data_types.get(value) == EnumToken::WildCardFunctionTokenType {
            for condition in arena.tokens(value) {
                stack.extend(substitute_if_else_node(arena, declaration, condition, value, parent_wrapper, parent));
            }
        } else {
            stack.extend(substitute_if_else_node(arena, declaration, value, parent_wrapper, grand_parent, parent));
        }
        // the node is substituted: release it unless it is the node being replaced
        if ast_node != declaration_node {
            arena.detach(ast_node);
        }
    }
    if !result.is_empty() && arena.parents.get(declaration_node) != 0 {
        arena.replace_child(declaration_node, &result);
        if !result.contains(&declaration_node) {
            arena.detach(declaration_node);
        }
    }
    result
}

pub struct ExpandIfFeature;

const ACCEPT: [u8; 1] = [EnumToken::DeclarationNodeType];

impl ExpandIfFeature {
    pub fn register(options: &ParserOptions, features: &mut Vec<SharedFeature>) {
        if options.expand_if_syntax == Some(true) {
            features.push(Rc::new(RefCell::new(ExpandIfFeature)));
        }
    }
}

impl MinifyFeature for ExpandIfFeature {
    fn accept(&self) -> Option<&[u8]> {
        Some(&ACCEPT)
    }

    fn ordering(&self) -> i32 {
        1
    }

    fn process_mode(&self) -> u32 {
        FeatureWalkMode::Pre
    }

    fn run(&mut self, arena: &AstArena, declaration: u32, _options: &ParserOptions, _parent: u32, _context: &mut MinifyContext, _mode: u32) -> FeatureResult {
        FeatureResult::Nodes(process_node(arena, declaration, 0))
    }
}
