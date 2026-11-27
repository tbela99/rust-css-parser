//! Port of `src/ast/arena.ts`.
//!
//! Arena memory manager: ast nodes and value tokens are integer ids, their data is stored column-wise (structure of
//! arrays) in Rust owned vectors, lists are doubly linked lists stored in the `first_child` / `last_child` /
//! `first_token` / `last_token` / `next_sibling` / `previous_sibling` columns and strings are interned.
//!
//! The columns are interior mutable ([`Column`]): every method takes `&self`, like the typescript implementation
//! where the arena is a shared mutable object (visitors re-enter the arena while it is transformed).

use super::column::Column;
use super::strings::SharedStr;
use super::strings::{StringBuilder, StringDerived, StringFlag, StringTable, StringTableData};
use super::types::{EnumAstNodeStatus, EnumToken};
use super::walk::{NodeWalker, ValueWalker, WalkerFilter, WalkerOptions, WalkerValueOptions};
use crate::parser::utils::codes::CodeUnits;
use crate::types::ErrorDescription;
use std::cell::{Cell, RefCell, UnsafeCell};
use crate::utils::fx::HashMap;
use std::rc::Rc;

/// position of the structured token fields in the children list
#[allow(non_snake_case)]
pub mod TokenField {
    /// left hand side (`l`)
    pub const L: u32 = 0;
    /// right hand side (`r`)
    pub const R: u32 = 1;
    /// operator (`op`, `op1`)
    pub const Op: u32 = 2;
    /// second operator (`op2`)
    pub const Op2: u32 = 3;
    /// value (`val`)
    pub const Val: u32 = 4;
}

/// numeric token types: the value is stored in the `numeric_value` column
pub fn is_numeric_token_type(typ: u8) -> bool {
    matches!(
        typ,
        EnumToken::NumberTokenType
            | EnumToken::DimensionTokenType
            | EnumToken::LengthTokenType
            | EnumToken::AngleTokenType
            | EnumToken::TimeTokenType
            | EnumToken::FrequencyTokenType
            | EnumToken::ResolutionTokenType
            | EnumToken::PercentageTokenType
            | EnumToken::FlexTokenType
            | EnumToken::UserDefinedDimensionTokenType
    )
}

/// token types whose fields are stored as list children (see [`TokenField`])
pub fn is_structured_token_type(typ: u8) -> bool {
    matches!(
        typ,
        EnumToken::BinaryExpressionTokenType
            | EnumToken::MatchExpressionTokenType
            | EnumToken::MediaQueryConditionTokenType
            | EnumToken::MediaQueryUnaryFeatureTokenType
            | EnumToken::SupportsQueryConditionTokenType
            | EnumToken::SupportsQueryUnaryConditionTokenType
            | EnumToken::WhenElseQueryConditionTokenType
            | EnumToken::WhenElseUnaryConditionTokenType
            | EnumToken::IfConditionTokenType
            | EnumToken::IfElseConditionTokenType
            | EnumToken::ContainerStyleRangeTokenType
            | EnumToken::MediaRangeQueryTokenType
            | EnumToken::ComposesSelectorNodeType
            | EnumToken::NameSpaceAttributeTokenType
    )
}

