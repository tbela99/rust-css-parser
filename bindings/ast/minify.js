import { runtimeOf } from "../core.js";
import "./arena.js";

export function minify(arena, ast, options = {}, recursive = false, errors, nestingContent) {
    return runtimeOf(arena).call("minify", arena, ast, options, recursive, errors, nestingContent);
}
