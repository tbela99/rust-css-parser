//! Port of `src/parser/parse.ts`.
//!
//! `doParse()` / `doParseSync()`: see `parse_main.rs`. this module holds the statement parsers (`parseNode()`,
//! `parseAtRule()`), the token parsers (`parseString()`, `parseTokens()`) and their helpers.

use crate::ast::arena::{AstArena, StrOrId, TokenField, TokenValue};
use crate::ast::types::{EnumAstNodeStatus, EnumToken};
use crate::options::{ConvertColor, ParserOptions};
use crate::parser::input::CssInput;
use crate::parser::source::SourceFile;
use crate::parser::tokenize::{create_parse_info, TokenValueField, Tokenizer};
use crate::parser::utils::at_rule::{match_at_rule_syntax, parse_at_rule_font_feature_values};
use crate::parser::utils::at_rule_container::parse_at_rule_container_query_list;
use crate::parser::utils::at_rule_generic::match_generic_syntax;
use crate::parser::utils::at_rule_import::match_at_rule_import_syntax;
use crate::parser::utils::at_rule_media::parse_mediaquery_list;
use crate::parser::utils::at_rule_support::parse_at_rule_support_syntax;
use crate::parser::utils::at_rule_when_else::match_at_rule_when_else_syntax;
use crate::parser::utils::declaration::parse_declaration;
use crate::parser::utils::selector::{is_trim_white_space, parse_selector};
use crate::parser::utils::text::{camelize, dasherize, equals_ignore_case};
use crate::parser::utils::token::trim_white_space_tokens;
use crate::printer::render::{render_value, render_value_default};
use crate::syntax::constants::{is_function_def_type, tokens_func_def_map, PAGE_MARGIN_BOX_TYPE};
use crate::syntax::syntax::{is_color, parse_color};
use crate::types::ErrorDescription;
use crate::validation::config::get_syntax_rule;
use crate::validation::match_::{create_validation_context, drop_error, match_all_syntaxes, match_selector_syntax, token_location, trim_array};
use crate::validation::parser::typedef::ValidationSyntaxGroupEnum;
use std::rc::Rc;

pub use super::parse_main::*;

/// `ParseResultStats`
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParseResultStats {
    pub src: String,
    pub bytes_in: u64,
    pub imported_bytes_in: u64,
    pub tokenize: String,
    pub parse: String,
    pub minify: String,
    pub module: Option<String>,
    pub total: String,
    pub imports: Vec<ParseResultStats>,
    pub nodes_count: u64,
    pub tokens_count: u64,
}

impl ParseResultStats {
    pub fn new(src: &str) -> Self {
        Self {
            src: src.to_string(),
            tokenize: "0ms".into(),
            parse: "0ms".into(),
            minify: "0ms".into(),
            total: "0ms".into(),
            ..Default::default()
        }
    }
}

/// render tokens (`renderTokens()`)
pub fn render_tokens(arena: &AstArena, tokens: &[u32], options: Option<&ParserOptions>) -> String {
    tokens
        .iter()
        .map(|&t| match options {
            Some(o) => render_value(arena, t, o),
            None => render_value_default(arena, t),
        })
        .collect()
}

/// allocate an at-rule node from an at-rule token
#[allow(clippy::too_many_arguments)]
pub fn create_at_rule_node(
    arena: &AstArena,
    at_rule: u32,
    typ: u8,
    tokens: &[u32],
    state: u32,
    errors: Vec<ErrorDescription>,
    end: u32,
    val: &str,
    block: bool,
) -> u32 {
    let node = arena.create_at_rule(typ, StrOrId::Id(arena.name.get(at_rule)), StrOrId::Str(val), block, Some(tokens), state, Some(errors));
    arena.span_location(node, at_rule, end);
    node
}

/// allocate a comment node from a comment token
pub fn create_comment_node(arena: &AstArena, token: u32) -> u32 {
    let node = arena.create_comment(arena.data_types.get(token), StrOrId::Id(arena.string_value.get(token)));
    arena.copy_location(node, token);
    node
}

/// initial arena capacity: one node for every 4 bytes of css
pub fn arena_capacity(tokenizer: &Tokenizer) -> usize {
    let length = tokenizer.stream_length();
    if length == 0 {
        4096
    } else {
        (1usize << 25).min(length.div_ceil(4) + 1024)
    }
}

/// allocate the current tokenizer token in the arena
pub fn allocate_token(arena: &AstArena, tokenizer: &mut Tokenizer, source_index: i32) -> u32 {
    let item = arena.allocate();
    let typ = tokenizer.typ.unwrap_or(0);
    arena.data_types.set(item, typ);
    match tokenizer.field {
        TokenValueField::Unit => {
            arena.numeric_value.set(item, tokenizer.num);
            let id = tokenizer.intern_value(arena, source_index);
            arena.unit.set(item, id);
        }
        TokenValueField::Name => {
            let id = tokenizer.intern_value(arena, source_index);
            arena.name.set(item, id);
        }
        TokenValueField::Number => arena.numeric_value.set(item, tokenizer.num),
        TokenValueField::Value => {
            if typ != EnumToken::WhitespaceTokenType {
                let id = tokenizer.intern_value(arena, source_index);
                arena.string_value.set(item, id);
            }
            if let Some(kin) = tokenizer.kin {
                arena.color_type.set(item, kin);
            }
        }
        TokenValueField::None => {}
    }
    if source_index >= 0 {
        arena.set_token_location(item, source_index as u32, tokenizer.sta.unwrap_or(0) as u32, tokenizer.end.unwrap_or(0) as u32);
    }
    item
}

/// recycle the tokens of a statement that were not attached to the tree
pub fn recycle_statement_tokens(
    arena: &AstArena,
    tokens: &[u32],
    errors: &[ErrorDescription],
    error_start: usize,
    invalid_nodes: &[u32],
    invalid_start: usize,
) {
    for &token in tokens {
        if arena.data_types.get(token) == 0 || arena.parents.get(token) != 0 || arena.first_child.get(token) != 0 || arena.first_token.get(token) != 0 {
            continue;
        }
        let mut referenced = invalid_nodes.get(invalid_start..).is_some_and(|v| v.contains(&token));
        if !referenced {
            referenced = errors.get(error_start..).is_some_and(|e| e.iter().any(|e| e.node == Some(token)));
        }
        if !referenced {
            arena.recycle(token);
        }
    }
}

/// bad comment, cdo, url or string token
pub fn is_bad_token(t: u8) -> bool {
    matches!(t, EnumToken::BadCommentTokenType | EnumToken::BadCdoTokenType | EnumToken::BadUrlTokenType | EnumToken::BadStringTokenType)
}

