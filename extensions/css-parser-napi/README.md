# @tbela99/rust-css-parser-napi

CSS parser, validator, minifier and renderer for Node.js, with a native N-API addon: the Rust port of
[@tbela99/css-parser](https://github.com/tbela99/css-parser), with the same JavaScript API.

```shell
npm install @tbela99/rust-css-parser-napi
# or
npx jsr add @tbela99/rust-css-parser-napi
```

The addon is built on install from the Rust sources of the package: it requires the
[Rust toolchain](https://rustup.rs) (cargo), on macOS or Linux. The build takes about a minute.

npm versions that block install scripts need an approval first:

```shell
npm install-scripts approve @tbela99/rust-css-parser-napi
npm rebuild @tbela99/rust-css-parser-napi
```

JSR does not run install scripts: after `npx jsr add`, build the addon with
`node node_modules/@tbela99/rust-css-parser-napi/install.js`.

The addon can also be built by hand: `node install.js` in the package directory. Without a Rust toolchain, use
[@tbela99/rust-css-parser-wasm](https://www.npmjs.com/package/@tbela99/rust-css-parser-wasm).

```js
import { transform, ColorType } from "@tbela99/rust-css-parser-napi";

const result = await transform(".a { color: rgb(255 0 0); width: calc(1px + 2px) }", {
    beautify: true,
    convertColor: ColorType.SRGB,
});

console.log(result.code);
```

See the [guides](https://github.com/tbela99/rust-css-parser/tree/master/files) for the API.

## License

[MIT](LICENSE.md)
