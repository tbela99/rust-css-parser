//! Port of `src/ast/features/prefix.ts`: vendor prefixes removal.

use super::r#type::FeatureWalkMode;
use super::{FeatureResult, MinifyFeature, SharedFeature};
use crate::ast::arena::{AstArena, TokenField, TokenValue};
use crate::ast::minify::{split_rule, MinifyContext};
use crate::ast::types::{EnumAstNodeStatus, EnumToken};
use crate::ast::walk::{ValueWalker, WalkerValueOptions};
use crate::options::ParserOptions;
use crate::parser::utils::angle::to_degrees;
use crate::printer::render::render_value_default;
use crate::syntax::constants::{reg_match_linear_gradient, reg_match_radial_gradient, FUNC_LIKE};
use crate::syntax::syntax::pseudo_alias_map;
use crate::validation::config::get_syntax_config;
use crate::validation::utils::list::split_token_list;
use std::cell::RefCell;
use crate::utils::fx::HashSet;
use std::rc::Rc;

/// `/^-([^-]+)-(.+)$/`: the name without its vendor prefix
fn unprefixed(name: &str) -> Option<&str> {
    let rest = name.strip_prefix('-')?;
    let dash = rest.find('-')?;
    let tail = &rest[dash + 1..];
    if dash == 0 || tail.is_empty() || tail.contains('\n') || tail.contains('\r') || tail.contains('\u{2028}') || tail.contains('\u{2029}') {
        return None;
    }
    Some(tail)
}

fn config_has(group: &str, key: &str) -> bool {
    get_syntax_config()[group].as_object().is_some_and(|o| o.contains_key(key))
}

fn replace_pseudo(tokens: Vec<Vec<String>>) -> Vec<Vec<String>> {
    tokens
        .into_iter()
        .map(|raw| {
            raw.into_iter()
                .map(|r| {
                    if let Some(index) = r.find('(') {
                        let name = format!("{}()", &r[..index]);
                        return match pseudo_alias_map(&name) {
                            Some(alias) => format!("{}{}", alias, &r[index..]),
                            None => r,
                        };
                    }
                    match pseudo_alias_map(&r) {
                        Some(alias) if config_has("selectors", alias) => alias.to_string(),
                        _ => r,
                    }
                })
                .collect()
        })
        .collect()
}

fn replace_ast_nodes(arena: &AstArena, tokens: &mut Vec<u32>) -> bool {
    let mut result = false;
    let mut walker = ValueWalker::of_tokens(tokens, WalkerValueOptions::default(), false);
    while let Some(value) = walker.next(arena) {
        let typ = arena.data_types.get(value);
        if typ == EnumToken::MediaQueryConditionTokenType {
            let token = arena.field_tokens(value, TokenField::L).into_iter().find(|&t| arena.data_types.get(t) == EnumToken::IdenTokenType);
            let Some(token) = token else {
                continue;
            };
            let val = arena.resolve_string(arena.string_value.get(token));
            let Some(alias) = pseudo_alias_map(&val) else {
                continue;
            };
            arena.string_value.set(token, arena.intern_string(alias));
            let r = arena.field_tokens(value, TokenField::R);
            if (alias.eq_ignore_ascii_case("min-resolution") || alias.eq_ignore_ascii_case("max-resolution"))
                && r.first().is_some_and(|&r0| arena.data_types.get(r0) == EnumToken::NumberTokenType)
            {
                arena.data_types.set(r[0], EnumToken::ResolutionTokenType);
                arena.set_unit_string(r[0], Some("x"));
                result = true;
            }
            let is_min = alias.starts_with("min-");
            let is_max = alias.starts_with("max-");
            if is_min || is_max {
                arena.string_value.set(token, arena.intern_string(&alias[4..]));
                arena.data_types.set(arena.field(value, TokenField::Op), if is_max { EnumToken::LteTokenType } else { EnumToken::GteTokenType });
            }
        } else if matches!(typ, EnumToken::IdenTokenType | EnumToken::PseudoClassFuncTokenType | EnumToken::PseudoClassTokenType | EnumToken::PseudoElementTokenType) {
            let mut key = arena.resolve_string(arena.string_value.get(value)).to_string();
            if typ == EnumToken::PseudoClassFuncTokenType {
                key.push_str("()");
            }
            if let Some(alias) = pseudo_alias_map(&key) {
                arena.string_value.set(value, arena.intern_string(alias));
                if alias.starts_with("::") && typ == EnumToken::PseudoElementTokenType {
                    arena.data_types.set(value, EnumToken::PseudoClassTokenType);
                }
                result = true;
            }
        }
    }
    // remove duplicate tokens
    if tokens.iter().any(|&t| arena.data_types.get(t) == EnumToken::CommaTokenType) {
        let mut set: HashSet<String> = HashSet::default();
        let split = split_token_list(arena, tokens, &[EnumToken::CommaTokenType], false);
        tokens.clear();
        for curr in split {
            let str: String = curr.iter().map(|&t| render_value_default(arena, t)).collect();
            if !set.insert(str) {
                continue;
            }
            if !tokens.is_empty() {
                tokens.push(arena.create_token(EnumToken::CommaTokenType, TokenValue::None, None));
            }
            tokens.extend(curr);
        }
    }
    result
}

