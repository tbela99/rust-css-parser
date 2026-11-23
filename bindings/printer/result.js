// Port of src/printer/result.ts: the code is decoded from the utf-8 output when it is read.

const decoder = new TextDecoder();

export class RenderResult {
    write;
    errors;
    stats;
    map;
    #code = null;
    #codeLength;

    constructor(write, errors, stats, map, codeLength = -1) {
        this.write = write;
        this.errors = errors;
        this.stats = stats;
        this.map = map;
        this.#codeLength = codeLength;
    }

    static codeLength(result) {
        return result.#codeLength >= 0 ? result.#codeLength : result.code.length;
    }

    get code() {
        if (this.#code === null) {
            this.#code = decoder.decode(this.write);
        }

        return this.#code;
    }

    toJSON() {
        const { write, ...json } = this;
        json.code = this.code;
        return json;
    }
}

export class TransformResult extends RenderResult {
    ast;
    arena;

    constructor(parseResult, renderResult, stats) {
        super(renderResult.write, parseResult.errors.concat(renderResult.errors), stats, renderResult.map, RenderResult.codeLength(renderResult));
        this.ast = parseResult.ast;
        this.arena = parseResult.arena;

        if (parseResult.mapping !== undefined) {
            this.mapping = parseResult.mapping;
        }

        if (parseResult.cssModuleVariables !== undefined) {
            this.cssModuleVariables = parseResult.cssModuleVariables;
        }

        if (parseResult.importMapping !== undefined) {
            this.importMapping = parseResult.importMapping;
        }

        if (parseResult.revMapping !== undefined) {
            this.revMapping = parseResult.revMapping;
        }
    }
}

RenderResult.shape = new RenderResult(new Uint8Array(0), [], { total: "0ms" });
