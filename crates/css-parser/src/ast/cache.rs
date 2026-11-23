//! Port of `src/ast/cache.ts`: structural memoization of values computed from token trees.
//!
//! A token tree is serialized into i32 values (a header with the type, the color type and the fields that follow, the
//! fields that are not empty, then the token list and the children list). The typescript open addressing table over a
//! key pool is ported as is: the keys are appended to a pool, the slots are numbered in insertion order, a lookup
//! reuses a scratch buffer and a miss keeps the key.
//!
//! The keys do not reference arena ids, the interned string ids are part of the keys: a memo must not outlive the
//! string table of the arena.

use crate::ast::arena::Arena;

const NAME: i32 = 1 << 16;
const STRING: i32 = 1 << 17;
const UNIT: i32 = 1 << 18;
/// 32 bits integer numeric value, other than 0
const INTEGER: i32 = 1 << 19;
/// raw bits of the numeric value (2 values)
const NUMBER: i32 = 1 << 20;
/// token list, terminated by an end marker
const TOKENS: i32 = 1 << 21;
/// children list, terminated by an end marker
const CHILDREN: i32 = 1 << 22;
/// end of a token list or of a children list
const END: i32 = -1;

/// memoize values by token structure. lookups return a slot, the value of a slot is `None` until it is set
pub struct StructuralMemo<T> {
    /// values by slot
    pub values: Vec<Option<T>>,
    /// number of distinct structures
    pub size: usize,
    /// number of lookups that found an existing entry
    pub hits: usize,
    /// open addressing table: slot + 1 by hash index, 0 when empty
    table: Vec<u32>,
    /// the keys, one after the other (no allocation per key)
    pool: Vec<i32>,
    /// start, length and hash of the key of each slot
    keys: Vec<(u32, u32, u64)>,
    scratch: Vec<i32>,
    /// compare the numeric values with all the `NaN` values equal (the raw bits are compared otherwise)
    numeric_equality: bool,
}

impl<T> Default for StructuralMemo<T> {
    fn default() -> Self {
        Self::new(false)
    }
}

impl<T> StructuralMemo<T> {
    pub fn new(numeric_equality: bool) -> Self {
        Self {
            values: Vec::new(),
            size: 0,
            hits: 0,
            table: vec![0; 256],
            pool: Vec::new(),
            keys: Vec::new(),
            scratch: Vec::with_capacity(64),
            numeric_equality,
        }
    }

    /// slot of a token list (the token and its next siblings). `prefix`: extra key component, not negative
    pub fn list(&mut self, arena: &Arena, first: u32, prefix: i32) -> usize {
        self.scratch.clear();
        self.scratch.push(prefix);
        let mut token = first;
        while token != 0 {
            self.serialize(arena, token);
            token = arena.next_sibling.get(token);
        }
        self.lookup()
    }

    /// slot of a single token (its next siblings are ignored)
    pub fn token(&mut self, arena: &Arena, token: u32) -> usize {
        self.scratch.clear();
        self.serialize(arena, token);
        self.lookup()
    }

    pub fn get(&self, slot: usize) -> Option<&T> {
        self.values.get(slot).and_then(|v| v.as_ref())
    }

    pub fn set(&mut self, slot: usize, value: T) {
        self.values[slot] = Some(value);
    }

    /// drop the entries
    pub fn clear(&mut self) {
        self.values.clear();
        self.table.fill(0);
        self.pool.clear();
        self.keys.clear();
        self.size = 0;
        self.hits = 0;
    }

    fn serialize(&mut self, arena: &Arena, token: u32) {
        let name = arena.name.get(token) as i32;
        let str = arena.string_value.get(token) as i32;
        let unit = arena.unit.get(token) as i32;
        let first_token = arena.first_token.get(token);
        let first_child = arena.first_child.get(token);
        let mut value = arena.numeric_value.get(token);
        // type and color type are 8 bits values
        let mut header = arena.data_types.get(token) as i32 | ((arena.color_type.get(token) as i32) << 8);
        let index = self.scratch.len();
        self.scratch.push(0);
        if name != 0 {
            header |= NAME;
            self.scratch.push(name);
        }
        if str != 0 {
            header |= STRING;
            self.scratch.push(str);
        }
        if unit != 0 {
            header |= UNIT;
            self.scratch.push(unit);
        }
        if self.numeric_equality && value.is_nan() {
            value = f64::NAN;
        }
        // 0 is not serialized. -0 is not an integer: its raw bits are serialized
        if value != 0.0 || value.is_sign_negative() && value == 0.0 {
            if value.fract() == 0.0 && value >= i32::MIN as f64 && value <= i32::MAX as f64 && value != 0.0 {
                header |= INTEGER;
                self.scratch.push(value as i32);
            } else {
                header |= NUMBER;
                let bits = if value.is_nan() { f64::NAN.to_bits() } else { value.to_bits() };
                self.scratch.push(bits as u32 as i32);
                self.scratch.push((bits >> 32) as u32 as i32);
            }
        }
        if first_token != 0 {
            header |= TOKENS;
        }
        if first_child != 0 {
            header |= CHILDREN;
        }
        self.scratch[index] = header;
        if first_token != 0 {
            let mut child = first_token;
            while child != 0 {
                self.serialize(arena, child);
                child = arena.next_sibling.get(child);
            }
            self.scratch.push(END);
        }
        if first_child != 0 {
            let mut child = first_child;
            while child != 0 {
                self.serialize(arena, child);
                child = arena.next_sibling.get(child);
            }
            self.scratch.push(END);
        }
    }

    /// find or insert the serialized key: the key is hashed once, a new key is appended to the pool
    fn lookup(&mut self) -> usize {
        let hash = hash_key(&self.scratch);
        let mask = self.table.len() - 1;
        let mut index = hash as usize & mask;
        loop {
            let entry = self.table[index];
            if entry == 0 {
                break;
            }
            let slot = entry as usize - 1;
            let (start, length, key_hash) = self.keys[slot];
            if key_hash == hash && self.pool[start as usize..(start + length) as usize] == self.scratch[..] {
                self.hits += 1;
                return slot;
            }
            index = (index + 1) & mask;
        }
        let slot = self.size;
        self.size += 1;
        self.keys.push((self.pool.len() as u32, self.scratch.len() as u32, hash));
        self.pool.extend_from_slice(&self.scratch);
        self.values.push(None);
        self.table[index] = slot as u32 + 1;
        // load factor 1/2
        if self.size * 2 > self.table.len() {
            self.rehash(self.table.len() * 2);
        }
        slot
    }

    fn rehash(&mut self, capacity: usize) {
        self.table.clear();
        self.table.resize(capacity, 0);
        let mask = capacity - 1;
        for (slot, &(_, _, hash)) in self.keys.iter().enumerate() {
            let mut index = hash as usize & mask;
            while self.table[index] != 0 {
                index = (index + 1) & mask;
            }
            self.table[index] = slot as u32 + 1;
        }
    }
}

/// Fx hash of a serialized key, with a final mix: the low bits index the table
fn hash_key(key: &[i32]) -> u64 {
    let mut hash: u64 = key.len() as u64;
    for &value in key {
        hash = (hash.rotate_left(5) ^ value as u32 as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    hash ^ (hash >> 32)
}