/// plain object key, field and single value flag of a structured token field
pub type StructuredField = (&'static str, u32, bool);

/// fields of a structured token type: the plain object key (`l`, `r`, `op`, `op1`, `op2`, `val`), the field index and
/// whether the field holds a single token. empty for the other token types
pub fn structured_fields(typ: u8) -> Vec<StructuredField> {
    if !is_structured_token_type(typ) {
        return Vec::new();
    }
    let single = matches!(
        typ,
        EnumToken::BinaryExpressionTokenType
            | EnumToken::IfElseConditionTokenType
            | EnumToken::MatchExpressionTokenType
            | EnumToken::NameSpaceAttributeTokenType
    );
    let single_left = single
        || matches!(
            typ,
            EnumToken::MediaQueryConditionTokenType
                | EnumToken::MediaQueryUnaryFeatureTokenType
                | EnumToken::SupportsQueryUnaryConditionTokenType
                | EnumToken::WhenElseUnaryConditionTokenType
        );
    let single_right = single || typ == EnumToken::ComposesSelectorNodeType;
    vec![
        ("l", TokenField::L, single_left),
        ("r", TokenField::R, single_right),
        (
            if typ == EnumToken::MediaRangeQueryTokenType { "op1" } else { "op" },
            TokenField::Op,
            typ != EnumToken::ContainerStyleRangeTokenType,
        ),
        ("op2", TokenField::Op2, true),
        ("val", TokenField::Val, false),
    ]
}

/// arena columns, in the order of the transferable buffers
pub const ARENA_COLUMNS: [&str; 15] = [
    "dataTypes",
    "parents",
    "nextSibling",
    "previousSibling",
    "firstToken",
    "lastToken",
    "firstChild",
    "lastChild",
    "name",
    "stringValue",
    "numericValue",
    "unit",
    "flags",
    "colorType",
    "spans",
];

/// properties of an arena, see [`Arena::export_properties`] (`Arena.toTransferableArenaObject()`)
#[derive(Clone, Debug, Default)]
pub struct ArenaProperties {
    pub kind: String,
    pub current: u32,
    pub strings: StringTableData,
    pub errors: Vec<(u32, Vec<ErrorDescription>)>,
    pub sources: Option<Vec<u32>>,
    pub input_lengths: Option<Vec<Option<u32>>>,
}

/// column buffers of an arena, in the order of [`ARENA_COLUMNS`]
#[derive(Clone, Debug, Default)]
pub struct ArenaTransferables {
    pub data_types: Vec<u8>,
    pub parents: Vec<u32>,
    pub next_sibling: Vec<u32>,
    pub previous_sibling: Vec<u32>,
    pub first_token: Vec<u32>,
    pub last_token: Vec<u32>,
    pub first_child: Vec<u32>,
    pub last_child: Vec<u32>,
    pub name: Vec<u32>,
    pub string_value: Vec<u32>,
    pub numeric_value: Vec<f64>,
    pub unit: Vec<u32>,
    pub flags: Vec<u32>,
    pub color_type: Vec<u8>,
    pub spans: Vec<u64>,
}

/// the nesting rules of these nodes are not expanded
pub const SKIP_STATES: u32 = EnumAstNodeStatus::Invalid
    | EnumAstNodeStatus::Disallowed
    | EnumAstNodeStatus::Unknown
    | EnumAstNodeStatus::Unparsed
    | EnumAstNodeStatus::Malformed;

/// Arena memory manager.
///
/// The node data is stored column-wise in vectors allocated for the capacity of the arena (62 bytes per node). The
/// arena grows by doubling its capacity and never frees memory by itself: detached nodes keep their memory and their
/// ids stay valid. [`Arena::release`], [`Arena::compact`], [`Arena::trim`] and [`Arena::compact_strings`] reclaim the
/// memory.
pub struct Arena {
    /// interned strings
    string_table: UnsafeCell<StringTable>,
    /// root node
    pub root: u32,
    /// last allocated node id. 0 is never allocated: it means "no node"
    current: Cell<u32>,
    pub data_types: Column<u8>,
    pub parents: Column<u32>,
    pub next_sibling: Column<u32>,
    pub previous_sibling: Column<u32>,
    pub first_token: Column<u32>,
    pub last_token: Column<u32>,
    pub first_child: Column<u32>,
    pub last_child: Column<u32>,
    /// interned name (`nam`) of rules / declarations / at-rules
    pub name: Column<u32>,
    pub string_value: Column<u32>,
    pub numeric_value: Column<f64>,
    pub unit: Column<u32>,
    pub flags: Column<u32>,
    pub color_type: Column<u8>,
    /// packed spans: 8 bits for the source, 28 bits for the start, 28 bits for the end
    pub spans: Column<u64>,
    /// validation errors by node id
    pub errors: RefCell<HashMap<u32, Vec<ErrorDescription>>>,
    /// head of the list of recycled node ids, linked through `next_sibling`
    free_list: Cell<u32>,
    /// incremented when the columns are reallocated: the javascript views are recreated
    generation: Cell<u32>,
    /// the columns have been transferred
    transferred: Cell<bool>,
}

macro_rules! for_each_column {
    ($self:ident, $m:ident $(, $arg:expr)*) => {{
        $self.data_types.$m($($arg),*);
        $self.parents.$m($($arg),*);
        $self.next_sibling.$m($($arg),*);
        $self.previous_sibling.$m($($arg),*);
        $self.first_token.$m($($arg),*);
        $self.last_token.$m($($arg),*);
        $self.first_child.$m($($arg),*);
        $self.last_child.$m($($arg),*);
        $self.name.$m($($arg),*);
        $self.string_value.$m($($arg),*);
        $self.numeric_value.$m($($arg),*);
        $self.unit.$m($($arg),*);
        $self.flags.$m($($arg),*);
        $self.color_type.$m($($arg),*);
        $self.spans.$m($($arg),*);
    }};
}

impl Default for Arena {
    fn default() -> Self {
        Self::new(4096)
    }
}

impl Arena {
    pub fn new(size: usize) -> Self {
        let arena = Self {
            string_table: UnsafeCell::new(StringTable::new()),
            root: 1,
            current: Cell::new(1),
            data_types: Column::new(size),
            parents: Column::new(size),
            next_sibling: Column::new(size),
            previous_sibling: Column::new(size),
            first_token: Column::new(size),
            last_token: Column::new(size),
            first_child: Column::new(size),
            last_child: Column::new(size),
            name: Column::new(size),
            string_value: Column::new(size),
            numeric_value: Column::new(size),
            unit: Column::new(size),
            flags: Column::new(size),
            color_type: Column::new(size),
            spans: Column::new(size),
            errors: RefCell::new(HashMap::default()),
            free_list: Cell::new(0),
            generation: Cell::new(0),
            transferred: Cell::new(false),
        };
        arena.data_types.set(1, EnumToken::StyleSheetNodeType);
        arena
    }

    /// the string table. SAFETY: the table is never borrowed across a call that can re-enter the arena
    #[inline(always)]
    #[allow(clippy::mut_from_ref)]
    pub(crate) fn strings(&self) -> &mut StringTable {
        unsafe { &mut *self.string_table.get() }
    }

    /// last allocated node id
    #[inline]
    pub fn current(&self) -> u32 {
        self.current.get()
    }

    pub fn set_current(&self, current: u32) {
        self.current.set(current);
    }

    /// incremented when the columns are reallocated
    pub fn generation(&self) -> u32 {
        self.generation.get()
    }

    /// address of the generation counter: the javascript views read it to detect a growth of the columns
    pub fn generation_ptr(&self) -> *const u32 {
        self.generation.as_ptr()
    }

    /// pointer and length (in elements) of a column, by index of `arenaColumns`. the column views of javascript
    pub fn column_raw(&self, index: u32) -> (*const u8, usize) {
        fn raw<T: Copy + Default>(c: &Column<T>) -> (*const u8, usize) {
            (c.as_ptr() as *const u8, c.len())
        }
        match index {
            0 => raw(&self.data_types),
            1 => raw(&self.parents),
            2 => raw(&self.next_sibling),
            3 => raw(&self.previous_sibling),
            4 => raw(&self.first_token),
            5 => raw(&self.last_token),
            6 => raw(&self.first_child),
            7 => raw(&self.last_child),
            8 => raw(&self.name),
            9 => raw(&self.string_value),
            10 => raw(&self.numeric_value),
            11 => raw(&self.unit),
            12 => raw(&self.flags),
            13 => raw(&self.color_type),
            14 => raw(&self.spans),
            _ => (std::ptr::null(), 0),
        }
    }

    /// true when the column buffers have been transferred: the arena can no longer be used
    pub fn transferred(&self) -> bool {
        self.transferred.get()
    }

    pub fn allocate(&self) -> u32 {
        let id = self.free_list.get();
        if id != 0 {
            // reuse a recycled node: its record is already zeroed, except the free list link
            self.free_list.set(self.next_sibling.get(id));
            self.next_sibling.set(id, 0);
            return id;
        }
        let id = self.current.get() + 1;
        self.current.set(id);
        if id as usize >= self.data_types.len() {
            self.resize((self.data_types.len() * 2).max(16));
        }
        id
    }

    /// recycle a detached leaf node that is no longer referenced: its id is reused by the next allocation
    pub fn recycle(&self, id: u32) {
        self.data_types.set(id, 0);
        self.name.set(id, 0);
        self.string_value.set(id, 0);
        self.numeric_value.set(id, 0.0);
        self.unit.set(id, 0);
        self.flags.set(id, 0);
        self.color_type.set(id, 0);
        self.spans.set(id, 0);
        if !self.errors.borrow().is_empty() {
            self.errors.borrow_mut().remove(&id);
        }
        self.next_sibling.set(id, self.free_list.get());
        self.free_list.set(id);
    }

    // ---- strings ---------------------------------------------------------------------

    /// intern a string, returns its id
    pub fn intern_string(&self, str: &str) -> u32 {
        self.strings().intern(str)
    }

    /// intern a shared string without copying it
    pub fn intern_rc(&self, str: &Rc<str>) -> u32 {
        self.strings().intern_rc(str)
    }

    /// intern a shared string: a shared `Rc<str>` is not copied
    pub fn intern_shared(&self, str: &SharedStr) -> u32 {
        self.strings().intern_shared(str)
    }

    /// intern a range of code units of a source (or of the canonical text / the pool, see `StringSource`)
    pub fn intern_range(&self, codes: &CodeUnits, start: usize, end: usize, source: i32) -> u32 {
        self.strings().intern_range(codes, start, end, source)
    }

    /// intern utf-16 code units
    pub fn intern_units(&self, codes: &[u16]) -> u32 {
        self.strings().intern_units(codes)
    }

    /// intern a range of the canonical text
    pub fn intern_canonical(&self, start: u32, end: u32) -> u32 {
        self.strings().intern_canonical(start, end)
    }

    /// set the code units of a source: the strings interned from the source are ranges of it
    pub fn set_source_text(&self, source: usize, codes: CodeUnits) {
        self.strings().set_text(source, codes);
    }

    pub fn release_source_codes(&self, source: usize) {
        self.strings().release_codes(source);
    }

    /// code units of a source
    pub fn source_text(&self, source: usize) -> Option<CodeUnits> {
        self.strings().text(source).cloned()
    }

    /// id of an interned string, -1 when the string is not interned
    pub fn string_id(&self, str: &str) -> i32 {
        self.strings().lookup(str)
    }

    pub fn string_length(&self, id: u32) -> usize {
        self.strings().length(id)
    }

    pub fn string_code_at(&self, id: u32, index: usize) -> u32 {
        self.strings().code_at(id, index)
    }

    pub fn string_index_of(&self, id: u32, code: u32) -> isize {
        self.strings().index_of_code(id, code)
    }

    pub fn string_equals(&self, id: u32, str: &str) -> bool {
        self.strings().equals(id, str)
    }

    pub fn string_equals_ignore_case(&self, id: u32, str: &str) -> bool {
        self.strings().equals_ignore_case(id, str)
    }

    pub fn string_starts_with(&self, id: u32, str: &str) -> bool {
        self.strings().starts_with(id, str)
    }

    /// id of the lowercase version of an interned string
    pub fn lower_string_id(&self, id: u32) -> u32 {
        self.strings().lower(id)
    }

    /// append an interned string to a byte buffer when it is ascii. -1 when it is not ascii
    pub fn copy_string_ascii(&self, id: u32, target: &mut Vec<u8>) -> isize {
        self.strings().copy_ascii(id, target)
    }

    /// a boolean property of an interned string, computed once per string id
    pub fn string_flag(&self, id: u32, flag: StringFlag, compute: impl FnOnce(&str) -> bool) -> bool {
        if let Some(value) = self.strings().cached_flag(id, flag) {
            return value;
        }
        // the computation may use the arena: the table is not borrowed while it runs
        let value = self.with_string(id, compute);
        self.strings().set_flag(id, flag, value);
        value
    }

    /// a boolean property of an interned string, computed once per string id from the id
    pub fn string_flag_id(&self, id: u32, flag: StringFlag, compute: impl FnOnce(u32) -> bool) -> bool {
        if let Some(value) = self.strings().cached_flag(id, flag) {
            return value;
        }
        let value = compute(id);
        self.strings().set_flag(id, flag, value);
        value
    }

    /// a value derived from an interned string, computed once per string id
    pub fn string_derived<T: Clone + 'static>(&self, id: u32, slot: StringDerived, compute: impl FnOnce(u32) -> T) -> T {
        if let Some(value) = self.strings().derived::<T>(id, slot) {
            return value;
        }
        let value = compute(id);
        self.strings().set_derived(id, slot, value.clone());
        value
    }

    /// append the code units of an interned string to a buffer
    pub fn copy_string(&self, id: u32, target: &mut Vec<u16>) -> usize {
        self.strings().copy_into(id, target)
    }

    /// append an interned string to a builder
    pub fn append_string<'b>(&self, builder: &'b mut StringBuilder, id: u32) -> &'b mut StringBuilder {
        builder.append_id(self.strings(), id)
    }

    /// intern the string built by a builder
    pub fn intern_builder(&self, builder: &StringBuilder) -> u32 {
        builder.intern(self.strings())
    }

    /// identity of the string ids: changed when the strings are renumbered
    pub fn string_epoch(&self) -> u64 {
        self.strings().epoch
    }

    /// run a function with an interned string. ascii ranges of a source are borrowed: no string is created
    pub fn with_string<R>(&self, id: u32, f: impl FnOnce(&str) -> R) -> R {
        // copy out first when the table may be re-entered: `f` must not mutate the string table. the borrowed
        // ascii range belongs to an immutable shared buffer
        let table: &StringTable = self.strings();
        table.with_str(id, f)
    }

    /// the string of an id (created and cached the first time it is read)
    pub fn resolve_string(&self, id: u32) -> SharedStr {
        self.strings().get(id)
    }

    // ---- columns ---------------------------------------------------------------------

    #[inline]
    pub fn get_type(&self, id: u32) -> u8 {
        self.data_types.get(id)
    }

    /// type of a node or a token. None when the id is None
    pub fn type_of(&self, id: Option<u32>) -> Option<u8> {
        id.map(|id| self.data_types.get(id))
    }

    #[inline]
    pub fn set_type(&self, id: u32, typ: u8) {
        self.data_types.set(id, typ);
    }

    #[inline]
    pub fn set_name(&self, id: u32, name_id: u32) {
        self.name.set(id, name_id);
    }

    pub fn get_name(&self, id: u32) -> SharedStr {
        self.resolve_string(self.name.get(id))
    }

    #[inline]
    pub fn set_value(&self, id: u32, value_id: u32) {
        self.string_value.set(id, value_id);
    }

    pub fn get_value(&self, id: u32) -> SharedStr {
        self.resolve_string(self.string_value.get(id))
    }

    #[inline]
    pub fn set_numeric_value(&self, id: u32, value: f64) {
        self.numeric_value.set(id, value);
    }

    #[inline]
    pub fn set_unit(&self, id: u32, unit: u32) {
        self.unit.set(id, unit);
    }

    pub fn get_unit(&self, id: u32) -> SharedStr {
        self.resolve_string(self.unit.get(id))
    }

    #[inline]
    pub fn get_numeric_value(&self, id: u32) -> f64 {
        self.numeric_value.get(id)
    }

    #[inline]
    pub fn set_parent(&self, id: u32, parent: u32) {
        self.parents.set(id, parent);
    }

    #[inline]
    pub fn get_first_child(&self, id: u32) -> u32 {
        self.first_child.get(id)
    }

    #[inline]
    pub fn get_next_sibling(&self, id: u32) -> u32 {
        self.next_sibling.get(id)
    }

    #[inline]
    pub fn get_previous_sibling(&self, id: u32) -> u32 {
        self.previous_sibling.get(id)
    }

    #[inline]
    pub fn get_first_token(&self, id: u32) -> u32 {
        self.first_token.get(id)
    }

    #[inline]
    pub fn get_last_token(&self, id: u32) -> u32 {
        self.last_token.get(id)
    }

    /// number of nodes the columns can hold
    pub fn get_capacity(&self) -> usize {
        self.data_types.len()
    }

    pub fn get_last_id(&self) -> u32 {
        self.current.get()
    }

    /// approximate number of bytes held by the arena: the columns (62 bytes per node of the capacity) and the string
    /// table
    pub fn memory_size(&self) -> usize {
        self.get_capacity() * 62 + self.strings().memory_size()
    }

    pub fn get_string_count(&self) -> u32 {
        self.strings().count
    }

    /// replace the first child
    pub fn set_first_child(&self, id: u32, first_child_id: u32) {
        let first_child = self.first_child.get(id);
        if first_child == first_child_id {
            return;
        }
        self.detach(first_child_id);
        if first_child != 0 {
            self.insert_before(id, first_child, first_child_id);
            self.detach(first_child);
        } else {
            self.append_child_list(id, &[first_child_id]);
        }
    }

    #[inline]
    pub fn get_last_child(&self, id: u32) -> u32 {
        self.last_child.get(id)
    }

    /// replace the last child
    pub fn set_last_child(&self, id: u32, last_child_id: u32) {
        let last_child = self.last_child.get(id);
        if last_child == last_child_id {
            return;
        }
        self.detach(last_child_id);
        if last_child != 0 {
            self.detach(last_child);
        }
        self.append_child_list(id, &[last_child_id]);
    }

    /// insert a node after another one
    pub fn set_next_sibling(&self, id: u32, next_sibling_id: u32) {
        if id == next_sibling_id {
            return;
        }
        self.detach(next_sibling_id);
        let parent = self.parents.get(id);
        let next = self.next_sibling.get(id);
        self.next_sibling.set(next_sibling_id, next);
        self.previous_sibling.set(next_sibling_id, id);
        self.next_sibling.set(id, next_sibling_id);
        self.parents.set(next_sibling_id, parent);
        if next != 0 {
            self.previous_sibling.set(next, next_sibling_id);
        } else if self.last_child.get(parent) == id {
            self.last_child.set(parent, next_sibling_id);
        } else if self.last_token.get(parent) == id {
            self.last_token.set(parent, next_sibling_id);
        }
    }

    /// insert a node before another one
    pub fn set_previous_sibling(&self, id: u32, previous_sibling_id: u32) {
        if id == previous_sibling_id {
            return;
        }
        self.detach(previous_sibling_id);
        self.insert_before(self.parents.get(id), id, previous_sibling_id);
    }

    /// replace the first token
    pub fn set_first_token(&self, id: u32, first_token_id: u32) {
        let first_token = self.first_token.get(id);
        if first_token == first_token_id {
            return;
        }
        self.detach(first_token_id);
        if first_token != 0 {
            self.insert_before(id, first_token, first_token_id);
            self.detach(first_token);
        } else {
            self.append_token_list(id, &[first_token_id]);
        }
    }

    #[inline]
    pub fn set_color_type(&self, id: u32, color_type: u8) {
        self.color_type.set(id, color_type);
    }

    #[inline]
    pub fn get_color_type(&self, id: u32) -> u8 {
        self.color_type.get(id)
    }

    #[inline]
    pub fn set_flags(&self, id: u32, flags: u32) {
        self.flags.set(id, flags);
    }

    #[inline]
    pub fn get_flags(&self, id: u32) -> u32 {
        self.flags.get(id)
    }

    #[inline]
    pub fn has_flags(&self, id: u32, flag: u32) -> bool {
        self.flags.get(id) & flag == flag
    }

    /// child at an index
    pub fn get_child_at(&self, id: u32, mut index: u32) -> Option<u32> {
        let mut child = self.first_child.get(id);
        while index > 0 && child != 0 {
            child = self.next_sibling.get(child);
            index -= 1;
        }
        if child == 0 {
            None
        } else {
            Some(child)
        }
    }

    /// token at an index of the token list
    pub fn get_token_at(&self, id: u32, mut index: u32) -> Option<u32> {
        let mut token = self.first_token.get(id);
        while index > 0 && token != 0 {
            token = self.next_sibling.get(token);
            index -= 1;
        }
        if token == 0 {
            None
        } else {
            Some(token)
        }
    }

    /// replace the child at an index. the node is appended when the index is out of range
    pub fn set_child_at(&self, id: u32, index: u32, child: u32) {
        let current = self.get_child_at(id, index);
        if current == Some(child) {
            return;
        }
        self.detach(child);
        match current {
            None => self.append_child_list(id, &[child]),
            Some(current) => {
                self.insert_before(id, current, child);
                self.detach(current);
            }
        }
    }

    // ---- structured token fields ---------------------------------------------------------

    /// list node holding the tokens of a structured token field. created when missing
    pub fn field_list(&self, id: u32, field: u32) -> u32 {
        let mut child = self.first_child.get(id);
        let mut index = 0;
        while index < field {
            if child == 0 {
                break;
            }
            child = self.next_sibling.get(child);
            index += 1;
        }
        if child != 0 {
            return child;
        }
        let mut count = self.child_count(id);
        while count <= field {
            child = self.allocate();
            self.data_types.set(child, EnumToken::ListTokenType);
            self.append_child_list(id, &[child]);
            count += 1;
        }
        child
    }

    /// true if the field exists and is not empty
    pub fn has_field(&self, id: u32, field: u32) -> bool {
        self.get_child_at(id, field).is_some_and(|list| self.first_token.get(list) != 0)
    }

    /// first token of a structured token field
    pub fn field(&self, id: u32, field: u32) -> u32 {
        self.get_child_at(id, field).map_or(0, |list| self.first_token.get(list))
    }

    /// tokens of a structured token field
    pub fn field_tokens(&self, id: u32, field: u32) -> Vec<u32> {
        self.get_child_at(id, field).map_or_else(Vec::new, |list| self.tokens(list))
    }

    /// set the tokens of a structured token field
    pub fn set_field(&self, id: u32, field: u32, tokens: &[u32]) {
        self.set_token_list(self.field_list(id, field), tokens);
    }

    /// append tokens to a structured token field
    pub fn append_field(&self, id: u32, field: u32, tokens: &[u32]) {
        self.append_token_list(self.field_list(id, field), tokens);
    }

    pub fn set_left(&self, id: u32, children: &[u32]) {
        self.set_token_list(self.field_list(id, TokenField::L), children);
    }

    pub fn get_left(&self, id: u32) -> Option<Vec<u32>> {
        self.get_child_at(id, TokenField::L).map(|_| self.field_tokens(id, TokenField::L))
    }

    pub fn set_right(&self, id: u32, right: &[u32]) {
        self.set_token_list(self.field_list(id, TokenField::R), right);
    }

    pub fn get_right(&self, id: u32) -> Option<Vec<u32>> {
        self.get_child_at(id, TokenField::R).map(|_| self.field_tokens(id, TokenField::R))
    }

    pub fn set_op(&self, id: u32, op: u32) {
        self.set_token_list(self.field_list(id, TokenField::Op), &[op]);
    }

    pub fn get_op(&self, id: u32) -> u32 {
        self.field(id, TokenField::Op)
    }

    pub fn set_op2(&self, id: u32, op2: u32) {
        self.set_token_list(self.field_list(id, TokenField::Op2), &[op2]);
    }

    pub fn get_op2(&self, id: u32) -> u32 {
        self.field(id, TokenField::Op2)
    }

    // ---- spans ---------------------------------------------------------------------

    #[inline]
    pub fn set_span(&self, id: u32, source_id: u32, start: u32, end: u32) {
        self.spans.set(
            id,
            ((source_id as u64 & 0xff) << 56) | ((start as u64 & 0xfffffff) << 28) | (end as u64 & 0xfffffff),
        );
    }

    /// copy the packed location of a node
    #[inline]
    pub fn copy_span(&self, target: u32, source: u32) {
        self.spans.set(target, self.spans.get(source));
    }

    #[inline]
    pub fn get_source_id(&self, id: u32) -> u32 {
        ((self.spans.get(id) >> 56) & 0xff) as u32
    }

    #[inline]
    pub fn get_span_start(&self, id: u32) -> u32 {
        ((self.spans.get(id) >> 28) & 0xfffffff) as u32
    }

    /// (sic: the typescript name is `setSpandStart`)
    pub fn set_span_start(&self, id: u32, start: u32) {
        self.set_span(id, self.get_source_id(id), start, self.get_span_end(id));
    }

    #[inline]
    pub fn get_span_end(&self, id: u32) -> u32 {
        (self.spans.get(id) & 0xfffffff) as u32
    }

    pub fn set_span_end(&self, id: u32, end: u32) {
        self.set_span(id, self.get_source_id(id), self.get_span_start(id), end);
    }

    #[inline]
    pub fn get_parent(&self, id: u32) -> u32 {
        self.parents.get(id)
    }

    /// detach a node from its parent
    pub fn detach(&self, id: u32) {
        if id == 0 || self.parents.get(id) == 0 {
            return;
        }
        let parent = self.parents.get(id);
        let next = self.next_sibling.get(id);
        let previous = self.previous_sibling.get(id);
        if previous != 0 {
            self.next_sibling.set(previous, next);
        } else if self.first_child.get(parent) == id {
            self.first_child.set(parent, next);
        } else if self.first_token.get(parent) == id {
            self.first_token.set(parent, next);
        }
        if next != 0 {
            self.previous_sibling.set(next, previous);
        } else if self.last_child.get(parent) == id {
            self.last_child.set(parent, previous);
        } else if self.last_token.get(parent) == id {
            self.last_token.set(parent, previous);
        }
        self.next_sibling.set(id, 0);
        self.previous_sibling.set(id, 0);
        self.parents.set(id, 0);
    }

    // ---- walkers ---------------------------------------------------------------------

    /// walk the ast nodes: the node and its descendants, in document order
    pub fn walk<'f>(&self, node: u32, filter: Option<WalkerFilter<'f>>, reverse: bool) -> NodeWalker<'f> {
        NodeWalker::new(node, filter, reverse)
    }

    /// walk the ast nodes with options
    pub fn walk_with<'f>(&self, node: u32, options: WalkerOptions<'f>) -> NodeWalker<'f> {
        NodeWalker::with_options(node, options)
    }

    /// walk the value tokens of a node / token and their descendants
    pub fn walk_values<'f>(&self, node: u32, options: WalkerValueOptions<'f>, reverse: bool) -> ValueWalker<'f> {
        ValueWalker::of_node(self, node, options, reverse)
    }

    /// walk tokens and their descendants
    pub fn walk_token_values<'f>(&self, tokens: &[u32], options: WalkerValueOptions<'f>, reverse: bool) -> ValueWalker<'f> {
        ValueWalker::of_tokens(tokens, options, reverse)
    }

    /// node ids of a subtree in document order (collected)
    pub fn walk_ids(&self, node: u32) -> Vec<u32> {
        let mut walker = NodeWalker::new(node, None, false);
        walker.iter(self).collect()
    }

    /// call the visitor for the tokens and their children, in the order of [`Arena::walk_values`] (the children of a
    /// token are read after the visitor returns)
    pub fn visit_values(&self, tokens: &[u32], mut visitor: impl FnMut(&Self, u32)) {
        // a pooled stack: no allocation per traversal
        let mut stack = crate::utils::pool::PooledVec::new();
        stack.extend(tokens.iter().rev().copied());
        while let Some(value) = stack.pop() {
            visitor(self, value);
            let first = stack.len();
            let mut token = self.first_token.get(value);
            while token != 0 {
                stack.push(token);
                token = self.next_sibling.get(token);
            }
            if self.flags.get(value) & EnumAstNodeStatus::HasChildren == 0 {
                let mut child = self.first_child.get(value);
                while child != 0 {
                    if self.data_types.get(child) == EnumToken::ListTokenType {
                        token = self.first_token.get(child);
                        while token != 0 {
                            stack.push(token);
                            token = self.next_sibling.get(token);
                        }
                    } else {
                        stack.push(child);
                    }
                    child = self.next_sibling.get(child);
                }
            }
            stack[first..].reverse();
        }
    }

    /// parent of a value token: the node / token that owns the token list, or the structured token that owns the
    /// field (the field list token is skipped). 0 when the token is detached
    pub fn get_value_parent(&self, id: u32) -> u32 {
        let parent = self.parents.get(id);
        if parent != 0 && self.data_types.get(parent) == EnumToken::ListTokenType {
            self.parents.get(parent)
        } else {
            parent
        }
    }

    /// node that owns a value token: the declaration, the rule or the at-rule. 0 when the token is detached
    pub fn get_value_owner(&self, id: u32) -> u32 {
        let mut parent = self.parents.get(id);
        while parent != 0 && self.flags.get(parent) & (EnumAstNodeStatus::HasChildren | EnumAstNodeStatus::HasValues) == 0 {
            parent = self.parents.get(parent);
        }
        parent
    }

    /// first match of a subtree, in document order
    pub fn find(&self, node: u32, mut matcher: impl FnMut(&Self, u32) -> bool) -> Option<u32> {
        let mut walker = NodeWalker::new(node, None, false);
        while let Some(id) = walker.next(self) {
            if matcher(self, id) {
                walker.finish();
                return Some(id);
            }
        }
        None
    }

    /// all the matches of a subtree, in document order
    pub fn find_all(&self, node: u32, mut matcher: impl FnMut(&Self, u32) -> bool) -> Vec<u32> {
        let mut result = Vec::new();
        let mut walker = NodeWalker::new(node, None, false);
        while let Some(id) = walker.next(self) {
            if matcher(self, id) {
                result.push(id);
            }
        }
        result
    }

    /// last match of a subtree in document order
    pub fn find_last(&self, node: u32, mut matcher: impl FnMut(&Self, u32) -> bool) -> Option<u32> {
        let mut walker = NodeWalker::new(node, None, true);
        while let Some(id) = walker.next(self) {
            if matcher(self, id) {
                walker.finish();
                return Some(id);
            }
        }
        None
    }

    /// first value token match of a subtree: the node that owns the token and the token
    pub fn find_by_value(&self, node: u32, mut matcher: impl FnMut(&Self, u32) -> bool) -> Option<(u32, u32)> {
        let mut walker = NodeWalker::new(node, None, false);
        while let Some(current) = walker.next(self) {
            if self.first_token.get(current) == 0 {
                continue;
            }
            let tokens = self.tokens(current);
            let mut values = ValueWalker::of_tokens(&tokens, WalkerValueOptions::default(), false);
            while let Some(value) = values.next(self) {
                if matcher(self, value) {
                    walker.finish();
                    return Some((current, value));
                }
            }
        }
        None
    }

    /// first match of a token and its descendants, or of the value tokens of a node
    pub fn find_value(&self, node: u32, mut matcher: impl FnMut(&Self, u32) -> bool) -> Option<u32> {
        if node == 0 {
            return None;
        }
        if self.flags.get(node) & (EnumAstNodeStatus::HasChildren | EnumAstNodeStatus::HasValues) == 0 && matcher(self, node) {
            return Some(node);
        }
        let mut values = ValueWalker::of_node(self, node, WalkerValueOptions::default(), false);
        while let Some(value) = values.next(self) {
            if matcher(self, value) {
                values.finish();
                return Some(value);
            }
        }
        None
    }

    /// direct value tokens of a node or a token: the token list, then the tokens of the structured fields and the
    /// child tokens
    pub fn sub_tokens(&self, id: u32) -> Vec<u32> {
        let mut result = self.tokens(id);
        if self.flags.get(id) & EnumAstNodeStatus::HasChildren != 0 {
            return result;
        }
        let mut child = self.first_child.get(id);
        while child != 0 {
            if self.data_types.get(child) == EnumToken::ListTokenType {
                let mut token = self.first_token.get(child);
                while token != 0 {
                    result.push(token);
                    token = self.next_sibling.get(token);
                }
            } else {
                result.push(child);
            }
            child = self.next_sibling.get(child);
        }
        result
    }

    pub fn find_child(&self, id: u32, mut predicate: impl FnMut(u32) -> bool) -> Option<u32> {
        let mut child = self.first_child.get(id);
        while child != 0 {
            if predicate(child) {
                return Some(child);
            }
            child = self.next_sibling.get(child);
        }
        None
    }

    /// find a token in the token list
    pub fn find_token(&self, id: u32, mut predicate: impl FnMut(&Self, u32) -> bool) -> Option<u32> {
        let mut token = self.first_token.get(id);
        while token != 0 {
            if predicate(self, token) {
                return Some(token);
            }
            token = self.next_sibling.get(token);
        }
        None
    }

    /// the children of the parent of a node, the node included
    pub fn siblings(&self, id: u32, mut filter: Option<&mut dyn FnMut(&Self, u32) -> bool>) -> Vec<u32> {
        let mut siblings = Vec::new();
        let mut child = self.first_child.get(self.parents.get(id));
        while child != 0 {
            if filter.as_mut().is_none_or(|f| f(self, child)) {
                siblings.push(child);
            }
            child = self.next_sibling.get(child);
        }
        siblings
    }

    /// child node ids
    pub fn children(&self, id: u32) -> Vec<u32> {
        let mut children = Vec::new();
        let mut child = self.first_child.get(id);
        while child != 0 {
            children.push(child);
            child = self.next_sibling.get(child);
        }
        children
    }

    /// child node ids accepted by a filter
    pub fn children_filtered(&self, id: u32, mut filter: impl FnMut(&Self, u32) -> bool) -> Vec<u32> {
        let mut children = Vec::new();
        let mut child = self.first_child.get(id);
        while child != 0 {
            if filter(self, child) {
                children.push(child);
            }
            child = self.next_sibling.get(child);
        }
        children
    }

    /// token ids of the token list
    pub fn tokens(&self, id: u32) -> Vec<u32> {
        let mut tokens = Vec::new();
        let mut token = self.first_token.get(id);
        while token != 0 {
            tokens.push(token);
            token = self.next_sibling.get(token);
        }
        tokens
    }

    /// token ids of the token list accepted by a filter
    pub fn tokens_filtered(&self, id: u32, mut filter: impl FnMut(&Self, u32) -> bool) -> Vec<u32> {
        let mut tokens = Vec::new();
        let mut token = self.first_token.get(id);
        while token != 0 {
            if filter(self, token) {
                tokens.push(token);
            }
            token = self.next_sibling.get(token);
        }
        tokens
    }

    /// iterate the token list without allocation. the next token is read before the current one is returned
    pub fn token_iter(&self, id: u32) -> impl Iterator<Item = u32> + '_ {
        let mut token = self.first_token.get(id);
        std::iter::from_fn(move || {
            if token == 0 {
                None
            } else {
                let current = token;
                token = self.next_sibling.get(token);
                Some(current)
            }
        })
    }

    /// iterate the children without allocation. the next child is read before the current one is returned
    pub fn child_iter(&self, id: u32) -> impl Iterator<Item = u32> + '_ {
        let mut child = self.first_child.get(id);
        std::iter::from_fn(move || {
            if child == 0 {
                None
            } else {
                let current = child;
                child = self.next_sibling.get(child);
                Some(current)
            }
        })
    }

    /// number of tokens in the token list
    pub fn token_count(&self, id: u32) -> u32 {
        self.token_iter(id).count() as u32
    }

    pub fn set_children(&self, id: u32, children: &[u32]) {
        self.set_child_list(id, children);
    }

    /// replace the children
    pub fn set_child_list(&self, id: u32, children: &[u32]) {
        let mut child = self.first_child.get(id);
        while child != 0 {
            let next = self.next_sibling.get(child);
            self.next_sibling.set(child, 0);
            self.previous_sibling.set(child, 0);
            self.parents.set(child, 0);
            child = next;
        }
        self.first_child.set(id, 0);
        self.last_child.set(id, 0);
        self.append_child_list(id, children);
    }

    pub fn set_tokens(&self, id: u32, tokens: &[u32]) {
        self.set_token_list(id, tokens);
    }

    /// replace the token list
    pub fn set_token_list(&self, id: u32, tokens: &[u32]) {
        let mut token = self.first_token.get(id);
        while token != 0 {
            let next = self.next_sibling.get(token);
            self.next_sibling.set(token, 0);
            self.previous_sibling.set(token, 0);
            self.parents.set(token, 0);
            token = next;
        }
        self.first_token.set(id, 0);
        self.last_token.set(id, 0);
        self.append_token_list(id, tokens);
    }

    /// move the children of a node into another node, in order
    pub fn move_children(&self, from: u32, to: u32, before: u32) {
        if from == to {
            return;
        }
        let mut child = self.first_child.get(from);
        while child != 0 {
            let next = self.next_sibling.get(child);
            self.detach(child);
            if before == 0 {
                self.append_child(to, child);
            } else {
                self.insert_before(to, before, child);
            }
            child = next;
        }
    }

    pub fn append_children(&self, id: u32, children: &[u32]) {
        self.append_child_list(id, children);
    }

    /// append children
    pub fn append_child_list(&self, id: u32, children: &[u32]) {
        for &child in children {
            if child == id || child == 0 {
                continue;
            }
            // the child may already be the last child: detach before reading the last child
            self.detach(child);
            let last_child = self.last_child.get(id);
            if last_child == 0 {
                self.first_child.set(id, child);
            } else {
                self.next_sibling.set(last_child, child);
            }
            self.previous_sibling.set(child, last_child);
            self.next_sibling.set(child, 0);
            self.parents.set(child, id);
            self.last_child.set(id, child);
        }
    }

    pub fn append_tokens(&self, id: u32, tokens: &[u32]) {
        self.append_token_list(id, tokens);
    }

    /// append tokens
    pub fn append_token_list(&self, id: u32, tokens: &[u32]) {
        for &token in tokens {
            if token == id || token == 0 {
                continue;
            }
            self.detach(token);
            let last_token = self.last_token.get(id);
            if last_token == 0 {
                self.first_token.set(id, token);
            } else {
                self.next_sibling.set(last_token, token);
            }
            self.previous_sibling.set(token, last_token);
            self.next_sibling.set(token, 0);
            self.parents.set(token, id);
            self.last_token.set(id, token);
        }
    }

    /// append a single child node
    pub fn append_child(&self, id: u32, child: u32) {
        if child == id || child == 0 {
            return;
        }
        if self.parents.get(child) != 0 {
            self.detach(child);
        }
        let last_child = self.last_child.get(id);
        if last_child == 0 {
            self.first_child.set(id, child);
        } else {
            self.next_sibling.set(last_child, child);
        }
        self.previous_sibling.set(child, last_child);
        self.next_sibling.set(child, 0);
        self.parents.set(child, id);
        self.last_child.set(id, child);
    }

    /// append a single token
    pub fn append_token(&self, id: u32, token: u32) {
        if token == id || token == 0 {
            return;
        }
        if self.parents.get(token) != 0 {
            self.detach(token);
        }
        let last_token = self.last_token.get(id);
        if last_token == 0 {
            self.first_token.set(id, token);
        } else {
            self.next_sibling.set(last_token, token);
        }
        self.previous_sibling.set(token, last_token);
        self.next_sibling.set(token, 0);
        self.parents.set(token, id);
        self.last_token.set(id, token);
    }

    /// prepend tokens to the token list
    pub fn prepend_tokens(&self, id: u32, tokens: &[u32]) {
        let first = self.first_token.get(id);
        if first == 0 {
            self.append_token_list(id, tokens);
            return;
        }
        for &token in tokens {
            if token == id || token == 0 {
                continue;
            }
            self.set_previous_sibling(first, token);
        }
    }

    /// `key` is "nam" or "val"
    pub fn equals_ignore_case(&self, node: u32, key: &str, string: &str) -> bool {
        let id = if key == "nam" { self.name.get(node) } else { self.string_value.get(node) };
        self.strings().equals_ignore_case(id, string)
    }

    /// `key` is "nam" or "val"
    pub fn starts_with(&self, node: u32, key: &str, string: &str) -> bool {
        let id = if key == "nam" { self.name.get(node) } else { self.string_value.get(node) };
        self.strings().starts_with(id, string)
    }

    /// grow the columns
    pub fn resize(&self, new_size: usize) {
        if new_size <= self.data_types.len() {
            return;
        }
        for_each_column!(self, resize, new_size);
        self.generation.set(self.generation.get() + 1);
    }

    /// drop the interned strings that are no longer referenced by any node
    pub fn compact_strings(&self) {
        let table = self.strings();
        let mut used = vec![0u8; table.count as usize];
        let size = self.current.get() + 1;
        if used.is_empty() {
            return;
        }
        used[0] = 1;
        for id in 1..size {
            for v in [self.name.get(id), self.string_value.get(id), self.unit.get(id)] {
                if let Some(u) = used.get_mut(v as usize) {
                    *u = 1;
                }
            }
        }
        let count: u32 = used.iter().map(|&u| u as u32).sum();
        if count == table.count {
            return;
        }
        let mapping = table.compact(&used);
        let map = |v: u32| mapping.get(v as usize).copied().unwrap_or(0);
        for id in 1..size {
            self.name.set(id, map(self.name.get(id)));
            self.string_value.set(id, map(self.string_value.get(id)));
            self.unit.set(id, map(self.unit.get(id)));
        }
    }

    /// drop the nodes that are not reachable from the root and renumber the remaining nodes in document order. the
    /// unused strings are dropped too. returns the mapping of the old ids to the new ids (0 when dropped)
    pub fn compact(&self, errors: Option<&mut [ErrorDescription]>) -> Vec<u32> {
        let size = self.current.get() as usize + 1;
        let mut mapping = vec![0u32; size];
        let mut order = vec![0u32; size];
        let mut stack = vec![self.root];
        let mut next = 1usize;
        while let Some(id) = stack.pop() {
            order[next] = id;
            mapping[id as usize] = next as u32;
            next += 1;
            let mut child = self.last_child.get(id);
            while child != 0 {
                stack.push(child);
                child = self.previous_sibling.get(child);
            }
            let mut token = self.last_token.get(id);
            while token != 0 {
                stack.push(token);
                token = self.previous_sibling.get(token);
            }
        }
        let capacity = next + 64.max(next >> 5);
        let m = |v: u32| mapping[v as usize];
        let mut t = ArenaTransferables {
            data_types: vec![0; capacity],
            parents: vec![0; capacity],
            next_sibling: vec![0; capacity],
            previous_sibling: vec![0; capacity],
            first_token: vec![0; capacity],
            last_token: vec![0; capacity],
            first_child: vec![0; capacity],
            last_child: vec![0; capacity],
            name: vec![0; capacity],
            string_value: vec![0; capacity],
            numeric_value: vec![0.0; capacity],
            unit: vec![0; capacity],
            flags: vec![0; capacity],
            color_type: vec![0; capacity],
            spans: vec![0; capacity],
        };
        let table = self.strings();
        let mut used = vec![0u8; table.count as usize];
        if !used.is_empty() {
            used[0] = 1;
        }
        for target in 1..next {
            let id = order[target];
            t.data_types[target] = self.data_types.get(id);
            t.parents[target] = m(self.parents.get(id));
            t.next_sibling[target] = m(self.next_sibling.get(id));
            t.previous_sibling[target] = m(self.previous_sibling.get(id));
            t.first_token[target] = m(self.first_token.get(id));
            t.last_token[target] = m(self.last_token.get(id));
            t.first_child[target] = m(self.first_child.get(id));
            t.last_child[target] = m(self.last_child.get(id));
            t.name[target] = self.name.get(id);
            t.string_value[target] = self.string_value.get(id);
            t.numeric_value[target] = self.numeric_value.get(id);
            t.unit[target] = self.unit.get(id);
            t.flags[target] = self.flags.get(id);
            t.color_type[target] = self.color_type.get(id);
            t.spans[target] = self.spans.get(id);
            for v in [t.name[target], t.string_value[target], t.unit[target]] {
                if let Some(u) = used.get_mut(v as usize) {
                    *u = 1;
                }
            }
        }
        let count: u32 = used.iter().map(|&u| u as u32).sum();
        if count < table.count {
            let string_mapping = table.compact(&used);
            let sm = |v: u32| string_mapping.get(v as usize).copied().unwrap_or(0);
            for id in 1..next {
                t.name[id] = sm(t.name[id]);
                t.string_value[id] = sm(t.string_value[id]);
                t.unit[id] = sm(t.unit[id]);
            }
        }
        let node_errors: HashMap<u32, Vec<ErrorDescription>> = self
            .errors
            .borrow_mut()
            .drain()
            .filter_map(|(id, list)| {
                let target = mapping.get(id as usize).copied().unwrap_or(0);
                (target != 0).then_some((target, list))
            })
            .collect();
        if let Some(errors) = errors {
            for error in errors.iter_mut() {
                if let Some(node) = error.node {
                    let target = mapping.get(node as usize).copied().unwrap_or(0);
                    error.node = if target == 0 { None } else { Some(target) };
                }
            }
        }
        self.import_columns(t);
        *self.errors.borrow_mut() = node_errors;
        self.current.set(next as u32 - 1);
        self.free_list.set(0);
        mapping
    }

    /// replace the columns
    pub fn import_columns(&self, t: ArenaTransferables) {
        self.data_types.replace(t.data_types);
        self.parents.replace(t.parents);
        self.next_sibling.replace(t.next_sibling);
        self.previous_sibling.replace(t.previous_sibling);
        self.first_token.replace(t.first_token);
        self.last_token.replace(t.last_token);
        self.first_child.replace(t.first_child);
        self.last_child.replace(t.last_child);
        self.name.replace(t.name);
        self.string_value.replace(t.string_value);
        self.numeric_value.replace(t.numeric_value);
        self.unit.replace(t.unit);
        self.flags.replace(t.flags);
        self.color_type.replace(t.color_type);
        self.spans.replace(t.spans);
        self.generation.set(self.generation.get() + 1);
    }

    /// mark nodes to be released or trimmed later
    pub fn mark(&self, ids: &[u32]) {
        for &id in ids {
            if id != 0 && id != self.root {
                self.flags.set(id, self.flags.get(id) | EnumAstNodeStatus::Marked);
            }
        }
    }

    pub fn is_marked(&self, id: u32) -> bool {
        self.flags.get(id) & EnumAstNodeStatus::Marked != 0
    }

    /// release nodes: detach them and drop their data. `None` releases the marked nodes
    pub fn release(&self, id: Option<u32>, deep: bool) {
        let Some(id) = id else {
            let mut node = 1;
            while node <= self.current.get() {
                if self.flags.get(node) & EnumAstNodeStatus::Marked != 0 {
                    self.flags.set(node, self.flags.get(node) & !EnumAstNodeStatus::Marked);
                    self.release(Some(node), true);
                }
                node += 1;
            }
            return;
        };
        if id == 0 || id == self.root {
            return;
        }
        if deep {
            let mut child = self.first_child.get(id);
            while child != 0 {
                let next = self.next_sibling.get(child);
                self.release(Some(child), true);
                child = next;
            }
            let mut token = self.first_token.get(id);
            while token != 0 {
                let next = self.next_sibling.get(token);
                self.release(Some(token), true);
                token = next;
            }
        }
        self.detach(id);
        self.name.set(id, 0);
        self.string_value.set(id, 0);
        self.unit.set(id, 0);
        self.errors.borrow_mut().remove(&id);
    }

    /// reclaim the memory: release the marked nodes, then compact
    pub fn trim(&self, errors: Option<&mut [ErrorDescription]>) -> Vec<u32> {
        self.release(None, false);
        self.compact(errors)
    }

    pub fn set_errors(&self, id: u32, errors: Option<Vec<ErrorDescription>>) {
        match errors {
            Some(errors) if !errors.is_empty() => {
                self.errors.borrow_mut().insert(id, errors);
            }
            _ => {
                self.errors.borrow_mut().remove(&id);
            }
        }
    }

    pub fn get_errors(&self, id: u32) -> Option<Vec<ErrorDescription>> {
        self.errors.borrow().get(&id).cloned()
    }

    /// number of children
    pub fn child_count(&self, id: u32) -> u32 {
        self.child_iter(id).count() as u32
    }

    /// properties of the arena
    pub fn export_properties(&self) -> ArenaProperties {
        ArenaProperties {
            kind: "Arena".into(),
            current: self.current.get(),
            strings: self.strings().export(),
            errors: self.errors.borrow().iter().map(|(k, v)| (*k, v.clone())).collect(),
            sources: None,
            input_lengths: None,
        }
    }

    /// copy of the columns
    pub fn export_transferables(&self) -> ArenaTransferables {
        ArenaTransferables {
            data_types: self.data_types.to_vec(),
            parents: self.parents.to_vec(),
            next_sibling: self.next_sibling.to_vec(),
            previous_sibling: self.previous_sibling.to_vec(),
            first_token: self.first_token.to_vec(),
            last_token: self.last_token.to_vec(),
            first_child: self.first_child.to_vec(),
            last_child: self.last_child.to_vec(),
            name: self.name.to_vec(),
            string_value: self.string_value.to_vec(),
            numeric_value: self.numeric_value.to_vec(),
            unit: self.unit.to_vec(),
            flags: self.flags.to_vec(),
            color_type: self.color_type.to_vec(),
            spans: self.spans.to_vec(),
        }
    }

    /// move the columns out (transfer): the arena can no longer be used
    pub fn take_transferables(&self) -> ArenaTransferables {
        let t = ArenaTransferables {
            data_types: self.data_types.replace(Vec::new()),
            parents: self.parents.replace(Vec::new()),
            next_sibling: self.next_sibling.replace(Vec::new()),
            previous_sibling: self.previous_sibling.replace(Vec::new()),
            first_token: self.first_token.replace(Vec::new()),
            last_token: self.last_token.replace(Vec::new()),
            first_child: self.first_child.replace(Vec::new()),
            last_child: self.last_child.replace(Vec::new()),
            name: self.name.replace(Vec::new()),
            string_value: self.string_value.replace(Vec::new()),
            numeric_value: self.numeric_value.replace(Vec::new()),
            unit: self.unit.replace(Vec::new()),
            flags: self.flags.replace(Vec::new()),
            color_type: self.color_type.replace(Vec::new()),
            spans: self.spans.replace(Vec::new()),
        };
        self.transferred.set(true);
        self.generation.set(self.generation.get() + 1);
        t
    }

    /// initialize the arena from exported properties and column buffers
    pub fn import_properties(&self, properties: &ArenaProperties, transferables: ArenaTransferables) {
        self.import_columns(transferables);
        self.current.set(properties.current);
        self.strings().import(&properties.strings);
        *self.errors.borrow_mut() = properties.errors.iter().cloned().collect();
        self.transferred.set(false);
    }

    /// link a node before a reference node of the given parent
    fn insert_before(&self, parent: u32, reference: u32, node: u32) {
        let previous = self.previous_sibling.get(reference);
        self.previous_sibling.set(node, previous);
        self.next_sibling.set(node, reference);
        self.previous_sibling.set(reference, node);
        self.parents.set(node, parent);
        if previous != 0 {
            self.next_sibling.set(previous, node);
        } else if self.first_child.get(parent) == reference {
            self.first_child.set(parent, node);
        } else if self.first_token.get(parent) == reference {
            self.first_token.set(parent, node);
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// AstArena
// ---------------------------------------------------------------------------------------------------------------------

use crate::plain::{Plain, PlainObject};
use crate::types::SourceLocation;

/// raw selector (`RawSelectorTokens`)
pub type RawSelectorTokens = Vec<Vec<String>>;

/// a split selector of the minification cache (`arena.raw`) with its vendor prefixes, computed once. a mutable access
/// clears the prefixes
#[derive(Clone, Debug, Default)]
pub struct RawSelector {
    tokens: RawSelectorTokens,
    prefixes: std::cell::OnceCell<Rc<[String]>>,
}

impl RawSelector {
    /// the vendor prefixes of the selector, computed once by `compute`
    pub fn prefixes(&self, compute: impl FnOnce(&RawSelectorTokens) -> Vec<String>) -> Rc<[String]> {
        self.prefixes.get_or_init(|| compute(&self.tokens).into()).clone()
    }
}

impl From<RawSelectorTokens> for RawSelector {
    fn from(tokens: RawSelectorTokens) -> Self {
        Self { tokens, prefixes: std::cell::OnceCell::new() }
    }
}

impl std::ops::Deref for RawSelector {
    type Target = RawSelectorTokens;

    fn deref(&self) -> &RawSelectorTokens {
        &self.tokens
    }
}

impl std::ops::DerefMut for RawSelector {
    fn deref_mut(&mut self) -> &mut RawSelectorTokens {
        // the tokens may change: the prefixes are computed again
        self.prefixes = std::cell::OnceCell::new();
        &mut self.tokens
    }
}

/// optimized selector shared by reference, like the typescript objects (the minifier mutates it in place and a cloned
/// rule shares the object of its source)
pub type SharedOptimizedSelector = Rc<RefCell<OptimizedSelector>>;

/// optimized selector (`OptimizedSelector`)
#[derive(Clone, Debug, PartialEq)]
pub struct OptimizedSelector {
    pub match_: bool,
    pub optimized: Vec<String>,
    pub selector: Vec<Vec<String>>,
    pub reducible: bool,
}

/// true for the node types whose plain object is a node (`isNodeObjectType()` of `arena-node.ts`)
pub fn is_node_object_type(typ: u8) -> bool {
    matches!(
        typ,
        EnumToken::StyleSheetNodeType
            | EnumToken::RuleNodeType
            | EnumToken::KeyframesRuleNodeType
            | EnumToken::AtRuleNodeType
            | EnumToken::KeyframesAtRuleNodeType
            | EnumToken::CommentNodeType
            | EnumToken::CDOCOMMNodeType
            | EnumToken::DeclarationNodeType
            | EnumToken::CssVariableTokenType
            | EnumToken::CssVariableImportTokenType
            | EnumToken::CssVariableDeclarationMapTokenType
    )
}

/// a string or an interned string id (`sel: string | number` arguments)
#[derive(Clone, Copy, Debug)]
pub enum StrOrId<'a> {
    Str(&'a str),
    Id(u32),
}

impl<'a> From<&'a str> for StrOrId<'a> {
    fn from(s: &'a str) -> Self {
        StrOrId::Str(s)
    }
}

impl<'a> From<&'a String> for StrOrId<'a> {
    fn from(s: &'a String) -> Self {
        StrOrId::Str(s)
    }
}

