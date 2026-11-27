# Changelog

The changes of the port. The changes of the parser itself are in the changelog of
[@tbela99/css-parser](https://github.com/tbela99/css-parser/blob/master/CHANGELOG.md): the port follows its versions.

# v2.3.2

## Performance

- [x] tokenizer: the numbers of 15 digits or less without exponent are read without the float parser (the digits divided by a power of ten: the same correctly rounded value)
- [x] `toFixed()`: the numbers that are not close to a tie are rounded without the exact decimal expansion (rendering of the angles)
- [x] input: the utf-16 code units of a non ascii input are written to a buffer allocated once
- [x] `ReadableStream` input: the chunks are read by rust while the css is parsed (a `stream` callback), instead of being read to the end by javascript first. the code units are appended to a buffer that doubles when it is full
- [x] `ReadableStream` input: the size of the stream when it is known (`streamSize`, recorded by the loader for a file or an uncompressed response, 0 when unknown): the code units and the arena are allocated once
- [x] arena growth: the columns are reallocated with zeroed memory (the free slots are not written: their pages are not resident), the previous buffers are freed when javascript has no view of the columns (they were kept until the arena was dropped)

# v2.3.1

- [x] fix deno install
- [x] document native and wasm builds

# v2.3.0

## Bugs

- [x] syntax: @webref/css 8.7.10. `<marker-ref>` (removed upstream, still referenced by `marker`, `marker-start`, `marker-mid`, `marker-end`) is defined by the local patch. an at-rule defined with several forms (`@property … { … } | @property … ;`) keeps the form with a block: `@property` rules were dropped. the generated syntax file is identical to the typescript one
- [x] wasm: the pointers and the lengths returned by the WebAssembly module are read as unsigned numbers: the memory above 2 GB was read at a negative offset (`RangeError: Start offset … is outside the bounds of the buffer`) when a large stylesheet was parsed
- [x] selectors: the error list of the parser was copied for each valid selector, the time was quadratic with the number of errors: test/files/css/larger.css (23 MB, 1,100 errors) was parsed in 3.8 s, it is parsed in 0.63 s (N-API)

## Performance

- [x] validation: the syntaxes of the declarations, the types and the functions are compiled at build time into a static table of nodes (`npm run syntax-compile`, `scripts/compile_syntax.rs`, `crates/css-parser/src/validation/compiled`) matched without allocating. a value accepted by a compiled matcher is valid, the other values are validated by the interpreter, which reports the errors. the values that contain `var()` or a custom function call are validated by the interpreter. the test `compiled_syntax` checks the compiled matchers against the interpreter
- [x] copies removed (allocations halved, 9.4 M to 4.6 M for tailwind, bootstrap 5 and larger.css):
  - the arguments of a function syntax are borrowed from the shared parsed syntax instead of being copied for each validation
  - the syntaxes of the functions and of the selector functions are looked up once per name: the names (`name()`, `@name`) are not formatted for each lookup
  - the validation contexts copy their tokens once
  - the selectors and the declarations use pooled buffers, the tokens of the nested lists are read in place before they are removed
  - the selectors moved by the minifier are not cloned
  - the identifiers, strings, literals, separators and numbers are written to the output when they are rendered: no string is created per token
  - the strings of a source that is not ascii are decoded into a reused buffer when they are read
  - the token lists that are only read are iterated in place instead of being copied

## Feature

- [x] syntax: `npm run syntax-update` compiles the syntaxes (`npm run syntax-compile`). a type that is referenced but neither defined nor built in fails the build: a definition renamed or removed upstream is reported (`crates/css-parser/src/data/syntax-report.json`)
- [x] tests: test/files/css/larger.css is parsed and minified (`tests/larger.rs`)
