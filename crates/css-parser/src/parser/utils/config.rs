//! Port of `src/parser/utils/config.ts`: the shorthand properties configuration (`src/data/properties.json`).

use std::sync::OnceLock;

pub static PROPERTIES_JSON: &str = include_str!("../../data/properties.json");

/// the properties configuration (`getConfig()`)
pub fn get_config() -> &'static serde_json::Value {
    static CONFIG: OnceLock<serde_json::Value> = OnceLock::new();
    CONFIG.get_or_init(|| serde_json::from_str(PROPERTIES_JSON).expect("properties.json"))
}

/// the groups of the properties configuration (`properties`, `map`, `property`) indexed by name: the lookups of the
/// minifier hash the name once with the fast hasher instead of the json object lookups
pub struct ConfigIndex {
    pub properties: crate::utils::fx::HashMap<&'static str, &'static serde_json::Value>,
    pub map: crate::utils::fx::HashMap<&'static str, &'static serde_json::Value>,
    pub property: crate::utils::fx::HashMap<&'static str, &'static serde_json::Value>,
}

/// `null`: the value of a missing key (`config[group][name]`)
pub static NULL: serde_json::Value = serde_json::Value::Null;

impl ConfigIndex {
    /// `config[group][name]`: `null` when the name is not defined
    pub fn get(map: &crate::utils::fx::HashMap<&'static str, &'static serde_json::Value>, name: &str) -> &'static serde_json::Value {
        map.get(name).copied().unwrap_or(&NULL)
    }
}

/// the index of the properties configuration
pub fn config_index() -> &'static ConfigIndex {
    static INDEX: OnceLock<ConfigIndex> = OnceLock::new();
    INDEX.get_or_init(|| {
        let config = get_config();
        let group = |key: &str| {
            config[key].as_object().map(|o| o.iter().map(|(k, v)| (k.as_str(), v)).collect()).unwrap_or_default()
        };
        ConfigIndex { properties: group("properties"), map: group("map"), property: group("property") }
    })
}
