//! The functions called by javascript, by name. Port of the entry points of `src/node.ts` / `src/web.ts`
//! (`parseSyncInput`, `transformSync`, `render`, `parseResult` of `src/utils/sync.ts`) and of the arena methods.

use super::value::Val;
use super::{arena_handle, arena_of, free_arena, JsHost};
use crate::ast::arena::{AstArena, StrOrId, TokenValue};
use crate::ast::arena_node::{AstArenaNode, NodeInput, NodeValue};
use crate::ast::types::EnumToken;
use crate::fs::resolve::{dirname, resolve};
use crate::host::{HostError, Task, TaskState};
use crate::options::{ModuleOption, ModuleScoped, ParserOptions, SourceMapOption, SourcesMap};
use crate::parser::input::CssInput;
use crate::parser::parse::ParseResultStats;
use crate::parser::parse_main::{do_parse, ParseResult};
use crate::parser::source::SourceFile;
use crate::parser::tokenize::{create_parse_info, Tokenizer};
use crate::plain::Plain;
use crate::printer::render::{do_render, RenderMapping};
use crate::printer::result::RenderResult;
use crate::types::ErrorDescription;
use crate::utils::time::{format_ms, now};
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

type Result<T> = std::result::Result<T, HostError>;

fn arg(args: &[Val], i: usize) -> &Val {
    args.get(i).unwrap_or(super::value::undefined())
}

fn arena_arg(args: &[Val], i: usize) -> Result<Rc<AstArena>> {
    match arg(args, i) {
        Val::Arena(h) => arena_of(*h),
        Val::Num(n) => arena_of(*n as u32),
        other => Err(HostError(format!("an arena is expected, got {:?}", other))),
    }
}

/// a node id argument: a number or an `AstArenaNode`
fn id_arg(args: &[Val], i: usize) -> u32 {
    match arg(args, i) {
        Val::Node(_, id) => *id,
        v => v.u32(),
    }
}

fn str_arg(args: &[Val], i: usize) -> Rc<str> {
    match arg(args, i) {
        Val::Str(s) => s.clone(),
        Val::Bytes(b) => Rc::from(String::from_utf8_lossy(b).as_ref()),
        Val::Num(n) => Rc::from(crate::utils::number::js_number_to_string(*n).as_str()),
        _ => Rc::from(""),
    }
}

fn ids(list: Vec<u32>) -> Val {
    Val::Array(list.into_iter().map(|id| Val::Num(id as f64)).collect())
}

fn opt_id(id: Option<u32>) -> Val {
    id.map_or(Val::Null, |id| Val::Num(id as f64))
}

fn stats_val(stats: &ParseResultStats) -> Val {
    let mut entries = vec![
        ("src", Val::str(&stats.src)),
        ("bytesIn", Val::Num(stats.bytes_in as f64)),
        ("nodesCount", Val::Num(stats.nodes_count as f64)),
        ("tokensCount", Val::Num(stats.tokens_count as f64)),
        ("importedBytesIn", Val::Num(stats.imported_bytes_in as f64)),
        ("tokenize", Val::str(&stats.tokenize)),
        ("parse", Val::str(&stats.parse)),
        ("minify", Val::str(&stats.minify)),
        ("total", Val::str(&stats.total)),
        ("imports", Val::Array(stats.imports.iter().map(stats_val).collect())),
    ];
    if let Some(module) = &stats.module {
        entries.push(("module", Val::str(module)));
    }
    Val::object(entries)
}

fn errors_val(errors: &[ErrorDescription]) -> Val {
    Val::Array(errors.iter().map(|e| Val::from_plain(&e.to_plain())).collect())
}

fn str_map_val(entries: &[(String, String)]) -> Val {
    Val::Object(entries.iter().map(|(k, v)| (Rc::from(k.as_str()), Val::str(v))).collect())
}

fn parse_result_val(result: &ParseResult, keep_rev_mapping: bool) -> Val {
    let handle = arena_handle(&result.arena);
    let mut entries = vec![
        ("ast", Val::Num(result.ast as f64)),
        ("arena", Val::Arena(handle)),
        ("errors", errors_val(&result.errors)),
        ("stats", stats_val(&result.stats)),
    ];
    if let Some(mapping) = &result.mapping {
        entries.push(("mapping", str_map_val(mapping.entries())));
    }
    if let Some(variables) = &result.css_module_variables {
        entries.push(("cssModuleVariables", Val::Object(variables.iter().map(|(k, id)| (Rc::from(k.as_str()), Val::Node(handle, *id))).collect())));
    }
    if let Some(import_mapping) = &result.import_mapping {
        entries.push(("importMapping", Val::Object(import_mapping.iter().map(|(k, m)| (Rc::from(k.as_str()), str_map_val(m.entries()))).collect())));
    }
    if keep_rev_mapping {
        if let Some(rev) = &result.rev_mapping {
            entries.push(("revMapping", str_map_val(rev.entries())));
        }
    }
    Val::object(entries)
}

fn render_result_val(result: RenderResult) -> Val {
    let mut entries = vec![
        ("write", super::give_output(result.bytes)),
        ("errors", errors_val(&result.errors)),
        ("stats", Val::object(vec![("total", Val::str(&result.total))])),
        ("codeLength", Val::Num(result.code_length as f64)),
    ];
    if let Some(map) = result.map {
        entries.push(("map", Val::from_plain(&map.to_json().to_plain())));
    }
    Val::object(entries)
}

/// the input of a parse: a string or utf-8 bytes
fn css_input(input: &Val) -> CssInput {
    match input {
        Val::Bytes(b) => CssInput::Bytes(b.clone()),
        Val::Str(s) => CssInput::Str(s.clone()),
        other => CssInput::Str(Rc::from(other.as_str().unwrap_or(""))),
    }
}

/// a parse prepared like `parseSyncInput()` / `parseInput()`: the src is resolved, the source file is created
struct PreparedParse {
    tokenizer: Tokenizer,
    options: ParserOptions,
    arena: Option<Rc<AstArena>>,
}

fn prepare(input: &Val, options_val: &Val) -> Result<PreparedParse> {
    let mut options = ParserOptions::from_host(&options_val.to_def());
    let arena = match options_val.get("arena") {
        Val::Arena(h) => Some(arena_of(*h)?),
        _ => None,
    };
    let cwd = options.cwd.clone().unwrap_or_default();
    let src = resolve(options.src.as_deref().unwrap_or(""), &cwd, None).relative;
    options.src = Some(src.clone());
    let sources = options.sources_map.clone().unwrap_or_else(SourcesMap::default);
    options.sources_map = Some(sources.clone());
    let content: Rc<str> = match input {
        Val::Str(s) => s.clone(),
        _ => Rc::from(""),
    };
    let source = SourceFile::new(content, Vec::new(), Some(Rc::from(src.as_str())));
    sources.borrow_mut().insert(source.id, source.clone());
    options.source = Some(source.clone());
    let tokenizer = Tokenizer::new(create_parse_info(Some(&css_input(input)), source));
    Ok(PreparedParse { tokenizer, options, arena })
}

/// `parseResult()`: the input source map of the source maps, the reverse mapping of the css modules is dropped
fn finish_parse(result: &ParseResult, options: &ParserOptions) -> Val {
    if options.sourcemap.is_some_and(|s| s != SourceMapOption::Off) {
        if let Some(source) = &options.source {
            if source.get_input_source_map().is_none() {
                if let Some(input) = &options.input_source_map {
                    source.set_input_source_map(Some(input));
                } else {
                    // extract the inline source map of the input css
                    let arena = &result.arena;
                    let token = arena.last_child.get(result.ast);
                    if token != 0 && arena.data_types.get(token) == EnumToken::CommentTokenType && arena.starts_with(token, "val", "/*# sourceMappingURL=") {
                        let value = arena.resolve_string(arena.string_value.get(token));
                        let units: Vec<u16> = value.encode_utf16().collect();
                        let data = String::from_utf16_lossy(&units[21.min(units.len())..units.len().saturating_sub(2).max(21.min(units.len()))]).trim().to_string();
                        if data.ends_with(".map") {
                            // the map file is loaded asynchronously by the typescript port: the render does not wait
                            if let Some(load) = options.load {
                                let base = dirname(options.src.as_deref().unwrap_or(""));
                                let absolute = resolve(&data, &base, None).absolute;
                                let _ = super::call_js(load, None, vec![Val::str(&absolute), Val::str("."), Val::Num(3.0)]);
                            }
                        } else {
                            source.set_input_source_map(Some(&data));
                        }
                    }
                }
            }
        }
    }
    parse_result_val(result, options.module.is_none())
}

