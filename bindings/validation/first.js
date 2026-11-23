import { runtimeOf } from "../core.js";
import "../ast/arena.js";
import { SYNTAX } from "./config.js";

function reference(list) {
    const ref = list?.[SYNTAX];

    if (ref === undefined) {
        throw new TypeError("the syntax must be a value returned by getParsedSyntax()");
    }

    return ref;
}

export function matchingAlternatives(arena, alternatives, token) {
    return runtimeOf(arena).call("matchingAlternatives", arena, reference(alternatives), token);
}

export function mayStartWith(arena, syntaxes, token) {
    return runtimeOf(arena).call("mayStartWith", arena, reference(syntaxes), token);
}
