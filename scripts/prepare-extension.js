// the prepare script of the extensions (package.json): builds the package when it is installed from git, where dist/
// is not committed, e.g. pnpm add "github:tbela99/rust-css-parser#v.next&path:/extensions/css-parser-wasm"
// usage: node ../../scripts/prepare-extension.js napi|wasm
// - napi: the javascript modules and the rust sources: the install script of the package builds the addon
// - wasm: the javascript modules and the WebAssembly module (the Rust toolchain and the wasm32 target are required)
// nothing is done when dist/ exists, or by `npm install` at the root of the monorepo (it runs the prepare script of
// the workspaces): there, `npm run build` builds the extensions

import { spawnSync } from "node:child_process";
import { existsSync, realpathSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const name = process.argv[2];
const targets = { napi: "napi-package", wasm: "wasm" };

if (!(name in targets)) {
    console.error(`usage: node prepare-extension.js napi|wasm`);
    process.exit(1);
}

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

// the directory the install was started from
const initCwd = process.env.INIT_CWD;
const monorepoInstall = initCwd != null && existsSync(initCwd) && realpathSync(initCwd) === realpathSync(root);

if (monorepoInstall || existsSync(join(root, "extensions", `css-parser-${name}`, "dist"))) {
    process.exit(0);
}

const build = spawnSync("sh", [join(root, "build-bindings.sh"), targets[name]], { stdio: "inherit" });

process.exit(build.status ?? 1);
