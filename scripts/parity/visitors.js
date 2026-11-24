// visitors called sync and async: `node scripts/parity/visitors.js <dist directory>` prints the results of every case,
// to compare the typescript build with the rust bindings
import { pathToFileURL } from "node:url";
import { resolve } from "node:path";

const m = await import(pathToFileURL(resolve(process.argv[2], "node.js")).href);
const { transform, transformSync, EnumToken, WalkerEvent } = m;

const css = `@media screen { .a { height: 10px; color: rgb(255 0 0); background: url("a.png") } }
.b { height: 2px; margin: 1px 2px; transform: rotate(10deg) }
@supports (display: grid) { .c { width: calc(1px + 2px) } }`;

const delay = () => new Promise((resolve) => setTimeout(resolve, 1));

// a handler, sync or async. the logging handlers return the length of the log: not a node id
const result = (value) => (typeof value === "number" ? undefined : value);
const make = (async, fn) => (async ? async (arena, id) => (await delay(), result(fn(arena, id))) : (arena, id) => result(fn(arena, id)));

const cases = {
    "declaration rename": (async, log) => ({
        DeclarationNodeType: make(async, (arena, id) => {
            const node = arena.toObject(id);
            log.push(node.nam);
            if (node.nam === "height") node.nam = "width";
        }),
    }),
    "declaration replace": (async) => ({
        DeclarationNodeType: make(async, (arena, id) =>
            arena.getName(id) === "margin"
                ? [{ typ: EnumToken.DeclarationNodeType, nam: "padding", val: [{ typ: EnumToken.LengthTokenType, val: 3, unit: "px" }] }]
                : null,
        ),
    }),
    "declaration remove (empty array)": (async) => ({
        DeclarationNodeType: make(async, (arena, id) => (arena.getName(id) === "transform" ? [] : null)),
    }),
    "rule selector": (async, log) => ({
        RuleNodeType: make(async, (arena, id) => {
            log.push(arena.getSelector(id));
            arena.setSelector(id, arena.getSelector(id) + ".x");
        }),
    }),
    "at-rule by name": (async, log) => ({
        AtRule: {
            media: make(async, (arena, id) => {
                log.push("media " + arena.getValue(id));
                arena.setValueString(id, "print");
            }),
        },
    }),
    "value token": (async, log) => ({
        UrlFunctionTokenType: make(async, (arena, id) => log.push(arena.str(id))),
        AngleTokenType: make(async, (arena, id) => {
            log.push("angle " + arena.getNumericValue(id));
            arena.setNumericValue(id, 20);
        }),
    }),
    "enter / leave": (async, log) => ({
        RuleNodeType: [
            { type: WalkerEvent.Enter, handler: make(async, (arena, id) => log.push("enter " + arena.getSelector(id))) },
            { type: WalkerEvent.Leave, handler: make(async, (arena, id) => log.push("leave " + arena.getSelector(id))) },
        ],
    }),
    "enter | leave flags": (async, log) => ({
        AngleTokenType: { type: WalkerEvent.Enter | WalkerEvent.Leave, handler: make(async, (arena, id) => log.push("angle " + arena.getNumericValue(id))) },
        DeclarationNodeType: { type: WalkerEvent.Enter | WalkerEvent.Leave, handler: make(async, (arena, id) => log.push("decl " + arena.getName(id))) },
    }),
    "visit all": (async, log) => make(async, (arena, id) => log.push(EnumToken[arena.getType(id)])),
    "visitor list": (async, log) => [
        { DeclarationNodeType: make(async, (arena, id) => log.push("first " + arena.getName(id))) },
        { DeclarationNodeType: make(!async, (arena, id) => log.push("second " + arena.getName(id))) },
    ],
    "mixed sync and async": (async, log) => ({
        DeclarationNodeType: make(async, (arena, id) => log.push("decl " + arena.getName(id))),
        RuleNodeType: make(!async, (arena, id) => log.push("rule " + arena.getSelector(id))),
    }),
    "throws": (async) => ({
        DeclarationNodeType: make(async, (arena, id) => {
            if (arena.getName(id) === "color") throw new Error("visitor error");
        }),
    }),
};

const run = async (name, mode) => {
    const log = [];
    const async = mode !== "sync";
    const visitor = cases[name](async, log);

    try {
        const result =
            mode === "async in sync"
                ? transformSync(css, { visitor, beautify: true })
                : mode === "sync"
                  ? await transform(css, { visitor, beautify: true })
                  : await transform(css, { visitor, beautify: true });
        const syncResult = mode === "sync" ? transformSync(css, { visitor: cases[name](false, []), beautify: true }).code : "";
        return { code: result.code, syncSame: mode !== "sync" || syncResult === result.code, log, errors: result.errors.map((e) => e.message) };
    } catch (error) {
        return { error: error.message, log };
    }
};

// css modules: the case transforms are flags
for (const naming of [m.ModuleCaseTransformEnum.CamelCase, m.ModuleCaseTransformEnum.DashCaseOnly, m.ModuleCaseTransformEnum.IgnoreCase | m.ModuleCaseTransformEnum.CamelCase, m.ModuleCaseTransformEnum.CamelCase | m.ModuleCaseTransformEnum.DashCase]) {
    for (const sync of [true, false]) {
        const source = ".foo-bar { color: red } .baz_qux { color: blue } :root { --my-var: 1px }";
        const options = { module: { naming }, src: "a.css" };
        const result = sync ? transformSync(source, options) : await transform(source, options);
        console.log(JSON.stringify({ name: "module naming " + naming, sync, code: result.code, mapping: result.mapping }));
    }
}

for (const name of Object.keys(cases)) {
    for (const mode of ["sync", "async", "async in sync"]) {
        const timeout = new Promise((resolve) => setTimeout(() => resolve({ error: "timeout" }), 5000).unref());
        console.log(JSON.stringify({ name, mode, ...(await Promise.race([run(name, mode), timeout])) }));
    }
}
