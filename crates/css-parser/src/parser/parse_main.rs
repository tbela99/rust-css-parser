//! Port of `doParse()` / `doParseSync()` of `src/parser/parse.ts`.
//!
//! One `async fn` serves both, with every feature: `do_parse_sync()` runs it to completion (a host that returns its
//! values synchronously resolves the `@import` rules, the css modules, ...). The restrictions of the typescript
//! `doParseSync()` (no import resolution, errors instead of loading css modules, `generateSyncScopedName()`) are kept by
//! the javascript bindings only, with `ParserOptions::sync_api_compat`.

use crate::ast::arena::{AstArena, TokenField};
use crate::ast::features::mixin::evaluate_mixins;
use crate::ast::minify::{merge_rules, minify};
use crate::ast::types::{EnumAstNodeStatus, EnumToken, ModuleCaseTransformEnum as Case, ModuleScopeEnumOptions as Scope, WalkerEvent};
use crate::ast::walk::{NodeWalker, ValueWalker, WalkerValueOptions, WalkerValueResult};
use crate::fs::resolve::{dirname, resolve};
use crate::host::{resolve as await_host, Host, HostDef, HostError, HostValue};
use crate::ast::types::MergeRulesEnum;
use crate::options::{ModuleOption, ModuleScoped, ParserOptions, RemoveDuplicateDeclarations, SourceMapOption};
use crate::parser::input::CssInput;
use crate::parser::parse::{
    allocate_token, arena_capacity, drop_invalid_nodes, get_key_name, is_bad_token, is_definition_block, parse_node, parse_string,
    recycle_statement_tokens, render_tokens, ParseResultStats,
};
use crate::parser::source::SourceFile;
use crate::parser::tokenize::{create_parse_info, Tokenizer};
use crate::parser::utils::hash::{hash, sync_hash, HASH_ALGORITHMS};
use crate::parser::utils::text::{camelize, equals_ignore_case};
use crate::plain::{Plain, PlainObject};
use crate::printer::render::render_value_default;
use crate::syntax::constants::is_function_def_type;
use crate::syntax::syntax::is_ident_color;
use crate::types::{ErrorDescription, ResponseType};
use crate::utils::time::{format_ms, now};
use crate::validation::match_::{drop_error, match_selector_syntax, trim_array};
use std::cell::RefCell;
use crate::utils::fx::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

/// ordered string map (`Record<string, string>`)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StrMap {
    entries: Vec<(String, String)>,
    index: HashMap<String, usize>,
}

impl StrMap {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.index.get(key).map(|&i| self.entries[i].1.as_str())
    }

    pub fn has(&self, key: &str) -> bool {
        self.index.contains_key(key)
    }

    pub fn set(&mut self, key: &str, value: String) {
        match self.index.get(key) {
            Some(&i) => self.entries[i].1 = value,
            None => {
                self.index.insert(key.to_string(), self.entries.len());
                self.entries.push((key.to_string(), value));
            }
        }
    }

    pub fn entries(&self) -> &[(String, String)] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// `ParseResult`
#[derive(Clone)]
pub struct ParseResult {
    pub ast: u32,
    pub arena: Rc<AstArena>,
    pub errors: Vec<ErrorDescription>,
    pub stats: ParseResultStats,
    pub mapping: Option<StrMap>,
    pub css_module_variables: Option<Vec<(String, u32)>>,
    pub import_mapping: Option<Vec<(String, StrMap)>>,
    pub rev_mapping: Option<StrMap>,
}

/// parse css synchronously: [`do_parse()`] run to completion. every feature is available; a host callback that
/// returns a pending promise is an error
pub fn do_parse_sync(tokenizer: Tokenizer, options: ParserOptions, arena: Option<Rc<AstArena>>, host: Rc<dyn Host>) -> Result<ParseResult, HostError> {
    crate::host::block_on(do_parse(tokenizer, options, arena, host))
}

/// boxed future (recursive parses)
pub type ParseFuture = Pin<Box<dyn Future<Output = Result<ParseResult, HostError>>>>;

/// parse css (`doParse()`). `arena`: the arena of the parent parse (nested parses share it)
pub fn do_parse(tokenizer: Tokenizer, options: ParserOptions, arena: Option<Rc<AstArena>>, host: Rc<dyn Host>) -> ParseFuture {
    Box::pin(do_parse_impl(tokenizer, options, arena, host))
}

/// the default options of `doParse()`, then the options of the caller
fn with_defaults(options: ParserOptions) -> ParserOptions {
    let mut o = options;
    // the fast mode settings (`fastModeOptions`), an option given explicitly takes precedence
    if o.fast_mode == Some(true) {
        o.minify.get_or_insert(false);
        o.merge_rules.get_or_insert(MergeRulesEnum::Identical);
    }
    o.src.get_or_insert_with(String::new);
    o.sourcemap.get_or_insert(SourceMapOption::Off);
    o.minify.get_or_insert(true);
    o.pass.get_or_insert(1);
    o.expand_if_syntax.get_or_insert(false);
    o.parse_color.get_or_insert(true);
    o.nesting_rules.get_or_insert(true);
    o.resolve_import.get_or_insert(false);
    o.resolve_urls.get_or_insert(false);
    o.remove_charset.get_or_insert(true);
    o.remove_empty.get_or_insert(true);
    o.remove_duplicate_declarations.get_or_insert(RemoveDuplicateDeclarations::Bool(true));
    o.compute_transform.get_or_insert(true);
    o.compute_shorthand.get_or_insert(true);
    o.compute_calc_expression.get_or_insert(true);
    o.inline_css_variables.get_or_insert(false);
    o.evaluate_mixins.get_or_insert(false);
    o.keep_mixins.get_or_insert(false);
    o.set_parent.get_or_insert(true);
    o.remove_prefix.get_or_insert(false);
    o.validation.get_or_insert(false);
    o.lenient.get_or_insert(true);
    o
}

fn module_is_set(module: &Option<ModuleOption>) -> bool {
    match module {
        None => false,
        Some(ModuleOption::Bool(b)) => *b,
        Some(ModuleOption::Flags(f)) => *f != 0,
        Some(ModuleOption::Options(_)) => true,
    }
}