pub struct ComputePrefixFeature;

impl ComputePrefixFeature {
    pub fn register(options: &ParserOptions, features: &mut Vec<SharedFeature>) {
        if options.remove_prefix == Some(true) {
            features.push(Rc::new(RefCell::new(ComputePrefixFeature)));
        }
    }
}

/// `arena.dataTypes[tokens[i]]`: 0 when out of range (`undefined`)
fn typ_at(arena: &AstArena, tokens: &[u32], i: usize) -> u8 {
    tokens.get(i).map_or(0, |&t| arena.data_types.get(t))
}

fn str_is(arena: &AstArena, tokens: &[u32], i: usize, value: &str) -> bool {
    tokens.get(i).is_some_and(|&t| arena.string_equals_ignore_case(arena.string_value.get(t), value))
}

fn iden(arena: &AstArena, value: &str) -> u32 {
    arena.create_token(EnumToken::IdenTokenType, TokenValue::Str(value), None)
}

fn ws(arena: &AstArena) -> u32 {
    arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None)
}

impl MinifyFeature for ComputePrefixFeature {
    fn ordering(&self) -> i32 {
        0
    }

    fn process_mode(&self) -> u32 {
        FeatureWalkMode::Pre
    }

    fn run(&mut self, arena: &AstArena, node: u32, _options: &ParserOptions, _parent: u32, _context: &mut MinifyContext, _mode: u32) -> FeatureResult {
        if arena.flags.get(node) & (EnumAstNodeStatus::Disallowed | EnumAstNodeStatus::Invalid | EnumAstNodeStatus::Malformed) != 0 {
            return FeatureResult::None;
        }
        let typ = arena.data_types.get(node);
        if typ == EnumToken::RuleNodeType {
            let selector = replace_pseudo(split_rule(&arena.resolve_string(arena.name.get(node))))
                .iter()
                .map(|curr| curr.concat())
                .collect::<Vec<_>>()
                .join(",");
            arena.name.set(node, arena.intern_string(&selector));
            if arena.first_token.get(node) != 0 {
                let mut tokens = arena.tokens(node);
                replace_ast_nodes(arena, &mut tokens);
                arena.set_token_list(node, &tokens);
            }
        } else if typ == EnumToken::DeclarationNodeType {
            let name = arena.resolve_string(arena.name.get(node));
            if name.starts_with('-') {
                if let Some(nam) = unprefixed(&name) {
                    let mut nam = nam;
                    if !config_has("declarations", nam) {
                        if let Some(alias) = pseudo_alias_map(&name) {
                            nam = alias;
                        }
                    }
                    if config_has("declarations", nam) {
                        arena.name.set(node, arena.intern_string(nam));
                    }
                }
            }
            let mut has_prefix = false;
            let mut walker = ValueWalker::of_node(arena, node, WalkerValueOptions::default(), false);
            while let Some(value) = walker.next(arena) {
                let typ = arena.data_types.get(value);
                let val = arena.resolve_string(arena.string_value.get(value));
                if (typ == EnumToken::IdenTokenType || (typ != EnumToken::ParensTokenType && FUNC_LIKE.contains(&typ))) && unprefixed(&val).is_some() {
                    if val.ends_with("-gradient") {
                        if reg_match_linear_gradient(&val) {
                            webkit_linear_to_linear_gradient(arena, value);
                        } else if reg_match_radial_gradient(&val) {
                            webkit_radial_to_radial_gradient(arena, value);
                        } else if val.eq_ignore_ascii_case("-webkit-gradient") {
                            webkit_gradient_to_gradient(arena, value);
                        }
                        // not supported yet
                        break;
                    }
                    has_prefix = true;
                    break;
                }
            }
            if has_prefix {
                let mut walker = ValueWalker::of_node(arena, node, WalkerValueOptions::default(), false);
                while let Some(value) = walker.next(arena) {
                    let typ = arena.data_types.get(value);
                    if typ == EnumToken::IdenTokenType || FUNC_LIKE.contains(&typ) {
                        let val = arena.resolve_string(arena.string_value.get(value));
                        if let Some(stripped) = unprefixed(&val) {
                            arena.string_value.set(value, arena.intern_string(stripped));
                        }
                    }
                }
            }
        } else if typ == EnumToken::AtRuleNodeType || typ == EnumToken::KeyframesAtRuleNodeType {
            let name = arena.resolve_string(arena.name.get(node));
            if name.starts_with('-') {
                if let Some(stripped) = unprefixed(&name) {
                    if config_has("atRules", &format!("@{}", stripped)) {
                        arena.name.set(node, arena.intern_string(stripped));
                    }
                }
            }
            if typ == EnumToken::AtRuleNodeType && arena.string_value.get(node) != 0 {
                let mut tokens = arena.tokens(node);
                let result = replace_ast_nodes(arena, &mut tokens);
                arena.set_token_list(node, &tokens);
                if result {
                    let value: String = tokens.iter().map(|&t| render_value_default(arena, t)).collect();
                    arena.string_value.set(node, arena.intern_string(&value));
                }
            }
        }
        FeatureResult::Node(node)
    }
}

