//! N-API backend: the functions of the native addon, called by `backend.js` of the `css-parser-napi` package. The N-API functions are resolved
//! when node loads the addon (`-undefined dynamic_lookup` on macOS, see `build.rs`).
//!
//! The exchange buffer and the arena columns are external `ArrayBuffer`s over the rust memory (no copy). A node
//! built with the V8 sandbox refuses external buffers: the column buffers are then copies, the exchange buffer too.

#![allow(non_camel_case_types)]

use rust_css_parser::bindings::{self, EXCHANGE};
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::os::raw::c_char;
use std::ptr;

type napi_env = *mut c_void;
type napi_value = *mut c_void;
type napi_ref = *mut c_void;
type napi_callback_info = *mut c_void;
type napi_status = i32;
type napi_callback = unsafe extern "C" fn(napi_env, napi_callback_info) -> napi_value;
type napi_finalize = unsafe extern "C" fn(napi_env, *mut c_void, *mut c_void);

const NAPI_OK: napi_status = 0;

extern "C" {
    fn napi_create_function(env: napi_env, name: *const c_char, length: usize, cb: napi_callback, data: *mut c_void, result: *mut napi_value) -> napi_status;
    fn napi_set_named_property(env: napi_env, object: napi_value, name: *const c_char, value: napi_value) -> napi_status;
    fn napi_get_cb_info(env: napi_env, info: napi_callback_info, argc: *mut usize, argv: *mut napi_value, this: *mut napi_value, data: *mut *mut c_void) -> napi_status;
    fn napi_get_value_double(env: napi_env, value: napi_value, result: *mut f64) -> napi_status;
    fn napi_create_double(env: napi_env, value: f64, result: *mut napi_value) -> napi_status;
    fn napi_create_external_arraybuffer(env: napi_env, data: *mut c_void, length: usize, finalize: Option<napi_finalize>, hint: *mut c_void, result: *mut napi_value) -> napi_status;
    fn napi_create_arraybuffer(env: napi_env, length: usize, data: *mut *mut c_void, result: *mut napi_value) -> napi_status;
    fn napi_call_function(env: napi_env, recv: napi_value, func: napi_value, argc: usize, argv: *const napi_value, result: *mut napi_value) -> napi_status;
    fn napi_create_reference(env: napi_env, value: napi_value, count: u32, result: *mut napi_ref) -> napi_status;
    fn napi_get_reference_value(env: napi_env, reference: napi_ref, result: *mut napi_value) -> napi_status;
    fn napi_get_undefined(env: napi_env, result: *mut napi_value) -> napi_status;
    fn napi_get_and_clear_last_exception(env: napi_env, result: *mut napi_value) -> napi_status;
    fn napi_is_exception_pending(env: napi_env, result: *mut bool) -> napi_status;
}

thread_local! {
    /// the env of the call being run: the callbacks are called with it
    static ENV: Cell<napi_env> = const { Cell::new(ptr::null_mut()) };
    /// the javascript host function: `(length) => responseLength`
    static HOST: Cell<napi_ref> = const { Cell::new(ptr::null_mut()) };
    /// address of the exchange buffer given to javascript: javascript asks for a new buffer when it changes
    static BUFFER_PTR: Cell<*const u8> = const { Cell::new(ptr::null()) };
    /// the external buffers are refused (V8 sandbox): copies are returned
    static NO_EXTERNAL: Cell<bool> = const { Cell::new(false) };
    /// the copy of the exchange buffer when the external buffers are refused
    static COPY: RefCell<napi_ref> = const { RefCell::new(ptr::null_mut()) };
}

unsafe fn args<const N: usize>(env: napi_env, info: napi_callback_info) -> [napi_value; N] {
    let mut argv = [ptr::null_mut(); N];
    let mut argc = N;
    napi_get_cb_info(env, info, &mut argc, argv.as_mut_ptr(), ptr::null_mut(), ptr::null_mut());
    argv
}

