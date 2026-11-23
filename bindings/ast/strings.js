// StringBuilder of src/ast/strings.ts: a string built from code units and interned strings
// (see AstArena.appendString() / AstArena.internBuilder()).

export { StringFlag, StringSource } from "../enums-extra.js";

export class StringBuilder {
    #parts = [];

    reset() {
        this.#parts.length = 0;
        return this;
    }

    appendString(str) {
        this.#parts.push(str);
        return this;
    }

    appendCode(code) {
        this.#parts.push(String.fromCharCode(code));
        return this;
    }

    toString() {
        return this.#parts.join("");
    }
}
