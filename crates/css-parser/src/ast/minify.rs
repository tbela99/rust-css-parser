//! Port of `src/ast/minify.ts`.
//!
//! Selector parts are strings (`Vec<String>` per selector), lengths are utf-16 lengths like the javascript strings.
//! The typescript minification context (`{ [key]: any }`) is [`MinifyContext`]; the render size cache stored on the
//! options (`options.cache`) lives in the context: both belong to one `minify()` call.

use super::arena::{AstArena, OptimizedSelector, RawSelector, RawSelectorTokens, TokenValue};
use super::features::r#type::FeatureWalkMode;
use super::features::{register_features, FeatureResult, SharedFeature};
use super::mixins::utils::move_result;
use super::strings::StringFlag;
use super::types::{EnumAstNodeStatus, EnumToken};
use super::walk::{ValueWalker, WalkerValueOptions};
use crate::options::ParserOptions;
use crate::parser::parse::parse_string;
use crate::parser::utils::eq::eq_token;
use crate::parser::utils::text::equals_ignore_case;
use crate::printer::render::{render_value_default, NodeRenderer};
use crate::syntax::constants::COMBINATORS;
use crate::syntax::syntax::{is_function, is_ident, is_white_space};
use crate::types::ErrorDescription;
use crate::validation::match_::trim_array;
use std::any::Any;
use std::cell::RefCell;
use crate::utils::fx::{HashMap, HashSet};
use std::rc::Rc;

/// state of a minification (`context`)
#[derive(Default)]
pub struct MinifyContext {
    /// the minified nodes
    pub nodes: HashSet<u32>,
    /// rendered sizes of the nodes (`options.cache`)
    pub cache: Option<HashMap<u32, usize>>,
    /// renderer of the rule sizes
    pub renderer: Option<NodeRenderer>,
    /// state of the features (`context.declarations`, `context.lookup`, ...), by key
    pub data: HashMap<&'static str, Box<dyn Any>>,
}

impl MinifyContext {
    /// a feature state, created on demand (`context.key ??= value`)
    pub fn get_or_insert_with<T: 'static>(&mut self, key: &'static str, f: impl FnOnce() -> T) -> &mut T {
        self.data.entry(key).or_insert_with(|| Box::new(f())).downcast_mut::<T>().expect("context type")
    }

    pub fn get_mut<T: 'static>(&mut self, key: &'static str) -> Option<&mut T> {
        self.data.get_mut(key).and_then(|v| v.downcast_mut::<T>())
    }
}

/// options of a minification: the parser options and the registered features (`MinifyFeatureOptions`)
pub struct MinifyOptions {
    pub options: ParserOptions,
    pub features: Vec<SharedFeature>,
}

/// `["(", "["].concat(combinators)`
fn not_ending_with(c: char) -> bool {
    c == '(' || c == '[' || COMBINATORS.iter().any(|x| x.len() == 1 && x.starts_with(c))
}

fn is_combinator(s: &str) -> bool {
    COMBINATORS.contains(&s)
}

/// utf-16 length of a string (`str.length`)
fn js_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// first character as a string (`str.charAt(0)`)
fn char_at0(s: &str) -> String {
    s.chars().next().map(String::from).unwrap_or_default()
}

/// apply the minification rules to the ast tree
pub fn minify(arena: &AstArena, ast: u32, options: &ParserOptions, recursive: bool, errors: Option<&mut Vec<ErrorDescription>>, nesting_content: bool) -> u32 {
    let mut context = MinifyContext::default();
    minify_with(arena, ast, options, None, recursive, errors, nesting_content, &mut context)
}

/// [`minify`] with registered features and a context
#[allow(clippy::too_many_arguments)]
pub fn minify_with(
    arena: &AstArena,
    ast: u32,
    options: &ParserOptions,
    features: Option<Vec<SharedFeature>>,
    recursive: bool,
    mut errors: Option<&mut Vec<ErrorDescription>>,
    nesting_content: bool,
    context: &mut MinifyContext,
) -> u32 {
    let mut preprocess = false;
    let mut postprocess = false;
    // `let { sourcemap, module, ...options2 } = options`
    let mut options2 = ParserOptions { sourcemap: None, module: None, ..options.clone() };
    let features = match features {
        Some(features) => features,
        None => {
            options2.remove_duplicate_declarations.get_or_insert(crate::options::RemoveDuplicateDeclarations::Bool(true));
            options2.compute_shorthand.get_or_insert(true);
            options2.compute_calc_expression.get_or_insert(true);
            options2.remove_prefix.get_or_insert(false);
            let mut features = Vec::new();
            register_features(&options2, &mut features);
            // stable sort, like the typescript sort
            features.sort_by_key(|f| f.borrow().ordering());
            features
        }
    };
    for feature in &features {
        let mode = feature.borrow().process_mode();
        if mode & FeatureWalkMode::Pre != 0 {
            preprocess = true;
        }
        if mode & FeatureWalkMode::Post != 0 {
            postprocess = true;
        }
    }
    let run_features = |mode: u32, context: &mut MinifyContext| {
        // breadth first traversal
        let mut queue: Vec<u32> = vec![ast];
        let mut k = 0;
        while k < queue.len() {
            let node = queue[k];
            k += 1;
            let typ = arena.data_types.get(node);
            if typ == EnumToken::CommentTokenType || typ == EnumToken::CDOCOMMTokenType {
                continue;
            }
            let mut replacement = FeatureResult::Node(node);
            for feature in &features {
                let mut f = feature.borrow_mut();
                if f.process_mode() & mode == 0 || f.accept().is_some_and(|a| !a.contains(&typ)) {
                    continue;
                }
                let current = match replacement {
                    FeatureResult::Nodes(_) => break,
                    FeatureResult::Node(n) => n,
                    FeatureResult::None => 0,
                };
                let parent = if arena.parents.get(node) == 0 { ast } else { arena.parents.get(node) };
                match f.run(arena, current, &options2, parent, context, mode) {
                    FeatureResult::None => {}
                    result => replacement = result,
                }
            }
            match &replacement {
                FeatureResult::Nodes(nodes) if !nodes.is_empty() && arena.parents.get(node) != 0 => arena.replace_child(node, nodes),
                FeatureResult::Node(n) if *n != node && arena.parents.get(node) != 0 => arena.replace_child(node, &[*n]),
                _ => {}
            }
            if let FeatureResult::Node(n) = replacement {
                if arena.flags.get(n) & EnumAstNodeStatus::HasChildren != 0 {
                    let mut child = arena.first_child.get(n);
                    while child != 0 {
                        queue.push(child);
                        child = arena.next_sibling.get(child);
                    }
                }
            }
        }
        for feature in &features {
            let mut f = feature.borrow_mut();
            if f.process_mode() & mode != 0 && f.has_cleanup() {
                f.cleanup(arena, ast, &options2, context, mode);
            }
        }
    };
    if preprocess {
        run_features(FeatureWalkMode::Pre, context);
    }
    do_minify(arena, ast, &options2, recursive, errors.as_deref_mut(), nesting_content, context);
    if postprocess {
        run_features(FeatureWalkMode::Post, context);
    }
    ast
}

/// `@apply --mixin()` is equivalent to `@apply --mixin` (https://drafts.csswg.org/css-mixins-1/#apply-rule)
fn minify_apply_prelude(arena: &AstArena, node: u32) {
    let t = |x: u32| arena.data_types.get(x);
    let mut reference = arena.first_token.get(node);
    while reference != 0 && matches!(t(reference), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType) {
        reference = arena.next_sibling.get(reference);
    }
    if reference == 0 || t(reference) != EnumToken::CustomFunctionTokenType {
        return;
    }
    // the comments and the whitespace are ignored: `@apply --mixin(/* comment */)` has no argument
    let mut token = arena.first_token.get(reference);
    while token != 0 {
        if !matches!(t(token), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType) {
            return;
        }
        token = arena.next_sibling.get(token);
    }
    let name = arena.resolve_string(arena.string_value.get(reference));
    let token = arena.create_token(EnumToken::DashedIdenTokenType, TokenValue::Str(&name), None);
    arena.set_token_list(node, &[token]);
    arena.string_value.set(node, arena.intern_string(&name));
}

