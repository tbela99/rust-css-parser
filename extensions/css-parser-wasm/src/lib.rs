//! WASM backend: the exports called by `backend.js` of the `css-parser-wasm` package. The exchange buffer and the arena columns are read and
//! written by javascript in the linear memory.
//!
//! The crate is empty on the other targets: `cargo build --workspace` builds it natively.

#![cfg(target_arch = "wasm32")]

use rust_css_parser::bindings::{self, EXCHANGE};

#[link(wasm_import_module = "env")]
extern "C" {
    /// call a javascript callback: the request is in the exchange buffer, the response replaces it. returns the
    /// response length
    fn css_host(length: u32) -> u32;
}

fn host_call(request: &[u8]) -> Vec<u8> {
    bindings::write_exchange(request);
    // SAFETY: the host provides `css_host`
    let length = unsafe { css_host(request.len() as u32) } as usize;
    EXCHANGE.with(|e| { let b = e.borrow(); b[..length.min(b.len())].to_vec() })
}

/// make room for `capacity` bytes in the exchange buffer: its address (it changes when the buffer grows)
#[no_mangle]
pub extern "C" fn css_buffer(capacity: u32) -> *mut u8 {
    bindings::set_host_call(host_call);
    EXCHANGE.with(|e| {
        let mut buffer = e.borrow_mut();
        let capacity = capacity as usize;
        if buffer.len() < capacity {
            buffer.resize(capacity, 0);
        }
        buffer.as_mut_ptr()
    })
}

/// run the call encoded in the first `length` bytes of the exchange buffer. returns the length of the result
#[no_mangle]
pub extern "C" fn css_call(length: u32) -> u32 {
    bindings::set_host_call(host_call);
    bindings::call(length as usize) as u32
}

/// address of the exchange buffer
#[no_mangle]
pub extern "C" fn css_buffer_ptr() -> *const u8 {
    EXCHANGE.with(|e| e.borrow().as_ptr())
}

/// address of a column of an arena
#[no_mangle]
pub extern "C" fn css_column_ptr(handle: u32, index: u32) -> *const u8 {
    bindings::arena_of(handle).map_or(std::ptr::null(), |a| a.column_raw(index).0)
}

/// length (elements) of a column of an arena
#[no_mangle]
pub extern "C" fn css_column_len(handle: u32, index: u32) -> u32 {
    bindings::arena_of(handle).map_or(0, |a| a.column_raw(index).1 as u32)
}

/// address of the generation counter of an arena
#[no_mangle]
pub extern "C" fn css_generation_ptr(handle: u32) -> *const u32 {
    bindings::arena_of(handle).map_or(std::ptr::null(), |a| a.generation_ptr())
}

thread_local! {
    /// the buffer being copied by javascript
    static TAKEN: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// take a buffer given to javascript: its length (capacity) is returned, its address by `css_output_ptr()`
#[no_mangle]
pub extern "C" fn css_output_take(id: u32) -> u32 {
    let bytes = bindings::take_output(id).unwrap_or_default();
    let len = bytes.len() as u32;
    TAKEN.with(|t| *t.borrow_mut() = bytes);
    len
}

/// address of the buffer taken by `css_output_take()`
#[no_mangle]
pub extern "C" fn css_output_ptr() -> *const u8 {
    TAKEN.with(|t| t.borrow().as_ptr())
}

/// release the buffer taken by `css_output_take()` once javascript copied it
#[no_mangle]
pub extern "C" fn css_output_free() {
    TAKEN.with(|t| *t.borrow_mut() = Vec::new());
}
