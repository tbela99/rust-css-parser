//! Port of `src/ast/mixins/utils.ts`: helpers shared by the minification features.

use crate::ast::arena::AstArena;
use crate::ast::types::EnumToken;

/// true if the node is a `@mixin` or a `@function` rule, or is located inside one of them. the body of a mixin or a
/// custom function is a template: it is evaluated when the mixin is applied or the function is called
pub fn is_in_template(arena: &AstArena, node: u32) -> bool {
    let mut node = node;
    while node != 0 {
        if arena.data_types.get(node) == EnumToken::AtRuleNodeType
            && (arena.string_equals(arena.name.get(node), "mixin") || arena.string_equals(arena.name.get(node), "function"))
        {
            return true;
        }
        node = arena.parents.get(node);
    }
    false
}

/// move the last `result` descriptor of a custom function after its conditional rules: there is no early return.
/// `result: 16px; @media (...) { result: 20px }` -> `@media (...) { result: 20px } result: 16px`
pub fn move_result(arena: &AstArena, node: u32) {
    let mut last = arena.last_child.get(node);
    // the last result
    while last != 0 && !(arena.data_types.get(last) == EnumToken::DeclarationNodeType && arena.string_equals(arena.name.get(last), "result")) {
        last = arena.previous_sibling.get(last);
    }
    if last == 0 {
        return;
    }
    let mut next = arena.next_sibling.get(last);
    while next != 0 {
        if arena.data_types.get(next) == EnumToken::AtRuleNodeType {
            arena.detach(last);
            arena.append_child(node, last);
            return;
        }
        next = arena.next_sibling.get(next);
    }
}
