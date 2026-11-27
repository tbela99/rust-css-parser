// Port of the public API of src/ast/arena.ts over the rust arenas.
//
// The columns (dataTypes, parents, ...) are typed arrays over the rust memory: reads and writes go to the arena
// without a copy. They are created again when the arena reallocates its columns (its generation changes) or when the
// memory moves (WASM memory growth). The other methods are rust calls.
//
// An arena exported with toTransferableArenaObject() or created with fromTransferableArenaObjects() uses javascript
// buffers as its columns, like the typescript arenas: the buffers can be transferred to another thread. Rust memory
// cannot be transferred, so the buffers are copied into the rust arena before each call and copied back after it,
// until the arena grows (or until the buffers are transferred: the arena can no longer be used).

import { classes, columnTypes, rt } from "../core.js";
import { ARENA, ARENAID, AstArenaNode } from "./arena-node.js";
import { EnumToken } from "./types.js";

export const TokenField = Object.freeze({
    L: 0,
    R: 1,
    Op: 2,
    Op2: 3,
    Val: 4,
    0: "L",
    1: "R",
    2: "Op",
    3: "Op2",
    4: "Val",
});

export const arenaColumns = Object.freeze([
    "dataTypes",
    "parents",
    "nextSibling",
    "previousSibling",
    "firstToken",
    "lastToken",
    "firstChild",
    "lastChild",
    "name",
    "stringValue",
    "numericValue",
    "unit",
    "flags",
    "colorType",
    "spans",
]);

export const numericTokenTypes = new Set([
    EnumToken.NumberTokenType,
    EnumToken.DimensionTokenType,
    EnumToken.LengthTokenType,
    EnumToken.AngleTokenType,
    EnumToken.TimeTokenType,
    EnumToken.FrequencyTokenType,
    EnumToken.ResolutionTokenType,
    EnumToken.PercentageTokenType,
    EnumToken.FlexTokenType,
    EnumToken.UserDefinedDimensionTokenType,
]);

export const structuredTokenTypes = new Set([
    EnumToken.BinaryExpressionTokenType,
    EnumToken.MatchExpressionTokenType,
    EnumToken.MediaQueryConditionTokenType,
    EnumToken.MediaQueryUnaryFeatureTokenType,
    EnumToken.SupportsQueryConditionTokenType,
    EnumToken.SupportsQueryUnaryConditionTokenType,
    EnumToken.WhenElseQueryConditionTokenType,
    EnumToken.WhenElseUnaryConditionTokenType,
    EnumToken.IfConditionTokenType,
    EnumToken.IfElseConditionTokenType,
    EnumToken.ContainerStyleRangeTokenType,
    EnumToken.MediaRangeQueryTokenType,
    EnumToken.ComposesSelectorNodeType,
    EnumToken.NameSpaceAttributeTokenType,
]);

const structuredFieldsCache = [];
const noFields = Object.freeze([]);

export function structuredFields(typ) {
    let fields = structuredFieldsCache[typ];

    if (fields != null) {
        return fields;
    }

    if (!structuredTokenTypes.has(typ)) {
        fields = noFields;
    } else {
        const single =
            typ === EnumToken.BinaryExpressionTokenType ||
            typ === EnumToken.IfElseConditionTokenType ||
            typ === EnumToken.MatchExpressionTokenType ||
            typ === EnumToken.NameSpaceAttributeTokenType;
        const singleLeft =
            single ||
            typ === EnumToken.MediaQueryConditionTokenType ||
            typ === EnumToken.MediaQueryUnaryFeatureTokenType ||
            typ === EnumToken.SupportsQueryUnaryConditionTokenType ||
            typ === EnumToken.WhenElseUnaryConditionTokenType;
        const singleRight = single || typ === EnumToken.ComposesSelectorNodeType;

        fields = [
            ["l", TokenField.L, singleLeft],
            ["r", TokenField.R, singleRight],
            [typ === EnumToken.MediaRangeQueryTokenType ? "op1" : "op", TokenField.Op, typ !== EnumToken.ContainerStyleRangeTokenType],
            ["op2", TokenField.Op2, true],
            ["val", TokenField.Val, false],
        ];
    }

    structuredFieldsCache[typ] = fields;
    return fields;
}

// ---- arena registry ------------------------------------------------------------------------------------------------

/**
 * state of an arena wrapper: rust handle, cached column views, javascript columns (transfer)
 */