/// transform the case of a key name (`getKeyName()`)
pub fn get_key_name(key: &str, how: u32) -> String {
    use crate::ast::types::ModuleCaseTransformEnum as M;
    // the case transforms are flags
    if how & (M::CamelCase | M::CamelCaseOnly) != 0 {
        camelize(key)
    } else if how & (M::DashCase | M::DashCaseOnly) != 0 {
        dasherize(key)
    } else {
        key.to_string()
    }
}

const INVALID_STATES: u32 = EnumAstNodeStatus::Invalid
    | EnumAstNodeStatus::Disallowed
    | EnumAstNodeStatus::Unknown
    | EnumAstNodeStatus::Unparsed
    | EnumAstNodeStatus::Malformed;

/// parse a statement: an at-rule, a rule or a declaration
#[allow(clippy::too_many_arguments)]
pub fn parse_node(
    arena: &AstArena,
    tokens: &mut Vec<u32>,
    context: u32,
    options: &ParserOptions,
    errors: &mut Vec<ErrorDescription>,
    stats: &mut ParseResultStats,
    invalid_nodes: &mut Vec<u32>,
    parens_match: i32,
) -> Option<u32> {
    let t = |x: u32| arena.data_types.get(x);
    let context_type = t(context);
    if tokens.last().is_some_and(|&l| t(l) == EnumToken::EOFTokenType) {
        tokens.pop();
        let mut match_count = parens_match;
        if match_count > 0 {
            let mut k = tokens.len() as isize;
            loop {
                let cont = k > 0;
                k -= 1;
                if !(cont && matches!(t(tokens[k as usize]), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType)) {
                    break;
                }
            }
            if k >= 0 && t(tokens[k as usize]) == EnumToken::SemiColonTokenType {
                match_count = 0;
            }
            while match_count > 0 {
                let closing = arena.create_token(EnumToken::EndParensTokenType, TokenValue::None, None);
                if k >= 0 {
                    arena.copy_location(closing, tokens[k as usize]);
                }
                tokens.push(closing);
                match_count -= 1;
            }
        }
    }
    let cdo_error = |errors: &mut Vec<ErrorDescription>, token: u32| {
        errors.push(drop_error(arena, format!("CDOCOMM not allowed here {}", token), token, options));
        arena.data_types.set(token, EnumToken::BadCdoTokenType);
    };
    let mut i = 0usize;
    while i < tokens.len() {
        let tk = tokens[i];
        if t(tk) == EnumToken::CDOCOMMTokenType && context_type != EnumToken::StyleSheetNodeType {
            cdo_error(errors, tk);
            i += 1;
            continue;
        }
        if matches!(t(tk), EnumToken::CommentTokenType | EnumToken::CDOCOMMTokenType | EnumToken::WhitespaceTokenType) {
            i += 1;
            continue;
        }
        break;
    }
    if i > 0 {
        for k in 0..i {
            if t(tokens[k]) != EnumToken::WhitespaceTokenType {
                let comment = create_comment_node(arena, tokens[k]);
                arena.append_child_list(context, &[comment]);
            }
        }
        tokens.drain(..i);
    }
    let mut i = 0usize;
    while i < tokens.len() {
        let tk = tokens[i];
        if matches!(t(tk), EnumToken::CommentTokenType | EnumToken::CDOCOMMTokenType) {
            if t(tk) == EnumToken::CDOCOMMTokenType && context_type != EnumToken::StyleSheetNodeType {
                cdo_error(errors, tk);
                i += 1;
                continue;
            }
            let comment = create_comment_node(arena, tk);
            arena.append_child_list(context, &[comment]);
            stats.nodes_count += 1;
        } else if t(tk) != EnumToken::WhitespaceTokenType {
            break;
        }
        i += 1;
    }
    let delim = *tokens.last()?;
    if matches!(t(delim), EnumToken::SemiColonTokenType | EnumToken::BlockStartTokenType | EnumToken::BlockEndTokenType) {
        tokens.pop();
    }
    if tokens.is_empty() {
        return None;
    }
    if t(tokens[0]) == EnumToken::AtRuleTokenType {
        let mut nested_rule = false;
        let mut parent = context;
        while parent != 0 {
            if t(parent) == EnumToken::RuleNodeType {
                nested_rule = true;
                break;
            }
            parent = arena.parents.get(parent);
        }
        let at_options = ParserOptions { nested_rule: Some(nested_rule), ..options.clone() };
        let node = parse_at_rule(arena, tokens, context, &at_options, errors, Some(t(delim) == EnumToken::BlockStartTokenType))?;
        if arena.flags.get(node) & INVALID_STATES != 0 {
            invalid_nodes.push(node);
        }
        stats.nodes_count += 1;
        arena.append_child_list(context, &[node]);
        return Some(node);
    }
    stats.nodes_count += 1;
    if t(delim) == EnumToken::BlockStartTokenType {
        let node = parse_selector(arena, tokens, Some(context), options, errors);
        arena.append_child_list(context, &[node]);
        if arena.flags.get(node) & INVALID_STATES != 0 {
            invalid_nodes.push(node);
        }
        return Some(node);
    }
    let node = parse_declaration(arena, tokens, Some(context), options, errors);
    if context_type == EnumToken::StyleSheetNodeType && t(node) == EnumToken::DeclarationNodeType {
        arena.flags.set(node, arena.flags.get(node) | EnumAstNodeStatus::Invalid);
        let mut error = ErrorDescription::drop("<declaration> not allowed in <stylesheet>").with_node(node);
        error.location = token_location(arena, node, options);
        errors.push(error);
    } else if options.lenient == Some(true) || t(node) == EnumToken::DeclarationNodeType {
        arena.append_child_list(context, &[node]);
    }
    if arena.flags.get(node) & INVALID_STATES != 0 {
        invalid_nodes.push(node);
    }
    None
}

/// detach the invalid nodes. the unknown nodes are kept in lenient mode
pub fn drop_invalid_nodes(arena: &AstArena, invalid_nodes: &[u32], options: &ParserOptions) {
    for &node in invalid_nodes {
        if options.lenient == Some(true) && arena.flags.get(node) & EnumAstNodeStatus::Unknown != 0 {
            continue;
        }
        if arena.flags.get(node) & INVALID_STATES != 0 {
            arena.detach(node);
        }
    }
}

/// true if the node is an at-rule whose block is kept when it is empty: `@apply`, `@mixin`, `@function`
pub fn is_definition_block(arena: &AstArena, node: u32) -> bool {
    arena.data_types.get(node) == EnumToken::AtRuleNodeType
        && (arena.string_equals(arena.name.get(node), "apply")
            || arena.string_equals(arena.name.get(node), "mixin")
            || arena.string_equals(arena.name.get(node), "function"))
}