fn module_icss(options: &ParserOptions) -> bool {
    use crate::ast::types::ModuleScopeEnumOptions;
    match &options.module {
        Some(ModuleOption::Flags(flags)) => flags & ModuleScopeEnumOptions::ICSS != 0,
        Some(ModuleOption::Options(m)) => matches!(m.scoped, Some(ModuleScoped::Flags(f)) if f & ModuleScopeEnumOptions::ICSS != 0),
        _ => false,
    }
}

/// `render()` / the render of `transform()`
fn render_impl(arena: &AstArena, ast: u32, options: &ParserOptions, mapping: Option<&RenderMapping>) -> RenderResult {
    do_render(arena, ast, options, mapping)
}

fn mapping_of(result: &ParseResult, options: &ParserOptions) -> Option<RenderMapping> {
    if !module_icss(options) {
        return None;
    }
    Some(RenderMapping {
        mapping: result.mapping.as_ref().map(|m| m.entries().to_vec()).unwrap_or_default(),
        import_mapping: result.import_mapping.as_ref().map(|m| m.iter().map(|(k, v)| (k.clone(), v.entries().to_vec())).collect()),
    })
}

/// the transform defaults (`transformSync()`)
fn transform_options(options: &Val) -> Val {
    let mut entries = match options {
        Val::Object(entries) => entries.clone(),
        _ => Vec::new(),
    };
    // the fast mode sets the default value of minify (see fastModeOptions)
    let fast_mode = entries.iter().any(|(k, v)| &**k == "fastMode" && v.truthy());
    for key in ["minify", "removeEmpty", "removeCharset"] {
        if fast_mode && key == "minify" {
            continue;
        }
        match entries.iter_mut().find(|(k, _)| &**k == key) {
            Some((_, v)) if v.is_nullish() => *v = Val::Bool(true),
            Some(_) => {}
            None => entries.push((Rc::from(key), Val::Bool(true))),
        }
    }
    Val::Object(entries)
}

/// the transform result: the parse result, the render result and the stats
fn transform_result(result: &ParseResult, options: &ParserOptions, start: f64) -> Val {
    let render_options = ParserOptions { expand_nesting_rules: Some(false), ..options.clone() };
    let mapping = mapping_of(result, options);
    // the input source map is set by the parse result before the render
    let parse = finish_parse(result, options);
    let rendered = render_impl(&result.arena, result.ast, &render_options, mapping.as_ref());
    let code_length = rendered.code_length;
    let total = rendered.total.clone();
    let render = render_result_val(rendered);
    let mut stats = match parse.get("stats") {
        Val::Object(entries) => entries.clone(),
        _ => Vec::new(),
    };
    stats.insert(0, (Rc::from("bytesOut"), Val::Num(code_length as f64)));
    stats.push((Rc::from("render"), Val::str(&total)));
    match stats.iter_mut().find(|(k, _)| &**k == "total") {
        Some((_, v)) => *v = Val::str(&format_ms(now() - start)),
        None => stats.push((Rc::from("total"), Val::str(&format_ms(now() - start)))),
    }
    Val::object(vec![("parse", parse), ("render", render), ("stats", Val::Object(stats))])
}

type BoxedFuture = Pin<Box<dyn Future<Output = Result<Val>>>>;

thread_local! {
    /// the async calls (`parse()`, `transform()`) driven by javascript, by id (1 based)
    static TASKS: RefCell<Vec<Option<Task<Result<Val>>>>> = const { RefCell::new(Vec::new()) };
}

fn start_task(future: BoxedFuture) -> Val {
    let task = Task::new(future);
    let id = TASKS.with(|t| {
        let mut tasks = t.borrow_mut();
        match tasks.iter().position(|t| t.is_none()) {
            Some(i) => {
                tasks[i] = Some(task);
                i + 1
            }
            None => {
                tasks.push(Some(task));
                tasks.len()
            }
        }
    });
    Val::Num(id as f64)
}

/// run a task until it completes or waits for javascript promises: `{ done: true, value }`, `{ pending: [ids] }`
fn poll_task(id: u32) -> Result<Val> {
    let mut task = TASKS.with(|t| t.borrow_mut().get_mut((id as usize).wrapping_sub(1)).and_then(|t| t.take())).ok_or_else(|| HostError("invalid task".into()))?;
    let state = task.poll();
    match state {
        TaskState::Done(result) => Ok(Val::object(vec![("done", Val::Bool(true)), ("value", result?)])),
        TaskState::Pending(ids) => {
            TASKS.with(|t| t.borrow_mut()[id as usize - 1] = Some(task));
            Ok(Val::object(vec![("done", Val::Bool(false)), ("pending", Val::Array(ids.into_iter().map(|i| Val::Num(i as f64)).collect()))]))
        }
    }
}

/// a callback option (`filter`, `matcher`): a function called with `(node)`. its result is truthy or not
fn predicate(arena: &Rc<AstArena>, callback: &Val) -> impl Fn(u32) -> bool {
    let id = match callback {
        Val::Fn(id, _) => Some(*id),
        _ => None,
    };
    let arena = arena.clone();
    move |node: u32| match id {
        None => true,
        Some(id) => super::call_js(id, Some(&arena), vec![Val::Num(node as f64)]).map(|v| v.truthy()).unwrap_or(false),
    }
}

fn location_val(l: &crate::types::SourceLocation) -> Val {
    Val::object(vec![("sta", Val::Num(l.sta as f64)), ("end", Val::Num(l.end as f64)), ("srcId", Val::Num(l.src_id as f64))])
}

/// a value read from an `AstArenaNode` view
fn node_value(arena: &Rc<AstArena>, value: NodeValue) -> Val {
    let handle = arena_handle(arena);
    match value {
        NodeValue::Undefined => Val::Undefined,
        NodeValue::Null => Val::Null,
        NodeValue::Str(s) => Val::Str(s),
        NodeValue::Num(n) => Val::Num(n),
        NodeValue::Node(id) => Val::Node(handle, id),
        NodeValue::Nodes(ids) => Val::Array(ids.into_iter().map(|id| Val::Node(handle, id)).collect()),
        NodeValue::Location(l) => location_val(&l),
        NodeValue::Errors(errors) => errors_val(&errors),
    }
}

/// a node of a javascript value: an id of the arena, a view of another arena is imported
fn foreign_node(arena: &Rc<AstArena>, handle: u32, id: u32) -> u32 {
    match arena_of(handle) {
        Ok(other) if !Rc::ptr_eq(&other, arena) => arena.import(&other, id, true),
        _ => id,
    }
}

/// a value assigned to an `AstArenaNode` view property
fn node_input(arena: &Rc<AstArena>, key: &str, value: &Val) -> NodeInput {
    match value {
        Val::Null | Val::Undefined if key == "errors" => NodeInput::Errors(None),
        Val::Array(items) if key == "errors" => NodeInput::Errors(Some(items.iter().map(|e| crate::types::ErrorDescription::from_plain(&e.to_plain())).collect())),
        Val::Object(_) if key == "loc" => NodeInput::Location(Some(crate::types::SourceLocation {
            sta: value.get("sta").u32(),
            end: value.get("end").u32(),
            src_id: value.get("srcId").u32(),
        })),
        Val::Null | Val::Undefined if key == "loc" => NodeInput::Location(None),
        Val::Null | Val::Undefined => NodeInput::Null,
        Val::Str(s) => NodeInput::Str(s.clone()),
        Val::Num(n) => NodeInput::Num(*n),
        Val::Node(handle, id) => NodeInput::Id(foreign_node(arena, *handle, *id)),
        Val::Array(items) => NodeInput::List(items.iter().map(|v| node_input(arena, "", v)).collect()),
        other => NodeInput::Plain(other.to_plain()),
    }
}

