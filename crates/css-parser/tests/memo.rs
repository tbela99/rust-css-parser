//! identical constructs are validated once and rendered once per render, like the typescript memos
use rust_css_parser::host::NoHost;
use rust_css_parser::options::{ParserOptions, SourcesMap};
use rust_css_parser::parser::input::CssInput;
use rust_css_parser::parser::parse::do_parse_sync;
use rust_css_parser::parser::source::SourceFile;
use rust_css_parser::parser::tokenize::{create_parse_info, Tokenizer};
use rust_css_parser::printer::render::{declaration_renders, do_render};
use rust_css_parser::validation::match_::declaration_validations;
use std::rc::Rc;

fn parse(css: &str) -> rust_css_parser::parser::parse_main::ParseResult {
    let source = SourceFile::new(Rc::from(""), Vec::new(), Some(Rc::from("")));
    let sources = SourcesMap::default();
    sources.borrow_mut().insert(source.id, source.clone());
    let options = ParserOptions {
        minify: Some(false),
        validation: Some(true),
        source: Some(source.clone()),
        sources_map: Some(sources),
        ..Default::default()
    };
    let tokenizer = Tokenizer::new(create_parse_info(Some(&CssInput::from(css.as_bytes().to_vec())), source));
    do_parse_sync(tokenizer, options, None, Rc::new(NoHost)).expect("parse")
}

#[test]
fn identical_declarations_are_validated_once() {
    let before = declaration_validations();
    // a value that no other test validates on this thread
    let _ = parse(".s{width:12.5px}.m{width:12.5px}.n{width:12.5px}");
    assert_eq!(declaration_validations() - before, 1);
}

#[test]
fn identical_declarations_are_rendered_once() {
    let result = parse(".s{width:10px}.m{width:10px}.n{width:10px}");
    let before = declaration_renders();
    let out = do_render(&result.arena, result.ast, &ParserOptions { minify: Some(true), ..Default::default() }, None);
    assert_eq!(declaration_renders() - before, 1);
    assert_eq!(String::from_utf8(out.bytes).unwrap(), ".s{width:10px}.m{width:10px}.n{width:10px}");
}