/// true if a mixin can be defined in the node
fn is_mixin_scope(arena: &AstArena, mut node: u32) -> bool {
    while node != 0 && arena.data_types.get(node) != EnumToken::StyleSheetNodeType {
        if arena.data_types.get(node) != EnumToken::AtRuleNodeType
            || !arena.with_string(arena.name.get(node), |n| matches!(n, "media" | "supports" | "container" | "layer" | "when" | "else"))
        {
            return false;
        }
        node = arena.parents.get(node);
    }
    true
}

/// `/^-[a-z]+-/` removed, lowercase
fn unprefixed_at_rule_name(nam: &str) -> String {
    if let Some(rest) = nam.strip_prefix('-') {
        let letters = rest.bytes().take_while(|b| b.is_ascii_lowercase()).count();
        if letters > 0 && rest.as_bytes().get(letters) == Some(&b'-') {
            return rest[letters + 1..].to_lowercase();
        }
    }
    nam.to_lowercase()
}

fn last_or(stream: &[u32], default: u32) -> u32 {
    stream.last().copied().unwrap_or(default)
}

/// render the tokens of `@namespace` / `@import`: the comments and the whitespace before a comment are dropped
fn render_skip_comments(arena: &AstArena, stream: &[u32], options: &ParserOptions) -> String {
    let t = |x: u32| arena.data_types.get(x);
    let mut acc = String::new();
    for (index, &tk) in stream.iter().enumerate() {
        let skip = t(tk) == EnumToken::CommentTokenType
            || (t(tk) == EnumToken::WhitespaceTokenType
                && stream.get(index + 1).is_some_and(|&n| t(n) == EnumToken::CommentTokenType)
                && (stream.len() < index + 3 || stream.get(index + 2).is_some_and(|&n| t(n) == EnumToken::WhitespaceTokenType)));
        if !skip {
            acc.push_str(&render_value(arena, tk, options));
        }
    }
    acc
}

