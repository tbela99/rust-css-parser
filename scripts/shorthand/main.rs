//! Port of `scripts/shorthand.ts` (`npm run config-update`): the shorthand computation rules
//! (`crates/css-parser/src/data/properties.json`).
//!
//! `npm run config-update` (`cargo run --release --manifest-path scripts/Cargo.toml --bin shorthand`). Environment: `CSS_DATA_OUTPUT_DIR` (default `crates/css-parser/src/data`).
//!
//! The definitions are the literals of the typescript script (`map.json`, `properties.json`, `property.json`, in their
//! source order). The objects are built with the javascript semantics of the script: an assigned key keeps its position
//! when it exists, a spread copies the keys in order.

use serde_json::{Map, Value};
use std::path::PathBuf;

/// `Object.assign(target, source)`
fn assign(target: &mut Map<String, Value>, source: Map<String, Value>) {
    for (key, value) in source {
        // an existing key keeps its position
        target.insert(key, value);
    }
}

fn object(value: &Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn str_of(value: &Value) -> String {
    value.as_str().unwrap_or("undefined").to_string()
}

/// the definitions of a shorthand and of its properties
fn create_properties(data: &Value) -> Map<String, Value> {
    let shorthand = str_of(&data["shorthand"]);
    let map = data.get("map").cloned();
    let mut properties = Map::new();

    for property in data["properties"].as_array().into_iter().flatten() {
        let mut definition = Map::new();
        // `{ map, shorthand }`: an undefined map is not serialized
        if let Some(map) = &map {
            definition.insert("map".into(), map.clone());
        }
        definition.insert("shorthand".into(), Value::String(shorthand.clone()));
        let mut entry = Map::new();
        entry.insert(str_of(property), Value::Object(definition));
        assign(&mut properties, entry);
    }

    // `{ [data.shorthand]: { ...data }, ...properties }`
    let mut result = Map::new();
    result.insert(shorthand, Value::Object(object(data)));
    assign(&mut result, properties);
    result
}

/// the definitions of a shorthand map and of its fields
fn create_map(data: &Value, fields: &Value) -> Map<String, Value> {
    let shorthand = str_of(&data["shorthand"]);
    let mut definition = object(data);
    definition.insert("properties".into(), Value::Object(Map::new()));
    let mut acc = Map::new();
    acc.insert(shorthand.clone(), Value::Object(definition));

    for field in fields.as_array().into_iter().flatten() {
        let mut properties = Map::new();
        properties.insert(str_of(&field["shorthand"]), Value::Object(object(&field["properties"])));
        if let Some(Value::Object(target)) = acc.get_mut(&shorthand).and_then(|d| d.get_mut("properties")) {
            assign(target, properties);
        }

        let mut entry = Map::new();
        let mut reference = Map::new();
        reference.insert("shorthand".into(), Value::String(shorthand.clone()));
        entry.insert(str_of(&field["shorthand"]), Value::Object(reference));
        assign(&mut acc, entry);
    }

    acc
}

fn main() {
    let parse = |source: &str| -> Value { serde_json::from_str(source).expect("shorthand definitions") };
    let definitions = parse(include_str!("map.json"));
    let property_definitions = parse(include_str!("properties.json"));
    let property = parse(include_str!("property.json"));

    let mut map = Map::new();
    for data in definitions.as_array().into_iter().flatten() {
        assign(&mut map, create_map(&data[0], &data[1]));
    }

    let mut properties = Map::new();
    for data in property_definitions.as_array().into_iter().flatten() {
        if let Some(group) = data.get("map").and_then(|m| m.as_str()) {
            let shorthand = str_of(&data["shorthand"]);
            if let Some(Value::Object(target)) = map.get_mut(group).and_then(|m| m.get_mut("properties")).and_then(|p| p.get_mut(&shorthand)) {
                for key in ["types", "default", "keywords"] {
                    match data.get(key) {
                        Some(value) => {
                            target.insert(key.into(), value.clone());
                        }
                        // assigned undefined: not serialized
                        None => {
                            target.shift_remove(key);
                        }
                    }
                }
            }
        }
        assign(&mut properties, create_properties(data));
    }

    let mut result = Map::new();
    result.insert("properties".into(), Value::Object(properties));
    result.insert("map".into(), Value::Object(map));
    result.insert("property".into(), property);
    let result = serde_json::to_string(&Value::Object(result)).expect("json");

    let output_directory = std::env::var("CSS_DATA_OUTPUT_DIR").map(PathBuf::from).unwrap_or_else(|_| repository_root().join("crates/css-parser/src/data"));
    let output_file = output_directory.join("properties.json");
    std::fs::write(&output_file, &result).unwrap_or_else(|e| panic!("{}: {}", output_file.display(), e));

    println!("{}", result);
}

/// the root of the repository (the parent of `scripts/`)
fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().expect("repository root").to_path_buf()
}
