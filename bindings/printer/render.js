import { runtimeOf } from "../core.js";
import "../ast/arena.js";
import { RenderResult } from "./result.js";
import { SourceMap } from "./sourcemap.js";

/**
 * render statistics of a render: the identical declarations and constructs are rendered once
 */
export class RenderMemo {
    declarations = { size: 0, hits: 0 };
    constructs = { size: 0, hits: 0 };
}

/**
 * a render result from the rust result
 */
export function renderResult(result) {
    return new RenderResult(result.write, result.errors, result.stats, result.map == null ? undefined : new SourceMap(result.map), result.codeLength);
}

export function doRender(arena, data, options = {}, mapping, memo) {
    const result = runtimeOf(arena).call("render", arena, data, options, mapping ?? null, memo != null);

    if (memo != null) {
        memo.declarations = result.memo.declarations;
        memo.constructs = result.memo.constructs;
    }

    return renderResult(result);
}

export function renderValue(arena, token, options = {}) {
    return runtimeOf(arena).call("renderValue", arena, token, options);
}