/// convert -webkit-linear-gradient to the linear-gradient syntax
fn webkit_linear_to_linear_gradient(arena: &AstArena, token: u32) {
    let mut tokens = arena.tokens(token);
    let mut i = 0usize;
    while i < tokens.len() {
        let t = tokens[i];
        let typ = arena.data_types.get(t);
        if typ == EnumToken::AngleTokenType || (typ == EnumToken::NumberTokenType && arena.numeric_value.get(t) == 0.0) {
            let degrees = if typ == EnumToken::NumberTokenType { arena.numeric_value.get(t) } else { arena.numeric_value.get(to_degrees(arena, t)) };
            arena.numeric_value.set(t, (90.0 - degrees) % 360.0);
        } else if typ == EnumToken::IdenTokenType {
            let key = arena.resolve_string(arena.string_value.get(t)).to_lowercase();
            match key.as_str() {
                "right" | "left" => {
                    let replacement = [iden(arena, "to"), ws(arena), iden(arena, if key == "right" { "left" } else { "right" })];
                    tokens.splice(i..i + 1, replacement);
                    i += 2;
                }
                "top" | "bottom" => 'sw: {
                    let mut k = i + 1;
                    let to = if key == "top" { "bottom" } else { "top" };
                    while k < tokens.len() && matches!(arena.data_types.get(tokens[k]), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType) {
                        k += 1;
                    }
                    if typ_at(arena, &tokens, k) == EnumToken::IdenTokenType {
                        let val = if str_is(arena, &tokens, k, "left") {
                            "right"
                        } else if str_is(arena, &tokens, k, "right") {
                            "left"
                        } else {
                            ""
                        };
                        if !val.is_empty() {
                            let replacement = [iden(arena, "to"), ws(arena), iden(arena, to), ws(arena), iden(arena, val)];
                            tokens.splice(i..k + 1, replacement);
                            i += 4;
                            break 'sw;
                        }
                    }
                    let replacement = [iden(arena, "to"), ws(arena), iden(arena, to)];
                    tokens.splice(i..i + 1, replacement);
                    i += 2;
                }
                _ => {}
            }
        }
        i += 1;
    }
    let repeating = arena.string_equals_ignore_case(arena.string_value.get(token), "-webkit-repeating-linear-gradient");
    arena.string_value.set(token, arena.intern_string(if repeating { "repeating-linear-gradient" } else { "linear-gradient" }));
    arena.set_token_list(token, &tokens);
}

