// Runtime shared by the N-API and the WASM backends: the value encoding (see src/bindings/value.rs), the calls to
// rust, the javascript callbacks called by rust and the async calls driven by javascript.
//
// A backend provides:
// - exchange(capacity): Uint8Array view of the exchange buffer, at least `capacity` bytes
// - invoke(length): run the call encoded in the exchange buffer, return the result length
// - column(handle, index, Type): typed array view of an arena column
// - generation(handle): Uint32Array view of the generation counter of an arena
// - setHost(fn): the function rust calls to run a callback: (length) => responseLength

const encoder = new TextEncoder();
const decoder = new TextDecoder();

const ARENAID = Symbol.for("@tbela99/css-parser/ARENAID");
const ARENA = Symbol.for("@tbela99/css-parser/ARENA");

const TAG = {
    undefined: 0,
    null: 1,
    false: 2,
    true: 3,
    number: 4,
    string: 5,
    bytes: 6,
    array: 7,
    object: 8,
    function: 9,
    arena: 10,
    node: 11,
    promise: 12,
    error: 13,
    output: 14,
};

/**
 * growable output buffer of the encoder
 */
class Output {
    bytes = new Uint8Array(1 << 12);
    view = new DataView(this.bytes.buffer);
    length = 0;

    reserve(size) {
        if (this.length + size > this.bytes.length) {
            let capacity = this.bytes.length * 2;

            while (capacity < this.length + size) {
                capacity *= 2;
            }

            const bytes = new Uint8Array(capacity);

            bytes.set(this.bytes.subarray(0, this.length));
            this.bytes = bytes;
            this.view = new DataView(bytes.buffer);
        }
    }

    u8(v) {
        this.reserve(1);
        this.bytes[this.length++] = v;
    }

    u32(v) {
        this.reserve(4);
        this.view.setUint32(this.length, v, true);
        this.length += 4;
    }

    f64(v) {
        this.reserve(8);
        this.view.setFloat64(this.length, v, true);
        this.length += 8;
    }

    string(s) {
        // at most 3 bytes per utf-16 code unit
        this.reserve(4 + s.length * 3);
        const { written } = encoder.encodeInto(s, this.bytes.subarray(this.length + 4));
        this.view.setUint32(this.length, written, true);
        this.length += 4 + written;
    }

    raw(bytes) {
        this.u32(bytes.byteLength);
        this.reserve(bytes.byteLength);
        this.bytes.set(bytes, this.length);
        this.length += bytes.byteLength;
    }
}

/**
 * the content of a stream: the resolved values of the callbacks (load()) are read before they are sent to rust
 */
export async function readAll(value) {
    if (typeof ReadableStream === "undefined" || !(value instanceof ReadableStream)) {
        return value;
    }

    const chunks = [];
    let length = 0;
    let text = false;

    for await (const chunk of value) {
        if (typeof chunk === "string") {
            text = true;
        }

        chunks.push(chunk);
        length += chunk.length;
    }

    if (text) {
        return chunks.map((chunk) => (typeof chunk === "string" ? chunk : decoder.decode(chunk))).join("");
    }

    const result = new Uint8Array(length);
    let offset = 0;

    for (const chunk of chunks) {
        result.set(chunk, offset);
        offset += chunk.length;
    }

    return result;
}

export class Runtime {
    /**
     * the arena wrappers by handle: an arena returned twice is the same object
     */
    arenas = new Map();
    /**
     * the callbacks passed to rust, by id
     */
    callbacks = new Map();
    callbackIds = new WeakMap();
    nextCallback = 1;
    /**
     * the promises returned by the callbacks, by id
     */
    promises = new Map();
    nextPromise = 1;
    output = new Output();
    depth = 0;
    touched = null;

    /**
     * @param backend
     * @param classes the classes created from the values: { wrapArena(handle), AstArenaNode, isArena(value), handleOf(arena) }
     */
    constructor(backend, classes) {
        this.backend = backend;
        this.classes = classes;
        backend.setHost((length) => this.host(length));
    }

