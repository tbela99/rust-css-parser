//! Host (javascript) interface: callbacks, values exchanged with the host, and the executor of the async ports.
//!
//! The typescript pipeline awaits host promises (visitors, loaders, scoped name generators). The rust ports are
//! `async fn`s: a [`HostPromise`] future is pending until the host settles the promise ([`settle_promise`]). The host
//! drives a [`Task`]: `poll()` returns the ids of the pending promises, the host awaits them, settles them and polls
//! again. The same mechanism serves the N-API and the WASM bindings: no thread, no runtime.

use crate::ast::arena::AstArena;
use crate::plain::Plain;
use std::cell::RefCell;
use crate::utils::fx::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

/// a value exchanged with the host
#[derive(Clone, Debug, Default)]
pub enum HostValue {
    #[default]
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    Str(Rc<str>),
    /// utf-8 bytes (`ArrayBuffer`, `Uint8Array`), not copied by the host
    Bytes(Rc<[u8]>),
    /// a view (`AstArenaNode`) of another arena: the arena handle and the node id
    ForeignNode(u32, u32),
    /// a plain object (`PlainAstNode`, `PlainToken`, json data)
    Plain(Plain),
    Array(Vec<HostValue>),
    /// a pending host promise
    Promise(u32),
}

impl HostValue {
    pub fn is_nullish(&self) -> bool {
        matches!(self, HostValue::Undefined | HostValue::Null)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            HostValue::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// a host definition: the javascript visitor maps and options, with the functions replaced by callback ids
#[derive(Clone, Debug)]
pub enum HostDef {
    /// a function: its callback id and its name (`Function.prototype.name`)
    Fn { id: u32, name: Rc<str> },
    Object(Vec<(Rc<str>, HostDef)>),
    Array(Vec<HostDef>),
    Number(f64),
    Str(Rc<str>),
    Bool(bool),
    Null,
}

impl HostDef {
    /// `Object.entries()`: the keys of an object, the indexes of an array
    pub fn entries(&self) -> Vec<(Rc<str>, HostDef)> {
        match self {
            HostDef::Object(entries) => entries.clone(),
            HostDef::Array(items) => items.iter().enumerate().map(|(i, v)| (Rc::from(i.to_string().as_str()), v.clone())).collect(),
            _ => Vec::new(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&HostDef> {
        match self {
            HostDef::Object(entries) => entries.iter().find(|(k, _)| &**k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn is_fn(&self) -> bool {
        matches!(self, HostDef::Fn { .. })
    }

    /// `typeof value === "object"` (arrays included, null excluded)
    pub fn is_object(&self) -> bool {
        matches!(self, HostDef::Object(_) | HostDef::Array(_))
    }
}

/// error raised by the host or by the port (`throw new Error(message)`)
#[derive(Clone, Debug, PartialEq)]
pub struct HostError(pub String);

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for HostError {
    fn from(s: String) -> Self {
        HostError(s)
    }
}

impl From<&str> for HostError {
    fn from(s: &str) -> Self {
        HostError(s.to_string())
    }
}

/// the host: calls the javascript callbacks
pub trait Host {
    /// call a callback. the arena is the arena being processed (the callbacks receive it as their first argument)
    fn call(&self, callback: u32, arena: Option<&Rc<AstArena>>, args: Vec<HostValue>) -> Result<HostValue, HostError>;

    /// the arena of a handle (views of other arenas returned by the callbacks)
    fn arena(&self, handle: u32) -> Option<Rc<AstArena>>;
}

/// a host without callbacks (rust callers)
pub struct NoHost;

impl Host for NoHost {
    fn call(&self, _callback: u32, _arena: Option<&Rc<AstArena>>, _args: Vec<HostValue>) -> Result<HostValue, HostError> {
        Err(HostError("no host".into()))
    }

    fn arena(&self, _handle: u32) -> Option<Rc<AstArena>> {
        None
    }
}

thread_local! {
    /// the settled promises, by id
    static SETTLED: RefCell<HashMap<u32, Result<HostValue, HostError>>> = RefCell::new(HashMap::default());
    /// the promises awaited by the task being polled
    static PENDING: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

/// settle a host promise: the task that awaits it is polled again by the host
pub fn settle_promise(id: u32, result: Result<HostValue, HostError>) {
    SETTLED.with(|s| s.borrow_mut().insert(id, result));
}

/// a host promise awaited by the rust code
pub struct HostPromise(pub u32);

impl Future for HostPromise {
    type Output = Result<HostValue, HostError>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        match SETTLED.with(|s| s.borrow_mut().remove(&self.0)) {
            Some(result) => Poll::Ready(result),
            None => {
                PENDING.with(|p| {
                    let mut p = p.borrow_mut();
                    if !p.contains(&self.0) {
                        p.push(self.0);
                    }
                });
                Poll::Pending
            }
        }
    }
}

/// await a host value: a promise is awaited, the other values are returned
pub async fn resolve(value: Result<HostValue, HostError>) -> Result<HostValue, HostError> {
    match value? {
        HostValue::Promise(id) => HostPromise(id).await,
        value => Ok(value),
    }
}

fn noop_waker() -> Waker {
    fn clone(_: *const ()) -> RawWaker {
        RawWaker::new(std::ptr::null(), &VTABLE)
    }
    fn noop(_: *const ()) {}
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    // SAFETY: the vtable functions do nothing
    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}

/// an async computation driven by the host
pub struct Task<T> {
    future: Pin<Box<dyn Future<Output = T>>>,
}

/// state of a task after a poll
pub enum TaskState<T> {
    Done(T),
    /// the ids of the host promises the task waits for
    Pending(Vec<u32>),
}

impl<T> Task<T> {
    pub fn new(future: impl Future<Output = T> + 'static) -> Self {
        Self { future: Box::pin(future) }
    }

    /// run the task until it completes or waits for host promises
    pub fn poll(&mut self) -> TaskState<T> {
        PENDING.with(|p| p.borrow_mut().clear());
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        match self.future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => TaskState::Done(value),
            Poll::Pending => TaskState::Pending(PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()))),
        }
    }
}

/// run a future synchronously: the sync api (`parseSync()`). a pending host promise is an error, like the typescript
/// sync functions that do not await
pub fn block_on<T>(future: impl Future<Output = Result<T, HostError>> + 'static) -> Result<T, HostError> {
    let mut task = Task::new(future);
    match task.poll() {
        TaskState::Done(value) => value,
        TaskState::Pending(_) => Err(HostError("a promise was returned to a synchronous call".into())),
    }
}