impl From<u32> for StrOrId<'_> {
    fn from(id: u32) -> Self {
        StrOrId::Id(id)
    }
}

/// a token value: a string or a number (`val?: string | number | null`)
#[derive(Clone, Copy, Debug)]
pub enum TokenValue<'a> {
    None,
    Str(&'a str),
    Num(f64),
}

/// Css ast arena: ast nodes and value tokens. Derefs to [`Arena`] (typescript `extends`).
pub struct AstArena {
    base: Arena,
    /// raw selector (minification cache), by rule id
    /// split selectors by rule, shared: a read does not copy the selector
    pub raw: RefCell<HashMap<u32, Rc<RawSelector>>>,
    /// optimized selector (minification cache), by rule id
    pub optimized: RefCell<HashMap<u32, SharedOptimizedSelector>>,
    /// first part of the first selector of a selector string (minification cache), by string id: the string epoch the
    /// ids belong to, then the code units (`None` when the first selector is empty)
    pub first_parts: RefCell<(u64, HashMap<u32, Option<Rc<[u16]>>>)>,
    /// vendor prefixes of a selector string (minification cache), by string id, see `first_parts`
    pub selector_prefixes: RefCell<(u64, HashMap<u32, Rc<[String]>>)>,
    /// source ids. `spans` store the index in this table (8 bits)
    pub sources: RefCell<Vec<u32>>,
    /// length of the css parsed into a stylesheet, by stylesheet id
    pub input_lengths: RefCell<HashMap<u32, u32>>,
    /// the shared handle of the arena when it is created by [`AstArena::shared`]: the functions that receive a
    /// reference can parse into the arena (`parseDeclarations(css, arena)`)
    this: RefCell<std::rc::Weak<AstArena>>,
}