const states = new WeakMap();
// the wrappers by handle are `runtime.arenas`: an arena returned twice by rust is the same object. the rust arena is
// released when its wrapper is garbage collected
const registry = new FinalizationRegistry(({ runtime, handle }) => {
    runtime.arenas.delete(handle);

    try {
        runtime.call("freeArena", handle);
    } catch {}
});

let creating = null;

function register(arena, handle, runtime) {
    states.set(arena, { handle, runtime, views: new Array(arenaColumns.length).fill(null), gen: null, genValue: -1, exported: null });
    runtime.arenas.set(handle, new WeakRef(arena));
    registry.register(arena, { runtime, handle });
}

function wrapArena(handle, runtime) {
    const existing = runtime.arenas.get(handle)?.deref();

    if (existing !== undefined) {
        return existing;
    }

    if (creating !== null) {
        const arena = creating;

        creating = null;
        register(arena, handle, runtime);
        return arena;
    }

    const arena = Object.create(AstArena.prototype);

    register(arena, handle, runtime);
    return arena;
}

function state(arena) {
    const st = states.get(arena);

    if (st === undefined) {
        throw new TypeError("not an arena");
    }

    return st;
}

function handleOf(arena, runtime) {
    const st = state(arena);

    if (runtime !== undefined && st.runtime !== runtime) {
        throw new TypeError(`the arena belongs to the ${st.runtime.name} backend, it cannot be used by the ${runtime.name} backend`);
    }

    return st.handle;
}

/**
 * a column view
 */
function column(arena, index) {
    const st = state(arena);

    if (st.exported !== null) {
        return st.exported[index];
    }

    const backend = st.runtime.backend;
    let gen = st.gen;

    if (gen === null || backend.stale(gen)) {
        gen = st.gen = backend.generation(st.handle);
        st.genValue = -1;
    }

    if (gen[0] !== st.genValue) {
        st.views.fill(null);
        st.genValue = gen[0];
    }

    let view = st.views[index];

    if (view === null || backend.stale(view)) {
        view = st.views[index] = backend.column(st.handle, index);
    }

    return view;
}

function isTransferred(st) {
    return st.exported !== null && st.exported[0].buffer.byteLength === 0;
}

function currentGeneration(st) {
    const backend = st.runtime.backend;

    if (st.gen === null || backend.stale(st.gen)) {
        st.gen = backend.generation(st.handle);
    }

    return st.gen[0];
}

function isExported(arena) {
    return states.get(arena)?.exported != null;
}

/**
 * copy the javascript columns into the rust arenas
 */
function beforeCall(arenas) {
    for (const arena of arenas) {
        const st = state(arena);

        if (isTransferred(st)) {
            // the buffers were moved to another thread: the rust arena is no longer used
            continue;
        }

        st.runtime.call("a.writeColumns", arena, st.exported.map((view) => new Uint8Array(view.buffer, view.byteOffset, view.byteLength)));
        st.syncGen = currentGeneration(st);
    }
}

/**
 * copy the rust columns back into the javascript columns. the arena uses the rust memory again when it grew
 */
function afterCall(arenas) {
    for (const arena of arenas) {
        const st = state(arena);

        if (isTransferred(st)) {
            continue;
        }

        if (currentGeneration(st) !== st.syncGen) {
            st.exported = null;
            st.views.fill(null);
            st.genValue = -1;
            continue;
        }

        for (let i = 0; i < arenaColumns.length; i++) {
            const view = st.runtime.backend.column(st.handle, i);

            if (view.length !== st.exported[i].length) {
                st.exported = null;
                st.views.fill(null);
                st.genValue = -1;
                break;
            }

            st.exported[i].set(view);
        }
    }
}

Object.assign(classes, {
    AstArenaNode,
    runtimeOf: (value) => states.get(value instanceof AstArenaNode ? value[ARENA] : value)?.runtime,
    wrapArena,
    handleOf,
    isArena: (value) => states.has(value),
    isExported,
    beforeCall,
    afterCall,
});

// ---- walkers -------------------------------------------------------------------------------------------------------

class Walker {
    #handle;
    #runtime;

    constructor(handle, runtime) {
        this.#handle = handle;
        this.#runtime = runtime;
    }

    next() {
        if (this.#handle === 0) {
            return { done: true, value: undefined };
        }

        const value = this.#runtime.call("walkerNext", this.#handle);

        if (value === null) {
            this.#handle = 0;
            return { done: true, value: undefined };
        }

        return { done: false, value };
    }