/// convert -webkit-gradient(linear) to the linear-gradient syntax
fn webkit_gradient_to_gradient(arena: &AstArena, token: u32) {
    let mut tokens = arena.tokens(token);
    let mut i = 0usize;
    let mut kind = "";
    // linear or radial
    if str_is(arena, &tokens, 0, "linear") {
        kind = "linear-gradient";
        i += 1;
    }
    let n = (i + 1).min(tokens.len());
    tokens.drain(0..n);
    let mut comma_count = 0;
    let mut key = String::new();
    i = 0;
    while i < tokens.len() {
        if arena.data_types.get(tokens[i]) == EnumToken::CommaTokenType {
            comma_count += 1;
            if comma_count > 1 {
                break;
            }
        }
        if arena.data_types.get(tokens[i]) == EnumToken::IdenTokenType {
            if !key.is_empty() {
                key.push(' ');
            }
            key.push_str(&arena.resolve_string(arena.string_value.get(tokens[i])));
        }
        i += 1;
    }
    let directions: &[&str] = match key.as_str() {
        "left top left bottom" => &["bottom"],
        "left bottom left top" => &["top"],
        "left top right top" => &["right"],
        "right top left top" => &["left"],
        "left top right bottom" => &["bottom", "right"],
        "right top left bottom" => &["bottom", "left"],
        "left bottom right top" => &["top", "right"],
        "right bottom left top" => &["top", "left"],
        _ => &[],
    };
    let mut replacements: Vec<u32> = Vec::new();
    if !directions.is_empty() {
        replacements.push(iden(arena, "to"));
        for d in directions {
            replacements.push(ws(arena));
            replacements.push(iden(arena, d));
        }
    }
    let end = i.min(tokens.len());
    tokens.splice(0..end, replacements.iter().copied());
    let mut check_stop = true;
    let mut check_stop_index = replacements.len() + 1;
    let mut color_stop: Vec<u32> = Vec::new();
    i = replacements.len() + 1;
    while i < tokens.len() {
        if arena.data_types.get(tokens[i]) == EnumToken::CommaTokenType && check_stop {
            check_stop_index = i;
            i += 1;
            continue;
        }
        if arena.data_types.get(tokens[i]) == EnumToken::FunctionTokenType {
            if str_is(arena, &tokens, i, "to") {
                if let Some(&t) = tokens.get(check_stop_index) {
                    color_stop.push(t);
                }
                color_stop.extend(arena.tokens(tokens[i]));
                let end = (i + 1).min(tokens.len());
                if check_stop_index < end {
                    tokens.drain(check_stop_index..end);
                }
                i = check_stop_index;
                check_stop = false;
            } else if str_is(arena, &tokens, i, "from") {
                let inner = arena.tokens(tokens[i]);
                let k = inner.len() + 1;
                let mut replacement = inner;
                replacement.push(ws(arena));
                replacement.push(arena.create_token(EnumToken::PercentageTokenType, TokenValue::Num(0.0), None));
                tokens.splice(i..i + 1, replacement);
                i += k;
            } else if str_is(arena, &tokens, i, "color-stop") {
                let inner = arena.tokens(tokens[i]);
                let k = inner.len().saturating_sub(1);
                let replacement: Vec<u32> = inner
                    .into_iter()
                    .rev()
                    .map(|t| match arena.data_types.get(t) {
                        EnumToken::CommaTokenType => ws(arena),
                        EnumToken::NumberTokenType => arena.create_token(EnumToken::PercentageTokenType, TokenValue::Num(arena.numeric_value.get(t) * 100.0), None),
                        _ => t,
                    })
                    .collect();
                tokens.splice(i..i + 1, replacement);
                i += k;
            }
        }
        i += 1;
    }
    tokens.extend(color_stop);
    if !kind.is_empty() {
        arena.string_value.set(token, arena.intern_string(kind));
        arena.set_token_list(token, &tokens);
    }
}

