//! render every syntax of the validation data after parsing it (parity check, see scripts/parity/syntaxes.js)
use rust_css_parser::validation::json::config_json;
use rust_css_parser::validation::parser::parse::{parse_syntax, render_syntax, RenderSyntaxOptions};

fn main() {
    let config = config_json();
    for group in ["atRules", "declarations", "mediaFeatures", "selectors", "syntaxes", "types"] {
        for (name, node) in config[group].as_object().unwrap() {
            let mut syntaxes = vec![];
            if let Some(s) = node["syntax"].as_str() {
                syntaxes.push(("".to_string(), s.to_string()));
            }
            if let Some(d) = node["descriptors"].as_object() {
                for (k, v) in d {
                    let s = v.as_str().map(String::from).or_else(|| v["syntax"].as_str().map(String::from));
                    if let Some(s) = s {
                        syntaxes.push((k.clone(), s));
                    }
                }
            }
            for (key, syntax) in syntaxes {
                let tokens = parse_syntax(&syntax);
                let mut options = RenderSyntaxOptions::default();
                let rendered: String = tokens.iter().map(|t| render_syntax(t, &mut options)).collect();
                println!("{}\t{}\t{}\t{}", group, name, key, rendered);
            }
        }
    }
}