unsafe fn number(env: napi_env, value: napi_value) -> f64 {
    let mut n = 0.0;
    napi_get_value_double(env, value, &mut n);
    n
}

unsafe fn js_number(env: napi_env, n: f64) -> napi_value {
    let mut v = ptr::null_mut();
    napi_create_double(env, n, &mut v);
    v
}

unsafe fn undefined(env: napi_env) -> napi_value {
    let mut v = ptr::null_mut();
    napi_get_undefined(env, &mut v);
    v
}

/// an `ArrayBuffer` over rust memory: external when allowed, a copy otherwise
unsafe fn array_buffer(env: napi_env, data: *const u8, length: usize) -> napi_value {
    let mut result = ptr::null_mut();
    if !NO_EXTERNAL.with(|n| n.get()) && length > 0 {
        if napi_create_external_arraybuffer(env, data as *mut c_void, length, None, ptr::null_mut(), &mut result) == NAPI_OK {
            return result;
        }
        NO_EXTERNAL.with(|n| n.set(true));
    }
    let mut target: *mut c_void = ptr::null_mut();
    napi_create_arraybuffer(env, length, &mut target, &mut result);
    if length > 0 {
        ptr::copy_nonoverlapping(data, target as *mut u8, length);
    }
    result
}

fn host_call(request: &[u8]) -> Vec<u8> {
    bindings::write_exchange(request);
    let env = ENV.with(|e| e.get());
    // SAFETY: called while a javascript call runs (the env is valid); the host function is referenced
    unsafe {
        let mut function = ptr::null_mut();
        napi_get_reference_value(env, HOST.with(|h| h.get()), &mut function);
        let changed = buffer_changed();
        let argv = [js_number(env, request.len() as f64), js_number(env, changed as u8 as f64)];
        let mut result = ptr::null_mut();
        napi_call_function(env, undefined(env), function, 2, argv.as_ptr(), &mut result);
        let mut pending = false;
        napi_is_exception_pending(env, &mut pending);
        if pending {
            let mut exception = ptr::null_mut();
            napi_get_and_clear_last_exception(env, &mut exception);
            let mut out = Vec::new();
            bindings::value::Val::Error(std::rc::Rc::from("javascript callback threw")).encode(&mut out);
            return out;
        }
        let length = number(env, result) as usize;
        EXCHANGE.with(|e| { let b = e.borrow(); b[..length.min(b.len())].to_vec() })
    }
}

/// true when the exchange buffer moved since javascript received it
fn buffer_changed() -> bool {
    EXCHANGE.with(|e| e.borrow().as_ptr()) != BUFFER_PTR.with(|p| p.get())
}

/// `buffer(capacity)`: the exchange buffer, at least `capacity` bytes
unsafe extern "C" fn buffer(env: napi_env, info: napi_callback_info) -> napi_value {
    let [capacity] = args::<1>(env, info);
    let capacity = number(env, capacity) as usize;
    let (data, length) = EXCHANGE.with(|e| {
        let mut buffer = e.borrow_mut();
        if buffer.len() < capacity.max(1 << 16) {
            buffer.resize(capacity.max(1 << 16), 0);
        }
        (buffer.as_ptr(), buffer.len())
    });
    BUFFER_PTR.with(|p| p.set(data));
    array_buffer(env, data, length)
}

/// `call(length)`: run the call encoded in the exchange buffer. returns the result length, negative when the buffer
/// moved (javascript asks for the new buffer)
unsafe extern "C" fn call(env: napi_env, info: napi_callback_info) -> napi_value {
    let [length] = args::<1>(env, info);
    let length = number(env, length) as usize;
    let previous = ENV.with(|e| e.replace(env));
    bindings::set_host_call(host_call);
    let copied = NO_EXTERNAL.with(|n| n.get());
    let result = bindings::call(length);
    ENV.with(|e| e.set(previous));
    let moved = buffer_changed() || copied;
    js_number(env, if moved { -(result as f64) - 1.0 } else { result as f64 })
}

