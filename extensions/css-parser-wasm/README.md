# @tbela99/rust-css-parser-wasm

CSS parser, validator, minifier and renderer for Node.js, Deno, Bun and the browsers, with a WebAssembly module: the Rust port of
[@tbela99/css-parser](https://github.com/tbela99/css-parser), with the same JavaScript API.

```shell
npm install @tbela99/rust-css-parser-wasm
# or
npx jsr add @tbela99/rust-css-parser-wasm
```

```js
import { transform, ColorType } from "@tbela99/rust-css-parser-wasm";       // node, deno, bun
// import { transform, ColorType } from "@tbela99/rust-css-parser-wasm/web"; // browsers

const result = await transform(".a { color: rgb(255 0 0); width: calc(1px + 2px) }", {
    beautify: true,
    convertColor: ColorType.SRGB,
});

console.log(result.code);
```

See the [guides](https://github.com/tbela99/rust-css-parser/tree/master/files) for the API.

## License

[MIT](LICENSE.md)
