//! Port of `src/printer/render.ts`.
//!
//! The rendered css is written to a utf-8 [`Writer`]. The typescript renderer memoizes identical declarations and
//! constructs (`RenderMemo`): the rendering only depends on the token structure, the port renders them each time
//! (same output).

use crate::ast::arena::{AstArena, TokenField, TokenValue};
use crate::ast::types::{ColorType, EnumAstNodeStatus, EnumToken};
use crate::fs::resolve::{dirname, resolve};
use crate::options::{ConvertColor, ParserOptions, SourceMapOption};
use crate::parser::linesmap::LineMap;
use crate::parser::source::SourceFile;
use crate::parser::utils::angle::to_degrees;
use crate::parser::utils::text::equals_ignore_case;
use crate::printer::result::RenderResult;
use crate::printer::sourcemap::SourceMap;
use crate::printer::writer::{Writer, DEFAULT_CAPACITY};
use crate::syntax::color::color::{convert_color, get_angle};
use crate::syntax::color::hex::reduce_hex_value;
use crate::syntax::constants::{anglePrecision, is_function_type, url_token_matcher, PSEUDO_ELEMENTS};
use crate::syntax::syntax::{
    is_white_space, minify_number, minify_number_str, parse_color, reduce_color_stops, reduce_conic_color_stops,
    reducegradient_background_position, to_precision_angle, to_precision_value,
};
use crate::types::{ErrorDescription, SourceLocation};
use crate::utils::number::{js_number_to_string, js_parse_number, to_fixed};
use crate::utils::time::{format_ms, now};
use crate::ast::cache::StructuralMemo;
use std::cell::RefCell;
use crate::utils::fx::HashMap;
use std::rc::Rc;

/// render options (`RenderOptions`): the parser options hold the render options
pub type RenderOptions = ParserOptions;

/// css modules mapping of a render (`:export` / `:import()`)
#[derive(Clone, Debug, Default)]
pub struct RenderMapping {
    pub mapping: Vec<(String, String)>,
    pub import_mapping: Option<Vec<(String, Vec<(String, String)>)>>,
}

/// rendered strings shared by the renderer (`RenderMemo`): identical declarations and identical constructs (colors,
/// functions, parentheses, binary expressions, ...) are rendered once. a memo is only shared by renders that use the
/// same options. the source map locations are not memoized: they are computed from the rendered string
#[derive(Default)]
pub struct RenderMemo {
    /// rendered declarations (`name:value`), by name and value tokens
    pub declarations: StructuralMemo<Rc<str>>,
    /// rendered constructs: colors and tokens with sub tokens
    pub constructs: StructuralMemo<Rc<str>>,
    /// location of the declarations in the render buffer, by declaration slot: writer id, start, end, extra
    locations: Vec<(u32, usize, usize, i64)>,
    /// rendered values of the declarations (without the name), by declaration slot
    values: Vec<Option<Rc<str>>>,
}

thread_local! {
    /// cleared memos reused by the next renders
    static FREE_MEMOS: RefCell<Vec<RenderMemo>> = const { RefCell::new(Vec::new()) };
}

impl RenderMemo {
    /// a cleared memo
    pub fn acquire() -> Self {
        FREE_MEMOS.with(|f| f.borrow_mut().pop()).unwrap_or_default()
    }

    /// clear a memo that is no longer used and keep it for the next renders (4 memos at most)
    pub fn release(mut memo: RenderMemo) {
        FREE_MEMOS.with(|f| {
            let mut free = f.borrow_mut();
            if free.len() < 4 {
                memo.declarations.clear();
                memo.constructs.clear();
                memo.values.clear();
                // the writer ids are never reused: the locations are kept
                free.push(memo);
            }
        });
    }

    /// record the location of a declaration in a render buffer
    fn locate(&mut self, slot: usize, writer: &Writer, start: usize, extra: i64) {
        if self.locations.len() <= slot {
            self.locations.resize(slot + 1, (0, 0, 0, 0));
        }
        self.locations[slot] = (writer.id, start, writer.length(), writer.extra - extra);
    }
}

/// render state shared by the nodes and the tokens of a render: the memo, valid for the options it was created for,
/// and the resolved source file names of the source map
pub struct RenderCache {
    paths: RefCell<HashMap<String, String>>,
    pub memo: RefCell<RenderMemo>,
    /// the options the memo is valid for (identity, like `cache.options === options`)
    options: *const RenderOptions,
}

impl RenderCache {
    pub fn new(memo: RenderMemo, options: &RenderOptions) -> Self {
        Self { paths: RefCell::new(HashMap::default()), memo: RefCell::new(memo), options }
    }
}

/// the memo of the cache when the tokens are rendered with the options it was created for (`getRenderMemo()`)
fn get_render_memo<'c>(cache: Option<&'c RenderCache>, options: &RenderOptions) -> Option<&'c RefCell<RenderMemo>> {
    cache.filter(|c| std::ptr::eq(c.options, options)).map(|c| &c.memo)
}

/// a rendered string: a memoized string is shared, not copied
pub enum Rendered {
    Owned(String),
    Shared(Rc<str>),
}

impl std::ops::Deref for Rendered {
    type Target = str;
    fn deref(&self) -> &str {
        match self {
            Rendered::Owned(s) => s,
            Rendered::Shared(s) => s,
        }
    }
}

impl std::fmt::Display for Rendered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self)
    }
}

impl From<Rendered> for String {
    fn from(r: Rendered) -> String {
        match r {
            Rendered::Owned(s) => s,
            Rendered::Shared(s) => s.to_string(),
        }
    }
}

/// source map records collected while rendering
#[derive(Default)]
struct SourceMapRecords {
    sources: Vec<u32>,
    maps: Vec<[i64; 5]>,
}

fn add_source(sourcemaps: &mut SourceMapRecords, src_id: u32) {
    if !sourcemaps.sources.contains(&src_id) {
        sourcemaps.sources.push(src_id);
    }
}

/// copy of the options with the defaults of the missing options
pub fn resolve_render_options(options: &RenderOptions) -> RenderOptions {
    let minify = options.minify.unwrap_or(true);
    let beautify = options.beautify.unwrap_or(!minify);
    let mut o = options.clone();
    o.indent.get_or_insert_with(|| if beautify { " ".into() } else { String::new() });
    o.new_line.get_or_insert_with(|| if beautify { "\n".into() } else { String::new() });
    o.remove_empty.get_or_insert(minify);
    o.remove_comments.get_or_insert(minify);
    if minify && o.minify.is_none() {
        o.minify = Some(true);
    }
    o.minify_values.get_or_insert(true);
    o.sourcemap.get_or_insert(SourceMapOption::Off);
    o.convert_color.get_or_insert(ConvertColor::Bool(true));
    o.expand_nesting_rules.get_or_insert(false);
    o.preserve_license.get_or_insert(false);
    o
}

/// token reducer: the comments are removed according to the options (`createReducer()`)
pub struct Reducer<'a> {
    pub options: &'a RenderOptions,
    pub cache: Option<&'a RenderCache>,
}

impl<'a> Reducer<'a> {
    pub fn new(options: &'a RenderOptions, cache: Option<&'a RenderCache>) -> Self {
        Self { options, cache }
    }

    fn reduce(&self, arena: &AstArena, acc: &mut String, curr: u32) {
        let o = self.options;
        if arena.data_types.get(curr) == EnumToken::CommentTokenType && o.remove_comments == Some(true) {
            if o.preserve_license != Some(true) || !arena.string_starts_with(arena.string_value.get(curr), "/*!") {
                return;
            }
            acc.push_str(&arena.resolve_string(arena.string_value.get(curr)));
            return;
        }
        if !render_simple_into(arena, curr, acc) {
            acc.push_str(&render_value_with(arena, curr, o, self.cache, Some(self)));
        }
    }
}

/// append a token without children that renders its value as is (idents, strings, literals, separators, numbers): the
/// value is written to the output, no string is created. false for the other tokens (see `render_token()`)
fn render_simple_into(arena: &AstArena, token: u32, out: &mut String) -> bool {
    if arena.first_token.get(token) != 0 || arena.first_child.get(token) != 0 {
        return false;
    }
    match arena.data_types.get(token) {
        EnumToken::CssTypeTokenType
        | EnumToken::UrlTokenTokenType
        | EnumToken::HashTokenType
        | EnumToken::IdenTokenType
        | EnumToken::StringTokenType
        | EnumToken::LiteralTokenType
        | EnumToken::UniCodeRange
        | EnumToken::DashedIdenTokenType
        | EnumToken::PseudoPageTokenType
        | EnumToken::ClassSelectorTokenType => arena.with_string(arena.string_value.get(token), |v| out.push_str(v)),
        EnumToken::NestingSelectorTokenType => out.push('&'),
        EnumToken::DescendantCombinatorTokenType | EnumToken::WhitespaceTokenType => out.push(' '),
        EnumToken::ColonTokenType => out.push(':'),
        EnumToken::SemiColonTokenType => out.push(';'),
        EnumToken::CommaTokenType => out.push(','),
        // no fraction: no child
        EnumToken::NumberTokenType => crate::syntax::syntax::with_minified_number(arena.numeric_value.get(token), |v| out.push_str(v)),
        _ => return false,
    }
    true
}

/// renders nodes with the same options (`NodeRenderer`): the size of the rendered node
pub struct NodeRenderer {
    options: RenderOptions,
    /// resolved options: boxed, the cache refers to them
    resolved: Box<RenderOptions>,
    /// render state: the memo keys contain interned string ids, the memo is valid for one arena
    cache: RenderCache,
    /// arena the cache is valid for
    arena: *const AstArena,
    direct: bool,
    /// output buffer, reused by the renders: only the size of the output is read
    writer: Writer,
    indents: Vec<String>,
}

impl NodeRenderer {
    pub fn new(options: &RenderOptions) -> Self {
        let resolved = Box::new(resolve_render_options(options));
        let direct = resolved.expand_nesting_rules != Some(true)
            && resolved.with_parents != Some(true)
            && matches!(resolved.sourcemap, None | Some(SourceMapOption::Off))
            && resolved.output.is_none();
        let cache = RenderCache::new(RenderMemo::default(), &resolved);
        Self { options: options.clone(), resolved, cache, arena: std::ptr::null(), direct, writer: Writer::new(DEFAULT_CAPACITY), indents: Vec::new() }
    }

