//! Rust port of `@tbela99/css-parser`. The module tree mirrors `src/*` of the typescript project.
#![allow(clippy::too_many_arguments, clippy::new_without_default, non_upper_case_globals, non_snake_case)]

/// version of the package (`version` of package.json, set by build.rs)
pub const VERSION: &str = env!("CSS_PARSER_VERSION");

pub mod ast;
pub mod fs;
pub mod host;
pub mod options;
pub mod parser;
pub mod plain;
pub mod printer;
pub mod syntax;
pub mod types;
pub mod validation;
pub mod utils;
pub mod bindings;