impl std::ops::Deref for AstArena {
    type Target = Arena;

    fn deref(&self) -> &Arena {
        &self.base
    }
}

impl Default for AstArena {
    fn default() -> Self {
        Self::new(4096)
    }
}

impl AstArena {
    /// a shared arena: [`AstArena::rc`] returns its handle
    pub fn shared(size: usize) -> Rc<Self> {
        let arena = Rc::new(Self::new(size));
        *arena.this.borrow_mut() = Rc::downgrade(&arena);
        arena
    }

    /// register the shared handle of an arena (an arena created by `Rc::new`)
    pub fn register(arena: &Rc<Self>) {
        *arena.this.borrow_mut() = Rc::downgrade(arena);
    }

    /// the shared handle of the arena
    pub fn rc(&self) -> Rc<AstArena> {
        self.this.borrow().upgrade().expect("the arena is not shared: create it with AstArena::shared()")
    }

    pub fn new(size: usize) -> Self {
        let arena = Self {
            base: Arena::new(size),
            raw: RefCell::new(HashMap::default()),
            optimized: RefCell::new(HashMap::default()),
            first_parts: RefCell::new((0, HashMap::default())),
            selector_prefixes: RefCell::new((0, HashMap::default())),
            sources: RefCell::new(Vec::new()),
            input_lengths: RefCell::new(HashMap::default()),
            this: RefCell::new(std::rc::Weak::new()),
        };
        // string id 0 is the null / empty string
        arena.intern_string("");
        arena.flags.set(arena.root, EnumAstNodeStatus::HasChildren);
        arena
    }