    /// rendered size of a node, in utf-16 code units
    pub fn size(&mut self, arena: &AstArena, node: u32) -> usize {
        if !std::ptr::eq(self.arena, arena) {
            if !self.arena.is_null() {
                self.cache = RenderCache::new(RenderMemo::default(), &self.resolved);
            }
            self.arena = arena;
        }
        if !self.direct {
            let mut memo = self.cache.memo.borrow_mut();
            return do_render_with(arena, node, &self.options, None, Some(&mut memo)).code_length;
        }
        let reducer = Reducer::new(&self.resolved, Some(&self.cache));
        // the size is computed from the sizes of the parts: the node is not rendered
        let (size, _) = node_size(arena, node, &self.resolved, &reducer, &self.cache, 0);
        if size_check() {
            self.writer.reset();
            let mut location = SourceLocation::default();
            render_ast_node(arena, node, &self.resolved, None, &mut location, None, &reducer, &self.cache, &mut self.writer, 0, &mut self.indents);
            assert_eq!(size, self.writer.code_length(), "computed size of node {} differs from its rendered size", node);
        }
        size
    }
}

/// `CSS_PARSER_CHECK_SIZE`: the computed sizes are checked against the rendered sizes (tests)
fn size_check() -> bool {
    use std::sync::OnceLock;
    static CHECK: OnceLock<bool> = OnceLock::new();
    *CHECK.get_or_init(|| std::env::var_os("CSS_PARSER_CHECK_SIZE").is_some())
}

const OUTPUT_PADDING: usize = 1 << 12;

/// capacity of the buffer a node is rendered to
pub fn output_capacity(arena: &AstArena, node: u32) -> usize {
    let length = if arena.data_types.get(node) == EnumToken::StyleSheetNodeType {
        arena.input_lengths.borrow().get(&node).copied().unwrap_or(0) as usize
    } else if arena.flags.get(node) & EnumAstNodeStatus::HasLocation != 0 {
        (arena.get_span_end(node) - arena.get_span_start(node)) as usize
    } else {
        0
    };
    if length > 0 {
        length + OUTPUT_PADDING
    } else {
        DEFAULT_CAPACITY
    }
}

/// render an ast (`doRender()`)
pub fn do_render(arena: &AstArena, data: u32, options: &RenderOptions, mapping: Option<&RenderMapping>) -> RenderResult {
    do_render_with(arena, data, options, mapping, None)
}

/// render an ast with the rendered values of a memo shared with other renders that use the same options (see
/// [`NodeRenderer`]). without a memo, a memo is acquired and released
pub fn do_render_with(arena: &AstArena, data: u32, options: &RenderOptions, mapping: Option<&RenderMapping>, memo: Option<&mut RenderMemo>) -> RenderResult {
    let mut options = resolve_render_options(options);
    let capacity = output_capacity(arena, data);
    let mut data = data;
    let expand_nesting = options.expand_nesting_rules == Some(true)
        && matches!(arena.data_types.get(data), EnumToken::StyleSheetNodeType | EnumToken::AtRuleNodeType | EnumToken::RuleNodeType)
        && arena.flags.get(data) & EnumAstNodeStatus::HasChildren != 0;
    let mut sheet = 0u32;
    if options.with_parents == Some(true) || expand_nesting {
        let mut parent = arena.parents.get(data);
        sheet = arena.create_style_sheet();
        data = arena.clone_node_deep(data, true, None);
        if options.with_parents == Some(true) {
            while parent != 0 {
                let clone = arena.clone_node_deep(parent, false, None);
                arena.append_child(clone, data);
                data = clone;
                parent = arena.parents.get(parent);
            }
        }
        arena.append_child(sheet, data);
        if expand_nesting {
            data = arena.expand(data);
        }
    }
    let start_time = now();
    let with_map = !matches!(options.sourcemap, None | Some(SourceMapOption::Off));
    let mut sourcemaps = if with_map { Some(SourceMapRecords::default()) } else { None };
    let mut source_location = SourceLocation::default();
    let mut lines_map = if with_map { Some(LineMap::new(Vec::new())) } else { None };
    let mut writer = Writer::new(capacity);
    let indent = options.indent.clone().unwrap_or_default();
    let new_line = options.new_line.clone().unwrap_or_default();
    let cwd = options.cwd.clone().unwrap_or_default();
    if let Some(mapping) = mapping {
        let mut code = String::new();
        if let Some(import_mapping) = &mapping.import_mapping {
            let base = match &options.output {
                Some(output) => dirname(output),
                None => dirname(options.src.as_deref().unwrap_or("")),
            };
            let absolute_path = resolve(&base, &cwd, None).absolute;
            for (key, value) in import_mapping {
                let mut key = resolve(&resolve(key, &cwd, None).absolute, &absolute_path, None).relative;
                if !key.starts_with('/') && !key.starts_with('.') {
                    key = format!("./{}", key);
                }
                let body = value.iter().map(|(k, v)| format!("{}{}:{}{};", indent, v, indent, k)).collect::<Vec<_>>().join(&new_line);
                code.push_str(&format!(":import(\"{}\"){}{{{}{}{}}}{}", key, indent, new_line, body, new_line, new_line));
            }
        }
        let body = mapping.mapping.iter().map(|(k, v)| format!("{}{}:{}{};", indent, k, indent, v)).collect::<Vec<_>>().join(&new_line);
        code.push_str(&format!(":export{}{{{}{}{}}}{}", indent, new_line, body, new_line, new_line));
        writer.write(&code);
        if let Some(lines_map) = lines_map.as_mut() {
            move_location(&mut source_location, lines_map, &code, 0, None);
        }
    }
    if let Some(output) = &options.output {
        options.output = Some(resolve(output, &cwd, None).absolute);
    }
    // identical values are rendered once. the memo is valid for these options only
    let own_memo = memo.is_none();
    let mut memo = memo;
    let initial = match memo.as_deref_mut() {
        Some(m) => std::mem::take(m),
        None => RenderMemo::acquire(),
    };
    let cache = RenderCache::new(initial, &options);
    let reducer = Reducer::new(&options, Some(&cache));
    let mut indents: Vec<String> = Vec::new();
    render_ast_node(
        arena,
        data,
        &options,
        sourcemaps.as_mut(),
        &mut source_location,
        lines_map.as_mut(),
        &reducer,
        &cache,
        &mut writer,
        0,
        &mut indents,
    );
    let total = format_ms(now() - start_time);
    let used = cache.memo.replace(RenderMemo::default());
    if own_memo {
        RenderMemo::release(used);
    } else if let Some(m) = memo {
        *m = used;
    }
    if sheet != 0 {
        arena.detach(data);
    }
    let mut map: Option<SourceMap> = None;
    if let Some(records) = sourcemaps {
        let mut sourcemap = SourceMap::new();
        if let Some(sources) = &options.sources_map {
            for &source_id in &records.sources {
                if let Some(source) = sources.borrow().get(source_id) {
                    let content = source.content.borrow().clone();
                    sourcemap.add_source_content(source.id as i64, source.file.as_deref(), Some(&content));
                }
            }
        }
        let _ = sourcemap.add(&records.maps);
        if options.sourcemap == Some(SourceMapOption::Inline) {
            writer.write(&format!("\n/*# sourceMappingURL={} */", sourcemap.to_url()));
        }
        map = Some(sourcemap);
    }
    let code_length = writer.code_length();
    RenderResult { bytes: writer.into_bytes(), errors: Vec::<ErrorDescription>::new(), total, map, code_length }
}

/// update the source map with the location of a node
#[allow(clippy::too_many_arguments)]
fn update_source_map(
    arena: &AstArena,
    node: u32,
    options: &RenderOptions,
    cache: &RenderCache,
    sourcemaps: &mut SourceMapRecords,
    source_location: &mut SourceLocation,
    lines_map: &mut LineMap,
    str: &str,
) {
    let offset = str.encode_utf16().take_while(|&u| is_white_space(u as u32)).count();
    if offset > 0 {
        move_location(source_location, lines_map, str, 0, Some(offset + 1));
    }
    if arena.flags.get(node) & EnumAstNodeStatus::HasLocation != 0 {
        let mut src_id = arena.get_source(node);
        let sources = options.sources_map.clone().unwrap_or_default();
        let Some(source) = sources.borrow().get(src_id) else {
            move_location(source_location, lines_map, str, offset, None);
            return;
        };
        let input_source_map = source.get_input_source_map();
        let (mut line, mut column) = source.line_starts.borrow().get_offsets(arena.get_span_start(node));
        let generated_line = lines_map.search(source_location.end);
        let new_line = generated_line + 1;
        let new_column = source_location.end as i64 - lines_map.line_starts[generated_line.max(0) as usize] as i64 + if generated_line == 0 { 1 } else { 0 };
        let records = input_source_map.and_then(|m| {
            let mut m = (*m).clone();
            m.find(line as i64, column as i64)
        });
        if let Some(records) = records {
            for (file, l, c, content) in records {
                let mut new_id: Option<u32> = None;
                let mut source_file_name: Option<String> = file.as_deref().filter(|f| !f.is_empty()).map(String::from);
                line = l as u32;
                column = c as u32;
                let source_content: Option<String> = content.as_deref().filter(|c| !c.is_empty()).map(String::from);
                if let (Some(name), Some(output)) = (source_file_name.clone(), options.output.as_ref()) {
                    if !name.starts_with("data:") {
                        let cached = cache.paths.borrow().get(&name).cloned();
                        let resolved = match cached {
                            Some(p) => p,
                            None => {
                                let cwd = options.cwd.clone().unwrap_or_default();
                                let absolute = resolve(&dirname(output), &cwd, None).absolute;
                                let absolute_source_path = resolve(&dirname(options.src.as_deref().unwrap_or("")), &cwd, None).absolute;
                                let absolute_source_file_name = resolve(&name, &absolute_source_path, None).absolute;
                                let p = resolve(&absolute_source_file_name, &absolute, None).relative;
                                cache.paths.borrow_mut().insert(name.clone(), p.clone());
                                p
                            }
                        };
                        source_file_name = Some(resolved);
                    }
                }
                for (id, f) in sources.borrow().iter() {
                    if f.file.as_deref() == source_file_name.as_deref() {
                        new_id = Some(*id);
                        break;
                    }
                    if source_file_name.is_none() && Some(&**f.content.borrow()) == source_content.as_deref() {
                        new_id = Some(*id);
                        break;
                    }
                }
                let new_id = match new_id {
                    Some(id) => id,
                    None => {
                        let s = SourceFile::new(Rc::from(source_content.as_deref().unwrap_or("")), Vec::new(), source_file_name.as_deref().map(Rc::from));
                        sources.borrow_mut().insert(s.id, s.clone());
                        s.id
                    }
                };
                src_id = new_id;
                add_source(sourcemaps, src_id);
                sourcemaps.maps.push([new_line, new_column, src_id as i64, line as i64, column as i64]);
            }
        } else {
            add_source(sourcemaps, src_id);
            sourcemaps.maps.push([new_line, new_column, src_id as i64, line as i64, column as i64]);
        }
    }
    move_location(source_location, lines_map, str, offset, None);
}

