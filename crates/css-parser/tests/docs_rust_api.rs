//! the example of files/rust.md: keep both in sync

use rust_css_parser::host::NoHost;
use rust_css_parser::options::{ParserOptions, SourcesMap};
use rust_css_parser::parser::input::CssInput;
use rust_css_parser::parser::parse::do_parse_sync;
use rust_css_parser::parser::source::SourceFile;
use rust_css_parser::parser::tokenize::{create_parse_info, Tokenizer};
use rust_css_parser::printer::render::do_render;
use std::rc::Rc;

#[test]
fn parse_minify_render() {
    let css = ".a { color: rgb(255 0 0); width: calc(1px + 2px) } .b { color: red }";

    // the source file: the locations, the source maps and the string table refer to it
    let source = SourceFile::new(Rc::from(css), Vec::new(), Some(Rc::from("in.css")));
    let sources = SourcesMap::default();
    sources.borrow_mut().insert(source.id, source.clone());

    let options = ParserOptions { minify: Some(true), source: Some(source.clone()), sources_map: Some(sources), ..Default::default() };
    let tokenizer = Tokenizer::new(create_parse_info(Some(&CssInput::from(css)), source));

    // NoHost: no javascript callbacks (visitors, load(), ...)
    let result = do_parse_sync(tokenizer, options, None, Rc::new(NoHost)).expect("parse");

    // the ast is a node id of the arena
    assert_eq!(result.arena.child_count(result.ast), 2);

    let rendered = do_render(&result.arena, result.ast, &ParserOptions { minify: Some(true), ..Default::default() }, None);

    assert_eq!(rendered.code(), ".a{color:red;width:3px}.b{color:red}");
}

#[test]
fn arena_children() {
    let css = ".a { color: red; width: 3px }";
    let source = SourceFile::new(Rc::from(css), Vec::new(), Some(Rc::from("")));
    let sources = SourcesMap::default();
    sources.borrow_mut().insert(source.id, source.clone());
    let options = ParserOptions { source: Some(source.clone()), sources_map: Some(sources), ..Default::default() };
    let result = do_parse_sync(Tokenizer::new(create_parse_info(Some(&CssInput::from(css)), source)), options, None, Rc::new(NoHost)).expect("parse");

    // files/rust.md, "The arena"
    let arena = &result.arena;
    let rule = arena.first_child.get(result.ast);
    let mut names = Vec::new();

    for declaration in arena.children(rule) {
        names.push(arena.get_name(declaration).to_string());
    }

    assert_eq!(names, ["color", "width"]);
}
