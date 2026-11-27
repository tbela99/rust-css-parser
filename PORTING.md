# TypeScript → Rust port

Source of truth: `[css-parser](https://github.com/tbela99/css-parser)` (TypeScript). This crate is a **file-by-file port** of it: each
`src/<path>.ts` becomes `src/<path>.rs` (`-` becomes `_`), keeping the function names
(snake_case), structure and behaviour. Status: every module is ported; the TypeScript specs pass against the N-API and
the WASM builds (`npm run test:node`, `npm run test:web`). The previous reimplementation is kept for reference in
`.legacy/` (gitignored) and is not compiled.

## Design rules

1. **Arena** (`src/ast/arena.rs`): Rust-owned `Vec` columns (`Column<T>`, interior mutable through `&self`). JavaScript
   re-enters the arena (visitors, walker filters) while Rust transforms it, exactly like the shared mutable TS object:
   column accessors copy values in/out and never leak references, so this is sound and single threaded.
   `AstArena` wraps `Arena` and derefs to it (TS `extends`).
2. **Strings**: sources are kept once as `CodeUnits` (`Rc<[u8]>` when ascii, `Rc<[u16]>` otherwise) shared by the
   tokenizer, the sources and the string table. Interned strings are ranges; `Rc<str>` is created only when read
   (`resolve_string`) and cached. Use `with_string`, `string_equals*`, `string_starts_with` to avoid creating strings.
3. **Enums**: `src/ast/types.rs` is generated from the compiled TS enums (`scripts/gen/enums.js`): numeric constants
   in modules named like the TS enums (aliases included), plus `name()` / `value()` lookups.
4. **Async**: the TS pipeline awaits visitors, loaders and hash functions. The Rust ports are `async fn`s driven by a
   host executor: when Rust awaits a host promise it suspends and returns the promise id; JS awaits it and resumes the
   task. Same mechanism for N-API and WASM, no threads. Sync variants poll once.
5. **Bindings**: one low level ABI shared by N-API and WASM; a single JS glue layer (`bindings/`) exposes the exact TS
   public API (classes, overloads, enums). **Zero copy**: strings cross as `ArrayBuffer`/`Uint8Array` (UTF-8),
   outgoing data (rendered code, columns, interned strings) are views over Rust memory, decoded **lazily** by JS getters
   like the TS lazy getters. Column views are recreated when `Arena::generation()` changes.
6. Tests: the original suites (`test/specs/`, copied unchanged from the TS repository) run against `dist/`, the
   bindings, never against the TS build.

## Progress

Legend: `[x]` ported, `[~]` partial, `[ ]` todo.

