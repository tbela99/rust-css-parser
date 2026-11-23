// dump the plain object of the ast of a css file (parity check with src/bin/ast.rs)
import { readFileSync } from "node:fs";
const [dist, file, mode] = process.argv.slice(2);
const { parseSync } = await import(`${dist}/node.js`);
const result = parseSync(readFileSync(file), { minify: false, validation: mode === "validation", src: "", cwd: "" });
const plain = result.arena.toPlainObject(result.ast);
const strip = (v) => (Array.isArray(v) ? v.map(strip) : v && typeof v === "object" ? Object.fromEntries(Object.entries(v).filter(([k, x]) => x !== undefined).map(([k, x]) => [k, strip(x)])) : v);
console.log(JSON.stringify(strip(plain)));
for (const e of result.errors) console.log("error\t" + e.message);
