//! Port of `src/parser/utils/declaration.ts`.

use crate::ast::arena::{AstArena, StrOrId, TokenField, TokenValue};
use crate::ast::math::expression::build_expression;
use crate::ast::strings::StringFlag;
use crate::ast::types::{ColorType, EnumAstNodeStatus, EnumToken};
use crate::options::ParserOptions;
use crate::parser::utils::text::{equals_ignore_case, lowercase};
use crate::syntax::constants::{
    color_name_to_hex, is_function_def_type, non_standard_colors, tokens_func_def_map, tokens_map, trim_token_space,
    DEPRECATED_SYSTEM_COLORS, SYSTEM_COLORS,
};
use crate::syntax::syntax::{is_color, is_color_name, is_white_space, parse_color, renamed_standard_properties};
use crate::types::ErrorDescription;
use crate::validation::config::{get_parsed_syntax, get_syntax_rule};
use crate::validation::match_::{match_declaration_syntax, token_location, trim_array, Syntax};
use crate::validation::parser::typedef::{ValidationSyntaxGroupEnum, ValidationTokenEnum};
use crate::validation::utils::list::split_token_list;
use std::rc::Rc;
use crate::ast::strings::SharedStr;

/// normalize the whitespace and the dots of a grid template string
fn parse_grid_template(template: &str) -> String {
    let chars: Vec<char> = template.chars().collect();
    let ws = |c: char| is_white_space(c as u32);
    let mut result = String::new();
    let mut buffer = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if ws(c) {
            while i + 1 < chars.len() && ws(chars[i + 1]) {
                i += 1;
            }
            result.push_str(&buffer);
            result.push(' ');
            buffer.clear();
        } else if c == '.' {
            while i + 1 < chars.len() && chars[i + 1] == '.' {
                i += 1;
            }
            if result.chars().last().is_some_and(ws) {
                result.pop();
            }
            result.push_str(&buffer);
            result.push(c);
            buffer.clear();
        } else {
            buffer.push(c);
        }
        i += 1;
    }
    if buffer.is_empty() {
        result
    } else {
        result + &buffer
    }
}

/// parse the color of an identifier or a color token
fn parse_color_token(arena: &AstArena, value: u32) {
    let t = arena.data_types.get(value);
    if (t == EnumToken::IdenTokenType || t == EnumToken::ColorTokenType) && is_color(arena, value, None) {
        parse_color(arena, value);
    }
}

/// parse the color of an identifier
fn parse_color_ident(arena: &AstArena, value: u32) {
    if arena.data_types.get(value) == EnumToken::IdenTokenType && is_color(arena, value, None) {
        parse_color(arena, value);
    }
}

fn create_declaration_node(arena: &AstArena, name: u32, tokens: &[u32], state: u32, errors: Option<Vec<ErrorDescription>>) -> u32 {
    let node = arena.create_declaration(EnumToken::DeclarationNodeType, StrOrId::Id(arena.string_value.get(name)), Some(tokens), state, errors);
    arena.copy_location(node, name);
    node
}

fn location_error(arena: &AstArena, message: &str, node: u32, options: &ParserOptions) -> ErrorDescription {
    let mut error = ErrorDescription::drop(message).with_node(node);
    error.location = token_location(arena, node, options);
    error
}

