//! Port of the second half of `src/validation/match.ts`: `matchSyntax()`, the unordered syntaxes (`||`, `&&`)
//! and `matchProperty()` (the built-in types, see `match_property.rs`).

use super::config::get_parsed_syntax;
use super::first::{is_nullable, matching_alternatives, may_start_with};
use super::match_::{
    create_validation_context, find_mandatory_syntax, function_syntax, is_css_wide_keyword, is_func_type, is_in_range,
    is_literal_number, match_list_syntax, match_occurence_syntax, match_wild_card_function, set_done_match, trim_array,
    wild_card_probe, Context, ContextExt, Syntax, ValidationMatch,
};
use super::parser::typedef::{ValidationSyntaxGroupEnum, ValidationTokenEnum as V};
use super::parser::types::ValidationToken;
use crate::ast::arena::AstArena;
use crate::ast::types::EnumToken;
use crate::options::ParserOptions;
use crate::parser::utils::text::{equals_ignore_case, lowercase};
use crate::syntax::constants::{is_function_def_type, is_function_type, FUNC_LIKE};
use crate::types::{ErrorDescription, ErrorSyntax};
use crate::utils::fx::HashMap;
use std::rc::Rc;

pub use super::match_property::match_property;

fn fail(token: Option<u32>, context: &Context, syntax: Option<Syntax>, errors: Vec<ErrorDescription>) -> ValidationMatch {
    ValidationMatch::new(false, true, token, Some(context.clone()), syntax, errors)
}

fn syntax_error(message: &str, token: Option<u32>, syntax: &Syntax) -> ErrorDescription {
    let mut error = ErrorDescription::drop(message);
    error.node = token;
    error.syntax = Some(ErrorSyntax::Token(syntax.clone()));
    error
}

/// first item of a function syntax (`name()`): its arguments
fn function_chi(syntax: Option<Rc<Vec<Syntax>>>) -> Option<Vec<Syntax>> {
    syntax.and_then(|s| s.first().map(|f| f.children().to_vec()))
}

/// outcome of a syntax of the sequence
enum Step {
    /// continue with the next syntax. `success`: the syntax matched
    Next(bool),
    /// return this result
    Return(ValidationMatch),
}