/// parse an at-rule
pub fn parse_at_rule(
    arena: &AstArena,
    stream: &mut Vec<u32>,
    context: u32,
    options: &ParserOptions,
    errors: &mut Vec<ErrorDescription>,
    parse_as_block: Option<bool>,
) -> Option<u32> {
    let t = |x: u32| arena.data_types.get(x);
    let mut success = true;
    let context_type = t(context);
    let context_name: Option<String> = (context_type == EnumToken::AtRuleNodeType).then(|| arena.get_name(context).to_string());
    let at_rule = stream.remove(0);
    let nam = arena.get_name(at_rule).to_string();
    let at_rule_name = if nam.starts_with('-') { unprefixed_at_rule_name(&nam) } else { nam.clone() };
    let mut at_rule_end = at_rule;
    let last_error = |errors: &Vec<ErrorDescription>| errors.last().cloned().into_iter().collect::<Vec<_>>();
    let syntax_rules = get_syntax_rule(ValidationSyntaxGroupEnum::AtRules, &format!("@{}", nam));
    let syntax: Option<Vec<_>> = syntax_rules.as_ref().and_then(|r| r.get_prelude_rules()).map(|p| p.iter().skip(1).cloned().collect());
    let block_allowed = syntax_rules.as_ref().is_some_and(|r| r.get_block_rules().is_some());
    let as_block = |p: Option<bool>| p.unwrap_or(false);
    let Some(syntax_rules) = syntax_rules else {
        errors.push(drop_error(arena, "unknown at-rule".into(), at_rule, options));
        let (ok, result_errors) = match_generic_syntax(arena, stream, options);
        parse_tokens(arena, stream, None, None);
        let state = if ok { EnumAstNodeStatus::Unknown } else { EnumAstNodeStatus::Invalid };
        let mut node_errors = last_error(errors);
        if !ok {
            node_errors.extend(result_errors);
        }
        trim_array(arena, stream);
        let val = render_tokens(arena, stream, Some(options));
        return Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, stream, state, node_errors, at_rule_end, &val, as_block(parse_as_block)));
    };
    if context_name.as_deref() == Some("page") && PAGE_MARGIN_BOX_TYPE.contains(&at_rule_name.to_lowercase().as_str()) {
        let invalid = |stream: &mut Vec<u32>, errors: &mut Vec<ErrorDescription>| {
            parse_tokens(arena, stream, None, None);
            let node_errors = last_error(errors);
            trim_array(arena, stream);
            let val = render_tokens(arena, stream, Some(options));
            Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, stream, EnumAstNodeStatus::Invalid, node_errors, at_rule, &val, as_block(parse_as_block)))
        };
        if parse_as_block == Some(false) && !syntax_rules.optional_body {
            errors.push(drop_error(arena, "at-rule block is required".into(), at_rule, options));
            return invalid(stream, errors);
        }
        if let Some(token) = stream.iter().copied().find(|&x| t(x) != EnumToken::WhitespaceTokenType && t(x) == EnumToken::CommentTokenType) {
            errors.push(drop_error(arena, "unexpected token".into(), token, options));
            return invalid(stream, errors);
        }
    }
    let parse_as_block = parse_as_block.unwrap_or(block_allowed);
    if syntax.is_some() && nam != "layer" && parse_as_block != block_allowed && !syntax_rules.optional_body {
        errors.push(drop_error(arena, if parse_as_block { "at-rule block not supported" } else { "at-rule block is required" }.into(), at_rule, options));
        parse_tokens(arena, stream, None, None);
        let node_errors = last_error(errors);
        trim_array(arena, stream);
        let val = render_tokens(arena, stream, Some(options));
        return Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, stream, EnumAstNodeStatus::Invalid, node_errors, at_rule_end, &val, parse_as_block));
    }
    let no_color = |o: &ParserOptions| ParserOptions { convert_color: Some(ConvertColor::Bool(false)), ..o.clone() };
    match at_rule_name.as_str() {
        "charset" => {
            let mut ok = true;
            let first = stream.first().copied();
            let is_single_space = first.is_some_and(|f| {
                t(f) == EnumToken::WhitespaceTokenType
                    && arena.get_span_end(f) - arena.get_span_start(f) == 1
                    && arena.source_text(arena.get_source_id(f) as usize).is_some_and(|c| c.at(arena.get_span_start(f) as usize) == 0x20)
            });
            if !is_single_space {
                ok = false;
                errors.push(drop_error(arena, "expecting <space>".into(), first.unwrap_or(at_rule), options));
            } else if stream.get(1).is_none_or(|&s| t(s) != EnumToken::StringTokenType) {
                ok = false;
                errors.push(drop_error(arena, "expecting <string>".into(), stream.get(1).copied().unwrap_or(at_rule), options));
            }
            if ok && !arena.string_starts_with(arena.string_value.get(stream[1]), "\"") {
                ok = false;
                errors.push(drop_error(arena, "expecting double-quoted string".into(), stream[1], options));
            }
            if !ok {
                let node_errors = last_error(errors);
                at_rule_end = last_or(stream, at_rule);
                trim_array(arena, stream);
                let val = render_tokens(arena, stream, Some(options));
                return Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, stream, EnumAstNodeStatus::Invalid, node_errors, at_rule_end, &val, false));
            }
            if options.remove_charset == Some(true) {
                return None;
            }
            at_rule_end = last_or(stream, at_rule);
            trim_array(arena, stream);
            let val = render_tokens(arena, stream, Some(options));
            Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, stream, EnumAstNodeStatus::Validated, Vec::new(), at_rule_end, &val, false))
        }
        "font-feature-values" => {
            let (ok, result_errors) = parse_at_rule_font_feature_values(arena, stream, at_rule, options);
            errors.extend(result_errors.iter().cloned());
            at_rule_end = last_or(stream, at_rule);
            trim_white_space_tokens(arena, stream);
            let val = render_tokens(arena, stream, Some(options));
            let state = if ok { EnumAstNodeStatus::Validated } else { EnumAstNodeStatus::Invalid };
            Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, stream, state, if ok { Vec::new() } else { result_errors }, at_rule_end, &val, true))
        }
        "stylistic" | "historical-forms" | "character-variant" | "swash" | "ornaments" | "annotation" => {
            let mut ok = context_name.as_deref() == Some("font-feature-values");
            if !ok {
                errors.push(drop_error(arena, format!("unexpected at-rule {}", nam), at_rule, options));
            }
            if ok {
                for &token in stream.iter() {
                    if !matches!(t(token), EnumToken::CommentTokenType | EnumToken::WhitespaceTokenType) {
                        ok = false;
                        errors.push(drop_error(arena, "unexpected token".into(), token, options));
                    }
                }
            }
            at_rule_end = last_or(stream, at_rule);
            let node_errors = last_error(errors);
            trim_white_space_tokens(arena, stream);
            let val = render_tokens(arena, stream, Some(options));
            let state = if ok { EnumAstNodeStatus::Validated } else { EnumAstNodeStatus::Invalid };
            Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, stream, state, node_errors, at_rule_end, &val, true))
        }
        "container" => {
            let (ok, result_errors) = parse_at_rule_container_query_list(arena, stream, at_rule, options);
            errors.extend(result_errors.iter().cloned());
            at_rule_end = last_or(stream, at_rule);
            trim_white_space_tokens(arena, stream);
            let val = render_tokens(arena, stream, Some(options));
            let state = if ok { EnumAstNodeStatus::Validated } else { EnumAstNodeStatus::Invalid };
            Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, stream, state, if ok { Vec::new() } else { result_errors }, at_rule_end, &val, true))
        }
        "custom-media" => {
            let mut tokens: Vec<u32> = stream.iter().skip(1).copied().collect();
            trim_array(arena, &mut tokens);
            let result = match_all_syntaxes(arena, syntax.as_deref(), &create_validation_context(arena, &tokens), options);
            errors.extend(result.errors.iter().cloned());
            let options = no_color(options);
            at_rule_end = last_or(&tokens, at_rule);
            let state = if success { EnumAstNodeStatus::Validated } else { EnumAstNodeStatus::ValidationFailed };
            let node_errors = if result.success { Vec::new() } else { result.errors };
            trim_white_space_tokens(arena, &mut tokens);
            let val = render_tokens(arena, &tokens, Some(&options));
            Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, &tokens, state, node_errors, at_rule_end, &val, false))
        }
        "keyframes" => {
            let mut tokens: Vec<u32> = stream.iter().skip(1).copied().collect();
            trim_array(arena, &mut tokens);
            let filtered: Vec<u32> = stream.iter().copied().filter(|&x| !matches!(t(x), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType)).collect();
            if filtered.len() != 1 || !matches!(t(filtered[0]), EnumToken::IdenTokenType | EnumToken::StringTokenType | EnumToken::DashedIdenTokenType) {
                errors.push(drop_error(arena, "expected <keyframe-name>".into(), at_rule, options));
                success = false;
            }
            let options = no_color(options);
            at_rule_end = last_or(&tokens, at_rule);
            let state = if success { EnumAstNodeStatus::Validated } else { EnumAstNodeStatus::Invalid };
            let node_errors = if success { Vec::new() } else { last_error(errors) };
            let val = render_tokens(arena, &tokens, Some(&options));
            Some(create_at_rule_node(arena, at_rule, EnumToken::KeyframesAtRuleNodeType, &tokens, state, node_errors, at_rule_end, &val, true))
        }
        "namespace" => {
            let result = match_all_syntaxes(arena, syntax.as_deref(), &create_validation_context(arena, stream), options);
            if !result.success {
                errors.extend(result.errors.iter().cloned());
            }
            let valid = block_allowed == parse_as_block && result.success;
            if valid {
                let mut start = 0usize;
                let mut end: isize = -1;
                let mut has_string = false;
                while start < stream.len() {
                    if t(stream[start]) == EnumToken::UrlFunctionTokenDefType {
                        start += 1;
                        end = start as isize;
                        while (end as usize) < stream.len() {
                            if t(stream[end as usize]) == EnumToken::EndParensTokenType {
                                break;
                            }
                            if t(stream[end as usize]) == EnumToken::StringTokenType {
                                has_string = true;
                            }
                            end += 1;
                        }
                        break;
                    }
                    start += 1;
                }
                if has_string {
                    let inner: Vec<u32> = stream[start..(end as usize).min(stream.len())].to_vec();
                    let remove_end = ((end + 1) as usize).min(stream.len());
                    stream.splice(start - 1..remove_end, inner);
                }
            }
            at_rule_end = last_or(stream, at_rule);
            let at_rule_tokens = stream.clone();
            trim_array(arena, stream);
            let val = render_skip_comments(arena, stream, options);
            let state = if valid { EnumAstNodeStatus::Validated } else { EnumAstNodeStatus::Invalid };
            Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, &at_rule_tokens, state, if valid { Vec::new() } else { result.errors }, at_rule_end, &val, parse_as_block))
        }
        "import" => {
            let (ok, result_errors) = match_at_rule_import_syntax(arena, at_rule, stream, options);
            if !result_errors.is_empty() {
                errors.extend(result_errors.iter().cloned());
            } else if stream.first().is_some_and(|&f| {
                t(f) == EnumToken::UrlFunctionTokenType
                    && arena.token_iter(f).any(|x| matches!(t(x), EnumToken::StringTokenType | EnumToken::UrlTokenTokenType))
            }) {
                let inner = arena.tokens(stream[0]);
                stream.splice(0..1, inner);
            }
            at_rule_end = last_or(stream, at_rule);
            let state = if ok { EnumAstNodeStatus::Validated } else { EnumAstNodeStatus::Invalid };
            let val = render_skip_comments(arena, stream, options);
            Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, stream, state, if ok { Vec::new() } else { result_errors }, at_rule_end, &val, false))
        }
        "supports" | "when" | "else" => {
            trim_white_space_tokens(arena, stream);
            let (result_success, result_errors) = if at_rule_name == "supports" {
                parse_at_rule_support_syntax(arena, stream, at_rule, options)
            } else {
                match_at_rule_when_else_syntax(arena, stream, at_rule, options)
            };
            errors.extend(result_errors.iter().cloned());
            let mut ok = result_success;
            if nam == "else" {
                let mut sibling = arena.last_child.get(context);
                while sibling != 0 && matches!(t(sibling), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType | EnumToken::CDOCOMMTokenType) {
                    sibling = arena.previous_sibling.get(sibling);
                }
                let mut missing_when = false;
                let mut defined_after_last_else = false;
                if sibling == 0 || t(sibling) != EnumToken::AtRuleNodeType {
                    missing_when = true;
                } else if !arena.string_equals(arena.name.get(sibling), "when") {
                    if !arena.string_equals(arena.name.get(sibling), "else") {
                        missing_when = true;
                    } else if arena.string_value.get(sibling) == 0 {
                        defined_after_last_else = true;
                    }
                }
                if missing_when {
                    ok = false;
                    errors.push(drop_error(arena, "at-rule @when is required before @else block".into(), at_rule, options));
                } else if defined_after_last_else {
                    ok = false;
                    errors.push(drop_error(arena, "at-rule @else block is defined after last @else block".into(), at_rule, options));
                }
            }
            let options = ParserOptions { minify: Some(false), convert_color: Some(ConvertColor::Bool(false)), ..options.clone() };
            at_rule_end = last_or(stream, at_rule);
            let state = if ok { EnumAstNodeStatus::Validated } else { EnumAstNodeStatus::Invalid };
            let node_errors = if result_success {
                Vec::new()
            } else {
                let mut e = last_error(errors);
                e.extend(result_errors);
                e
            };
            let val = render_tokens(arena, stream, Some(&options));
            Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, stream, state, node_errors, at_rule_end, &val, true))
        }
        "media" => {
            let options = ParserOptions { parse_color: Some(false), ..options.clone() };
            let (ok, result_errors) = parse_mediaquery_list(arena, stream, &options);
            errors.extend(result_errors.iter().cloned());
            at_rule_end = last_or(stream, at_rule);
            let state = if ok { EnumAstNodeStatus::Validated } else { EnumAstNodeStatus::Invalid };
            let val = render_tokens(arena, stream, Some(&options));
            Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, stream, state, if ok { Vec::new() } else { result_errors }, at_rule_end, &val, true))
        }
        "scope" => {
            trim_array(arena, stream);
            let context_v = create_validation_context(arena, stream);
            // typescript passes a function as the closing token type: it never matches
            let mut range = context_v.borrow().peek_range(arena, 0, 0);
            let mut ok = true;
            if range.first().is_none_or(|&r| t(r) != EnumToken::StartParensTokenType) {
                errors.push(drop_error(arena, "expected '(' at start of @scope block".into(), range.first().copied().unwrap_or(at_rule), options));
                ok = false;
            } else if range.last().is_none_or(|&r| t(r) != EnumToken::EndParensTokenType) {
                errors.push(drop_error(arena, "expected ')' at end of @scope block".into(), range.last().copied().unwrap_or(at_rule), options));
                ok = false;
            } else {
                let mut srange: Vec<u32> = range[1..range.len() - 1].to_vec();
                let (selector_ok, _) = match_selector_syntax(arena, &mut srange, errors, options, true);
                if !selector_ok {
                    ok = false;
                } else {
                    let start = stream.iter().position(|&x| x == range[0]).map_or(0, |p| p + 1);
                    trim_white_space_tokens(arena, &mut srange);
                    let end = (start + range.len() - 2).min(stream.len());
                    stream.splice(start..end, srange);
                }
            }
            let mut index = stream.iter().position(|&x| Some(x) == range.last().copied()).map_or(-1, |p| p as isize);
            if stream.len() as isize > index + 1 {
                let skip_ws = |index: &mut isize, stream: &Vec<u32>| loop {
                    *index += 1;
                    if !stream.get(*index as usize).is_some_and(|&x| matches!(t(x), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType)) {
                        break;
                    }
                };
                skip_ws(&mut index, stream);
                if (index as usize) < stream.len() {
                    let tk = stream[index as usize];
                    if t(tk) != EnumToken::IdenTokenType || !arena.with_string(arena.string_value.get(tk), |v| v.eq_ignore_ascii_case("to")) {
                        errors.push(drop_error(arena, "expected 'to' at end of @scope block".into(), tk, options));
                        ok = false;
                    } else {
                        skip_ws(&mut index, stream);
                        let tk = stream.get(index as usize).copied();
                        if tk.is_none_or(|x| t(x) != EnumToken::StartParensTokenType) {
                            errors.push(drop_error(arena, "expected 'to' at end of @scope block".into(), tk.unwrap_or(0), options));
                            ok = false;
                        } else {
                            let context_v = create_validation_context(arena, &stream[index as usize..]);
                            range = context_v.borrow().peek_range(arena, 0, 0);
                            if range.last().is_none_or(|&r| t(r) != EnumToken::EndParensTokenType) {
                                errors.push(drop_error(arena, "expected ')' at end of @scope block".into(), range.last().copied().unwrap_or(at_rule), options));
                                ok = false;
                            } else {
                                let mut srange: Vec<u32> = range[1..range.len() - 1].to_vec();
                                let (selector_ok, _) = match_selector_syntax(arena, &mut srange, errors, options, true);
                                if !selector_ok {
                                    ok = false;
                                } else {
                                    let start = stream.iter().position(|&x| x == range[0]).map_or(0, |p| p + 1);
                                    trim_white_space_tokens(arena, &mut srange);
                                    let end = (start + range.len() - 2).min(stream.len());
                                    stream.splice(start..end, srange);
                                }
                            }
                        }
                    }
                }
            }
            at_rule_end = last_or(stream, at_rule);
            let state = if ok { EnumAstNodeStatus::Validated } else { EnumAstNodeStatus::Invalid };
            let node_errors = if ok { Vec::new() } else { last_error(errors) };
            let val = render_tokens(arena, stream, Some(options));
            Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, stream, state, node_errors, at_rule_end, &val, true))
        }
        "page" => {
            trim_array(arena, stream);
            at_rule_end = last_or(stream, at_rule);
            let state = if success { EnumAstNodeStatus::Validated } else { EnumAstNodeStatus::Invalid };
            let val = render_tokens(arena, stream, Some(options));
            Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, stream, state, Vec::new(), at_rule_end, &val, true))
        }
        "top-left-corner" | "top-left" | "top-center" | "top-right" | "top-right-corner" | "bottom-left-corner" | "bottom-left"
        | "bottom-right" | "bottom-right-corner" | "left-top" | "left-middle" | "left-bottom" | "right-top" | "right-middle"
        | "right-bottom" => {
            if context_name.as_deref() != Some("page") {
                success = false;
                errors.push(drop_error(arena, "node is allowed only in @page rule".into(), at_rule, options));
            } else {
                trim_array(arena, stream);
                if let Some(&tk) = stream.iter().find(|&&x| !matches!(t(x), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType)) {
                    success = false;
                    errors.push(drop_error(arena, "expected whitespace or comment".into(), tk, options));
                }
            }
            at_rule_end = last_or(stream, at_rule);
            let state = if success { EnumAstNodeStatus::Validated } else { EnumAstNodeStatus::Invalid };
            let node_errors = if success { Vec::new() } else { last_error(errors) };
            let val = render_tokens(arena, stream, Some(options));
            Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, stream, state, node_errors, at_rule_end, &val, true))
        }
        "value" => {
            let mut index = 0usize;
            let mut is_var_declaration = false;
            while index < stream.len() {
                let tk = stream[index];
                if t(tk) == EnumToken::PseudoClassTokenType {
                    let colon = arena.create_token(EnumToken::ColonTokenType, TokenValue::None, None);
                    arena.copy_location(colon, tk);
                    let value = arena.resolve_string(arena.string_value.get(tk));
                    let rest: String = value.chars().skip(1).collect();
                    arena.string_value.set(tk, arena.intern_string(&rest));
                    arena.data_types.set(tk, EnumToken::IdenTokenType);
                    stream.insert(index, colon);
                    is_var_declaration = true;
                    break;
                } else if t(tk) == EnumToken::ColonTokenType {
                    is_var_declaration = true;
                    break;
                } else if t(tk) == EnumToken::IdenTokenType && arena.string_equals_ignore_case(arena.string_value.get(tk), "from") {
                    break;
                }
                index += 1;
            }
            let result = match_all_syntaxes(arena, syntax.as_deref(), &create_validation_context(arena, stream), options);
            at_rule_end = last_or(stream, at_rule);
            if !result.success {
                errors.extend(result.errors.iter().cloned());
                let val = render_tokens(arena, stream, Some(options));
                return Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, stream, EnumAstNodeStatus::Invalid, result.errors, at_rule_end, &val, false));
            }
            if is_var_declaration {
                let nam_token = stream.iter().copied().find(|&x| t(x) == EnumToken::IdenTokenType).unwrap_or(0);
                let mut value: Vec<u32> = stream.iter().skip(index + 1).copied().filter(|&x| t(x) != EnumToken::CommentTokenType).collect();
                trim_array(arena, &mut value);
                let typ = if value.len() == 1 && t(value[0]) == EnumToken::StringTokenType {
                    EnumToken::CssVariableImportTokenType
                } else {
                    EnumToken::CssVariableTokenType
                };
                let node = arena.create_declaration(typ, StrOrId::Id(arena.string_value.get(nam_token)), Some(&value), EnumAstNodeStatus::Validated, Some(Vec::new()));
                arena.span_location(node, at_rule, at_rule_end);
                return Some(node);
            }
            let mut names: Vec<u32> = stream[..index.min(stream.len())].to_vec();
            trim_array(arena, &mut names);
            let node = arena.create_declaration(EnumToken::CssVariableDeclarationMapTokenType, StrOrId::Str(&nam), Some(&names), EnumAstNodeStatus::Validated, Some(Vec::new()));
            let alias: Vec<u32> = stream.iter().skip(index + 1).copied().collect();
            arena.set_token_list(arena.field_list(node, TokenField::L), &alias);
            arena.span_location(node, at_rule, at_rule_end);
            Some(node)
        }
        _ => {
            let options = ParserOptions { parse_color: Some(false), ..options.clone() };
            let (result_success, mut result_errors) = if syntax.is_none() {
                let r = match_generic_syntax(arena, stream, &options);
                errors.extend(r.1.iter().cloned());
                r
            } else {
                let r = match_at_rule_syntax(arena, at_rule, stream, &options);
                errors.extend(r.1.iter().cloned());
                if r.0 {
                    let mut stack: Vec<u32> = Vec::new();
                    let mut i = 0usize;
                    while i < stream.len() {
                        let tk = stream[i];
                        if t(tk) == EnumToken::StartParensTokenType || is_function_def_type(t(tk)) {
                            stack.push(tk);
                            i += 1;
                            continue;
                        }
                        if t(tk) == EnumToken::EndParensTokenType && !stack.is_empty() {
                            let index = stream.iter().position(|&x| x == *stack.last().unwrap()).unwrap();
                            let func = stream[index];
                            arena.set_span(func, arena.get_source_id(func), arena.get_span_start(func), arena.get_span_end(tk));
                            arena.data_types.set(func, tokens_func_def_map(t(func)).unwrap_or(0));
                            let mut inner: Vec<u32> = stream.drain(index + 1..i).collect();
                            trim_white_space_tokens(arena, &mut inner);
                            arena.set_token_list(func, &inner);
                            i = index;
                            if index + 1 < stream.len() {
                                stream.remove(index + 1);
                            }
                            stack.pop();
                        }
                        i += 1;
                    }
                }
                r
            };
            at_rule_end = last_or(stream, at_rule);
            let at_rule_tokens = stream.clone();
            let mut state = if result_success { EnumAstNodeStatus::Validated } else { EnumAstNodeStatus::Invalid };
            let location = || token_location(arena, at_rule, &options);
            match nam.as_str() {
                "mixin" => {
                    if !is_mixin_scope(arena, context) {
                        state |= EnumAstNodeStatus::Disallowed;
                        let mut e = ErrorDescription::drop("mixin rule cannot be a nested group rule or rule style body");
                        e.location = location();
                        result_errors.push(e);
                    }
                }
                "contents" | "result" => {
                    let mut parent = context;
                    let mut valid = false;
                    while parent != 0 {
                        if t(parent) == EnumToken::AtRuleNodeType && arena.string_equals(arena.name.get(parent), "mixin") {
                            valid = true;
                            break;
                        }
                        parent = arena.parents.get(parent);
                    }
                    if !valid {
                        state |= EnumAstNodeStatus::Disallowed;
                        let mut e = ErrorDescription::drop(format!("{} rule cannot be used outside mixin rule", nam));
                        e.location = location();
                        result_errors.push(e);
                    }
                }
                "layer" => {
                    if context_type != EnumToken::StyleSheetNodeType {
                        let mut parent = context;
                        while parent != 0 {
                            if t(parent) == EnumToken::AtRuleNodeType && arena.string_equals(arena.name.get(parent), "mixin") {
                                state |= EnumAstNodeStatus::Disallowed;
                                let mut e = ErrorDescription::drop("at-rule layer is not allowed inside mixin rule");
                                e.location = location();
                                result_errors.push(e);
                                break;
                            }
                            parent = arena.parents.get(parent);
                        }
                    }
                }
                _ => {}
            }
            trim_white_space_tokens(arena, stream);
            let val = render_tokens(arena, stream, Some(&options));
            Some(create_at_rule_node(arena, at_rule, EnumToken::AtRuleNodeType, &at_rule_tokens, state, result_errors, at_rule_end, &val, parse_as_block))
        }
    }
}