    /// the base arena
    pub fn base(&self) -> &Arena {
        &self.base
    }

    pub fn release(&self, id: Option<u32>, deep: bool) {
        self.base.release(id, deep);
        if let Some(id) = id {
            self.raw.borrow_mut().remove(&id);
            self.optimized.borrow_mut().remove(&id);
            self.input_lengths.borrow_mut().remove(&id);
        }
    }

    pub fn compact(&self, errors: Option<&mut [ErrorDescription]>) -> Vec<u32> {
        let mapping = self.base.compact(errors);
        self.raw.borrow_mut().clear();
        self.optimized.borrow_mut().clear();
        let lengths: HashMap<u32, u32> = self
            .input_lengths
            .borrow()
            .iter()
            .filter_map(|(&id, &len)| mapping.get(id as usize).filter(|&&m| m != 0).map(|&m| (m, len)))
            .collect();
        *self.input_lengths.borrow_mut() = lengths;
        mapping
    }

    pub fn trim(&self, errors: Option<&mut [ErrorDescription]>) -> Vec<u32> {
        self.release(None, false);
        self.compact(errors)
    }

    /// validation state bits
    pub fn get_state(&self, id: u32) -> u32 {
        self.flags.get(id) & EnumAstNodeStatus::StateMask
    }

    /// set the validation state bits
    pub fn set_state(&self, id: u32, state: u32) {
        self.flags.set(id, (self.flags.get(id) & !EnumAstNodeStatus::StateMask) | (state & EnumAstNodeStatus::StateMask));
    }

    pub fn is_invalid(&self, id: u32) -> bool {
        self.flags.get(id) & EnumAstNodeStatus::Invalid != 0
    }

    /// index of a source id in the sources table
    pub fn source_index(&self, src_id: u32) -> u32 {
        let mut sources = self.sources.borrow_mut();
        match sources.iter().position(|&s| s == src_id) {
            Some(index) => index as u32,
            None => {
                sources.push(src_id);
                sources.len() as u32 - 1
            }
        }
    }

    /// set the source location. `None` for both the source and the start drops the location
    pub fn set_location(&self, id: u32, src_id: Option<u32>, sta: Option<u32>, end: Option<u32>) {
        if src_id.is_none() && sta.is_none() {
            self.flags.set(id, self.flags.get(id) & !EnumAstNodeStatus::HasLocation);
            self.spans.set(id, 0);
            return;
        }
        self.flags.set(id, self.flags.get(id) | EnumAstNodeStatus::HasLocation);
        self.set_span(id, self.source_index(src_id.unwrap_or(0)), sta.unwrap_or(0), end.unwrap_or(0));
    }

    /// set the location of a token from the tokenizer values
    #[inline]
    pub fn set_token_location(&self, id: u32, source_index: u32, sta: u32, end: u32) {
        self.flags.set(id, self.flags.get(id) | EnumAstNodeStatus::HasLocation);
        self.set_span(id, source_index, sta, end);
    }

    /// copy the source location of another node
    pub fn copy_location(&self, id: u32, source: u32) {
        self.copy_span(id, source);
        self.flags.set(
            id,
            (self.flags.get(id) & !EnumAstNodeStatus::HasLocation) | (self.flags.get(source) & EnumAstNodeStatus::HasLocation),
        );
    }

    /// set the location from the start of a node to the end of another one
    pub fn span_location(&self, id: u32, start: u32, end: u32) {
        if self.flags.get(start) & EnumAstNodeStatus::HasLocation == 0 {
            self.copy_location(id, end);
            return;
        }
        self.flags.set(id, self.flags.get(id) | EnumAstNodeStatus::HasLocation);
        self.set_span(
            id,
            self.get_source_id(start),
            self.get_span_start(start),
            if self.flags.get(end) & EnumAstNodeStatus::HasLocation != 0 { self.get_span_end(end) } else { self.get_span_end(start) },
        );
    }

    pub fn has_location(&self, id: u32) -> bool {
        self.flags.get(id) & EnumAstNodeStatus::HasLocation != 0
    }

    /// source location
    pub fn get_location(&self, id: u32) -> Option<SourceLocation> {
        if self.flags.get(id) & EnumAstNodeStatus::HasLocation == 0 {
            return None;
        }
        Some(SourceLocation { sta: self.get_span_start(id), end: self.get_span_end(id), src_id: self.get_source(id) })
    }

    /// source id of the node location
    pub fn get_source(&self, id: u32) -> u32 {
        self.sources.borrow().get(self.get_source_id(id) as usize).copied().unwrap_or(0)
    }

    fn intern_str_or_id(&self, value: StrOrId) -> u32 {
        match value {
            StrOrId::Str(s) => self.intern_string(s),
            StrOrId::Id(id) => id,
        }
    }

    // ---- ast nodes -------------------------------------------------------------------

    pub fn create_style_sheet(&self) -> u32 {
        let id = self.allocate();
        self.data_types.set(id, EnumToken::StyleSheetNodeType);
        self.flags.set(id, EnumAstNodeStatus::HasChildren);
        id
    }

    pub fn create_rule(&self, typ: u8, sel: StrOrId, tokens: Option<&[u32]>, state: u32, errors: Option<Vec<ErrorDescription>>) -> u32 {
        let id = self.allocate();
        self.data_types.set(id, typ);
        self.name.set(id, self.intern_str_or_id(sel));
        self.flags.set(id, EnumAstNodeStatus::HasChildren | (state & EnumAstNodeStatus::StateMask));
        if let Some(tokens) = tokens {
            self.append_token_list(id, tokens);
        }
        if let Some(errors) = errors.filter(|e| !e.is_empty()) {
            self.errors.borrow_mut().insert(id, errors);
        }
        id
    }

    pub fn create_at_rule(
        &self,
        typ: u8,
        nam: StrOrId,
        val: StrOrId,
        block: bool,
        tokens: Option<&[u32]>,
        state: u32,
        errors: Option<Vec<ErrorDescription>>,
    ) -> u32 {
        let id = self.allocate();
        self.data_types.set(id, typ);
        self.name.set(id, self.intern_str_or_id(nam));
        self.string_value.set(id, self.intern_str_or_id(val));
        self.flags.set(id, (if block { EnumAstNodeStatus::HasChildren } else { 0 }) | (state & EnumAstNodeStatus::StateMask));
        if let Some(tokens) = tokens {
            self.append_token_list(id, tokens);
        }
        if let Some(errors) = errors.filter(|e| !e.is_empty()) {
            self.errors.borrow_mut().insert(id, errors);
        }
        id
    }

