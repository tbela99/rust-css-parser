---
title: Getting started
group: Documents
category: Guides
---

## About

CSS-Parser is a high-performance, fault-tolerant, all-in-one CSS parsing solution for Node.js and browsers.

This repository is the Rust port of [@tbela99/css-parser](https://github.com/tbela99/css-parser): the parser, the validator, the minifier and the renderer are written in Rust, and the JavaScript API is the same as the TypeScript library. Node.js uses the N-API addon, the browsers use the WebAssembly module (see [Bindings](./bindings.md)). The Rust crate can be used directly (see [Rust API](./rust.md)).

It implements the [CSS Syntax Module Level 3](https://www.w3.org/TR/css-syntax-3/) specification and validates CSS using the syntax definitions of the W3C specifications from [@webref/css](https://github.com/w3c/webref/tree/main/packages/css). Every stylesheet is fully parsed into a structured AST, and token values are exposed as typed data so the library can support robust transformations, validation, and plugin-oriented workflows without falling back to raw strings.

In addition to parsing and validation, CSS-Parser provides advanced optimization and minification capabilities. According to [this benchmark](https://tbela99.github.io/css-parser/benchmark/index.html), it is the most efficient CSS minifier available, producing smaller output than competing solutions while maintaining competitive performance.

## Playground

Want to see it in action? Try the [CSS-Parser playground](https://tbela99.github.io/css-parser/playground/).

## Features

A non-exhaustive list of features is provided below:


* **Native code** — the N-API addon in Node.js, WebAssembly in the browsers, the same API as the TypeScript library.
* **All-in-one CSS parsing solution** for parsing, validation, transformation, and minification.
* **Standards-based CSS validation** powered by the W3C specification definitions ([@webref/css](https://github.com/w3c/webref/tree/main/packages/css)).
* **Full CSS Modules support** for modern component-based workflows.
* **Fault-tolerant parsing** that follows the CSS Syntax Module Level 3 specification and always yields a complete parse tree.
* **Typed tokens and AST** — parsed CSS is exposed as strongly typed tokens and nodes for safer plugin and transform logic.
* **High-performance minification** with safe optimizations and no unsafe transforms.
* **Advanced color processing** with support for modern color spaces and functions, including `color()`, `lab()`, `lch()`, `oklab()`, `oklch()`, `color-mix()`, `light-dark()`, system colors, and relative colors.
* **Color conversion engine** capable of transforming colors between all supported formats.
* **Automatic CSS nesting generation** from compatible selectors.
* **Nested CSS transpilation** to legacy-compatible syntax.
* **`if()` function transpilation** for broader browser compatibility.
* **Source map generation** for easier debugging and development workflows.
* **Shorthand property computation** to reduce output size and improve optimization.
* **Transform function optimization** for more compact CSS output.
* **Math function evaluation**, including `calc()`, `clamp()`, `min()`, `max()`, and related expressions.
* **CSS variable inlining** where values can be safely resolved.
* **Duplicate declaration removal** to eliminate redundant rules.
* **`@import` flattening** to produce self-contained stylesheets.
* **Vendor prefix cleanup** to modernize generated CSS.
* **Binary input** — the css can be a string, an `ArrayBuffer` or an `ArrayBufferView` (utf-8 bytes, `Uint8Array`, `Buffer`): an ASCII buffer is tokenized without copy.
* **Experimental**: evaluate @mixin and @function

## Installation

Two packages are available:

- `@tbela99/rust-css-parser-wasm`: the WebAssembly module, for Node.js and the browsers
- `@tbela99/rust-css-parser-napi`: the N-API addon, for Node.js on macOS and Linux (the fastest)

### From NPM

```shell
$ npm install @tbela99/rust-css-parser-wasm
$ npm install @tbela99/rust-css-parser-napi
```

`@tbela99/rust-css-parser-napi` builds its addon when it is installed: it requires the
[Rust toolchain](https://rustup.rs) (`cargo`). npm versions that block install scripts need an approval first:

```shell
$ npm install-scripts approve @tbela99/rust-css-parser-napi
$ npm rebuild @tbela99/rust-css-parser-napi
```

### From JSR

```shell
$ deno add jsr:@tbela99/rust-css-parser-wasm
$ npx jsr add @tbela99/rust-css-parser-wasm
$ npx jsr add @tbela99/rust-css-parser-napi
```

JSR does not run install scripts: after installing `@tbela99/rust-css-parser-napi`, build the addon with
`node node_modules/@tbela99/rust-css-parser-napi/install.js`. With npm 12 or later, `npx jsr add` needs
`npm_config_allow_remote=all` (npm refuses the JSR tarballs, served from another host, by default).

From Deno, the WebAssembly module is fetched from jsr.io when the package is imported: allow it with
`deno run --allow-net=jsr.io`.

### From crates.io

The Rust library, see [Rust API](./rust.md):

```shell
$ cargo add rust-css-parser
```

### From the sources

Install the Rust toolchain and the WebAssembly target, then build both packages into `extensions/*/dist/`:

```shell
$ rustup target add --toolchain stable wasm32-unknown-unknown
$ npm install
$ npm run build
```

## From node, bun or deno

```ts

import {transform, ColorType} from '@tbela99/rust-css-parser-napi'; // or '@tbela99/rust-css-parser-wasm'

const css = `

.ruler {

    height: 10px;
    background-color: orange
}
.hsl { color: #b3222280; }
`;

const result = await transform(css, {

    beautify: true,
    convertColor: ColorType.SRGB
});

console.debug(result.code);
```

The library is published as two packages:

- `css-parser-napi` (or `css-parser-napi/node`) which relies on node fs and fs/promises to read files. It loads the
  N-API addon
- `css-parser-wasm`, which loads the WebAssembly module: `css-parser-wasm/node` relies on node fs and fs/promises,
  `css-parser-wasm/web` on the web fetch api to read files. `css-parser-wasm` selects the entry point of the
  environment

The library is an ES module. The WebAssembly module is loaded when the entry point is imported (top-level `await`).

## From the web browser

Load as javascript module. The WebAssembly module (`dist/css_parser.wasm` of `css-parser-wasm`) is fetched next to the
javascript modules: serve the `dist/` directory of the `css-parser-wasm` package.

```html
<script type="module">

    import {transform, ColorType} from './dist/web.js';

const css = `

.ruler {

    height: 10px;
    background-color: orange
}
.hsl { color: #b3222280; }
`;

const result = await transform(css, {

    beautify: true,
    convertColor: ColorType.RGB
});

console.debug(result.code);

</script>
```


------
[← Index](./index.md) | [Usage →](./usage.md) 