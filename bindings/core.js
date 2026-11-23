// Loads the native code of the extension: the N-API addon (package css-parser-napi) or the WASM module (package
// css-parser-wasm). Each package provides its own `backend.js` next to this file.

import { Runtime, readAll } from "./runtime.js";
import { backendName, createBackend } from "./backend.js";
import { columnTypes } from "./columns.js";

export { readAll, backendName, columnTypes };

/**
 * the classes created from the values (set by ast/arena.js), shared by the runtimes
 */
export const classes = {};

const runtimes = new Map();

/**
 * the runtime of the backend of the package ("napi" or "wasm"), created once
 */
export function getRuntime(name = backendName) {
    if (name !== backendName) {
        throw new Error(`the ${name} backend is not available: this package provides the ${backendName} backend`);
    }

    let runtime = runtimes.get(name);

    if (runtime === undefined) {
        runtime = createBackend().then((backend) => {
            const result = new Runtime(backend, classes);

            result.name = name;
            return result;
        });
        runtimes.set(name, runtime);
    }

    return runtime;
}

/**
 * the runtime of the package backend
 */
export const rt = await getRuntime(backendName);

/**
 * the runtime of an arena, or of an arena node: the default runtime otherwise
 */
export function runtimeOf(value) {
    return (value != null && classes.runtimeOf?.(value)) || rt;
}
