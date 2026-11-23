//! Javascript bindings: one call interface shared by the N-API addon (`extensions/css-parser-napi`) and the WASM
//! module (`extensions/css-parser-wasm`). The backends are separate crates of the workspace: this crate only
//! provides the exchange buffer, the arena handles and the dispatch of the calls.
//!
//! Javascript encodes a call (`[method, ...arguments]`, see [`value`]) into the exchange buffer and calls `call()`;
//! the result is encoded back into the exchange buffer. The arena columns are not exchanged: javascript reads and
//! writes them through typed array views of the rust memory (see `column()` in the backends), refreshed when the
//! arena generation changes. The callbacks (visitors, `load`, `generateScopedName`, walk filters, ...) are called
//! through the backend `host_call()`: the arguments and the result go through the exchange buffer too. The buffer is
//! decoded before the work starts and encoded when it ends, so the reentrant calls (a visitor that calls the arena)
//! share it safely.

pub mod api;
pub mod value;

use crate::ast::arena::AstArena;
use crate::host::{Host, HostError, HostValue};
use std::cell::RefCell;
use std::rc::Rc;
use value::Val;

thread_local! {
    /// the exchange buffer
    pub static EXCHANGE: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(1 << 16));
    /// the arenas used by javascript, by handle (1 based). a handle is released by `freeArena`
    static ARENAS: RefCell<Vec<Option<Rc<AstArena>>>> = const { RefCell::new(Vec::new()) };
    /// the backend callback function: encoded request -> encoded result (`None` outside of a binding)
    static HOST_CALL: RefCell<Option<fn(&[u8]) -> Vec<u8>>> = const { RefCell::new(None) };
}

/// write bytes at the start of the exchange buffer. the buffer never shrinks: javascript keeps a view of its whole
/// length and writes the next request anywhere in it
pub fn write_exchange(bytes: &[u8]) {
    EXCHANGE.with(|e| {
        let mut buffer = e.borrow_mut();
        if buffer.len() < bytes.len() {
            buffer.resize(bytes.len(), 0);
        }
        buffer[..bytes.len()].copy_from_slice(bytes);
    });
}

thread_local! {
    /// the buffers given to javascript, by id (1 based), until javascript takes them
    static OUTPUTS: RefCell<Vec<Option<Vec<u8>>>> = const { RefCell::new(Vec::new()) };
}

/// give a buffer to javascript without a copy: its whole capacity becomes the `ArrayBuffer`, the content is a view
/// of its length (like the output buffer of the typescript writer)
pub fn give_output(mut bytes: Vec<u8>) -> Val {
    let len = bytes.len() as u32;
    // the spare capacity is part of the ArrayBuffer: initialized
    bytes.resize(bytes.capacity(), 0);
    OUTPUTS.with(|o| {
        let mut outputs = o.borrow_mut();
        let entry = Some(bytes);
        let id = match outputs.iter().position(|x| x.is_none()) {
            Some(i) => {
                outputs[i] = entry;
                i + 1
            }
            None => {
                outputs.push(entry);
                outputs.len()
            }
        };
        Val::Output(id as u32, len)
    })
}

/// take a buffer given to javascript
pub fn take_output(id: u32) -> Option<Vec<u8>> {
    OUTPUTS.with(|o| o.borrow_mut().get_mut((id as usize).wrapping_sub(1)).and_then(|x| x.take()))
}

/// install the backend callback function
pub fn set_host_call(f: fn(&[u8]) -> Vec<u8>) {
    HOST_CALL.with(|h| *h.borrow_mut() = Some(f));
}

/// the handle of an arena: an arena already known keeps its handle
pub fn arena_handle(arena: &Rc<AstArena>) -> u32 {
    ARENAS.with(|a| {
        let mut arenas = a.borrow_mut();
        if let Some(index) = arenas.iter().position(|x| x.as_ref().is_some_and(|x| Rc::ptr_eq(x, arena))) {
            return index as u32 + 1;
        }
        if let Some(index) = arenas.iter().position(|x| x.is_none()) {
            arenas[index] = Some(arena.clone());
            return index as u32 + 1;
        }
        arenas.push(Some(arena.clone()));
        arenas.len() as u32
    })
}

/// the arena of a handle
pub fn arena_of(handle: u32) -> Result<Rc<AstArena>, HostError> {
    ARENAS.with(|a| a.borrow().get((handle as usize).wrapping_sub(1)).cloned().flatten()).ok_or_else(|| HostError(format!("invalid arena handle {}", handle)))
}

/// release a handle: the arena is dropped when no other reference remains
pub fn free_arena(handle: u32) {
    ARENAS.with(|a| {
        if let Some(slot) = a.borrow_mut().get_mut((handle as usize).wrapping_sub(1)) {
            *slot = None;
        }
    });
}

/// call a javascript callback: `[callback id, arena, ...arguments]` -> result
pub fn call_js(callback: u32, arena: Option<&Rc<AstArena>>, args: Vec<Val>) -> Result<Val, HostError> {
    let mut request = Vec::with_capacity(64);
    let mut list = vec![Val::Num(callback as f64), arena.map_or(Val::Undefined, |a| Val::Arena(arena_handle(a)))];
    list.extend(args);
    Val::Array(list).encode(&mut request);
    let f = HOST_CALL.with(|h| *h.borrow()).ok_or_else(|| HostError("no javascript host".into()))?;
    let response = f(&request);
    match Val::decode(&response, &mut 0) {
        Val::Error(message) => Err(HostError(message.to_string())),
        value => Ok(value),
    }
}

/// the javascript host of the parser: the callbacks of the options (visitors, loaders, ...)
pub struct JsHost;

impl Host for JsHost {
    fn call(&self, callback: u32, arena: Option<&Rc<AstArena>>, args: Vec<HostValue>) -> Result<HostValue, HostError> {
        let handle = arena.map_or(0, arena_handle);
        let args = args.iter().map(|a| Val::from_host(a, handle)).collect();
        call_js(callback, arena, args).map(|v| v.to_host())
    }

    fn arena(&self, handle: u32) -> Option<Rc<AstArena>> {
        arena_of(handle).ok()
    }
}

/// run the call encoded in the exchange buffer (`[method, ...arguments]`) and encode its result in place. a panic
/// is returned as an error
pub fn call(length: usize) -> usize {
    let request = EXCHANGE.with(|e| {
        let buffer = e.borrow();
        Val::decode(&buffer[..length.min(buffer.len())], &mut 0)
    });
    let (method, args) = match request {
        Val::Array(mut items) if !items.is_empty() => {
            let method = items.remove(0);
            (method.as_str().unwrap_or("").to_string(), items)
        }
        _ => (String::new(), Vec::new()),
    };
    let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| api::dispatch(&method, args))) {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => Val::Error(Rc::from(error.0.as_str())),
        Err(panic) => {
            let message = panic.downcast_ref::<String>().cloned().or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            Val::Error(Rc::from(format!("internal error in {}: {}", method, message).as_str()))
        }
    };
    let mut out = Vec::with_capacity(256);
    result.encode(&mut out);
    write_exchange(&out);
    out.len()
}