/// parse a declaration: the name token followed by the colon and the value tokens
pub fn parse_declaration(arena: &AstArena, tokens: &mut Vec<u32>, parent: Option<u32>, options: &ParserOptions, errors: &mut Vec<ErrorDescription>) -> u32 {
    let t = |x: u32| arena.data_types.get(x);
    let ty = |v: &Vec<u32>, i: isize| if i < 0 { None } else { v.get(i as usize).map(|&x| t(x)) };
    let name = tokens.remove(0);
    // the interned string, shared: no copy
    let mut nam: SharedStr = arena.resolve_string(arena.string_value.get(name));
    let is_at_rule = parent.is_some_and(|p| t(p) == EnumToken::AtRuleNodeType);
    let parent_nam: Option<String> = if is_at_rule { Some(arena.get_name(parent.unwrap()).to_string()) } else { None };
    let pn = parent_nam.as_deref();
    let mut syntax_rules: Option<Rc<Vec<Syntax>>> = None;
    let mut success = true;
    let mut has_important_flag = false;
    let validate = options.validation == Some(true);
    if let Some(renamed) = renamed_standard_properties(&nam) {
        nam = SharedStr::from(renamed);
        arena.string_value.set(name, arena.intern_string(&nam));
    }
    let mut i = 0usize;
    while i < tokens.len() && matches!(t(tokens[i]), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType | EnumToken::BadCdoTokenType) {
        i += 1;
    }
    if (t(name) != EnumToken::IdenTokenType && t(name) != EnumToken::DashedIdenTokenType) || ty(tokens, i as isize) != Some(EnumToken::ColonTokenType) {
        if let Some(&last) = tokens.last() {
            arena.span_location(name, name, last);
        }
        let node = arena.create_declaration(EnumToken::RawNodeTokenType, StrOrId::Str(""), Some(tokens), 0, None);
        arena.copy_location(node, name);
        return node;
    }
    let mut start = i + 1;
    let mut end = tokens.len();
    while start < end && t(tokens[start]) == EnumToken::WhitespaceTokenType {
        start += 1;
    }
    while end > start && t(tokens[end - 1]) == EnumToken::WhitespaceTokenType {
        end -= 1;
    }
    // the value tokens, trimmed in place: the buffer of the caller is reused for the next statement
    tokens.truncate(end);
    tokens.drain(..start.min(end));
    let lower_nam = lowercase(&nam);
    if validate && t(name) == EnumToken::IdenTokenType {
        if is_at_rule && pn != Some("import") && pn != Some("supports") && pn != Some("media") {
            if let Some(rules) = get_syntax_rule(ValidationSyntaxGroupEnum::AtRules, &format!("@{}", pn.unwrap())) {
                syntax_rules = match rules.get_property_descriptors() {
                    Some(descriptors) => descriptors.get(&*lower_nam).cloned(),
                    None => {
                        if rules.accept_any_declaration && rules.accept_any_rule {
                            get_parsed_syntax(ValidationSyntaxGroupEnum::Declarations, &lower_nam)
                        } else {
                            rules.get_block_rules()
                        }
                    }
                };
            }
        } else {
            syntax_rules = get_parsed_syntax(ValidationSyntaxGroupEnum::Declarations, &lower_nam);
        }
        let generic = syntax_rules.as_ref().is_none_or(|s| {
            s.len() == 1
                && s[0].typ == ValidationTokenEnum::PropertyType
                && matches!(s[0].val_str(), "declaration-list" | "declaration-rule-list" | "block-contents")
        });
        if generic {
            syntax_rules = if is_at_rule && matches!(pn, Some("swash" | "annotation" | "ornaments" | "stylistic" | "historical-forms")) {
                get_parsed_syntax(ValidationSyntaxGroupEnum::Syntaxes, "font-feature-custom-ident")
            } else if is_at_rule && pn == Some("styleset") {
                get_parsed_syntax(ValidationSyntaxGroupEnum::Syntaxes, "font-feature-custom-ident-list")
            } else if is_at_rule && pn == Some("character-variant") {
                get_parsed_syntax(ValidationSyntaxGroupEnum::Syntaxes, "font-feature-custom-ident-character-variant")
            } else {
                get_parsed_syntax(ValidationSyntaxGroupEnum::Declarations, &lower_nam)
            };
        }
    }
    if tokens.is_empty()
        && t(name) != EnumToken::DashedIdenTokenType
        && !(pn == Some("function") && arena.string_equals_ignore_case(arena.string_value.get(name), "result"))
    {
        let error = location_error(arena, "declaration value missing", name, options);
        errors.push(error.clone());
        return create_declaration_node(arena, name, &tokens, EnumAstNodeStatus::Invalid, Some(vec![error]));
    }
    let mut stack: Vec<u32> = Vec::new();
    let mut result_errors: Option<Vec<ErrorDescription>> = None;
    let mut has_result = false;
    if let Some(rules) = &syntax_rules {
        let do_not_validate = options.validation == Some(false);
        if do_not_validate {
            arena.visit_values(&tokens, |_, v| parse_color_token(arena, v));
        } else {
            let result = match_declaration_syntax(arena, rules, &tokens, options);
            has_result = true;
            success = result.success;
            if success {
                arena.visit_values(&tokens, |_, v| parse_color_token(arena, v));
            } else {
                errors.extend(result.errors.iter().cloned());
            }
            result_errors = Some(result.errors);
        }
    }
    let mut i: isize = 0;
    while (i as usize) < tokens.len() {
        let token = tokens[i as usize];
        let tt = t(token);
        if is_function_def_type(tt) || tt == EnumToken::StartParensTokenType || tt == EnumToken::BlockStartTokenType {
            stack.push(token);
            i += 1;
            continue;
        }
        if tt == EnumToken::ColonTokenType && stack.is_empty() {
            stack.push(token);
            i += 1;
            continue;
        }
        let top = stack.last().map(|&s| t(s));
        match tt {
            EnumToken::Literal => {
                if top == Some(EnumToken::MathFunctionTokenDefType) {
                    if arena.string_equals(arena.string_value.get(token), "/") {
                        arena.data_types.set(token, EnumToken::Div);
                    } else if arena.string_equals(arena.string_value.get(token), "-") {
                        arena.data_types.set(token, EnumToken::Sub);
                    }
                }
            }
            EnumToken::Star | EnumToken::Plus => arena.data_types.set(token, tokens_map(tt).unwrap_or(0)),
            EnumToken::BlockEndTokenType => {
                if top == Some(EnumToken::BlockStartTokenType) {
                    let open = *stack.last().unwrap();
                    let index = tokens.iter().position(|&x| x == open).unwrap();
                    tokens.remove(i as usize);
                    arena.data_types.set(tokens[index], EnumToken::WrappedValuesTokenType);
                    let mut inner: Vec<u32> = tokens.drain(index + 1..i as usize).collect();
                    trim_array(arena, &mut inner);
                    arena.set_token_list(tokens[index], &inner);
                    i = index as isize;
                    stack.pop();
                }
            }
            EnumToken::EndParensTokenType => {
                if top.is_some_and(|s| s == EnumToken::StartParensTokenType || is_function_def_type(s)) {
                    let open = *stack.last().unwrap();
                    let index = tokens.iter().position(|&x| x == open).unwrap();
                    tokens.remove(i as usize);
                    let func = tokens[index];
                    arena.data_types.set(
                        func,
                        if top == Some(EnumToken::StartParensTokenType) { EnumToken::ParensTokenType } else { tokens_func_def_map(top.unwrap()).unwrap_or(0) },
                    );
                    let mut inner: Vec<u32> = tokens.drain(index + 1..i as usize).collect();
                    trim_array(arena, &mut inner);
                    arena.set_token_list(func, &inner);
                    if t(func) == EnumToken::WildCardFunctionTokenType && arena.string_equals_ignore_case(arena.string_value.get(func), "if") {
                        build_if_function(arena, func);
                    } else if t(func) == EnumToken::UrlFunctionTokenType {
                        if let Some(url) = arena.find_token(func, |a, x| a.data_types.get(x) == EnumToken::StringTokenType) {
                            let value = arena.resolve_string(arena.string_value.get(url));
                            let inner: String = {
                                let chars: Vec<char> = value.chars().collect();
                                if chars.len() >= 2 { chars[1..chars.len() - 1].iter().collect() } else { String::new() }
                            };
                            if !inner.is_empty() && inner.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'.' | b'-')) {
                                arena.data_types.set(url, EnumToken::UrlTokenTokenType);
                                arena.string_value.set(url, arena.intern_string(&inner));
                            }
                        }
                    }
                    i = index as isize;
                    if t(func) == EnumToken::MathFunctionTokenType && arena.string_equals_ignore_case(arena.string_value.get(func), "calc") {
                        let expression = build_expression(arena, &arena.tokens(func));
                        arena.set_token_list(func, &[expression]);
                    } else if t(func) == EnumToken::ColorTokenType {
                        if is_color(arena, func, None) {
                            parse_color(arena, func);
                        } else {
                            success = false;
                            arena.data_types.set(func, EnumToken::FunctionTokenType);
                            errors.push(location_error(arena, "invalid color", func, options));
                        }
                    }
                    stack.pop();
                }
            }
            EnumToken::IdenTokenType => {
                if arena.string_flag(arena.string_value.get(token), StringFlag::Color, is_color_name) {
                    let val = lowercase(&arena.resolve_string(arena.string_value.get(token))).into_owned();
                    if color_name_to_hex(&val).is_some() || val == "currentcolor" {
                        arena.string_value.set(token, arena.lower_string_id(arena.string_value.get(token)));
                        arena.data_types.set(token, EnumToken::ColorTokenType);
                        arena.color_type.set(token, ColorType::LIT);
                    } else if non_standard_colors().contains(&val) {
                        arena.data_types.set(token, EnumToken::ColorTokenType);
                        arena.color_type.set(token, ColorType::NON_STD);
                    } else if SYSTEM_COLORS.contains(&val.as_str()) {
                        arena.data_types.set(token, EnumToken::ColorTokenType);
                        arena.color_type.set(token, ColorType::SYS);
                    } else if DEPRECATED_SYSTEM_COLORS.contains(&val.as_str()) {
                        arena.data_types.set(token, EnumToken::ColorTokenType);
                        arena.color_type.set(token, ColorType::DPSYS);
                    }
                }
            }
            EnumToken::ImportantTokenType => has_important_flag = true,
            _ => {}
        }
        if (i as usize) < tokens.len() && trim_token_space(t(tokens[i as usize])) {
            if ty(&tokens, i + 1) == Some(EnumToken::WhitespaceTokenType) {
                tokens.remove(i as usize + 1);
                // `i--; continue`: the token is checked again
                continue;
            }
            if ty(&tokens, i - 1) == Some(EnumToken::WhitespaceTokenType) {
                tokens.remove(i as usize - 1);
                i -= 1;
            }
        }
        i += 1;
    }
    if let Some(&top) = stack.last() {
        errors.push(location_error(arena, "unbalanced token", top, options));
        if let Some(&last) = tokens.last() {
            arena.span_location(name, name, last);
        }
        return create_declaration_node(arena, name, &tokens, EnumAstNodeStatus::Invalid, result_errors);
    }
    arena.visit_values(&tokens, |_, v| parse_color_ident(arena, v));
    if matches!(&*nam, "grid" | "grid-template-areas" | "grid-template-rows" | "grid-template-columns") {
        let mut i = 0;
        while i < tokens.len() {
            if t(tokens[i]) == EnumToken::StringTokenType {
                let s = arena.resolve_string(arena.string_value.get(tokens[i]));
                if s.chars().any(|c| c == '.' || crate::ast::arena::is_js_whitespace(c)) {
                    arena.string_value.set(tokens[i], arena.intern_string(&parse_grid_template(&s)));
                }
                if ty(&tokens, i as isize + 1) == Some(EnumToken::WhitespaceTokenType) && ty(&tokens, i as isize + 2) == Some(EnumToken::StringTokenType) {
                    tokens.remove(i + 1);
                }
            }
            i += 1;
        }
    }
    if validate && syntax_rules.is_none() && t(name) == EnumToken::IdenTokenType {
        if let Some(&last) = tokens.last() {
            arena.span_location(name, name, last);
        }
        let state = if has_important_flag { EnumAstNodeStatus::Unknown | EnumAstNodeStatus::ImportantDeclaration } else { EnumAstNodeStatus::Unknown };
        return create_declaration_node(arena, name, &tokens, state, result_errors);
    }
    if equals_ignore_case("composes", &nam) {
        let index = tokens
            .iter()
            .position(|&x| t(x) == EnumToken::IdenTokenType && arena.string_equals_ignore_case(arena.string_value.get(x), "from"));
        let first = tokens.first().copied();
        let mut left: Vec<u32> = match index {
            None => tokens.clone(),
            Some(index) => tokens[..index].to_vec(),
        };
        trim_array(arena, &mut left);
        let right: Option<Vec<u32>> = index.map(|index| {
            let mut r = tokens[index + 1..].to_vec();
            trim_array(arena, &mut r);
            r
        });
        let composes = arena.create_token(EnumToken::ComposesSelectorNodeType, TokenValue::None, None);
        let last = match &right {
            Some(r) if !r.is_empty() => r.last().copied(),
            _ => left.last().copied(),
        };
        arena.set_token_list(arena.field_list(composes, TokenField::L), &left);
        if let Some(r) = right.as_ref().filter(|r| !r.is_empty()) {
            arena.set_token_list(arena.field_list(composes, TokenField::R), &[r[0]]);
        }
        if let (Some(first), Some(last)) = (first, last) {
            arena.span_location(composes, first, last);
        }
        tokens.clear();
        tokens.push(composes);
    }
    if let Some(&last) = tokens.last() {
        arena.span_location(name, name, last);
    }
    let mut state = if success {
        if has_result { EnumAstNodeStatus::Validated } else { EnumAstNodeStatus::Unvalidated }
    } else {
        EnumAstNodeStatus::ValidationFailed
    };
    if has_important_flag {
        state |= EnumAstNodeStatus::ImportantDeclaration;
    }
    create_declaration_node(arena, name, &tokens, state, result_errors)
}