/// parse a string as declaration nodes (detached). a new arena is created when none is given
pub fn parse_declarations(declaration: &str, arena: Option<Rc<AstArena>>) -> (Rc<AstArena>, Vec<u32>) {
    let stream = format!(".x{{{}}}", declaration);
    let source = SourceFile::new(Rc::from(""), Vec::new(), Some(Rc::from("")));
    let sources = crate::options::SourcesMap::default();
    sources.borrow_mut().insert(source.id, source.clone());
    let options = ParserOptions {
        set_parent: Some(false),
        minify: Some(false),
        validation: Some(false),
        source: Some(source.clone()),
        sources_map: Some(sources),
        ..Default::default()
    };
    let tokenizer = Tokenizer::new(create_parse_info(Some(&CssInput::from(stream)), source));
    let result = do_parse_sync(tokenizer, options, arena, Rc::new(crate::host::NoHost)).expect("parse declarations");
    let arena = result.arena.clone();
    let rule = arena.first_child.get(result.ast);
    let mut nodes = Vec::new();
    let mut node = if rule == 0 { 0 } else { arena.first_child.get(rule) };
    while node != 0 {
        let next = arena.next_sibling.get(node);
        if matches!(arena.data_types.get(node), EnumToken::DeclarationNodeType | EnumToken::CommentNodeType) {
            arena.detach(node);
            nodes.push(node);
        }
        node = next;
    }
    (arena, nodes)
}

