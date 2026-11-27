//! dump the tokens of css files (parity check with the typescript tokenizer, see scripts/parity/tokens.js)
use rust_css_parser::ast::arena::AstArena;
use rust_css_parser::parser::input::CssInput;
use rust_css_parser::parser::tokenize::{TokenValueField, Tokenizer};

fn main() {
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path).expect("read");
        let arena = AstArena::default();
        let mut tokenizer = Tokenizer::from_input(&CssInput::from(bytes));
        let mut out = String::new();
        loop {
            tokenizer.next();
            let typ = tokenizer.typ.unwrap_or(0);
            let value = if matches!(tokenizer.field, TokenValueField::Value | TokenValueField::Name | TokenValueField::Unit) {
                let id = tokenizer.intern_value(&arena, -1);
                arena.resolve_string(id).to_string()
            } else {
                String::new()
            };
            let num = if matches!(tokenizer.field, TokenValueField::Number | TokenValueField::Unit) || typ == 14 {
                rust_css_parser::utils::number::js_number_to_string(tokenizer.num)
            } else {
                String::new()
            };
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\n",
                typ,
                tokenizer.sta.unwrap_or(0),
                tokenizer.end.unwrap_or(0),
                tokenizer.field as u8,
                num,
                serde_json::to_string(&value).unwrap()
            ));
            if tokenizer.done() {
                break;
            }
        }
        let lines = tokenizer.source().line_starts.borrow().line_starts.len();
        out.push_str(&format!("lines\t{}\n", lines));
        print!("{}", out);
    }
}