/// update the position (`move()`): the offsets are computed from the rendered string, its utf-16 code units are read
/// in place (no copy)
pub fn move_location(source_location: &mut SourceLocation, lines_map: &mut LineMap, str: &str, start: usize, end: Option<usize>) {
    let end = end.unwrap_or(usize::MAX);
    let mut units = str.encode_utf16().enumerate().peekable();
    let mut previous: u16 = 0;
    while let Some((i, unit)) = units.next() {
        if i >= end {
            break;
        }
        if i >= start {
            source_location.end += 1;
            let codepoint = unit as u32;
            if matches!(codepoint, 0xa | 0xb | 0xc | 0xd | 0x2028 | 0x2029) {
                let crlf_start = codepoint == 0xd && units.peek().is_some_and(|&(_, n)| n == 0xa);
                let crlf_end = codepoint == 0xa && i > 0 && previous == 0xd;
                if !crlf_start && !crlf_end {
                    lines_map.line_starts.push(source_location.end);
                }
            }
        }
        previous = unit;
    }
    // `end` beyond the string: the missing code units are counted (`codepoint` is undefined in typescript)
    let length = str.encode_utf16().count();
    if end != usize::MAX && end > length {
        source_location.end += (end - length.max(start)) as u32;
    }
}

/// write an interned string (copied from the string table when ascii)
fn write_string(arena: &AstArena, id: u32, writer: &mut Writer) {
    if arena.string_length(id) == 0 {
        return;
    }
    if arena.copy_string_ascii(id, &mut writer.buffer) < 0 {
        writer.write(&arena.resolve_string(id));
    }
}

/// render an ast node to the writer
#[allow(clippy::too_many_arguments)]
fn render_ast_node(
    arena: &AstArena,
    data: u32,
    options: &RenderOptions,
    mut sourcemaps: Option<&mut SourceMapRecords>,
    source_location: &mut SourceLocation,
    mut lines_map: Option<&mut LineMap>,
    reducer: &Reducer,
    cache: &RenderCache,
    writer: &mut Writer,
    level: usize,
    indents: &mut Vec<String>,
) {
    let opt_indent = options.indent.clone().unwrap_or_default();
    if indents.len() < level + 1 {
        indents.push(opt_indent.repeat(level));
    }
    if indents.len() < level + 2 {
        indents.push(opt_indent.repeat(level + 1));
    }
    let indent = indents[level].clone();
    let typ = arena.data_types.get(data);
    let new_line = options.new_line.clone().unwrap_or_default();
    match typ {
        EnumToken::DeclarationNodeType => {
            write_string(arena, arena.name.get(data), writer);
            writer.write_byte(0x3a);
            writer.write(&opt_indent);
            writer.write(&render_declaration_value(arena, data, options.minify_values == Some(true), reducer));
        }
        EnumToken::CommentNodeType | EnumToken::CDOCOMMNodeType => {
            let id = arena.string_value.get(data);
            if arena.string_starts_with(id, "/*# sourceMappingURL=") {
                return;
            }
            if options.remove_comments != Some(true) || (options.preserve_license == Some(true) && arena.string_starts_with(id, "/*!")) {
                write_string(arena, id, writer);
            }
        }
        EnumToken::StyleSheetNodeType => {
            let mut empty = true;
            let mut node = arena.first_child.get(data);
            while node != 0 {
                let mark = writer.length();
                let mark_extra = writer.extra;
                if !empty && !new_line.is_empty() {
                    writer.write(&new_line);
                }
                let start = writer.length();
                render_ast_node(arena, node, options, sourcemaps.as_deref_mut(), source_location, lines_map.as_deref_mut(), reducer, cache, writer, level, indents);
                if writer.length() == start {
                    writer.truncate(mark, mark_extra);
                    node = arena.next_sibling.get(node);
                    continue;
                }
                empty = false;
                if sourcemaps.is_some() && !new_line.is_empty() {
                    move_location(source_location, lines_map.as_deref_mut().unwrap(), &new_line, 0, None);
                }
                node = arena.next_sibling.get(node);
            }
        }
        EnumToken::AtRuleNodeType | EnumToken::RuleNodeType | EnumToken::KeyframesRuleNodeType | EnumToken::KeyframesAtRuleNodeType => {
            let is_at_rule = typ == EnumToken::AtRuleNodeType || typ == EnumToken::KeyframesAtRuleNodeType;
            let name_id = arena.name.get(data);
            let value_id = arena.string_value.get(data);
            let start = writer.length();
            let start_extra = writer.extra;
            if !indent.is_empty() && !new_line.is_empty() {
                writer.write(&new_line);
            }
            if !indent.is_empty() {
                writer.write(&indent);
            }
            if is_at_rule {
                writer.write_byte(0x40);
                write_string(arena, name_id, writer);
                if arena.string_length(value_id) > 0 {
                    writer.write(if opt_indent.is_empty() { " " } else { &opt_indent });
                    write_string(arena, value_id, writer);
                }
                if arena.flags.get(data) & EnumAstNodeStatus::HasChildren == 0 {
                    writer.write_byte(0x3b);
                    return;
                }
            } else {
                write_string(arena, name_id, writer);
            }
            if !opt_indent.is_empty() {
                writer.write(&opt_indent);
            }
            writer.write_byte(0x7b);
            let line_map_length = lines_map.as_ref().map_or(0, |l| l.line_starts.len());
            let mut prelude = String::new();
            if let Some(sm) = sourcemaps.as_deref_mut() {
                let nam = arena.resolve_string(name_id);
                let val = arena.resolve_string(value_id);
                prelude = format!(
                    "{}{}{}",
                    if !indent.is_empty() { new_line.as_str() } else { "" },
                    indent,
                    if is_at_rule {
                        format!("@{}{}{}{}{{", nam, if val.is_empty() { "" } else if opt_indent.is_empty() { " " } else { &opt_indent }, val, opt_indent)
                    } else {
                        format!("{}{}{{", nam, opt_indent)
                    }
                );
                update_source_map(arena, data, options, cache, sm, source_location, lines_map.as_deref_mut().unwrap(), &prelude);
            }
            let indent_sub = indents[level + 1].clone();
            let body = writer.length();
            let record_declaration_source_map = typ == EnumToken::AtRuleNodeType;
            let mut semicolon = false;
            let mut node = arena.first_child.get(data);
            while node != 0 {
                let next = arena.next_sibling.get(node);
                let node_type = arena.data_types.get(node);
                if node_type == EnumToken::DeclarationNodeType && sourcemaps.is_none() {
                    if semicolon {
                        writer.write_byte(0x3b);
                    }
                    if !new_line.is_empty() {
                        writer.write(&new_line);
                    }
                    if !indent_sub.is_empty() {
                        writer.write(&indent_sub);
                    }
                    write_declaration(arena, node, options, cache, reducer, writer);
                    semicolon = true;
                    node = next;
                    continue;
                }
                let str: String;
                if node_type == EnumToken::CommentNodeType {
                    let comment_id = arena.string_value.get(node);
                    if options.remove_comments == Some(true) && (options.preserve_license != Some(true) || !arena.string_starts_with(comment_id, "/*!")) {
                        node = next;
                        continue;
                    }
                    str = arena.resolve_string(comment_id).to_string();
                } else if node_type == EnumToken::DeclarationNodeType {
                    str = render_declaration(arena, node, options, cache, reducer).into();
                } else {
                    let mark = writer.length();
                    if semicolon {
                        writer.write_byte(0x3b);
                    }
                    let child_start = writer.length();
                    render_ast_node(arena, node, options, sourcemaps.as_deref_mut(), source_location, lines_map.as_deref_mut(), reducer, cache, writer, level + 1, indents);
                    if writer.length() == child_start {
                        writer.buffer.truncate(mark);
                        node = next;
                        continue;
                    }
                    semicolon = ends_with_semicolon(arena, node, writer);
                    if semicolon {
                        writer.buffer.pop();
                    }
                    node = next;
                    continue;
                }
                if str.is_empty() {
                    node = next;
                    continue;
                }
                if semicolon {
                    writer.write_byte(0x3b);
                }
                let written: String;
                if sourcemaps.is_none() {
                    if !new_line.is_empty() {
                        writer.write(&new_line);
                    }
                    if !indent_sub.is_empty() {
                        writer.write(&indent_sub);
                    }
                    writer.write(&str);
                    written = str;
                } else {
                    written = format!("{}{}{}", new_line, indent_sub, str);
                    writer.write(&written);
                }
                if node_type == EnumToken::DeclarationNodeType {
                    semicolon = true;
                } else {
                    semicolon = writer.last_byte() == 0x3b;
                    if semicolon {
                        writer.buffer.pop();
                    }
                }
                if let Some(sm) = sourcemaps.as_deref_mut() {
                    if node_type == EnumToken::DeclarationNodeType && record_declaration_source_map {
                        update_source_map(arena, node, options, cache, sm, source_location, lines_map.as_deref_mut().unwrap(), &written);
                    } else {
                        move_location(source_location, lines_map.as_deref_mut().unwrap(), &written, 0, None);
                    }
                    if node_type == EnumToken::DeclarationNodeType {
                        source_location.end += 1;
                    }
                }
                node = next;
            }
            if semicolon {
                source_location.end = source_location.end.saturating_sub(1);
            }
            if options.remove_empty == Some(true)
                && writer.length() == body
                && !(typ == EnumToken::AtRuleNodeType
                    && (arena.string_equals(name_id, "apply") || arena.string_equals(name_id, "mixin") || arena.string_equals(name_id, "function")))
            {
                if sourcemaps.is_some() {
                    source_location.end = source_location.end.saturating_sub(prelude.encode_utf16().count() as u32);
                    if let Some(l) = lines_map.as_deref_mut() {
                        l.line_starts.truncate(line_map_length);
                    }
                }
                writer.truncate(start, start_extra);
                return;
            }
            if !new_line.is_empty() {
                writer.write(&new_line);
            }
            if !indent.is_empty() {
                writer.write(&indent);
            }
            writer.write_byte(0x7d);
            if sourcemaps.is_some() {
                move_location(source_location, lines_map.as_deref_mut().unwrap(), &format!("{}{}}}", new_line, indent), 0, None);
            }
        }
        _ => {}
    }
}

/// true if a rendered child node ends with a semicolon
fn ends_with_semicolon(arena: &AstArena, node: u32, writer: &Writer) -> bool {
    match arena.data_types.get(node) {
        EnumToken::AtRuleNodeType | EnumToken::KeyframesAtRuleNodeType => arena.flags.get(node) & EnumAstNodeStatus::HasChildren == 0,
        EnumToken::RuleNodeType | EnumToken::KeyframesRuleNodeType => false,
        _ => writer.last_byte() == 0x3b,
    }
}

