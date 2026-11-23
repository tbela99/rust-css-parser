// Object view of an arena node (port of src/ast/arena-node.ts): the properties are read from and written to the
// rust arena when they are accessed.

import { EnumToken } from "./types.js";
import { runtimeOf } from "../core.js";

export const ARENAID = Symbol.for("@tbela99/css-parser/ARENAID");
export const ARENA = Symbol.for("@tbela99/css-parser/ARENA");

export function isNodeObjectType(typ) {
    switch (typ) {
        case EnumToken.StyleSheetNodeType:
        case EnumToken.RuleNodeType:
        case EnumToken.KeyframesRuleNodeType:
        case EnumToken.AtRuleNodeType:
        case EnumToken.KeyframesAtRuleNodeType:
        case EnumToken.CommentNodeType:
        case EnumToken.CDOCOMMNodeType:
        case EnumToken.DeclarationNodeType:
        case EnumToken.CssVariableTokenType:
        case EnumToken.CssVariableImportTokenType:
        case EnumToken.CssVariableDeclarationMapTokenType:
            return true;
    }

    return false;
}

function frozen(value) {
    return Array.isArray(value) ? Object.freeze(value) : value;
}

function get(node, key) {
    return frozen(runtimeOf(node[ARENA]).call("a.nodeGet", node[ARENA], node[ARENAID], key));
}

function set(node, key, value) {
    runtimeOf(node[ARENA]).call("a.nodeSet", node[ARENA], node[ARENAID], key, value);
}

export class AstArenaNode {
    constructor(arena, id) {
        this[ARENA] = arena;
        this[ARENAID] = id;
    }

    get typ() {
        return this[ARENA].dataTypes[this[ARENAID]];
    }

    set typ(typ) {
        this[ARENA].dataTypes[this[ARENAID]] = typ;
    }

    toJSON() {
        return runtimeOf(this[ARENA]).call("a.nodeJSON", this[ARENA], this[ARENAID]);
    }

    [Symbol.for("nodejs.util.inspect.custom")]() {
        return this.toJSON();
    }
}

for (const key of [
    "nam",
    "sel",
    "val",
    "chi",
    "tokens",
    "unit",
    "cal",
    "attr",
    "kin",
    "sign",
    "op",
    "op1",
    "op2",
    "l",
    "r",
    "loc",
    "state",
    "errors",
    "parent",
]) {
    Object.defineProperty(AstArenaNode.prototype, key, {
        configurable: true,
        enumerable: false,
        get() {
            const value = get(this, key);
            return key === "loc" && value != null ? Object.freeze(value) : value;
        },
        set(value) {
            set(this, key, value);
        },
    });
}
