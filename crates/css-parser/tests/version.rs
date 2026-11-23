//! the version of the crate is the version of package.json (see build.rs)

#[test]
fn version_is_the_package_version() {
    let package = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../package.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&package).unwrap();
    assert_eq!(rust_css_parser::VERSION, json["version"].as_str().unwrap());
    // the workspace Cargo.toml is kept in sync by build-bindings.sh
    assert_eq!(env!("CARGO_PKG_VERSION"), json["version"].as_str().unwrap());
}