/// call a function by name
pub fn dispatch(method: &str, args: Vec<Val>) -> Result<Val> {
    let a = &args;
    Ok(match method {
        // ---- arenas
        "newArena" => {
            let size = arg(a, 0).u32().max(16) as usize;
            Val::Arena(arena_handle(&AstArena::shared(size)))
        }
        "freeArena" => {
            free_arena(arg(a, 0).u32());
            Val::Undefined
        }
        // ---- parser
        "parseSync" => {
            let mut prepared = prepare(arg(a, 0), arg(a, 1))?;
            // the restrictions of the typescript parseSync()
            prepared.options.sync_api_compat = true;
            let options = prepared.options.clone();
            let result = crate::host::block_on(do_parse(prepared.tokenizer, prepared.options, prepared.arena, Rc::new(JsHost)))?;
            finish_parse(&result, &options)
        }
        "parse" => {
            let prepared = prepare(arg(a, 0), arg(a, 1))?;
            let options = prepared.options.clone();
            start_task(Box::pin(async move {
                let result = do_parse(prepared.tokenizer, prepared.options, prepared.arena, Rc::new(JsHost)).await?;
                Ok(finish_parse(&result, &options))
            }))
        }
        "transformSync" => {
            let start = now();
            let options_val = transform_options(arg(a, 1));
            let mut prepared = prepare(arg(a, 0), &options_val)?;
            // the restrictions of the typescript transformSync()
            prepared.options.sync_api_compat = true;
            let options = prepared.options.clone();
            let result = crate::host::block_on(do_parse(prepared.tokenizer, prepared.options, prepared.arena, Rc::new(JsHost)))?;
            transform_result(&result, &options, start)
        }
        "transform" => {
            let start = now();
            let options_val = transform_options(arg(a, 1));
            let prepared = prepare(arg(a, 0), &options_val)?;
            let options = prepared.options.clone();
            start_task(Box::pin(async move {
                let result = do_parse(prepared.tokenizer, prepared.options, prepared.arena, Rc::new(JsHost)).await?;
                Ok(transform_result(&result, &options, start))
            }))
        }
        "poll" => poll_task(arg(a, 0).u32())?,
        "settle" => {
            let result = if arg(a, 1).truthy() { Ok(arg(a, 2).to_host()) } else { Err(HostError(str_arg(a, 2).to_string())) };
            crate::host::settle_promise(arg(a, 0).u32(), result);
            Val::Undefined
        }
        "render" => {
            let arena = arena_arg(a, 0)?;
            let options = ParserOptions::from_host(&arg(a, 2).to_def());
            let mapping = match arg(a, 3) {
                Val::Object(_) => Some(RenderMapping {
                    mapping: match arg(a, 3).get("mapping") {
                        Val::Object(e) => e.iter().map(|(k, v)| (k.to_string(), v.as_str().unwrap_or("").to_string())).collect(),
                        _ => Vec::new(),
                    },
                    import_mapping: match arg(a, 3).get("importMapping") {
                        Val::Object(e) => Some(
                            e.iter()
                                .map(|(k, v)| {
                                    let inner = match v {
                                        Val::Object(e) => e.iter().map(|(k, v)| (k.to_string(), v.as_str().unwrap_or("").to_string())).collect(),
                                        _ => Vec::new(),
                                    };
                                    (k.to_string(), inner)
                                })
                                .collect(),
                        ),
                        _ => None,
                    },
                }),
                _ => None,
            };
            if arg(a, 4).truthy() {
                // a memo given by the caller: its statistics are returned
                let mut memo = crate::printer::render::RenderMemo::default();
                let result = crate::printer::render::do_render_with(&arena, id_arg(a, 1), &options, mapping.as_ref(), Some(&mut memo));
                let stats = |m: &crate::ast::cache::StructuralMemo<Rc<str>>| Val::object(vec![("size", Val::Num(m.size as f64)), ("hits", Val::Num(m.hits as f64))]);
                let memo_val = Val::object(vec![("declarations", stats(&memo.declarations)), ("constructs", stats(&memo.constructs))]);
                match render_result_val(result) {
                    Val::Object(mut entries) => {
                        entries.push((Rc::from("memo"), memo_val));
                        Val::Object(entries)
                    }
                    other => other,
                }
            } else {
                render_result_val(render_impl(&arena, id_arg(a, 1), &options, mapping.as_ref()))
            }
        }
        "renderValue" => {
            let arena = arena_arg(a, 0)?;
            let options = ParserOptions::from_host(&arg(a, 2).to_def());
            Val::str(&crate::printer::render::render_value(&arena, id_arg(a, 1), &options))
        }
        "parseString" => {
            let arena = arena_arg(a, 0)?;
            let options = match arg(a, 2) {
                Val::Object(_) => Some(ParserOptions::from_host(&arg(a, 2).to_def())),
                _ => None,
            };
            ids(crate::parser::parse::parse_string_with(&arena, &str_arg(a, 1), options.as_ref(), None))
        }
        "parseDeclarations" => {
            let arena = match arg(a, 1) {
                Val::Arena(h) => Some(arena_of(*h)?),
                _ => None,
            };
            let (arena, nodes) = crate::parser::parse::parse_declarations(&str_arg(a, 0), arena);
            Val::object(vec![("arena", Val::Arena(arena_handle(&arena))), ("nodes", ids(nodes))])
        }
        "minify" => {
            let arena = arena_arg(a, 0)?;
            let options = ParserOptions::from_host(&arg(a, 2).to_def());
            let mut errors = Vec::new();
            let ast = crate::ast::minify::minify(&arena, id_arg(a, 1), &options, arg(a, 3).truthy(), Some(&mut errors), arg(a, 5).truthy());
            Val::Num(ast as f64)
        }
        "convertColor" => {
            let arena = arena_arg(a, 0)?;
            opt_id(crate::syntax::color::color::convert_color(&arena, id_arg(a, 1), arg(a, 2).u32() as u8))
        }
        "isOkLabClose" => {
            let arena = arena_arg(a, 0)?;
            let threshold = if arg(a, 3).is_nullish() { crate::syntax::constants::colorDistancePrecision } else { arg(a, 3).num() };
            Val::Bool(crate::syntax::color::utils::distance::is_ok_lab_close_with(&arena, id_arg(a, 1), id_arg(a, 2), threshold))
        }
        "okLabDistance" => {
            let arena = arena_arg(a, 0)?;
            crate::syntax::color::utils::distance::ok_lab_distance(&arena, id_arg(a, 1), id_arg(a, 2)).map_or(Val::Null, Val::Num)
        }
        "toPrecision6" => Val::Num(crate::ast::transform::utils::to_precision6(arg(a, 0).num())),
        "resolve" => {
            let cwd = arg(a, 2).as_str();
            let r = resolve(&str_arg(a, 0), &str_arg(a, 1), cwd);
            Val::object(vec![("absolute", Val::str(&r.absolute)), ("relative", Val::str(&r.relative))])
        }
        "dirname" => Val::str(&dirname(&str_arg(a, 0))),
        "parseSyntax" => {
            let tokens = crate::validation::parser::parse::parse_syntax(&str_arg(a, 0));
            Val::Array(tokens.iter().map(|t| Val::from_plain(&t.to_plain())).collect())
        }
        "renderSyntax" => {
            let tokens = crate::validation::parser::parse::parse_syntax(&str_arg(a, 0));
            let mut options = crate::validation::parser::parse::RenderSyntaxOptions::default();
            Val::str(&tokens.iter().map(|t| crate::validation::parser::parse::render_syntax(t, &mut options)).collect::<String>())
        }
        "sourceMapUrl" => {
            let json = Plain::from_json(&serde_json::from_str(&str_arg(a, 0)).unwrap_or(serde_json::Value::Null)).to_json();
            Val::str(&format!("data:application/json;charset=utf-8;base64,{}", crate::parser::utils::base64::to_base64(&json)))
        }
        "propertyList" => {
            // `new PropertyList(arena, options)`, `addDeclaration()` for each declaration, then the iteration
            let arena = arena_arg(a, 0)?;
            let options = ParserOptions::from_host(&arg(a, 1).to_def());
            let mut list = crate::parser::declaration::list::PropertyList::new(&options, None, None);
            for declaration in arg(a, 2).ids() {
                list.add_declaration(&arena, declaration);
            }
            ids(list.iterate(&arena))
        }
        "getParsedSyntax" => parsed_syntax(&str_arg(a, 0), &str_arg(a, 1)),
        "matchingAlternatives" => {
            let arena = arena_arg(a, 0)?;
            with_syntax(arg(a, 1), |list| match list {
                Some(SyntaxList::Lists(lists)) => crate::validation::first::matching_alternatives(&arena, lists, opt_token(arg(a, 2)))
                    .map_or(Val::Null, |l| Val::Array(l.iter().map(|&i| Val::Num(i as f64)).collect())),
                _ => Val::Null,
            })
        }
        "mayStartWith" => {
            let arena = arena_arg(a, 0)?;
            with_syntax(arg(a, 1), |list| match list {
                Some(SyntaxList::Tokens(tokens)) => Val::Bool(crate::validation::first::may_start_with(&arena, tokens, opt_token(arg(a, 2)))),
                _ => Val::Bool(true),
            })
        }
        "walkerNext" => walker_next(arg(a, 0).u32()),
        "walkerFree" => {
            walker_free(arg(a, 0).u32());
            Val::Undefined
        }
        "sourceMapFind" => source_map_find(&str_arg(a, 0), arg(a, 1).num() as i64, arg(a, 2).num() as i64),
        "arenaFromTransferables" => {
            let arena = AstArena::shared(16);
            import_properties(&arena, arg(a, 0), arg(a, 1));
            Val::Arena(arena_handle(&arena))
        }
        method if method.starts_with("a.") => arena_method(&method[2..], a)?,
        _ => return Err(HostError(format!("unknown function {}", method))),
    })
}