/// convert -webkit-radial-gradient to the radial-gradient syntax
fn webkit_radial_to_radial_gradient(arena: &AstArena, token: u32) -> Vec<u32> {
    let mut tokens = arena.tokens(token);
    let t = |tokens: &[u32], i: usize| typ_at(arena, tokens, i);
    let is_ws = |tokens: &[u32], i: usize| matches!(t(tokens, i), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType);
    let mut i = 0usize;
    while i < tokens.len() && is_ws(&tokens, i) {
        i += 1;
    }
    let mut positions: Vec<u32> = Vec::new();
    let mut form: Vec<u32> = Vec::new();
    let mut size: Vec<u32> = Vec::new();
    let position_keyword = ["center", "top", "bottom", "left", "right"].iter().any(|k| t(&tokens, i) == EnumToken::IdenTokenType && str_is(arena, &tokens, i, k));
    if t(&tokens, i) == EnumToken::PercentageTokenType
        || (t(&tokens, i) == EnumToken::NumberTokenType && arena.numeric_value.get(tokens[i]) == 0.0)
        || position_keyword
    {
        // the typescript loop does not stop at the end of the list: it stops at the last token here
        loop {
            positions.push(tokens[i]);
            i += 1;
            if i >= tokens.len() || t(&tokens, i) == EnumToken::CommaTokenType {
                break;
            }
        }
        if t(&tokens, i) == EnumToken::CommaTokenType {
            i += 1;
        }
    }
    for _ in 0..2 {
        if t(&tokens, i) != EnumToken::IdenTokenType {
            continue;
        }
        if ["closest-side", "closest-corner", "farthest-side", "farthest-corner"].iter().any(|k| str_is(arena, &tokens, i, k)) {
            size.push(tokens[i]);
            i += 1;
        } else if str_is(arena, &tokens, i, "circle") || str_is(arena, &tokens, i, "ellipse") {
            form.push(tokens[i]);
            i += 1;
        }
        while is_ws(&tokens, i) {
            i += 1;
        }
        if t(&tokens, i) == EnumToken::CommaTokenType {
            i += 1;
        }
    }
    let color_stops: Vec<u32> = tokens.get(i..).map_or_else(Vec::new, |s| s.to_vec());
    tokens.clear();
    if !form.is_empty() || !size.is_empty() {
        if form.is_empty() {
            form.push(iden(arena, "ellipse"));
        }
        if !size.is_empty() {
            form.push(ws(arena));
            form.extend(size);
        }
        if !positions.is_empty() {
            form.push(ws(arena));
            form.push(iden(arena, "at"));
            form.push(ws(arena));
            form.extend(positions);
        }
        tokens.extend(form);
        tokens.push(arena.create_token(EnumToken::CommaTokenType, TokenValue::None, None));
    }
    let repeating = arena.string_equals_ignore_case(arena.string_value.get(token), "-webkit-repeating-radial-gradient");
    arena.string_value.set(token, arena.intern_string(if repeating { "repeating-radial-gradient" } else { "radial-gradient" }));
    tokens.extend(color_stops);
    arena.set_token_list(token, &tokens);
    tokens
}
