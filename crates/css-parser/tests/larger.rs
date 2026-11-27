//! the largest stylesheet of the tests (test/files/css/larger.css, 23 MB, about 1,100 errors) is parsed and minified.
//! the error list of the parser was copied for every valid selector: quadratic with the number of errors
use rust_css_parser::host::NoHost;
use rust_css_parser::options::{ParserOptions, SourcesMap};
use rust_css_parser::parser::input::CssInput;
use rust_css_parser::parser::parse::do_parse_sync;
use rust_css_parser::parser::source::SourceFile;
use rust_css_parser::parser::tokenize::{create_parse_info, Tokenizer};
use rust_css_parser::printer::render::do_render;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

#[test]
fn larger_css_is_parsed_and_minified() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/files/css/larger.css");
    let css = std::fs::read(&path).expect("larger.css");
    let source = SourceFile::new(Rc::from(""), Vec::new(), Some(Rc::from("")));
    let sources = SourcesMap::default();
    sources.borrow_mut().insert(source.id, source.clone());
    let options = ParserOptions { minify: Some(true), source: Some(source.clone()), sources_map: Some(sources), ..Default::default() };
    let start = Instant::now();
    let tokenizer = Tokenizer::new(create_parse_info(Some(&CssInput::from(css)), source));
    let result = do_parse_sync(tokenizer, options, None, Rc::new(NoHost)).expect("parse");
    let out = do_render(&result.arena, result.ast, &ParserOptions { minify: Some(true), ..Default::default() }, None);
    let elapsed = start.elapsed();

    assert!(!result.errors.is_empty());
    // minified: about 40% of the input. 21 MB of the input are in `@media not all and (...)` blocks
    assert!(out.code().len() > 8_000_000 && out.code().len() < 11_000_000, "{}", out.code().len());
    // a debug build and a build instrumented for the coverage (scripts/coverage.sh) are slower: the bound only catches a
    // quadratic regression
    let slow = cfg!(debug_assertions) || std::env::var_os("CARGO_LLVM_COV").is_some();
    assert!(elapsed < Duration::from_secs(if slow { 60 } else { 10 }), "{:?}", elapsed);
}
