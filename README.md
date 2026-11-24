# CSS parser — Rust port

Rust port of [@tbela99/css-parser](https://github.com/tbela99/css-parser) 2.2.0: a fault-tolerant CSS parser,
validator, minifier and renderer, for Node.js (N-API) and the browsers (WebAssembly), with the same JavaScript API as
the TypeScript library.

The crate is a file by file port of the TypeScript sources (see [PORTING.md](PORTING.md)). The specs of the TypeScript
project run unchanged against the Rust builds.

## Layout

The repository is a monorepo: a Cargo workspace and an npm workspace.

| Directory | Cargo crate | npm package | |
|-----------|-------------|-------------|---|
| `crates/css-parser` | `rust-css-parser` (rlib) | | the library: parser, validator, minifier, renderer, the call interface of the bindings |
| `extensions/css-parser-napi` | `css-parser-napi` (cdylib) | `css-parser-napi` | Node.js, N-API addon (`css_parser.node`) |
| `extensions/css-parser-wasm` | `css-parser-wasm` (cdylib) | `css-parser-wasm` | Node.js and the browsers, WebAssembly module (`css_parser.wasm`) |
| `bindings/` | | | the JavaScript modules shared by both packages; each package adds its `backend.js` |

## Installation

### From NPM

```shell
npm install @tbela99/rust-css-parser-wasm   # Node.js and the browsers
npm install @tbela99/rust-css-parser-napi   # Node.js, macOS and Linux
```

`@tbela99/rust-css-parser-napi` builds its addon when it is installed: it requires the
[Rust toolchain](https://rustup.rs) (`cargo`). npm versions that block install scripts need an approval first:

```shell
npm install-scripts approve @tbela99/rust-css-parser-napi
npm rebuild @tbela99/rust-css-parser-napi
```

### From JSR

```shell
deno add jsr:@tbela99/rust-css-parser-wasm
npx jsr add @tbela99/rust-css-parser-wasm
npx jsr add @tbela99/rust-css-parser-napi
```

JSR does not run install scripts: after installing `@tbela99/rust-css-parser-napi`, build the addon with
`node node_modules/@tbela99/rust-css-parser-napi/install.js`. With npm 12 or later, `npx jsr add` needs
`npm_config_allow_remote=all` (npm refuses the JSR tarballs, served from another host, by default).

From Deno, the WebAssembly module is fetched from jsr.io when the package is imported: allow it with
`deno run --allow-net=jsr.io`.

### From crates.io

```shell
cargo add rust-css-parser
```

## Build

```shell
rustup target add --toolchain stable wasm32-unknown-unknown
npm install
npm run build          # both extensions
npm run build:napi     # css-parser-napi only
npm run build:wasm     # css-parser-wasm only
```

`npm run build` writes `extensions/css-parser-napi/dist/` (the JavaScript modules and the N-API addon) and
`extensions/css-parser-wasm/dist/` (the JavaScript modules and the WebAssembly module, optimized by `wasm-opt`).

## Usage

```js
import { transform, ColorType } from "@tbela99/rust-css-parser-napi"; // or "@tbela99/rust-css-parser-wasm"

const result = await transform(`.a { color: rgb(255 0 0); width: calc(1px + 2px) }`, {
    beautify: true,
    convertColor: ColorType.SRGB,
});

console.log(result.code);
```

- `@tbela99/rust-css-parser-napi` (`dist/node.js`): Node.js, N-API.
- `@tbela99/rust-css-parser-wasm`: Node.js (`@tbela99/rust-css-parser-wasm/node`, `dist/node.js`) and the browsers (`@tbela99/rust-css-parser-wasm/web`,
  `dist/web.js`), WebAssembly.

The API is the API of the TypeScript library: see the [guides](files/index.md) and the
[JavaScript API reference](https://tbela99.github.io/css-parser/docs/modules/node.html). The crate can be used from
Rust: see [Rust API](files/rust.md).

## Documentation

- [files/](files/index.md): the guides (markdown)
- `npm run doc`: the guides converted to html and the Rust API reference (`cargo doc`), in `docs/`

## Tests

```shell
npm run test:rust    # cargo test --release --workspace
npm run test:napi    # the TypeScript specs against css-parser-napi
npm run test:wasm    # the TypeScript specs against css-parser-wasm in Node.js
npm run test:node    # test:napi, then test:wasm
npm run test:web     # the TypeScript specs against css-parser-wasm in Chromium, Firefox and WebKit (npx playwright install)
```

The specs import `#dist/...` (the `imports` of the root package.json): the `napi` and `wasm` conditions (set by
`test:napi` and `test:wasm`) select the package, the browsers get `css-parser-wasm`.

### CI

- `.github/workflows/ci.yml` (push, pull request): the Rust tests and the specs, and `napi-install`: the N-API package
  is packed, installed outside of the repository on Linux and macOS, built from its sources, then smoke tested.
- `.github/workflows/publish.yml` (a version tag `v*`, or by hand): runs the CI, checks that the tag is the version of
  package.json, builds once, then publishes both packages to npm (with provenance) and to JSR, and the library
  (`rust-css-parser`) to crates.io. Authentication: npm trusted publishing or an `NPM_TOKEN` secret; JSR: link the
  repository in the settings of each package on jsr.io; crates.io: trusted publishing or a `CARGO_REGISTRY_TOKEN`
  secret.
- crates.io by hand: `npm run publish:crate` (the Rust tests, then `cargo publish -p rust-css-parser`; `cargo login`
  first). `npm run publish:dry-run` also packages the crate and builds it from the package.

## Data

| Command | Output |
|---------|--------|
| `npm run syntax-update` | `crates/css-parser/src/data/validation.json`: the syntaxes of the validation, from [@webref/css](https://github.com/w3c/webref/tree/main/packages/css) |
| `npm run config-update` | `crates/css-parser/src/data/properties.json`: the shorthand properties |

## License

[MIT](LICENSE.md)