    return(value) {
        if (this.#handle !== 0) {
            this.#runtime.call("walkerFree", this.#handle);
            this.#handle = 0;
        }

        return { done: true, value };
    }

    [Symbol.iterator]() {
        return this;
    }
}

// ---- arenas --------------------------------------------------------------------------------------------------------

const epochs = new Map();

function call(arena, method, ...args) {
    return state(arena).runtime.call("a." + method, arena, ...args);
}

export class Arena {
    constructor(size = 4096) {
        creating = this;

        try {
            rt.call("newArena", size);
        } finally {
            creating = null;
        }

    }

    get root() {
        return 1;
    }

    get current() {
        return call(this, "current");
    }

    get transferred() {
        const st = state(this);
        return isTransferred(st) || (st.exported === null && call(this, "transferred"));
    }

    get stringTable() {
        const arena = this;

        return {
            get cache() {
                return call(arena, "stringCache");
            },
            get count() {
                return call(arena, "getStringCount");
            },
        };
    }

    get stringEpoch() {
        const epoch = call(this, "stringEpoch");
        let object = epochs.get(epoch);

        if (object === undefined) {
            object = {};
            epochs.set(epoch, object);
        }

        return object;
    }

    get errors() {
        return new Map(call(this, "errors"));
    }

    static toTransferableArenaObject(arena) {
        const st = state(arena);
        const properties = call(arena, "exportProperties", arena instanceof AstArena);
        const views = arenaColumns.map((_, i) => column(arena, i).slice());

        st.exported = views;
        st.syncGen = currentGeneration(st);

        return [properties, views.map((view) => view.buffer)];
    }

    static fromTransferableArenaObjects(properties, transferables) {
        creating = Object.create(this.prototype);

        let arena;

        try {
            arena = rt.call("arenaFromTransferables", properties, transferables.map((buffer) => new Uint8Array(buffer)));
        } finally {
            creating = null;
        }

        const st = state(arena);

        st.exported = transferables.map((buffer, i) => new columnTypes[i](buffer));
        st.syncGen = currentGeneration(st);
        return arena;
    }

    // -- columns

    getType(id) {
        return this.dataTypes[id];
    }

    typeOf(id) {
        return id == null ? null : this.dataTypes[id];
    }

    setType(id, type) {
        this.dataTypes[id] = type;
    }

    setName(id, nameId) {
        this.name[id] = nameId;
    }

    setValue(id, valueId) {
        this.stringValue[id] = valueId;
    }

    setNumericValue(id, numericValue) {
        this.numericValue[id] = numericValue;
    }

    setUnit(id, unit) {
        this.unit[id] = unit;
    }

    getNumericValue(id) {
        return this.numericValue[id];
    }

    setParent(id, parentId) {
        this.parents[id] = parentId;
    }

    getParent(id) {
        return this.parents[id];
    }

    getFirstChild(id) {
        return this.firstChild[id];
    }

    getLastChild(id) {
        return this.lastChild[id];
    }

    getNextSibling(id) {
        return this.nextSibling[id];
    }

    getPreviousSibling(id) {
        return this.previousSibling[id];
    }

    getFirstToken(id) {
        return this.firstToken[id];
    }

    getLastToken(id) {
        return this.lastToken[id];
    }

    setColorType(id, colorType) {
        this.colorType[id] = colorType;
    }

    getColorType(id) {
        return this.colorType[id];
    }

    setFlags(id, flags) {
        this.flags[id] = flags;
    }

    getFlags(id) {
        return this.flags[id];
    }

    hasFlags(id, flag) {
        return (this.flags[id] & flag) === flag;
    }

    getCapacity() {
        return this.dataTypes.length;
    }

    getLastId() {
        return this.current;
    }

    // -- strings

    internRange(codes, start, end, source) {
        if (typeof codes === "string") {
            return call(this, "internRange", codes, start, end, source, false);
        }

        return call(this, "internRange", new Uint8Array(codes.buffer, codes.byteOffset, codes.byteLength), start, end, source, codes.BYTES_PER_ELEMENT === 2);
    }

    setSourceText(source, text, codes = null) {
        if (codes == null) {
            return call(this, "setSourceText", source, text);
        }

        return call(this, "setSourceText", source, text, new Uint8Array(codes.buffer, codes.byteOffset, codes.byteLength), codes.BYTES_PER_ELEMENT === 2);
    }