    touch(arena) {
        if (this.touched != null && this.classes.isExported(arena) && !this.touched.includes(arena)) {
            this.touched.push(arena);
        }
    }

    encode(value, out) {
        switch (typeof value) {
            case "undefined":
                out.u8(TAG.undefined);
                return;
            case "boolean":
                out.u8(value ? TAG.true : TAG.false);
                return;
            case "number":
                out.u8(TAG.number);
                out.f64(value);
                return;
            case "bigint":
                out.u8(TAG.number);
                out.f64(Number(value));
                return;
            case "string":
                out.u8(TAG.string);
                out.string(value);
                return;
            case "function": {
                let id = this.callbackIds.get(value);

                if (id === undefined) {
                    id = this.nextCallback++;
                    this.callbackIds.set(value, id);
                    this.callbacks.set(id, value);
                }

                out.u8(TAG.function);
                out.u32(id);
                out.string(value.name ?? "");
                return;
            }
            case "symbol":
                out.u8(TAG.undefined);
                return;
        }

        if (value === null) {
            out.u8(TAG.null);
            return;
        }

        if (value instanceof Uint8Array) {
            out.u8(TAG.bytes);
            out.raw(value);
            return;
        }

        if (value instanceof ArrayBuffer) {
            out.u8(TAG.bytes);
            out.raw(new Uint8Array(value));
            return;
        }

        if (ArrayBuffer.isView(value)) {
            out.u8(TAG.bytes);
            out.raw(new Uint8Array(value.buffer, value.byteOffset, value.byteLength));
            return;
        }

        if (Array.isArray(value)) {
            out.u8(TAG.array);
            out.u32(value.length);

            for (const item of value) {
                this.encode(item, out);
            }

            return;
        }

        if (this.classes.isArena(value)) {
            this.touch(value);
            out.u8(TAG.arena);
            out.u32(this.classes.handleOf(value, this));
            return;
        }

        if (value instanceof this.classes.AstArenaNode) {
            this.touch(value[ARENA]);
            out.u8(TAG.node);
            out.u32(this.classes.handleOf(value[ARENA], this));
            out.u32(value[ARENAID]);
            return;
        }

        if (typeof value.then === "function") {
            const id = this.nextPromise++;

            this.promises.set(id, value);
            out.u8(TAG.promise);
            out.u32(id);
            return;
        }

        if (value instanceof Map || value instanceof Set || value instanceof WeakMap) {
            out.u8(TAG.undefined);
            return;
        }

        // a class instance that knows its encoding (SourceMap, ...)
        if (typeof value.toJSON === "function" && Object.getPrototypeOf(value) !== Object.prototype) {
            this.encode(value.toJSON(), out);
            return;
        }

        const keys = Object.keys(value);

        out.u8(TAG.object);
        out.u32(keys.length);

        for (const key of keys) {
            out.string(key);
            this.encode(value[key], out);
        }
    }