/// minify the prelude of a media rule: the sections after `all` are dropped
fn minify_at_rule_media(arena: &AstArena, tokens: &mut Vec<u32>) {
    let mut parts: Vec<Vec<u32>> = vec![Vec::new()];
    for &token in tokens.iter() {
        if arena.data_types.get(token) == EnumToken::CommaTokenType {
            parts.push(Vec::new());
        } else {
            parts.last_mut().unwrap().push(token);
        }
    }
    let mut keys: Vec<String> = Vec::new();
    for mut values in parts {
        if keys.iter().any(|k| k == "all") {
            break;
        }
        // `transformAtRuleMediaPrelude()`: the values are trimmed, there is no update
        trim_array(arena, &mut values);
        if values.is_empty() {
            continue;
        }
        let key: String = values.iter().map(|&t| render_value_default(arena, t)).collect();
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    if keys.iter().any(|k| k == "all") {
        tokens.clear();
    }
}

/// render the tokens of a prelude without the comments
fn render_prelude(arena: &AstArena, arr: &[u32]) -> String {
    let t = |i: usize| arr.get(i).map_or(0, |&x| arena.data_types.get(x));
    let mut acc = String::new();
    for (index, &curr) in arr.iter().enumerate() {
        let skip = t(index) == EnumToken::CommentTokenType
            || (t(index) == EnumToken::WhitespaceTokenType
                && t(index + 1) == EnumToken::CommentTokenType
                && (index + 3 < arr.len() || t(index + 2) == EnumToken::WhitespaceTokenType));
        if !skip {
            acc.push_str(&render_value_default(arena, curr));
        }
    }
    acc
}

/// reduce selectors: the leading `& ` is removed (the selector is modified in place)
fn reduce(acc: &mut Vec<String>, curr: &mut Vec<String>) {
    if curr.first().map(String::as_str) == Some("&") && curr.get(1).map(String::as_str) == Some(" ") {
        let third = curr.get(2).map(String::as_str).unwrap_or("");
        if !is_ident(third) && !is_function(third) {
            curr.drain(0..2);
        }
    }
    acc.push(curr.concat());
}

/// `selectors.reduce(reducer, []).join(",")`
fn reduce_join(selectors: &mut [Vec<String>]) -> String {
    let mut acc = Vec::new();
    for curr in selectors.iter_mut() {
        reduce(&mut acc, curr);
    }
    acc.join(",")
}

/// true if the selector part is a simple selector that can be moved into `:is()`
fn is_compound_part(part: &str) -> bool {
    let lower = part.to_lowercase();
    !part.trim().is_empty()
        && part != "&"
        && !is_combinator(part)
        && !part.starts_with("::")
        && !matches!(lower.as_str(), ":before" | ":after" | ":first-line" | ":first-letter")
}

/// factor the common prefix of the selectors of a rule into `:is()`. `None` when the selectors cannot be factored
fn factor_selector(optimized: &OptimizedSelector) -> Option<String> {
    let prefix = &optimized.optimized;
    if prefix.is_empty() || !prefix.iter().all(|part| part == " " || is_compound_part(part) || is_combinator(part)) {
        return None;
    }
    let mut combinator: Option<String> = None;
    let mut members: Vec<String> = Vec::new();
    for selector in &optimized.selector {
        let parts: &[String] = if selector.first().map(String::as_str) == Some("&") { &selector[1..] } else { selector };
        // the parts that follow the prefix. the whitespace around the combinators is removed
        let suffix: Vec<String> = parts
            .iter()
            .enumerate()
            .filter(|(index, part)| {
                !part.trim().is_empty()
                    || (!(*index > 0 && is_combinator(&parts[index - 1])) && !parts.get(index + 1).is_some_and(|p| is_combinator(p)))
            })
            .map(|(_, p)| p.clone())
            .collect();
        // the prefix itself
        if suffix.is_empty() {
            if combinator.as_deref().is_some_and(|c| !c.is_empty()) {
                return None;
            }
            combinator = Some(String::new());
            members.push("*".into());
            continue;
        }
        // the combinator that joins the suffix to the prefix. "" for a compound selector
        let current: String = if suffix[0].trim().is_empty() {
            " ".into()
        } else if is_combinator(&suffix[0]) {
            suffix[0].clone()
        } else {
            String::new()
        };
        let compound: &[String] = if current.is_empty() { &suffix } else { &suffix[1..] };
        if combinator.as_ref().is_some_and(|c| *c != current) || current == "|" || compound.is_empty() || !compound.iter().all(|p| is_compound_part(p)) {
            return None;
        }
        combinator = Some(current);
        members.push(compound.concat());
    }
    Some(format!("{}{}:is({})", prefix.concat(), combinator.unwrap_or_default(), members.join(",")))
}

fn optimized_of(arena: &AstArena, node: u32) -> Option<Rc<RefCell<OptimizedSelector>>> {
    arena.optimized.borrow().get(&node).cloned()
}

fn set_selector(arena: &AstArena, node: u32, sel: &str) {
    arena.name.set(node, arena.intern_string(sel));
}

/// the selector of `optimized.selector` joined with the optimized prefix: `sel1` is the `:is()` form
fn reducible_selector(arena: &AstArena, node: u32, optimized: &Rc<RefCell<OptimizedSelector>>, nesting_content: bool) {
    let mut o = optimized.borrow_mut();
    if o.optimized.len() == 1 {
        let prefix = o.optimized[0].clone();
        let sel1 = format!("{}:is({})", prefix, reduce_join(&mut o.selector));
        let sel2 = o.selector.iter().map(|curr| format!("{}{}", prefix, curr.concat())).collect::<Vec<_>>().join(",");
        set_selector(arena, node, if js_len(&sel1) < js_len(&sel2) { &sel1 } else { &sel2 });
        arena.set_token_list(node, &[]);
    } else if o.optimized.is_empty() {
        // `/^[a-zA-Z]/.test(curr[0])`: `undefined` is tested as "undefined"
        let test_ident = |s: Option<&String>| s.map_or(true, |s| s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()));
        let sel = o
            .selector
            .iter()
            .map(|curr| format!("{}{}", if nesting_content && test_ident(curr.first()) { "& " } else { "" }, curr.concat()))
            .collect::<Vec<_>>()
            .join(",");
        set_selector(arena, node, &sel);
        arena.set_token_list(node, &[]);
    }
}