    pub fn create_declaration(&self, typ: u8, nam: StrOrId, val: Option<&[u32]>, state: u32, errors: Option<Vec<ErrorDescription>>) -> u32 {
        let id = self.allocate();
        self.data_types.set(id, typ);
        self.name.set(id, self.intern_str_or_id(nam));
        self.flags.set(id, EnumAstNodeStatus::HasValues | (state & EnumAstNodeStatus::StateMask));
        if let Some(val) = val {
            self.append_token_list(id, val);
        }
        if let Some(errors) = errors.filter(|e| !e.is_empty()) {
            self.errors.borrow_mut().insert(id, errors);
        }
        id
    }

    pub fn create_comment(&self, typ: u8, val: StrOrId) -> u32 {
        let id = self.allocate();
        self.data_types.set(id, typ);
        self.string_value.set(id, self.intern_str_or_id(val));
        self.flags.set(id, 0);
        id
    }

    // ---- tokens ----------------------------------------------------------------------

    /// allocate a token. string values are interned, numeric values are stored in the `numeric_value` column
    pub fn create_token(&self, typ: u8, val: TokenValue, unit: Option<&str>) -> u32 {
        let id = self.allocate();
        self.data_types.set(id, typ);
        match val {
            TokenValue::Str(s) => self.string_value.set(id, self.intern_string(s)),
            TokenValue::Num(n) => self.numeric_value.set(id, n),
            TokenValue::None => {}
        }
        if let Some(unit) = unit {
            self.unit.set(id, self.intern_string(unit));
        }
        id
    }

    /// allocate a token with a string value
    pub fn create_str_token(&self, typ: u8, val: &str) -> u32 {
        self.create_token(typ, TokenValue::Str(val), None)
    }

    /// allocate a token with a numeric value
    pub fn create_num_token(&self, typ: u8, val: f64, unit: Option<&str>) -> u32 {
        self.create_token(typ, TokenValue::Num(val), unit)
    }

    /// allocate a color token
    pub fn create_color(&self, val: &str, kin: u8, tokens: Option<&[u32]>) -> u32 {
        let id = self.allocate();
        self.data_types.set(id, EnumToken::ColorTokenType);
        self.string_value.set(id, self.intern_string(val));
        self.color_type.set(id, kin);
        if let Some(tokens) = tokens {
            self.append_token_list(id, tokens);
        }
        id
    }

    /// allocate a function token
    pub fn create_function(&self, typ: u8, name: &str, tokens: Option<&[u32]>) -> u32 {
        let id = self.allocate();
        self.data_types.set(id, typ);
        self.string_value.set(id, self.intern_string(name));
        if let Some(tokens) = tokens {
            self.append_token_list(id, tokens);
        }
        id
    }

    /// allocate a list token (parens, attribute, wrapped values, ...)
    pub fn create_list(&self, typ: u8, tokens: Option<&[u32]>) -> u32 {
        let id = self.allocate();
        self.data_types.set(id, typ);
        if let Some(tokens) = tokens {
            self.append_token_list(id, tokens);
        }
        id
    }

    /// allocate a binary expression token. the operator is stored in the `numeric_value` column
    pub fn create_binary_expression(&self, op: u8, l: u32, r: u32) -> u32 {
        let id = self.allocate();
        self.data_types.set(id, EnumToken::BinaryExpressionTokenType);
        self.numeric_value.set(id, op as f64);
        self.set_token_list(self.field_list(id, TokenField::L), &[l]);
        self.set_token_list(self.field_list(id, TokenField::R), &[r]);
        id
    }

    /// allocate a fraction token
    pub fn create_fraction(&self, l: u32, r: u32) -> u32 {
        let id = self.allocate();
        self.data_types.set(id, EnumToken::FractionTokenType);
        self.append_child_list(id, &[l, r]);
        id
    }

    /// true if the numeric token value is a fraction
    pub fn has_fraction(&self, id: u32) -> bool {
        let child = self.first_child.get(id);
        child != 0 && self.data_types.get(child) == EnumToken::FractionTokenType
    }

    /// fraction token of a numeric token
    pub fn get_fraction(&self, id: u32) -> u32 {
        let child = self.first_child.get(id);
        if child != 0 && self.data_types.get(child) == EnumToken::FractionTokenType {
            child
        } else {
            0
        }
    }

    /// set the fraction value of a numeric token. the numeric value is set to the fraction value
    pub fn set_fraction(&self, id: u32, fraction: Option<u32>) {
        let child = self.get_fraction(id);
        if child != 0 {
            self.detach(child);
        }
        if let Some(fraction) = fraction.filter(|&f| f != 0) {
            self.append_child_list(id, &[fraction]);
            self.numeric_value.set(
                id,
                self.numeric_value.get(self.first_child.get(fraction)) / self.numeric_value.get(self.last_child.get(fraction)),
            );
        }
    }

    pub fn numerator(&self, id: u32) -> u32 {
        self.first_child.get(id)
    }

    pub fn denominator(&self, id: u32) -> u32 {
        self.last_child.get(id)
    }

    /// string value of a token (`val`)
    pub fn str(&self, id: u32) -> SharedStr {
        self.resolve_string(self.string_value.get(id))
    }

    /// set the string value of a token (`val`)
    pub fn set_str(&self, id: u32, value: &str) {
        self.string_value.set(id, self.intern_string(value));
    }

    /// set the unit of a token
    pub fn set_unit_string(&self, id: u32, unit: Option<&str>) {
        self.unit.set(id, unit.map_or(0, |u| self.intern_string(u)));
    }

    /// true if the token is a numeric token
    pub fn is_numeric(&self, id: u32) -> bool {
        is_numeric_token_type(self.data_types.get(id))
    }

    /// replace a token by other tokens in its parent token list
    pub fn replace_token(&self, id: u32, tokens: &[u32]) {
        self.replace_child(id, tokens);
    }

    /// a token that can be attached to a new parent: the token itself when it is detached, a deep clone otherwise
    pub fn take(&self, token: u32) -> u32 {
        if self.parents.get(token) == 0 {
            token
        } else {
            self.clone_node_deep(token, true, None)
        }
    }

    /// copy the data columns of a token
    pub fn copy_token(&self, target: u32, source: u32) {
        self.data_types.set(target, self.data_types.get(source));
        self.name.set(target, self.name.get(source));
        self.string_value.set(target, self.string_value.get(source));
        self.numeric_value.set(target, self.numeric_value.get(source));
        self.unit.set(target, self.unit.get(source));
        self.color_type.set(target, self.color_type.get(source));
        self.copy_span(target, source);
        self.flags.set(target, self.flags.get(source) & !(EnumAstNodeStatus::HasChildren | EnumAstNodeStatus::HasValues));
    }

    /// reset a token to a new type. the values, the token list and the fields are dropped
    pub fn reset_token(&self, id: u32, typ: u8, val: TokenValue, unit: Option<&str>) {
        self.data_types.set(id, typ);
        self.string_value.set(id, 0);
        self.numeric_value.set(id, 0.0);
        self.unit.set(id, 0);
        self.color_type.set(id, 0);
        self.name.set(id, 0);
        match val {
            TokenValue::Str(s) => self.string_value.set(id, self.intern_string(s)),
            TokenValue::Num(n) => self.numeric_value.set(id, n),
            TokenValue::None => {}
        }
        if let Some(unit) = unit {
            self.unit.set(id, self.intern_string(unit));
        }
        self.set_token_list(id, &[]);
        self.set_child_list(id, &[]);
    }

    // ---- plain objects -----------------------------------------------------------------

    /// import a plain object (`PlainAstNode` / `PlainToken`). numbers in `chi`, `val`, `l`, ... are node ids
    pub fn from_object(&self, node: &Plain) -> u32 {
        if let Some(id) = node.as_f64() {
            return id as u32;
        }
        let typ = node.get("typ").as_f64().unwrap_or(0.0) as u8;
        if is_node_object_type(typ) {
            self.node_from_object(node)
        } else {
            self.token_from_object(node)
        }
    }

    fn location_to_plain(loc: SourceLocation) -> Plain {
        let mut o = PlainObject::new();
        o.set("sta", loc.sta);
        o.set("end", loc.end);
        o.set("srcId", loc.src_id);
        Plain::Object(o)
    }

    /// export a node or a token as a plain object (deep copy)
    pub fn to_plain_object(&self, id: u32) -> Plain {
        let typ = self.data_types.get(id);
        let flags = self.flags.get(id);
        if !is_node_object_type(typ) {
            return self.token_to_object(id);
        }
        let mut node = PlainObject::new();
        node.set("typ", typ);
        match typ {
            EnumToken::StyleSheetNodeType => {}
            EnumToken::RuleNodeType | EnumToken::KeyframesRuleNodeType => {
                node.set("sel", self.resolve_string(self.name.get(id)));
            }
            EnumToken::AtRuleNodeType | EnumToken::KeyframesAtRuleNodeType => {
                node.set("nam", self.resolve_string(self.name.get(id)));
                node.set("val", self.resolve_string(self.string_value.get(id)));
            }
            EnumToken::CommentNodeType | EnumToken::CDOCOMMNodeType => {
                node.set("val", self.resolve_string(self.string_value.get(id)));
            }
            _ => {
                node.set("nam", self.resolve_string(self.name.get(id)));
                node.set("val", Plain::Array(self.tokens(id).into_iter().map(|t| self.token_to_object(t)).collect()));
            }
        }
        if flags & EnumAstNodeStatus::HasChildren != 0 {
            node.set("chi", Plain::Array(self.children(id).into_iter().map(|c| self.to_plain_object(c)).collect()));
        }
        if flags & EnumAstNodeStatus::HasLocation != 0 {
            node.set("loc", self.get_location(id).map_or(Plain::Null, Self::location_to_plain));
        }
        if flags & EnumAstNodeStatus::StateMask != 0 {
            node.set("state", flags & EnumAstNodeStatus::StateMask);
        }
        if flags & EnumAstNodeStatus::HasValues == 0 && self.first_token.get(id) != 0 {
            node.set("tokens", Plain::Array(self.tokens(id).into_iter().map(|t| self.token_to_object(t)).collect()));
        }
        if let Some(errors) = self.errors.borrow().get(&id) {
            node.set("errors", Plain::Array(errors.iter().map(|e| e.to_plain()).collect()));
        }
        if let Some(raw) = self.raw.borrow().get(&id) {
            node.set("raw", raw_to_plain(raw));
        }
        if let Some(optimized) = self.optimized.borrow().get(&id) {
            let optimized = optimized.borrow();
            let mut o = PlainObject::new();
            o.set("match", optimized.match_);
            o.set("optimized", Plain::Array(optimized.optimized.iter().map(|s| Plain::from(s.as_str())).collect()));
            o.set("selector", raw_to_plain(&optimized.selector));
            o.set("reducible", optimized.reducible);
            node.set("optimized", o);
        }
        Plain::Object(node)
    }

    /// export a token as a plain object (deep copy)
    pub fn token_to_object(&self, id: u32) -> Plain {
        let typ = self.data_types.get(id);
        let mut token = PlainObject::new();
        token.set("typ", typ);
        if self.name.get(id) != 0 {
            token.set("nam", self.resolve_string(self.name.get(id)));
        }
        if typ == EnumToken::BinaryExpressionTokenType {
            token.set("op", self.numeric_value.get(id));
        } else if typ == EnumToken::UnaryExpressionTokenType {
            token.set("sign", self.numeric_value.get(id));
            let child = self.first_child.get(id);
            token.set("val", if child == 0 { Plain::Null } else { self.token_to_object(child) });
        } else if typ == EnumToken::FractionTokenType {
            token.set("l", self.token_to_object(self.first_child.get(id)));
            token.set("r", self.token_to_object(self.last_child.get(id)));
        } else if is_numeric_token_type(typ) {
            let fraction = self.get_fraction(id);
            token.set("val", if fraction == 0 { Plain::Number(self.numeric_value.get(id)) } else { self.token_to_object(fraction) });
        } else if self.string_value.get(id) != 0 || typ == EnumToken::IdenTokenType || typ == EnumToken::StringTokenType {
            token.set("val", self.resolve_string(self.string_value.get(id)));
        }
        if self.unit.get(id) != 0 {
            let unit = self.resolve_string(self.unit.get(id));
            if typ == EnumToken::ColorTokenType {
                token.set("cal", unit);
            } else if typ == EnumToken::MatchExpressionTokenType {
                token.set("attr", unit);
            } else {
                token.set("unit", unit);
            }
        }
        if typ == EnumToken::ColorTokenType && self.color_type.get(id) != crate::ast::types::ColorType::NONE {
            token.set("kin", self.color_type.get(id));
        }
        if self.first_token.get(id) != 0 {
            let tokens = Plain::Array(self.tokens(id).into_iter().map(|t| self.token_to_object(t)).collect());
            if matches!(typ, EnumToken::DeclarationNodeType | EnumToken::CssVariableTokenType | EnumToken::CssVariableImportTokenType) {
                token.set("val", tokens);
            } else {
                token.set("chi", tokens);
            }
        }
        if is_structured_token_type(typ) {
            for (key, field, single) in structured_fields(typ) {
                let Some(list) = self.get_child_at(id, field) else {
                    continue;
                };
                let mut values: Vec<Plain> = self.tokens(list).into_iter().map(|t| self.token_to_object(t)).collect();
                token.set(key, if single { if values.is_empty() { Plain::Null } else { values.swap_remove(0) } } else { Plain::Array(values) });
            }
        }
        if self.flags.get(id) & EnumAstNodeStatus::HasLocation != 0 {
            token.set("loc", self.get_location(id).map_or(Plain::Null, Self::location_to_plain));
        }
        Plain::Object(token)
    }