/// parse a string as the content of a style rule (detached nodes, the comments are dropped)
pub fn parse_rule_content(content: &str, arena: Rc<AstArena>, parse_color: Option<bool>) -> Vec<u32> {
    let source = SourceFile::new(Rc::from(""), Vec::new(), Some(Rc::from("")));
    let sources = crate::options::SourcesMap::default();
    sources.borrow_mut().insert(source.id, source.clone());
    let options = ParserOptions {
        set_parent: Some(false),
        minify: Some(false),
        validation: Some(false),
        lenient: Some(true),
        nesting_rules: Some(true),
        expand_nesting_rules: Some(false),
        evaluate_mixins: Some(false),
        parse_color: Some(parse_color.unwrap_or(true)),
        source: Some(source.clone()),
        sources_map: Some(sources),
        ..Default::default()
    };
    let tokenizer = Tokenizer::new(create_parse_info(Some(&CssInput::from(format!(".x{{{}}}", content))), source));
    let result = do_parse_sync(tokenizer, options, Some(arena.clone()), Rc::new(crate::host::NoHost)).expect("parse rule content");
    let rule = arena.first_child.get(result.ast);
    let mut nodes = Vec::new();
    let mut node = if rule == 0 { 0 } else { arena.first_child.get(rule) };
    while node != 0 {
        let next = arena.next_sibling.get(node);
        if arena.data_types.get(node) != EnumToken::CommentNodeType {
            arena.detach(node);
            nodes.push(node);
        }
        node = next;
    }
    nodes
}