/// `if()`: the conditions are split by semicolons into `IfConditionTokenType` / `IfElseConditionTokenType` tokens
fn build_if_function(arena: &AstArena, func: u32) {
    let t = |x: u32| arena.data_types.get(x);
    let last = arena.last_token.get(func);
    if last != 0 && t(last) == EnumToken::SemiColonTokenType {
        arena.detach(last);
    }
    let mut found_else = false;
    let mut acc: Vec<u32> = Vec::new();
    for curr in split_token_list(arena, &arena.tokens(func), &[EnumToken::SemiColonTokenType], true) {
        if found_else {
            continue;
        }
        let mut index = 0usize;
        while index < curr.len() {
            if t(curr[index]) == EnumToken::IdenTokenType && arena.string_equals_ignore_case(arena.string_value.get(curr[index]), "else") {
                found_else = true;
            }
            if t(curr[index]) == EnumToken::ColonTokenType {
                break;
            }
            index += 1;
        }
        let condition = arena.create_token(EnumToken::IfConditionTokenType, TokenValue::None, None);
        let mut left: Vec<u32> = curr[..index.min(curr.len())].to_vec();
        trim_array(arena, &mut left);
        arena.set_token_list(arena.field_list(condition, TokenField::L), &left);
        let right_end = if found_else && !acc.is_empty() && curr.last().is_some_and(|&x| t(x) == EnumToken::SemiColonTokenType) {
            curr.len() - 1
        } else {
            curr.len()
        };
        let mut right: Vec<u32> = if index + 1 <= right_end { curr[index + 1..right_end].to_vec() } else { Vec::new() };
        trim_array(arena, &mut right);
        arena.set_token_list(arena.field_list(condition, TokenField::R), &right);
        if found_else && !acc.is_empty() {
            let if_else = arena.create_token(EnumToken::IfElseConditionTokenType, TokenValue::None, None);
            let previous = *acc.last().unwrap();
            arena.set_token_list(arena.field_list(if_else, TokenField::L), &[previous]);
            arena.set_token_list(arena.field_list(if_else, TokenField::R), &[condition]);
            *acc.last_mut().unwrap() = if_else;
        } else {
            acc.push(condition);
        }
    }
    arena.set_token_list(func, &acc);
}
