//! Reused scratch buffers: the walkers and the traversals use a stack of node ids that is taken from a thread local
//! pool and returned when it is dropped, instead of allocating a stack for each traversal (the typescript port shares
//! a single stack for the same reason, see `visitStack` in `src/ast/arena.ts`).

use std::cell::RefCell;
use std::ops::{Deref, DerefMut};

/// the buffers kept by the pool
const MAX_POOLED: usize = 32;
/// a buffer larger than this is released instead of being kept
const MAX_CAPACITY: usize = 1 << 16;

thread_local! {
    static POOL: RefCell<Vec<Vec<u32>>> = const { RefCell::new(Vec::new()) };
}

/// a `Vec<u32>` taken from the pool, returned to the pool when it is dropped
pub struct PooledVec(Vec<u32>);

impl PooledVec {
    /// an empty buffer
    pub fn new() -> Self {
        PooledVec(POOL.with(|p| p.borrow_mut().pop()).unwrap_or_default())
    }

    /// a buffer containing the values
    pub fn from_slice(values: &[u32]) -> Self {
        let mut v = Self::new();
        v.0.extend_from_slice(values);
        v
    }
}

impl Default for PooledVec {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for PooledVec {
    fn drop(&mut self) {
        let mut v = std::mem::take(&mut self.0);
        if v.capacity() == 0 || v.capacity() > MAX_CAPACITY {
            return;
        }
        v.clear();
        // the pool is not available while the thread is being destroyed
        let _ = POOL.try_with(|p| {
            let mut pool = p.borrow_mut();
            if pool.len() < MAX_POOLED {
                pool.push(v);
            }
        });
    }
}

impl Deref for PooledVec {
    type Target = Vec<u32>;

    #[inline]
    fn deref(&self) -> &Vec<u32> {
        &self.0
    }
}

impl DerefMut for PooledVec {
    #[inline]
    fn deref_mut(&mut self) -> &mut Vec<u32> {
        &mut self.0
    }
}