/// apply the minification rules to the ast tree
#[allow(clippy::too_many_arguments)]
fn do_minify(
    arena: &AstArena,
    ast: u32,
    options: &ParserOptions,
    recursive: bool,
    mut errors: Option<&mut Vec<ErrorDescription>>,
    nesting_content: bool,
    context: &mut MinifyContext,
) -> u32 {
    if context.nodes.contains(&ast) {
        return ast;
    }
    context.nodes.insert(ast);
    if arena.flags.get(ast) & EnumAstNodeStatus::HasChildren == 0 || arena.first_child.get(ast) == 0 {
        return ast;
    }
    let t = |x: u32| arena.data_types.get(x);
    let nesting_content = nesting_content || (options.nesting_rules == Some(true) && t(ast) == EnumToken::RuleNodeType);
    let opt = |x: u32| if x == 0 { None } else { Some(x) };
    // the children list is modified while it is visited: every branch sets the next child to visit
    let mut cursor = arena.first_child.get(ast);
    let mut previous: Option<u32> = None;
    let mut node: Option<u32> = None;
    // the node located `count` positions before a child of ast (the end of the list when 0)
    let before = |child: u32, count: i32| -> Option<u32> {
        let mut result = if child == 0 { arena.last_child.get(ast) } else { arena.previous_sibling.get(child) };
        let mut count = count;
        loop {
            count -= 1;
            if !(count > 0 && result != 0) {
                break;
            }
            result = arena.previous_sibling.get(result);
        }
        opt(result)
    };
    macro_rules! recurse {
        ($n:expr) => {
            do_minify(arena, $n, options, recursive, errors.as_deref_mut(), nesting_content, context)
        };
    }
    while cursor != 0 {
        if t(cursor) == EnumToken::CommentNodeType {
            cursor = arena.next_sibling.get(cursor);
            continue;
        }
        while let Some(p) = previous {
            if t(p) != EnumToken::CommentNodeType {
                break;
            }
            previous = opt(arena.previous_sibling.get(p));
        }
        let mut n = cursor;
        node = Some(n);
        let mut node_type = t(n);
        let mut previous_type = previous.map(t);
        if node_type == EnumToken::AtRuleNodeType {
            let nam = arena.resolve_string(arena.name.get(n));
            if &*nam == "font-face" {
                cursor = arena.next_sibling.get(n);
                continue;
            }
            // there is no early return: the result that precedes the conditional rules is moved after them
            if equals_ignore_case(&nam, "function") {
                move_result(arena, n);
            }
        }
        if node_type == EnumToken::KeyframesAtRuleNodeType {
            if let Some(p) = previous {
                if previous_type == Some(EnumToken::KeyframesAtRuleNodeType)
                    && arena.name.get(n) == arena.name.get(p)
                    && arena.string_value.get(n) == arena.string_value.get(p)
                {
                    // the previous keyframes are dropped, the node is visited again
                    let previous_node = arena.previous_sibling.get(p);
                    cursor = arena.next_sibling.get(p);
                    arena.detach(p);
                    previous = opt(previous_node);
                    continue;
                }
            }
        } else if node_type == EnumToken::KeyframesRuleNodeType {
            if let Some(p) = previous {
                if previous_type == Some(EnumToken::KeyframesRuleNodeType) && arena.name.get(n) == arena.name.get(p) {
                    // https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/At-rules/@keyframes#resolving_duplicates
                    arena.move_children(n, p, 0);
                    if let Some(cache) = context.cache.as_mut() {
                        cache.remove(&p);
                    }
                    arena.detach(n);
                    cursor = arena.next_sibling.get(p);
                    continue;
                }
            }
            let mut k = arena.first_child.get(n);
            while k != 0 {
                let next = arena.next_sibling.get(k);
                if t(k) == EnumToken::DeclarationNodeType {
                    let mut l = arena.last_token.get(k);
                    while l != 0 {
                        if t(l) == EnumToken::ImportantTokenType {
                            arena.detach(k);
                            break;
                        }
                        if matches!(t(l), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType) {
                            l = arena.previous_sibling.get(l);
                            continue;
                        }
                        break;
                    }
                }
                k = next;
            }
        } else if node_type == EnumToken::AtRuleNodeType {
            let nam = arena.resolve_string(arena.name.get(n));
            let tokens: Option<Vec<u32>> = if arena.first_token.get(n) == 0 { None } else { Some(arena.tokens(n)) };
            if nam == "apply" {
                minify_apply_prelude(arena, n);
            } else if nam == "media" {
                if let Some(tokens) = &tokens {
                    let mut slice = tokens.clone();
                    minify_at_rule_media(arena, &mut slice);
                    if slice.len() != tokens.len() {
                        arena.set_token_list(n, &slice);
                        arena.string_value.set(n, arena.intern_string(&render_prelude(arena, &slice)));
                    }
                }
                let value = arena.resolve_string(arena.string_value.get(n));
                if &*value == "all" || value.is_empty() {
                    // the node is replaced by its children
                    cursor = match arena.first_child.get(n) {
                        0 => arena.next_sibling.get(n),
                        c => c,
                    };
                    arena.move_children(n, ast, n);
                    arena.detach(n);
                    continue;
                }
            } else if nam == "import" && tokens.is_some() {
                let mut tokens = tokens.unwrap();
                let l = tokens
                    .iter()
                    .position(|&token| {
                        matches!(t(token), EnumToken::ParensTokenType | EnumToken::MediaQueryConditionTokenType)
                            || (t(token) == EnumToken::IdenTokenType && !arena.string_equals(arena.string_value.get(token), "layer"))
                    })
                    .unwrap_or(tokens.len());
                if l < tokens.len() {
                    let mut slice: Vec<u32> = tokens[l..].to_vec();
                    minify_at_rule_media(arena, &mut slice);
                    tokens.splice(l.., slice);
                    arena.set_token_list(n, &tokens);
                    trim_array(arena, &mut tokens);
                    arena.string_value.set(n, arena.intern_string(&render_prelude(arena, &tokens)));
                }
            } else if t(ast) == node_type && arena.name.get(ast) == arena.name.get(n) && arena.string_value.get(ast) == arena.string_value.get(n) {
                // the node is replaced by its children
                cursor = match arena.first_child.get(n) {
                    0 => arena.next_sibling.get(n),
                    c => c,
                };
                arena.move_children(n, ast, n);
                arena.detach(n);
                continue;
            }
            if let Some(p) = previous {
                if previous_type == Some(EnumToken::AtRuleNodeType)
                    && nam != "font-face"
                    && arena.name.get(p) == arena.name.get(n)
                    && arena.string_value.get(p) == arena.string_value.get(n)
                {
                    if arena.flags.get(n) & EnumAstNodeStatus::HasChildren != 0 {
                        arena.move_children(n, p, 0);
                        if let Some(cache) = context.cache.as_mut() {
                            cache.remove(&p);
                        }
                        if !has_declaration(arena, p) {
                            context.nodes.remove(&p);
                            recurse!(p);
                        }
                    }
                    cursor = arena.next_sibling.get(n);
                    arena.detach(n);
                    continue;
                }
            }
            if arena.flags.get(n) & EnumAstNodeStatus::HasChildren != 0 {
                recurse!(n);
            }
            previous = Some(n);
            cursor = arena.next_sibling.get(n);
            continue;
        } else if node_type == EnumToken::RuleNodeType {
            // a single selector that does not end with a combinator is not reduced (see is_simple_selector())
            if !is_simple_selector(arena, arena.name.get(n)) {
                reduce_rule_selector(arena, n);
            }
            let mut wrapper: Option<u32> = None;
            if options.nesting_rules == Some(true) {
                // the rules that apply a mixin are not merged
                if let Some(p) = previous {
                    if previous_type == Some(EnumToken::RuleNodeType) && !has_at_rule_statement(arena, p) && !has_at_rule_statement(arena, n) {
                        if !is_simple_selector(arena, arena.name.get(p)) {
                            reduce_rule_selector(arena, p);
                        }
                        // the selectors are split when their first parts are equal: they do not match otherwise
                        if may_match_selectors(arena, p, n) {
                            if let Some(m) = match_selectors_of(arena, p, n) {
                                wrapper = Some(wrap_nodes(arena, p, n, m));
                            }
                        }
                    }
                }
                let optimized = optimized_of(arena, n);
                if let Some(mut w) = wrapper {
                    // wrap the following rules that match the wrapper selector
                    let mut next_node = arena.next_sibling.get(w);
                    while next_node != 0 && t(next_node) == EnumToken::RuleNodeType && !has_at_rule_statement(arena, next_node) {
                        if !is_simple_selector(arena, arena.name.get(next_node)) {
                            reduce_rule_selector(arena, next_node);
                        }
                        // the selector is split when its first part matches the wrapper selector
                        let m = if may_match_selectors(arena, w, next_node) { match_selectors_of(arena, w, next_node) } else { None };
                        let Some(m) = m else {
                            break;
                        };
                        w = wrap_nodes(arena, w, next_node, m);
                        next_node = arena.next_sibling.get(w);
                    }
                    previous = Some(w);
                    cursor = arena.next_sibling.get(w);
                    recurse!(w);
                    continue;
                } else if let Some(optimized) = optimized.as_ref().filter(|o| o.borrow().match_ && o.borrow().selector.len() > 1) {
                    let w = arena.clone_node(n);
                    arena.optimized.borrow_mut().insert(w, optimized.clone());
                    let first = optimized.borrow().optimized[0].clone();
                    set_selector(arena, w, &first);
                    arena.raw.borrow_mut().insert(w, Rc::<RawSelector>::new(RawSelector::from(vec![vec![first]])));
                    arena.set_token_list(w, &[]);
                    let (sel, raw) = {
                        let mut o = optimized.borrow_mut();
                        let sel = reduce_join(&mut o.selector);
                        (sel, o.selector.clone())
                    };
                    set_selector(arena, n, &sel);
                    arena.raw.borrow_mut().insert(n, Rc::<RawSelector>::new(RawSelector::from(raw)));
                    arena.set_token_list(n, &[]);
                    // the wrapper takes the place of the node
                    arena.insert_before(n, &[w]);
                    arena.append_child(w, n);
                    n = w;
                    node = Some(n);
                    node_type = EnumToken::RuleNodeType;
                } else if let Some(optimized) = optimized.as_ref().filter(|o| o.borrow().reducible) {
                    reducible_selector(arena, n, optimized, nesting_content);
                }
            } else {
                let optimized = optimized_of(arena, n);
                match optimized {
                    Some(o) if o.borrow().match_ => {
                        let sel = factor_selector(&o.borrow());
                        if let Some(sel) = sel {
                            if js_len(&sel) < arena.string_length(arena.name.get(n)) {
                                set_selector(arena, n, &sel);
                                arena.set_token_list(n, &[]);
                            }
                        }
                    }
                    Some(o) if o.borrow().reducible => reducible_selector(arena, n, &o, nesting_content),
                    Some(o) if !o.borrow().optimized.is_empty() => {
                        let sel = o.borrow().optimized.concat();
                        if js_len(&sel) < arena.string_length(arena.name.get(n)) {
                            set_selector(arena, n, &sel);
                            arena.raw.borrow_mut().insert(n, Rc::<RawSelector>::new(RawSelector::from(vec![o.borrow().optimized.clone()])));
                            arena.set_token_list(n, &[]);
                        }
                    }
                    _ => {}
                }
            }
            recurse!(n);
        }
        if let Some(p) = previous {
            previous_type = Some(t(p));
            if arena.flags.get(p) & EnumAstNodeStatus::HasChildren != 0 && arena.flags.get(n) & EnumAstNodeStatus::HasChildren != 0 {
                if previous_type == Some(node_type) {
                    let mut should_merge = true;
                    let mut k = arena.last_child.get(p);
                    while k != 0 {
                        if t(k) == EnumToken::CommentNodeType {
                            k = arena.previous_sibling.get(k);
                            continue;
                        }
                        should_merge = t(k) == EnumToken::DeclarationNodeType;
                        break;
                    }
                    // the rules that apply a mixin are not merged
                    if should_merge && (node_type != EnumToken::RuleNodeType || (!has_at_rule_statement(arena, p) && !has_at_rule_statement(arena, n))) {
                        if ((node_type == EnumToken::RuleNodeType || node_type == EnumToken::KeyframesRuleNodeType) && arena.name.get(n) == arena.name.get(p))
                            || (node_type == EnumToken::AtRuleNodeType
                                && !arena.string_equals(arena.name.get(n), "font-face")
                                && arena.name.get(n) == arena.name.get(p))
                        {
                            arena.move_children(p, n, arena.first_child.get(n));
                            if let Some(cache) = context.cache.as_mut() {
                                cache.remove(&n);
                            }
                            recurse!(n);
                            arena.detach(p);
                            previous = Some(n);
                            cursor = arena.next_sibling.get(n);
                            continue;
                        } else if matches!(node_type, EnumToken::KeyframesRuleNodeType | EnumToken::RuleNodeType) {
                            if let Some((result, node1, node2)) = diff(arena, p, n, options, context) {
                                // [result, node2] take the place of node1 (previous), node1 takes the place of node2 (node)
                                let after = arena.next_sibling.get(node2);
                                let keep2 = arena.first_child.get(node2) != 0;
                                if let Some(result) = result {
                                    arena.insert_before(node1, &[result]);
                                }
                                if keep2 {
                                    arena.insert_before(node1, &[node2]);
                                } else {
                                    arena.detach(node2);
                                }
                                if arena.first_child.get(node1) == 0 {
                                    arena.detach(node1);
                                } else if after == 0 {
                                    arena.append_child(ast, node1);
                                } else {
                                    arena.insert_before(after, &[node1]);
                                }
                                // next node to visit
                                node = if keep2 {
                                    opt(arena.next_sibling.get(result.unwrap_or(node2)))
                                } else {
                                    before(if arena.parents.get(node1) == ast { node1 } else { after }, if result.is_none() { 1 } else { 2 })
                                };
                                previous = result;
                            }
                        }
                    }
                }
                if recursive {
                    if let Some(p) = previous {
                        if Some(p) != node && !has_declaration(arena, p) {
                            recurse!(p);
                        }
                    }
                }
            }
        }
        if !nesting_content {
            if let Some(p) = previous {
                if t(p) == EnumToken::RuleNodeType && arena.string_index_of(arena.name.get(p), 0x26) != -1 {
                    fix_selector(arena, p);
                }
            }
        }
        previous = node;
        cursor = match node {
            None => arena.first_child.get(ast),
            Some(n) => arena.next_sibling.get(n),
        };
    }
    if let Some(n) = node {
        if recursive
            && arena.flags.get(n) & EnumAstNodeStatus::HasChildren != 0
            && (t(n) == EnumToken::KeyframesAtRuleNodeType || !has_declaration_child(arena, n))
            && !(t(n) == EnumToken::AtRuleNodeType && !arena.string_equals(arena.name.get(n), "font-face"))
        {
            recurse!(n);
        }
        if !nesting_content && t(n) == EnumToken::RuleNodeType && arena.string_index_of(arena.name.get(n), 0x26) != -1 {
            fix_selector(arena, n);
        }
    }
    ast
}