/// the methods of the arenas (`a.<name>`): the first argument is the arena handle
fn arena_method(name: &str, args: &[Val]) -> Result<Val> {
    let arena = arena_arg(args, 0)?;
    let a = &args[1..];
    let id = || id_arg(a, 0);
    let ar = &arena;
    Ok(match name {
        "current" => Val::Num(ar.current() as f64),
        "generation" => Val::Num(ar.generation() as f64),
        "allocate" => Val::Num(ar.allocate() as f64),
        "internString" => Val::Num(ar.intern_string(&str_arg(a, 0)) as f64),
        "stringId" => Val::Num(ar.string_id(&str_arg(a, 0)) as f64),
        "stringLength" => Val::Num(ar.string_length(id()) as f64),
        "stringCodeAt" => Val::Num(ar.string_code_at(id(), arg(a, 1).u32() as usize) as f64),
        "stringIndexOf" => Val::Num(ar.string_index_of(id(), arg(a, 1).u32()) as f64),
        "stringEquals" => Val::Bool(ar.string_equals(id(), &str_arg(a, 1))),
        "stringEqualsIgnoreCase" => Val::Bool(ar.string_equals_ignore_case(id(), &str_arg(a, 1))),
        "stringStartsWith" => Val::Bool(ar.string_starts_with(id(), &str_arg(a, 1))),
        "lowerStringId" => Val::Num(ar.lower_string_id(id()) as f64),
        "resolveString" => Val::Str(ar.resolve_string(id()).into()),
        "getStringCount" => Val::Num(ar.get_string_count() as f64),
        "stringEpoch" => Val::Num(ar.string_epoch() as f64),
        "getCapacity" => Val::Num(ar.get_capacity() as f64),
        "getLastId" => Val::Num(ar.get_last_id() as f64),
        "getName" => Val::Str(ar.get_name(id()).into()),
        "getValue" => Val::Str(ar.get_value(id()).into()),
        "getUnit" => Val::Str(ar.get_unit(id()).into()),
        "getSelector" => Val::Str(ar.get_name(id()).into()),
        "str" => Val::Str(ar.resolve_string(ar.string_value.get(id())).into()),
        "getChildAt" => opt_id(ar.get_child_at(id(), arg(a, 1).u32())),
        "getTokenAt" => opt_id(ar.get_token_at(id(), arg(a, 1).u32())),
        "setChildAt" => {
            ar.set_child_at(id(), arg(a, 1).u32(), id_arg(a, 2));
            Val::Undefined
        }
        "fieldList" => Val::Num(ar.field_list(id(), arg(a, 1).u32()) as f64),
        "hasField" => Val::Bool(ar.has_field(id(), arg(a, 1).u32())),
        "field" => Val::Num(ar.field(id(), arg(a, 1).u32()) as f64),
        "fieldTokens" => ids(ar.field_tokens(id(), arg(a, 1).u32())),
        "setField" => {
            ar.set_field(id(), arg(a, 1).u32(), &a[2..].iter().flat_map(|v| v.ids()).collect::<Vec<_>>());
            Val::Undefined
        }
        "setSpan" => {
            ar.set_span(id(), arg(a, 1).u32(), arg(a, 2).u32(), arg(a, 3).u32());
            Val::Undefined
        }
        "getSourceId" => Val::Num(ar.get_source_id(id()) as f64),
        "getSpanStart" => Val::Num(ar.get_span_start(id()) as f64),
        "getSpanEnd" => Val::Num(ar.get_span_end(id()) as f64),
        "detach" => {
            ar.detach(id());
            Val::Undefined
        }
        "walk" => walk_start(ar, a),
        "walkIds" => ids(ar.walk_ids(id())),
        "getValueParent" => Val::Num(ar.get_value_parent(id()) as f64),
        "getValueOwner" => Val::Num(ar.get_value_owner(id()) as f64),
        "find" => opt_id(ar.find(id(), |_, n| predicate(ar, arg(a, 1))(n))),
        "findAll" => ids(ar.find_all(id(), |_, n| predicate(ar, arg(a, 1))(n))),
        "findLast" => opt_id(ar.find_last(id(), |_, n| predicate(ar, arg(a, 1))(n))),
        "findValue" => opt_id(ar.find_value(id(), |_, n| predicate(ar, arg(a, 1))(n))),
        "findByValue" => match ar.find_by_value(id(), |_, n| predicate(ar, arg(a, 1))(n)) {
            Some((node, value)) => Val::object(vec![("node", Val::Num(node as f64)), ("value", Val::Num(value as f64))]),
            None => Val::Null,
        },
        "subTokens" => ids(ar.sub_tokens(id())),
        "children" | "tokens" | "siblings" => {
            let list = match name {
                "children" => ar.children(id()),
                "tokens" => ar.tokens(id()),
                _ => ar.siblings(id(), None),
            };
            match arg(a, 1) {
                Val::Fn(..) => {
                    let filter = predicate(ar, arg(a, 1));
                    ids(list.into_iter().filter(|&n| filter(n)).collect())
                }
                _ => ids(list),
            }
        }
        "findChild" => {
            let callback = arg(a, 1).clone();
            opt_id(ar.find_child(id(), |n| match &callback {
                Val::Fn(cb, _) => super::call_js(*cb, None, vec![Val::Num(n as f64)]).map(|v| v.truthy()).unwrap_or(false),
                _ => false,
            }))
        }
        "findToken" => opt_id(ar.find_token(id(), |_, n| predicate(ar, arg(a, 1))(n))),
        "walkValues" => walk_values_start(ar, a),
        "stringFlag" => string_flag(ar, a),
        "stringText" => string_text(ar, id()),
        "stringCache" => {
            let count = ar.get_string_count();
            Val::Array((0..count).map(|i| ar.strings().cached(i).map_or(Val::Undefined, |s| Val::Str(s.into()))).collect())
        }
        "internRange" => Val::Num(ar.intern_range(&code_units(arg(a, 0), arg(a, 4).truthy()), arg(a, 1).u32() as usize, arg(a, 2).u32() as usize, arg(a, 3).num() as i32) as f64),
        "setSourceText" => {
            let units = match arg(a, 2) {
                Val::Bytes(_) => code_units(arg(a, 2), arg(a, 3).truthy()),
                _ => code_units(arg(a, 1), false),
            };
            ar.set_source_text(arg(a, 0).u32() as usize, units);
            Val::Undefined
        }
        "releaseSourceCodes" => {
            ar.release_source_codes(arg(a, 0).u32() as usize);
            Val::Undefined
        }
        "copyStringAscii" => {
            let mut out = Vec::new();
            match ar.copy_string_ascii(id(), &mut out) {
                n if n < 0 => Val::Num(n as f64),
                _ => Val::Bytes(out.into()),
            }
        }
        "exportProperties" => export_properties(ar, arg(a, 0).truthy()),
        "importColumns" => {
            ar.import_columns(transferables(arg(a, 0)));
            Val::Undefined
        }
        "errors" => Val::Array(ar.errors.borrow().iter().map(|(id, e)| Val::Array(vec![Val::Num(*id as f64), errors_val(e)])).collect()),
        "inputLengths" => {
            let lengths = ar.input_lengths.borrow();
            let max = lengths.keys().max().map_or(0, |&m| m as usize + 1);
            Val::Array((0..max).map(|i| lengths.get(&(i as u32)).map_or(Val::Undefined, |&l| Val::Num(l as f64))).collect())
        }
        "transferred" => Val::Bool(ar.transferred()),
        "writeColumns" => {
            // javascript copies of the columns written back in place (same capacity)
            if let Val::Array(columns) = arg(a, 0) {
                for (index, column) in columns.iter().enumerate() {
                    if let Val::Bytes(bytes) = column {
                        let (ptr, length) = ar.column_raw(index as u32);
                        let size = match index {
                            0 | 13 => 1,
                            10 | 14 => 8,
                            _ => 4,
                        };
                        if bytes.len() == length * size {
                            // SAFETY: the column has `length` elements of `size` bytes, no borrow is alive
                            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr as *mut u8, bytes.len()) };
                        }
                    }
                }
            }
            Val::Undefined
        }
        "setFirstChild" => {
            ar.set_first_child(id(), id_arg(a, 1));
            Val::Undefined
        }
        "setLastChild" => {
            ar.set_last_child(id(), id_arg(a, 1));
            Val::Undefined
        }
        "setNextSibling" => {
            ar.set_next_sibling(id(), id_arg(a, 1));
            Val::Undefined
        }
        "setPreviousSibling" => {
            ar.set_previous_sibling(id(), id_arg(a, 1));
            Val::Undefined
        }
        "setFirstToken" => {
            ar.set_first_token(id(), id_arg(a, 1));
            Val::Undefined
        }
        "appendField" => {
            ar.append_field(id(), arg(a, 1).u32(), &a[2..].iter().flat_map(|v| v.ids()).collect::<Vec<_>>());
            Val::Undefined
        }
        "setLeft" => {
            ar.set_left(id(), &a[1..].iter().flat_map(|v| v.ids()).collect::<Vec<_>>());
            Val::Undefined
        }
        "getLeft" => ar.get_left(id()).map_or(Val::Null, ids),
        "setRight" => {
            ar.set_right(id(), &a[1..].iter().flat_map(|v| v.ids()).collect::<Vec<_>>());
            Val::Undefined
        }
        "getRight" => ar.get_right(id()).map_or(Val::Null, ids),
        "setOp" => {
            ar.set_op(id(), id_arg(a, 1));
            Val::Undefined
        }
        "getOp" => Val::Num(ar.get_op(id()) as f64),
        "copySpan" => {
            ar.copy_span(id(), id_arg(a, 1));
            Val::Undefined
        }
        "setSpanEnd" => {
            ar.set_span_end(id(), arg(a, 1).u32());
            Val::Undefined
        }
        "sourceIndex" => Val::Num(ar.source_index(arg(a, 0).u32()) as f64),
        "setTokenLocation" => {
            ar.set_token_location(id(), arg(a, 1).u32(), arg(a, 2).u32(), arg(a, 3).u32());
            Val::Undefined
        }
        "setFraction" => {
            ar.set_fraction(id(), if arg(a, 1).is_nullish() { None } else { Some(id_arg(a, 1)) });
            Val::Undefined
        }
        "numerator" => Val::Num(ar.numerator(id()) as f64),
        "denominator" => Val::Num(ar.denominator(id()) as f64),
        "tokenToObject" => Val::from_plain(&ar.token_to_object(id())),
        "replaceCompound" => Val::str(&ar.replace_compound(&str_arg(a, 0), &str_arg(a, 1))),
        "recycle" => {
            ar.recycle(id());
            Val::Undefined
        }
        "tokenCount" => Val::Num(ar.token_count(id()) as f64),
        "childCount" => Val::Num(ar.child_count(id()) as f64),
        "setChildren" | "setChildList" => {
            ar.set_child_list(id(), &a[1..].iter().flat_map(|v| v.ids()).collect::<Vec<_>>());
            Val::Undefined
        }
        "setTokens" | "setTokenList" => {
            ar.set_token_list(id(), &a[1..].iter().flat_map(|v| v.ids()).collect::<Vec<_>>());
            Val::Undefined
        }
        "moveChildren" => {
            ar.move_children(id(), id_arg(a, 1), id_arg(a, 2));
            Val::Undefined
        }
        "appendChildren" | "appendChildList" => {
            ar.append_child_list(id(), &a[1..].iter().flat_map(|v| v.ids()).collect::<Vec<_>>());
            Val::Undefined
        }
        "appendTokens" | "appendTokenList" => {
            ar.append_token_list(id(), &a[1..].iter().flat_map(|v| v.ids()).collect::<Vec<_>>());
            Val::Undefined
        }
        "appendChild" => {
            ar.append_child(id(), id_arg(a, 1));
            Val::Undefined
        }
        "appendToken" => {
            ar.append_token(id(), id_arg(a, 1));
            Val::Undefined
        }
        "prependTokens" => {
            ar.prepend_tokens(id(), &a[1..].iter().flat_map(|v| v.ids()).collect::<Vec<_>>());
            Val::Undefined
        }
        "prependChildren" => {
            ar.prepend_children(id(), &a[1..].iter().flat_map(|v| v.ids()).collect::<Vec<_>>());
            Val::Undefined
        }
        "insertBefore" => {
            ar.insert_before(id(), &a[1..].iter().flat_map(|v| v.ids()).collect::<Vec<_>>());
            Val::Undefined
        }
        "insertAfter" => {
            ar.insert_after(id(), &a[1..].iter().flat_map(|v| v.ids()).collect::<Vec<_>>());
            Val::Undefined
        }
        "replaceChild" | "replaceToken" => {
            ar.replace_child(id(), &a[1..].iter().flat_map(|v| v.ids()).collect::<Vec<_>>());
            Val::Undefined
        }
        "indexOf" => Val::Num(ar.index_of(id()) as f64),
        "equalsIgnoreCase" => Val::Bool(ar.equals_ignore_case(id(), &str_arg(a, 1), &str_arg(a, 2))),
        "startsWith" => Val::Bool(ar.starts_with(id(), &str_arg(a, 1), &str_arg(a, 2))),
        "compactStrings" => {
            ar.compact_strings();
            Val::Undefined
        }
        "compact" => ids(ar.compact(None)),
        "trim" => ids(ar.trim(None)),
        "mark" => {
            ar.mark(&a.iter().flat_map(|v| v.ids()).collect::<Vec<_>>());
            Val::Undefined
        }
        "isMarked" => Val::Bool(ar.is_marked(id())),
        "release" => {
            ar.release(if arg(a, 0).is_nullish() { None } else { Some(id()) }, arg(a, 1).truthy());
            Val::Undefined
        }
        "getErrors" => ar.get_errors(id()).map_or(Val::Null, |e| errors_val(&e)),
        "getState" => Val::Num(ar.get_state(id()) as f64),
        "setState" => {
            ar.set_state(id(), arg(a, 1).u32());
            Val::Undefined
        }
        "isInvalid" => Val::Bool(ar.is_invalid(id())),
        "setLocation" => {
            let o = |i: usize| if arg(a, i).is_nullish() { None } else { Some(arg(a, i).u32()) };
            ar.set_location(id(), o(1), o(2), o(3));
            Val::Undefined
        }
        "copyLocation" => {
            ar.copy_location(id(), id_arg(a, 1));
            Val::Undefined
        }
        "spanLocation" => {
            ar.span_location(id(), id_arg(a, 1), id_arg(a, 2));
            Val::Undefined
        }
        "hasLocation" => Val::Bool(ar.has_location(id())),
        "getLocation" => ar.get_location(id()).map_or(Val::Null, |l| location_val(&l)),
        "getSource" => Val::Num(ar.get_source(id()) as f64),
        "createStyleSheet" => Val::Num(ar.create_style_sheet() as f64),
        "createRule" => Val::Num(ar.create_rule(arg(a, 0).u32() as u8, StrOrId::Str(&str_arg(a, 1)), Some(&arg(a, 2).ids()), arg(a, 3).u32(), None) as f64),
        "createAtRule" => Val::Num(ar.create_at_rule(
            arg(a, 0).u32() as u8,
            StrOrId::Str(&str_arg(a, 1)),
            StrOrId::Str(&str_arg(a, 2)),
            arg(a, 3).truthy(),
            Some(&arg(a, 4).ids()),
            arg(a, 5).u32(),
            None,
        ) as f64),
        "createDeclaration" => Val::Num(ar.create_declaration(arg(a, 0).u32() as u8, StrOrId::Str(&str_arg(a, 1)), Some(&arg(a, 2).ids()), arg(a, 3).u32(), None) as f64),
        "createComment" => Val::Num(ar.create_comment(arg(a, 0).u32() as u8, StrOrId::Str(&str_arg(a, 1))) as f64),
        "createToken" => {
            let value = match arg(a, 1) {
                Val::Num(n) => TokenValue::Num(*n),
                _ => TokenValue::None,
            };
            let s = str_arg(a, 1);
            let value = match arg(a, 1) {
                Val::Str(_) => TokenValue::Str(&s),
                _ => value,
            };
            let unit = str_arg(a, 2);
            Val::Num(ar.create_token(arg(a, 0).u32() as u8, value, if arg(a, 2).is_nullish() { None } else { Some(&unit) }) as f64)
        }
        "createColor" => Val::Num(ar.create_color(&str_arg(a, 0), arg(a, 1).u32() as u8, Some(&arg(a, 2).ids())) as f64),
        "createFunction" => Val::Num(ar.create_function(arg(a, 0).u32() as u8, &str_arg(a, 1), Some(&arg(a, 2).ids())) as f64),
        "createList" => Val::Num(ar.create_list(arg(a, 0).u32() as u8, Some(&arg(a, 1).ids())) as f64),
        "createBinaryExpression" => Val::Num(ar.create_binary_expression(arg(a, 0).u32() as u8, id_arg(a, 1), id_arg(a, 2)) as f64),
        "createFraction" => Val::Num(ar.create_fraction(id_arg(a, 0), id_arg(a, 1)) as f64),
        "hasFraction" => Val::Bool(ar.has_fraction(id())),
        "getFraction" => Val::Num(ar.get_fraction(id()) as f64),
        "setStr" | "setValueString" => {
            let s = str_arg(a, 1);
            ar.string_value.set(id(), if arg(a, 1).is_nullish() { 0 } else { ar.intern_string(&s) });
            Val::Undefined
        }
        "setNameString" | "setSelector" => {
            let s = str_arg(a, 1);
            ar.name.set(id(), ar.intern_string(&s));
            if name == "setSelector" {
                ar.set_token_list(id(), &[]);
                ar.raw.borrow_mut().remove(&id());
                ar.optimized.borrow_mut().remove(&id());
            }
            Val::Undefined
        }
        "setUnitString" => {
            let s = str_arg(a, 1);
            ar.set_unit_string(id(), if arg(a, 1).is_nullish() { None } else { Some(&s) });
            Val::Undefined
        }
        "isNumeric" => Val::Bool(ar.is_numeric(id())),
        "take" => Val::Num(ar.take(id()) as f64),
        "copyToken" => {
            ar.copy_token(id(), id_arg(a, 1));
            Val::Undefined
        }
        "resetToken" => {
            let s = str_arg(a, 2);
            let value = match arg(a, 2) {
                Val::Num(n) => TokenValue::Num(*n),
                Val::Str(_) => TokenValue::Str(&s),
                _ => TokenValue::None,
            };
            let unit = str_arg(a, 3);
            ar.reset_token(id(), arg(a, 1).u32() as u8, value, if arg(a, 3).is_nullish() { None } else { Some(&unit) });
            Val::Undefined
        }
        "toObject" => Val::Node(arena_handle(ar), id()),
        "toPlainObject" => Val::from_plain(&ar.to_plain_object(id())),
        "fromObject" => match arg(a, 0) {
            Val::Node(handle, node) => Val::Num(foreign_node(ar, *handle, *node) as f64),
            value => Val::Num(ar.from_object(&value.to_plain()) as f64),
        },
        "cloneNode" => Val::Num(ar.clone_node(id()) as f64),
        "clone" => Val::Num(ar.clone_node_deep(id(), arg(a, 1).truthy(), None) as f64),
        "import" => {
            let source = arena_arg(a, 0)?;
            Val::Num(ar.import(&source, id_arg(a, 1), !matches!(arg(a, 2), Val::Bool(false))) as f64)
        }
        "hasChildren" => Val::Bool(ar.has_children(id())),
        "expand" => Val::Num(ar.expand(id()) as f64),
        "resize" => {
            ar.resize(arg(a, 0).u32() as usize);
            Val::Undefined
        }
        "inputLength" => ar.input_lengths.borrow().get(&id()).map_or(Val::Undefined, |&l| Val::Num(l as f64)),
        "nodeGet" => node_value(ar, AstArenaNode::new(ar, id()).get(&str_arg(a, 1))),
        "nodeSet" => {
            let key = str_arg(a, 1);
            Val::Bool(AstArenaNode::new(ar, id()).set(&key, node_input(ar, &key, arg(a, 2))))
        }
        "nodeJSON" => Val::from_plain(&AstArenaNode::new(ar, id()).to_json()),
        _ => return Err(HostError(format!("unknown arena method {}", name))),
    })
}

