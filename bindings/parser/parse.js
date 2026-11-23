import { runtimeOf } from "../core.js";
import "../ast/arena.js";

export function parseDeclarations(declaration, arena) {
    return runtimeOf(arena).call("parseDeclarations", declaration, arena);
}

export function parseString(arena, src, options = { parseColor: true }, errors) {
    return runtimeOf(arena).call("parseString", arena, src, options);
}