- [x] `ast/arena-node.ts`
- [x] `ast/arena.ts`
- [x] `ast/cache.ts` → `ast/cache.rs` (`StructuralMemo`: HashMap keyed by the serialized i32 key)
- [x] `ast/features/calc.ts`
- [x] `ast/features/if.ts`
- [x] `ast/features/index.ts`
- [x] `ast/features/inlinecssvariables.ts`
- [x] `ast/features/mixin.ts`
- [x] `ast/features/prefix.ts`
- [x] `ast/features/shorthand.ts`
- [x] `ast/features/transform.ts`
- [x] `ast/features/type.ts`
- [x] `ast/math/expression.ts`
- [x] `ast/math/math.ts`
- [x] `ast/minify.ts`
- [x] `ast/mixins/custom-function.ts`
- [x] `ast/mixins/utils.ts`
- [x] `ast/mixins/variables.ts`
- [x] `ast/strings.ts`
- [x] `ast/transform/compute.ts`
- [x] `ast/transform/matrix.ts`
- [x] `ast/transform/minify.ts`
- [x] `ast/transform/perspective.ts`
- [x] `ast/transform/rotate.ts`
- [x] `ast/transform/scale.ts`
- [x] `ast/transform/skew.ts`
- [x] `ast/transform/translate.ts`
- [x] `ast/transform/utils.ts`
- [x] `ast/types.ts`
- [x] `ast/walk.ts`
- [x] `fs/resolve.ts`
- [x] `parser/declaration/list.ts`
- [x] `parser/declaration/map.ts`
- [x] `parser/declaration/set.ts`
- [x] `parser/input.ts`
- [x] `parser/linesmap.ts`
- [x] `parser/parse.ts` → `parser/parse.rs` + `parser/parse_main.rs` (doParse / doParseSync)
- [x] `parser/source.ts`
- [x] `parser/tokenize.ts` (token parity: tests/tokenizer_parity.sh)
- [x] `parser/utils/angle.ts`
- [x] `parser/utils/at-rule-container.ts`
- [x] `parser/utils/at-rule-font-feature-values.ts`
- [x] `parser/utils/at-rule-generic.ts`
- [x] `parser/utils/at-rule-import.ts`
- [x] `parser/utils/at-rule-media.ts`
- [x] `parser/utils/at-rule-support.ts`
- [x] `parser/utils/at-rule-when-else.ts`
- [x] `parser/utils/at-rule.ts`
- [x] `parser/utils/base64.ts`
- [x] `parser/utils/cache.ts`
- [x] `parser/utils/canonical.ts`
- [x] `parser/utils/codes.ts`
- [x] `parser/utils/config.ts`
- [x] `parser/utils/declaration.ts`
- [x] `parser/utils/eq.ts`
- [x] `parser/utils/hash.ts`
- [x] `parser/utils/selector.ts`
- [x] `parser/utils/text.ts`
- [x] `parser/utils/token.ts`
- [x] `parser/utils/type.ts`
- [x] `printer/render.ts` → `printer/render.rs` (with `RenderMemo`)
- [x] `printer/result.ts` → `printer/result.rs` (Rust `RenderResult`) + `bindings/printer/result.js`
  (`RenderResult`, `TransformResult`: the lazy `code` getter)
- [x] `printer/lib/codec.ts`
- [x] `printer/sourcemap.ts`
- [x] `printer/writer.ts`
- [x] `syntax/color/a98rgb.ts`
- [x] `syntax/color/alpha.ts`
- [x] `syntax/color/cmyk.ts`
- [x] `syntax/color/color-mix.ts`
- [x] `syntax/color/color.ts`
- [x] `syntax/color/hex.ts`
- [x] `syntax/color/hsl.ts`
- [x] `syntax/color/hwb.ts`
- [x] `syntax/color/lab.ts`
- [x] `syntax/color/lch.ts`
- [x] `syntax/color/oklab.ts`
- [x] `syntax/color/oklch.ts`
- [x] `syntax/color/p3.ts`
- [x] `syntax/color/prophotorgb.ts`
- [x] `syntax/color/rec2020.ts`
- [x] `syntax/color/relative-color.ts`
- [x] `syntax/color/rgb.ts`
- [x] `syntax/color/srgb.ts`
- [x] `syntax/color/utils/color-type.ts`
- [x] `syntax/color/utils/colorspace.ts`
- [x] `syntax/color/utils/components.ts`
- [x] `syntax/color/utils/distance.ts`
- [x] `syntax/color/utils/make-color.ts`
- [x] `syntax/color/utils/matrix.ts`
- [x] `syntax/color/xyz.ts`
- [x] `syntax/color/xyzd50.ts`
- [x] `syntax/constants.ts`
- [x] `syntax/syntax.ts`
- [x] `validation/config.ts`
- [x] `validation/first.ts`
- [x] `validation/json.ts`
- [x] `validation/match.ts` (match_.rs, match_syntax.rs, match_property.rs)
- [x] `validation/parser/parse.ts`
- [x] `validation/parser/typedef.ts`
- [x] `validation/utils/list.ts`
- [x] `node.ts` (`src/bindings/api.rs` + `bindings/node.js`)
- [x] `types.ts` (`bindings/types.js`)
- [x] `utils/sync.ts` (`src/bindings/api.rs`: `prepare`, `finish_parse`; `bindings/api.js`: `validateSyncArguments`)
- [x] `version.ts` (`bindings/version.js`, generated from package.json by `build-bindings.sh` like `rollup.config.js`
  generates `src/version.ts`; `Cargo.toml` is synced; `rust_css_parser::VERSION` is read from package.json by `build.rs`)