/// the values of the custom properties and of `initial-value` are not minified
fn is_unminified_declaration(arena: &AstArena, node: u32) -> bool {
    arena.starts_with(node, "nam", "--") || arena.string_equals(arena.name.get(node), "initial-value")
}

/// render a declaration: `name:value`, without the semicolon
/// render a declaration without memoization: `name:value`, without the semicolon
fn render_declaration_string(arena: &AstArena, node: u32, options: &RenderOptions, cache: &RenderCache, reducer: &Reducer) -> String {
    let unminified;
    let own_reducer;
    let (options, reducer) = if options.minify_values == Some(true) && is_unminified_declaration(arena, node) {
        // the values of the custom properties are not minified
        unminified = RenderOptions { minify_values: Some(false), ..options.clone() };
        own_reducer = Reducer::new(&unminified, Some(cache));
        (&unminified, &own_reducer)
    } else {
        (options, reducer)
    };
    format!(
        "{}:{}{}",
        arena.resolve_string(arena.name.get(node)),
        options.indent.as_deref().unwrap_or(""),
        render_declaration_value(arena, node, options.minify_values == Some(true), reducer).trim_end()
    )
}

/// render a declaration of a block: `name:value`, without the semicolon. identical declarations (same name, same
/// value tokens) are rendered once per render
fn render_declaration(arena: &AstArena, node: u32, options: &RenderOptions, cache: &RenderCache, reducer: &Reducer) -> Rendered {
    let Some(memo) = get_render_memo(Some(cache), options) else {
        return Rendered::Owned(render_declaration_string(arena, node, options, cache, reducer));
    };
    // the name is part of the key
    let slot = memo.borrow_mut().declarations.list(arena, arena.first_token.get(node), arena.name.get(node) as i32);
    if let Some(rendered) = memo.borrow().declarations.get(slot) {
        return Rendered::Shared(rendered.clone());
    }
    let rendered: Rc<str> = Rc::from(render_declaration_string(arena, node, options, cache, reducer));
    memo.borrow_mut().declarations.set(slot, rendered.clone());
    Rendered::Shared(rendered)
}

/// write a declaration of a block: `name:value`, without the semicolon. identical declarations are rendered and
/// encoded once per render: the bytes written first are copied
/// the rendered value of a declaration (without the name): rendered once per structure, see [`RenderMemo`]. returns
/// the memo slot (`usize::MAX` without memo) and the value
fn declaration_value(arena: &AstArena, node: u32, options: &RenderOptions, cache: &RenderCache, reducer: &Reducer) -> (usize, Rc<str>) {
    declaration_value_at(arena, node, options, cache, reducer, None)
}

/// see [`declaration_value`]: `slot` is the memo slot of the declaration when it has been looked up (one lookup per
/// declaration)
fn declaration_value_at(arena: &AstArena, node: u32, options: &RenderOptions, cache: &RenderCache, reducer: &Reducer, slot: Option<usize>) -> (usize, Rc<str>) {
    let memo = get_render_memo(Some(cache), options);
    let mut slot = slot.unwrap_or(usize::MAX);
    if let Some(memo) = memo {
        // the name is part of the key
        if slot == usize::MAX {
            slot = memo.borrow_mut().declarations.list(arena, arena.first_token.get(node), arena.name.get(node) as i32);
        }
        if let Some(value) = memo.borrow().values.get(slot).cloned().flatten() {
            return (slot, value);
        }
    }
    let value = if options.minify_values == Some(true) && is_unminified_declaration(arena, node) {
        let unminified = RenderOptions { minify_values: Some(false), ..options.clone() };
        let own = Reducer::new(&unminified, Some(cache));
        render_declaration_value(arena, node, false, &own)
    } else {
        render_declaration_value(arena, node, options.minify_values == Some(true), reducer)
    };
    let value: Rc<str> = Rc::from(value.trim_end());
    if let Some(memo) = memo {
        let mut m = memo.borrow_mut();
        if m.values.len() <= slot {
            m.values.resize(slot + 1, None);
        }
        m.values[slot] = Some(value.clone());
    }
    (slot, value)
}

fn write_declaration(arena: &AstArena, node: u32, options: &RenderOptions, cache: &RenderCache, reducer: &Reducer, writer: &mut Writer) {
    let memo = get_render_memo(Some(cache), options);
    let mut looked_up = None;
    if let Some(memo) = memo {
        // an identical declaration is already written to the buffer: its bytes are copied
        let slot = memo.borrow_mut().declarations.list(arena, arena.first_token.get(node), arena.name.get(node) as i32);
        let location = memo.borrow().locations.get(slot).copied();
        if let Some((id, start, end, extra)) = location {
            if id == writer.id {
                writer.copy(start, end, extra);
                return;
            }
        }
        looked_up = Some(slot);
    }
    let start = writer.length();
    let extra = writer.extra;
    let (slot, value) = declaration_value_at(arena, node, options, cache, reducer, looked_up);
    // the parts are written directly: the name is copied from the string table
    write_string(arena, arena.name.get(node), writer);
    writer.write_byte(0x3a);
    if let Some(indent) = options.indent.as_deref().filter(|i| !i.is_empty()) {
        writer.write(indent);
    }
    writer.write(&value);
    if let Some(memo) = memo {
        memo.borrow_mut().locate(slot, writer, start, extra);
    }
}

/// length of a string in utf-16 code units (`string.length`)
fn units(s: &str) -> usize {
    if s.is_ascii() {
        s.len()
    } else {
        s.encode_utf16().count()
    }
}

/// the size of a rendered node, computed without rendering it: the declarations are measured from their rendered
/// values (rendered once per structure, see [`declaration_value`]), the layout follows [`render_ast_node`] (no source
/// map). returns the size in utf-16 code units (0 when the node renders nothing) and whether the output ends with a
/// semicolon
fn node_size(arena: &AstArena, data: u32, options: &RenderOptions, reducer: &Reducer, cache: &RenderCache, level: usize) -> (usize, bool) {
    let opt_indent = options.indent.as_deref().unwrap_or("");
    let new_line = options.new_line.as_deref().unwrap_or("");
    let indent_len = units(opt_indent) * level;
    let new_line_len = units(new_line);
    let typ = arena.data_types.get(data);
    match typ {
        EnumToken::DeclarationNodeType => {
            let (_, value) = declaration_value(arena, data, options, cache, reducer);
            let size = arena.string_length(arena.name.get(data)) + 1 + units(opt_indent) + units(&value);
            (size, value.ends_with(';'))
        }
        EnumToken::CommentNodeType | EnumToken::CDOCOMMNodeType => {
            let id = arena.string_value.get(data);
            if arena.string_starts_with(id, "/*# sourceMappingURL=") {
                return (0, false);
            }
            if options.remove_comments != Some(true) || (options.preserve_license == Some(true) && arena.string_starts_with(id, "/*!")) {
                let length = arena.string_length(id);
                return (length, length > 0 && arena.with_string(id, |s| s.ends_with(';')));
            }
            (0, false)
        }
        EnumToken::StyleSheetNodeType => {
            let mut size = 0;
            let mut empty = true;
            let mut last_semicolon = false;
            let mut node = arena.first_child.get(data);
            while node != 0 {
                let (child, semicolon) = node_size(arena, node, options, reducer, cache, level);
                if child > 0 {
                    if !empty {
                        size += new_line_len;
                    }
                    size += child;
                    empty = false;
                    last_semicolon = semicolon;
                }
                node = arena.next_sibling.get(node);
            }
            (size, last_semicolon)
        }
        EnumToken::AtRuleNodeType | EnumToken::RuleNodeType | EnumToken::KeyframesRuleNodeType | EnumToken::KeyframesAtRuleNodeType => {
            let is_at_rule = typ == EnumToken::AtRuleNodeType || typ == EnumToken::KeyframesAtRuleNodeType;
            let name_id = arena.name.get(data);
            let value_id = arena.string_value.get(data);
            let mut size = 0;
            if indent_len > 0 && new_line_len > 0 {
                size += new_line_len;
            }
            size += indent_len;
            if is_at_rule {
                size += 1 + arena.string_length(name_id);
                if arena.string_length(value_id) > 0 {
                    size += if opt_indent.is_empty() { 1 } else { units(opt_indent) } + arena.string_length(value_id);
                }
                if arena.flags.get(data) & EnumAstNodeStatus::HasChildren == 0 {
                    return (size + 1, true);
                }
            } else {
                size += arena.string_length(name_id);
            }
            size += units(opt_indent) + 1;
            let indent_sub_len = units(opt_indent) * (level + 1);
            let mut body = 0;
            let mut semicolon = false;
            let mut node = arena.first_child.get(data);
            while node != 0 {
                let next = arena.next_sibling.get(node);
                let node_type = arena.data_types.get(node);
                if node_type == EnumToken::DeclarationNodeType {
                    let (declaration, _) = node_size(arena, node, options, reducer, cache, level + 1);
                    body += semicolon as usize + new_line_len + indent_sub_len + declaration;
                    semicolon = true;
                } else if node_type == EnumToken::CommentNodeType {
                    let comment_id = arena.string_value.get(node);
                    if !(options.remove_comments == Some(true) && (options.preserve_license != Some(true) || !arena.string_starts_with(comment_id, "/*!"))) {
                        let length = arena.string_length(comment_id);
                        if length > 0 {
                            body += semicolon as usize + new_line_len + indent_sub_len + length;
                            // a trailing semicolon is removed: it is written before the next node
                            semicolon = arena.with_string(comment_id, |s| s.ends_with(';'));
                            if semicolon {
                                body -= 1;
                            }
                        }
                    }
                } else {
                    let (child, ends_with) = node_size(arena, node, options, reducer, cache, level + 1);
                    if child > 0 {
                        body += semicolon as usize + child;
                        semicolon = match node_type {
                            EnumToken::AtRuleNodeType | EnumToken::KeyframesAtRuleNodeType => arena.flags.get(node) & EnumAstNodeStatus::HasChildren == 0,
                            EnumToken::RuleNodeType | EnumToken::KeyframesRuleNodeType => false,
                            _ => ends_with,
                        };
                        if semicolon {
                            body -= 1;
                        }
                    }
                }
                node = next;
            }
            if options.remove_empty == Some(true)
                && body == 0
                && !(typ == EnumToken::AtRuleNodeType
                    && (arena.string_equals(name_id, "apply") || arena.string_equals(name_id, "mixin") || arena.string_equals(name_id, "function")))
            {
                return (0, false);
            }
            (size + body + new_line_len + indent_len + 1, false)
        }
        _ => (0, false),
    }
}

