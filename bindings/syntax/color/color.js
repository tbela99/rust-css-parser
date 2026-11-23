import { runtimeOf } from "../../core.js";
import "../../ast/arena.js";

export function convertColor(arena, token, to) {
    return runtimeOf(arena).call("convertColor", arena, token, to);
}