- [x] `web.ts` (`bindings/web.js`)

## Scripts

- [x] `scripts/webref.ts` → `scripts/webref.rs` (`npm run syntax-update`). Writes
  `src/data/validation.json`, `validation.origins.json`, appends to `SYNTAX-CHANGELOG.md`. `localeCompare()` is the ICU
  root collation for the definition names. Verified byte-identical with the typescript script (outputs, summary, log).
- [x] `scripts/shorthand.ts` → `scripts/shorthand/main.rs` (`npm run config-update`). The definitions are the source
  literals transcribed to JSON (`scripts/shorthand/*.json`). Verified byte-identical `src/data/properties.json`.
- `scripts/local-patch.json` is copied from the typescript repository.
- The generators are utilities, not part of the library: a separate crate (`scripts/Cargo.toml`, its own workspace and
  dependencies: `ureq`, `regex`).

## Bindings

- Rust: `crates/css-parser/src/bindings/{mod,value,api}.rs`, the backends in the crates
  `extensions/css-parser-napi` and `extensions/css-parser-wasm`. One call interface (`[method, ...args]` encoded in a shared
  exchange buffer) for the N-API addon and the WASM module.
- JavaScript: `bindings/` (copied with the `backend.js` of the extension to `extensions/css-parser-napi/dist/` and
  `extensions/css-parser-wasm/dist/` by `build-bindings.sh`, same module layout as the typescript build).
- The WASM module is optimized by `wasm-opt -O3` in `build-bindings.sh` (binaryen 123.0.0, dev dependency).
  `bindings/core.js` loads the backend of the package: the N-API addon (`css-parser-napi`) or the WASM module
  (`css-parser-wasm`, node and browsers).
- Arena columns are typed arrays over the rust memory (no copy), refreshed when the arena generation changes or the
  WASM memory grows. Render outputs are external `ArrayBuffer`s owning the rust buffer (N-API) / one copy (WASM).
- Walkers (`walk()`, `walkValues()`) are lazy: one rust call per step, the filters are javascript callbacks.
- Transferred arenas (`toTransferableArenaObject`/`fromTransferableArenaObjects`) use javascript column buffers,
  synchronized with rust around the calls that receive them, until the arena grows.
- The JavaScript enums (`bindings/ast/types.js`, ...) are generated from the TS build by `scripts/gen-enums.js`
  (data only). The Rust enums (`src/ast/types.rs`) by `scripts/gen/enums.js`.
- `npm run test:napi` / `npm run test:wasm` (`npm run test:node`: both): the specs against each package. `npm run test:web`: WASM in chromium, firefox, webkit.
- Design and usage: [files/bindings.md](files/bindings.md).

## Documentation

- `files/`: the guides, adapted from `files/` of the TS repository (same JavaScript API), plus `bindings.md` and
  `rust.md`. The Rust examples of `rust.md` are the test `tests/docs_rust_api.rs`.
- `npm run doc` (`scripts/docs.js`): the guides converted to html in `docs/`, and the Rust API reference
  (`cargo doc`) in `docs/api/`.

## Parity checks

- `sh tests/tokenizer_parity.sh ../css`: token stream (type, span, value, number) of every test css file.
- `../css/node_modules/.bin/tsx scripts/parity/syntaxes.js ../css/src` vs `cargo run --release --bin syntaxes`:
  parse + render of every syntax of `validation.json` (1936 syntaxes, identical).
- `node scripts/parity/ast.js ../css/dist <file> [validation]` vs `./target/release/ast <file> [validation]`: plain
  object of the ast + errors (compare without `srcId`: the dist consumes source id 0 while loading). All test css
  files identical except those hitting a PORT-STUB, and `test/css-modules/styles.css` in validation mode where the
  TS throws (`parseTokens` reads `options.source.file` without a source, from `matchProperty`).