    copyStringAscii(id, target, offset) {
        const bytes = call(this, "copyStringAscii", id);

        if (typeof bytes === "number") {
            return bytes;
        }

        target.set(bytes, offset);
        return bytes.length;
    }

    appendString(builder, id) {
        return builder.appendString(call(this, "stringText", id));
    }

    internBuilder(builder) {
        return this.internString(builder.toString());
    }

    // -- walkers

    walk(node, filter, reverse) {
        return new Walker(call(this, "walk", node, filter, reverse), state(this).runtime);
    }

    walkValues(tokens, filter, reverse) {
        return new Walker(call(this, "walkValues", tokens, filter, reverse), state(this).runtime);
    }

    compact(errors) {
        return Uint32Array.from(call(this, "compact", errors));
    }

    trim(errors) {
        return Uint32Array.from(call(this, "trim", errors));
    }
}

for (let i = 0; i < arenaColumns.length; i++) {
    Object.defineProperty(Arena.prototype, arenaColumns[i], {
        configurable: true,
        get() {
            return column(this, i);
        },
    });
}

// the methods implemented by rust
for (const method of [
    "allocate",
    "recycle",
    "internString",
    "stringId",
    "stringLength",
    "stringCodeAt",
    "stringIndexOf",
    "stringEquals",
    "stringEqualsIgnoreCase",
    "stringStartsWith",
    "lowerStringId",
    "stringFlag",
    "releaseSourceCodes",
    "getName",
    "getValue",
    "getUnit",
    "getStringCount",
    "setFirstChild",
    "setLastChild",
    "setNextSibling",
    "setPreviousSibling",
    "setFirstToken",
    "getChildAt",
    "getTokenAt",
    "setChildAt",
    "fieldList",
    "hasField",
    "field",
    "fieldTokens",
    "setField",
    "appendField",
    "setLeft",
    "getLeft",
    "setRight",
    "getRight",
    "setOp",
    "getOp",
    "setOp2",
    "getOp2",
    "setSpan",
    "copySpan",
    "getSourceId",
    "getSpanStart",
    "getSpanEnd",
    "setSpandStart",
    "setSpanEnd",
    "resolveString",
    "detach",
    "getValueParent",
    "getValueOwner",
    "find",
    "findAll",
    "findLast",
    "findByValue",
    "findValue",
    "subTokens",
    "findChild",
    "findToken",
    "siblings",
    "children",
    "tokens",
    "tokenCount",
    "setChildren",
    "setChildList",
    "setTokens",
    "setTokenList",
    "moveChildren",
    "appendChildren",
    "appendChildList",
    "appendTokens",
    "appendTokenList",
    "appendChild",
    "appendToken",
    "prependTokens",
    "equalsIgnoreCase",
    "startsWith",
    "resize",
    "compactStrings",
    "mark",
    "isMarked",
    "release",
    "getErrors",
    "childCount",
]) {
    Arena.prototype[method] = function (...args) {
        return call(this, method, ...args);
    };
}

export class AstArena extends Arena {
    get inputLengths() {
        return call(this, "inputLengths");
    }

    toObject(id) {
        return new AstArenaNode(this, id);
    }

    import(source, id, deep = true) {
        return call(this, "import", source, id, deep);
    }
}

for (const method of [
    "getState",
    "setState",
    "isInvalid",
    "sourceIndex",
    "setLocation",
    "setTokenLocation",
    "copyLocation",
    "spanLocation",
    "hasLocation",
    "getLocation",
    "getSource",
    "createStyleSheet",
    "createRule",
    "createAtRule",
    "createDeclaration",
    "createComment",
    "createToken",
    "createColor",
    "createFunction",
    "createList",
    "createBinaryExpression",
    "createFraction",
    "hasFraction",
    "getFraction",
    "setFraction",
    "numerator",
    "denominator",
    "str",
    "setStr",
    "setUnitString",
    "isNumeric",
    "replaceToken",
    "take",
    "copyToken",
    "resetToken",
    "fromObject",
    "toPlainObject",
    "tokenToObject",
    "cloneNode",
    "clone",
    "indexOf",
    "insertBefore",
    "insertAfter",
    "prependChildren",
    "replaceChild",
    "hasChildren",
    "getSelector",
    "setSelector",
    "setNameString",
    "setValueString",
    "expand",
    "replaceCompound",
]) {
    AstArena.prototype[method] = function (...args) {
        return call(this, method, ...args);
    };
}

export { ARENA, ARENAID };
