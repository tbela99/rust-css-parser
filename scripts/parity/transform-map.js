// transform a css file with a source map (parity check with `src/bin/render.rs <file> map`)
import { readFileSync } from "node:fs";
const [dist, file] = process.argv.slice(2);
const { transformSync } = await import(`${dist}/node.js`);
const out = transformSync(readFileSync(file, "utf8"), { minify: false, beautify: true, sourcemap: true, src: "", cwd: "" });
console.log(out.code);
console.log(JSON.stringify(out.map.toJSON ? out.map.toJSON() : out.map));
