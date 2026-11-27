// Entry points shared by node.js and web.js (port of src/node.ts / src/web.ts and src/utils/sync.ts): the options
// are completed like the typescript entry points, the work is done by rust.

import { readAll, rt as defaultRuntime, runtimeOf } from "./core.js";
import { getStreamSize } from "./runtime.js";
import "./ast/arena.js";
import { ModuleScopeEnumOptions } from "./ast/types.js";
import { ResponseType } from "./enums-extra.js";
import { RenderResult, TransformResult } from "./printer/result.js";
import { renderResult } from "./printer/render.js";

export { readAll };

/**
 * the inputs accepted as css: a string or utf-8 bytes
 */
export function isCssInput(value) {
    return typeof value === "string" || value instanceof ArrayBuffer || ArrayBuffer.isView(value);
}

export function validateSyncArguments(options, prefix = "options.") {
    for (const [key, value] of Object.entries(options)) {
        if (typeof value === "function") {
            if (value instanceof Promise || Object.getPrototypeOf(value).constructor.name == "AsyncFunction") {
                throw new Error(`[${prefix + key}]: Async functions are not supported in sync mode. Use parse() or transform() instead.`);
            }
        } else if (
            value != null &&
            typeof value === "object" &&
            (Array.isArray(value) || Object.getPrototypeOf(value) === Object.prototype || Object.getPrototypeOf(value) === null)
        ) {
            validateSyncArguments(value, prefix + key + ".");
        }
    }
}

function isIcss(options) {
    return (
        (typeof options.module === "number" && options.module & ModuleScopeEnumOptions.ICSS) ||
        (typeof options.module === "object" && options.module != null && typeof options.module.scoped === "number" && options.module.scoped & ModuleScopeEnumOptions.ICSS)
    );
}

function transformResult(result) {
    return new TransformResult(result.parse, renderResult(result.render), result.stats);
}

function transformDefaults(options) {
    options = { ...options };
    // the fast mode sets the default value of minify (see fastModeOptions)
    if (!options.fastMode) {
        options.minify ??= true;
    }
    options.removeEmpty ??= true;
    options.removeCharset ??= true;
    return options;
}

/**
 * the arguments `(input, options)` or `({ input, ...options })`
 */
function splitArguments(args, stream) {
    if (isCssInput(args[0]) || (stream && typeof ReadableStream !== "undefined" && args[0] instanceof ReadableStream)) {
        return [args[0], args[1], null];
    }

    const { file, input, ...options } = args[0] ?? {};
    return [input, options, file];
}

/**
 * the arguments of an async parse: a stream is read by rust while the css is parsed, chunk by chunk. the `stream`
 * option is the reader of the chunks: it returns the next chunk (bytes or text), `null` at the end of the stream
 * @param input the css: a string, an ArrayBuffer, a view of an ArrayBuffer or a stream
 * @param options
 */
function streamArguments(input, options) {
    if (typeof ReadableStream === "undefined" || !(input instanceof ReadableStream)) {
        return [input, options];
    }

    const reader = input.getReader();

    return [
        null,
        {
            ...options,
            // the size of the css when it is known: rust allocates the code units and the arena once
            streamSize: getStreamSize(input),
            stream: async function stream() {
                const { done, value } = await reader.read();

                if (done) {
                    reader.releaseLock();
                    return null;
                }

                return value;
            },
        },
    ];
}

/**
 * @param host `{ load, cwd(), runtime }`: the default loader, the current directory of the platform and the runtime of
 * the backend (the default runtime when it is not set)
 */
export function createApi(host) {
    const { load } = host;
    const rt = host.runtime ?? defaultRuntime;

    function render(arena, data, options = {}, mapping) {
        return renderResult(runtimeOf(arena).call("render", arena, data, { ...options, cwd: options.cwd ?? host.cwd() }, mapping ?? null, false));
    }

    function parseSync(...args) {
        const [stream, options] = splitArguments(args, false);

        options?.signal?.throwIfAborted();

        if (options != null) {
            validateSyncArguments(options);
        }

        return rt.call("parseSync", stream, { ...options, cwd: options?.cwd ?? host.cwd() });
    }

    function transformSync(...args) {
        const [stream, opts] = splitArguments(args, false);

        opts?.signal?.throwIfAborted();

        if (opts != null) {
            validateSyncArguments(opts);
        }

        const options = transformDefaults(opts);

        options.cwd ??= host.cwd();
        return transformResult(rt.call("transformSync", stream, options));
    }

    function loadFile(file, options) {
        return Promise.resolve(
            options.load != null
                ? options.load(file, "", options.asStream ?? false)
                : load(file, "", options.asStream ? ResponseType.ReadableStream : ResponseType.ArrayBuffer),
        );
    }

    async function parse(...args) {
        const [stream, options = {}, file] = splitArguments(args, true);

        options.signal?.throwIfAborted();

        if (file != null) {
            return loadFile(file, options).then((stream) => parse(stream, { src: file, ...options }));
        }

        return rt.run(
            rt.call("parse", ...streamArguments(stream, { ...options, load: options.load ?? load, cwd: options.cwd ?? host.cwd() })),
        );
    }

    async function transform(...args) {
        const [stream, opts = {}, file] = splitArguments(args, true);

        opts.signal?.throwIfAborted();

        if (file != null) {
            return loadFile(file, opts).then((stream) => transform(stream, { src: file, ...opts }));
        }

        const options = transformDefaults(opts);

        options.load ??= load;
        options.cwd ??= host.cwd();

        return transformResult(await rt.run(rt.call("transform", ...streamArguments(stream, options))));
    }

    return { render, parse, parseSync, transform, transformSync };
}

export { isIcss, RenderResult, TransformResult };