    /// copy the data of a node (type, strings, values, flags, location, errors)
    pub fn clone_node(&self, id: u32) -> u32 {
        let target = self.allocate();
        self.data_types.set(target, self.data_types.get(id));
        self.name.set(target, self.name.get(id));
        self.string_value.set(target, self.string_value.get(id));
        self.numeric_value.set(target, self.numeric_value.get(id));
        self.unit.set(target, self.unit.get(id));
        self.color_type.set(target, self.color_type.get(id));
        self.flags.set(target, self.flags.get(id));
        self.copy_span(target, id);
        let errors = self.errors.borrow().get(&id).cloned();
        if let Some(errors) = errors {
            self.errors.borrow_mut().insert(target, errors);
        }
        target
    }

    /// copy a node or a token (`clone(id, deep, cloneMap)`). the tokens of a node are copied, its children are copied
    /// with `deep`. the values of a declaration are copied with `deep`
    pub fn clone_node_deep(&self, id: u32, deep: bool, mut clone_map: Option<&mut HashMap<u32, u32>>) -> u32 {
        let target = self.allocate();
        let flags = self.flags.get(id);
        if let Some(map) = clone_map.as_deref_mut() {
            map.insert(id, target);
        }
        self.data_types.set(target, self.data_types.get(id));
        self.name.set(target, self.name.get(id));
        self.string_value.set(target, self.string_value.get(id));
        self.numeric_value.set(target, self.numeric_value.get(id));
        self.unit.set(target, self.unit.get(id));
        self.color_type.set(target, self.color_type.get(id));
        self.flags.set(target, flags);
        self.copy_span(target, id);
        let raw = self.raw.borrow().get(&id).cloned();
        if let Some(raw) = raw {
            self.raw.borrow_mut().insert(target, raw);
        }
        let optimized = self.optimized.borrow().get(&id).cloned();
        if let Some(optimized) = optimized {
            self.optimized.borrow_mut().insert(target, optimized);
        }
        let errors = self.errors.borrow().get(&id).cloned();
        if let Some(errors) = errors {
            self.errors.borrow_mut().insert(target, errors);
        }
        let is_node = flags & (EnumAstNodeStatus::HasChildren | EnumAstNodeStatus::HasValues) != 0;
        if !is_node || deep || flags & EnumAstNodeStatus::HasValues == 0 {
            let mut token = self.first_token.get(id);
            while token != 0 {
                let copy = self.clone_node_deep(token, true, clone_map.as_deref_mut());
                self.append_token(target, copy);
                token = self.next_sibling.get(token);
            }
        }
        if !is_node || deep {
            let mut child = self.first_child.get(id);
            while child != 0 {
                let copy = self.clone_node_deep(child, true, clone_map.as_deref_mut());
                self.append_child(target, copy);
                child = self.next_sibling.get(child);
            }
        }
        target
    }

    /// `clone(id)` with the default arguments
    pub fn clone(&self, id: u32) -> u32 {
        self.clone_node_deep(id, false, None)
    }

    /// copy a node or a token of another arena into this arena
    pub fn import(&self, source: &AstArena, id: u32, deep: bool) -> u32 {
        if std::ptr::eq(source, self) {
            return id;
        }
        let target = self.allocate();
        let flags = source.flags.get(id);
        self.data_types.set(target, source.data_types.get(id));
        self.name.set(target, self.intern_shared(&source.resolve_string(source.name.get(id))));
        self.string_value.set(target, self.intern_shared(&source.resolve_string(source.string_value.get(id))));
        self.numeric_value.set(target, source.numeric_value.get(id));
        self.unit.set(target, self.intern_shared(&source.resolve_string(source.unit.get(id))));
        self.color_type.set(target, source.color_type.get(id));
        self.flags.set(target, flags & !EnumAstNodeStatus::HasLocation);
        if let Some(raw) = source.raw.borrow().get(&id) {
            self.raw.borrow_mut().insert(target, raw.clone());
        }
        if let Some(optimized) = source.optimized.borrow().get(&id) {
            self.optimized.borrow_mut().insert(target, optimized.clone());
        }
        if flags & EnumAstNodeStatus::HasLocation != 0 {
            self.set_location(target, Some(source.get_source(id)), Some(source.get_span_start(id)), Some(source.get_span_end(id)));
        }
        if let Some(errors) = source.errors.borrow().get(&id) {
            self.errors.borrow_mut().insert(target, errors.clone());
        }
        let mut token = source.first_token.get(id);
        while token != 0 {
            let copy = self.import(source, token, true);
            self.append_token(target, copy);
            token = source.next_sibling.get(token);
        }
        if deep || flags & EnumAstNodeStatus::HasChildren == 0 {
            let mut child = source.first_child.get(id);
            while child != 0 {
                let copy = self.import(source, child, true);
                self.append_child(target, copy);
                child = source.next_sibling.get(child);
            }
        }
        target
    }

    /// index of a child in its parent's children (or tokens) list
    pub fn index_of(&self, id: u32) -> i32 {
        if self.parents.get(id) == 0 {
            return -1;
        }
        let mut index = 0;
        let mut child = self.previous_sibling.get(id);
        while child != 0 {
            index += 1;
            child = self.previous_sibling.get(child);
        }
        index
    }

    /// insert nodes before a child
    pub fn insert_before(&self, reference: u32, nodes: &[u32]) {
        for &node in nodes {
            if node == reference || node == 0 {
                continue;
            }
            self.set_previous_sibling(reference, node);
        }
    }

    /// insert nodes after a child
    pub fn insert_after(&self, mut reference: u32, nodes: &[u32]) {
        for &node in nodes {
            if node == reference || node == 0 {
                continue;
            }
            self.set_next_sibling(reference, node);
            reference = node;
        }
    }

    /// prepend children
    pub fn prepend_children(&self, id: u32, nodes: &[u32]) {
        let first = self.first_child.get(id);
        if first == 0 {
            self.append_child_list(id, nodes);
        } else {
            self.insert_before(first, nodes);
        }
    }

    /// replace a child with other nodes. the node itself may be part of the replacement
    pub fn replace_child(&self, id: u32, nodes: &[u32]) {
        if self.parents.get(id) == 0 {
            return;
        }
        match nodes.iter().position(|&n| n == id) {
            None => {
                self.insert_before(id, nodes);
                self.detach(id);
            }
            Some(index) => {
                self.insert_before(id, &nodes[..index]);
                self.insert_after(id, &nodes[index + 1..]);
            }
        }
    }

    pub fn has_children(&self, id: u32) -> bool {
        self.flags.get(id) & EnumAstNodeStatus::HasChildren != 0
    }

    /// rule selector
    pub fn get_selector(&self, id: u32) -> SharedStr {
        self.resolve_string(self.name.get(id))
    }

    pub fn set_selector(&self, id: u32, sel: &str) {
        self.name.set(id, self.intern_string(sel));
    }

    pub fn set_name_string(&self, id: u32, nam: &str) {
        self.name.set(id, self.intern_string(nam));
    }

    pub fn set_value_string(&self, id: u32, val: &str) {
        self.string_value.set(id, self.intern_string(val));
    }

    // ---- nesting rules expansion ---------------------------------------------------------------

    /// expand the css nesting rules of a stylesheet, a rule or an at-rule, in place
    pub fn expand(&self, ast: u32) -> u32 {
        if self.flags.get(ast) & SKIP_STATES != 0 {
            return ast;
        }
        let mut node = self.first_child.get(ast);
        while node != 0 {
            let next = self.next_sibling.get(node);
            let typ = self.data_types.get(node);
            if typ == EnumToken::RuleNodeType {
                self.expand_rule(node, ast, next);
            } else if typ == EnumToken::AtRuleNodeType
                && self.flags.get(node) & EnumAstNodeStatus::HasChildren != 0
                && !self.string_equals(self.name.get(node), "mixin")
            {
                let mut j = self.last_child.get(node);
                while j != 0 {
                    if matches!(self.data_types.get(j), EnumToken::RuleNodeType | EnumToken::AtRuleNodeType) {
                        self.expand(node);
                        break;
                    }
                    j = self.previous_sibling.get(j);
                }
            }
            node = next;
        }
        ast
    }

    /// replace the nesting selector '&' of a selector
    pub fn replace_compound(&self, input: &str, replace: &str) -> String {
        let tokens = crate::parser::parse::parse_string(self, input);
        let mut walker = ValueWalker::of_tokens(&tokens, WalkerValueOptions::default(), false);
        while let Some(t) = walker.next(self) {
            if self.data_types.get(t) == EnumToken::NestingSelectorTokenType {
                if tokens.len() == 2 {
                    let literal = self.replace_compound_literal(&self.resolve_string(self.string_value.get(t)), replace);
                    self.reset_token(t, EnumToken::LiteralTokenType, TokenValue::Str(&literal), None);
                    continue;
                }
                let rule = crate::ast::minify::split_rule(replace);
                let value = if rule.len() > 1 { format!(":is({})", replace) } else { replace.to_string() };
                self.reset_token(t, EnumToken::LiteralTokenType, TokenValue::Str(&value), None);
            }
        }
        tokens.iter().map(|&t| crate::printer::render::render_value_default(self, t)).collect()
    }

    /// properties of the arena
    pub fn export_properties(&self) -> ArenaProperties {
        let mut properties = self.base.export_properties();
        properties.kind = "AstArena".into();
        properties.sources = Some(self.sources.borrow().clone());
        let lengths = self.input_lengths.borrow();
        let max = lengths.keys().max().map_or(0, |&m| m as usize + 1);
        properties.input_lengths = Some((0..max).map(|i| lengths.get(&(i as u32)).copied()).collect());
        properties
    }

    /// initialize the arena from exported properties and column buffers
    pub fn import_properties(&self, properties: &ArenaProperties, transferables: ArenaTransferables) {
        self.base.import_properties(properties, transferables);
        *self.sources.borrow_mut() = properties.sources.clone().unwrap_or_default();
        self.raw.borrow_mut().clear();
        self.optimized.borrow_mut().clear();
        *self.input_lengths.borrow_mut() = properties
            .input_lengths
            .as_ref()
            .map(|l| l.iter().enumerate().filter_map(|(i, v)| v.map(|v| (i as u32, v))).collect())
            .unwrap_or_default();
    }

    /// create an ast node from a plain object
    fn node_from_object(&self, node: &Plain) -> u32 {
        let id = self.allocate();
        let typ = node.get("typ").as_f64().unwrap_or(0.0) as u8;
        self.data_types.set(id, typ);
        let str_of = |v: &Plain| v.as_str().map(|s| s.to_string());
        match typ {
            EnumToken::StyleSheetNodeType => self.flags.set(id, EnumAstNodeStatus::HasChildren),
            EnumToken::RuleNodeType | EnumToken::KeyframesRuleNodeType => {
                let sel = str_of(node.get("sel")).or_else(|| str_of(node.get("nam"))).unwrap_or_default();
                self.name.set(id, self.intern_string(&sel));
                self.flags.set(id, EnumAstNodeStatus::HasChildren);
                if let Some(raw) = plain_to_raw(node.get("raw")) {
                    self.raw.borrow_mut().insert(id, Rc::new(RawSelector::from(raw)));
                }
                if let Some(o) = node.get("optimized").as_object() {
                    self.optimized.borrow_mut().insert(
                        id,
                        Rc::new(RefCell::new(OptimizedSelector {
                            match_: o.get("match").is_some_and(|m| m.truthy()),
                            optimized: o
                                .get("optimized")
                                .and_then(|v| v.as_array())
                                .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
                                .unwrap_or_default(),
                            selector: o.get("selector").and_then(plain_to_raw).unwrap_or_default(),
                            reducible: o.get("reducible").is_some_and(|m| m.truthy()),
                        })),
                    );
                }
                if let Some(val) = node.get("val").as_str() {
                    self.string_value.set(id, self.intern_string(val));
                }
            }
            EnumToken::AtRuleNodeType | EnumToken::KeyframesAtRuleNodeType => {
                self.name.set(id, self.intern_string(node.get("nam").as_str().unwrap_or("")));
                self.string_value.set(id, self.intern_string(node.get("val").as_str().unwrap_or("")));
                self.flags.set(id, if node.get("chi").is_nullish() { 0 } else { EnumAstNodeStatus::HasChildren });
            }
            EnumToken::CommentNodeType | EnumToken::CDOCOMMNodeType => {
                self.string_value.set(id, self.intern_string(node.get("val").as_str().unwrap_or("")));
                self.flags.set(id, 0);
            }
            _ => {
                self.name.set(id, self.intern_string(node.get("nam").as_str().unwrap_or("")));
                self.flags.set(id, EnumAstNodeStatus::HasValues);
                if let Some(values) = node.get("val").as_array() {
                    for token in values {
                        let t = self.from_object(token);
                        self.append_token(id, t);
                    }
                }
            }
        }
        if let Some(tokens) = node.get("tokens").as_array() {
            for token in tokens {
                let t = self.from_object(token);
                self.append_token(id, t);
            }
        }
        self.set_state(id, node.get("state").as_f64().unwrap_or(0.0) as u32);
        if let Some(loc) = node.get("loc").as_object() {
            self.set_location(id, plain_u32(loc.get("srcId")), plain_u32(loc.get("sta")), plain_u32(loc.get("end")));
        }
        if let Some(errors) = node.get("errors").as_array().filter(|e| !e.is_empty()) {
            self.errors.borrow_mut().insert(id, errors.iter().map(ErrorDescription::from_plain).collect());
        }
        if let Some(children) = node.get("chi").as_array() {
            if self.flags.get(id) & EnumAstNodeStatus::HasChildren != 0 {
                for child in children {
                    let c = self.from_object(child);
                    self.append_child_list(id, &[c]);
                }
            }
        }
        id
    }