/// true if the first non comment child is a declaration
fn has_declaration(arena: &AstArena, node: u32) -> bool {
    let mut child = arena.first_child.get(node);
    while child != 0 {
        if arena.data_types.get(child) != EnumToken::CommentNodeType {
            return arena.data_types.get(child) == EnumToken::DeclarationNodeType;
        }
        child = arena.next_sibling.get(child);
    }
    true
}

/// true if the node has a declaration child
fn has_declaration_child(arena: &AstArena, node: u32) -> bool {
    let mut child = arena.first_child.get(node);
    while child != 0 {
        if arena.data_types.get(child) == EnumToken::DeclarationNodeType {
            return true;
        }
        child = arena.next_sibling.get(child);
    }
    false
}

/// optimize a selector
pub fn optimize_selector(selector: &[Vec<String>]) -> Option<OptimizedSelector> {
    if selector.is_empty() {
        return None;
    }
    if selector.len() == 1 {
        let current = &selector[0];
        if current.last().is_none_or(|last| !last.starts_with(":is(")) {
            let mut end = current.len();
            while end > 0 && (current[end - 1] == " " || is_combinator(&current[end - 1])) {
                end -= 1;
            }
            let mut optimized: Vec<String> = current[..end].to_vec();
            if optimized.first().map(String::as_str) == Some("&") && optimized.get(1).map(String::as_str) == Some(" ") {
                optimized.drain(0..2);
            }
            let remainder: Vec<String> = current[end..].to_vec();
            let remainder = if remainder.first().map(String::as_str) == Some("&") && remainder.get(1).map(String::as_str) == Some(" ") {
                remainder[2..].to_vec()
            } else {
                remainder
            };
            return Some(OptimizedSelector { match_: false, optimized, selector: vec![remainder], reducible: false });
        }
    }
    let mut expanded: Vec<Vec<String>> = Vec::new();
    for curr in selector {
        if curr.last().is_some_and(|l| l.starts_with(":is(")) {
            let last = curr.last().unwrap();
            let inner: Vec<u16> = last.encode_utf16().collect();
            let inner = String::from_utf16_lossy(&inner[4..inner.len().saturating_sub(1).max(4)]);
            let rules = split_rule(&inner).into_iter().map(|x| {
                if x.first().map(String::as_str) == Some("&") && x.len() > 1 {
                    x[if x[1] == " " { 2 } else { 1 }..].to_vec()
                } else {
                    x
                }
            });
            let part = &curr[..curr.len() - 1];
            for rule in rules {
                let mut v = part.to_vec();
                v.extend(rule);
                expanded.push(v);
            }
            continue;
        }
        expanded.push(curr.clone());
    }
    let mut seen: HashSet<String> = HashSet::default();
    let mut selector: Vec<Vec<String>> = expanded.into_iter().filter(|x| seen.insert(x.concat())).collect();
    let k = selector.iter().fold(0usize, |acc, curr| if acc == 0 { curr.len() } else if curr.is_empty() { acc } else { acc.min(curr.len()) });
    let mut optimized: Vec<String> = Vec::new();
    for i in 0..k {
        let item = selector[0].get(i);
        if !(1..selector.len()).all(|j| selector[j].get(i) == item) {
            break;
        }
        optimized.push(item.cloned().unwrap_or_default());
    }
    while let Some(last) = optimized.last() {
        if last == " " || is_combinator(last) {
            optimized.pop();
            continue;
        }
        break;
    }
    for s in selector.iter_mut() {
        let n = optimized.len().min(s.len());
        s.drain(0..n);
    }
    if optimized.first().map(String::as_str) == Some("&") && optimized.get(1).map(String::as_str) == Some(" ") {
        optimized.drain(0..2);
    }
    if optimized.is_empty() || optimized[0].starts_with('&') || selector.len() == 1 {
        let reducible = selector.len() > 1 && selector.iter().all(|s| s.first().is_none_or(|f| !is_combinator(f)));
        let selector = selector
            .into_iter()
            .map(|mut s| {
                // the leading `& ` is removed in place
                if s.first().map(String::as_str) == Some("&") && s.get(1).map(String::as_str) == Some(" ") {
                    s.drain(0..2);
                }
                s
            })
            .collect();
        return Some(OptimizedSelector { match_: false, optimized, selector, reducible });
    }
    let mut result: Vec<Vec<String>> = Vec::with_capacity(selector.len());
    let mut reducible = true;
    // the selectors are moved into the result
    for mut curr in selector {
        let mut has_compound = true;
        if !curr.is_empty() {
            let c0 = char_at0(&curr[0]);
            has_compound = c0 != "&" && !is_combinator(&c0);
        }
        if has_compound && curr.first().map(String::as_str) == Some(" ") {
            has_compound = false;
            curr.insert(0, "&".into());
        }
        if curr.is_empty() {
            curr.push("&".into());
            has_compound = false;
        }
        reducible &= curr.first().is_none_or(|f| !matches!(f.as_str(), ">" | "+" | "~" | "&"));
        if has_compound {
            curr.insert(0, "&".to_string());
        }
        result.push(curr);
    }
    Some(OptimizedSelector { match_: true, optimized, selector: result, reducible })
}