/// match a sequence of syntaxes
pub fn match_syntax(arena: &AstArena, syntaxes: Option<&[Syntax]>, context: &Context, options: &ParserOptions) -> ValidationMatch {
    let Some(syntaxes) = syntaxes else {
        return ValidationMatch::new(true, false, context.peek(arena), Some(context.clone()), None, Vec::new());
    };
    let mut i: isize = -1;
    let mut token: Option<u32> = None;
    let mut result: Option<ValidationMatch> = None;
    {
        let ctx = context.borrow();
        let tokens = ctx.tokens();
        if tokens.len() == 1
            && arena.data_types.get(tokens[0]) == EnumToken::IdenTokenType
            && arena.with_string(arena.string_value.get(tokens[0]), is_css_wide_keyword)
        {
            drop(ctx);
            context.end();
            return ValidationMatch::new(true, true, context.peek(arena), Some(context.clone()), None, Vec::new());
        }
    }
    loop {
        i += 1;
        if i as usize >= syntaxes.len() {
            break;
        }
        let iu = i as usize;
        let syntax = &syntaxes[iu];
        if syntax.typ == V::Whitespace {
            continue;
        }
        let is_optional = (syntax.is_optional() || syntax.is_repeatable()) && !syntax.is_mandatatory_group();
        token = context.peek(arena);
        let Some(tk) = token else {
            let mandatory = find_mandatory_syntax(syntaxes, if is_optional { iu + 1 } else { iu });
            if mandatory != -1 {
                let m = syntaxes[mandatory as usize].clone();
                return ValidationMatch::new(false, true, None, Some(context.clone()), Some(m.clone()), vec![syntax_error("could not match syntax", None, &m)]);
            }
            break;
        };
        let probe = wild_card_probe(tk);
        let probed = probe.as_ref().is_some_and(|p| Rc::ptr_eq(p, syntax));
        if arena.data_types.get(tk) == EnumToken::WildCardFunctionTokenType && arena.string_equals(arena.string_value.get(tk), "var") && !probed {
            let r = match_syntax(
                arena,
                function_chi(function_syntax(arena, tk)).as_deref(),
                &create_validation_context(arena, &arena.tokens(tk)),
                options,
            );
            let ok = r.success && r.ctx().done(arena);
            result = Some(r);
            if ok {
                match match_wild_card_function(arena, syntaxes, iu, context, tk, Some(tk), options) {
                    None => continue,
                    Some(w) => return w,
                }
            }
        }
        // a custom function call (`--fn()`) is substituted by the result of the function: like var(), it may be
        // substituted by any number of values
        let custom = arena.data_types.get(tk);
        if (custom == EnumToken::CustomFunctionTokenType || custom == EnumToken::CustomFunctionTokenDefType) && !probed {
            // the arguments of the call are not validated: they are the arguments of the custom function
            let end = if custom == EnumToken::CustomFunctionTokenDefType {
                let mut range = context.borrow().peek_range(arena, EnumToken::EndParensTokenType, 0);
                trim_array(arena, &mut range);
                range.last().copied()
            } else {
                Some(tk)
            };
            match match_wild_card_function(arena, syntaxes, iu, context, tk, end, options) {
                None => continue,
                Some(w) => return w,
            }
        }
        if arena.data_types.get(tk) == EnumToken::WildCardFunctionTokenDefType && !probed {
            let mut range = context.borrow().peek_range(arena, EnumToken::EndParensTokenType, 0);
            trim_array(arena, &mut range);
            let inner: &[u32] = if range.len() >= 2 { &range[1..range.len() - 1] } else { &[] };
            let r = match_syntax(arena, function_chi(function_syntax(arena, tk)).as_deref(), &create_validation_context(arena, inner), options);
            let ok = r.success;
            result = Some(r);
            if ok {
                match match_wild_card_function(arena, syntaxes, iu, context, tk, range.last().copied(), options) {
                    None => continue,
                    Some(w) => return w,
                }
            }
        }
        if syntax.is_repeatable_at_least_once() || syntax.is_repeatable() {
            let r = match_repeatable_syntax(arena, syntax, &context.slice(), options);
            if r.success {
                if r.ctx().done(arena) {
                    context.end();
                    let ctx = r.ctx().clone();
                    return set_done_match(syntaxes, iu + 1, r.valid, r.token, &ctx, r.errors);
                }
                context.update(r.ctx().current());
                result = Some(r);
                continue;
            }
            if is_optional {
                result = Some(r);
                continue;
            }
            return r;
        }
        if syntax.is_list() {
            let r = match_list_syntax(arena, syntax, &context.slice(), options);
            if r.success {
                if r.ctx().done(arena) {
                    context.end();
                    return set_done_match(syntaxes, iu + 1, true, None, context, r.errors);
                }
                context.update(r.ctx().current());
                result = Some(r);
                continue;
            }
            if is_optional {
                result = Some(r);
                continue;
            }
            return fail(Some(tk), context, Some(syntax.clone()), r.errors);
        }
        if syntax.match_.is_some() {
            let r = match_occurence_syntax(arena, syntax, &context.slice(), options);
            if r.success {
                if r.ctx().done(arena) {
                    context.end();
                    return set_done_match(syntaxes, iu + 1, r.valid, r.token, context, r.errors);
                }
                context.update(r.ctx().current());
                result = Some(r);
                continue;
            }
            if is_optional {
                result = Some(r);
                continue;
            }
            return fail(Some(tk), context, Some(syntax.clone()), r.errors);
        }
        if super::match_::is_visited(tk, syntax) {
            return fail(Some(tk), context, Some(syntax.clone()), Vec::new());
        }
        let visit = super::match_::enter_syntax(tk, syntax);
        let step = match_one(arena, syntaxes, iu, syntax, tk, is_optional, context, options, visit, &mut result);
        // finally: the syntax is no longer being matched
        super::match_::leave_syntax(visit);
        match step {
            Step::Return(r) => return r,
            Step::Next(success) => {
                if success && !is_in_range(arena, tk, syntax) {
                    return fail(Some(tk), context, Some(syntax.clone()), Vec::new());
                }
            }
        }
    }
    let (valid, errors) = match &result {
        Some(r) if r.success => (r.valid, r.errors.clone()),
        _ => (true, Vec::new()),
    };
    ValidationMatch::new(true, valid, token, Some(context.clone()), syntaxes.get(i.max(0) as usize).cloned(), errors)
}