    /// create a token from a plain object
    fn token_from_object(&self, plain: &Plain) -> u32 {
        let id = self.allocate();
        let typ = plain.get("typ").as_f64().unwrap_or(0.0) as u8;
        self.data_types.set(id, typ);
        if let Some(nam) = plain.get("nam").as_str() {
            self.name.set(id, self.intern_string(nam));
        }
        if let Some(unit) = plain.get("unit").as_str() {
            self.unit.set(id, self.intern_string(unit));
        }
        if let Some(kin) = plain.get("kin").as_f64() {
            self.color_type.set(id, kin as u8);
        }
        if let Some(cal) = plain.get("cal").as_str() {
            self.unit.set(id, self.intern_string(cal));
        }
        if let Some(attr) = plain.get("attr").as_str() {
            self.unit.set(id, self.intern_string(attr));
        }
        let val = plain.get("val");
        if typ == EnumToken::BinaryExpressionTokenType {
            self.numeric_value.set(id, plain.get("op").as_f64().unwrap_or(f64::NAN));
        } else if typ == EnumToken::UnaryExpressionTokenType {
            self.numeric_value.set(id, plain.get("sign").as_f64().unwrap_or(f64::NAN));
            let v = self.from_object(val);
            self.append_child_list(id, &[v]);
        } else if typ == EnumToken::FractionTokenType {
            let l = self.from_object(plain.get("l"));
            let r = self.from_object(plain.get("r"));
            self.append_child_list(id, &[l, r]);
        } else if let Some(s) = val.as_str() {
            if is_numeric_token_type(typ) {
                self.numeric_value.set(id, crate::utils::number::js_parse_number(s));
            } else {
                self.string_value.set(id, self.intern_string(s));
            }
        } else if let Some(n) = val.as_f64() {
            self.numeric_value.set(id, n);
        } else if val.as_object().is_some() {
            let fraction = self.from_object(val);
            self.set_fraction(id, Some(fraction));
        } else if let Some(values) = val.as_array().filter(|_| typ != EnumToken::MediaRangeQueryTokenType) {
            for child in values {
                let c = self.from_object(child);
                self.append_token(id, c);
            }
        }
        if let Some(children) = plain.get("chi").as_array() {
            for child in children {
                let c = self.from_object(child);
                self.append_token(id, c);
            }
        }
        if is_structured_token_type(typ) {
            for (key, field) in [("l", TokenField::L), ("r", TokenField::R), ("op", TokenField::Op), ("op1", TokenField::Op), ("op2", TokenField::Op2), ("val", TokenField::Val)] {
                let value = plain.get(key);
                if value.is_nullish() {
                    continue;
                }
                let field_tokens: Vec<u32> = match value.as_array() {
                    Some(values) => values.iter().map(|v| self.from_object(v)).collect(),
                    None => vec![self.from_object(value)],
                };
                self.set_token_list(self.field_list(id, field), &field_tokens);
            }
        }
        if let Some(loc) = plain.get("loc").as_object() {
            self.set_location(id, plain_u32(loc.get("srcId")), plain_u32(loc.get("sta")), plain_u32(loc.get("end")));
        }
        id
    }

    /// expand a rule: the nested rules and at-rules are moved out of the rule, into `parent` before `before`. the rule
    /// is detached when it has no children left
    pub fn expand_rule(&self, node: u32, parent: u32, before: u32) {
        if self.flags.get(node) & SKIP_STATES != 0 {
            return;
        }
        let ast = node;
        let sel = self.resolve_string(self.name.get(ast));
        let place = |child: u32| {
            if before == 0 {
                self.append_child(parent, child);
            } else {
                self.insert_before(before, &[child]);
            }
        };
        let is_pseudo_element = |t: &String| t == ":before" || t == ":after" || t.starts_with("::");
        if self.data_types.get(ast) == EnumToken::RuleNodeType {
            let mut child = self.first_child.get(ast);
            while child != 0 {
                let next = self.next_sibling.get(child);
                let typ = self.data_types.get(child);
                if typ == EnumToken::RuleNodeType {
                    let rule = child;
                    let rule_sel = self.resolve_string(self.name.get(rule));
                    if !rule_sel.contains('&') {
                        let mut sel_rule = crate::ast::minify::split_rule(&rule_sel);
                        let ar_self = crate::ast::minify::split_rule(&sel)
                            .into_iter()
                            .filter(|r| r.iter().all(|t| !is_pseudo_element(t)))
                            .map(|r| r.concat())
                            .collect::<Vec<_>>()
                            .join(",");
                        if ar_self.is_empty() {
                            self.detach(rule);
                            child = next;
                            continue;
                        }
                        for arr in sel_rule.iter_mut() {
                            let first = arr.first().and_then(|s| s.chars().next()).map(String::from).unwrap_or_default();
                            if crate::syntax::constants::COMBINATORS.contains(&first.as_str()) {
                                arr.insert(0, ar_self.clone());
                            } else {
                                arr.insert(0, " ".into());
                                arr.insert(0, ar_self.clone());
                            }
                        }
                        let selector = sel_rule.iter().map(|r| r.concat()).collect::<Vec<_>>().join(",");
                        self.name.set(rule, self.intern_string(&selector));
                    } else {
                        let mut child_selector_compound: Vec<String> = Vec::new();
                        let mut with_compound: Vec<String> = Vec::new();
                        let mut without_compound: Vec<String> = Vec::new();
                        let rules: Vec<Vec<String>> = crate::ast::minify::split_rule(&sel)
                            .into_iter()
                            .filter(|r| r.iter().all(|t| !is_pseudo_element(t)))
                            .collect();
                        let parent_selector = !sel.contains('&');
                        if rules.is_empty() {
                            self.detach(rule);
                            child = next;
                            continue;
                        }
                        let raw = self.raw.borrow().get(&rule).cloned().unwrap_or_else(|| Rc::new(RawSelector::from(crate::ast::minify::split_rule(&rule_sel))));
                        for sel in raw.iter() {
                            let s: String = sel.concat();
                            if s.contains('&') || parent_selector {
                                if s.get(1..).is_none_or(|rest| !rest.contains('&')) {
                                    if s.starts_with('&') {
                                        if s.as_bytes().get(1) == Some(&b' ') {
                                            child_selector_compound.push(s[2..].to_string());
                                        } else if s == "&" || parent_selector {
                                            with_compound.push(s);
                                        }
                                    } else {
                                        without_compound.push(s);
                                    }
                                } else {
                                    with_compound.push(s);
                                }
                            }
                        }
                        let mut selectors: Vec<String> = Vec::new();
                        let selector: String = if rules.len() > 1 {
                            format!(":is({})", rules.iter().map(|a| a.concat()).collect::<Vec<_>>().join(","))
                        } else {
                            rules[0].concat()
                        };
                        if !child_selector_compound.is_empty() {
                            if child_selector_compound.len() == 1 {
                                selectors.push(self.replace_compound(&format!("& {}", js_trim(&child_selector_compound[0])), &selector));
                            } else {
                                let joined = child_selector_compound.iter().map(|c| js_trim(c)).collect::<Vec<_>>().join(",");
                                selectors.push(self.replace_compound(&format!("& :is({})", joined), &selector));
                            }
                        }
                        if !with_compound.is_empty() && with_compound.iter().all(|t| t.starts_with('&') && !t[1..].contains('&')) {
                            without_compound.extend(with_compound.iter().map(|t| t[1..].to_string()));
                            with_compound.clear();
                        }
                        if !without_compound.is_empty() {
                            if without_compound.len() == 1 {
                                let w = &without_compound[0];
                                let all_alpha = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphabetic());
                                let use_is = rules.len() == 1
                                    && selector.bytes().next().is_some_and(|b| b.is_ascii_alphabetic() || b == b'.' || b == b':')
                                    && selector.contains(' ')
                                    && all_alpha(w);
                                let compound = if use_is { ":is(&)" } else { "&" };
                                let input = if rules.len() == 1 {
                                    if use_is {
                                        format!("{}:is(&)", w)
                                    } else if selector.bytes().next().is_some_and(|b| b == b'.' || b == b':') && all_alpha(w) {
                                        format!("{}{}", w, compound)
                                    } else {
                                        format!("{}{}", compound, w)
                                    }
                                } else if !w.is_empty() && w.bytes().all(|b| b.is_ascii_alphabetic() || b == b':') {
                                    format!("{}{}", js_trim(w), compound)
                                } else if !w.is_empty() && !w.chars().any(is_js_whitespace) {
                                    format!("&{}", js_trim(w))
                                } else {
                                    format!("&:is({})", js_trim(w))
                                };
                                selectors.push(self.replace_compound(&input, &selector));
                            } else {
                                let joined = without_compound.iter().map(|c| js_trim(c)).collect::<Vec<_>>().join(",");
                                selectors.push(self.replace_compound(&format!("&:is({})", joined), &selector));
                            }
                        }
                        if with_compound.len() == 1 {
                            selectors.push(self.replace_compound(&with_compound[0], &selector));
                        }
                        let joined = selectors.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(",");
                        self.name.set(rule, self.intern_string(&joined));
                    }
                    place(rule);
                    self.expand_rule(rule, parent, before);
                } else if typ == EnumToken::AtRuleNodeType
                    && (self.flags.get(child) & EnumAstNodeStatus::HasChildren == 0 || self.string_equals(self.name.get(child), "apply"))
                {
                    // an at-rule without a block or an `@apply` rule stays in the rule
                } else if typ == EnumToken::AtRuleNodeType {
                    let ast_at_rule = child;
                    let at_rule_name = self.resolve_string(self.name.get(ast_at_rule));
                    if &*at_rule_name == "scope" {
                        let at_rule_value = self.resolve_string(self.string_value.get(ast_at_rule));
                        if at_rule_value.contains('&') {
                            let replaced = self.replace_compound(&at_rule_value, &sel);
                            self.string_value.set(ast_at_rule, self.intern_string(&replaced));
                        }
                        let has_nesting = self
                            .child_iter(ast_at_rule)
                            .any(|t| self.data_types.get(t) == EnumToken::RuleNodeType && self.resolve_string(self.name.get(t)).contains('&'));
                        if has_nesting {
                            let clone = self.clone(node);
                            self.move_children(ast_at_rule, clone, 0);
                            self.expand_rule(clone, ast_at_rule, 0);
                            self.move_children(clone, ast_at_rule, self.first_child.get(ast_at_rule));
                        }
                        place(ast_at_rule);
                    } else {
                        let clone = self.clone(ast);
                        self.move_children(ast_at_rule, clone, 0);
                        self.expand_rule(clone, ast_at_rule, 0);
                        if self.first_child.get(clone) != 0 {
                            if self.first_child.get(ast_at_rule) == 0 {
                                self.append_child(ast_at_rule, clone);
                            } else {
                                self.insert_before(self.first_child.get(ast_at_rule), &[clone]);
                            }
                        }
                        place(ast_at_rule);
                        let mut r = self.first_child.get(ast_at_rule);
                        while r != 0 {
                            let next_rule = self.next_sibling.get(r);
                            if self.data_types.get(r) == EnumToken::AtRuleNodeType {
                                if self.flags.get(r) & EnumAstNodeStatus::HasChildren != 0 {
                                    let val = self.resolve_string(self.string_value.get(ast_at_rule));
                                    let r_val = self.resolve_string(self.string_value.get(r));
                                    if !val.is_empty() && !r_val.is_empty() {
                                        let r_name = self.resolve_string(self.name.get(r));
                                        if &*at_rule_name == "media" && &*r_name == "media" {
                                            self.string_value.set(r, self.intern_string(&format!("{} and {}", val, r_val)));
                                        } else if &*at_rule_name == "layer" && &*r_name == "layer" {
                                            self.string_value.set(r, self.intern_string(&format!("{}.{}", val, r_val)));
                                        }
                                    }
                                    place(r);
                                } else {
                                    self.detach(r);
                                }
                            }
                            r = next_rule;
                        }
                    }
                    if self.first_child.get(ast_at_rule) == 0 {
                        self.detach(ast_at_rule);
                    }
                }
                child = next;
            }
        }
        if self.first_child.get(ast) == 0 {
            self.detach(ast);
        }
    }

    /// (sic) port of `replaceCompoundLiteral()`: the tokens are `""` followed by `"&", ""` per `&` of the selector,
    /// sorted so that the `&` come last, then the `&` are replaced
    pub fn replace_compound_literal(&self, selector: &str, replace: &str) -> String {
        let count = selector.matches('&').count();
        // [""] + ["&", ""] * count, stable sorted with "&" last: all the "" then all the "&"
        replace.repeat(count)
    }
}

/// `String.prototype.trim()`
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_whitespace)
}

/// javascript `\s`
pub fn is_js_whitespace(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

fn plain_u32(v: Option<&Plain>) -> Option<u32> {
    v.and_then(|v| v.as_f64()).map(|n| n as u32)
}

fn raw_to_plain(raw: &RawSelectorTokens) -> Plain {
    Plain::Array(raw.iter().map(|r| Plain::Array(r.iter().map(|s| Plain::from(s.as_str())).collect())).collect())
}

fn plain_to_raw(v: &Plain) -> Option<RawSelectorTokens> {
    v.as_array().map(|a| {
        a.iter()
            .map(|r| r.as_array().map(|x| x.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default())
            .collect()
    })
}
