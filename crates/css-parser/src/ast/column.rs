//! Interior mutable column of the arena.
//!
//! The arena follows the shared mutable model of the typescript implementation: the columns are read and written
//! through `&self` (visitors written in javascript re-enter the arena while a transformation runs). A column is a
//! `Vec` in an `UnsafeCell`. Soundness: no reference into the vector ever escapes these methods (values are copied in
//! and out), and the arena is `!Sync` (single threaded), so a growth never invalidates a live borrow.

use std::cell::UnsafeCell;

pub struct Column<T: Copy + Default> {
    data: UnsafeCell<Vec<T>>,
    /// the buffers replaced by a growth: the javascript typed array views of a column keep reading valid memory until
    /// they are refreshed (like a typed array replaced by a larger one). dropped with the column
    retired: UnsafeCell<Vec<Vec<T>>>,
}

impl<T: Copy + Default> Column<T> {
    pub fn new(size: usize) -> Self {
        Self { data: UnsafeCell::new(vec![T::default(); size]), retired: UnsafeCell::new(Vec::new()) }
    }

    pub fn from_vec(data: Vec<T>) -> Self {
        Self { data: UnsafeCell::new(data), retired: UnsafeCell::new(Vec::new()) }
    }

    #[inline(always)]
    fn vec(&self) -> &Vec<T> {
        // SAFETY: see the module documentation
        unsafe { &*self.data.get() }
    }

    #[inline(always)]
    #[allow(clippy::mut_from_ref)]
    fn vec_mut(&self) -> &mut Vec<T> {
        // SAFETY: see the module documentation
        unsafe { &mut *self.data.get() }
    }

    /// value of a slot (`column[id]`). out of range reads return the default value (`undefined` -> 0)
    #[inline(always)]
    pub fn get(&self, id: u32) -> T {
        self.vec().get(id as usize).copied().unwrap_or_default()
    }

    /// write a slot (`column[id] = value`). out of range writes are ignored, like typed arrays
    #[inline(always)]
    pub fn set(&self, id: u32, value: T) {
        if let Some(slot) = self.vec_mut().get_mut(id as usize) {
            *slot = value;
        }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.vec().len()
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.vec().is_empty()
    }

    /// grow the column, the new slots are zeroed
    pub fn resize(&self, size: usize) {
        let vec = self.vec_mut();
        if size <= vec.capacity() {
            vec.resize(size, T::default());
            return;
        }
        // a new buffer: the previous one is retired, not freed (javascript views may still point to it)
        let mut grown = Vec::with_capacity(size);
        grown.extend_from_slice(vec);
        grown.resize(size, T::default());
        let previous = std::mem::replace(vec, grown);
        // SAFETY: see the module documentation
        unsafe { (*self.retired.get()).push(previous) };
    }

    /// replace the data
    pub fn replace(&self, data: Vec<T>) -> Vec<T> {
        std::mem::replace(self.vec_mut(), data)
    }

    /// copy of the data
    pub fn to_vec(&self) -> Vec<T> {
        self.vec().clone()
    }

    /// pointer to the data, for the javascript typed array views. valid until the next resize / replace
    pub fn as_ptr(&self) -> *const T {
        self.vec().as_ptr()
    }

    /// mutable pointer to the data, for the javascript typed array views
    pub fn as_mut_ptr(&self) -> *mut T {
        self.vec_mut().as_mut_ptr()
    }
}
