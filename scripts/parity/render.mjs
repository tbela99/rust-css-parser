// render a css file (parity check with src/bin/render.rs): parse without minification, render with the options
// usage: node scripts/parity/render.mjs <dist> <file> [beautify|minify]
import { readFileSync } from "node:fs";
const [dist, file, mode] = process.argv.slice(2);
const { parseSync, render } = await import(`${dist}/node.js`);
// astmin: the ast is minified (default features)
const parseOptions = mode === "astmin" ? { minify: true } : { minify: false };
// PARSE_FLAGS: comma separated parser options set to true (inlineCssVariables, removePrefix, ...)
const flags = Object.fromEntries((process.env.PARSE_FLAGS ?? "").split(",").filter(Boolean).map((f) => [f, true]));
const result = parseSync(readFileSync(file), { ...parseOptions, ...flags, src: "", cwd: "" });
const renderOptions =
    mode === "beautify" ? { minify: false, beautify: true } : mode === "map" ? { minify: false, beautify: true, sourcemap: true } : { minify: mode === "minify" || mode === "astmin" };
const out = render(result.arena, result.ast, renderOptions);
console.log(out.code);
if (mode === "map") console.log(JSON.stringify(out.map.toJSON()));
