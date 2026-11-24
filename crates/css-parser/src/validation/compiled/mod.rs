//! Port of `src/validation/compiled`: the syntaxes compiled at build time (`npm run syntax-compile`).

pub mod builtins;
pub mod runtime;
#[rustfmt::skip]
pub mod syntaxes;