/// split a selector string into selectors of parts
pub fn split_rule(buffer: &str) -> Vec<Vec<String>> {
    // the delimiters are ascii: an ascii selector is split on its bytes (the byte offsets are the utf-16 offsets), the
    // other selectors on their utf-16 code units
    if buffer.is_ascii() {
        let bytes = buffer.as_bytes();
        return split_units(bytes, |a, b| buffer[a..b.min(bytes.len())].to_owned());
    }
    let units: Vec<u16> = buffer.encode_utf16().collect();
    let length = units.len();
    split_units(&units, |a, b| String::from_utf16_lossy(&units[a..b.min(length)]))
}

/// split a selector given as code units (bytes of an ascii selector or utf-16 code units)
fn split_units<U: Copy + Into<u32>>(units: &[U], slice: impl Fn(usize, usize) -> String) -> Vec<Vec<String>> {
    let length = units.len();
    let code_at = |i: usize| units.get(i).map_or(u32::MAX, |&c| c.into());
    let mut result: Vec<Vec<String>> = vec![Vec::new()];
    let mut start: Option<usize> = None;
    let mut i = 0usize;
    while i < length {
        let code: u32 = units[i].into();
        let group = result.last_mut().unwrap();
        if is_white_space(code) {
            if let Some(s) = start.take() {
                group.push(slice(s, i));
            }
            if !group.is_empty() {
                group.push(" ".into());
            }
            i += 1;
            continue;
        }
        if code == 0x2c {
            if let Some(s) = start.take() {
                group.push(slice(s, i));
            }
            result.push(Vec::new());
            i += 1;
            continue;
        }
        if code == 0x2e {
            if let Some(s) = start {
                group.push(slice(s, i));
            }
            start = Some(i);
            i += 1;
            continue;
        }
        // the combinators: + > ~ | ||
        if matches!(code, 0x2b | 0x3e | 0x7e | 0x7c) {
            if let Some(s) = start.take() {
                group.push(slice(s, i));
            }
            if code == 0x7c && code_at(i + 1) == 0x7c {
                group.push("||".into());
                i += 1;
            } else {
                group.push(match code {
                    0x2b => "+",
                    0x3e => ">",
                    0x7e => "~",
                    _ => "|",
                }
                .into());
            }
            i += 1;
            continue;
        }
        // : ::
        if code == 0x3a {
            if let Some(s) = start {
                group.push(slice(s, i));
            }
            start = Some(i);
            if code_at(i + 1) == 0x3a {
                i += 1;
            }
            i += 1;
            continue;
        }
        if start.is_none() {
            start = Some(i);
        }
        // \: the escaped character
        if code == 0x5c {
            i += 2;
            continue;
        }
        // ( [: the part ends with the matching ) ]
        if code == 0x28 || code == 0x5b {
            i = matching_close(units, i, length);
        }
        i += 1;
    }
    if let Some(s) = start {
        result.last_mut().unwrap().push(slice(s, length));
    }
    result
}

/// index of the `)` / `]` that closes the `(` / `[` at `i` (`length` when it is not closed)
fn matching_close<U: Copy + Into<u32>>(codes: &[U], i: usize, length: usize) -> usize {
    let code: u32 = codes[i].into();
    let close = if code == 0x28 { 0x29 } else { 0x5d };
    let mut depth = 1;
    let mut k = i;
    loop {
        k += 1;
        if k >= length {
            break;
        }
        let c: u32 = codes[k].into();
        if c == 0x5c {
            k += 1;
            continue;
        }
        if c == code {
            depth += 1;
        } else if c == close {
            depth -= 1;
        }
        if depth == 0 {
            break;
        }
    }
    k
}

/// reduce a selector relative to the matched prefix (`reduceSelector.bind({ match })`)
fn reduce_selector(match_: &[Vec<String>], curr: &[String]) -> Vec<String> {
    let mut has_compound_selector = true;
    let mut curr: Vec<String> = curr.get(match_[0].len().min(curr.len())..).unwrap_or(&[]).to_vec();
    while curr.first().map(String::as_str) == Some(" ") {
        has_compound_selector = false;
        curr.insert(0, "&".into());
        // `continue`: the first part is now "&"
        break;
    }
    if has_compound_selector && !curr.is_empty() {
        let c0 = char_at0(&curr[0]);
        has_compound_selector = c0 != "&" && !is_combinator(&c0);
    }
    if curr.first().map(String::as_str) == Some(":is(") {
        let mut can_reduce = true;
        let mut is_compound: Vec<Vec<String>> = vec![Vec::new()];
        for (index, token) in curr.iter().enumerate() {
            if index == 0 {
                can_reduce = curr.get(1).map(String::as_str) == Some("&");
            } else if token == ")" {
            } else if token == "," {
                if !can_reduce {
                    can_reduce = curr.get(index + 1).map(String::as_str) == Some("&");
                }
                is_compound.push(Vec::new());
            } else {
                is_compound.last_mut().unwrap().push(token.clone());
            }
        }
        if can_reduce {
            let mut acc: Vec<String> = Vec::new();
            for c in is_compound {
                if !acc.is_empty() {
                    acc.push(",".into());
                }
                acc.extend(c);
            }
            curr = acc;
        }
    }
    if match_.is_empty() {
        return vec!["&".into()];
    }
    if has_compound_selector && curr.first().map(String::as_str) != Some("&") && curr.first().is_none_or(|c| !is_combinator(&char_at0(c))) {
        let mut v = vec!["&".to_string()];
        v.extend(curr);
        return v;
    }
    curr
}

/// matched selectors (`MatchedSelector`)
pub struct MatchedSelector {
    pub eq: bool,
    pub match_: Vec<Vec<String>>,
    pub selector1: Vec<Vec<String>>,
    pub selector2: Vec<Vec<String>>,
}

/// `/^:is\(([:.][^\s,]+)\)$/`
fn is_single_is(token: &str) -> Option<String> {
    let inner = token.strip_prefix(":is(")?.strip_suffix(')')?;
    let mut chars = inner.chars();
    let first = chars.next()?;
    if first != ':' && first != '.' {
        return None;
    }
    let rest: Vec<char> = chars.collect();
    if rest.is_empty() || rest.iter().any(|c| c.is_whitespace() || *c == ',' || *c == '\u{feff}') {
        return None;
    }
    Some(inner.to_string())
}

/// the `:is(.a)` parts of the selectors that start with `&` are replaced by `.a` (in place)
/// true if a selector may contain a single `:is()` to unwrap (see unwrap_single_is()): the selector is not copied
/// otherwise
fn needs_unwrap(selector: &[Vec<String>]) -> bool {
    selector.iter().any(|s| s.first().map(String::as_str) == Some("&") && s.iter().skip(1).any(|t| t.starts_with(":is(")))
}

/// returns true when the selector is modified
fn unwrap_single_is(selector: &mut [Vec<String>]) -> bool {
    let mut modified = false;
    for s in selector.iter_mut() {
        if s.first().map(String::as_str) != Some("&") {
            continue;
        }
        for token in s.iter_mut().skip(1) {
            if token.starts_with(":is(") {
                if let Some(m) = is_single_is(token) {
                    *token = m;
                    modified = true;
                }
            }
        }
    }
    modified
}

/// match the selectors of two rules: their raw selectors are modified in place (`:is(.a)` -> `.a`)
fn match_selectors_of(arena: &AstArena, node1: u32, node2: u32) -> Option<MatchedSelector> {
    let mut s1 = raw_selector(arena, node1);
    let mut s2 = raw_selector(arena, node2);
    // raw_selector() cached the selectors (shared): a selector is copied and stored again only when it is modified
    if needs_unwrap(&s1) && unwrap_single_is(Rc::make_mut(&mut s1).as_mut_slice()) {
        arena.raw.borrow_mut().insert(node1, s1.clone());
    }
    if needs_unwrap(&s2) && unwrap_single_is(Rc::make_mut(&mut s2).as_mut_slice()) && node1 != node2 {
        arena.raw.borrow_mut().insert(node2, s2.clone());
    }
    match_selectors(&s1, &s2)
}