/// `setHost(fn)`: the javascript function that runs the callbacks
unsafe extern "C" fn set_host(env: napi_env, info: napi_callback_info) -> napi_value {
    let [function] = args::<1>(env, info);
    let mut reference = ptr::null_mut();
    napi_create_reference(env, function, 1, &mut reference);
    HOST.with(|h| h.set(reference));
    undefined(env)
}

/// `column(handle, index)`: an `ArrayBuffer` over a column of an arena
unsafe extern "C" fn column(env: napi_env, info: napi_callback_info) -> napi_value {
    let [handle, index] = args::<2>(env, info);
    let (handle, index) = (number(env, handle) as u32, number(env, index) as u32);
    let Ok(arena) = bindings::arena_of(handle) else {
        return undefined(env);
    };
    let (data, length) = arena.column_raw(index);
    let size = match index {
        0 | 13 => 1,
        10 | 14 => 8,
        _ => 4,
    };
    array_buffer(env, data, length * size)
}

/// `generation(handle)`: the generation counter of an arena (an `ArrayBuffer` of 4 bytes)
unsafe extern "C" fn generation(env: napi_env, info: napi_callback_info) -> napi_value {
    let [handle] = args::<1>(env, info);
    let Ok(arena) = bindings::arena_of(number(env, handle) as u32) else {
        return undefined(env);
    };
    array_buffer(env, arena.generation_ptr() as *const u8, 4)
}

unsafe extern "C" fn free_output(_env: napi_env, _data: *mut c_void, hint: *mut c_void) {
    drop(Box::from_raw(hint as *mut Vec<u8>));
}

/// `output(id)`: the `ArrayBuffer` of a buffer given by rust. it owns the rust memory (no copy)
unsafe extern "C" fn output(env: napi_env, info: napi_callback_info) -> napi_value {
    let [id] = args::<1>(env, info);
    let Some(bytes) = bindings::take_output(number(env, id) as u32) else {
        return undefined(env);
    };
    if !NO_EXTERNAL.with(|n| n.get()) && !bytes.is_empty() {
        let boxed = Box::new(bytes);
        let (data, length) = (boxed.as_ptr() as *mut c_void, boxed.len());
        let hint = Box::into_raw(boxed) as *mut c_void;
        let mut result = ptr::null_mut();
        if napi_create_external_arraybuffer(env, data, length, Some(free_output), hint, &mut result) == NAPI_OK {
            return result;
        }
        NO_EXTERNAL.with(|n| n.set(true));
        let bytes = Box::from_raw(hint as *mut Vec<u8>);
        return array_buffer(env, bytes.as_ptr(), bytes.len());
    }
    array_buffer(env, bytes.as_ptr(), bytes.len())
}

/// `external()`: true when the buffers are views of the rust memory (false: copies)
unsafe extern "C" fn external(env: napi_env, _info: napi_callback_info) -> napi_value {
    js_number(env, (!NO_EXTERNAL.with(|n| n.get())) as u8 as f64)
}

unsafe fn export(env: napi_env, exports: napi_value, name: &[u8], cb: napi_callback) {
    let mut function = ptr::null_mut();
    napi_create_function(env, name.as_ptr() as *const c_char, name.len() - 1, cb, ptr::null_mut(), &mut function);
    napi_set_named_property(env, exports, name.as_ptr() as *const c_char, function);
}

/// module initialization: called by node when the addon is loaded
#[no_mangle]
pub unsafe extern "C" fn napi_register_module_v1(env: napi_env, exports: napi_value) -> napi_value {
    export(env, exports, b"buffer\0", buffer);
    export(env, exports, b"call\0", call);
    export(env, exports, b"setHost\0", set_host);
    export(env, exports, b"column\0", column);
    export(env, exports, b"generation\0", generation);
    export(env, exports, b"external\0", external);
    export(env, exports, b"output\0", output);
    exports
}