// ---- lazy walkers ---------------------------------------------------------------------------------------------------

use crate::ast::walk::{NodeWalker, ValueWalker, WalkerFilter, WalkerOptions, WalkerValueFilter, WalkerValueOptions, WalkerValueResult, WalkerValueType};

/// a walker driven by javascript (`arena.walk()`, `arena.walkValues()`): the children of the returned node are read
/// by the next call, like the typescript iterators
enum Walker {
    Node(NodeWalker<'static>),
    Value(ValueWalker<'static>),
}

thread_local! {
    static WALKERS: RefCell<Vec<Option<(Rc<AstArena>, Walker)>>> = const { RefCell::new(Vec::new()) };
}

fn add_walker(arena: &Rc<AstArena>, walker: Walker) -> Val {
    WALKERS.with(|w| {
        let mut walkers = w.borrow_mut();
        let entry = Some((arena.clone(), walker));
        match walkers.iter().position(|w| w.is_none()) {
            Some(i) => {
                walkers[i] = entry;
                Val::Num(i as f64 + 1.0)
            }
            None => {
                walkers.push(entry);
                Val::Num(walkers.len() as f64)
            }
        }
    })
}

/// a javascript walker option: a number or nothing
fn walker_option(value: &Val) -> Option<u32> {
    match value {
        Val::Num(n) => Some(*n as u32),
        _ => None,
    }
}

fn node_filter(arena: &Rc<AstArena>, callback: &Val) -> Option<WalkerFilter<'static>> {
    let Val::Fn(id, _) = callback else {
        return None;
    };
    let (id, arena) = (*id, arena.clone());
    Some(Box::new(move |_, node| super::call_js(id, Some(&arena), vec![Val::Num(node as f64)]).ok().and_then(|v| walker_option(&v))))
}

fn value_filter(arena: &Rc<AstArena>, callback: &Val) -> Option<WalkerValueFilter<'static>> {
    let Val::Fn(id, _) = callback else {
        return None;
    };
    let (id, arena) = (*id, arena.clone());
    Some(Box::new(move |_, token, event| match super::call_js(id, Some(&arena), vec![Val::Num(token as f64), Val::Num(event as f64)]) {
        Ok(Val::Num(n)) => WalkerValueResult::Option(n as u32),
        Ok(Val::Array(items)) => WalkerValueResult::Tokens(items.iter().flat_map(|v| v.ids()).collect()),
        _ => WalkerValueResult::None,
    }))
}

