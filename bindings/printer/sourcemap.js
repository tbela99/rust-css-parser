// Source map of a render result (src/printer/sourcemap.ts). The mappings are encoded by rust; find()
// decodes them in rust.

import { rt } from "../core.js";

export class SourceMap {
    #json;

    constructor(sourcemaps) {
        if (sourcemaps == null) {
            this.#json = { version: 3, sources: [], names: [], mappings: "" };
        } else {
            this.#json = typeof sourcemaps === "string" ? JSON.parse(sourcemaps) : { ...sourcemaps };
        }
    }

    computePositions() {}

    find(line, column) {
        return rt.call("sourceMapFind", JSON.stringify(this.#json), line, column);
    }

    toUrl() {
        return rt.call("sourceMapUrl", JSON.stringify(this.#json));
    }

    toJSON() {
        return this.#json;
    }
}