/// parse a css string into tokens (`parseString(arena, src)` with `{ parseColor: true }`)
pub fn parse_string(arena: &AstArena, src: &str) -> Vec<u32> {
    let options = ParserOptions { parse_color: Some(true), ..Default::default() };
    parse_string_with(arena, src, Some(&options), None)
}

/// parse a css string into tokens. `options`: `src` and `parseColor`; `None` is `null` (the colors are parsed)
pub fn parse_string_with(arena: &AstArena, src: &str, options: Option<&ParserOptions>, errors: Option<&mut Vec<ErrorDescription>>) -> Vec<u32> {
    let file = options.and_then(|o| o.src.clone()).unwrap_or_default();
    let source = SourceFile::new(Rc::from(""), Vec::new(), Some(Rc::from(file.as_str())));
    let mut tokenizer = Tokenizer::new(create_parse_info(Some(&CssInput::from(src)), source));
    let mut mapped: Vec<u32> = Vec::new();
    while tokenizer.typ != Some(EnumToken::EOF) {
        tokenizer.next();
        mapped.push(allocate_token(arena, &mut tokenizer, -1));
    }
    parse_tokens(arena, &mut mapped, options, errors);
    let len = mapped.len();
    let start = len.saturating_sub(if len >= 2 && arena.data_types.get(mapped[len - 2]) == EnumToken::WhitespaceTokenType { 2 } else { 1 });
    let removed: Vec<u32> = mapped.drain(start..(start + 2).min(len)).collect();
    for r in removed {
        if arena.parents.get(r) == 0 && arena.first_token.get(r) == 0 && arena.first_child.get(r) == 0 {
            arena.recycle(r);
        }
    }
    mapped
}

fn parse_error(arena: &AstArena, message: String, node: Option<u32>, options: Option<&ParserOptions>) -> ErrorDescription {
    let mut error = ErrorDescription::drop(message);
    error.node = node;
    error.location = options.and_then(|o| node.and_then(|n| token_location(arena, n, o)));
    error
}