async fn do_parse_impl(tokenizer: Tokenizer, options: ParserOptions, arena: Option<Rc<AstArena>>, host: Rc<dyn Host>) -> Result<ParseResult, HostError> {
    let mut options = with_defaults(options);
    // the restrictions of the typescript sync api (bindings only)
    let is_sync = options.sync_api_compat;
    if !is_sync {
        options.minify_ast = options.minify_ast.or(options.minify);
    }
    if module_is_set(&options.module) {
        options.expand_nesting_rules = Some(true);
    }
    if options.expand_nesting_rules == Some(true) {
        options.nesting_rules = Some(false);
    }
    if is_sync {
        options.minify_ast = options.minify_ast.or(options.minify);
    } else if options.resolve_import == Some(true) {
        options.resolve_urls = Some(true);
    }
    let start_time = now();
    let mut errors: Vec<ErrorDescription> = Vec::new();
    let mut stack: Vec<u32> = Vec::new();
    let mut stats = ParseResultStats::new(options.src.as_deref().unwrap_or(""));
    let mut invalid_nodes: Vec<u32> = Vec::new();
    let mut error_start = 0usize;
    let mut invalid_start = 0usize;
    let mut statement: Vec<u32> = Vec::new();
    let mut tokenizer = tokenizer;
    let arena = arena.unwrap_or_else(|| AstArena::shared(arena_capacity(&tokenizer)));
    AstArena::register(&arena);
    let mut ast = if arena.current() == 1 { arena.root } else { arena.create_style_sheet() };
    let mut tokens: Vec<u32> = Vec::new();
    let mut context = ast;
    let mut imports: Vec<u32> = Vec::new();
    let mut parens_match = 0i32;
    let mut curly_bracket_match = 0i32;
    let mut parens_block_match = 0i32;
    arena.set_location(ast, Some(tokenizer.source_id()), Some(0), Some(0));
    let source_index = arena.source_index(tokenizer.source_id());
    arena.set_source_text(source_index as usize, tokenizer.codes().clone());
    let a = &*arena;
    let t = |x: u32| a.data_types.get(x);
    while tokenizer.typ != Some(EnumToken::EOF) {
        tokenizer.next();
        let mut item = allocate_token(a, &mut tokenizer, source_index as i32);
        let mut item_type = tokenizer.typ.unwrap_or(0);
        stats.bytes_in = tokenizer.bytes_in.unwrap_or(0) as u64;
        stats.tokens_count += 1;
        if is_bad_token(item_type) {
            tokens.push(item);
            statement.push(item);
            errors.push(drop_error(a, "Bad token".into(), item, &options));
            continue;
        }
        if item_type == EnumToken::StartParensTokenType || is_function_def_type(item_type) {
            parens_match += 1;
        } else if item_type == EnumToken::EndParensTokenType && parens_match > 0 {
            parens_match -= 1;
            if parens_match == 0 {
                parens_block_match = 0;
            }
        }
        let mut parens_block = false;
        if parens_match > 0 && item_type == EnumToken::BlockStartTokenType {
            parens_block_match += 1;
            parens_block = true;
        } else if parens_match > 0 && parens_block_match > 0 && item_type == EnumToken::BlockEndTokenType {
            parens_block_match -= 1;
            parens_block = true;
        } else if item_type == EnumToken::BlockStartTokenType {
            curly_bracket_match += 1;
        } else if item_type == EnumToken::BlockEndTokenType && curly_bracket_match > 0 {
            curly_bracket_match -= 1;
        }
        tokens.push(item);
        statement.push(item);
        if parens_match == 0
            && matches!(item_type, EnumToken::SemiColonTokenType | EnumToken::BlockStartTokenType | EnumToken::EOFTokenType)
        {
            let node = parse_node(a, &mut tokens, context, &options, &mut errors, &mut stats, &mut invalid_nodes, parens_match);
            if let Some(node) = node {
                if a.flags.get(node) & EnumAstNodeStatus::HasChildren != 0 {
                    stack.push(node);
                    context = node;
                } else if !is_sync && t(node) == EnumToken::AtRuleNodeType && a.string_equals(a.name.get(node), "import") {
                    imports.push(node);
                }
            } else if item_type == EnumToken::BlockStartTokenType {
                let mut in_block = 1;
                tokens.clear();
                tokens.push(item);
                loop {
                    tokenizer.next();
                    item = allocate_token(a, &mut tokenizer, source_index as i32);
                    item_type = tokenizer.typ.unwrap_or(0);
                    tokens.push(item);
                    statement.push(item);
                    if item_type == EnumToken::BlockStartTokenType {
                        in_block += 1;
                    } else if item_type == EnumToken::BlockEndTokenType {
                        in_block -= 1;
                    }
                    if !(in_block != 0 && tokenizer.typ != Some(EnumToken::EOF)) {
                        break;
                    }
                }
                if !tokens.is_empty() {
                    let mut error = ErrorDescription::drop("invalid block");
                    error.location = crate::validation::match_::token_location(a, tokens[0], &options);
                    errors.push(error);
                }
            }
            recycle_statement_tokens(a, &statement, &errors, error_start, &invalid_nodes, invalid_start);
            statement.clear();
            tokens.clear();
            error_start = errors.len();
            invalid_start = invalid_nodes.len();
        } else if !parens_block && (parens_match == 0 || curly_bracket_match == 0) && item_type == EnumToken::BlockEndTokenType {
            parse_node(a, &mut tokens, context, &options, &mut errors, &mut stats, &mut invalid_nodes, parens_match);
            a.set_span(context, a.get_source_id(context), a.get_span_start(context), a.get_span_end(item));
            let previous_node = stack.pop();
            context = stack.last().copied().unwrap_or(ast);
            if options.remove_empty == Some(true) {
                if let Some(previous) = previous_node {
                    if a.first_child.get(previous) == 0 && !is_definition_block(a, previous) && a.last_child.get(context) == previous {
                        a.detach(previous);
                    }
                }
            }
            recycle_statement_tokens(a, &statement, &errors, error_start, &invalid_nodes, invalid_start);
            statement.clear();
            tokens.clear();
            error_start = errors.len();
            invalid_start = invalid_nodes.len();
            parens_match = 0;
            parens_block_match = 0;
            curly_bracket_match = 0;
        }
    }
    if !tokens.is_empty() {
        if let Some(node) = parse_node(a, &mut tokens, context, &options, &mut errors, &mut stats, &mut invalid_nodes, parens_match) {
            // typescript compares the value (not the name) of the node with "import"
            if !is_sync && t(node) == EnumToken::AtRuleNodeType && a.string_equals(a.string_value.get(node), "import") {
                imports.push(node);
            }
            if a.flags.get(node) & EnumAstNodeStatus::HasChildren != 0 {
                stack.push(node);
            }
        }
    }
    if !imports.is_empty() && options.resolve_import == Some(true) {
        resolve_imports(&arena, &imports, &options, &host, &mut stats, &mut errors).await;
    }
    let end_parse_time = now();
    let minify_options = if options.evaluate_mixins == Some(true) {
        ParserOptions { evaluate_mixins: Some(false), ..options.clone() }
    } else {
        options.clone()
    };
    if options.evaluate_mixins == Some(true) {
        drop_invalid_nodes(a, &invalid_nodes, &options);
        evaluate_mixins(a, ast, &options);
    }
    if options.expand_nesting_rules == Some(true) {
        ast = a.expand(ast);
    }
    if let Some(visitor) = options.visitor.clone() {
        apply_visitors(&arena, ast, &visitor, &mut errors, &host, is_sync).await?;
    }
    drop_invalid_nodes(a, &invalid_nodes, &options);
    if options.minify_ast == Some(true) {
        if a.first_child.get(ast) != 0 {
            for _ in 0..options.pass.unwrap_or(1) {
                minify(a, ast, &minify_options, true, Some(&mut errors), false);
            }
        }
    } else if matches!(options.merge_rules, Some(m) if m != MergeRulesEnum::None) && a.first_child.get(ast) != 0 {
        // the ast is not minified: the adjacent rules are merged
        merge_rules(a, ast, &minify_options);
    }
    stats.bytes_in += stats.imported_bytes_in;
    a.release_source_codes(source_index as usize);
    a.input_lengths.borrow_mut().insert(ast, stats.bytes_in as u32);
    let mut end_time = now();
    stats.parse = format_ms(end_parse_time - start_time);
    stats.minify = format_ms(end_time - end_parse_time);
    stats.total = format_ms(end_time - start_time);
    let mut result = ParseResult {
        ast,
        arena: arena.clone(),
        errors,
        stats,
        mapping: None,
        css_module_variables: None,
        import_mapping: None,
        rev_mapping: None,
    };
    if module_is_set(&options.module) {
        let parse_module_time = now();
        process_module(&mut result, &options, &host, is_sync).await?;
        end_time = now();
        result.stats.module = Some(format_ms(end_time - parse_module_time));
        result.stats.total = format_ms(end_time - start_time);
    }
    Ok(result)
}

/// load a file with the host loader: the source file and the input
async fn load_source(host: &Rc<dyn Host>, options: &ParserOptions, absolute: &str, relative: &str, current: &str) -> Result<(Rc<SourceFile>, CssInput), HostError> {
    let load = options.load.ok_or_else(|| HostError("no loader".into()))?;
    let mut src = PlainObject::new();
    src.set("absolute", absolute);
    src.set("relative", relative);
    let result = await_host(host.call(load, None, vec![HostValue::Plain(Plain::Object(src)), HostValue::Str(Rc::from(current)), HostValue::Number(ResponseType::ArrayBuffer as f64)])).await?;
    let (input, text) = match result {
        HostValue::Str(s) => (CssInput::Str(s.clone()), s),
        HostValue::Bytes(b) => (CssInput::Bytes(b), Rc::from("")),
        other => return Err(HostError(format!("unexpected load result {:?}", other))),
    };
    let source = SourceFile::new(text, Vec::new(), Some(Rc::from(relative)));
    if let Some(map) = &options.sources_map {
        map.borrow_mut().insert(source.id, source.clone());
    }
    Ok((source, input))
}

