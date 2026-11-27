/* @ts-self-types="./index.d.ts" */
// node entry point (port of src/node.ts)
import { Readable } from "node:stream";
import { createReadStream } from "node:fs";
import { lstat, readFile } from "node:fs/promises";
import { cwd } from "node:process";
import { createApi } from "./api.js";
import { matchUrl, resolve } from "./fs/resolve.js";
import { ResponseType } from "./enums-extra.js";
import { responseSize, setStreamSize } from "./runtime.js";

export * from "./exports.js";

export async function load(url, currentDirectory = ".", responseType = false) {
    const resolved = typeof url === "string" ? resolve(url, currentDirectory) : url;

    if (typeof responseType === "boolean") {
        responseType = responseType ? ResponseType.ReadableStream : ResponseType.Text;
    }

    if (matchUrl.test(resolved.absolute)) {
        return fetch(resolved.absolute).then(async (response) => {
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

    try {
        const stats = await lstat(resolved.absolute);

        if (stats.isFile()) {
            if (responseType === ResponseType.Text || responseType === ResponseType.JSON) {
                return readFile(resolved.absolute, "utf-8").then((buffer) => (responseType === ResponseType.JSON ? JSON.parse(buffer) : buffer));
            }

            if (responseType === ResponseType.ArrayBuffer) {
                return readFile(resolved.absolute).then((buffer) =>
                    buffer.byteOffset === 0 && buffer.byteLength === buffer.buffer.byteLength
                        ? buffer.buffer
                        : buffer.buffer.slice(buffer.byteOffset, buffer.byteOffset + buffer.byteLength),
                );
            }

            // the size of the file: rust allocates its buffers once
            return setStreamSize(Readable.toWeb(createReadStream(resolved.absolute, { encoding: "utf-8", highWaterMark: 64 * 1024 })), stats.size);
        }
    } catch (error) {}

    throw new Error(`File not found: '${resolved.absolute || url}'`);
}

export const { render, parse, parseSync, transform, transformSync } = createApi({ load, cwd: () => cwd() });
