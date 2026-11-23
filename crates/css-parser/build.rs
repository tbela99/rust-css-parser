fn main() {
    // the version of the package (package.json of the repository root), like the typescript build (src/version.ts):
    // `crate::VERSION`
    let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../package.json");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let package = std::fs::read_to_string(&manifest).expect("package.json");
    println!("cargo:rustc-env=CSS_PARSER_VERSION={}", package_version(&package).expect("package.json: version"));
}

/// the value of the top level `"version"` key of package.json
fn package_version(package: &str) -> Option<String> {
    let mut depth = 0usize;
    let bytes = package.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth = depth.saturating_sub(1),
            b'"' => {
                let end = i + 1 + package[i + 1..].find('"')?;
                let key = &package[i + 1..end];
                i = end;
                // a top level key followed by a string value
                if depth == 1 && key == "version" {
                    let rest = package[end + 1..].trim_start().strip_prefix(':')?.trim_start();
                    let value = rest.strip_prefix('"')?;
                    return Some(value[..value.find('"')?].to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}
