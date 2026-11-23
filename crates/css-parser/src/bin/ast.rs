//! dump the plain object of the ast of a css file (parity check, see scripts/parity/ast.mjs)
use rust_css_parser::host::NoHost;
use rust_css_parser::options::{ParserOptions, SourcesMap};
use rust_css_parser::parser::input::CssInput;
use rust_css_parser::parser::parse::do_parse_sync;
use rust_css_parser::parser::source::SourceFile;
use rust_css_parser::parser::tokenize::{create_parse_info, Tokenizer};
use std::rc::Rc;

fn main() {
    let path = std::env::args().nth(1).expect("file");
    let validation = std::env::args().nth(2).as_deref() == Some("validation");
    let bytes = std::fs::read(&path).expect("read");
    // `parseInput()`: `options.src = resolve(options.src, options.cwd).relative`
    let src = rust_css_parser::fs::resolve::resolve("", "", None).relative;
    let source = SourceFile::new(Rc::from(""), Vec::new(), Some(Rc::from(src.as_str())));
    let sources = SourcesMap::default();
    sources.borrow_mut().insert(source.id, source.clone());
    let options = ParserOptions {
        minify: Some(false),
        validation: Some(validation),
        source: Some(source.clone()),
        sources_map: Some(sources),
        cwd: Some(String::new()),
        src: Some(src.clone()),
        ..Default::default()
    };
    let tokenizer = Tokenizer::new(create_parse_info(Some(&CssInput::from(bytes)), source));
    let result = do_parse_sync(tokenizer, options, None, Rc::new(NoHost)).expect("parse");
    println!("{}", result.arena.to_plain_object(result.ast).to_json());
    for e in &result.errors {
        println!("error\t{}", e.message);
    }
}
