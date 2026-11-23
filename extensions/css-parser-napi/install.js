// Builds the N-API addon on the client machine (the `install` script of the package): the Rust sources shipped in
// rust/ are compiled by cargo, the library is copied to dist/css_parser.node, then the build directory is removed.
//
// Requires the Rust toolchain (https://rustup.rs). Environment:
// - CARGO: the cargo executable (default: cargo)
// - CSS_PARSER_NAPI_SKIP_BUILD=1: do not build (the addon is provided some other way)

import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const rust = join(here, "rust");
const addon = join(here, "dist", "css_parser.node");

// in the monorepo, `npm run build:napi` builds the addon
const monorepo = existsSync(join(here, "..", "..", "crates", "css-parser", "Cargo.toml"));

if (monorepo || process.env.CSS_PARSER_NAPI_SKIP_BUILD) {
    process.exit(0);
}

const libraries = {
    darwin: "libcss_parser_napi.dylib",
    linux: "libcss_parser_napi.so",
};
const library = libraries[process.platform];

if (library === undefined) {
    console.error(
        `@tbela99/rust-css-parser-napi: the N-API addon cannot be built on ${process.platform} (macOS and Linux only). ` +
            "Use @tbela99/rust-css-parser-wasm instead.",
    );
    process.exit(1);
}

const cargo = process.env.CARGO ?? "cargo";

if (spawnSync(cargo, ["--version"], { stdio: "ignore" }).status !== 0) {
    console.error(
        "@tbela99/rust-css-parser-napi: cargo is missing. The N-API addon is built on install: install the Rust " +
            "toolchain (https://rustup.rs), then reinstall the package. Or use @tbela99/rust-css-parser-wasm.",
    );
    process.exit(1);
}

const target = join(rust, "target");
// the versions of the dependencies are the ones of rust/Cargo.lock (no --locked: the lock file of the monorepo also
// lists the WebAssembly extension, cargo prunes it)
const args = ["build", "--release", "-p", "css-parser-napi", "--manifest-path", join(rust, "Cargo.toml")];

console.log("@tbela99/rust-css-parser-napi: building the N-API addon (cargo build --release)");

const build = spawnSync(cargo, args, {
    stdio: "inherit",
    env: { ...process.env, CARGO_TARGET_DIR: target },
});

if (build.status !== 0) {
    console.error("@tbela99/rust-css-parser-napi: the build of the N-API addon failed");
    process.exit(build.status ?? 1);
}

copyFileSync(join(target, "release", library), addon);
rmSync(target, { recursive: true, force: true });
