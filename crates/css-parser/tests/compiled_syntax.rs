//! the compiled matchers (`validation::compiled`) agree with the interpreter: the css files of the tests are validated
//! in verified mode (the interpreter validates again every value validated by a compiled matcher, see
//! `set_compiled_validation()`). a value accepted by a compiled matcher must be accepted by the interpreter
use rust_css_parser::host::NoHost;
use rust_css_parser::options::{ParserOptions, SourcesMap};
use rust_css_parser::parser::input::CssInput;
use rust_css_parser::parser::parse::do_parse_sync;
use rust_css_parser::parser::source::SourceFile;
use rust_css_parser::parser::tokenize::{create_parse_info, Tokenizer};
use rust_css_parser::validation::match_::{compiled_stats, set_compiled_validation};
use std::path::PathBuf;
use std::rc::Rc;

fn parse(css: &str) {
    let source = SourceFile::new(Rc::from(""), Vec::new(), Some(Rc::from("")));
    let sources = SourcesMap::default();
    sources.borrow_mut().insert(source.id, source.clone());
    let options = ParserOptions {
        validation: Some(true),
        lenient: Some(true),
        source: Some(source.clone()),
        sources_map: Some(sources),
        ..Default::default()
    };
    let tokenizer = Tokenizer::new(create_parse_info(Some(&CssInput::from(css.as_bytes().to_vec())), source));
    do_parse_sync(tokenizer, options, None, Rc::new(NoHost)).expect("parse");
}

#[test]
fn compiled_matchers_agree_with_the_interpreter() {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/files/css");
    set_compiled_validation(2);

    for entry in std::fs::read_dir(&directory).expect("test/files/css") {
        let path = entry.expect("entry").path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        // larger.css repeats the other files
        if !name.ends_with(".css") || name.starts_with("larger") {
            continue;
        }
        parse(&std::fs::read_to_string(&path).expect("css"));
    }

    let stats = compiled_stats();
    println!("valid: {}, deferred: {} (bail: {}, uncompiled: {}, rejected and valid: {})", stats.valid, stats.deferred, stats.bail, stats.uncompiled, stats.rejected_valid);
    for (syntax, value) in stats.rejected_valid_samples.iter().take(20) {
        println!("deferred to the interpreter: {} >> {}", syntax, value);
    }
    assert!(stats.valid > 0, "no value validated by the compiled matchers");
    assert!(stats.mismatches.is_empty(), "accepted by the compiled matchers, rejected by the interpreter: {:?}", &stats.mismatches[..stats.mismatches.len().min(20)]);
}
