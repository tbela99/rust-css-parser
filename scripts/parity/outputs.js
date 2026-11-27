// byte-for-byte comparison of the rust bindings with the typescript build:
// `node scripts/parity/outputs.js <typescript dist directory> [rust dist directory]`
import { readFileSync, readdirSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const tsDist = resolve(process.argv[2] ?? "../css/dist");
const rsDist = resolve(process.argv[3] ?? "extensions/css-parser-napi/dist");
const ts = await import(pathToFileURL(join(tsDist, "node.js")).href);
const rs = await import(pathToFileURL(join(rsDist, "node.js")).href);
const root = resolve(tsDist, "..");
const dirs = [join(root, "benchmark/fixtures"), join(root, "test/files/css")];
const files = dirs.flatMap((dir) => readdirSync(dir).filter((f) => f.endsWith(".css")).map((f) => join(dir, f)));
const optionSets = {
    default: {},
    fast: { minify: false, removeEmpty: true, removeComments: true, convertColor: true },
    beautify: { beautify: true, minify: false },
    validation: { validation: true },
    sourcemap: { sourcemap: true, beautify: true },
};
let same = 0;
const diffs = [];
for (const file of files) {
    const css = readFileSync(file, "utf8");
    for (const [name, options] of Object.entries(optionSets)) {
        const o = { ...options, src: file.slice(root.length + 1) };
        const a = ts.transformSync(css, o);
        const b = rs.transformSync(css, o);
        const ok =
            a.code === b.code &&
            JSON.stringify(a.errors.map((e) => [e.message, e.location])) === JSON.stringify(b.errors.map((e) => [e.message, e.location])) &&
            JSON.stringify(a.map?.toJSON() ?? null) === JSON.stringify(b.map?.toJSON() ?? null);
        if (ok) same++;
        else diffs.push(`${file.slice(root.length + 1)} [${name}] code ${a.code.length}/${b.code.length}${a.code === b.code ? "" : " DIFF"}`);
    }
}
console.log(`${same} identical, ${diffs.length} different`);
for (const d of diffs.slice(0, 20)) console.log("  " + d);
