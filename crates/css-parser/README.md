# rust-css-parser

A fault-tolerant CSS parser, validator, minifier and renderer: the Rust port of
[@tbela99/css-parser](https://github.com/tbela99/css-parser). The module tree mirrors the TypeScript sources and the
functions keep their names in snake case.

- parse, minify and render CSS, with source maps
- syntax validation of the declarations, at-rules and selectors
- CSS nesting, `calc()` evaluation, color conversion (`rgb()`, `oklch()`, `color-mix()`, ...), shorthand computation
- CSS modules

The same library is available for JavaScript: [@tbela99/rust-css-parser-napi](https://www.npmjs.com/package/@tbela99/rust-css-parser-napi)
(Node.js, N-API) and [@tbela99/rust-css-parser-wasm](https://www.npmjs.com/package/@tbela99/rust-css-parser-wasm)
(Node.js and the browsers, WebAssembly).

## Usage

```toml
[dependencies]
rust-css-parser = "2"
```

```rust
use rust_css_parser::host::NoHost;
use rust_css_parser::options::{ParserOptions, SourcesMap};
use rust_css_parser::parser::input::CssInput;
use rust_css_parser::parser::parse::do_parse_sync;
use rust_css_parser::parser::source::SourceFile;
use rust_css_parser::parser::tokenize::{create_parse_info, Tokenizer};
use rust_css_parser::printer::render::do_render;
use std::rc::Rc;

let css = ".a { color: rgb(255 0 0); width: calc(1px + 2px) } .b { color: red }";

// the source file: the locations, the source maps and the string table refer to it
let source = SourceFile::new(Rc::from(css), Vec::new(), Some(Rc::from("in.css")));
let sources = SourcesMap::default();
sources.borrow_mut().insert(source.id, source.clone());

let options = ParserOptions { minify: Some(true), source: Some(source.clone()), sources_map: Some(sources), ..Default::default() };
let tokenizer = Tokenizer::new(create_parse_info(Some(&CssInput::from(css)), source));

// NoHost: no callbacks (visitors, load(), ...)
let result = do_parse_sync(tokenizer, options, None, Rc::new(NoHost)).expect("parse");

let rendered = do_render(&result.arena, result.ast, &ParserOptions { minify: Some(true), ..Default::default() }, None);

assert_eq!(rendered.code(), ".a{color:red;width:3px}.b{color:red}");
```

See the [Rust API guide](https://github.com/tbela99/rust-css-parser/blob/master/files/rust.md) and the
[API reference](https://docs.rs/rust-css-parser).

## License

[MIT](LICENSE.md)