thread_local! {
    /// number of declaration values rendered, see [`declaration_renders`]
    static DECLARATION_RENDERS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// number of declaration values rendered on this thread: an identical declaration is rendered once per render
pub fn declaration_renders() -> usize {
    DECLARATION_RENDERS.with(|c| c.get())
}

/// render the value tokens of a declaration with the reducer. with `minify_values`, the whitespace before
/// `!important` and after a function is dropped
pub fn render_declaration_value(arena: &AstArena, node: u32, minify_values: bool, reducer: &Reducer) -> String {
    DECLARATION_RENDERS.with(|c| c.set(c.get() + 1));
    let mut rendered = String::new();
    let mut token = arena.first_token.get(node);
    if !minify_values {
        while token != 0 {
            reducer.reduce(arena, &mut rendered, token);
            token = arena.next_sibling.get(token);
        }
        return rendered;
    }
    let t = |x: u32| arena.data_types.get(x);
    let mut pending = 0u32;
    let mut check = true;
    let mut drop_whitespace = false;
    while token != 0 {
        let typ = t(token);
        if drop_whitespace {
            drop_whitespace = false;
            if typ == EnumToken::WhitespaceTokenType {
                token = arena.next_sibling.get(token);
                continue;
            }
        }
        if !check {
            check = true;
        } else if typ == EnumToken::ImportantTokenType && pending != 0 && t(pending) == EnumToken::WhitespaceTokenType {
            pending = 0;
            check = false;
        } else if is_function_type(typ)
            && (typ != EnumToken::ColorTokenType || arena.first_token.get(token) != 0)
            && typ != EnumToken::WildCardFunctionTokenType
            && arena.next_sibling.get(token) != 0
            && t(arena.next_sibling.get(token)) == EnumToken::WhitespaceTokenType
        {
            drop_whitespace = true;
        }
        if pending != 0 {
            reducer.reduce(arena, &mut rendered, pending);
        }
        pending = token;
        token = arena.next_sibling.get(token);
    }
    if pending != 0 {
        reducer.reduce(arena, &mut rendered, pending);
    }
    rendered
}

/// render the tokens of a linked list with the reducer, or with the comment rules of the options
fn render_reduced(arena: &AstArena, first: u32, options: &RenderOptions, cache: Option<&RenderCache>, reducer: Option<&Reducer>) -> String {
    let mut result = String::new();
    let mut token = first;
    while token != 0 {
        if let Some(reducer) = reducer {
            reducer.reduce(arena, &mut result, token);
        } else if arena.data_types.get(token) == EnumToken::CommentTokenType && options.remove_comments == Some(true) {
            if options.preserve_license == Some(true) && arena.string_starts_with(arena.string_value.get(token), "/*!") {
                result.push_str(&arena.resolve_string(arena.string_value.get(token)));
            }
        } else if !render_simple_into(arena, token, &mut result) {
            result.push_str(&render_value_with(arena, token, options, cache, None));
        }
        token = arena.next_sibling.get(token);
    }
    result
}

/// render the tokens of a linked list
fn render_list(arena: &AstArena, first: u32, options: &RenderOptions, cache: Option<&RenderCache>, reducer: Option<&Reducer>) -> String {
    let mut result = String::new();
    let mut token = first;
    while token != 0 {
        if !render_simple_into(arena, token, &mut result) {
            result.push_str(&render_value_with(arena, token, options, cache, reducer));
        }
        token = arena.next_sibling.get(token);
    }
    result
}

/// render the tokens of a field of a structured token
fn render_field(arena: &AstArena, token: u32, field: u32, options: &RenderOptions, cache: Option<&RenderCache>, reducer: Option<&Reducer>) -> String {
    match arena.get_child_at(token, field) {
        None => String::new(),
        Some(list) => render_list(arena, arena.first_token.get(list), options, cache, reducer),
    }
}

fn is_legacy_color_function(name: &str) -> bool {
    matches!(name, "rgb" | "rgba" | "hsl" | "hsla")
}

fn is_alpha_color_function(name: &str) -> bool {
    matches!(name, "rgb" | "rgba" | "hsl" | "hsla" | "hwb" | "oklab" | "oklch" | "lab" | "lch")
}

/// `/[,/]\s*$/`
fn has_trailing_separator(s: &str) -> bool {
    s.trim_end_matches(crate::ast::arena::is_js_whitespace).ends_with([',', '/'])
}

/// the color type a color is converted to
fn convert_target(options: &RenderOptions) -> u8 {
    match options.convert_color {
        None | Some(ConvertColor::Bool(_)) => {
            if matches!(options.convert_color, Some(ConvertColor::Bool(_))) {
                ColorType::HEX
            } else {
                // `ColorType[options.convertColor ?? "HEX"]` -> the name "HEX" is not a number: `ColorType["HEX"]` = 3
                ColorType::value(ColorType::name(ColorType::HEX).unwrap_or("HEX")).unwrap_or(ColorType::HEX)
            }
        }
        Some(ConvertColor::Type(t)) => ColorType::name(t)
            .map(|n| n.to_uppercase().replace('-', "_"))
            .and_then(|n| ColorType::value(&n))
            .unwrap_or(ColorType::HEX),
    }
}

/// render a color token
fn render_color(arena: &AstArena, token: u32, options: &RenderOptions, cache: Option<&RenderCache>, reducer: Option<&Reducer>) -> String {
    let mut token = token;
    if options.convert_color != Some(ConvertColor::Bool(false)) {
        if let Some(value) = convert_color(arena, token, convert_target(options)) {
            token = value;
        }
        if matches!(arena.color_type.get(token), ColorType::HEX | ColorType::LIT) {
            return reduce_hex_value(&arena.resolve_string(arena.string_value.get(token)));
        }
    }
    let kin = arena.color_type.get(token);
    if matches!(kin, ColorType::SYS | ColorType::DPSYS | ColorType::NON_STD | ColorType::HEX | ColorType::LIT) {
        return arena.resolve_string(arena.string_value.get(token)).to_string();
    }
    if arena.first_token.get(token) == 0 {
        return format!("{}({})", arena.resolve_string(arena.string_value.get(token)), render_reduced(arena, 0, options, cache, reducer));
    }
    let fn_name = arena.resolve_string(arena.string_value.get(token));
    let count = arena.token_count(token) as usize;
    let is_legacy = is_legacy_color_function(&fn_name);
    let has_alpha = is_alpha_color_function(&fn_name);
    let use_alpha = (has_alpha && count == if is_legacy { 7 } else { 4 }) || (fn_name == "color" && count == 5);
    let mut result = String::new();
    let t = |x: u32| arena.data_types.get(x);
    for (index, curr) in arena.token_iter(token).enumerate() {
        if has_trailing_separator(&result) {
            if t(curr) == EnumToken::WhitespaceTokenType {
                result = result.trim_end().to_string();
                continue;
            }
            result = format!("{}{}", result.trim_start(), render_value_with(arena, curr, options, cache, None));
            continue;
        }
        if (is_legacy && t(curr) == EnumToken::CommaTokenType) || t(curr) == EnumToken::WhitespaceTokenType {
            result = format!("{} ", result.trim_end());
        } else if t(curr) == EnumToken::CommaTokenType || (t(curr) == EnumToken::LiteralTokenType && arena.string_equals(arena.string_value.get(curr), "/")) {
            result = format!("{}{}", result.trim_end(), if t(curr) == EnumToken::CommaTokenType { "," } else { "/" });
        } else {
            let sep = if use_alpha && index == count - 1 { "/" } else { " " };
            result = format!("{}{}{}", result.trim_end(), sep, render_value_with(arena, curr, options, cache, None));
        }
    }
    let name = if has_alpha && fn_name.ends_with('a') { &fn_name[..fn_name.len() - 1] } else { fn_name.as_str() };
    format!("{}({})", name, result.trim_start())
}

/// render a token with the options (`renderValue()`)
pub fn render_value(arena: &AstArena, token: u32, options: &RenderOptions) -> String {
    render_value_with(arena, token, options, None, None).into()
}

/// render a token with the default options
pub fn render_value_default(arena: &AstArena, token: u32) -> String {
    thread_local! {
        static DEFAULT: RenderOptions = RenderOptions::default();
    }
    DEFAULT.with(|options| render_value_with(arena, token, options, None, None).into())
}

/// render a token. identical constructs (colors and tokens with sub tokens: functions, parentheses, binary
/// expressions, ...) are rendered once per render, see [`RenderMemo`]
pub fn render_value_with(arena: &AstArena, token: u32, options: &RenderOptions, cache: Option<&RenderCache>, reducer: Option<&Reducer>) -> Rendered {
    if arena.data_types.get(token) == EnumToken::ColorTokenType || arena.first_token.get(token) != 0 || arena.first_child.get(token) != 0 {
        if let Some(memo) = get_render_memo(cache, options) {
            let slot = memo.borrow_mut().constructs.token(arena, token);
            if let Some(rendered) = memo.borrow().constructs.get(slot) {
                return Rendered::Shared(rendered.clone());
            }
            let rendered: Rc<str> = Rc::from(render_token(arena, token, options, cache, reducer));
            memo.borrow_mut().constructs.set(slot, rendered.clone());
            return Rendered::Shared(rendered);
        }
    }
    Rendered::Owned(render_token(arena, token, options, cache, reducer))
}

/// `minifyNumber(String(n))`
fn minify_str(v: &str) -> String {
    minify_number_str(v)
}

fn is_ws_or_comment(arena: &AstArena, t: Option<u32>) -> bool {
    t.is_some_and(|t| matches!(arena.data_types.get(t), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType))
}

fn str_eq(arena: &AstArena, t: Option<u32>, s: &str) -> bool {
    t.is_some_and(|t| arena.with_string(arena.string_value.get(t), |v| equals_ignore_case(v, s)))
}

fn typ_of(arena: &AstArena, t: Option<u32>) -> u8 {
    t.map_or(0, |t| arena.data_types.get(t))
}

/// minify the arguments of a gradient (`linear-gradient`, `radial-gradient`, `conic-gradient`)
fn minify_gradient(arena: &AstArena, token: u32, slice: &mut Vec<u32>) {
    let name = arena.resolve_string(arena.string_value.get(token));
    let at = |slice: &Vec<u32>, i: usize| slice.get(i).copied();
    let ws = |_: u32| arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None);
    match &*name {
        "linear-gradient" | "repeating-linear-gradient" => {
            let mut i: isize = 0;
            while (i as usize) < slice.len() {
                let iu = i as usize;
                let ti = typ_of(arena, at(slice, iu));
                if ti == EnumToken::ColorTokenType || ti == EnumToken::CommaTokenType {
                    break;
                }
                if ti == EnumToken::IdenTokenType && str_eq(arena, at(slice, iu), "to") {
                    let mut k = iu + 1;
                    while k < slice.len() && is_ws_or_comment(arena, at(slice, k)) {
                        k += 1;
                    }
                    if k < slice.len() && typ_of(arena, at(slice, k)) == EnumToken::IdenTokenType {
                        let key = arena.resolve_string(arena.string_value.get(slice[k])).to_lowercase();
                        match key.as_str() {
                            "right" => {
                                let angle = arena.create_token(EnumToken::AngleTokenType, TokenValue::Num(90.0), Some("deg"));
                                slice.splice(iu..k + 1, [angle]);
                            }
                            "left" => {
                                let angle = arena.create_token(EnumToken::AngleTokenType, TokenValue::Num(270.0), Some("deg"));
                                slice.splice(iu..k + 1, [angle]);
                            }
                            "top" | "bottom" => 'sw: {
                                let mut j = k + 1;
                                while j < slice.len() && is_ws_or_comment(arena, at(slice, j)) {
                                    j += 1;
                                }
                                if j < slice.len() && typ_of(arena, at(slice, j)) != EnumToken::CommaTokenType {
                                    i = j as isize - 1;
                                    break 'sw;
                                }
                                if key == "bottom" {
                                    while k < slice.len() && typ_of(arena, at(slice, k)) != EnumToken::CommaTokenType {
                                        k += 1;
                                    }
                                    let end = (k + 1).min(slice.len());
                                    slice.drain(iu..end);
                                    i -= 1;
                                    break 'sw;
                                }
                                let angle = arena.create_token(EnumToken::AngleTokenType, TokenValue::Num(if key == "top" { 0.0 } else { 180.0 }), Some("deg"));
                                slice.splice(iu..k + 1, [angle]);
                                i -= 1;
                            }
                            _ => {}
                        }
                    }
                } else if ti == EnumToken::AngleTokenType {
                    let degrees = arena.numeric_value.get(to_degrees(arena, slice[iu]));
                    if (180.0 - degrees) % 360.0 == 0.0 {
                        let k = iu + 1;
                        let end = (k + 1).min(slice.len());
                        slice.drain(iu..end);
                    }
                }
                i += 1;
            }
            let mut i = i.max(0) as usize;
            if typ_of(arena, at(slice, i)) == EnumToken::CommaTokenType {
                i += 1;
            }
            if typ_of(arena, at(slice, i)) == EnumToken::ColorTokenType {
                let mut stops: Vec<u32> = slice.drain(i..).collect();
                reduce_color_stops(arena, &mut stops);
                slice.extend(stops);
            }
        }
        "radial-gradient" | "repeating-radial-gradient" => {
            let mut i = 0usize;
            let mut positions: Vec<u32> = Vec::new();
            let mut form: Vec<u32> = Vec::new();
            let mut size: Vec<u32> = Vec::new();
            let mut color_space_def: Vec<u32> = Vec::new();
            for _ in 0..2 {
                if typ_of(arena, at(slice, i)) == EnumToken::IdenTokenType {
                    let tk = at(slice, i);
                    if str_eq(arena, tk, "farthest-corner") || str_eq(arena, tk, "ellipse") {
                        i += 1;
                    } else if str_eq(arena, tk, "closest-side") || str_eq(arena, tk, "closest-corner") || str_eq(arena, tk, "farthest-side") {
                        size.push(slice[i]);
                        i += 1;
                    } else if str_eq(arena, tk, "circle") {
                        form.push(slice[i]);
                        i += 1;
                    }
                    while is_ws_or_comment(arena, at(slice, i)) {
                        i += 1;
                    }
                }
            }
            if typ_of(arena, at(slice, i)) == EnumToken::IdenTokenType && str_eq(arena, at(slice, i), "at") {
                i += 1;
            }
            while is_ws_or_comment(arena, at(slice, i)) {
                i += 1;
            }
            let tk = at(slice, i);
            let tt = typ_of(arena, tk);
            if tt == EnumToken::PercentageTokenType
                || (tt == EnumToken::NumberTokenType && arena.numeric_value.get(tk.unwrap()) == 0.0)
                || (tt == EnumToken::IdenTokenType && ["center", "top", "bottom", "left", "right"].iter().any(|k| str_eq(arena, tk, k)))
            {
                loop {
                    if typ_of(arena, at(slice, i)) == EnumToken::IdenTokenType && at(slice, i).is_some_and(|x| arena.string_equals(arena.string_value.get(x), "in")) {
                        break;
                    }
                    if let Some(&x) = slice.get(i) {
                        positions.push(x);
                    }
                    i += 1;
                    if !(i < slice.len() && typ_of(arena, at(slice, i)) != EnumToken::CommaTokenType) {
                        break;
                    }
                }
            }
            while i < slice.len() && typ_of(arena, at(slice, i)) != EnumToken::CommaTokenType {
                color_space_def.push(slice[i]);
                i += 1;
            }
            if typ_of(arena, at(slice, i)) == EnumToken::CommaTokenType {
                i += 1;
            }
            if !positions.is_empty() {
                let mut position1 = String::new();
                let mut position2 = String::new();
                let mut k = 0usize;
                while k < positions.len() {
                    let p = positions[k];
                    let pt = arena.data_types.get(p);
                    if pt == EnumToken::IdenTokenType {
                        let v = arena.resolve_string(arena.string_value.get(p)).to_string();
                        if position1.is_empty() {
                            position1 = v;
                        } else {
                            position2 = v;
                        }
                    } else if pt == EnumToken::PercentageTokenType || pt == EnumToken::NumberTokenType {
                        let v = format!("{}%", js_number_to_string(arena.numeric_value.get(p)));
                        if position1.is_empty() {
                            position1 = v;
                        } else {
                            position2 = v;
                        }
                    }
                    if !position2.is_empty() {
                        break;
                    }
                    k += 1;
                }
                if !position1.is_empty() {
                    let key = format!("{} {}", position1, position2).trim().to_string();
                    let pc = |v: f64| arena.create_token(EnumToken::PercentageTokenType, TokenValue::Num(v), None);
                    let iden = |v: &str| arena.create_token(EnumToken::IdenTokenType, TokenValue::Str(v), None);
                    match key.as_str() {
                        "50% 50%" | "center center" | "50%" | "center" => positions.clear(),
                        "0% 0%" | "0 0" | "top left" | "left top" => positions = vec![pc(0.0)],
                        "50% 0%" | "50% 0" | "center top" | "top center" => positions = vec![iden("top")],
                        "top right" | "right top" => positions = vec![pc(100.0), ws(0), pc(0.0)],
                        "0 50%" | "0% 50%" | "left center" | "center left" => positions = vec![iden("left")],
                        "100% 50%" | "right center" | "center right" => positions = vec![iden("right")],
                        "0 100%" | "0% 100%" | "left bottom" | "bottom left" => positions = vec![iden("bottom")],
                        "50% 100%" | "center bottom" | "bottom center" => positions = vec![iden("bottom")],
                        "100% 100%" | "right bottom" | "bottom right" => positions = vec![pc(100.0)],
                        _ => {
                            if position1 == position2 {
                                positions = positions.get(k).copied().into_iter().collect();
                            }
                        }
                    }
                }
            }
            let mut result: Vec<u32> = Vec::new();
            result.extend(form);
            if !size.is_empty() {
                if !result.is_empty() {
                    result.push(ws(0));
                }
                result.extend(size);
            }
            if !positions.is_empty() {
                if !result.is_empty() {
                    result.push(ws(0));
                }
                result.push(arena.create_token(EnumToken::IdenTokenType, TokenValue::Str("at"), None));
                result.push(ws(0));
                result.extend(positions);
            }
            if !color_space_def.is_empty() {
                if !result.is_empty() {
                    result.push(ws(0));
                }
                result.extend(color_space_def);
            }
            if !result.is_empty() {
                result.push(arena.create_token(EnumToken::CommaTokenType, TokenValue::None, None));
            }
            let mut stops: Vec<u32> = slice.get(i..).map_or_else(Vec::new, |s| s.to_vec());
            reduce_color_stops(arena, &mut stops);
            result.extend(stops);
            *slice = result;
        }
        "conic-gradient" | "repeating-conic-gradient" => {
            let mut i = 0usize;
            let mut angles: Vec<u32> = Vec::new();
            let mut positions: Vec<u32> = Vec::new();
            let mut color_space_def: Vec<u32> = Vec::new();
            if typ_of(arena, at(slice, i)) == EnumToken::IdenTokenType && str_eq(arena, at(slice, i), "from") {
                angles.push(slice[i]);
                i += 1;
                while i < slice.len() && is_ws_or_comment(arena, at(slice, i)) {
                    angles.push(slice[i]);
                    i += 1;
                }
                let tt = typ_of(arena, at(slice, i));
                if (tt == EnumToken::NumberTokenType || tt == EnumToken::AngleTokenType) && arena.numeric_value.get(to_degrees(arena, slice[i])) == 0.0 {
                    angles.clear();
                    i += 1;
                } else if tt != EnumToken::CommaTokenType && tt != EnumToken::IdenTokenType {
                    if let Some(&x) = slice.get(i) {
                        angles.push(x);
                    }
                    i += 1;
                }
                while i < slice.len() && is_ws_or_comment(arena, at(slice, i)) {
                    i += 1;
                }
            }
            if typ_of(arena, at(slice, i)) == EnumToken::IdenTokenType && str_eq(arena, at(slice, i), "at") {
                i += 1;
                while i < slice.len() && is_ws_or_comment(arena, at(slice, i)) {
                    i += 1;
                }
                let mut position1 = String::new();
                let mut position2 = String::new();
                for pos in 0..2 {
                    let tk = at(slice, i);
                    let tt = typ_of(arena, tk);
                    let value = if tt == EnumToken::IdenTokenType && !str_eq(arena, tk, "in") {
                        Some(arena.resolve_string(arena.string_value.get(tk.unwrap())).to_string())
                    } else if tt == EnumToken::PercentageTokenType || tt == EnumToken::NumberTokenType {
                        Some(format!("{}%", js_number_to_string(arena.numeric_value.get(tk.unwrap()))))
                    } else {
                        None
                    };
                    if let Some(v) = value {
                        if pos == 0 {
                            position1 = v;
                        } else {
                            position2 = v;
                        }
                        positions.push(slice[i]);
                        i += 1;
                    }
                    while i < slice.len() && is_ws_or_comment(arena, at(slice, i)) {
                        if pos == 0 {
                            positions.push(slice[i]);
                        }
                        i += 1;
                    }
                }
                if !position1.is_empty() {
                    let key = format!("{} {}", position1, position2).trim().to_string();
                    reducegradient_background_position(arena, &mut positions, &key);
                }
            }
            while i < slice.len() && is_ws_or_comment(arena, at(slice, i)) {
                i += 1;
            }
            if typ_of(arena, at(slice, i)) == EnumToken::IdenTokenType && str_eq(arena, at(slice, i), "in") {
                while i < slice.len() && typ_of(arena, at(slice, i)) != EnumToken::CommaTokenType {
                    color_space_def.push(slice[i]);
                    i += 1;
                }
            }
            if typ_of(arena, at(slice, i)) == EnumToken::CommaTokenType {
                i += 1;
            }
            let mut result: Vec<u32> = Vec::new();
            if !positions.is_empty() {
                if !angles.is_empty() {
                    angles.push(ws(0));
                }
                angles.push(arena.create_token(EnumToken::IdenTokenType, TokenValue::Str("at"), None));
                angles.push(ws(0));
                angles.extend(positions);
            }
            if !angles.is_empty() {
                result.extend(angles);
                result.push(arena.create_token(EnumToken::CommaTokenType, TokenValue::None, None));
            }
            if !color_space_def.is_empty() {
                if !result.is_empty() {
                    result.push(ws(0));
                }
                result.extend(color_space_def);
                result.push(arena.create_token(EnumToken::CommaTokenType, TokenValue::None, None));
            }
            let mut stops: Vec<u32> = slice.get(i..).map_or_else(Vec::new, |s| s.to_vec());
            reduce_conic_color_stops(arena, &mut stops);
            result.extend(stops);
            *slice = result;
        }
        _ => {}
    }
}

