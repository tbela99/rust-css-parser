//! Port of `src/parser/utils/cache.ts`.
//!
//! `memoize()` caches the results of a function by its arguments, `null` results included. In Rust the memoized
//! functions own a [`Memo`] (usually in a `thread_local!`) keyed by their arguments.

use std::cell::RefCell;
use crate::utils::fx::HashMap;
use std::hash::Hash;

/// memoized results of a function, by arguments
pub struct Memo<K, V> {
    values: RefCell<HashMap<K, V>>,
}

impl<K: Eq + Hash, V: Clone> Default for Memo<K, V> {
    fn default() -> Self {
        Self { values: RefCell::new(HashMap::default()) }
    }
}

impl<K: Eq + Hash, V: Clone> Memo<K, V> {
    pub fn new() -> Self {
        Self::default()
    }

    /// the cached value of a key, computed once. the computation may use other memoized functions
    pub fn get_or(&self, key: K, compute: impl FnOnce(&K) -> V) -> V {
        if let Some(value) = self.values.borrow().get(&key) {
            return value.clone();
        }
        let value = compute(&key);
        self.values.borrow_mut().insert(key, value.clone());
        value
    }
}