/// resolve the `@import` rules: the imported stylesheets are parsed in the same arena (concurrently, like
/// `Promise.all()`)
async fn resolve_imports(arena: &Rc<AstArena>, imports: &[u32], options: &ParserOptions, host: &Rc<dyn Host>, stats: &mut ParseResultStats, errors: &mut Vec<ErrorDescription>) {
    let a = &**arena;
    let mut tasks: Vec<Pin<Box<dyn Future<Output = (u32, Result<ParseResult, HostError>)>>>> = Vec::new();
    for &node in imports {
        if a.flags.get(node) & EnumAstNodeStatus::StateMask != EnumAstNodeStatus::Validated {
            continue;
        }
        let token = a.first_token.get(node);
        let value = a.resolve_string(a.string_value.get(token));
        let url: String = if a.data_types.get(token) == EnumToken::StringTokenType {
            let chars: Vec<char> = value.chars().collect();
            if chars.len() >= 2 { chars[1..chars.len() - 1].iter().collect() } else { String::new() }
        } else {
            value.to_string()
        };
        let options = options.clone();
        let host = host.clone();
        let arena = arena.clone();
        tasks.push(Box::pin(async move {
            let base = match options.src.as_deref() {
                Some(s) if !s.is_empty() => dirname(s),
                _ => options.cwd.clone().unwrap_or_default(),
            };
            let src = resolve(&url, &base, None);
            let result = async {
                let (source, input) = load_source(&host, &options, &src.absolute, &src.relative, ".").await?;
                let parse_info = create_parse_info(Some(&input), source);
                let child_src = resolve(&url, options.src.as_deref().filter(|s| !s.is_empty()).or(options.cwd.as_deref()).unwrap_or(""), None).relative;
                let child_options = ParserOptions {
                    minify: Some(false),
                    set_parent: Some(false),
                    evaluate_mixins: Some(false),
                    src: Some(child_src),
                    source: None,
                    ..options.clone()
                };
                do_parse(Tokenizer::new(parse_info), child_options, Some(arena), host.clone()).await
            }
            .await;
            (node, result)
        }));
    }
    for (node, result) in join_all(tasks).await {
        match result {
            Ok(root) => {
                stats.imported_bytes_in += root.stats.bytes_in;
                stats.nodes_count += root.stats.nodes_count;
                stats.tokens_count += root.stats.tokens_count;
                stats.imports.push(root.stats.clone());
                if a.parents.get(node) != 0 {
                    a.move_children(root.ast, a.parents.get(node), node);
                    a.detach(node);
                }
                errors.extend(root.errors);
            }
            Err(error) => {
                let mut e = ErrorDescription::ignore(format!("doParse: {}", error.0));
                e.error = Some(error.0);
                errors.push(e);
            }
        }
    }
}

/// run futures concurrently: the results are returned in the completion order (`Promise.all()` callbacks)
async fn join_all<T>(mut futures: Vec<Pin<Box<dyn Future<Output = T>>>>) -> Vec<T> {
    let mut results: Vec<T> = Vec::new();
    std::future::poll_fn(move |cx| {
        let mut i = 0;
        while i < futures.len() {
            match futures[i].as_mut().poll(cx) {
                std::task::Poll::Ready(v) => {
                    results.push(v);
                    // the future is complete
                    drop(futures.remove(i));
                }
                std::task::Poll::Pending => i += 1,
            }
        }
        if futures.is_empty() {
            std::task::Poll::Ready(std::mem::take(&mut results))
        } else {
            std::task::Poll::Pending
        }
    })
    .await
}

// ---------------------------------------------------------------------------------------------------------------------
// visitors
// ---------------------------------------------------------------------------------------------------------------------

