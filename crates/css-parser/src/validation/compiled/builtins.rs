//! Port of `src/validation/compiled/builtins.ts`.

/// the types validated by the code of [`match_property`](crate::validation::match_property::match_property), before
/// their definition (if any): the compiled matchers call `match_property()` for them (see `Node::Leaf`). the other
/// types are compiled from their definition. keep in sync with `match_property()` and with its calc() types
pub const BUILTIN_TYPES: &[&str] = &[
    "calc-product",
    "declaration-value",
    "calc-sum",
    "an+b",
    "quote",
    "display-box",
    "display-inside",
    "display-internal",
    "display-outside",
    "display-legacy",
    "content-position",
    "mf-name",
    "dashed-ident",
    "extension-name",
    "custom-property-name",
    "namespace-prefix",
    "ident",
    "ident-token",
    "custom-ident",
    "counter-name",
    "counter-style-name",
    "any-value",
    "unicode-range-token",
    "hash-token",
    "string",
    "wq-name",
    "number",
    "zero",
    "integer",
    "dimension",
    "angle",
    "angle-percentage",
    "percentage",
    "length-percentage",
    "flex",
    "length",
    "resolution",
    "frequency",
    "decibel",
    "semitones",
    "time",
    "color",
    "color-function",
    "type-selector",
    "compound-selector",
    "complex-selector",
    "subclass-selector",
    "attribute-selector",
    "complex-selector-unit",
    "pseudo-compound-selector",
    "forgiving-relative-selector-list",
    "complex-selector-list",
    "function-token",
    "language-code",
    "size-feature",
    "style-feature",
    "url",
    "css-type",
    "dashed-function",
    "block-contents",
];

/// the built in types that read the whitespace or the tokens after the value being matched: the compiled matchers
/// skip the whitespace, the interpreter validates the values of these types
pub const RAW_TYPES: &[&str] = &[
    "type-selector",
    "compound-selector",
    "complex-selector",
    "subclass-selector",
    "attribute-selector",
    "complex-selector-unit",
    "pseudo-compound-selector",
    "forgiving-relative-selector-list",
    "complex-selector-list",
    "size-feature",
    "style-feature",
    "css-type",
];

/// true if the type is built in, see [`BUILTIN_TYPES`]
pub fn is_builtin_type(name: &str) -> bool {
    BUILTIN_TYPES.contains(&name)
}

/// true if the type reads the whitespace, see [`RAW_TYPES`]
pub fn is_raw_type(name: &str) -> bool {
    RAW_TYPES.contains(&name)
}