- `node scripts/parity/render.js ../css/dist <file> [beautify|minify]` vs `./target/release/render <file> [mode]`:
  parse (no ast minification) + render. All test css files identical in the 3 modes. Mode `astmin`: the ast is
  minified with the default features (shorthand, calc, transform): identical.
- `node scripts/parity/transform-map.js ../css/dist <file>` vs `./target/release/render <file> map`: beautified code +
  source map. Identical except the 4 files with an input source map file (`*.map`): the TS loads it asynchronously
  with `load()` and does not wait for it before rendering; the parity script has no `load()`.
- `cargo test --release --test memo`: identical declarations are validated once and rendered once.

## Internal data structures (performance)

The internals differ from the typescript where rust allows sharing instead of copying; the bindings and the shapes
exposed to javascript are unchanged.

- Strings: `SharedStr` (`ast/strings.rs`) is a shared `Rc<str>`, a static string, or a range of an ascii source buffer
  (the buffer is shared). `resolve_string()` and the arena getters return it: reading a string of the input copies
  nothing. The ascii invariant of `CodeUnits::Bytes` makes any range valid utf-8.
- `JsMap` keys are `SharedStr`: interned names and configuration names are stored without a copy.
- Scratch stacks (`utils/pool.rs`, `PooledVec`): the walkers and `visit_values()` take their stack from a thread local
  pool (the typescript shares `visitStack` for the same reason).
- Minifier caches: the split selectors (`arena.raw`) are shared (`Rc`), copied only when modified; the first parts of
  the selectors are cached by string id (`arena.first_parts`, invalidated by the string epoch).
- `StructuralMemo`: a key pool with open addressing (like the typescript): one hash per lookup, no allocation per key.
- Rendered sizes (minifier `diff`): `NodeRenderer::size()` computes the size of a node from the sizes of its parts
  (`node_size()`, the declarations measured from their memoized rendered values, the layout of `render_ast_node()`)
  instead of rendering it. Same in the typescript (`nodeSize()`). `CSS_PARSER_CHECK_SIZE=1` checks every computed size
  against a render (used with `scripts/parity/outputs.js`).
- Split selectors: `RawSelector` keeps the vendor prefixes of the selector (computed once, cleared by a mutable
  access); the prefixes of a selector string are cached by string id. `split_rule()` splits an ascii selector on its
  bytes (no utf-16 conversion).
- Configuration: `config_index()` indexes the `properties.json` groups in Fx hash maps; the derived settings of the map
  configurations and the token types of the shorthand configurations are computed once per configuration node.
- The syntax caches are keyed by group then by a borrowed name.
- Kept as in the typescript because the bindings expose them: the column storage of the arena (one typed array per
  column, `arenaColumns`) and the linked token lists (`firstToken` / `nextSibling`).

## Working notes (keep updated)

- Validation tokens: `src/validation/parser/types.rs` (`ValidationToken`, `Rc` shared, immutable once parsed).
  `parse_syntax` builds by index then freezes (identity semantics of the TS `Object.assign` / `lastIndexOf`).
- Identity-keyed TS `WeakMap` caches → pointer-keyed thread-local maps that keep the key alive (see `first.rs`).
- `match.ts` → `src/validation/match_.rs` + `match_syntax.rs` + `match_property.rs`. `ValidationContext` = shared `Rc<[u32]>` tokens + start
  offset + index (`slice()` does not copy). `ValidationMatch` = struct {success, valid, token, context, syntax_token,
  errors}.
- TS sources can be executed for parity with `../css/node_modules/.bin/tsx` (the build tree-shakes internals).
- Host bridge: `src/host.rs` (`Host` trait = javascript callbacks, `HostValue`, `HostDef` = visitor maps/options with
  functions replaced by callback ids, `Task`/`HostPromise` executor, `block_on` for the sync api).
- `doParse` + `doParseSync` → one `async fn do_parse()`: the async TS version is the superset; the sync-only
  behaviours of the TS (throw on css variable import / composes from file, `generateSyncScopedName`) are taken when
  `options.sync_api_compat` (bindings only). Visitor results are `HostValue`s (numbers = node ids of the arena, `ForeignNode` = view of another arena,
  `Plain` = plain objects).
