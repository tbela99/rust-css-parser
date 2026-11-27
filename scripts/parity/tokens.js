// dump the tokens of css files with the typescript tokenizer (parity check with src/bin/tokens.rs)
// usage: node scripts/parity/tokens.js <typescript dist dir> files...
import { readFileSync } from "node:fs";
const [dist, ...files] = process.argv.slice(2);
const { Tokenizer, createParseInfo } = await import(`${dist}/parser/tokenize.js`);
const { SourceFile } = await import(`${dist}/parser/source.js`);
const { AstArena } = await import(`${dist}/ast/arena.js`);
let out = "";
for (const file of files) {
    const bytes = readFileSync(file);
    const arena = new AstArena();
    const source = new SourceFile("", [], "");
    const tokenizer = new Tokenizer(createParseInfo(new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength), source));
    for (;;) {
        tokenizer.next();
        const typ = tokenizer.typ ?? 0;
        const value = [2, 3, 4].includes(tokenizer.field) ? arena.resolveString(tokenizer.internValue(arena, -1)) : "";
        const num = [1, 4].includes(tokenizer.field) || typ === 14 ? String(tokenizer.num) : "";
        out += `${typ}\t${tokenizer.sta ?? 0}\t${tokenizer.end ?? 0}\t${tokenizer.field}\t${num}\t${JSON.stringify(value)}\n`;
        if (tokenizer.done()) break;
    }
    out += `lines\t${source.lineStarts.lineStarts.length}\n`;
}
process.stdout.write(out);
