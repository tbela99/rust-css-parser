import { rt } from "../core.js";

/**
 * reference of a token list of a parsed syntax (a hidden property of the arrays)
 */
export const SYNTAX = Symbol("syntax");

const syntaxes = new Map();

/**
 * tag the token lists with their path in the syntax
 */
function tag(list, handle, path) {
    Object.defineProperty(list, SYNTAX, { value: { handle, path } });

    for (let i = 0; i < list.length; i++) {
        const token = list[i];

        for (const field of ["chi", "l", "r", "prelude"]) {
            const value = token?.[field];

            if (!Array.isArray(value)) {
                continue;
            }

            const fieldPath = [...path, i, field];

            if (value.length > 0 && Array.isArray(value[0])) {
                Object.defineProperty(value, SYNTAX, { value: { handle, path: fieldPath } });
                value.forEach((alternative, j) => tag(alternative, handle, [...fieldPath, j]));
            } else {
                tag(value, handle, fieldPath);
            }
        }
    }

    return list;
}

/**
 * the parsed syntax of a property, a function, ...: the same object is returned for the same syntax
 */
export function getParsedSyntax(group, key) {
    const result = rt.call("getParsedSyntax", group, key);

    if (result === null) {
        return null;
    }

    let tokens = syntaxes.get(result.handle);

    if (tokens === undefined) {
        tokens = tag(result.tokens, result.handle, []);
        syntaxes.set(result.handle, tokens);
    }

    return tokens;
}