/// parse the function, parentheses and attribute tokens of a token list (in place). `None` options: `{ parseColor:
/// true }`
pub fn parse_tokens(arena: &AstArena, tokens: &mut Vec<u32>, options: Option<&ParserOptions>, mut errors: Option<&mut Vec<ErrorDescription>>) {
    let t = |x: u32| arena.data_types.get(x);
    let parse_colors = options.map_or(true, |o| o.parse_color == Some(true));
    let mut stack: Vec<u32> = Vec::new();
    let mut i: isize = 0;
    while (i as usize) < tokens.len() {
        let iu = i as usize;
        let mut tk = tokens[iu];
        let mut typ = t(tk);
        if typ == EnumToken::FunctionTokenDefType && iu > 0 && matches!(t(tokens[iu - 1]), EnumToken::ColonTokenType | EnumToken::DoubleColonTokenType) {
            let previous = tokens[iu - 1];
            let prefix = if t(previous) == EnumToken::ColonTokenType { ":" } else { "::" };
            let name = format!("{}{}", prefix, arena.resolve_string(arena.string_value.get(tk)));
            arena.string_value.set(previous, arena.intern_string(&name));
            arena.data_types.set(previous, EnumToken::PseudoClassFunctionTokenDefType);
            arena.set_span(previous, arena.get_source_id(previous), arena.get_span_start(previous), arena.get_span_end(tk));
            tokens.remove(iu);
            i -= 1;
            tk = previous;
            typ = EnumToken::PseudoClassFunctionTokenDefType;
        }
        if typ == EnumToken::AttrStartTokenType || typ == EnumToken::StartParensTokenType || is_function_def_type(typ) {
            stack.push(tk);
            i += 1;
            continue;
        }
        let iu = i as usize;
        if typ == EnumToken::EndParensTokenType {
            if stack.last().is_none_or(|&s| t(s) != EnumToken::StartParensTokenType && !is_function_def_type(t(s))) {
                if let Some(errors) = errors.as_deref_mut() {
                    errors.push(parse_error(arena, "Unbalanced token ')'".into(), stack.last().copied(), options));
                }
                i += 1;
                continue;
            }
            tokens.remove(iu);
            let index = tokens.iter().position(|&x| x == *stack.last().unwrap()).unwrap();
            let func = tokens[index];
            arena.data_types.set(
                func,
                if t(func) == EnumToken::StartParensTokenType { EnumToken::ParensTokenType } else { tokens_func_def_map(t(func)).unwrap_or(0) },
            );
            let mut inner: Vec<u32> = tokens.drain(index + 1..iu).collect();
            trim_array(arena, &mut inner);
            arena.set_token_list(func, &inner);
            arena.set_span(func, arena.get_source_id(func), arena.get_span_start(func), arena.get_span_end(tk));
            i = index as isize;
            if t(func) == EnumToken::ColorTokenType && parse_colors {
                parse_color(arena, func);
            }
            stack.pop();
            i += 1;
            continue;
        }
        if typ == EnumToken::AttrEndTokenType {
            if stack.last().is_none_or(|&s| t(s) != EnumToken::AttrStartTokenType) {
                if let Some(errors) = errors.as_deref_mut() {
                    errors.push(parse_error(arena, "Unbalanced token ']'".into(), stack.last().copied(), options));
                }
                i += 1;
                continue;
            }
            let attr = *stack.last().unwrap();
            let index = tokens.iter().position(|&x| x == attr).unwrap();
            arena.set_span(attr, arena.get_source_id(attr), arena.get_span_start(attr), arena.get_span_end(tk));
            tokens.remove(iu);
            arena.data_types.set(attr, EnumToken::AttrTokenType);
            let inner: Vec<u32> = tokens.drain(index + 1..iu).collect();
            arena.set_token_list(attr, &inner);
            i = index as isize;
            stack.pop();
            i += 1;
            continue;
        }
        if typ == EnumToken::IdenTokenType {
            if arena.string_equals(arena.string_value.get(tk), "from") && iu > 0 {
                let mut left: Vec<u32> = Vec::new();
                let mut right: Vec<u32> = Vec::new();
                let mut found_left = 0;
                let mut found_right = 0;
                let mut k = iu;
                let mut l = iu;
                while k > 0 {
                    let p = t(tokens[k - 1]);
                    if matches!(p, EnumToken::CommentTokenType | EnumToken::WhitespaceTokenType) {
                        k -= 1;
                        left.push(tokens[k]);
                        continue;
                    }
                    if matches!(p, EnumToken::IdenTokenType | EnumToken::DashedIdenTokenType) {
                        found_left += 1;
                        k -= 1;
                        left.push(tokens[k]);
                        continue;
                    }
                    break;
                }
                loop {
                    l += 1;
                    if l >= tokens.len() {
                        break;
                    }
                    let n = t(tokens[l]);
                    if matches!(n, EnumToken::CommentTokenType | EnumToken::WhitespaceTokenType) {
                        right.push(tokens[l]);
                        continue;
                    }
                    if matches!(n, EnumToken::IdenTokenType | EnumToken::StringTokenType) {
                        found_right += 1;
                        right.push(tokens[l]);
                        continue;
                    }
                    break;
                }
                if found_left > 0 && found_right == 1 {
                    while left.first().is_some_and(|&x| t(x) == EnumToken::WhitespaceTokenType) {
                        left.remove(0);
                    }
                    while left.last().is_some_and(|&x| t(x) == EnumToken::WhitespaceTokenType) {
                        left.pop();
                    }
                    let composes = arena.create_token(EnumToken::ComposesSelectorNodeType, TokenValue::None, None);
                    let mut from = 0u32;
                    for &b in &right {
                        if from == 0 || matches!(t(b), EnumToken::IdenTokenType | EnumToken::StringTokenType) {
                            from = b;
                        }
                    }
                    arena.set_token_list(arena.field_list(composes, TokenField::L), &left);
                    if from != 0 {
                        arena.set_token_list(arena.field_list(composes, TokenField::R), &[from]);
                    }
                    let end = (l + 1).min(tokens.len());
                    tokens.splice(k..end, [composes]);
                    i = k as isize;
                    i += 1;
                    continue;
                }
            } else if parse_colors && is_color(arena, tk, None) {
                parse_color(arena, tk);
            }
        }
        let iu = i as usize;
        if typ == EnumToken::WhitespaceTokenType
            && (iu == 0
                || iu + 1 == tokens.len()
                || tokens.get(iu + 1).is_some_and(|&n| {
                    matches!(t(n), EnumToken::CommaTokenType | EnumToken::GteTokenType | EnumToken::LteTokenType | EnumToken::ColumnCombinatorTokenType)
                })
                || (iu > 0 && is_trim_white_space(t(tokens[iu - 1]))))
        {
            tokens.remove(iu);
            continue;
        }
        if typ == EnumToken::ColonTokenType {
            if let Some(&next_token) = tokens.get(iu + 1) {
                let next = t(next_token);
                if next == EnumToken::FunctionTokenType {
                    arena.data_types.set(next_token, EnumToken::PseudoClassFuncTokenType);
                } else if next == EnumToken::IdenTokenType {
                    let name = format!(":{}", arena.resolve_string(arena.string_value.get(next_token)));
                    arena.string_value.set(next_token, arena.intern_string(&name));
                    arena.data_types.set(next_token, EnumToken::PseudoClassTokenType);
                }
                if next == EnumToken::FunctionTokenType || next == EnumToken::IdenTokenType {
                    tokens.remove(iu);
                    i -= 1;
                }
            }
        }
        if typ == EnumToken::CommaTokenType {
            let iu = i as usize;
            if iu + 1 < tokens.len() && t(tokens[iu + 1]) == EnumToken::WhitespaceTokenType {
                tokens.remove(iu + 1);
            }
            if iu > 0 && t(tokens[iu - 1]) == EnumToken::WhitespaceTokenType {
                tokens.remove(iu - 1);
                i -= 1;
            }
        }
        i += 1;
    }
    if let Some(&node) = stack.last() {
        if let Some(errors) = errors {
            let expecting = if t(node) == EnumToken::AttrStartTokenType { "']'" } else { ")" };
            errors.push(parse_error(arena, format!("Unbalanced token. Expecting {}'", expecting), Some(node), options));
        }
    }
}

#[allow(dead_code)]
fn _unused(a: &str) -> bool {
    equals_ignore_case(a, a)
}
