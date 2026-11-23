//! Port of `src/ast/arena-node.ts`: object view of an arena node or token.
//!
//! The properties (`typ`, `nam`, `sel`, `val`, `chi`, `tokens`, `unit`, `cal`, `attr`, `kin`, `sign`, `op`, `op1`,
//! `op2`, `l`, `r`, `loc`, `state`, `errors`, `parent`) are read and written in the arena: nothing is copied. The
//! javascript `AstArenaNode` class forwards each getter / setter to [`AstArenaNode::get`] / [`AstArenaNode::set`]
//! and wraps the node ids it receives in views, lazily.

use super::arena::{is_node_object_type, is_numeric_token_type, structured_fields, AstArena};
use super::types::{ColorType, EnumAstNodeStatus, EnumToken};
use crate::plain::Plain;
use crate::types::{ErrorDescription, SourceLocation};
use std::rc::Rc;

/// keyed symbol names (`Symbol.for()`) of the view properties
pub const ARENAID: &str = "@tbela99/css-parser/ARENAID";
pub const ARENA: &str = "@tbela99/css-parser/ARENA";

/// value read from a view
#[derive(Clone, Debug, PartialEq)]
pub enum NodeValue {
    Undefined,
    Null,
    Str(Rc<str>),
    Num(f64),
    /// a view of a node / token
    Node(u32),
    /// views of a list, frozen
    Nodes(Vec<u32>),
    Location(SourceLocation),
    Errors(Vec<ErrorDescription>),
}

/// value assigned to a view property (`AstArenaNodeInput`, strings, numbers)
#[derive(Clone, Debug)]
pub enum NodeInput {
    Null,
    Str(Rc<str>),
    Num(f64),
    /// node id of the arena (views of the arena are passed as ids)
    Id(u32),
    /// a plain object (or a view of another arena exported as a plain object)
    Plain(Plain),
    List(Vec<NodeInput>),
    Location(Option<SourceLocation>),
    Errors(Option<Vec<ErrorDescription>>),
}

/// node types whose value (`val`) is the token list: declarations and css module variables
fn has_value_tokens(typ: u8) -> bool {
    matches!(
        typ,
        EnumToken::DeclarationNodeType
            | EnumToken::CssVariableTokenType
            | EnumToken::CssVariableImportTokenType
            | EnumToken::CssVariableDeclarationMapTokenType
    )
}

/// convert setter values to node ids of the arena: ids are used as is, plain objects are allocated
pub fn to_ids(arena: &AstArena, values: &NodeInput) -> Vec<u32> {
    match values {
        NodeInput::Null => Vec::new(),
        NodeInput::List(list) => list.iter().flat_map(|v| to_ids(arena, v)).collect(),
        NodeInput::Id(id) => vec![*id],
        NodeInput::Num(n) => vec![*n as u32],
        NodeInput::Plain(p) => vec![arena.from_object(p)],
        _ => Vec::new(),
    }
}

/// ids of a linked list
fn list_views(arena: &AstArena, first: u32) -> NodeValue {
    let mut result = Vec::new();
    let mut id = first;
    while id != 0 {
        result.push(id);
        id = arena.next_sibling.get(id);
    }
    NodeValue::Nodes(result)
}

/// object view of an arena node or token
pub struct AstArenaNode<'a> {
    pub arena: &'a AstArena,
    pub id: u32,
}

impl<'a> AstArenaNode<'a> {
    pub fn new(arena: &'a AstArena, id: u32) -> Self {
        Self { arena, id }
    }