fn walk_start(arena: &Rc<AstArena>, a: &[Val]) -> Val {
    let node = id_arg(a, 0);
    let walker = match arg(a, 1) {
        Val::Object(_) => {
            let options = arg(a, 1);
            NodeWalker::with_options(
                node,
                WalkerOptions {
                    reverse: options.get("reverse").truthy() || arg(a, 2).truthy(),
                    children: if options.get("children").is_nullish() { None } else { Some(options.get("children").truthy()) },
                    attributes: options.get("attributes").truthy(),
                    filter: node_filter(arena, options.get("filter")),
                },
            )
        }
        filter => NodeWalker::new(node, node_filter(arena, filter), arg(a, 2).truthy()),
    };
    add_walker(arena, Walker::Node(walker))
}

fn walk_values_start(arena: &Rc<AstArena>, a: &[Val]) -> Val {
    let options = match arg(a, 1) {
        Val::Object(_) => {
            let o = arg(a, 1);
            let type_ = match o.get("type") {
                Val::Num(n) => Some(WalkerValueType::Type(*n as u8)),
                Val::Array(items) => Some(WalkerValueType::Types(items.iter().map(|v| v.u32() as u8).collect())),
                Val::Fn(id, _) => {
                    let (id, arena) = (*id, arena.clone());
                    Some(WalkerValueType::Fn(Box::new(move |_, token| {
                        super::call_js(id, Some(&arena), vec![Val::Num(token as f64)]).map(|v| v.truthy()).unwrap_or(false)
                    })))
                }
                _ => None,
            };
            WalkerValueOptions { event: walker_option(o.get("event")), fn_: value_filter(arena, o.get("fn")), type_ }
        }
        filter => WalkerValueOptions { event: None, fn_: value_filter(arena, filter), type_: None },
    };
    let reverse = arg(a, 2).truthy();
    let walker = match arg(a, 0) {
        Val::Array(items) => arena.walk_token_values(&items.iter().flat_map(|v| v.ids()).collect::<Vec<_>>(), options, reverse),
        _ => arena.walk_values(id_arg(a, 0), options, reverse),
    };
    add_walker(arena, Walker::Value(walker))
}