- `hash.ts`: sha digests are native (sha1/sha2 crates) and synchronous.
- Colors: number arrays are `Vec<f64>`; the optional alpha argument `values[3]` is `opt()` (`None` = `undefined`),
  `at()` reads `NaN` for a missing component (`syntax/color/utils/values.rs`, no TS counterpart). color-mix keeps
  `Vec<Option<f64>>` (holes / `undefined` alpha). `Math.max/min` are `js_max/js_min` (NaN propagation).
- Minify: features are `Rc<RefCell<dyn MinifyFeature>>` registered per `minify()` call (`features/mod.rs`), the TS
  `context` object is `MinifyContext` (feature state in `data`, the render size cache `options.cache` in `cache`).
  `arena.optimized` values are `Rc<RefCell<OptimizedSelector>>` (shared/mutated in place like the TS objects).
- Compute once (user requirement): identical declaration values are validated once (validation cache in
  `match_.rs`, counter `declaration_validations()`), identical declarations / constructs are rendered once per
  render (`RenderMemo`, valid for the options identity like `cache.options === options`: `RenderCache.options` is a
  pointer; counter `declaration_renders()`); a declaration repeated in the same buffer is a byte copy
  (`Writer::copy`); memo hits share `Rc<str>` (`Rendered`), no copy. Source map offsets are computed from the
  rendered string in place (`move_location`). Borrow slices of `options.source.content`, never allocate needlessly.
- Declarations (`parser/declaration`): `JsMap` = insertion ordered `Map`; `DeclEntry` = node | `Rc<RefCell<PropertySet>>`
  | `Rc<RefCell<PropertyMap>>` (shared like the TS objects); the iterators are eager (`iterate()`), each call runs the
  shorthand computation again like a new TS iterator. TS quirks kept: the leaked `property` loop variable in
  `PropertyMap` iteration, the live-array iteration of `removeDefaults`, the ungrouped `(^|\|)a|b|c(\||$)` regex
  of `mapValues`.
- Transform: matrices are `[f64; 16]`; `toPrecision(6)` is `js_to_precision` (exact decimal, half up like JS);
  `Math.hypot` follows the v8 algorithm (`js_hypot`); the TS `inverse()` indexing quirk is kept. The trig functions use
  the platform libm (v8 uses fdlibm): a last-bit difference is absorbed by the 6 digit rounding.
- Mixins: the scopes are `Rc<dyn VariableScope>`, the variables `Rc<RefCell<VariableEntry>>` (`text:
  Option<Option<String>>` = undefined / null / value); a variable refers to its scope with a `Weak` (no cycle). The
  `FunctionRegistry` is shared (`Rc`, interior mutability). `AstArena::shared()` / `rc()`: a feature that receives
  `&AstArena` parses into the same arena (`parseDeclarations(css, arena)`).
- Optional features parity: `PARSE_FLAGS=inlineCssVariables,removePrefix,expandIfSyntax,evaluateMixins` for both
  parity scripts. All test css files identical; the 120 mixin / custom function spec inputs identical.
- `matchSelectorSyntax` returns the caller's `errors` array: the port returns a copy (never `mem::take` a caller's
  error list).
- Flag enums (`ModuleCaseTransformEnum`, `ModuleScopeEnumOptions`, `WalkerEvent`, `WalkerOptionEnum`) are tested with
  bitwise operators, in the TS sources too: `(naming & IgnoreCase) == 0`, `getKeyName()` tests the case flags, a
  visitor `{type: Enter | Leave}` is registered for both events.
- Sync / async: one `do_parse()` with every feature; `do_parse_sync()` runs it to completion. The restrictions of the
  TS `parseSync()` / `transformSync()` (no `@import` resolution, errors on css variable imports and `composes` from a
  file, `generateSyncScopedName()`, visitor results not awaited, the module `mapping` reset before it is renamed) are
  applied by the bindings only (`ParserOptions::sync_api_compat`), for parity.
