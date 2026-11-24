// WASM backend of the runtime (see runtime.js): the WebAssembly module css_parser.wasm, in node and in the browsers

import { columnTypes } from "./columns.js";

const isNode = typeof process !== "undefined" && process.versions?.node != null && typeof window === "undefined";

/**
 * the backend of this package
 */
export const backendName = "wasm";

/**
 * the WASM backend: the exchange buffer and the columns are views of the linear memory
 */
export async function createBackend() {
    const url = new URL("./css_parser.wasm", import.meta.url);
    let bytes;

    if (isNode) {
        const { readFile } = await import("node:fs/promises");
        bytes = await readFile(url);
    } else {
        bytes = await fetch(url).then((response) => {
            if (!response.ok) {
                throw new Error(`could not load ${url}: ${response.status}`);
            }

            return response.arrayBuffer();
        });
    }

    let host = null;
    const { instance } = await WebAssembly.instantiate(bytes, {
        env: {
            css_now: () => performance.now(),
            css_host: (length) => host(length),
        },
    });
    const exports = instance.exports;
    const memory = exports.memory;
    // the pointers and the lengths are i32 values: the addresses above 2 GB of the linear memory (up to 4 GB) are
    // negative numbers, they are read as unsigned
    const u32 = (value) => value >>> 0;

    return {
        exchange(capacity) {
            const ptr = u32(exports.css_buffer(capacity));
            return new Uint8Array(memory.buffer, ptr, capacity);
        },
        invoke(length) {
            return exports.css_call(length);
        },
        setHost(fn) {
            host = fn;
        },
        column(handle, index) {
            const Type = columnTypes[index];
            const length = u32(exports.css_column_len(handle, index));

            if (length === 0) {
                return new Type(0);
            }

            return new Type(memory.buffer, u32(exports.css_column_ptr(handle, index)), length);
        },
        generation(handle) {
            return new Uint32Array(memory.buffer, u32(exports.css_generation_ptr(handle)), 1);
        },
        output(id) {
            // the linear memory cannot be given away: one copy
            const length = u32(exports.css_output_take(id));
            const buffer = new ArrayBuffer(length);

            new Uint8Array(buffer).set(new Uint8Array(memory.buffer, u32(exports.css_output_ptr()), length));
            exports.css_output_free();
            return buffer;
        },
        stale(view) {
            return view.buffer !== memory.buffer;
        },
    };
}