/// match the syntax at index (the body of the `switch` of `matchSyntax()`)
#[allow(clippy::too_many_arguments)]
fn match_one(
    arena: &AstArena,
    syntaxes: &[Syntax],
    i: usize,
    syntax: &Syntax,
    token: u32,
    is_optional: bool,
    context: &Context,
    options: &ParserOptions,
    visit: usize,
    result: &mut Option<ValidationMatch>,
) -> Step {
    let t = arena.data_types.get(token);
    let matched = |visit: usize| super::match_::clear_visited(visit);
    let no = || Step::Return(fail(Some(token), context, Some(syntax.clone()), Vec::new()));
    match syntax.typ {
        V::Colon => {
            if t == EnumToken::ColonTokenType {
                context.next_token(arena);
                return Step::Next(true);
            }
            if is_optional {
                return Step::Next(false);
            }
            no()
        }
        V::Comma => {
            if t == EnumToken::CommaTokenType {
                matched(visit);
                context.next_token(arena);
                return Step::Next(true);
            }
            no()
        }
        V::Keyword => {
            if t == EnumToken::IdenTokenType && arena.with_string(arena.string_value.get(token), |v| lowercase(v) == lowercase(syntax.val_str())) {
                matched(visit);
                context.next_token(arena);
                return Step::Next(true);
            }
            if is_optional {
                return Step::Next(false);
            }
            no()
        }
        V::DeclarationType => {
            let r = match_syntax(
                arena,
                get_parsed_syntax(ValidationSyntaxGroupEnum::Declarations, syntax.val_str()).as_deref().map(|v| v.as_slice()),
                context,
                options,
            );
            let mut success = false;
            if r.success {
                success = true;
                matched(visit);
                if r.ctx().done(arena) {
                    context.end();
                    return Step::Return(set_done_match(syntaxes, i + 1, r.valid, Some(token), context, r.errors));
                } else if r.ctx().current().is_some() {
                    context.update(r.ctx().current());
                }
            }
            let errors = r.errors.clone();
            *result = Some(r);
            if success || is_optional {
                return Step::Next(success);
            }
            Step::Return(fail(Some(token), context, Some(syntax.clone()), errors))
        }
        V::OptionalGroupToken => {
            let chi = syntax.children();
            if chi.len() == 1 && chi[0].is_list() {
                let mut item = (*chi[0]).clone();
                item.is_list = None;
                item.is_optional = None;
                item.match_ = None;
                let group = [Rc::new(item), Rc::new(ValidationToken::new(V::Comma))];
                while context.peek(arena).is_some() {
                    let tmp = match_syntax(arena, Some(&group), &context.slice(), options);
                    if !tmp.success || tmp.ctx().current().is_none() {
                        break;
                    }
                    context.update(tmp.ctx().current());
                    if tmp.ctx().done(arena) {
                        break;
                    }
                }
                return Step::Next(true);
            }
            let r = match_syntax(arena, Some(chi), &context.slice(), options);
            let mut success = false;
            if r.success {
                success = true;
                if r.ctx().done(arena) {
                    context.end();
                    return Step::Return(set_done_match(syntaxes, i + 1, true, None, context, r.errors));
                }
                context.update(r.ctx().current());
            }
            *result = Some(r);
            Step::Next(success)
        }
        V::StringToken => {
            let val = syntax.val_str();
            let inner: String = {
                let chars: Vec<char> = val.chars().collect();
                if chars.len() >= 2 { chars[1..chars.len() - 1].iter().collect() } else { String::new() }
            };
            let success = match t {
                EnumToken::Add => inner == "+",
                EnumToken::Sub => inner == "-",
                EnumToken::LiteralTokenType => arena.string_equals(arena.string_value.get(token), &inner),
                EnumToken::Plus => true,
                _ => false,
            };
            if success {
                context.next_token(arena);
                return Step::Next(true);
            }
            if is_optional {
                return Step::Next(false);
            }
            no()
        }
        V::PropertyType => {
            let r = match_property(arena, syntax, context, options);
            if !r.success {
                *result = Some(r);
                if is_optional {
                    return Step::Next(false);
                }
                return Step::Return(fail(context.current().or(Some(token)), context, Some(syntax.clone()), Vec::new()));
            }
            matched(visit);
            if r.ctx().done(arena) {
                if !is_in_range(arena, token, syntax) {
                    return no();
                }
                context.end();
                return Step::Return(set_done_match(syntaxes, i + 1, true, None, context, Vec::new()));
            }
            context.update(r.ctx().current());
            *result = Some(r);
            Step::Next(true)
        }
        V::PipeToken => {
            let alternatives = syntax.alternatives();
            let candidates = matching_alternatives(arena, alternatives, Some(token));
            let count = candidates.as_ref().map_or(alternatives.len(), |c| c.len());
            let mut best: Option<ValidationMatch> = None;
            for k in 0..count {
                let alternative = &alternatives[candidates.as_ref().map_or(k, |c| c[k])];
                let tmp = match_syntax(arena, Some(alternative), &context.slice(), options);
                if tmp.success {
                    if tmp.ctx().done(arena) && find_mandatory_syntax(syntaxes, i + 1) != -1 {
                        continue;
                    }
                    if best.as_ref().is_none_or(|b| tmp.ctx().index() >= b.ctx().index()) {
                        best = Some(tmp.clone());
                    }
                    if tmp.ctx().done(arena) {
                        context.end();
                        return Step::Return(ValidationMatch::new(
                            tmp.success,
                            tmp.valid,
                            tmp.token,
                            Some(context.clone()),
                            syntaxes.get(i + 1).cloned(),
                            tmp.errors,
                        ));
                    }
                }
            }
            *result = best.clone();
            if let Some(b) = best.filter(|b| b.success) {
                matched(visit);
                context.update(b.ctx().current());
                return Step::Next(true);
            }
            Step::Return(fail(Some(token), context, Some(syntax.clone()), vec![syntax_error("could not match syntax", Some(token), syntax)]))
        }
        V::Bracket => {
            let r = match_syntax(arena, Some(syntax.children()), &context.slice(), options);
            if r.success {
                matched(visit);
                if r.ctx().done(arena) {
                    context.end();
                    return Step::Return(set_done_match(syntaxes, i + 1, true, None, context, Vec::new()));
                }
                context.update(r.ctx().current());
                *result = Some(r);
                return Step::Next(true);
            }
            *result = Some(r);
            if is_optional {
                return Step::Next(false);
            }
            no()
        }
        V::ColumnToken | V::AmpersandToken => {
            let r = if syntax.typ == V::ColumnToken {
                match_column_syntax(arena, syntax, &context.slice(), options)
            } else {
                match_ampersand_syntax(arena, syntax, &context.slice(), options)
            };
            if r.success {
                matched(visit);
                if r.ctx().done(arena) {
                    context.end();
                    return Step::Return(set_done_match(syntaxes, i + 1, true, None, context, Vec::new()));
                }
                context.update(r.ctx().current());
                *result = Some(r);
                return Step::Next(true);
            }
            *result = Some(r);
            no()
        }
        V::Function => {
            let is_func = is_function_def_type(t) || FUNC_LIKE.contains(&t) || is_func_type(t);
            let name_ok = arena.with_string(arena.string_value.get(token), |v| equals_ignore_case(syntax.val_str(), &v.to_lowercase()));
            if !is_func || !name_ok {
                let mut error = ErrorDescription::drop(format!("Expecting a function {}", EnumToken::name(t).unwrap_or("undefined")));
                error.node = Some(token);
                return Step::Return(fail(Some(token), context, Some(syntax.clone()), vec![error]));
            }
            let r = if is_function_def_type(t) {
                let range = context.borrow().peek_range(arena, EnumToken::EndParensTokenType, 0);
                let inner: &[u32] = if range.len() >= 2 { &range[1..range.len() - 1] } else { &[] };
                let r = match_syntax(arena, Some(syntax.children()), &create_validation_context(arena, inner), options);
                if r.success && r.ctx().done(arena) {
                    context.update(range.last().copied());
                    matched(visit);
                    *result = Some(r);
                    // typescript: `success` is not set for a function definition token
                    return Step::Next(false);
                }
                r
            } else {
                let r = match_syntax(arena, Some(syntax.children()), &create_validation_context(arena, &arena.tokens(token)), options);
                if r.success && r.ctx().done(arena) {
                    matched(visit);
                    context.next_token(arena);
                    *result = Some(r);
                    return Step::Next(true);
                }
                r
            };
            let errors = r.errors.clone();
            *result = Some(r);
            Step::Return(fail(Some(token), context, Some(syntax.clone()), errors))
        }
        V::Number | V::Dimension => {
            if is_literal_number(arena, token, syntax) {
                matched(visit);
                context.next_token(arena);
                return Step::Next(true);
            }
            if is_optional {
                return Step::Next(false);
            }
            no()
        }
        V::Separator => {
            if t == EnumToken::LiteralTokenType && arena.string_equals(arena.string_value.get(token), "/") {
                matched(visit);
                context.next_token(arena);
                return Step::Next(false);
            }
            no()
        }
        V::FunctionDefinition => {
            let name = arena.resolve_string(arena.string_value.get(token));
            if !name.is_empty() && equals_ignore_case(&name, syntax.val_str()) {
                let parsed = get_parsed_syntax(ValidationSyntaxGroupEnum::Syntaxes, &format!("{}()", syntax.val_str()));
                if is_function_def_type(t) {
                    let mut children = context.borrow().peek_range(arena, EnumToken::EndParensTokenType, 0);
                    trim_array(arena, &mut children);
                    let chi = parsed.as_ref().and_then(|p| p.first()).map(|f| f.children().to_vec()).unwrap_or_default();
                    let inner: &[u32] = if children.len() >= 2 { &children[1..children.len() - 1] } else { &[] };
                    let r = match_syntax(arena, Some(&chi), &create_validation_context(arena, inner), options);
                    let ok = r.success && r.ctx().done(arena);
                    *result = Some(r);
                    if ok {
                        matched(visit);
                        context.update(children.last().copied());
                        return Step::Next(true);
                    }
                } else if is_function_type(t) {
                    let all: Vec<Syntax> = parsed.as_deref().cloned().unwrap_or_default();
                    let r = match_syntax(arena, Some(&all), &create_validation_context(arena, &[token]), options);
                    let ok = r.success && r.ctx().done(arena);
                    *result = Some(r);
                    if ok {
                        matched(visit);
                        context.next_token(arena);
                        return Step::Next(true);
                    }
                }
            }
            no()
        }
        V::OpenParenthesis | V::OpenCurlyBrace | V::CloseParenthesis => {
            let expected = match syntax.typ {
                V::OpenParenthesis => EnumToken::StartParensTokenType,
                V::OpenCurlyBrace => EnumToken::BlockStartTokenType,
                _ => EnumToken::EndParensTokenType,
            };
            if t == expected {
                matched(visit);
                context.next_token(arena);
                return Step::Next(false);
            }
            no()
        }
        // typescript throws `Unexpected syntax`
        _ => no(),
    }
}