/// match selectors
fn match_selectors(selector1: &[Vec<String>], selector2: &[Vec<String>]) -> Option<MatchedSelector> {
    let min_len = |s: &[Vec<String>]| s.iter().fold(s.first().map_or(0, |f| f.len()), |acc, curr| acc.min(curr.len()));
    let j = min_len(&selector1).min(min_len(&selector2));
    let mut match_: Vec<Vec<String>> = vec![Vec::new()];
    let mut match_function = 0;
    for i in 0..j {
        let token = &selector1[0][i];
        if !selector1.iter().all(|s| s.get(i) == Some(token)) || !selector2.iter().all(|s| s.get(i) == Some(token)) {
            break;
        }
        if token.ends_with('(') {
            match_function += 1;
        }
        match_.last_mut().unwrap().push(token.clone());
    }
    // invalid function
    if match_function != 0 {
        return None;
    }
    for part in match_.iter_mut() {
        while let Some(token) = part.last() {
            if token == " " || is_combinator(token) || token.chars().last().is_some_and(not_ending_with) {
                part.pop();
                continue;
            }
            break;
        }
    }
    if match_.iter().all(|t| t.is_empty()) {
        return None;
    }
    if match_ == vec![vec!["&".to_string()]] {
        return None;
    }
    let selector1: Vec<Vec<String>> = selector1.iter().map(|curr| reduce_selector(&match_, curr)).collect();
    let selector2: Vec<Vec<String>> = selector2.iter().map(|curr| reduce_selector(&match_, curr)).collect();
    Some(MatchedSelector { eq: selector1 == selector2, match_, selector1, selector2 })
}

/// the nesting selectors of the `:is()` functions of a rule selector are removed
fn fix_selector(arena: &AstArena, node: u32) {
    let sel = arena.resolve_string(arena.name.get(node));
    if !sel.contains('&') {
        return;
    }
    let attributes = parse_string(arena, &sel);
    let mut walker = ValueWalker::of_tokens(&attributes, WalkerValueOptions::default(), false);
    while let Some(attr) = walker.next(arena) {
        if arena.data_types.get(attr) == EnumToken::PseudoClassFuncTokenType && arena.string_equals(arena.string_value.get(attr), ":is") {
            let mut child = arena.last_token.get(attr);
            while child != 0 {
                let previous = arena.previous_sibling.get(child);
                if arena.data_types.get(child) == EnumToken::NestingSelectorTokenType {
                    arena.detach(child);
                }
                child = previous;
            }
        }
    }
    let rendered: String = attributes.iter().map(|&a| render_value_default(arena, a)).collect();
    set_selector(arena, node, &rendered);
    arena.set_token_list(node, &[]);
}

/// wrap two sibling rules into a new rule. the wrapper takes the place of node, previous is removed
fn wrap_nodes(arena: &AstArena, previous: u32, node: u32, mut m: MatchedSelector) -> u32 {
    let p_sel = reduce_join(&mut m.selector1);
    let n_sel = reduce_join(&mut m.selector2);
    let wrapper = arena.clone_node(previous);
    let mut match_copy = m.match_.clone();
    set_selector(arena, wrapper, &reduce_join(&mut match_copy));
    // the matched selector is moved: reduce_join() modified its copy
    arena.raw.borrow_mut().insert(wrapper, Rc::<RawSelector>::new(RawSelector::from(m.match_)));
    // not copied by cloneNode()
    arena.optimized.borrow_mut().remove(&wrapper);
    arena.set_token_list(wrapper, &[]);
    let parent = arena.parents.get(node);
    // the wrapper takes the place of node
    arena.insert_before(node, &[wrapper]);
    if p_sel == "&" || p_sel.is_empty() {
        arena.move_children(previous, wrapper, 0);
        if n_sel == "&" || n_sel.is_empty() {
            arena.move_children(node, wrapper, 0);
        } else {
            arena.append_child(wrapper, node);
        }
    } else {
        arena.append_child(wrapper, previous);
        arena.append_child(wrapper, node);
    }
    // the nodes whose children were moved into the wrapper are dropped
    if arena.parents.get(previous) == parent {
        arena.detach(previous);
    }
    if arena.parents.get(node) == parent {
        arena.detach(node);
    }
    // update the selectors of the wrapped nodes
    if arena.parents.get(previous) == wrapper {
        set_selector(arena, previous, &p_sel);
        arena.raw.borrow_mut().insert(previous, Rc::<RawSelector>::new(RawSelector::from(m.selector1)));
        arena.set_token_list(previous, &[]);
    }
    if arena.parents.get(node) == wrapper {
        set_selector(arena, node, &n_sel);
        arena.raw.borrow_mut().insert(node, Rc::<RawSelector>::new(RawSelector::from(m.selector2)));
        arena.set_token_list(node, &[]);
    }
    reduce_rule_selector(arena, wrapper);
    arena.set_token_list(wrapper, &[]);
    wrapper
}

/// true if a child of the node is an at-rule without a block or an `@apply` rule
fn has_at_rule_statement(arena: &AstArena, node: u32) -> bool {
    let mut child = arena.first_child.get(node);
    while child != 0 {
        if arena.data_types.get(child) == EnumToken::AtRuleNodeType
            && (arena.flags.get(child) & EnumAstNodeStatus::HasChildren == 0 || arena.string_equals(arena.name.get(child), "apply"))
        {
            return true;
        }
        child = arena.next_sibling.get(child);
    }
    false
}

/// the first vendor prefix of a part of a selector: `::?-([a-z]+)-`
fn part_prefix(codes: &[u16], start: usize, end: usize) -> Option<String> {
    for p in start..end {
        if codes[p] != 0x3a {
            continue;
        }
        let mut q = p + 1;
        if q < end && codes[q] == 0x3a {
            q += 1;
        }
        if q >= end || codes[q] != 0x2d {
            continue;
        }
        let mut k = q + 1;
        while k < end && (0x61..=0x7a).contains(&codes[k]) {
            k += 1;
        }
        if k > q + 1 && k < end && codes[k] == 0x2d {
            return Some(String::from_utf16_lossy(&codes[q + 1..k]));
        }
    }
    None
}

/// `t.match(/::?-([a-z]+)-/)?.[1]`
fn string_prefix(t: &str) -> Option<String> {
    let codes: Vec<u16> = t.encode_utf16().collect();
    part_prefix(&codes, 0, codes.len())
}

/// the vendor prefixes of the pseudo classes of a split selector
fn raw_selector_prefixes(raw: &RawSelectorTokens) -> Vec<String> {
    let mut prefixes: Vec<String> = Vec::new();
    'outer: for token in raw {
        for t in token {
            if t.contains(':') {
                let Some(prefix) = string_prefix(t) else {
                    continue;
                };
                if !prefixes.contains(&prefix) {
                    prefixes.push(prefix);
                }
                if prefixes.len() > 1 {
                    break 'outer;
                }
            }
        }
    }
    prefixes
}

/// code units of an interned string
fn read_selector(arena: &AstArena, id: u32) -> Vec<u16> {
    let mut codes = Vec::with_capacity(arena.string_length(id));
    arena.copy_string(id, &mut codes);
    codes
}

/// the vendor prefixes of the pseudo classes of a selector read from the string table
/// the vendor prefixes of a selector string, cached by string id (the string epoch invalidates the cache)
fn selector_prefixes_of(arena: &AstArena, id: u32) -> Rc<[String]> {
    let epoch = arena.string_epoch();
    {
        let cache = arena.selector_prefixes.borrow();
        if cache.0 == epoch {
            if let Some(prefixes) = cache.1.get(&id) {
                return prefixes.clone();
            }
        }
    }
    let prefixes: Rc<[String]> = selector_prefixes(arena, id).into();
    let mut cache = arena.selector_prefixes.borrow_mut();
    if cache.0 != epoch {
        cache.1.clear();
        cache.0 = epoch;
    }
    cache.1.insert(id, prefixes.clone());
    prefixes
}