    decode(bytes, state) {
        const view = state.view;
        const tag = bytes[state.pos++];

        const u32 = () => {
            const v = view.getUint32(state.pos, true);
            state.pos += 4;
            return v;
        };

        const string = () => {
            const length = u32();
            const s = decoder.decode(bytes.subarray(state.pos, state.pos + length));
            state.pos += length;
            return s;
        };

        switch (tag) {
            case TAG.undefined:
                return undefined;
            case TAG.null:
                return null;
            case TAG.false:
                return false;
            case TAG.true:
                return true;
            case TAG.number: {
                const v = view.getFloat64(state.pos, true);
                state.pos += 8;
                return v;
            }
            case TAG.string:
                return string();
            case TAG.bytes: {
                const length = u32();
                // a copy: the exchange buffer is reused by the next call
                const result = bytes.slice(state.pos, state.pos + length);
                state.pos += length;
                return result;
            }
            case TAG.array: {
                const n = u32();
                const result = new Array(n);

                for (let i = 0; i < n; i++) {
                    result[i] = this.decode(bytes, state);
                }

                return result;
            }
            case TAG.object: {
                const n = u32();
                const result = {};

                for (let i = 0; i < n; i++) {
                    const key = string();
                    result[key] = this.decode(bytes, state);
                }

                return result;
            }
            case TAG.function: {
                const id = u32();
                string();
                return this.callbacks.get(id);
            }
            case TAG.arena:
                return this.classes.wrapArena(u32(), this);
            case TAG.node: {
                const arena = this.classes.wrapArena(u32(), this);
                return new this.classes.AstArenaNode(arena, u32());
            }
            case TAG.promise:
                return this.promises.get(u32());
            case TAG.output: {
                const id = u32();
                const length = u32();
                return new Uint8Array(this.backend.output(id), 0, length);
            }
            case TAG.error: {
                const error = new Error(string());

                error.fromRust = true;
                return error;
            }
        }

        throw new Error(`invalid value tag ${tag}`);
    }

    /**
     * write a value in the exchange buffer. returns the length
     */
    write(value) {
        const out = this.output;

        out.length = 0;
        this.encode(value, out);

        const target = this.backend.exchange(out.length);

        target.set(out.bytes.subarray(0, out.length));
        return out.length;
    }

    read(length) {
        const bytes = this.backend.exchange(length);
        return this.decode(bytes, { pos: 0, view: new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength) });
    }

    /**
     * call a rust function
     */
    call(method, ...args) {
        // the arenas whose columns are javascript buffers (transferred arenas) and that are arguments of the outermost
        // call are synchronized with rust around it
        const outer = this.depth === 0;
        const touched = outer ? (this.touched = []) : null;
        const out = this.output;

        out.length = 0;

        try {
            this.encode([method, ...args], out);
        } finally {
            this.touched = null;
        }

        let request = out.bytes.subarray(0, out.length);

        if (touched !== null && touched.length > 0) {
            // the synchronization calls reuse the output buffer
            request = request.slice();
            this.depth++;

            try {
                this.classes.beforeCall(touched);
            } finally {
                this.depth--;
            }
        }

        this.backend.exchange(request.length).set(request);
        this.depth++;

        let result;

        try {
            result = this.read(this.backend.invoke(request.length));
        } finally {
            this.depth--;

            if (touched !== null && touched.length > 0) {
                this.classes.afterCall(touched);
            }
        }

        if (result instanceof Error && result.fromRust) {
            delete result.fromRust;
            throw result;
        }

        return result;
    }

    /**
     * run a callback for rust: the request is `[callback, arena, ...args]`
     */
    host(length) {
        let result;

        try {
            const [id, arena, ...args] = this.read(length);
            const callback = this.callbacks.get(id);

            result = arena === undefined ? callback(...args) : callback(arena, ...args);
        } catch (error) {
            return this.writeError(error);
        }

        return this.write(result);
    }

    writeError(error) {
        const out = this.output;

        out.length = 0;
        out.u8(TAG.error);
        out.string(error?.message ?? String(error));

        const target = this.backend.exchange(out.length);

        target.set(out.bytes.subarray(0, out.length));
        return out.length;
    }

    /**
     * drive an async rust call (parse(), transform()) until it completes
     */
    async run(task) {
        for (;;) {
            const state = this.call("poll", task);

            if (state.done) {
                return state.value;
            }

            await Promise.all(
                state.pending.map(async (id) => {
                    const promise = this.promises.get(id);

                    this.promises.delete(id);

                    try {
                        this.call("settle", id, true, await readAll(await promise));
                    } catch (error) {
                        this.call("settle", id, false, error?.message ?? String(error));
                    }
                }),
            );
        }
    }
}