/// matches a column of syntaxes: `<a> || <b> || <c>`, one or more syntaxes in any order
fn match_column_syntax(arena: &AstArena, syntax: &Syntax, context: &Context, options: &ParserOptions) -> ValidationMatch {
    match_unordered_syntaxes(arena, syntax, syntax.alternatives(), false, context, options)
}

/// matches syntaxes in any order: all the syntaxes (`&&`) or one or more syntaxes (`||`)
fn match_unordered_syntaxes(
    arena: &AstArena,
    syntax: &Syntax,
    syntaxes: &[Vec<Syntax>],
    all: bool,
    context: &Context,
    options: &ParserOptions,
) -> ValidationMatch {
    let remaining: Vec<usize> = (0..syntaxes.len()).collect();
    let mut cache: HashMap<(usize, i64), Option<Context>> = HashMap::default();
    let best = search_unordered_syntaxes(arena, syntaxes, &remaining, all, context, context, options, &mut cache);
    if let Some(best) = &best {
        if best.done(arena) {
            context.end();
        } else {
            context.update(best.current());
        }
    }
    let success = best.is_some() || (all && syntaxes.iter().all(|s| find_mandatory_syntax(s, 0) == -1));
    ValidationMatch::new(success, true, context.current(), Some(context.clone()), Some(syntax.clone()), Vec::new())
}

