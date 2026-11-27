/* @ts-self-types="./index.d.ts" */
// browser entry point (port of src/web.ts)
import { createApi } from "./api.js";
import { getRuntime } from "./core.js";
import { dirname, matchUrl, resolve } from "./fs/resolve.js";
import { ResponseType } from "./enums-extra.js";
import { responseSize, setStreamSize } from "./runtime.js";

export * from "./exports.js";
export { PropertyList } from "./parser/declaration/list.js";

export async function load(url, currentDirectory = ".", responseType = false) {
    if (typeof responseType === "boolean") {
        responseType = responseType ? ResponseType.ReadableStream : ResponseType.Text;
    }

    let t;

    // outside of a browser (node, workers without location), the urls are resolved against the current directory
    const origin = globalThis.location?.origin ?? "file://";

    if (typeof url === "object") {
        t = new URL(url.absolute, origin);
    } else if (matchUrl.test(url)) {
        t = new URL(url);
    } else if (currentDirectory != null && matchUrl.test(currentDirectory)) {
        t = new URL(url, currentDirectory);
    } else {
        t = new URL(resolve(url, currentDirectory).absolute, origin);
    }

    return fetch(t, t.origin !== origin ? { mode: "cors" } : {}).then(async (response) => {
        if (!response.ok) {
            throw new Error(`${response.status} ${response.statusText} ${response.url}`);
        }

        if (responseType === ResponseType.ArrayBuffer) {
            return response.arrayBuffer();
        }

        if (responseType === ResponseType.JSON) {
            return response.json();
        }

        // the size of the body: rust allocates its buffers once
        return responseType === ResponseType.ReadableStream ? setStreamSize(response.body, responseSize(response)) : response.text();
    });
}

function cwd() {
    const location = globalThis.location;

    // outside of a browser: the current directory of the process
    if (location == null) {
        return globalThis.process?.cwd?.() ?? "";
    }

    return location.pathname.endsWith("/") ? location.pathname : dirname(location.pathname);
}

// the web entry point always uses the WASM backend (in node too): its arenas are WASM arenas
export const { render, parse, parseSync, transform, transformSync } = createApi({ load, cwd, runtime: await getRuntime("wasm") });
