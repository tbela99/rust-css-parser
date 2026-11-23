#!/bin/sh
# build the extensions of the workspace: sh build-bindings.sh [napi] [wasm] (both by default)
#  - napi: extensions/css-parser-napi/dist, the javascript modules and the N-API addon (css_parser.node)
#  - wasm: extensions/css-parser-wasm/dist, the javascript modules and the WebAssembly module (css_parser.wasm)
# the javascript modules are shared (bindings/), the backend (backend.js) is the one of the extension
set -eu
cd "$(dirname "$0")"
targets=${*:-napi wasm}

# the version of package.json, like the typescript build (src/version.ts): the javascript module, the crates and the
# packages of the extensions
version=$(node -p 'require("./package.json").version')
printf '/**\n * Version number\n */\nexport const version = "%s";\n' "$version" > bindings/version.js
node -e '
const fs = require("fs");
const version = process.argv[1];
const toml = fs.readFileSync("Cargo.toml", "utf8");
// the version key of the [workspace.package] section
const updated = toml.replace(/(\[workspace\.package\][^\[]*?\nversion = ")[^"]*(")/, `$1${version}$2`);
if (updated !== toml) fs.writeFileSync("Cargo.toml", updated);
for (const name of ["napi", "wasm"]) {
    // the npm package and the jsr package of the extension
    for (const manifest of ["package.json", "jsr.json"]) {
        const file = `extensions/css-parser-${name}/${manifest}`;
        const json = JSON.parse(fs.readFileSync(file, "utf8"));
        if (json.version !== version) {
            json.version = version;
            fs.writeFileSync(file, JSON.stringify(json, null, 2) + "\n");
        }
    }
}
' "$version"

# the javascript modules, with the layout of the typescript build, and the backend of the extension
assemble() {
    dist="extensions/css-parser-$1/dist"
    rm -rf "$dist"
    mkdir -p "$dist"
    cp -R bindings/. "$dist/"
    cp "extensions/css-parser-$1/backend.js" "$dist/"
    # npm and jsr publish the license of the package directory
    cp LICENSE.md "extensions/css-parser-$1/LICENSE.md"
}

# the rust sources of the N-API addon, published in the package (rust/): the addon is built on the client machine by
# install.js. a cargo workspace of the library and the extension, without the binaries and the tests of the library
stage_napi_sources() {
    rust=extensions/css-parser-napi/rust
    rm -rf "$rust"
    mkdir -p "$rust/crates/css-parser" "$rust/extensions/css-parser-napi"
    cp crates/css-parser/Cargo.toml crates/css-parser/build.rs "$rust/crates/css-parser/"
    cp -R crates/css-parser/src "$rust/crates/css-parser/"
    rm -rf "$rust/crates/css-parser/src/bin"
    cp -R extensions/css-parser-napi/Cargo.toml extensions/css-parser-napi/build.rs extensions/css-parser-napi/src \
        "$rust/extensions/css-parser-napi/"
    # the workspace manifest without the wasm extension
    node -e '
const fs = require("fs");
const toml = fs.readFileSync("Cargo.toml", "utf8")
    .replace(/members = \[[^\]]*\]/, `members = [\n    "crates/css-parser",\n    "extensions/css-parser-napi",\n]`)
    .replace(/\n# the data generators[^\n]*\nexclude = \[[^\]]*\]\n/, "\n");
fs.writeFileSync(process.argv[1], toml);
' "$rust/Cargo.toml"
    if [ -f Cargo.lock ]; then cp Cargo.lock "$rust/"; fi
    # the library reads its version from the package.json two directories above it (crates/css-parser/build.rs)
    printf '{\n  "version": "%s"\n}\n' "$version" > "$rust/package.json"
}

for target in $targets; do
    case "$target" in
        napi)
            case "$(uname -s)" in
                Darwin) library=libcss_parser_napi.dylib ;;
                Linux) library=libcss_parser_napi.so ;;
                *) echo 'the N-API build supports macOS and Linux.' >&2; exit 1 ;;
            esac
            rustup run stable cargo build --workspace --release -p css-parser-napi
            assemble napi
            # the N-API addon has no browser entry point
            rm -f extensions/css-parser-napi/dist/web.js
            cp "target/release/$library" extensions/css-parser-napi/dist/css_parser.node
            stage_napi_sources
            ;;
        wasm)
            # the WASM module is optimized by wasm-opt (binaryen, a dev dependency: npm install)
            wasm_opt=node_modules/.bin/wasm-opt
            if [ ! -x "$wasm_opt" ]; then
                echo 'wasm-opt is missing: run npm install' >&2
                exit 1
            fi
            rustup run stable cargo build --workspace --release -p css-parser-wasm --target wasm32-unknown-unknown
            assemble wasm
            "$wasm_opt" -O3 target/wasm32-unknown-unknown/release/css_parser_wasm.wasm -o extensions/css-parser-wasm/dist/css_parser.wasm
            ;;
        *) echo "unknown extension: $target (napi or wasm)" >&2; exit 1 ;;
    esac
done
