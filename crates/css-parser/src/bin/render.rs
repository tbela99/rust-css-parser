//! render a css file (parity check, see scripts/parity/render.js)
use rust_css_parser::host::NoHost;
use rust_css_parser::options::{ParserOptions, SourcesMap};
use rust_css_parser::parser::input::CssInput;
use rust_css_parser::parser::parse::do_parse_sync;
use rust_css_parser::parser::source::SourceFile;
use rust_css_parser::parser::tokenize::{create_parse_info, Tokenizer};
use rust_css_parser::printer::render::do_render;
use std::rc::Rc;

fn main() {
    let path = std::env::args().nth(1).expect("file");
    let mode = std::env::args().nth(2).unwrap_or_default();
    let bytes = std::fs::read(&path).expect("read");
    let src = rust_css_parser::fs::resolve::resolve("", "", None).relative;
    let source = SourceFile::new(Rc::from(""), Vec::new(), Some(Rc::from(src.as_str())));
    let sources = SourcesMap::default();
    sources.borrow_mut().insert(source.id, source.clone());
    // astmin: the ast is minified (default features)
    let astmin = mode == "astmin";
    let options = ParserOptions {
        minify: Some(astmin),
        source: Some(source.clone()),
        sources_map: Some(sources),
        cwd: Some(String::new()),
        src: Some(src.clone()),
        ..Default::default()
    };
    // PARSE_FLAGS: comma separated parser options set to true
    let mut options = options;
    for flag in std::env::var("PARSE_FLAGS").unwrap_or_default().split(',').filter(|f| !f.is_empty()) {
        match flag {
            "inlineCssVariables" => options.inline_css_variables = Some(true),
            "removePrefix" => options.remove_prefix = Some(true),
            "expandIfSyntax" => options.expand_if_syntax = Some(true),
            "evaluateMixins" => options.evaluate_mixins = Some(true),
            "validation" => options.validation = Some(true),
            "expandNestingRules" => options.expand_nesting_rules = Some(true),
            other => panic!("unknown flag {}", other),
        }
    }
    let tokenizer = Tokenizer::new(create_parse_info(Some(&CssInput::from(bytes)), source));
    let result_sources = options.sources_map.clone();
    let result = do_parse_sync(tokenizer, options, None, Rc::new(NoHost)).expect("parse");
    let render_options = match mode.as_str() {
        "beautify" => ParserOptions { minify: Some(false), beautify: Some(true), ..Default::default() },
        "map" => ParserOptions {
            minify: Some(false),
            beautify: Some(true),
            sourcemap: Some(rust_css_parser::options::SourceMapOption::On),
            sources_map: result_sources.clone(),
            ..Default::default()
        },
        "minify" | "astmin" => ParserOptions { minify: Some(true), ..Default::default() },
        _ => ParserOptions { minify: Some(false), ..Default::default() },
    };
    let out = do_render(&result.arena, result.ast, &render_options, None);
    println!("{}", String::from_utf8(out.bytes).expect("utf8"));
    if mode == "map" {
        println!("{}", out.map.expect("map").to_json().to_json());
    }
}