fn selector_prefixes(arena: &AstArena, id: u32) -> Vec<String> {
    // the selectors without `:-` are skipped: computed once per string id
    let has_prefix = arena.string_flag_id(id, StringFlag::VendorPrefix, |id| read_selector(arena, id).windows(2).any(|w| w[0] == 0x3a && w[1] == 0x2d));
    let mut prefixes: Vec<String> = Vec::new();
    if !has_prefix {
        return prefixes;
    }
    let codes = read_selector(arena, id);
    let length = codes.len();
    let mut start: Option<usize> = None;
    let add_part = |start: usize, end: usize, prefixes: &mut Vec<String>| -> bool {
        if codes[start..end].contains(&0x3a) {
            if let Some(prefix) = part_prefix(&codes, start, end) {
                if !prefixes.contains(&prefix) {
                    prefixes.push(prefix);
                }
            }
        }
        prefixes.len() > 1
    };
    let mut i = 0usize;
    while i < length {
        let code = codes[i] as u32;
        if is_white_space(code) || code == 0x2c {
            if let Some(s) = start {
                if add_part(s, i, &mut prefixes) {
                    return prefixes;
                }
            }
            start = None;
        } else if code == 0x2e || code == 0x3a {
            if let Some(s) = start {
                if add_part(s, i, &mut prefixes) {
                    return prefixes;
                }
            }
            start = Some(i);
            if code == 0x3a && codes.get(i + 1) == Some(&0x3a) {
                i += 1;
            }
        } else if matches!(code, 0x2b | 0x3e | 0x7e | 0x7c) {
            if let Some(s) = start {
                if add_part(s, i, &mut prefixes) {
                    return prefixes;
                }
            }
            start = None;
            if code == 0x7c && codes.get(i + 1) == Some(&0x7c) {
                i += 1;
            }
        } else {
            if start.is_none() {
                start = Some(i);
            }
            if code == 0x5c {
                i += 1;
            } else if code == 0x28 || code == 0x5b {
                i = matching_close(&codes, i, length);
            }
        }
        i += 1;
    }
    if let Some(s) = start {
        add_part(s, length, &mut prefixes);
    }
    prefixes
}

fn same_prefixes(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().all(|p| b.contains(p))
}

/// move the declarations shared by both rules into a new rule. `(result, node1, node2)`
fn diff(arena: &AstArena, n1: u32, n2: u32, options: &ParserOptions, context: &mut MinifyContext) -> Option<(Option<u32>, u32, u32)> {
    // the declarations of a rule that applies a mixin are not moved
    if has_at_rule_statement(arena, n1) || has_at_rule_statement(arena, n2) {
        return None;
    }
    context.cache.get_or_insert_with(HashMap::default);
    let t = |x: u32| arena.data_types.get(x);
    let (mut node1, mut node2) = (n1, n2);
    let mut exchanged = false;
    if arena.child_count(node1) > arena.child_count(node2) {
        std::mem::swap(&mut node1, &mut node2);
        exchanged = true;
    }
    // the prefixes of the cached raw selectors are read in place
    // the prefixes are computed once per split selector (see RawSelector) or per selector string
    let cached1 = arena.raw.borrow().get(&node1).map(|r| r.prefixes(raw_selector_prefixes));
    let prefixes1 = cached1.unwrap_or_else(|| selector_prefixes_of(arena, arena.name.get(node1)));
    let cached2 = arena.raw.borrow().get(&node2).map(|r| r.prefixes(raw_selector_prefixes));
    let prefixes2 = cached2.unwrap_or_else(|| selector_prefixes_of(arena, arena.name.get(node2)));
    if !same_prefixes(&prefixes1, &prefixes2) {
        return None;
    }
    // match the declarations of both rules, from the last to the first
    let mut matches = 0;
    let mut child1 = arena.last_child.get(node1);
    while child1 != 0 {
        if t(child1) != EnumToken::CommentNodeType {
            let mut child2 = arena.last_child.get(node2);
            while child2 != 0 {
                if t(child2) != EnumToken::CommentNodeType
                    && arena.flags.get(child2) & EnumAstNodeStatus::Matched == 0
                    && arena.name.get(child1) == arena.name.get(child2)
                    && eq_node(arena, child1, child2)
                {
                    arena.flags.set(child1, arena.flags.get(child1) | EnumAstNodeStatus::Matched);
                    arena.flags.set(child2, arena.flags.get(child2) | EnumAstNodeStatus::Matched);
                    matches += 1;
                    break;
                }
                child2 = arena.previous_sibling.get(child2);
            }
        }
        child1 = arena.previous_sibling.get(child1);
    }
    if matches == 0 && (arena.first_child.get(node1) != 0 || arena.first_child.get(node2) != 0) {
        return None;
    }
    // the rules are rendered with the same options during the minification
    if context.renderer.is_none() {
        context.renderer = Some(NodeRenderer::new(options));
    }
    let rendered_size = |context: &mut MinifyContext, node: u32, cache: bool| -> usize {
        let mut size = if cache { context.cache.as_ref().unwrap().get(&node).copied() } else { None };
        if size.is_none() {
            let s = context.renderer.as_mut().unwrap().size(arena, node);
            if cache {
                context.cache.as_mut().unwrap().insert(node, s);
            }
            size = Some(s);
        }
        if arena.first_child.get(node) == 0 {
            0
        } else {
            size.unwrap()
        }
    };
    // size of the original nodes
    let size = rendered_size(context, n1, true) + rendered_size(context, n2, true);
    // the shared declarations are moved into the result node
    let result = arena.clone_node(node1);
    let merged = merge_selectors(arena, arena.name.get(node1), arena.name.get(node2)).unwrap_or_else(|| {
        let mut seen: Vec<String> = Vec::new();
        for s in split_rule(&arena.resolve_string(arena.name.get(node1)))
            .into_iter()
            .chain(split_rule(&arena.resolve_string(arena.name.get(node2))))
        {
            let joined = s.concat();
            if !seen.contains(&joined) {
                seen.push(joined);
            }
        }
        arena.intern_string(&seen.join(","))
    });
    arena.name.set(result, merged);
    // the matched declarations of node1 are moved into the result, the duplicates in node2 are dropped
    let mut moves: Vec<(u32, u32, u32)> = Vec::new();
    let mut move_matched = |parent: u32, target: Option<u32>| {
        let mut child = arena.first_child.get(parent);
        while child != 0 {
            let next = arena.next_sibling.get(child);
            if arena.flags.get(child) & EnumAstNodeStatus::Matched != 0 {
                arena.flags.set(child, arena.flags.get(child) & !EnumAstNodeStatus::Matched);
                moves.push((child, parent, next));
                match target {
                    None => arena.detach(child),
                    Some(target) => arena.append_child(target, child),
                }
            }
            child = next;
        }
    };
    move_matched(node1, Some(result));
    move_matched(node2, None);
    arena.insert_before(n1, &[result]);
    if let Some(cache) = context.cache.as_mut() {
        cache.remove(&node1);
        cache.remove(&node2);
    }
    let merged_size = rendered_size(context, node1, false) + rendered_size(context, node2, false) + rendered_size(context, result, false);
    if size < merged_size || (size == merged_size && arena.first_child.get(node1) != 0 && arena.first_child.get(node2) != 0) {
        // restore the nodes: undo the moves in reverse order
        for &(child, parent, next) in moves.iter().rev() {
            if next == 0 {
                arena.append_child(parent, child);
            } else {
                arena.insert_before(next, &[child]);
            }
        }
        arena.detach(result);
        return None;
    }
    arena.set_token_list(result, &[]);
    // the raw and optimized selectors are not copied by cloneNode()
    arena.raw.borrow_mut().remove(&result);
    arena.optimized.borrow_mut().remove(&result);
    if let Some(optimized) = optimize_selector(&split_rule(&arena.resolve_string(arena.name.get(result)))) {
        if optimized.match_ {
            if let Some(sel) = factor_selector(&optimized) {
                if js_len(&sel) < arena.string_length(arena.name.get(result)) {
                    set_selector(arena, result, &sel);
                    arena.set_token_list(result, &[]);
                }
            }
        }
    }
    arena.detach(result);
    Some((Some(result), if exchanged { node2 } else { node1 }, if exchanged { node1 } else { node2 }))
}

/// compare two nodes
fn eq_node(arena: &AstArena, a: u32, b: u32) -> bool {
    if a == b {
        return true;
    }
    if arena.data_types.get(a) != arena.data_types.get(b) || arena.name.get(a) != arena.name.get(b) || arena.string_value.get(a) != arena.string_value.get(b) {
        return false;
    }
    if arena.flags.get(a) & EnumAstNodeStatus::HasValues != 0 {
        let mut token1 = arena.first_token.get(a);
        let mut token2 = arena.first_token.get(b);
        while token1 != 0 && token2 != 0 {
            if !eq_token(arena, token1, token2) {
                return false;
            }
            token1 = arena.next_sibling.get(token1);
            token2 = arena.next_sibling.get(token2);
        }
        if token1 != token2 {
            return false;
        }
    }
    if arena.flags.get(a) & EnumAstNodeStatus::HasChildren != 0 {
        let mut child1 = arena.first_child.get(a);
        let mut child2 = arena.first_child.get(b);
        while child1 != 0 && child2 != 0 {
            if !eq_node(arena, child1, child2) {
                return false;
            }
            child1 = arena.next_sibling.get(child1);
            child2 = arena.next_sibling.get(child2);
        }
        return child1 == child2;
    }
    true
}