/// the next node of a walker, null at the end (the walker is released)
fn walker_next(handle: u32) -> Val {
    let index = (handle as usize).wrapping_sub(1);
    // the walker is taken out while it runs: the callbacks may start other walkers
    let Some((arena, mut walker)) = WALKERS.with(|w| w.borrow_mut().get_mut(index).and_then(|w| w.take())) else {
        return Val::Null;
    };
    let next = match &mut walker {
        Walker::Node(w) => w.next(&arena),
        Walker::Value(w) => w.next(&arena),
    };
    match next {
        Some(id) => {
            WALKERS.with(|w| w.borrow_mut()[index] = Some((arena, walker)));
            Val::Num(id as f64)
        }
        None => Val::Null,
    }
}

fn walker_free(handle: u32) {
    WALKERS.with(|w| {
        if let Some(slot) = w.borrow_mut().get_mut((handle as usize).wrapping_sub(1)) {
            *slot = None;
        }
    });
}

// ---- strings ---------------------------------------------------------------------------------------------------------

/// the text of a string without creating it (the cached strings are not modified)
fn string_text(arena: &AstArena, id: u32) -> Val {
    use crate::ast::strings::Units;
    arena.strings().with_units(id, |units| match units {
        Units::Bytes(b) => Val::Str(Rc::from(String::from_utf8_lossy(b).as_ref())),
        Units::Utf16(u) => Val::Str(Rc::from(String::from_utf16_lossy(u).as_str())),
    })
}

/// the code units of a javascript value: a string, utf-8 bytes (ascii) or utf-16 code units (a `Uint16Array`, sent as
/// bytes with `utf16` set)
fn code_units(value: &Val, utf16: bool) -> crate::parser::utils::codes::CodeUnits {
    use crate::parser::utils::codes::CodeUnits;
    match value {
        Val::Bytes(b) if utf16 => CodeUnits::from_utf16(&b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()),
        Val::Bytes(b) => match std::str::from_utf8(b) {
            Ok(s) if !s.is_ascii() => CodeUnits::from_str(s),
            _ => CodeUnits::Bytes(b.clone()),
        },
        other => CodeUnits::from_str(other.as_str().unwrap_or("")),
    }
}

fn string_flag(arena: &Rc<AstArena>, a: &[Val]) -> Val {
    use crate::ast::strings::StringFlag;
    let flag = match arg(a, 1).u32() {
        0 => StringFlag::Color,
        1 => StringFlag::SimpleSelector,
        _ => StringFlag::VendorPrefix,
    };
    let callback = match arg(a, 2) {
        Val::Fn(id, _) => Some(*id),
        _ => None,
    };
    Val::Bool(arena.string_flag(id_arg(a, 0), flag, |s| match callback {
        Some(id) => super::call_js(id, None, vec![Val::str(s)]).map(|v| v.truthy()).unwrap_or(false),
        None => false,
    }))
}

// ---- transfer ----------------------------------------------------------------------------------------------------------

fn le_bytes<T: Copy, const N: usize>(values: &[T], f: impl Fn(T) -> [u8; N]) -> Val {
    Val::Bytes(values.iter().flat_map(|&v| f(v)).collect::<Vec<u8>>().into())
}

fn from_le<T, const N: usize>(value: &Val, f: impl Fn([u8; N]) -> T) -> Vec<T> {
    match value {
        Val::Bytes(b) => b.chunks_exact(N).map(|c| f(c.try_into().unwrap())).collect(),
        _ => Vec::new(),
    }
}

/// the properties of an arena (`Arena.toTransferableArenaObject()`): structured-cloneable values
fn export_properties(arena: &AstArena, ast: bool) -> Val {
    use crate::parser::utils::codes::CodeUnits;
    let p = if ast { arena.export_properties() } else { arena.base().export_properties() };
    let s = &p.strings;
    let strings = Val::object(vec![
        ("count", Val::Num(s.count as f64)),
        ("hashes", le_bytes(&s.hashes, i32::to_le_bytes)),
        ("sources", le_bytes(&s.sources, i32::to_le_bytes)),
        ("starts", le_bytes(&s.starts, i32::to_le_bytes)),
        ("ends", le_bytes(&s.ends, i32::to_le_bytes)),
        (
            "texts",
            Val::Array(
                s.texts
                    .iter()
                    .map(|t| match t {
                        None => Val::Null,
                        Some(CodeUnits::Bytes(b)) => Val::object(vec![("utf16", Val::Bool(false)), ("codes", Val::Bytes(b.clone()))]),
                        Some(CodeUnits::Utf16(u)) => Val::object(vec![("utf16", Val::Bool(true)), ("codes", le_bytes(u, u16::to_le_bytes))]),
                    })
                    .collect(),
            ),
        ),
        ("pool", le_bytes(&s.pool, u16::to_le_bytes)),
        ("values", Val::Array(s.values.iter().map(|v| v.as_ref().map_or(Val::Undefined, |v| Val::Str(v.clone()))).collect())),
    ]);
    let mut entries = vec![
        ("kind", Val::str(&p.kind)),
        ("current", Val::Num(p.current as f64)),
        ("strings", strings),
        ("errors", Val::Array(p.errors.iter().map(|(id, e)| Val::Array(vec![Val::Num(*id as f64), errors_val(e)])).collect())),
    ];
    if let Some(sources) = &p.sources {
        entries.push(("sources", Val::Array(sources.iter().map(|&s| Val::Num(s as f64)).collect())));
    }
    if let Some(lengths) = &p.input_lengths {
        entries.push(("inputLengths", Val::Array(lengths.iter().map(|l| l.map_or(Val::Undefined, |l| Val::Num(l as f64))).collect())));
    }
    Val::object(entries)
}

