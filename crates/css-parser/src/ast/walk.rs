//! Port of `src/ast/walk.ts`: iterators of [`Arena::walk`] and [`Arena::walk_values`].
//!
//! The walkers do not borrow the arena: `next()` takes it. the children of the node returned by the last call are
//! read by the next call, the caller may modify the node in between.

use super::arena::Arena;
use crate::utils::pool::PooledVec;
use super::types::{EnumAstNodeStatus, EnumToken, WalkerEvent, WalkerOptionEnum};

/// value returned by a walker filter: `WalkerOption` (a [`WalkerOptionEnum`] combination or nothing)
pub type WalkerOption = Option<u32>;

/// filter of [`Arena::walk`]
pub type WalkerFilter<'f> = Box<dyn FnMut(&Arena, u32) -> WalkerOption + 'f>;

/// value returned by a value walker filter: an option or token ids to walk next
#[derive(Clone, Debug, PartialEq, Default)]
pub enum WalkerValueResult {
    #[default]
    None,
    Option(u32),
    Tokens(Vec<u32>),
}

impl From<WalkerOption> for WalkerValueResult {
    fn from(option: WalkerOption) -> Self {
        option.map_or(WalkerValueResult::None, WalkerValueResult::Option)
    }
}

/// filter of [`Arena::walk_values`]
pub type WalkerValueFilter<'f> = Box<dyn FnMut(&Arena, u32, u32) -> WalkerValueResult + 'f>;

/// token type filter of [`WalkerValueOptions`]
pub enum WalkerValueType<'f> {
    Type(u8),
    Types(Vec<u8>),
    Fn(Box<dyn FnMut(&Arena, u32) -> bool + 'f>),
}

/// options of [`Arena::walk`]
#[derive(Default)]
pub struct WalkerOptions<'f> {
    pub reverse: bool,
    pub children: Option<bool>,
    pub attributes: bool,
    pub filter: Option<WalkerFilter<'f>>,
}

/// options of [`Arena::walk_values`]
#[derive(Default)]
pub struct WalkerValueOptions<'f> {
    pub event: Option<u32>,
    pub fn_: Option<WalkerValueFilter<'f>>,
    pub type_: Option<WalkerValueType<'f>>,
}

/// push the direct value tokens of a token onto a stack: the token list, then the fields of a structured token.
/// pushed in order, then reversed in place (unless reverse) so that they are popped in order
pub fn push_values(arena: &Arena, stack: &mut Vec<u32>, value: u32, reverse: bool) {
    let first = stack.len();
    let mut token = arena.first_token.get(value);
    while token != 0 {
        stack.push(token);
        token = arena.next_sibling.get(token);
    }
    if arena.flags.get(value) & EnumAstNodeStatus::HasChildren == 0 {
        let mut child = arena.first_child.get(value);
        while child != 0 {
            if arena.data_types.get(child) == EnumToken::ListTokenType {
                token = arena.first_token.get(child);
                while token != 0 {
                    stack.push(token);
                    token = arena.next_sibling.get(token);
                }
            } else {
                stack.push(child);
            }
            child = arena.next_sibling.get(child);
        }
    }
    if !reverse {
        stack[first..].reverse();
    }
}

/// iterator of [`Arena::walk`]: node ids (and value tokens when `attributes` is set) in document order
pub struct NodeWalker<'f> {
    stack: PooledVec,
    filter: Option<WalkerFilter<'f>>,
    reverse: bool,
    children: bool,
    attributes: bool,
    current: u32,
    option: WalkerOption,
}

impl<'f> NodeWalker<'f> {
    pub fn new(node: u32, filter: Option<WalkerFilter<'f>>, reverse: bool) -> Self {
        Self { stack: PooledVec::from_slice(&[node]), filter, reverse, children: true, attributes: false, current: 0, option: None }
    }

    pub fn with_options(node: u32, options: WalkerOptions<'f>) -> Self {
        Self {
            stack: PooledVec::from_slice(&[node]),
            filter: options.filter,
            reverse: options.reverse,
            children: options.children.unwrap_or(true),
            attributes: options.attributes,
            current: 0,
            option: None,
        }
    }

    fn expand(&mut self, arena: &Arena, node: u32, option: WalkerOption) {
        if option.is_some_and(|o| o & WalkerOptionEnum::IgnoreChildren != 0) {
            return;
        }
        if self.attributes {
            push_values(arena, &mut self.stack, node, self.reverse);
        }
        if self.children && arena.flags.get(node) & EnumAstNodeStatus::HasChildren != 0 {
            if self.reverse {
                let mut child = arena.first_child.get(node);
                while child != 0 {
                    self.stack.push(child);
                    child = arena.next_sibling.get(child);
                }
            } else {
                let mut child = arena.last_child.get(node);
                while child != 0 {
                    self.stack.push(child);
                    child = arena.previous_sibling.get(child);
                }
            }
        }
    }

