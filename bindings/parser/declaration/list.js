// PropertyList of src/parser/declaration/list.ts: the declarations of a rule, the duplicates removed and the
// shorthands computed. the declarations are processed by rust when the list is iterated.

import { runtimeOf } from "../../core.js";
import "../../ast/arena.js";

export class PropertyList {
    #arena;
    #options;
    #declarations = [];

    constructor(arena, options = {}) {
        this.#arena = arena;
        this.#options = options;
    }

    addDeclaration(declaration) {
        this.#declarations.push(declaration);
        return this;
    }

    [Symbol.iterator]() {
        return runtimeOf(this.#arena).call("propertyList", this.#arena, this.#options, this.#declarations)[Symbol.iterator]();
    }
}
