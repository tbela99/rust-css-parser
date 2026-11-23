//! Port of `src/validation/json.ts`: the validation data (`src/data/validation.json`), parsed once.

use std::sync::OnceLock;

pub static VALIDATION_JSON: &str = include_str!("../data/validation.json");

/// the parsed validation data (`config`)
pub fn config_json() -> &'static serde_json::Value {
    static CONFIG: OnceLock<serde_json::Value> = OnceLock::new();
    CONFIG.get_or_init(|| serde_json::from_str(VALIDATION_JSON).expect("validation.json"))
}