    /// read a property
    pub fn get(&self, key: &str) -> NodeValue {
        match key {
            "typ" => NodeValue::Num(self.arena.data_types.get(self.id) as f64),
            "nam" => self.nam(),
            "sel" => self.sel(),
            "val" => self.val(),
            "chi" => self.chi(),
            "tokens" => self.tokens(),
            "unit" => self.unit(),
            "cal" => self.unit_of(EnumToken::ColorTokenType),
            "attr" => self.unit_of(EnumToken::MatchExpressionTokenType),
            "kin" => self.kin(),
            "sign" => self.sign(),
            "op" => self.op(),
            "op1" | "op2" => self.get_field(key),
            "l" => self.lr(true),
            "r" => self.lr(false),
            "loc" => self.arena.get_location(self.id).map_or(NodeValue::Null, NodeValue::Location),
            "state" => NodeValue::Num((self.arena.flags.get(self.id) & EnumAstNodeStatus::StateMask) as f64),
            "errors" => self.arena.errors.borrow().get(&self.id).map_or(NodeValue::Null, |e| NodeValue::Errors(e.clone())),
            "parent" => {
                let parent = self.arena.get_value_parent(self.id);
                if parent == 0 {
                    NodeValue::Null
                } else {
                    NodeValue::Node(parent)
                }
            }
            _ => NodeValue::Undefined,
        }
    }

    /// write a property. returns false when the property is not a view property
    pub fn set(&self, key: &str, value: NodeInput) -> bool {
        let arena = self.arena;
        let id = self.id;
        match key {
            "typ" => {
                if let NodeInput::Num(n) = value {
                    arena.data_types.set(id, n as u8);
                }
            }
            "nam" | "sel" => {
                if let NodeInput::Str(s) = &value {
                    arena.name.set(id, arena.intern_rc(s));
                }
            }
            "val" => self.set_val(value),
            "chi" => self.set_chi(value),
            "tokens" => arena.set_token_list(id, &to_ids(arena, &value)),
            "unit" | "cal" | "attr" => match &value {
                NodeInput::Str(s) => arena.set_unit_string(id, Some(s)),
                _ => arena.set_unit_string(id, None),
            },
            "kin" => arena.color_type.set(id, if let NodeInput::Num(n) = value { n as u8 } else { ColorType::NONE }),
            "sign" => {
                if let NodeInput::Num(n) = value {
                    arena.numeric_value.set(id, n);
                }
            }
            "op" => {
                if arena.data_types.get(id) == EnumToken::BinaryExpressionTokenType {
                    if let NodeInput::Num(n) = value {
                        arena.numeric_value.set(id, n);
                    }
                } else {
                    self.set_field("op", &value);
                }
            }
            "op1" | "op2" => {
                self.set_field(key, &value);
            }
            "l" => self.set_lr(true, value),
            "r" => self.set_lr(false, value),
            "loc" => {
                let loc = match value {
                    NodeInput::Location(loc) => loc,
                    _ => None,
                };
                arena.set_location(id, loc.map(|l| l.src_id), loc.map(|l| l.sta), loc.map(|l| l.end));
            }
            "state" => arena.set_state(id, if let NodeInput::Num(n) = value { n as u32 } else { 0 }),
            "errors" => arena.set_errors(id, if let NodeInput::Errors(e) = value { e } else { None }),
            _ => return false,
        }
        true
    }

    fn nam(&self) -> NodeValue {
        let arena = self.arena;
        let id = self.id;
        let typ = arena.data_types.get(id);
        if is_node_object_type(typ) {
            return if typ == EnumToken::AtRuleNodeType || typ == EnumToken::KeyframesAtRuleNodeType || has_value_tokens(typ) {
                NodeValue::Str(arena.resolve_string(arena.name.get(id)).into())
            } else {
                NodeValue::Undefined
            };
        }
        if arena.name.get(id) == 0 {
            NodeValue::Undefined
        } else {
            NodeValue::Str(arena.resolve_string(arena.name.get(id)).into())
        }
    }

    fn sel(&self) -> NodeValue {
        let typ = self.arena.data_types.get(self.id);
        if typ == EnumToken::RuleNodeType || typ == EnumToken::KeyframesRuleNodeType {
            NodeValue::Str(self.arena.resolve_string(self.arena.name.get(self.id)).into())
        } else {
            NodeValue::Undefined
        }
    }

