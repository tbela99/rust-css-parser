// render every syntax of the validation data after parsing it (parity check with src/bin/syntaxes.rs)
// the typescript sources are used (renderSyntax is not exported by the build): run with tsx
// usage: <ts project>/node_modules/.bin/tsx scripts/parity/syntaxes.js <ts project>/src
import { readFileSync } from "node:fs";
const src = process.argv[2];
await import(`${src}/node.ts`);
const { parseSyntax, renderSyntax } = await import(`${src}/validation/parser/parse.ts`);
const config = JSON.parse(readFileSync(`${src}/data/validation.json`, "utf8"));
let out = "";
for (const group of ["atRules", "declarations", "mediaFeatures", "selectors", "syntaxes", "types"]) {
    for (const [name, node] of Object.entries(config[group])) {
        const syntaxes = [];
        if (typeof node.syntax === "string") syntaxes.push(["", node.syntax]);
        for (const [k, v] of Object.entries(node.descriptors ?? {})) {
            const s = typeof v === "string" ? v : v.syntax;
            if (typeof s === "string") syntaxes.push([k, s]);
        }
        for (const [key, syntax] of syntaxes) {
            let rendered;
            try {
                rendered = parseSyntax(syntax).reduce((acc, t) => acc + renderSyntax(t), "");
            } catch (e) {
                rendered = "ERROR " + e.message;
            }
            out += `${group}\t${name}\t${key}\t${rendered}\n`;
        }
    }
}
process.stdout.write(out);