/// position of a context in the tokens of the unordered syntaxes
fn unordered_position(arena: &AstArena, base: &Context, context: &Context) -> i64 {
    if context.done(arena) {
        return i64::MAX;
    }
    let current = context.current();
    base.borrow().tokens().iter().position(|&t| Some(t) == current).map_or(-1, |p| p as i64)
}

/// the context after the unordered syntaxes that match the most tokens. None if the syntaxes do not match
#[allow(clippy::too_many_arguments)]
fn search_unordered_syntaxes(
    arena: &AstArena,
    syntaxes: &[Vec<Syntax>],
    remaining: &[usize],
    all: bool,
    base: &Context,
    context: &Context,
    options: &ParserOptions,
    cache: &mut HashMap<(usize, i64), Option<Context>>,
) -> Option<Context> {
    let position = unordered_position(arena, base, context);
    let mut best: Option<Context> = None;
    for k in 0..remaining.len() {
        let key = (remaining[k], position);
        let next = match cache.get(&key) {
            Some(n) => n.clone(),
            None => {
                let mut next = None;
                let result = if may_start_with(arena, &syntaxes[remaining[k]], context.peek(arena)) {
                    Some(match_syntax(arena, Some(&syntaxes[remaining[k]]), &context.slice(), options))
                } else {
                    None
                };
                if let Some(r) = result {
                    if r.success && (r.ctx().done(arena) || r.ctx().current() != context.current()) {
                        let n = context.slice();
                        if r.ctx().done(arena) {
                            n.end();
                        } else {
                            n.update(r.ctx().current());
                        }
                        next = Some(n);
                    }
                }
                cache.insert(key, next.clone());
                next
            }
        };
        let Some(next) = next else {
            continue;
        };
        let rest: Vec<usize> = remaining[..k].iter().chain(&remaining[k + 1..]).copied().collect();
        let complete = !all || rest.iter().all(|&index| find_mandatory_syntax(&syntaxes[index], 0) == -1);
        let mut candidates: Vec<Context> = Vec::new();
        if !next.done(arena) && !rest.is_empty() {
            if let Some(deeper) = search_unordered_syntaxes(arena, syntaxes, &rest, all, base, &next, options, cache) {
                candidates.push(deeper);
            }
        }
        if complete {
            candidates.push(next);
        }
        for candidate in candidates {
            if candidate.done(arena) {
                return Some(candidate);
            }
            if best.as_ref().is_none_or(|b| unordered_position(arena, base, &candidate) > unordered_position(arena, base, b)) {
                best = Some(candidate);
            }
        }
    }
    best
}