    pub fn next(&mut self, arena: &Arena) -> Option<u32> {
        if self.current != 0 {
            let (current, option) = (self.current, self.option);
            self.expand(arena, current, option);
            self.current = 0;
        }
        while let Some(node) = self.stack.pop() {
            let mut option = None;
            if let Some(filter) = self.filter.as_mut() {
                option = filter(arena, node);
                if let Some(o) = option {
                    if o & WalkerOptionEnum::Ignore != 0 {
                        continue;
                    }
                    if o & WalkerOptionEnum::Stop != 0 {
                        break;
                    }
                    if o & WalkerOptionEnum::Children != 0 {
                        self.expand(arena, node, option);
                        continue;
                    }
                }
            }
            self.current = node;
            self.option = option;
            return Some(node);
        }
        self.finish();
        None
    }

    /// stop the iteration
    pub fn finish(&mut self) {
        self.stack.clear();
        self.current = 0;
    }

    /// borrowing iterator
    pub fn iter<'w>(&'w mut self, arena: &'w Arena) -> impl Iterator<Item = u32> + use<'w, 'f> {
        std::iter::from_fn(move || self.next(arena))
    }
}

/// iterator of [`Arena::walk_values`]: token ids in document order
pub struct ValueWalker<'f> {
    stack: PooledVec,
    fn_: Option<WalkerValueFilter<'f>>,
    type_: Option<WalkerValueType<'f>>,
    event_type: u32,
    reverse: bool,
    current: u32,
    option: WalkerValueResult,
}

impl<'f> ValueWalker<'f> {
    /// walk the value tokens of a node / token
    pub fn of_node(arena: &Arena, node: u32, options: WalkerValueOptions<'f>, reverse: bool) -> Self {
        let mut stack = PooledVec::new();
        push_values(arena, &mut stack, node, reverse);
        Self::from_stack(stack, options, reverse)
    }

    /// walk tokens
    pub fn of_tokens(tokens: &[u32], options: WalkerValueOptions<'f>, reverse: bool) -> Self {
        let mut stack = PooledVec::from_slice(tokens);
        if !reverse {
            stack.reverse();
        }
        Self::from_stack(stack, options, reverse)
    }

    fn from_stack(stack: PooledVec, options: WalkerValueOptions<'f>, reverse: bool) -> Self {
        Self {
            stack,
            fn_: options.fn_,
            type_: options.type_,
            event_type: options.event.unwrap_or(WalkerEvent::Enter),
            reverse,
            current: 0,
            option: WalkerValueResult::None,
        }
    }

    fn is_valid(&mut self, arena: &Arena, token: u32) -> bool {
        match self.type_.as_mut() {
            None => true,
            Some(WalkerValueType::Type(t)) => arena.data_types.get(token) == *t,
            Some(WalkerValueType::Types(types)) => types.contains(&arena.data_types.get(token)),
            Some(WalkerValueType::Fn(f)) => f(arena, token),
        }
    }

    fn push_option(&mut self, option: &[u32]) {
        if self.reverse {
            self.stack.extend_from_slice(option);
        } else {
            self.stack.extend(option.iter().rev());
        }
    }

    fn leave(&mut self, arena: &Arena, value: u32) {
        let ignore = matches!(self.option, WalkerValueResult::Option(o) if o & WalkerOptionEnum::IgnoreChildren != 0);
        if !ignore {
            push_values(arena, &mut self.stack, value, self.reverse);
        }
        if self.event_type & WalkerEvent::Leave != 0 && self.fn_.is_some() && self.is_valid(arena, value) {
            let option = (self.fn_.as_mut().unwrap())(arena, value, WalkerEvent::Leave);
            if let WalkerValueResult::Tokens(tokens) = option {
                self.push_option(&tokens);
            }
        }
    }

    pub fn next(&mut self, arena: &Arena) -> Option<u32> {
        if self.current != 0 {
            let current = self.current;
            self.leave(arena, current);
            self.current = 0;
        }
        while let Some(value) = self.stack.pop() {
            self.option = WalkerValueResult::None;
            if self.fn_.is_some() && self.event_type & WalkerEvent::Enter != 0 && self.is_valid(arena, value) {
                let option = (self.fn_.as_mut().unwrap())(arena, value, WalkerEvent::Enter);
                match &option {
                    WalkerValueResult::Option(o) => {
                        if o & WalkerOptionEnum::Stop != 0 {
                            break;
                        }
                        if o & WalkerOptionEnum::Ignore != 0 {
                            continue;
                        }
                    }
                    WalkerValueResult::Tokens(tokens) => {
                        let tokens = tokens.clone();
                        self.push_option(&tokens);
                    }
                    WalkerValueResult::None => {}
                }
                self.option = option;
            }
            self.current = value;
            return Some(value);
        }
        self.finish();
        None
    }

    pub fn finish(&mut self) {
        self.stack.clear();
        self.current = 0;
    }

    pub fn iter<'w>(&'w mut self, arena: &'w Arena) -> impl Iterator<Item = u32> + use<'w, 'f> {
        std::iter::from_fn(move || self.next(arena))
    }
}