    fn val(&self) -> NodeValue {
        let arena = self.arena;
        let id = self.id;
        let typ = arena.data_types.get(id);
        if is_node_object_type(typ) {
            if has_value_tokens(typ) {
                return list_views(arena, arena.first_token.get(id));
            }
            return if matches!(typ, EnumToken::StyleSheetNodeType | EnumToken::RuleNodeType | EnumToken::KeyframesRuleNodeType) {
                NodeValue::Undefined
            } else {
                NodeValue::Str(arena.resolve_string(arena.string_value.get(id)).into())
            };
        }
        if typ == EnumToken::BinaryExpressionTokenType || typ == EnumToken::FractionTokenType {
            return NodeValue::Undefined;
        }
        if typ == EnumToken::UnaryExpressionTokenType {
            let child = arena.first_child.get(id);
            return if child == 0 { NodeValue::Null } else { NodeValue::Node(child) };
        }
        if is_numeric_token_type(typ) {
            let fraction = arena.get_fraction(id);
            return if fraction == 0 { NodeValue::Num(arena.numeric_value.get(id)) } else { NodeValue::Node(fraction) };
        }
        let field = self.get_field("val");
        if field != NodeValue::Undefined {
            return field;
        }
        if arena.string_value.get(id) != 0 || typ == EnumToken::IdenTokenType || typ == EnumToken::StringTokenType {
            NodeValue::Str(arena.resolve_string(arena.string_value.get(id)).into())
        } else {
            NodeValue::Undefined
        }
    }

    fn set_val(&self, val: NodeInput) {
        let arena = self.arena;
        let id = self.id;
        let typ = arena.data_types.get(id);
        if is_node_object_type(typ) && has_value_tokens(typ) {
            arena.set_token_list(id, &to_ids(arena, &val));
            return;
        }
        match &val {
            NodeInput::Str(s) => {
                if is_numeric_token_type(typ) {
                    arena.numeric_value.set(id, crate::utils::number::js_parse_number(s));
                    arena.set_fraction(id, None);
                } else {
                    arena.string_value.set(id, arena.intern_rc(s));
                }
                return;
            }
            NodeInput::Num(n) => {
                arena.numeric_value.set(id, *n);
                if is_numeric_token_type(typ) {
                    arena.set_fraction(id, None);
                }
                return;
            }
            _ => {}
        }
        if typ == EnumToken::UnaryExpressionTokenType {
            arena.set_child_list(id, &to_ids(arena, &val));
        } else if is_numeric_token_type(typ) {
            let fraction = match val {
                NodeInput::Null => None,
                _ => to_ids(arena, &val).first().copied(),
            };
            arena.set_fraction(id, fraction);
        } else if !self.set_field("val", &val) {
            arena.string_value.set(id, 0);
        }
    }

    fn chi(&self) -> NodeValue {
        let arena = self.arena;
        let id = self.id;
        if arena.flags.get(id) & EnumAstNodeStatus::HasChildren != 0 {
            return list_views(arena, arena.first_child.get(id));
        }
        if is_node_object_type(arena.data_types.get(id)) || arena.first_token.get(id) == 0 {
            return NodeValue::Undefined;
        }
        list_views(arena, arena.first_token.get(id))
    }

    fn set_chi(&self, chi: NodeInput) {
        let arena = self.arena;
        let id = self.id;
        let typ = arena.data_types.get(id);
        if matches!(
            typ,
            EnumToken::StyleSheetNodeType
                | EnumToken::RuleNodeType
                | EnumToken::KeyframesRuleNodeType
                | EnumToken::AtRuleNodeType
                | EnumToken::KeyframesAtRuleNodeType
        ) {
            if matches!(chi, NodeInput::Null) {
                arena.set_child_list(id, &[]);
                if typ == EnumToken::AtRuleNodeType || typ == EnumToken::KeyframesAtRuleNodeType {
                    arena.flags.set(id, arena.flags.get(id) & !EnumAstNodeStatus::HasChildren);
                }
                return;
            }
            arena.flags.set(id, arena.flags.get(id) | EnumAstNodeStatus::HasChildren);
            arena.set_child_list(id, &to_ids(arena, &chi));
            return;
        }
        arena.set_token_list(id, &to_ids(arena, &chi));
    }

