import { runtimeOf } from "../../../core.js";
import "../../../ast/arena.js";

export function okLabDistance(arena, color1, color2) {
    return runtimeOf(arena).call("okLabDistance", arena, color1, color2);
}

export function isOkLabClose(arena, color1, color2, threshold) {
    return runtimeOf(arena).call("isOkLabClose", arena, color1, color2, threshold);
}