/// the columns of an arena from javascript buffers, in the order of `arenaColumns`
fn transferables(columns: &Val) -> crate::ast::arena::ArenaTransferables {
    let column = |i: usize| match columns {
        Val::Array(items) => items.get(i).cloned().unwrap_or_default(),
        _ => Val::Undefined,
    };
    let bytes = |i: usize| match column(i) {
        Val::Bytes(b) => b.to_vec(),
        _ => Vec::new(),
    };
    let u32s = |i: usize| from_le(&column(i), u32::from_le_bytes);
    crate::ast::arena::ArenaTransferables {
        data_types: bytes(0),
        parents: u32s(1),
        next_sibling: u32s(2),
        previous_sibling: u32s(3),
        first_token: u32s(4),
        last_token: u32s(5),
        first_child: u32s(6),
        last_child: u32s(7),
        name: u32s(8),
        string_value: u32s(9),
        numeric_value: from_le(&column(10), f64::from_le_bytes),
        unit: u32s(11),
        flags: u32s(12),
        color_type: bytes(13),
        spans: from_le(&column(14), u64::from_le_bytes),
    }
}

/// initialize an arena from the properties and the column buffers of `toTransferableArenaObject()`
fn import_properties(arena: &AstArena, properties: &Val, columns: &Val) {
    use crate::ast::arena::ArenaProperties;
    use crate::ast::strings::StringTableData;
    let s = properties.get("strings");
    let i32s = |key: &str| from_le(s.get(key), i32::from_le_bytes);
    let strings = StringTableData {
        count: s.get("count").u32(),
        hashes: i32s("hashes"),
        sources: i32s("sources"),
        starts: i32s("starts"),
        ends: i32s("ends"),
        texts: match s.get("texts") {
            Val::Array(items) => items.iter().map(|t| if t.is_nullish() { None } else { Some(code_units(t.get("codes"), t.get("utf16").truthy())) }).collect(),
            _ => Vec::new(),
        },
        pool: from_le(s.get("pool"), u16::from_le_bytes),
        values: match s.get("values") {
            Val::Array(items) => items.iter().map(|v| if let Val::Str(s) = v { Some(s.clone()) } else { None }).collect(),
            _ => Vec::new(),
        },
    };
    let numbers = |key: &str| match properties.get(key) {
        Val::Array(items) => Some(items.iter().map(|v| if v.is_nullish() { None } else { Some(v.u32()) }).collect::<Vec<_>>()),
        _ => None,
    };
    let props = ArenaProperties {
        kind: properties.get("kind").as_str().unwrap_or("AstArena").to_string(),
        current: properties.get("current").u32(),
        strings,
        // the errors are plain objects: they are not restored
        errors: Vec::new(),
        sources: numbers("sources").map(|l| l.into_iter().map(|v| v.unwrap_or(0)).collect()),
        input_lengths: numbers("inputLengths"),
    };
    arena.import_properties(&props, transferables(columns));
}

fn source_map_find(json: &str, line: i64, column: i64) -> Val {
    let value = serde_json::from_str(json).unwrap_or(serde_json::Value::Null);
    let mut map = crate::printer::sourcemap::SourceMap::from_object(&crate::printer::sourcemap::SourceMapObject::from_json(&value));
    match map.find(line, column) {
        None => Val::Null,
        Some(list) => Val::Array(
            list.into_iter()
                .map(|(src, l, c, name)| {
                    Val::Array(vec![src.map_or(Val::Null, Val::Str), Val::Num(l as f64), Val::Num(c as f64), name.map_or(Val::Null, Val::Str)])
                })
                .collect(),
        ),
    }
}

// ---- parsed syntaxes ---------------------------------------------------------------------------------------------------

use crate::validation::parser::types::ValidationToken;

thread_local! {
    /// the parsed syntaxes given to javascript, by handle (1 based): javascript refers to their token lists by path
    static SYNTAXES: RefCell<Vec<Rc<Vec<Rc<ValidationToken>>>>> = const { RefCell::new(Vec::new()) };
}

/// `getParsedSyntax(group, name)`: `{ handle, tokens }`, null when there is no syntax
fn parsed_syntax(group: &str, name: &str) -> Val {
    use crate::validation::parser::typedef::ValidationSyntaxGroupEnum as G;
    let group = [G::Declarations, G::Functions, G::Syntaxes, G::Types, G::Selectors, G::AtRules, G::Units, G::Languages, G::MediaFeatures]
        .into_iter()
        .find(|g| g.as_str() == group)
        .unwrap_or(G::Declarations);
    let Some(syntax) = crate::validation::config::get_parsed_syntax(group, name) else {
        return Val::Null;
    };
    let handle = SYNTAXES.with(|s| {
        let mut list = s.borrow_mut();
        match list.iter().position(|x| Rc::ptr_eq(x, &syntax)) {
            Some(i) => i + 1,
            None => {
                list.push(syntax.clone());
                list.len()
            }
        }
    });
    Val::object(vec![("handle", Val::Num(handle as f64)), ("tokens", Val::Array(syntax.iter().map(|t| Val::from_plain(&t.to_plain())).collect()))])
}

/// a token list or a list of alternatives of a parsed syntax
enum SyntaxList<'a> {
    Tokens(&'a [Rc<ValidationToken>]),
    Lists(&'a [Vec<Rc<ValidationToken>>]),
}

/// resolve a path (`[index, field, index, field, ...]`, a field may be followed by an alternative index) from the root
/// list of a syntax
fn syntax_path<'a>(root: &'a [Rc<ValidationToken>], path: &[Val]) -> Option<SyntaxList<'a>> {
    let mut current = SyntaxList::Tokens(root);
    let mut i = 0;
    while i < path.len() {
        match (&current, &path[i]) {
            (SyntaxList::Tokens(tokens), Val::Num(n)) => {
                let token = tokens.get(*n as usize)?;
                let field = path.get(i + 1)?.as_str()?;
                current = match field {
                    "chi" => match (&token.chi_list, &token.chi) {
                        (Some(lists), _) => SyntaxList::Lists(lists),
                        (_, Some(chi)) => SyntaxList::Tokens(chi),
                        _ => return None,
                    },
                    "l" => SyntaxList::Tokens(token.l.as_deref()?),
                    "r" => SyntaxList::Tokens(token.r.as_deref()?),
                    "prelude" => SyntaxList::Tokens(token.prelude.as_deref()?),
                    _ => return None,
                };
                i += 2;
            }
            (SyntaxList::Lists(lists), Val::Num(n)) => {
                current = SyntaxList::Tokens(lists.get(*n as usize)?);
                i += 1;
            }
            _ => return None,
        }
    }
    Some(current)
}

/// run a function with the list of a syntax referenced by javascript (`{ handle, path }`)
fn with_syntax<R>(reference: &Val, f: impl FnOnce(Option<SyntaxList<'_>>) -> R) -> R {
    let syntax = SYNTAXES.with(|s| s.borrow().get((reference.get("handle").u32() as usize).wrapping_sub(1)).cloned());
    match syntax {
        None => f(None),
        Some(syntax) => {
            let path = match reference.get("path") {
                Val::Array(items) => items.clone(),
                _ => Vec::new(),
            };
            f(syntax_path(&syntax, &path))
        }
    }
}

fn opt_token(value: &Val) -> Option<u32> {
    if value.is_nullish() {
        None
    } else {
        Some(id_arg(std::slice::from_ref(value), 0))
    }
}