/// the operands of chained ampersands: `<a> && <b> && <c>` is parsed as `[ <a> && <b> ] && <c>`
fn ampersand_operands(syntax: &ValidationToken, operands: &mut Vec<Vec<Syntax>>) {
    for side in [syntax.l.as_deref().unwrap_or(&[]), syntax.r.as_deref().unwrap_or(&[])] {
        let tokens: Vec<&Syntax> = side.iter().filter(|t| t.typ != V::Whitespace).collect();
        if tokens.len() == 1
            && tokens[0].typ == V::AmpersandToken
            && !tokens[0].is_optional()
            && !tokens[0].is_repeatable()
            && !tokens[0].is_repeatable_at_least_once()
            && !tokens[0].is_list()
            && tokens[0].match_.is_none()
        {
            ampersand_operands(tokens[0], operands);
        } else {
            operands.push(side.to_vec());
        }
    }
}

/// matches an ampersand of syntaxes: `<a> && <b> && <c>`
fn match_ampersand_syntax(arena: &AstArena, syntax: &Syntax, context: &Context, options: &ParserOptions) -> ValidationMatch {
    thread_local! {
        /// the operands are shared by the matches of the syntax (the unordered caches are keyed by their address)
        static OPERANDS: std::cell::RefCell<HashMap<usize, (Syntax, Rc<Vec<Vec<Syntax>>>)>> = std::cell::RefCell::new(HashMap::default());
    }
    let key = Rc::as_ptr(syntax) as usize;
    let operands = OPERANDS.with(|o| {
        o.borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                let mut operands = Vec::new();
                ampersand_operands(syntax, &mut operands);
                (syntax.clone(), Rc::new(operands))
            })
            .1
            .clone()
    });
    match_unordered_syntaxes(arena, syntax, &operands, true, context, options)
}

/// the tokens of a media / container feature: the tokens before the closing parenthesis
pub fn peek_feature_tokens(arena: &AstArena, context: &Context) -> Vec<u32> {
    let ctx = context.borrow();
    let tokens = ctx.tokens();
    let mut result = Vec::new();
    let mut depth = 0i32;
    let mut index = ctx.index + 1;
    while (index as usize) < tokens.len() {
        let token = tokens[index as usize];
        let t = arena.data_types.get(token);
        if t == EnumToken::StartParensTokenType || is_function_def_type(t) {
            depth += 1;
        } else if t == EnumToken::EndParensTokenType {
            if depth == 0 {
                break;
            }
            depth -= 1;
        }
        if t != EnumToken::WhitespaceTokenType && t != EnumToken::CommentTokenType {
            result.push(token);
        }
        index += 1;
    }
    result
}

pub use super::match_property::match_repeatable_syntax;

#[allow(dead_code)]
fn _unused(s: &Syntax) -> bool {
    is_nullable(s)
}