/// the bounds of the comma separated selectors of a selector string appended to `codes`. `None` when the selector
/// string is not normalized (whitespace other than a single space between two parts, a space after a comma)
fn selector_ranges(arena: &AstArena, id: u32, codes: &mut Vec<u16>, bounds: &mut Vec<(usize, usize)>) -> Option<()> {
    let length = arena.string_length(id);
    let offset = codes.len();
    arena.copy_string(id, codes);
    let mut start = offset;
    let mut i = offset;
    while i < offset + length {
        let code = codes[i] as u32;
        if is_white_space(code) {
            // a single space between two parts
            if code != 0x20 || i == start || is_white_space(codes[i - 1] as u32) {
                return None;
            }
        } else if code == 0x2c {
            bounds.push((start, i));
            start = i + 1;
        } else if code == 0x5c {
            i += 1;
        } else if code == 0x28 || code == 0x5b {
            i = matching_close(codes, i, offset + length);
        }
        i += 1;
    }
    bounds.push((start, offset + length));
    Some(())
}

/// the distinct selectors of two selector strings, comma separated. `None` when a selector string is not normalized
fn merge_selectors(arena: &AstArena, id1: u32, id2: u32) -> Option<u32> {
    let mut codes: Vec<u16> = Vec::new();
    let mut bounds: Vec<(usize, usize)> = Vec::new();
    selector_ranges(arena, id1, &mut codes, &mut bounds)?;
    selector_ranges(arena, id2, &mut codes, &mut bounds)?;
    let mut result: Vec<u16> = Vec::new();
    let mut written = 0;
    for i in 0..bounds.len() {
        let (start1, end1) = bounds[i];
        let duplicate = (0..i).any(|k| codes[bounds[k].0..bounds[k].1] == codes[start1..end1]);
        if duplicate {
            continue;
        }
        if written > 0 {
            result.push(0x2c);
        }
        written += 1;
        result.extend_from_slice(&codes[start1..end1]);
    }
    Some(arena.intern_units(&result))
}

/// the split selector of a rule (see split_rule()): computed when it is not cached
fn raw_selector(arena: &AstArena, node: u32) -> Rc<RawSelector> {
    if let Some(raw) = arena.raw.borrow().get(&node) {
        return raw.clone();
    }
    let raw = Rc::new(RawSelector::from(split_rule(&arena.resolve_string(arena.name.get(node)))));
    arena.raw.borrow_mut().insert(node, raw.clone());
    raw
}

/// the first part of the first selector of a selector string, cached by string id (the string epoch invalidates the
/// cache when the ids are renumbered): the code units, `None` when the first selector is empty
fn first_part_of(arena: &AstArena, id: u32) -> Option<Rc<[u16]>> {
    let epoch = arena.string_epoch();
    {
        let cache = arena.first_parts.borrow();
        if cache.0 == epoch {
            if let Some(part) = cache.1.get(&id) {
                return part.clone();
            }
        }
    }
    let part: Option<Rc<[u16]>> = first_part_units(arena, id).map(Rc::from);
    let mut cache = arena.first_parts.borrow_mut();
    if cache.0 != epoch {
        cache.1.clear();
        cache.0 = epoch;
    }
    cache.1.insert(id, part.clone());
    part
}

/// the first part of the first selector of a selector string: the code units, `None` when the first selector is empty
fn first_part_units(arena: &AstArena, id: u32) -> Option<Vec<u16>> {
    let codes = read_selector(arena, id);
    let length = codes.len();
    let code_at = |i: usize| codes.get(i).copied().map_or(u32::MAX, u32::from);
    let mut start: Option<usize> = None;
    let mut i = 0;
    while i < length && is_white_space(codes[i] as u32) {
        i += 1;
    }
    let part_start = i;
    let range = |end: usize| Some(codes[part_start..end].to_vec());
    while i < length {
        let code = codes[i] as u32;
        if is_white_space(code) {
            return range(i);
        }
        if code == 0x2c {
            return if start.is_none() { None } else { range(i) };
        }
        if code == 0x2e || code == 0x3a {
            if start.is_some() {
                return range(i);
            }
            start = Some(i);
            if code == 0x3a && code_at(i + 1) == 0x3a {
                i += 1;
            }
            i += 1;
            continue;
        }
        if matches!(code, 0x2b | 0x3e | 0x7e | 0x7c) {
            let end = if start.is_some() {
                i
            } else if code == 0x7c && code_at(i + 1) == 0x7c {
                i + 2
            } else {
                i + 1
            };
            return range(end.min(length));
        }
        if start.is_none() {
            start = Some(i);
        }
        if code == 0x5c {
            i += 2;
            continue;
        }
        if code == 0x28 || code == 0x5b {
            i = matching_close(&codes, i, length);
        }
        i += 1;
    }
    if start.is_none() {
        None
    } else {
        range(i.min(length))
    }
}

/// false when the selectors of two rules cannot match: the first parts of their first selectors are different
fn may_match_selectors(arena: &AstArena, node1: u32, node2: u32) -> bool {
    // the first part of a split selector is its first token (a string), the first part of a selector string is read
    // from its code units: the parts are compared without a copy
    let raw = arena.raw.borrow();
    let part = |node: u32| -> Result<Option<&str>, u32> {
        match raw.get(&node) {
            Some(r) => Ok(r.first().and_then(|r| r.first()).map(String::as_str)),
            None => Err(arena.name.get(node)),
        }
    };
    match (part(node1), part(node2)) {
        (Ok(a), Ok(b)) => a.is_some() && a == b,
        (Ok(a), Err(id)) | (Err(id), Ok(a)) => match (a, first_part_of(arena, id)) {
            (Some(a), Some(b)) => a.encode_utf16().eq(b.iter().copied()),
            _ => false,
        },
        (Err(id1), Err(id2)) => match (first_part_of(arena, id1), first_part_of(arena, id2)) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        },
    }
}

/// true if a rule selector is not modified by reduce_rule_selector(): a single selector (no comma), that does not
/// start with `&`, does not end with `)`, a combinator or a whitespace, and has no whitespace other than single spaces
fn is_simple_selector(arena: &AstArena, id: u32) -> bool {
    arena.string_flag_id(id, StringFlag::SimpleSelector, |id| {
        let codes = read_selector(arena, id);
        if codes.is_empty() || codes[0] == 0x26 {
            return false;
        }
        let mut previous = 0u32;
        for (i, &code) in codes.iter().enumerate() {
            let code = code as u32;
            if code == 0x2c {
                return false;
            }
            if is_white_space(code) && (code != 0x20 || i == 0 || is_white_space(previous)) {
                return false;
            }
            previous = code;
        }
        !(matches!(previous, 0x29 | 0x2b | 0x3e | 0x7e | 0x7c) || is_white_space(previous))
    })
}

/// reduce a rule selector
fn reduce_rule_selector(arena: &AstArena, node: u32) {
    let raw = raw_selector(arena, node);
    let Some(mut optimized) = optimize_selector(&raw) else {
        return;
    };
    if optimized.match_ && optimized.reducible && optimized.selector.len() > 1 {
        for selector in optimized.selector.iter_mut() {
            if selector.len() > 1
                && selector[0] == "&"
                && (is_combinator(&selector[1]) || !selector[1].chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == ':'))
            {
                selector.remove(0);
            }
        }
        let mut unique: HashSet<String> = HashSet::default();
        let mut reduced: Vec<String> = Vec::new();
        for curr in &optimized.selector {
            if unique.insert(curr.concat()) {
                if !reduced.is_empty() {
                    reduced.push(",".into());
                }
                reduced.extend(curr.iter().cloned());
            }
        }
        let mut first = vec![optimized.optimized[0].clone(), if reduced.len() == 1 { reduced.concat() } else { ":is(".into() }];
        first.extend(reduced);
        first.push(")".into());
        let sel = first.concat();
        let shared = Rc::new(RefCell::new(optimized));
        arena.optimized.borrow_mut().insert(node, shared);
        if js_len(&sel) < arena.string_length(arena.name.get(node)) {
            set_selector(arena, node, &sel);
            arena.raw.borrow_mut().insert(node, Rc::<RawSelector>::new(RawSelector::from(vec![first])));
            arena.set_token_list(node, &[]);
        }
        return;
    }
    arena.optimized.borrow_mut().insert(node, Rc::new(RefCell::new(optimized)));
}