/// key of a handler map: a generic node name (`Declaration`, `Rule`, ...) or a token type
#[derive(Clone, Debug, PartialEq)]
enum MapKey {
    Name(&'static str),
    Token(u8),
}

type HandlerMap = Vec<(MapKey, Vec<HostDef>)>;

fn map_push(map: &mut HandlerMap, key: MapKey, value: HostDef) {
    match map.iter_mut().find(|(k, _)| *k == key) {
        Some((_, list)) => list.push(value),
        None => map.push((key, vec![value])),
    }
}

/// `ParsedVisitorHandlers`
struct ParsedVisitors {
    all_handlers: Vec<HandlerMap>,
    include_tokens: bool,
    all: Vec<HostDef>,
    pre_all: Vec<HostDef>,
    post_all: Vec<HostDef>,
}

/// `typeof value.type == "number" && (value.type & (WalkerEvent.Enter | WalkerEvent.Leave)) != 0`: the events are flags
fn walker_event(value: &HostDef) -> Option<u32> {
    match value {
        HostDef::Number(n) => {
            let n = *n as i64 as u32;
            (n & (WalkerEvent::Enter | WalkerEvent::Leave) != 0).then_some(n)
        }
        _ => None,
    }
}

fn set_add(set: &mut Vec<HostDef>, value: HostDef) {
    let id = |d: &HostDef| if let HostDef::Fn { id, .. } = d { Some(*id) } else { None };
    if !set.iter().any(|x| id(x).is_some() && id(x) == id(&value)) {
        set.push(value);
    }
}

const GENERIC_KEYS: [&str; 5] = ["Declaration", "Rule", "AtRule", "KeyframesRule", "KeyframesAtRule"];

fn parse_visitors(def: &HostDef, errors: &mut Vec<ErrorDescription>) -> ParsedVisitors {
    let mut visitors: Vec<(Rc<str>, HostDef)> = if def.is_fn() { vec![(Rc::from("0"), def.clone())] } else { def.entries() };
    let mut all: Vec<HostDef> = Vec::new();
    let mut pre_all: Vec<HostDef> = Vec::new();
    let mut post_all: Vec<HostDef> = Vec::new();
    let mut values: HandlerMap = Vec::new();
    let mut pre_values: HandlerMap = Vec::new();
    let mut post_values: HandlerMap = Vec::new();
    let mut visitors_map: HandlerMap = Vec::new();
    let mut pre_visitors_map: HandlerMap = Vec::new();
    let mut post_visitors_map: HandlerMap = Vec::new();
    let mut i = 0;
    while i < visitors.len() {
        let (mut key, value) = visitors[i].clone();
        i += 1;
        if key.parse::<f64>().is_ok_and(|n| n.fract() == 0.0) {
            match &value {
                HostDef::Fn { name, .. } => key = name.clone(),
                v if v.is_object() => {
                    visitors.extend(v.entries());
                    continue;
                }
                _ => {}
            }
        }
        let invalid = |errors: &mut Vec<ErrorDescription>| errors.push(ErrorDescription::ignore(format!("doParse: visitor.{} is not a valid key name", key)));
        if let Some(token) = EnumToken::value(&key) {
            match &value {
                HostDef::Fn { .. } => map_push(&mut values, MapKey::Token(token), value.clone()),
                v if v.is_object() || matches!(v, HostDef::Null) => {
                    let event = v.get("type").and_then(walker_event);
                    if let (Some(event), Some(handler)) = (event.filter(|_| v.get("handler").is_some()), v.get("handler")) {
                        if event & WalkerEvent::Enter != 0 {
                            map_push(&mut pre_values, MapKey::Token(token), handler.clone());
                        }
                        if event & WalkerEvent::Leave != 0 {
                            map_push(&mut post_values, MapKey::Token(token), handler.clone());
                        }
                    } else {
                        visitors.extend(v.entries());
                    }
                }
                _ => invalid(errors),
            }
        } else if let Some(&generic) = GENERIC_KEYS.iter().find(|k| **k == &*key) {
            match &value {
                HostDef::Fn { .. } => map_push(&mut visitors_map, MapKey::Name(generic), value.clone()),
                v if v.is_object() => {
                    let event = v.get("type").and_then(walker_event);
                    if let (Some(event), Some(handler)) = (event.filter(|_| v.get("handler").is_some()), v.get("handler")) {
                        if event & WalkerEvent::Enter != 0 {
                            map_push(&mut pre_visitors_map, MapKey::Name(generic), handler.clone());
                        }
                        if event & WalkerEvent::Leave != 0 {
                            map_push(&mut post_visitors_map, MapKey::Name(generic), handler.clone());
                        }
                    } else {
                        map_push(&mut visitors_map, MapKey::Name(generic), value.clone());
                    }
                }
                _ => invalid(errors),
            }
        } else if &*key == "All" || &*key == "visitor" {
            match &value {
                HostDef::Fn { .. } => set_add(&mut all, value.clone()),
                v if v.is_object() => {
                    let event = v.get("type").and_then(walker_event);
                    if let (Some(event), Some(handler)) = (event.filter(|_| v.get("handler").is_some()), v.get("handler")) {
                        if event & WalkerEvent::Enter != 0 {
                            set_add(&mut pre_all, handler.clone());
                        }
                        if event & WalkerEvent::Leave != 0 {
                            set_add(&mut post_all, handler.clone());
                        }
                    }
                }
                _ => {}
            }
        } else {
            invalid(errors);
        }
    }
    let include_tokens = !pre_values.is_empty() || !values.is_empty() || !post_values.is_empty() || !all.is_empty() || !pre_all.is_empty() || !post_all.is_empty();
    let all_handlers: Vec<HandlerMap> = [pre_visitors_map, pre_values, visitors_map, values, post_visitors_map, post_values]
        .into_iter()
        .filter(|m| !m.is_empty())
        .collect();
    ParsedVisitors { all_handlers, include_tokens, all, pre_all, post_all }
}

/// convert a visitor result to node ids
fn visitor_result(arena: &AstArena, host: &Rc<dyn Host>, replacement: &HostValue) -> Option<Vec<u32>> {
    let one = |v: &HostValue| -> Option<u32> {
        match v {
            HostValue::Number(n) => Some(*n as u32),
            HostValue::ForeignNode(handle, id) => {
                let other = host.arena(*handle)?;
                Some(arena.import(&other, *id, true))
            }
            HostValue::Plain(p) if p.as_object().is_some() => Some(arena.from_object(p)),
            _ => None,
        }
    };
    match replacement {
        HostValue::Array(items) => items.iter().map(|i| if matches!(i, HostValue::Number(_)) { one(i) } else { one(i) }).collect(),
        other => one(other).map(|id| vec![id]),
    }
}

async fn apply_visitors(arena: &Rc<AstArena>, ast: u32, visitor: &HostDef, errors: &mut Vec<ErrorDescription>, host: &Rc<dyn Host>, is_sync: bool) -> Result<(), HostError> {
    let a = &**arena;
    let visitors = parse_visitors(visitor, errors);
    let mut nodes: Vec<u32> = vec![ast];
    let enqueue = |node: u32, nodes: &mut Vec<u32>| {
        if a.flags.get(node) & EnumAstNodeStatus::HasChildren != 0 {
            let mut child = a.last_child.get(node);
            while child != 0 {
                nodes.push(child);
                child = a.previous_sibling.get(child);
            }
        }
        if visitors.include_tokens && !matches!(a.data_types.get(node), EnumToken::RuleNodeType | EnumToken::KeyframesRuleNodeType) {
            let values = a.sub_tokens(node);
            nodes.extend(values.iter().rev());
        }
    };
    while let Some(node) = nodes.pop() {
        let node_type = a.data_types.get(node);
        let mut handlers: Vec<u32> = Vec::new();
        let push_fn = |handlers: &mut Vec<u32>, d: &HostDef| {
            if let HostDef::Fn { id, .. } = d {
                handlers.push(*id);
            }
        };
        for h in &visitors.pre_all {
            push_fn(&mut handlers, h);
        }
        let generic_key: Option<&'static str> = match node_type {
            EnumToken::DeclarationNodeType => Some("Declaration"),
            EnumToken::AtRuleNodeType => Some("AtRule"),
            EnumToken::KeyframesAtRuleNodeType => Some("KeyframesAtRule"),
            EnumToken::KeyframesRuleNodeType => Some("KeyframesRule"),
            EnumToken::RuleNodeType => Some("Rule"),
            _ => None,
        };
        let key_name: Option<String> = match node_type {
            EnumToken::DeclarationNodeType | EnumToken::AtRuleNodeType => Some(camelize(&a.get_name(node))),
            EnumToken::KeyframesAtRuleNodeType => Some(camelize(&a.get_value(node))),
            _ => None,
        };
        for h in &visitors.all {
            push_fn(&mut handlers, h);
        }
        let keyed = |handler: &HostDef| key_name.as_deref().and_then(|k| handler.get(k)).filter(|h| h.is_fn()).cloned();
        for map in &visitors.all_handlers {
            if let Some(generic) = generic_key {
                if let Some((_, list)) = map.iter().find(|(k, _)| *k == MapKey::Name(generic)) {
                    for handler in list {
                        if handler.is_fn() {
                            push_fn(&mut handlers, handler);
                        } else if let Some(h) = keyed(handler) {
                            push_fn(&mut handlers, &h);
                        }
                    }
                }
            }
            if let Some((_, list)) = map.iter().find(|(k, _)| *k == MapKey::Token(node_type)) {
                for handler in list {
                    if handler.is_fn() {
                        push_fn(&mut handlers, handler);
                    } else if let HostDef::Array(items) = handler {
                        for h in items {
                            push_fn(&mut handlers, h);
                        }
                    } else if let Some(h) = handler.get("handler").filter(|h| h.is_fn()) {
                        push_fn(&mut handlers, h);
                    } else if let Some(h) = keyed(handler) {
                        push_fn(&mut handlers, &h);
                    }
                }
            }
        }
        for h in &visitors.post_all {
            push_fn(&mut handlers, h);
        }
        let mut replacement = HostValue::Null;
        let mut replaced: Option<Vec<u32>> = None;
        if !handlers.is_empty() {
            for callable in handlers {
                let result = host.call(callable, Some(arena), vec![HostValue::Number(node as f64)]);
                replacement = if is_sync { result? } else { await_host(result).await? };
                if replacement.is_nullish() {
                    continue;
                }
                if matches!(replacement, HostValue::Number(n) if n as u32 == node) {
                    continue;
                }
                if matches!(replacement, HostValue::Array(_)) {
                    break;
                }
            }
            let same = matches!(replacement, HostValue::Number(n) if n as u32 == node);
            if !replacement.is_nullish() && !same {
                match visitor_result(a, host, &replacement) {
                    Some(ids) if ids.len() == 1 && ids[0] == node && !matches!(replacement, HostValue::Array(_)) => {}
                    Some(ids) => {
                        if a.flags.get(node) & (EnumAstNodeStatus::HasChildren | EnumAstNodeStatus::HasValues) != 0 {
                            a.replace_child(node, &ids);
                        } else {
                            a.replace_token(node, &ids);
                        }
                        replaced = Some(ids);
                    }
                    None => {
                        errors.push(
                            ErrorDescription::ignore("doParse: visitor must return node ids, objects or arrays of node ids and objects").with_node(node),
                        );
                    }
                }
            }
        }
        match replaced {
            None => enqueue(node, &mut nodes),
            Some(ids) => {
                for &id in ids.iter().rev() {
                    enqueue(id, &mut nodes);
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------
// css modules
// ---------------------------------------------------------------------------------------------------------------------

thread_local! {
    static SCOPED_NAMES: RefCell<HashMap<(String, String, String, u32, bool), Result<String, String>>> = RefCell::new(HashMap::default());
    static SHORT_NAMES: RefCell<(u64, HashMap<(String, String, String, u32), String>)> = RefCell::new((0, HashMap::default()));
}

/// `getShortNameGenerator`: a short name per distinct arguments
fn short_name(local: &str, file_path: &str, pattern: &str, hash_length: u32) -> String {
    SHORT_NAMES.with(|s| {
        let mut s = s.borrow_mut();
        let key = (local.to_string(), file_path.to_string(), pattern.to_string(), hash_length);
        if let Some(v) = s.1.get(&key) {
            return v.clone();
        }
        let to36 = |mut n: u64| {
            if n == 0 {
                return "0".to_string();
            }
            let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
            let mut out = Vec::new();
            while n > 0 {
                out.push(digits[(n % 36) as usize]);
                n /= 36;
            }
            out.reverse();
            String::from_utf8(out).unwrap()
        };
        let mut value = to36(s.0);
        s.0 += 1;
        while value.as_bytes()[0].is_ascii_digit() {
            value = to36(s.0);
            s.0 += 1;
        }
        s.1.insert(key, value.clone());
        value
    })
}

/// `generateScopedName()` (async: the sha hashes are allowed) and `generateSyncScopedName()`
pub fn generate_scoped_name(local_name: &str, file_path: &str, pattern: &str, hash_length: u32, sync: bool) -> Result<String, String> {
    let key = (local_name.to_string(), file_path.to_string(), pattern.to_string(), hash_length, sync);
    if let Some(v) = SCOPED_NAMES.with(|c| c.borrow().get(&key).cloned()) {
        return v;
    }
    let result = generate_scoped_name_uncached(local_name, file_path, pattern, hash_length, sync);
    SCOPED_NAMES.with(|c| c.borrow_mut().insert(key, result.clone()));
    result
}

fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

fn generate_scoped_name_uncached(local_name: &str, file_path: &str, pattern: &str, hash_length: u32, sync: bool) -> Result<String, String> {
    let local_name = local_name.strip_prefix("--").unwrap_or(local_name);
    // `/.*?(([^/]+)\/)?([^/\\]*?)(\.([^?/]+))?([?].*)?$/`
    let (folder, file_base, ext) = split_file_path(file_path);
    let folder = sanitize(&folder);
    let path = sanitize(file_path);
    let safe_local = sanitize(local_name);
    let hash_string = format!("{}::{}", local_name, file_path);
    let mut result = String::new();
    let mut in_parens = 0;
    let mut key = String::new();
    let mut position = 0usize;
    for c in pattern.chars() {
        position += c.len_utf16();
        if c == '[' {
            in_parens += 1;
            if in_parens != 1 {
                return Err(format!("Unexpected character: '{} at position {}' in pattern '{}'", c, position - 1, pattern));
            }
            continue;
        }
        if c == ']' {
            in_parens -= 1;
            if in_parens != 0 {
                return Err(format!("Unexpected character: '{}:{}'", c, position - 1));
            }
            let mut hash_algo: Option<String> = None;
            let mut length: Option<String> = None;
            if key.contains(':') {
                let parts: Vec<String> = key.split(':').map(String::from).collect();
                if parts.len() == 2 {
                    key = parts[0].clone();
                    length = Some(parts[1].clone());
                    if key == "hash" && HASH_ALGORITHMS.contains(&parts[1].as_str()) {
                        hash_algo = length.take();
                        if sync && hash_algo.as_deref().is_some_and(|h| h.starts_with("sha")) {
                            return Err(format!(
                                "Unsupported hash algorithm: '{}'. Not supported by parseSync() or transformSync(). Use parse() or transform().",
                                hash_algo.unwrap()
                            ));
                        }
                    }
                }
                if parts.len() == 3 {
                    key = parts[0].clone();
                    hash_algo = Some(parts[1].clone());
                    length = Some(parts[2].clone());
                }
                if let Some(l) = &length {
                    let n = crate::utils::number::js_parse_number(l);
                    if !(n.is_finite() && n.fract() == 0.0) {
                        return Err(format!("Unsupported hash length: '{}'. expecting format [hash:length] or [hash:hash-algo:length]", l));
                    }
                }
            }
            let length_n: Option<usize> = length.as_deref().map(|l| crate::utils::number::js_parse_number(l) as usize);
            // `length !== fileBase.length` compares a string with a number: always true when a length is given
            let slice = length.is_some();
            let take = |s: &str| -> String {
                match length_n {
                    Some(n) if slice => s.chars().take(n).collect(),
                    _ => s.to_string(),
                }
            };
            match key.as_str() {
                "hash" => {
                    let len = length_n.unwrap_or(hash_length as usize);
                    let h = if sync { sync_hash(&hash_string, len, hash_algo.as_deref()) } else { hash(&hash_string, len, hash_algo.as_deref()) };
                    result.push_str(&h?);
                }
                "name" => result.push_str(&take(&file_base)),
                "local" => result.push_str(&if slice { take(&safe_local) } else { local_name.to_string() }),
                "ext" => result.push_str(&take(&ext)),
                "path" => result.push_str(&take(&path)),
                "folder" => result.push_str(&take(&folder)),
                _ => return Err(format!("Unsupported key: '{}'", key)),
            }
            key.clear();
            continue;
        }
        if in_parens > 0 {
            key.push(c);
        } else {
            result.push(c);
        }
    }
    Ok(format!("{}{}", if result.starts_with(|c: char| c.is_ascii_digit()) { "_" } else { "" }, result))
}

/// folder, file base and extension of a path (the groups 2, 3 and 5 of the typescript regular expression)
fn split_file_path(file_path: &str) -> (String, String, String) {
    // drop the query (`([?].*)?$`)
    let without_query = match file_path.find('?') {
        Some(i) => &file_path[..i],
        None => file_path,
    };
    let (dir, file) = match without_query.rfind('/') {
        Some(i) => (&without_query[..i], &without_query[i + 1..]),
        None => ("", without_query),
    };
    let folder = match dir.rfind('/') {
        Some(i) => &dir[i + 1..],
        None => dir,
    };
    // `([^/\\]*?)(\.([^?/]+))?`: the base stops at the first dot, the extension is the rest
    let (base, ext) = match file.find('.') {
        Some(i) if i + 1 < file.len() => (&file[..i], &file[i + 1..]),
        _ => (file, ""),
    };
    (folder.to_string(), base.to_string(), ext.to_string())
}

/// the scoped name generator of the module settings
#[derive(Clone)]
enum Generator {
    Default,
    Sync,
    Short,
    Host(u32),
}

struct ModuleSettings {
    hash_length: u32,
    file_path: String,
    scoped: u32,
    naming: u32,
    pattern: String,
    generator: Generator,
}

impl ModuleSettings {
    async fn scoped_name(&self, host: &Rc<dyn Host>, local: &str, file_path: &str) -> Result<String, HostError> {
        if self.scoped & Scope::Global != 0 {
            return Ok(local.to_string());
        }
        match self.generator {
            Generator::Default => Ok(generate_scoped_name(local, file_path, &self.pattern, self.hash_length, false)?),
            Generator::Sync => Ok(generate_scoped_name(local, file_path, &self.pattern, self.hash_length, true)?),
            Generator::Short => Ok(short_name(local, file_path, &self.pattern, self.hash_length)),
            Generator::Host(id) => {
                let args = vec![
                    HostValue::Str(Rc::from(local)),
                    HostValue::Str(Rc::from(file_path)),
                    HostValue::Str(Rc::from(self.pattern.as_str())),
                    HostValue::Number(self.hash_length as f64),
                ];
                match await_host(host.call(id, None, args)).await? {
                    HostValue::Str(s) => Ok(s.to_string()),
                    other => Ok(format!("{:?}", other)),
                }
            }
        }
    }

    /// `naming & (DashCaseOnly | CamelCaseOnly) ? getKeyName(value, naming) : value`
    fn name(&self, value: String) -> String {
        if self.naming & Case::DashCaseOnly != 0 || self.naming & Case::CamelCaseOnly != 0 {
            get_key_name(&value, self.naming)
        } else {
            value
        }
    }
}

fn unquote(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() >= 2 {
        chars[1..chars.len() - 1].iter().collect()
    } else {
        String::new()
    }
}

/// `[...new Set([...a.split(" "), ...b])].join(" ")`
fn merge_names(current: &str, more: &[String]) -> String {
    let mut out: Vec<String> = Vec::new();
    for n in current.split(' ').map(String::from).chain(more.iter().cloned()) {
        if !out.contains(&n) {
            out.push(n);
        }
    }
    out.join(" ")
}

const ANIMATION_KEYWORDS: [&str; 27] = [
    "none", "infinite", "normal", "reverse", "alternate", "alternate-reverse", "forwards", "backwards", "both", "running", "paused",
    "linear", "ease", "ease-in", "ease-out", "ease-in-out", "step-start", "step-end", "jump-start", "jump-end", "jump-none",
    "jump-both", "start", "end", "inherit", "initial", "unset",
];

const GRID_PROPERTIES: [&str; 9] = [
    "grid-column",
    "grid-column-start",
    "grid-column-end",
    "grid-row",
    "grid-row-start",
    "grid-row-end",
    "grid-template",
    "grid-template-columns",
    "grid-template-rows",
];

async fn process_module(result: &mut ParseResult, options: &ParserOptions, host: &Rc<dyn Host>, is_sync: bool) -> Result<(), HostError> {
    let arena = result.arena.clone();
    let a = &*arena;
    let t = |x: u32| a.data_types.get(x);
    let s = |id: u32| a.resolve_string(id).to_string();
    let module_options = match &options.module {
        Some(ModuleOption::Options(o)) => Some(o.clone()),
        _ => None,
    };
    let mut settings = ModuleSettings {
        hash_length: module_options.as_ref().and_then(|o| o.hash_length).unwrap_or(5),
        file_path: module_options.as_ref().and_then(|o| o.file_path.clone()).unwrap_or_default(),
        scoped: Scope::Local,
        naming: module_options.as_ref().and_then(|o| o.naming).unwrap_or(Case::IgnoreCase),
        pattern: module_options.as_ref().and_then(|o| o.pattern.clone()).unwrap_or_default(),
        generator: match module_options.as_ref().and_then(|o| o.generate_scoped_name) {
            Some(id) => Generator::Host(id),
            None => {
                if is_sync {
                    Generator::Sync
                } else {
                    Generator::Default
                }
            }
        },
    };
    if let Some(scoped) = module_options.as_ref().and_then(|o| o.scoped) {
        settings.scoped = match scoped {
            ModuleScoped::Bool(b) => {
                if b {
                    Scope::Local
                } else {
                    Scope::Global
                }
            }
            ModuleScoped::Flags(f) => f,
        };
    }
    let pattern: Option<String> = match &options.module {
        Some(ModuleOption::Bool(_)) => None,
        _ => Some(settings.pattern.clone()),
    };
    let src = options.src.clone().unwrap_or_default();
    let cwd = options.cwd.clone().unwrap_or_default();
    let mut file_path = match &options.module {
        Some(ModuleOption::Bool(_)) => src.clone(),
        _ => module_options.as_ref().and_then(|o| o.file_path.clone()).unwrap_or_else(|| src.clone()),
    };
    file_path = if file_path.is_empty() { resolve(&src, &cwd, None).relative } else { resolve(&file_path, &dirname(&src), Some(&cwd)).relative };
    if let Some(ModuleOption::Flags(m)) = &options.module {
        let m = *m;
        if m & Case::CamelCase != 0 {
            settings.naming = Case::CamelCase;
        } else if m & Case::CamelCaseOnly != 0 {
            settings.naming = Case::CamelCaseOnly;
        } else if m & Case::DashCase != 0 {
            settings.naming = Case::DashCase;
        } else if m & Case::DashCaseOnly != 0 {
            settings.naming = Case::DashCaseOnly;
        }
        if m & Scope::Global != 0 {
            settings.scoped = Scope::Global;
        }
        if m & Scope::Pure != 0 {
            settings.scoped |= Scope::Pure;
        }
        if m & Scope::Shortest != 0 {
            settings.scoped |= Scope::Shortest;
        }
        if m & Scope::ICSS != 0 {
            settings.scoped |= Scope::ICSS;
        }
    }
    if settings.scoped & Scope::Shortest != 0 {
        settings.generator = if is_sync { Generator::Sync } else { Generator::Short };
    }
    settings.file_path = file_path.clone();
    settings.pattern = match pattern.filter(|p| !p.is_empty()) {
        Some(p) => p,
        None => {
            if file_path.is_empty() {
                "[local]_[hash]".into()
            } else {
                "[local]_[hash]_[name]".into()
            }
        }
    };
    let mut global: crate::utils::fx::HashSet<u32> = std::collections::HashSet::default();
    let mut processed: crate::utils::fx::HashSet<u32> = std::collections::HashSet::default();
    let mut import_mapping: Vec<(String, StrMap)> = Vec::new();
    let mut css_variables_map: HashMap<String, Vec<(String, u32)>> = HashMap::default();
    let mut imported_css_variables: Vec<(String, u32)> = Vec::new();
    let mut mapping = StrMap::default();
    let mut rev_mapping = StrMap::default();
    let ast = result.ast;
    let mut walker = NodeWalker::new(ast, None, false);
    while let Some(node) = walker.next(a) {
        let parent = a.parents.get(node);
        let node_type = t(node);
        if node_type == EnumToken::CssVariableImportTokenType {
            if is_sync {
                let (line, column) = options.source.as_ref().map_or((0, 0), |s| s.line_starts.borrow().get_offsets(a.get_span_start(node)));
                let file = options.source.as_ref().and_then(|s| s.file.clone()).map_or(String::new(), |f| f.to_string());
                return Err(HostError(format!(
                    "css variable import not supported by parseSync() or transformSync(). use parse() or transform() instead.\nat {}:{}:{}",
                    file, line, column
                )));
            }
            let string_token = a.find_token(node, |ar, x| ar.data_types.get(x) == EnumToken::StringTokenType).unwrap_or(0);
            let url = unquote(&s(a.string_value.get(string_token)));
            let resolved = resolve(&url, &dirname(&src), Some(&cwd));
            let (source, input) = load_source(host, options, &resolved.absolute, &resolved.relative, "").await?;
            let parse_info = create_parse_info(Some(&input), source.clone());
            let child_options = ParserOptions {
                source: Some(source),
                minify: Some(false),
                set_parent: Some(false),
                evaluate_mixins: Some(false),
                src: Some(resolved.relative.clone()),
                ..options.clone()
            };
            let root = do_parse(Tokenizer::new(parse_info), child_options, Some(arena.clone()), host.clone()).await?;
            css_variables_map.insert(s(a.name.get(node)), root.css_module_variables.unwrap_or_default());
            a.detach(node);
            continue;
        }
        if node_type == EnumToken::CssVariableDeclarationMapTokenType {
            let from_token = a.field_tokens(node, TokenField::L).into_iter().find(|&x| t(x) == EnumToken::IdenTokenType || is_ident_color(a, x)).unwrap_or(0);
            let from = s(a.string_value.get(from_token));
            match css_variables_map.get(&from) {
                None => {
                    result.errors.push(ErrorDescription::drop(format!("could not resolve @value import from '{}'", from)).with_node(node));
                }
                Some(vars) => {
                    for token in a.token_iter(node) {
                        if t(token) == EnumToken::IdenTokenType || is_ident_color(a, token) {
                            let name = s(a.string_value.get(token));
                            match vars.iter().find(|(k, _)| *k == name) {
                                None => {
                                    result.errors.push(ErrorDescription::drop(format!("value '{}' is not exported from '{}'", name, from)).with_node(node));
                                }
                                Some((_, v)) => {
                                    let v = *v;
                                    let list = result.css_module_variables.get_or_insert_with(Vec::new);
                                    set_pair(list, &name, v);
                                    set_pair(&mut imported_css_variables, &name, v);
                                }
                            }
                        }
                    }
                }
            }
            a.detach(node);
            continue;
        }
        if node_type == EnumToken::CssVariableTokenType {
            if parent != 0 && t(parent) == EnumToken::StyleSheetNodeType {
                let list = result.css_module_variables.get_or_insert_with(Vec::new);
                set_pair(list, &s(a.name.get(node)), node);
            }
            a.detach(node);
            continue;
        }
        if node_type == EnumToken::DeclarationNodeType {
            let name = s(a.name.get(node));
            if name.starts_with("--") {
                let value = settings.scoped_name(host, &name, &settings.file_path).await?;
                mapping.set(&name, format!("--{}", settings.name(value)));
                rev_mapping.set(&name, name.clone());
                a.name.set(node, a.intern_string(mapping.get(&name).unwrap()));
            }
            let name = s(a.name.get(node));
            if equals_ignore_case("composes", &name) {
                let compose_selectors: Vec<u32> = a.token_iter(node).filter(|&x| t(x) == EnumToken::ComposesSelectorNodeType).collect();
                let mut parent_rule = parent;
                while parent_rule != 0 && t(parent_rule) != EnumToken::RuleNodeType {
                    parent_rule = a.parents.get(parent_rule);
                }
                if compose_selectors.is_empty() {
                    result.errors.push(ErrorDescription::drop("composes is empty").with_node(node));
                    a.detach(node);
                    continue;
                }
                let resolved_src = resolve(&src, &cwd, None);
                for token in compose_selectors {
                    if !a.has_field(token, TokenField::R) {
                        for rule in a.field_tokens(token, TokenField::L) {
                            if matches!(t(rule), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType) {
                                continue;
                            }
                            let rv = s(a.string_value.get(rule));
                            if !mapping.has(&rv) {
                                let value = settings.scoped_name(host, &rv, &settings.file_path).await?;
                                let prefix = if t(rule) == EnumToken::DashedIdenTokenType { "--" } else { "" };
                                mapping.set(&rv, format!("{}{}", prefix, settings.name(value)));
                                let mapped = mapping.get(&rv).unwrap().to_string();
                                rev_mapping.set(&mapped, rv.clone());
                            }
                            if parent_rule != 0 {
                                for tk in a.token_iter(parent_rule) {
                                    if t(tk) == EnumToken::ClassSelectorTokenType {
                                        let val: String = s(a.string_value.get(tk)).chars().skip(1).collect();
                                        if let Some(key) = rev_mapping.get(&val).map(String::from) {
                                            let merged = merge_names(mapping.get(&key).unwrap_or(""), &[mapping.get(&rv).unwrap_or("").to_string()]);
                                            mapping.set(&key, merged);
                                        }
                                    }
                                }
                            }
                        }
                    } else if t(a.field(token, TokenField::R)) == EnumToken::StringTokenType {
                        if is_sync {
                            let (line, column) = options.source.as_ref().map_or((0, 0), |s| s.line_starts.borrow().get_offsets(a.get_span_start(node)));
                            let file = options.source.as_ref().and_then(|s| s.file.clone()).map_or(String::new(), |f| f.to_string());
                            return Err(HostError(format!(
                                "composes from file is not supported using parseSync() or transformSync(). Use parse() or transform() instead.\nat {}:{}:{}",
                                file, line, column
                            )));
                        }
                        let url = unquote(&s(a.string_value.get(a.field(token, TokenField::R))));
                        let resolved = resolve(&url, &dirname(&src), Some(&cwd));
                        let (source, input) = load_source(host, options, &resolved.absolute, &resolved.relative, "").await?;
                        let child_source = SourceFile::new(Rc::from(""), Vec::new(), Some(Rc::from(resolved.relative.as_str())));
                        let _ = source;
                        let parse_info = create_parse_info(Some(&input), child_source);
                        let child_options = ParserOptions {
                            minify: Some(false),
                            set_parent: Some(false),
                            evaluate_mixins: Some(false),
                            src: Some(resolved.relative.clone()),
                            ..options.clone()
                        };
                        let mut root = do_parse(Tokenizer::new(parse_info), child_options, Some(arena.clone()), host.clone()).await?;
                        let mut src_index = resolve(&resolved.absolute, &resolved_src.absolute, None).relative;
                        if !src_index.starts_with('/') && !src_index.starts_with("../") {
                            src_index = format!("./{}", src_index);
                        }
                        let mut root_mapping = root.mapping.take().unwrap_or_default();
                        let mut root_rev = root.rev_mapping.take().unwrap_or_default();
                        if !root_mapping.is_empty() && !import_mapping.iter().any(|(k, _)| *k == src_index) {
                            import_mapping.push((src_index.clone(), StrMap::default()));
                        } else if !root_mapping.is_empty() {
                            if let Some(entry) = import_mapping.iter_mut().find(|(k, _)| *k == src_index) {
                                entry.1 = StrMap::default();
                            }
                        }
                        if parent_rule != 0 {
                            for tk in a.token_iter(parent_rule) {
                                if t(tk) == EnumToken::ClassSelectorTokenType {
                                    let val: String = s(a.string_value.get(tk)).chars().skip(1).collect();
                                    if let Some(key) = rev_mapping.get(&val).map(String::from) {
                                        let mut values: Vec<String> = Vec::new();
                                        for iden in a.field_tokens(token, TokenField::L) {
                                            if !matches!(t(iden), EnumToken::IdenTokenType | EnumToken::DashedIdenTokenType) {
                                                continue;
                                            }
                                            let iv = s(a.string_value.get(iden));
                                            if !root_mapping.has(&iv) {
                                                let value = settings.scoped_name(host, &iv, &src_index).await?;
                                                root_mapping.set(&iv, settings.name(value));
                                                let mapped = root_mapping.get(&iv).unwrap().to_string();
                                                root_rev.set(&mapped, iv.clone());
                                            }
                                            let mapped = root_mapping.get(&iv).unwrap().to_string();
                                            match import_mapping.iter_mut().find(|(k, _)| *k == src_index) {
                                                Some(entry) => entry.1.set(&iv, mapped.clone()),
                                                // typescript: `importMapping[srcIndex][...]` throws when the entry is missing
                                                None => return Err(HostError("Cannot set properties of undefined".into())),
                                            }
                                            values.push(mapped);
                                        }
                                        let merged = merge_names(mapping.get(&key).unwrap_or(""), &values);
                                        mapping.set(&key, merged);
                                    }
                                }
                            }
                        }
                    } else if t(a.field(token, TokenField::R)) == EnumToken::IdenTokenType && parent_rule != 0 {
                        let from = a.field(token, TokenField::R);
                        if equals_ignore_case("global", &s(a.string_value.get(from))) {
                            for tk in a.token_iter(parent_rule) {
                                if t(tk) == EnumToken::ClassSelectorTokenType {
                                    let val: String = s(a.string_value.get(tk)).chars().skip(1).collect();
                                    if let Some(key) = rev_mapping.get(&val).map(String::from) {
                                        let composed: Vec<String> = a
                                            .field_tokens(token, TokenField::L)
                                            .into_iter()
                                            .filter(|&c| t(c) == EnumToken::IdenTokenType)
                                            .map(|c| s(a.string_value.get(c)))
                                            .collect();
                                        let merged = merge_names(mapping.get(&key).unwrap_or(""), &composed);
                                        mapping.set(&key, merged);
                                    }
                                }
                            }
                        } else {
                            result.errors.push(ErrorDescription::drop(format!("composes '{}' is not supported", s(a.string_value.get(from)))).with_node(node));
                        }
                    }
                }
                a.detach(node);
            }
            let name = s(a.name.get(node));
            if GRID_PROPERTIES.contains(&name.as_str()) {
                let mut values = ValueWalker::of_node(a, node, WalkerValueOptions::default(), false);
                while let Some(value) = values.next(a) {
                    if t(value) != EnumToken::IdenTokenType {
                        continue;
                    }
                    let mut iden = s(a.string_value.get(value));
                    let mut suffix = "";
                    if let Some(rest) = iden.strip_suffix("-start") {
                        suffix = "-start";
                        iden = rest.to_string();
                    } else if let Some(rest) = iden.strip_suffix("-end") {
                        suffix = "-end";
                        iden = rest.to_string();
                    }
                    if !mapping.has(&iden) {
                        let generated = settings.scoped_name(host, &iden, &settings.file_path).await?;
                        mapping.set(&iden, generated.clone());
                        rev_mapping.set(&generated, iden.clone());
                        if !suffix.is_empty() {
                            iden.push_str(suffix);
                            if !mapping.has(&iden) {
                                mapping.set(&iden, format!("{}{}", generated, suffix));
                                rev_mapping.set(&format!("{}{}", generated, suffix), iden.clone());
                            }
                        }
                    }
                    a.string_value.set(value, a.intern_string(mapping.get(&iden).unwrap_or("undefined")));
                }
            } else if name == "grid-template-areas" || name == "grid-template" {
                for token in a.token_iter(node) {
                    if t(token) == EnumToken::StringTokenType {
                        let str = s(a.string_value.get(token));
                        let tokens = parse_string(a, &unquote(&str));
                        let mut values = ValueWalker::of_tokens(&tokens, WalkerValueOptions::default(), false);
                        while let Some(value) = values.next(a) {
                            if matches!(t(value), EnumToken::IdenTokenType | EnumToken::DashedIdenTokenType) {
                                let v = s(a.string_value.get(value));
                                if let Some(m) = mapping.get(&v).map(String::from) {
                                    a.string_value.set(value, a.intern_string(&m));
                                } else {
                                    let generated = settings.scoped_name(host, &v, &settings.file_path).await?;
                                    mapping.set(&v, generated.clone());
                                    rev_mapping.set(&generated, v.clone());
                                    a.string_value.set(value, a.intern_string(&generated));
                                }
                            }
                        }
                        let first = str.chars().next().map(String::from).unwrap_or_default();
                        let last = str.chars().last().map(String::from).unwrap_or_default();
                        let rendered = format!("{}{}{}", first, render_tokens(a, &tokens, None), last);
                        a.string_value.set(token, a.intern_string(&rendered));
                    }
                }
            } else if name == "animation" || name == "animation-name" {
                let mut values = ValueWalker::of_node(a, node, WalkerValueOptions::default(), false);
                while let Some(value) = values.next(a) {
                    let v = s(a.string_value.get(value));
                    if t(value) == EnumToken::IdenTokenType && !ANIMATION_KEYWORDS.contains(&v.as_str()) {
                        if !mapping.has(&v) {
                            let generated = settings.scoped_name(host, &v, &settings.file_path).await?;
                            mapping.set(&v, generated.clone());
                            rev_mapping.set(&generated, v.clone());
                        }
                        a.string_value.set(value, a.intern_string(mapping.get(&v).unwrap()));
                    }
                }
            }
            let mut values = ValueWalker::of_node(a, node, WalkerValueOptions::default(), false);
            while let Some(value) = values.next(a) {
                if t(value) == EnumToken::DashedIdenTokenType {
                    if let Some(mapped) = mapping.get(&s(a.string_value.get(value))).map(String::from) {
                        a.string_value.set(value, a.intern_string(&mapped));
                    }
                } else if t(value) == EnumToken::IdenTokenType || is_ident_color(a, value) {
                    let v = s(a.string_value.get(value));
                    if let Some(&(_, var)) = imported_css_variables.iter().find(|(k, _)| *k == v) {
                        let copies: Vec<u32> = a.tokens(var).into_iter().map(|tk| a.clone_node_deep(tk, true, None)).collect();
                        a.replace_token(value, &copies);
                    }
                }
            }
        } else if node_type == EnumToken::RuleNodeType {
            let mut tokens = parse_string(a, &s(a.name.get(node)));
            let mut ignored = Vec::new();
            match_selector_syntax(a, &mut tokens, &mut ignored, options, true);
            trim_array(a, &mut tokens);
            a.set_token_list(node, &tokens);
            let mut has_id_or_class = false;
            let mut global_tokens: Vec<u32> = Vec::new();
            let mut values = ValueWalker::of_node(
                a,
                node,
                WalkerValueOptions {
                    fn_: Some(Box::new(|ar, value, _| {
                        let tv = ar.data_types.get(value);
                        if tv == EnumToken::PseudoClassTokenType || tv == EnumToken::PseudoElementTokenType {
                            let val = ar.resolve_string(ar.string_value.get(value)).to_lowercase();
                            if val == ":local" || val == ":global" {
                                let next = ar.next_sibling.get(value);
                                ar.detach(value);
                                if next != 0 && matches!(ar.data_types.get(next), EnumToken::WhitespaceTokenType | EnumToken::DescendantCombinatorTokenType) {
                                    ar.detach(next);
                                }
                            }
                        } else if tv == EnumToken::PseudoClassFuncTokenType {
                            let val = ar.resolve_string(ar.string_value.get(value)).to_lowercase();
                            if val == ":global" || val == ":local" {
                                let children = ar.tokens(value);
                                if val == ":global" {
                                    global_tokens.extend(children.iter().copied());
                                }
                                a.replace_child(value, &children);
                                return WalkerValueResult::Tokens(children);
                            }
                        }
                        WalkerValueResult::None
                    })),
                    ..Default::default()
                },
                false,
            );
            let mut visited: Vec<u32> = Vec::new();
            while let Some(value) = values.next(a) {
                visited.push(value);
            }
            drop(values);
            global.extend(global_tokens);
            for value in visited {
                if matches!(t(value), EnumToken::HashTokenType | EnumToken::ClassSelectorTokenType) {
                    has_id_or_class = true;
                }
                if !processed.insert(value) {
                    continue;
                }
                if matches!(t(value), EnumToken::PseudoClassTokenType | EnumToken::PseudoElementTokenType | EnumToken::PseudoClassFuncTokenType) {
                    continue;
                }
                if global.contains(&value) {
                    continue;
                }
                if t(value) == EnumToken::ClassSelectorTokenType {
                    let val: String = s(a.string_value.get(value)).chars().skip(1).collect();
                    if !mapping.has(&val) {
                        let generated = settings.scoped_name(host, &val, &settings.file_path).await?;
                        mapping.set(&val, settings.name(generated));
                        let mapped = mapping.get(&val).unwrap().to_string();
                        rev_mapping.set(&mapped, val.clone());
                    }
                    a.string_value.set(value, a.intern_string(&format!(".{}", mapping.get(&val).unwrap())));
                }
            }
            if settings.scoped & Scope::Pure != 0 && !has_id_or_class {
                let (line, column) = options.source.as_ref().map_or((0, 0), |s| s.line_starts.borrow().get_offsets(a.get_span_start(node)));
                return Err(HostError(format!("pure module: No id or class found in selector '{}' at '{}:{}'", s(a.name.get(node)), line, column)));
            }
            let selector: String = a.tokens(node).iter().map(|&tk| render_value_default(a, tk)).collect();
            a.name.set(node, a.intern_string(&selector));
        } else if node_type == EnumToken::AtRuleNodeType || node_type == EnumToken::KeyframesAtRuleNodeType {
            let val = s(a.name.get(node)).to_lowercase();
            if a.first_token.get(node) == 0 {
                let tokens = parse_string(a, &s(a.string_value.get(node)));
                a.set_token_list(node, &tokens);
            }
            if val == "property" || val == "keyframes" {
                let prefix = if val == "property" { "--" } else { "" };
                for value in a.tokens(node) {
                    let matches_type = (prefix == "--" && t(value) == EnumToken::DashedIdenTokenType) || (prefix.is_empty() && t(value) == EnumToken::IdenTokenType);
                    if matches_type {
                        let v = s(a.string_value.get(value));
                        if !mapping.has(&v) {
                            let generated = settings.scoped_name(host, &v, &settings.file_path).await?;
                            mapping.set(&v, format!("{}{}", prefix, settings.name(generated)));
                            let mapped = mapping.get(&v).unwrap().to_string();
                            rev_mapping.set(&mapped, v.clone());
                        }
                        a.string_value.set(value, a.intern_string(mapping.get(&v).unwrap()));
                    }
                }
                let rendered = render_tokens(a, &a.tokens(node), None);
                a.string_value.set(node, a.intern_string(&rendered));
            }
        }
    }
    if settings.naming & Case::IgnoreCase == 0 {
        if is_sync {
            // typescript (`doParseSync()`): the mapping is reset before it is iterated
            rev_mapping = StrMap::default();
            mapping = StrMap::default();
        } else {
            let mut renamed = StrMap::default();
            rev_mapping = StrMap::default();
            for (key, value) in mapping.entries() {
                let key_name = get_key_name(key, settings.naming);
                renamed.set(&key_name, value.clone());
                rev_mapping.set(value, key_name);
            }
            mapping = renamed;
        }
    }
    result.mapping = Some(mapping);
    result.rev_mapping = Some(rev_mapping);
    if settings.scoped & Scope::ICSS != 0 && !import_mapping.is_empty() {
        result.import_mapping = Some(import_mapping);
    }
    Ok(())
}

fn set_pair(list: &mut Vec<(String, u32)>, key: &str, value: u32) {
    match list.iter_mut().find(|(k, _)| k == key) {
        Some(entry) => entry.1 = value,
        None => list.push((key.to_string(), value)),
    }
}
