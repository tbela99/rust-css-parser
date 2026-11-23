// N-API backend of the runtime (see runtime.js): the native addon css_parser.node

import { columnTypes } from "./columns.js";

/**
 * the backend of this package
 */
export const backendName = "napi";

/**
 * the N-API backend: the exchange buffer and the columns are external array buffers over the rust memory
 */
export async function createBackend() {
    const { createRequire } = await import("node:module");
    let native;

    try {
        native = createRequire(import.meta.url)("./css_parser.node");
    } catch (error) {
        // the addon is built on install (install.js)
        throw new Error(
            "@tbela99/rust-css-parser-napi: the N-API addon (css_parser.node) could not be loaded. It is built by the " +
                "install script of the package, which requires the Rust toolchain (https://rustup.rs). If npm blocked " +
                "the script: `npm install-scripts approve @tbela99/rust-css-parser-napi` then " +
                "`npm rebuild @tbela99/rust-css-parser-napi`; or run `node install.js` in the package directory",
            { cause: error },
        );
    }
    let buffer = new Uint8Array(native.buffer(0));

    const refresh = (capacity) => {
        buffer = new Uint8Array(native.buffer(capacity));
        return buffer;
    };

    return {
        exchange(capacity) {
            return capacity <= buffer.byteLength ? buffer : refresh(capacity);
        },
        invoke(length) {
            const result = native.call(length);

            if (result < 0) {
                refresh(0);
                return -result - 1;
            }

            return result;
        },
        setHost(host) {
            native.setHost((length, changed) => {
                if (changed) {
                    refresh(0);
                }

                return host(length);
            });
        },
        column(handle, index) {
            return new columnTypes[index](native.column(handle, index));
        },
        generation(handle) {
            return new Uint32Array(native.generation(handle));
        },
        output(id) {
            return native.output(id);
        },
        stale(view) {
            return view.buffer.byteLength === 0 && view.length > 0;
        },
    };
}