    fn tokens(&self) -> NodeValue {
        let arena = self.arena;
        let id = self.id;
        if arena.flags.get(id) & EnumAstNodeStatus::HasValues != 0 || arena.first_token.get(id) == 0 {
            NodeValue::Null
        } else {
            list_views(arena, arena.first_token.get(id))
        }
    }

    fn unit(&self) -> NodeValue {
        let arena = self.arena;
        let id = self.id;
        let typ = arena.data_types.get(id);
        if arena.unit.get(id) == 0 || typ == EnumToken::ColorTokenType || typ == EnumToken::MatchExpressionTokenType {
            NodeValue::Undefined
        } else {
            NodeValue::Str(arena.resolve_string(arena.unit.get(id)).into())
        }
    }

    /// `cal` of a color, `attr` of a match expression
    fn unit_of(&self, typ: u8) -> NodeValue {
        let arena = self.arena;
        if arena.unit.get(self.id) == 0 || arena.data_types.get(self.id) != typ {
            NodeValue::Undefined
        } else {
            NodeValue::Str(arena.resolve_string(arena.unit.get(self.id)).into())
        }
    }

    fn kin(&self) -> NodeValue {
        let arena = self.arena;
        if arena.data_types.get(self.id) != EnumToken::ColorTokenType || arena.color_type.get(self.id) == ColorType::NONE {
            NodeValue::Undefined
        } else {
            NodeValue::Num(arena.color_type.get(self.id) as f64)
        }
    }

    fn sign(&self) -> NodeValue {
        if self.arena.data_types.get(self.id) == EnumToken::UnaryExpressionTokenType {
            NodeValue::Num(self.arena.numeric_value.get(self.id))
        } else {
            NodeValue::Undefined
        }
    }

    fn op(&self) -> NodeValue {
        if self.arena.data_types.get(self.id) == EnumToken::BinaryExpressionTokenType {
            NodeValue::Num(self.arena.numeric_value.get(self.id))
        } else {
            self.get_field("op")
        }
    }

    fn lr(&self, left: bool) -> NodeValue {
        let arena = self.arena;
        if arena.data_types.get(self.id) == EnumToken::FractionTokenType {
            let child = if left { arena.first_child.get(self.id) } else { arena.last_child.get(self.id) };
            return if child == 0 { NodeValue::Null } else { NodeValue::Node(child) };
        }
        self.get_field(if left { "l" } else { "r" })
    }

    fn set_lr(&self, left: bool, value: NodeInput) {
        let arena = self.arena;
        let id = self.id;
        if arena.data_types.get(id) == EnumToken::FractionTokenType {
            let mut values = to_ids(arena, &value);
            if left {
                let r = arena.last_child.get(id);
                if r != 0 && r != arena.first_child.get(id) {
                    values.push(r);
                }
            } else {
                let l = arena.first_child.get(id);
                if l != 0 {
                    values.insert(0, l);
                }
            }
            arena.set_child_list(id, &values);
            return;
        }
        self.set_field(if left { "l" } else { "r" }, &value);
    }

    /// value of a structured token field. undefined when the token has no such field
    fn get_field(&self, key: &str) -> NodeValue {
        let arena = self.arena;
        for (name, field, single) in structured_fields(arena.data_types.get(self.id)) {
            if name != key {
                continue;
            }
            let Some(list) = arena.get_child_at(self.id, field) else {
                return NodeValue::Undefined;
            };
            if single {
                let first = arena.first_token.get(list);
                return if first == 0 { NodeValue::Null } else { NodeValue::Node(first) };
            }
            return list_views(arena, arena.first_token.get(list));
        }
        NodeValue::Undefined
    }

    /// set the value of a structured token field. false when the token has no such field
    fn set_field(&self, key: &str, value: &NodeInput) -> bool {
        let arena = self.arena;
        for (name, field, _) in structured_fields(arena.data_types.get(self.id)) {
            if name == key {
                arena.set_token_list(arena.field_list(self.id, field), &to_ids(arena, value));
                return true;
            }
        }
        false
    }

    /// deep plain copy of the node
    pub fn to_json(&self) -> Plain {
        self.arena.to_plain_object(self.id)
    }
}