/// render a token (`renderToken()`)
fn render_token(arena: &AstArena, token: u32, options: &RenderOptions, cache: Option<&RenderCache>, reducer: Option<&Reducer>) -> String {
    let t = |x: u32| arena.data_types.get(x);
    let typ = t(token);
    let supports_options;
    let (options, reducer) = if typ == EnumToken::WhenElseFunctionTokenType && arena.string_equals_ignore_case(arena.string_value.get(token), "supports") {
        supports_options = RenderOptions { minify_values: Some(false), minify: Some(false), convert_color: Some(ConvertColor::Bool(false)), ..options.clone() };
        (&supports_options, None)
    } else {
        (options, reducer)
    };
    // `options.minifyValues ??= options.minify`
    let minify_values = options.minify_values.or(options.minify) == Some(true);
    let effective;
    let options = if options.minify_values.is_none() && options.minify.is_some() {
        effective = RenderOptions { minify_values: options.minify, ..options.clone() };
        &effective
    } else {
        options
    };
    let s = |id: u32| arena.resolve_string(id);
    let own_reducer;
    match typ {
        EnumToken::UrlFunctionTokenDefType
        | EnumToken::FunctionTokenDefType
        | EnumToken::MathFunctionTokenDefType
        | EnumToken::ImageFunctionTokenDefType
        | EnumToken::ColorFunctionTokenDefType
        | EnumToken::TimingFunctionTokenDefType
        | EnumToken::WhenElseFunctionTokenDefType
        | EnumToken::SupportsFunctionTokenDefType
        | EnumToken::TimelineFunctionTokenDefType
        | EnumToken::GridTemplateFuncTokenDefType
        | EnumToken::TransformFunctionTokenDefType
        | EnumToken::ContainerFunctionTokenDefType
        | EnumToken::PseudoClassFunctionTokenDefType
        | EnumToken::GeneralEnclosedFunctionTokenDefType
        | EnumToken::CustomFunctionTokenDefType
        | EnumToken::WildCardFunctionTokenDefType => format!("{}(", s(arena.string_value.get(token))),
        EnumToken::ListToken => render_list(arena, arena.first_token.get(token), options, cache, None),
        EnumToken::BinaryExpressionTokenType => {
            let op = arena.numeric_value.get(token) as u8;
            let l = arena.get_child_at(token, TokenField::L).map_or(0, |_| arena.field(token, TokenField::L));
            let r = arena.get_child_at(token, TokenField::R).map_or(0, |_| arena.field(token, TokenField::R));
            let needs_parens = |x: u32| t(x) == EnumToken::BinaryExpressionTokenType && matches!(arena.numeric_value.get(x) as u8, EnumToken::Add | EnumToken::Sub);
            if op == EnumToken::Mul || op == EnumToken::Div {
                let mut result = if needs_parens(l) {
                    format!("({})", render_value_with(arena, l, options, cache, None))
                } else {
                    render_value_with(arena, l, options, cache, None).into()
                };
                result.push(if op == EnumToken::Mul { '*' } else { '/' });
                if needs_parens(r) {
                    result.push_str(&format!("({})", render_value_with(arena, r, options, cache, None)));
                } else {
                    result.push_str(&render_value_with(arena, r, options, cache, None));
                }
                return result;
            }
            format!("{}{}{}", render_value_with(arena, l, options, cache, None), if op == EnumToken::Add { " + " } else { " - " }, render_value_with(arena, r, options, cache, None))
        }
        EnumToken::FractionTokenType => {
            let numerator = arena.first_child.get(token);
            let denominator = arena.last_child.get(token);
            let fraction = format!("{}/{}", render_value_default(arena, numerator), render_value_default(arena, denominator));
            if arena.numeric_value.get(denominator) != 0.0 {
                let value = minify_number(arena.numeric_value.get(numerator) / arena.numeric_value.get(denominator));
                if value.len() <= fraction.len() {
                    return value;
                }
            }
            fraction
        }
        EnumToken::Add => " + ".into(),
        EnumToken::Sub => " - ".into(),
        EnumToken::UniversalSelectorTokenType | EnumToken::Star | EnumToken::Mul => "*".into(),
        EnumToken::Div => "/".into(),
        EnumToken::WrappedValuesTokenType => format!("{{{}}}", render_list(arena, arena.first_token.get(token), options, cache, None)),
        EnumToken::ColorTokenType => {
            if arena.color_type.get(token) == ColorType::LIGHT_DARK
                || (arena.first_token.get(token) != 0 && options.convert_color == Some(ConvertColor::Bool(false)) && arena.token_count(token) == 2)
            {
                let mut result = String::new();
                for curr in arena.token_iter(token) {
                    if t(curr) == EnumToken::IdenTokenType {
                        parse_color(arena, curr);
                    }
                    result.push_str(&render_value_with(arena, curr, options, cache, None));
                }
                return format!("{}({})", s(arena.string_value.get(token)), result);
            }
            render_color(arena, token, options, cache, reducer)
        }
        EnumToken::UrlFunctionTokenType
        | EnumToken::ParensTokenType
        | EnumToken::FunctionTokenType
        | EnumToken::MathFunctionTokenType
        | EnumToken::ImageFunctionTokenType
        | EnumToken::TimingFunctionTokenType
        | EnumToken::PseudoClassFuncTokenType
        | EnumToken::WhenElseFunctionTokenType
        | EnumToken::TimelineFunctionTokenType
        | EnumToken::GridTemplateFuncTokenType
        | EnumToken::SupportsFunctionTokenType
        | EnumToken::ContainerFunctionTokenType
        | EnumToken::TransformFunctionTokenType
        | EnumToken::GeneralEnclosedFunctionTokenType
        | EnumToken::CustomFunctionTokenType
        | EnumToken::WildCardFunctionTokenType => {
            if typ == EnumToken::UrlFunctionTokenType && minify_values {
                if t(arena.first_token.get(token)) == EnumToken::BadUrlTokenType {
                    return "url()".into();
                }
                for child in arena.token_iter(token) {
                    if t(child) == EnumToken::StringTokenType {
                        let val = s(arena.string_value.get(child));
                        let inner: String = val.chars().skip(1).take(4).collect();
                        if inner != "data:" && url_token_matcher(&val) {
                            arena.data_types.set(child, EnumToken::UrlTokenTokenType);
                            let unquoted: String = {
                                let c: Vec<char> = val.chars().collect();
                                c[1..c.len() - 1].iter().collect()
                            };
                            arena.string_value.set(child, arena.intern_string(&unquoted));
                        }
                        break;
                    }
                }
            }
            if typ == EnumToken::ImageFunctionTokenType && minify_values {
                let mut slice = arena.tokens(token);
                minify_gradient(arena, token, &mut slice);
                let reducer_ref: &Reducer = match reducer {
                    Some(r) => r,
                    None => {
                        own_reducer = Reducer::new(options, cache);
                        &own_reducer
                    }
                };
                let mut result = String::new();
                for &tk in &slice {
                    reducer_ref.reduce(arena, &mut result, tk);
                }
                return format!("{}({})", s(arena.string_value.get(token)), result);
            }
            if typ == EnumToken::MathFunctionTokenType && arena.token_count(token) == 1 {
                let first = arena.first_token.get(token);
                let first_type = t(first);
                if first_type != EnumToken::BinaryExpressionTokenType
                    && first_type != EnumToken::FractionTokenType
                    && first_type != EnumToken::IdenTokenType
                    && !arena.has_fraction(first)
                {
                    let reducer_ref: &Reducer = match reducer {
                        Some(r) => r,
                        None => {
                            own_reducer = Reducer::new(options, cache);
                            &own_reducer
                        }
                    };
                    return format!("{}({})", s(arena.string_value.get(token)), render_value_with(arena, first, options, cache, Some(reducer_ref)));
                }
            }
            format!("{}({})", s(arena.string_value.get(token)), render_reduced(arena, arena.first_token.get(token), options, cache, reducer))
        }
        EnumToken::BlockStartTokenType => "{".into(),
        EnumToken::BlockEndTokenType => "}".into(),
        EnumToken::StartParensTokenType => "(".into(),
        EnumToken::DelimTokenType | EnumToken::EqualMatchTokenType => "=".into(),
        EnumToken::IncludeMatchTokenType => "~=".into(),
        EnumToken::DashMatchTokenType => "|=".into(),
        EnumToken::StartMatchTokenType => "^=".into(),
        EnumToken::EndMatchTokenType => "$=".into(),
        EnumToken::ContainMatchTokenType => "*=".into(),
        EnumToken::LtTokenType => "<".into(),
        EnumToken::LteTokenType => "<=".into(),
        EnumToken::Tilda | EnumToken::SubsequentSiblingCombinatorTokenType => "~".into(),
        EnumToken::Plus | EnumToken::NextSiblingCombinatorTokenType => "+".into(),
        EnumToken::GtTokenType | EnumToken::ChildCombinatorTokenType => ">".into(),
        EnumToken::GteTokenType => ">=".into(),
        EnumToken::ColumnCombinatorTokenType => "||".into(),
        EnumToken::EndParensTokenType => ")".into(),
        EnumToken::AttrStartTokenType => "[".into(),
        EnumToken::AttrEndTokenType => "]".into(),
        EnumToken::DescendantCombinatorTokenType | EnumToken::WhitespaceTokenType => " ".into(),
        EnumToken::ColonTokenType => ":".into(),
        EnumToken::DoubleColonTokenType => "::".into(),
        EnumToken::SemiColonTokenType => ";".into(),
        EnumToken::CommaTokenType => ",".into(),
        EnumToken::ImportantTokenType => "!important".into(),
        EnumToken::PipeTokenType => "|".into(),
        EnumToken::AttrTokenType | EnumToken::IdenListTokenType => format!("[{}]", render_reduced(arena, arena.first_token.get(token), options, cache, reducer)),
        EnumToken::TimeTokenType
        | EnumToken::AngleTokenType
        | EnumToken::LengthTokenType
        | EnumToken::DimensionTokenType
        | EnumToken::FrequencyTokenType
        | EnumToken::ResolutionTokenType
        | EnumToken::UserDefinedDimensionTokenType => render_dimension(arena, token, options, cache, minify_values),
        EnumToken::FlexTokenType | EnumToken::PercentageTokenType => {
            let uni = if typ == EnumToken::PercentageTokenType { "%" } else { "fr" };
            let perc = if arena.has_fraction(token) {
                String::from(render_value_with(arena, arena.get_fraction(token), options, cache, None))
            } else {
                minify_number(arena.numeric_value.get(token))
            };
            if minify_values && perc == "0" {
                "0".into()
            } else if perc.contains('/') {
                perc.replacen('/', &format!("{}/", uni), 1)
            } else {
                format!("{}{}", perc, uni)
            }
        }
        EnumToken::NumberTokenType => {
            if arena.has_fraction(token) {
                String::from(render_value_with(arena, arena.get_fraction(token), options, cache, None))
            } else {
                minify_number(arena.numeric_value.get(token))
            }
        }
        EnumToken::InfinityTokenType => "1/0".into(),
        EnumToken::NegativeInfinityTokenType => "-1/0".into(),
        EnumToken::NaNTokenType => "NaN".into(),
        EnumToken::AtRuleTokenType => format!("@{}", s(arena.name.get(token))),
        EnumToken::CommentTokenType | EnumToken::CDOCOMMNodeType => {
            if options.remove_comments == Some(true)
                && (options.preserve_license != Some(true) || !arena.string_starts_with(arena.string_value.get(token), "/*!"))
            {
                return String::new();
            }
            s(arena.string_value.get(token)).to_string()
        }
        EnumToken::PseudoClassTokenType | EnumToken::PseudoElementTokenType => {
            let v = s(arena.string_value.get(token));
            if typ == EnumToken::PseudoElementTokenType {
                let rest: String = v.chars().skip(1).collect();
                if PSEUDO_ELEMENTS.contains(&rest.as_str()) {
                    return rest;
                }
            }
            v.to_string()
        }
        EnumToken::CssTypeTokenType
        | EnumToken::UrlTokenTokenType
        | EnumToken::HashTokenType
        | EnumToken::IdenTokenType
        | EnumToken::StringTokenType
        | EnumToken::LiteralTokenType
        | EnumToken::UniCodeRange
        | EnumToken::DashedIdenTokenType
        | EnumToken::PseudoPageTokenType
        | EnumToken::ClassSelectorTokenType => s(arena.string_value.get(token)).to_string(),
        EnumToken::NestingSelectorTokenType => "&".into(),
        EnumToken::SupportsQueryUnaryConditionTokenType | EnumToken::WhenElseUnaryConditionTokenType | EnumToken::MediaQueryUnaryFeatureTokenType => {
            format!(
                "{} {}",
                render_value_with(arena, arena.field(token, TokenField::L), options, cache, reducer),
                render_field(arena, token, TokenField::R, options, cache, reducer)
            )
        }
        EnumToken::SupportsQueryConditionTokenType | EnumToken::WhenElseQueryConditionTokenType => format!(
            "{} {} {}",
            render_field(arena, token, TokenField::L, options, cache, reducer),
            render_value_with(arena, arena.field(token, TokenField::Op), options, cache, reducer),
            render_field(arena, token, TokenField::R, options, cache, reducer)
        ),
        EnumToken::IfConditionTokenType => {
            if !arena.has_field(token, TokenField::L) {
                return String::new();
            }
            format!(
                "{}:{}{}",
                render_field(arena, token, TokenField::L, options, cache, reducer),
                options.indent.as_deref().unwrap_or(""),
                render_field(arena, token, TokenField::R, options, cache, reducer)
            )
        }
        EnumToken::IfElseConditionTokenType => format!(
            "{}{}{}",
            render_value_with(arena, arena.field(token, TokenField::L), options, cache, reducer),
            options.indent.as_deref().unwrap_or(""),
            render_value_with(arena, arena.field(token, TokenField::R), options, cache, reducer)
        ),
        EnumToken::DeclarationNodeType => {
            let mut values = arena.tokens(token);
            if minify_values {
                filter_values(arena, &mut values);
            }
            let mut result = format!("{}:", s(arena.name.get(token)));
            for v in values {
                result.push_str(&render_value_with(arena, v, options, cache, reducer));
            }
            result
        }
        EnumToken::MediaQueryConditionTokenType => {
            let op = arena.field(token, TokenField::Op);
            let indent = if matches!(
                t(op),
                EnumToken::LtTokenType | EnumToken::GtTokenType | EnumToken::ColonTokenType | EnumToken::DelimTokenType | EnumToken::LteTokenType | EnumToken::GteTokenType
            ) {
                ""
            } else {
                " "
            };
            format!(
                "{}{}{}{}{}",
                render_field(arena, token, TokenField::L, options, cache, reducer),
                indent,
                render_value_with(arena, op, options, cache, reducer),
                indent,
                render_field(arena, token, TokenField::R, options, cache, reducer)
            )
        }
        EnumToken::MediaRangeQueryTokenType => format!(
            "{}{}{}{}{}",
            render_field(arena, token, TokenField::L, options, cache, reducer),
            render_value_with(arena, arena.field(token, TokenField::Op), options, cache, reducer),
            render_field(arena, token, TokenField::Val, options, cache, reducer),
            render_value_with(arena, arena.field(token, TokenField::Op2), options, cache, reducer),
            render_field(arena, token, TokenField::R, options, cache, reducer)
        ),
        EnumToken::MediaFeatureTokenType => s(arena.string_value.get(token)).to_string(),
        EnumToken::NotTokenType => "not".into(),
        EnumToken::OnlyTokenType => "only".into(),
        EnumToken::AndTokenType => "and".into(),
        EnumToken::OrTokenType => "or".into(),
        EnumToken::BadCommentTokenType | EnumToken::BadCdoTokenType | EnumToken::BadStringTokenType | EnumToken::BadUrlTokenType | EnumToken::EOFTokenType => {
            String::new()
        }
        EnumToken::ComposesSelectorNodeType => format!(
            "{}{}",
            render_field(arena, token, TokenField::L, options, cache, reducer),
            if !arena.has_field(token, TokenField::R) {
                String::new()
            } else {
                format!(" from {}", render_value_with(arena, arena.field(token, TokenField::R), options, cache, reducer))
            }
        ),
        // typescript throws `Unsupported token type`
        _ => String::new(),
    }
}

/// render a dimension: the angles are rendered in the shortest unit, the zero dimensions without unit
fn render_dimension(arena: &AstArena, token: u32, options: &RenderOptions, cache: Option<&RenderCache>, minify_values: bool) -> String {
    let typ = arena.data_types.get(token);
    let mut val = if arena.has_fraction(token) {
        String::from(render_value_with(arena, arena.get_fraction(token), options, cache, None))
    } else {
        minify_number(arena.numeric_value.get(token))
    };
    let mut unit = arena.resolve_string(arena.unit.get(token)).to_string();
    if typ == EnumToken::AngleTokenType && !val.contains('/') {
        let angle = get_angle(arena, token);
        let mut value = format!("{}{}", val, unit);
        let original_unit = unit.clone();
        for u in ["deg", "turn", "rad", "grad"] {
            if original_unit == u {
                continue;
            }
            let (factor, extra) = match u {
                "deg" => (360.0, 3),
                "turn" => (1.0, 4),
                "rad" => (2.0 * std::f64::consts::PI, 3),
                _ => (400.0, 4),
            };
            let v = minify_str(&to_fixed(to_precision_angle(angle * factor, anglePrecision, false), anglePrecision as usize));
            if v.len() + extra < value.len() {
                val = v.clone();
                unit = u.to_string();
                value = format!("{}{}", v, u);
            }
        }
    }
    if val == "0" {
        return match typ {
            EnumToken::TimeTokenType => "0s".into(),
            EnumToken::FrequencyTokenType => "0Hz".into(),
            EnumToken::ResolutionTokenType => "0x".into(),
            _ => {
                if minify_values {
                    "0".into()
                } else {
                    format!("0{}", unit)
                }
            }
        };
    }
    if typ == EnumToken::TimeTokenType {
        if unit == "ms" {
            let v = minify_number(js_parse_number(&val) / 1000.0);
            if v.len() < val.len() {
                return format!("{}s", v);
            }
            return format!("{}ms", val);
        }
        return format!("{}s", val);
    }
    if typ == EnumToken::ResolutionTokenType && unit == "dppx" {
        unit = "x".into();
    }
    if val.contains('/') {
        val.replacen('/', &format!("{}/", unit), 1)
    } else {
        format!("{}{}", minify_number(to_precision_value(js_parse_number(&val), crate::syntax::constants::colorPrecision)), unit)
    }
}

/// remove the whitespace that is not needed: before `!important`, after a function
pub fn filter_values(arena: &AstArena, values: &mut Vec<u32>) {
    let t = |x: u32| arena.data_types.get(x);
    let mut i = 0usize;
    while i < values.len() {
        if t(values[i]) == EnumToken::ImportantTokenType && i > 0 && t(values[i - 1]) == EnumToken::WhitespaceTokenType {
            values.remove(i - 1);
        } else if is_function_type(t(values[i]))
            && (t(values[i]) != EnumToken::ColorTokenType || arena.first_token.get(values[i]) != 0)
            && t(values[i]) != EnumToken::WildCardFunctionTokenType
            && values.get(i + 1).is_some_and(|&n| t(n) == EnumToken::WhitespaceTokenType)
        {
            values.remove(i + 1);
        }
        i += 1;
    }
}
