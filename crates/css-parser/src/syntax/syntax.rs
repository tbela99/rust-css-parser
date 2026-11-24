//! Port of `src/syntax/syntax.ts`.

use crate::ast::arena::{AstArena, TokenField, TokenValue};
use crate::ast::strings::StringFlag;
use crate::ast::types::{ColorType, EnumToken, WalkerOptionEnum};
use crate::ast::walk::{ValueWalker, WalkerValueOptions, WalkerValueResult};
use crate::parser::utils::angle::to_degrees;
use crate::parser::utils::text::{equals_ignore_case, lowercase};
use crate::syntax::color::utils::colorspace::get_color_space;
use crate::syntax::color::utils::components::get_color_components;
use crate::syntax::color::utils::distance::is_ok_lab_close;
use crate::syntax::constants::{
    anglePrecision, colorPrecision, color_name_to_hex, epsilon, non_standard_colors, validation_units, COLORS_FUNC,
    COLOR_FUNC_COLOR_SPACE, DEPRECATED_SYSTEM_COLORS, SYSTEM_COLORS,
};
use crate::types::ErrorDescription;
use crate::utils::number::{js_number_to_string, js_round};
use crate::validation::match_::trim_array;
use crate::validation::utils::list::split_token_list_comma;
use std::cell::RefCell;
use crate::utils::fx::HashMap;

const REVERSE_SOLIDUS: u32 = 0x5c;

pub const FLEX_UNITS: [&str; 1] = ["fr"];
pub const FREQUENCY_UNITS: [&str; 2] = ["Hz", "kHz"];
pub const TIME_UNITS: [&str; 2] = ["ms", "s"];
pub const ANGLE_UNITS: [&str; 4] = ["rad", "turn", "deg", "grad"];
pub const RESOLUTION_UNITS: [&str; 4] = ["dpi", "dpcm", "dppx", "x"];
pub const DIMENSION_UNITS: [&str; 49] = ["cap", "ch", "cm", "cqb", "cqh", "cqi", "cqmax", "cqmin", "cqw", "dvb", "dvh", "dvi", "dvmax", "dvmin", "dvw", "em", "ex", "ic", "in", "lh", "lvb", "lvh", "lvi", "lvmax", "lvmin", "lvw", "mm", "pc", "pt", "px", "Q", "rcap", "rch", "rem", "rex", "ric", "rlh", "svb", "svh", "svi", "svmax", "svmin", "svw", "vb", "vh", "vi", "vmax", "vmin", "vw"];

/// `pseudoAliasMap`
pub fn pseudo_alias_map(name: &str) -> Option<&'static str> {
    match name {
        "-moz-center" => Some("center"),
        "-webkit-center" => Some("center"),
        "-ms-grid-columns" => Some("grid-template-columns"),
        "-ms-grid-rows" => Some("grid-template-rows"),
        "-ms-grid-row" => Some("grid-row-start"),
        "-ms-grid-column" => Some("grid-column-start"),
        "-ms-grid-row-align" => Some("align-self"),
        "-ms-grid-row-span" => Some("grid-row-end"),
        "-ms-grid-column-span" => Some("grid-column-end"),
        "-ms-grid-column-align" => Some("justify-self"),
        ":-ms-input-placeholder" => Some("::placeholder"),
        "::-ms-input-placeholder" => Some("::placeholder"),
        ":-moz-any()" => Some(":is"),
        "-moz-user-modify" => Some("user-modify"),
        "-webkit-match-parent" => Some("match-parent"),
        "-moz-background-clip" => Some("background-clip"),
        "-moz-background-origin" => Some("background-origin"),
        "-ms-input-placeholder" => Some("placeholder"),
        ":-webkit-autofill" => Some(":autofill"),
        ":-webkit-any()" => Some(":is"),
        "::-webkit-input-placeholder" => Some("::placeholder"),
        "::-webkit-file-upload-button" => Some("::file-selector-button"),
        "::-moz-placeholder" => Some("::placeholder"),
        ":-webkit-any-link" => Some(":any-link"),
        "-webkit-border-after" => Some("border-block-end"),
        "-webkit-border-after-color" => Some("border-block-end-color"),
        "-webkit-border-after-style" => Some("border-block-end-style"),
        "-webkit-border-after-width" => Some("border-block-end-width"),
        "-webkit-border-before" => Some("border-block-start"),
        "-webkit-border-before-color" => Some("border-block-start-color"),
        "-webkit-border-before-style" => Some("border-block-start-style"),
        "-webkit-border-before-width" => Some("border-block-start-width"),
        "-webkit-border-end" => Some("border-inline-end"),
        "-webkit-border-end-color" => Some("border-inline-end-color"),
        "-webkit-border-end-style" => Some("border-inline-end-style"),
        "-webkit-border-end-width" => Some("border-inline-end-width"),
        "-webkit-border-start" => Some("border-inline-start"),
        "-webkit-border-start-color" => Some("border-inline-start-color"),
        "-webkit-border-start-style" => Some("border-inline-start-style"),
        "-webkit-border-start-width" => Some("border-inline-start-width"),
        "-webkit-box-align" => Some("align-items"),
        "-webkit-box-direction" => Some("flex-direction"),
        "-webkit-box-flex" => Some("flex-grow"),
        "-webkit-box-lines" => Some("flex-flow"),
        "-webkit-box-ordinal-group" => Some("order"),
        "-webkit-box-orient" => Some("flex-direction"),
        "-webkit-box-pack" => Some("justify-content"),
        "-webkit-column-break-after" => Some("break-after"),
        "-webkit-column-break-before" => Some("break-before"),
        "-webkit-column-break-inside" => Some("break-inside"),
        "-webkit-font-feature-settings" => Some("font-feature-settings"),
        "-webkit-hyphenate-character" => Some("hyphenate-character"),
        "-webkit-initial-letter" => Some("initial-letter"),
        "-webkit-margin-end" => Some("margin-block-end"),
        "-webkit-margin-start" => Some("margin-block-start"),
        "-webkit-padding-after" => Some("padding-block-end"),
        "-webkit-padding-before" => Some("padding-block-start"),
        "-webkit-padding-end" => Some("padding-inline-end"),
        "-webkit-padding-start" => Some("padding-inline-start"),
        "-webkit-min-device-pixel-ratio" => Some("min-resolution"),
        "-webkit-max-device-pixel-ratio" => Some("max-resolution"),
        "-webkit-font-smoothing" => Some("font-smooth"),
        "-webkit-line-clamp" => Some("line-clamp"),
        ":-webkit-autofill-strong-password" => Some(":autofill"),
        ":-webkit-full-page-media" => Some(":fullscreen"),
        ":-webkit-full-screen" => Some(":fullscreen"),
        ":-webkit-full-screen-ancestor" => Some(":fullscreen"),
        ":-webkit-full-screen-document" => Some(":fullscreen"),
        ":-webkit-full-screen-controls-hidden" => Some(":fullscreen"),
        "-moz-background-inline-policy" => Some("box-decoration-break"),
        "-moz-background-size" => Some("background-size"),
        "-moz-border-end" => Some("border-inline-end"),
        "-moz-border-end-color" => Some("border-inline-end-color"),
        "-moz-border-end-style" => Some("border-inline-end-style"),
        "-moz-border-end-width" => Some("border-inline-end-width"),
        "-moz-border-image" => Some("border-inline-end-width"),
        "-moz-border-start" => Some("border-inline-start"),
        "-moz-border-start-color" => Some("border-inline-start-color"),
        "-moz-border-start-style" => Some("border-inline-start-style"),
        "-moz-border-start-width" => Some("border-inline-start-width"),
        "-moz-column-count" => Some("column-count"),
        "-moz-column-fill" => Some("column-fill"),
        "-moz-column-gap" => Some("column-gap"),
        "-moz-column-width" => Some("column-width"),
        "-moz-column-rule" => Some("column-rule"),
        "-moz-column-rule-width" => Some("column-rule-width"),
        "-moz-column-rule-style" => Some("column-rule-style"),
        "-moz-column-rule-color" => Some("column-rule-color"),
        "-moz-margin-end" => Some("margin-inline-end"),
        "-moz-margin-start" => Some("margin-inline-start"),
        "-moz-opacity" => Some("opacity"),
        "-moz-outline" => Some("outline"),
        "-moz-outline-color" => Some("outline-color"),
        "-moz-outline-offset" => Some("outline-offset"),
        "-moz-outline-style" => Some("outline-style"),
        "-moz-outline-width" => Some("outline-width"),
        "-moz-padding-end" => Some("padding-inline-end"),
        "-moz-padding-start" => Some("padding-inline-start"),
        "-moz-tab-size" => Some("tab-size"),
        "-moz-text-align-last" => Some("text-align-last"),
        "-moz-text-decoration-color" => Some("text-decoration-color"),
        "-moz-text-decoration-line" => Some("text-decoration-line"),
        "-moz-text-decoration-style" => Some("text-decoration-style"),
        "-moz-transition" => Some("transition"),
        "-moz-transition-delay" => Some("transition-delay"),
        "-moz-transition-duration" => Some("transition-duration"),
        "-moz-transition-property" => Some("transition-property"),
        "-moz-transition-timing-function" => Some("transition-timing-function"),
        "-moz-user-select" => Some("user-select"),
        "-moz-initial" => Some("initial"),
        "-moz-linear-gradient()" => Some("linear-gradient"),
        "-moz-radial-gradient()" => Some("radial-gradient"),
        "-moz-element()" => Some("element"),
        "-moz-crisp-edges" => Some("crisp-edges"),
        "-moz-calc()" => Some("calc"),
        "-moz-min-content" => Some("min-content"),
        "-moz-fit-content" => Some("fit-content"),
        "-moz-max-content" => Some("max-content"),
        "-moz-available" => Some("stretch"),
        ":-moz-any-link" => Some(":any-link"),
        ":-moz-full-screen" => Some(":fullscreen"),
        ":-moz-full-screen-ancestor" => Some(":fullscreen"),
        ":-moz-placeholder" => Some(":placeholder-shown"),
        ":-moz-read-only" => Some(":read-only"),
        ":-moz-read-write" => Some(":read-write"),
        ":-moz-submit-invalid" => Some(":invalid"),
        ":-moz-ui-invalid" => Some(":user-invalid"),
        ":-moz-ui-valid" => Some(":user-valid"),
        "::-moz-selection" => Some("::selection"),
        _ => None,
    }
}

/// `renamedStandardProperties`
pub fn renamed_standard_properties(name: &str) -> Option<&'static str> {
    match name {
        "color-adjust" => Some("print-color-adjust"),
        "font-stretch" => Some("font-width"),
        "grid-column-gap" => Some("column-gap"),
        "grid-gap" => Some("gap"),
        "grid-row-gap" => Some("row-gap"),
        "word-wrap" => Some("overflow-wrap"),
        _ => None,
    }
}

fn unit_in(units: &[&str], unit: &str) -> bool {
    units.iter().any(|u| u.eq_ignore_ascii_case(unit))
}

pub fn is_length_unit(unit: &str) -> bool {
    unit_in(&DIMENSION_UNITS, unit)
}

pub fn is_resolution_unit(unit: &str) -> bool {
    unit_in(&RESOLUTION_UNITS, unit)
}

pub fn is_angle_unit(unit: &str) -> bool {
    unit_in(&ANGLE_UNITS, unit)
}

pub fn is_time_unit(unit: &str) -> bool {
    unit_in(&TIME_UNITS, unit)
}

pub fn is_frequency_unit(unit: &str) -> bool {
    unit_in(&FREQUENCY_UNITS, unit)
}

pub fn is_flex_unit(unit: &str) -> bool {
    unit_in(&FLEX_UNITS, unit)
}

fn unit_test(arena: &AstArena, dimension: u32, test: fn(&str) -> bool) -> bool {
    arena.unit.get(dimension) != 0 && arena.with_string(arena.unit.get(dimension), test)
}

pub fn is_length(arena: &AstArena, dimension: u32) -> bool {
    unit_test(arena, dimension, is_length_unit)
}

pub fn is_resolution(arena: &AstArena, dimension: u32) -> bool {
    unit_test(arena, dimension, is_resolution_unit)
}

pub fn is_angle(arena: &AstArena, dimension: u32) -> bool {
    unit_test(arena, dimension, is_angle_unit)
}

pub fn is_time(arena: &AstArena, dimension: u32) -> bool {
    unit_test(arena, dimension, is_time_unit)
}

pub fn is_frequency(arena: &AstArena, dimension: u32) -> bool {
    unit_test(arena, dimension, is_frequency_unit)
}

pub fn is_flex(arena: &AstArena, dimension: u32) -> bool {
    unit_test(arena, dimension, is_flex_unit)
}

fn rebuild_stops(arena: &AstArena, stops: &mut Vec<u32>, parts: &[Vec<u32>]) {
    stops.clear();
    for part in parts {
        if !stops.is_empty() {
            stops.push(arena.create_token(EnumToken::CommaTokenType, TokenValue::None, None));
        }
        stops.extend_from_slice(part);
    }
}

/// reduce color stops
pub fn reduce_color_stops(arena: &AstArena, stops: &mut Vec<u32>) {
    let mut parts = split_token_list_comma(arena, stops);
    let n = if parts.len() == 1 { 1.0 } else { (parts.len() - 1) as f64 };
    let mut k: f64 = -1.0;
    let mut updated = false;
    let mut i: isize = 0;
    while (i as usize) < parts.len() {
        let iu = i as usize;
        k += 1.0;
        if parts[iu].len() != 3 {
            i += 1;
            continue;
        }
        if iu > 0 && is_ok_lab_close(arena, parts[iu - 1][0], parts[iu][0]) {
            if parts[iu - 1].len() == 1 {
                let ws = arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None);
                let pc = arena.create_token(EnumToken::PercentageTokenType, TokenValue::Num(((k - 1.0) * 100.0) / n), None);
                parts[iu - 1].push(ws);
                parts[iu - 1].push(pc);
            }
            let tail: Vec<u32> = parts[iu][1..].to_vec();
            parts[iu - 1].extend(tail);
            parts.remove(iu);
            updated = true;
            continue;
        }
        for j in 0..parts[iu].len() {
            let t = parts[iu][j];
            let typ = arena.data_types.get(t);
            if (typ == EnumToken::LengthTokenType && arena.numeric_value.get(t) == 0.0)
                || typ == EnumToken::NumberTokenType
                || typ == EnumToken::PercentageTokenType
            {
                if arena.numeric_value.get(t) == (k * 100.0) / n {
                    parts[iu].truncate(j);
                    trim_array(arena, &mut parts[iu]);
                    updated = true;
                    break;
                }
            }
        }
        i += 1;
    }
    if updated {
        rebuild_stops(arena, stops, &parts);
    }
}

/// reduce background-position values
pub fn reducegradient_background_position(arena: &AstArena, positions: &mut Vec<u32>, position: &str) {
    let pc = |v: f64| arena.create_token(EnumToken::PercentageTokenType, TokenValue::Num(v), None);
    let ws = || arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None);
    let iden = |v: &str| arena.create_token(EnumToken::IdenTokenType, TokenValue::Str(v), None);
    let replace: Option<Vec<u32>> = match position {
        "50%" | "50% 50%" | "center" | "center center" => Some(vec![]),
        "0% 50%" | "0 50%" | "left" | "left center" | "center left" => Some(vec![pc(0.0)]),
        "50% 0%" | "50% 0" | "top center" | "center top" => Some(vec![iden("top")]),
        "bottom center" | "center bottom" | "bottom" | "50% 100%" => Some(vec![iden("bottom")]),
        "right center" | "center right" | "100% 50%" | "right" => Some(vec![pc(100.0)]),
        "bottom left" | "left bottom" => Some(vec![pc(0.0), ws(), pc(100.0)]),
        "bottom right" | "right bottom" => Some(vec![pc(100.0), ws(), pc(100.0)]),
        "top left" | "left top" => Some(vec![pc(0.0), ws(), pc(0.0)]),
        "top right" | "right top" => Some(vec![pc(100.0), ws(), pc(0.0)]),
        _ => None,
    };
    if let Some(replace) = replace {
        *positions = replace;
    }
}

/// reduce conic-gradient color stops
pub fn reduce_conic_color_stops(arena: &AstArena, stops: &mut Vec<u32>) {
    let mut parts = split_token_list_comma(arena, stops);
    let n = if parts.len() == 1 { 1.0 } else { (parts.len() - 1) as f64 };
    let mut k: f64 = -1.0;
    let mut updated = false;
    let mut i: usize = 0;
    while i < parts.len() {
        k += 1.0;
        if parts[i].len() != 3 {
            i += 1;
            continue;
        }
        if i > 0 && is_ok_lab_close(arena, parts[i - 1][0], parts[i][0]) {
            if parts[i - 1].len() == 1 {
                let ws = arena.create_token(EnumToken::WhitespaceTokenType, TokenValue::None, None);
                let an = arena.create_token(EnumToken::AngleTokenType, TokenValue::Num(((k - 1.0) * 100.0) / n), Some("deg"));
                parts[i - 1].push(ws);
                parts[i - 1].push(an);
            }
            let tail: Vec<u32> = parts[i][1..].to_vec();
            parts[i - 1].extend(tail);
            parts.remove(i);
            updated = true;
            continue;
        }
        for j in 0..parts[i].len() {
            let t = parts[i][j];
            let typ = arena.data_types.get(t);
            if (typ == EnumToken::NumberTokenType && arena.numeric_value.get(t) == 0.0) || typ == EnumToken::AngleTokenType {
                if arena.numeric_value.get(to_degrees(arena, t)) == (k * 360.0) / n {
                    parts[i].truncate(j);
                    trim_array(arena, &mut parts[i]);
                    updated = true;
                    break;
                }
            }
        }
        i += 1;
    }
    if updated {
        rebuild_stops(arena, stops, &parts);
    }
}

/// is rectangular orthogonal colorspace
pub fn is_rectangular_orthogonal_colorspace(arena: &AstArena, token: u32) -> bool {
    arena.data_types.get(token) == EnumToken::IdenTokenType
        && arena.with_string(arena.string_value.get(token), |v| COLOR_FUNC_COLOR_SPACE.iter().any(|t| equals_ignore_case(t, v)))
}

/// is polar colorspace
pub fn is_polar_colorspace(arena: &AstArena, token: u32) -> bool {
    arena.data_types.get(token) == EnumToken::IdenTokenType
        && arena.with_string(arena.string_value.get(token), |v| ["hsl", "hwb", "lch", "oklch"].iter().any(|t| equals_ignore_case(t, v)))
}

/// is ident color
pub fn is_ident_color(arena: &AstArena, token: u32) -> bool {
    let kin = arena.color_type.get(token);
    arena.data_types.get(token) == EnumToken::ColorTokenType
        && kin != ColorType::NONE
        && [ColorType::SYS, ColorType::DPSYS, ColorType::LIT].contains(&kin)
        && arena.with_string(arena.string_value.get(token), is_ident)
}

/// true if the identifier is a color: a system color, a named color, `currentcolor` or `transparent`
pub fn is_color_name(str: &str) -> bool {
    let val = lowercase(str);
    let val: &str = &val;
    if SYSTEM_COLORS.contains(&val) || DEPRECATED_SYSTEM_COLORS.contains(&val) || non_standard_colors().contains(val) {
        return true;
    }
    color_name_to_hex(val).is_some() || val == "currentcolor" || val == "transparent"
}

/// color function arguments of the given types
fn filter_children(arena: &AstArena, token: u32, types: &[u8]) -> Vec<u32> {
    arena.token_iter(token).filter(|&t| types.contains(&arena.data_types.get(t))).collect()
}

const COLOR_KEYWORDS: [&str; 9] = ["in", "hue", "from", "alpha", "none", "shorter", "longer", "increasing", "decreasing"];

/// `typeof Math[val.toUpperCase()] === "number"`: the `Math` constants
fn is_math_constant(val: &str) -> bool {
    matches!(val.to_uppercase().as_str(), "E" | "LN10" | "LN2" | "LOG10E" | "LOG2E" | "PI" | "SQRT1_2" | "SQRT2")
}

/// `color_space` is a string of channel names (one character each)
/// `ColorType[name.replace('-', '_').toUpperCase()]`: the name is converted in a buffer on the stack
fn color_type_of(name: &str) -> u8 {
    let mut buffer = [0u8; 32];
    if name.len() > buffer.len() || !name.is_ascii() {
        return ColorType::value(&name.replace('-', "_").to_uppercase()).unwrap_or(0);
    }
    for (i, b) in name.bytes().enumerate() {
        buffer[i] = if b == b'-' { b'_' } else { b.to_ascii_uppercase() };
    }
    // SAFETY: ascii
    ColorType::value(unsafe { std::str::from_utf8_unchecked(&buffer[..name.len()]) }).unwrap_or(0)
}

fn unexpected_constant(val: &str, color_space: &str) -> bool {
    !is_math_constant(val)
        && !COLOR_KEYWORDS.contains(&val)
        && !COLORS_FUNC.contains(&val)
        && !(val.chars().count() == 1 && color_space.contains(val))
        && !COLOR_FUNC_COLOR_SPACE.contains(&val)
}

fn push_error(errors: &mut Option<&mut Vec<ErrorDescription>>, message: String, node: u32) {
    if let Some(errors) = errors.as_deref_mut() {
        errors.push(ErrorDescription::drop(message).with_node(node));
    }
}

/// is a color
pub fn is_color(arena: &AstArena, token: u32, mut errors: Option<&mut Vec<ErrorDescription>>) -> bool {
    let typ = arena.data_types.get(token);
    if typ == EnumToken::WildCardFunctionTokenType {
        return true;
    }
    if typ == EnumToken::ColorTokenType && arena.color_type.get(token) != ColorType::NONE && arena.first_token.get(token) == 0 {
        return true;
    }
    if typ == EnumToken::IdenTokenType {
        return arena.string_flag(arena.string_value.get(token), StringFlag::Color, is_color_name);
    }
    if (typ == EnumToken::FunctionTokenType || typ == EnumToken::ColorTokenType) && arena.first_token.get(token) != 0 {
        let name = arena.resolve_string(arena.string_value.get(token));
        if &*name == "light-dark" {
            let children = filter_children(
                arena,
                token,
                &[
                    EnumToken::IdenTokenType,
                    EnumToken::NumberTokenType,
                    EnumToken::LiteralTokenType,
                    EnumToken::ColorTokenType,
                    EnumToken::FunctionTokenType,
                    EnumToken::PercentageTokenType,
                    EnumToken::WildCardFunctionTokenType,
                ],
            );
            if children.len() != 2 {
                push_error(&mut errors, "light-dark function must have 2 arguments".into(), token);
                return false;
            }
            if is_color(arena, children[0], None) && is_color(arena, children[1], None) {
                return true;
            }
        }
        if let Some(components) = get_color_components(arena, token) {
            if let Some(color_space) = get_color_space(arena, token) {
                for &value in &components {
                    if arena.data_types.get(value) == EnumToken::IdenTokenType {
                        let val = arena.resolve_string(arena.string_value.get(value)).to_lowercase();
                        if unexpected_constant(&val, &color_space) {
                            push_error(&mut errors, format!("Unexpected constant '{}'", val), value);
                            return false;
                        }
                    } else if arena.data_types.get(value) == EnumToken::MathFunctionTokenType
                        && arena.with_string(arena.string_value.get(value), |n| equals_ignore_case("calc", n))
                    {
                        let tokens = arena.tokens(value);
                        let mut walker = ValueWalker::of_tokens(&tokens, WalkerValueOptions::default(), false);
                        while let Some(v) = walker.next(arena) {
                            if arena.data_types.get(v) == EnumToken::IdenTokenType {
                                let val = arena.resolve_string(arena.string_value.get(v)).to_lowercase();
                                if unexpected_constant(&val, &color_space) {
                                    push_error(&mut errors, format!("Unexpected constant '{}'", val), v);
                                    return false;
                                }
                            }
                        }
                    }
                }
            }
        }
        let tokens = arena.tokens(token);
        let mut walker = ValueWalker::of_tokens(
            &tokens,
            WalkerValueOptions {
                fn_: Some(Box::new(|arena, value, _| {
                    if arena.data_types.get(value) == EnumToken::WildCardFunctionTokenType {
                        WalkerValueResult::Option(WalkerOptionEnum::Ignore | WalkerOptionEnum::IgnoreChildren)
                    } else {
                        WalkerValueResult::None
                    }
                })),
                ..Default::default()
            },
            false,
        );
        while let Some(value) = walker.next(arena) {
            let parent = match arena.get_value_parent(value) {
                0 => token,
                p => p,
            };
            let mut tk = 0;
            let mut tl = 0;
            if arena.data_types.get(value) == EnumToken::BinaryExpressionTokenType {
                tk = arena.field(value, TokenField::L);
                tl = arena.field(value, TokenField::R);
            } else if parent != token
                && arena.data_types.get(parent) == EnumToken::MathFunctionTokenType
                && arena.string_equals(arena.string_value.get(parent), "calc")
            {
                let children = arena.tokens(parent);
                let k = 0;
                let mut l = k + 1;
                while l + 1 < children.len() {
                    let t = arena.data_types.get(children[l]);
                    if matches!(
                        t,
                        EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType | EnumToken::Add | EnumToken::Sub | EnumToken::Div | EnumToken::Mul
                    ) {
                        l += 1;
                        continue;
                    }
                    break;
                }
                tk = children.get(k).copied().unwrap_or(0);
                tl = children.get(l).copied().unwrap_or(0);
            }
            if tk != 0 && tl != 0 {
                let (a, b) = (arena.data_types.get(tk), arena.data_types.get(tl));
                if (a == EnumToken::PercentageTokenType || b == EnumToken::PercentageTokenType) && a != b {
                    push_error(&mut errors, "adding percentage and number is not allowed".into(), token);
                    return false;
                }
            }
        }
        if &*name == "color" {
            let children = filter_children(
                arena,
                token,
                &[
                    EnumToken::DashedIdenTokenType,
                    EnumToken::IdenTokenType,
                    EnumToken::NumberTokenType,
                    EnumToken::LiteralTokenType,
                    EnumToken::ColorTokenType,
                    EnumToken::FunctionTokenType,
                    EnumToken::MathFunctionTokenType,
                    EnumToken::PercentageTokenType,
                ],
            );
            let is_relative = children.first().is_some_and(|&c| {
                arena.data_types.get(c) == EnumToken::IdenTokenType && arena.string_equals(arena.string_value.get(c), "from")
            });
            let offset = if is_relative { 2 } else { 0 };
            if offset < children.len() && arena.data_types.get(children[offset]) == EnumToken::DashedIdenTokenType {
                if children.len() <= offset + 1 {
                    push_error(&mut errors, "Invalid color".into(), token);
                    return false;
                }
                for &child in &children[offset + 1..] {
                    let child_type = arena.data_types.get(child);
                    if matches!(
                        child_type,
                        EnumToken::NumberTokenType
                            | EnumToken::LiteralTokenType
                            | EnumToken::ColorTokenType
                            | EnumToken::FunctionTokenType
                            | EnumToken::MathFunctionTokenType
                            | EnumToken::PercentageTokenType
                    ) || is_color(arena, child, None)
                        || (child_type == EnumToken::IdenTokenType && arena.string_equals_ignore_case(arena.string_value.get(child), "none"))
                    {
                        continue;
                    }
                    return false;
                }
                return true;
            }
            return true;
        } else if &*name == "color-mix" {
            let mut children: Vec<Vec<u32>> = vec![Vec::new()];
            for t in arena.token_iter(token) {
                let tt = arena.data_types.get(t);
                if tt == EnumToken::CommaTokenType {
                    children.push(Vec::new());
                } else if tt != EnumToken::WhitespaceTokenType && tt != EnumToken::CommentTokenType {
                    children.last_mut().unwrap().push(t);
                }
            }
            let j = 0;
            let mut k = 0;
            let first = &children[j];
            let s = |t: u32| arena.resolve_string(arena.string_value.get(t));
            if !first.is_empty()
                && arena.data_types.get(first[0]) == EnumToken::IdenTokenType
                && equals_ignore_case("in", &s(first[k]))
            {
                k += 1;
                if k < first.len() && arena.data_types.get(first[k]) == EnumToken::IdenTokenType {
                    if !is_rectangular_orthogonal_colorspace(arena, first[k]) {
                        let polar = is_polar_colorspace(arena, first[k]);
                        k += 1;
                        if polar {
                            if k == first.len() {
                            } else if arena.data_types.get(first[k]) != EnumToken::IdenTokenType {
                                return false;
                            } else if equals_ignore_case("hue", &s(first[k])) {
                            } else {
                                match &*s(first[k]) {
                                    "increasing" | "decreasing" | "longer" | "shorter" => k += 1,
                                    _ => return false,
                                }
                                if k >= first.len()
                                    || arena.data_types.get(first[k]) != EnumToken::IdenTokenType
                                    || !equals_ignore_case("hue", &s(first[k]))
                                {
                                    return false;
                                }
                            }
                        }
                    }
                }
            }
            return true;
        }
        return true;
    }
    false
}

/// parse a color token: set the color type of identifiers and color functions
pub fn parse_color(arena: &AstArena, token: u32) -> u32 {
    let typ = arena.data_types.get(token);
    if typ == EnumToken::IdenTokenType {
        let val = arena.resolve_string(arena.string_value.get(token)).to_lowercase();
        if non_standard_colors().contains(&val) {
            arena.data_types.set(token, EnumToken::ColorTokenType);
            arena.color_type.set(token, ColorType::NON_STD);
            return token;
        }
        if SYSTEM_COLORS.contains(&val.as_str()) {
            arena.data_types.set(token, EnumToken::ColorTokenType);
            arena.color_type.set(token, ColorType::SYS);
            return token;
        }
        if DEPRECATED_SYSTEM_COLORS.contains(&val.as_str()) {
            arena.data_types.set(token, EnumToken::ColorTokenType);
            arena.color_type.set(token, ColorType::DPSYS);
            return token;
        }
        if color_name_to_hex(&val).is_some() || val == "currentcolor" {
            arena.data_types.set(token, EnumToken::ColorTokenType);
            arena.string_value.set(token, arena.intern_string(&val));
            arena.color_type.set(token, ColorType::LIT);
            return token;
        }
    }
    if typ == EnumToken::ColorTokenType {
        let val = arena.resolve_string(arena.string_value.get(token));
        if arena.color_type.get(token) == ColorType::NONE && arena.string_value.get(token) != 0 {
            arena.color_type.set(token, color_type_of(&val));
        }
        if arena.first_token.get(token) != 0 {
            let tk = arena.find_token(token, |arena, t| {
                let tt = arena.data_types.get(t);
                tt != EnumToken::WhitespaceTokenType && tt != EnumToken::CommentTokenType
            });
            if let Some(tk) = tk {
                if arena.data_types.get(tk) == EnumToken::IdenTokenType && arena.string_equals(arena.string_value.get(tk), "from") {
                    arena.unit.set(token, arena.intern_string("rel"));
                } else if &*val == "color-mix" && arena.string_equals(arena.string_value.get(tk), "in") {
                    arena.unit.set(token, arena.intern_string("mix"));
                }
                if &*val == "color" && arena.data_types.get(tk) == EnumToken::DashedIdenTokenType {
                    arena.color_type.set(token, ColorType::CUSTOM_COLOR);
                }
            }
        }
    }
    token
}

#[inline]
pub fn is_letter(codepoint: u32) -> bool {
    (0x61..=0x7a).contains(&codepoint) || (0x41..=0x5a).contains(&codepoint)
}

#[inline]
fn is_non_ascii(codepoint: u32) -> bool {
    codepoint >= 0x80
}

#[inline]
pub fn is_ident_start(codepoint: u32) -> bool {
    // u32::MAX stands for `undefined` / NaN
    codepoint != u32::MAX && (codepoint == 0x5f || is_letter(codepoint) || is_non_ascii(codepoint) || codepoint == REVERSE_SOLIDUS)
}

#[inline]
pub fn is_digit(codepoint: u32) -> bool {
    (0x30..=0x39).contains(&codepoint)
}

#[inline]
pub fn is_ident_codepoint(codepoint: u32) -> bool {
    codepoint == 0x2d || is_digit(codepoint) || is_ident_start(codepoint)
}

/// true if the name is an identifier (over the utf-16 code units, like the typescript implementation)
pub fn is_ident(name: &str) -> bool {
    let units: Vec<u16> = if name.is_ascii() { name.bytes().map(|b| b as u16).collect() } else { name.encode_utf16().collect() };
    is_ident_units(&units)
}

/// [`is_ident`] over utf-16 code units
pub fn is_ident_units(name: &[u16]) -> bool {
    let at = |i: usize| name.get(i).map_or(u32::MAX, |&c| c as u32);
    let j: isize = name.len() as isize - 1;
    let mut i: isize = 0;
    let mut codepoint = at(0);
    if codepoint == 0x2d {
        let next = at(1);
        if next == u32::MAX || is_digit(next) {
            return false;
        }
        codepoint = next;
        i = 1;
    }
    if codepoint != 0x2d && !is_ident_start(codepoint) {
        return false;
    }
    if codepoint == REVERSE_SOLIDUS {
        if i + 1 > j {
            return false;
        }
        i += 1;
    }
    while i < j {
        i += 1;
        codepoint = at(i as usize);
        if codepoint == REVERSE_SOLIDUS {
            i += 1;
            i += 1;
            continue;
        }
        if codepoint != 0x2d && !is_ident_codepoint(codepoint) {
            return false;
        }
    }
    true
}

#[inline]
pub fn is_non_printable(codepoint: u32) -> bool {
    codepoint <= 0x8 || codepoint == 0xb || codepoint == 0x7f || (0xe..=0x1f).contains(&codepoint)
}

/// `:name`, `::name`, `:name(`
pub fn is_pseudo(name: &str) -> bool {
    if !name.starts_with(':') {
        return false;
    }
    let skip = if name[1..].starts_with(':') { 2 } else { 1 };
    (name.ends_with('(') && name.len() > skip && is_ident(&name[skip..name.len() - 1])) || is_ident(&name[skip..])
}

pub fn is_hash(name: &str) -> bool {
    name.starts_with('#') && is_ident_start(name.encode_utf16().nth(1).map_or(u32::MAX, |c| c as u32))
}

/// result of [`parse_dimension`]
#[derive(Clone, Debug, PartialEq)]
pub struct Dimension {
    pub typ: u8,
    pub val: f64,
    pub unit: String,
}

/// parse a dimension (`10px`)
pub fn parse_dimension(name: &str) -> Option<Dimension> {
    let bytes = name.as_bytes();
    let mut index = bytes.len();
    while index > 0 && is_letter(bytes[index - 1] as u32) {
        index -= 1;
    }
    // every character is a letter (the typescript index reaches -1)
    if index == 0 {
        return None;
    }
    let unit = &name[index..];
    let val = crate::utils::number::js_parse_number(&name[..index]);
    if val.is_nan() {
        return None;
    }
    let unit = validation_units().iter().find(|u| equals_ignore_case(u, unit)).cloned().unwrap_or_else(|| unit.to_lowercase());
    let typ = if is_angle_unit(&unit) {
        EnumToken::AngleTokenType
    } else if is_length_unit(&unit) {
        EnumToken::LengthTokenType
    } else if is_time_unit(&unit) {
        EnumToken::TimeTokenType
    } else if is_resolution_unit(&unit) {
        EnumToken::ResolutionTokenType
    } else if is_frequency_unit(&unit) {
        EnumToken::FrequencyTokenType
    } else if is_flex_unit(&unit) {
        EnumToken::FlexTokenType
    } else {
        EnumToken::DimensionTokenType
    };
    Some(Dimension { typ, val, unit })
}

pub fn is_function(name: &str) -> bool {
    name.ends_with('(') && is_ident(&name[..name.len() - 1])
}

#[inline]
pub fn is_new_line(codepoint: u32) -> bool {
    matches!(codepoint, 0xa | 0xb | 0xc | 0xd | 0x2028 | 0x2029)
}

#[inline]
pub fn is_white_space(codepoint: u32) -> bool {
    matches!(codepoint, 0x9 | 0x20 | 0xa | 0xb | 0xc | 0xd | 0x2028 | 0x2029)
}

/// convert a length to px
pub fn length2_px(arena: &AstArena, value: u32) -> Option<f64> {
    let val = arena.numeric_value.get(value);
    let result = if arena.data_types.get(value) == EnumToken::NumberTokenType {
        Some(val)
    } else {
        arena.with_string(arena.unit.get(value), |unit| match unit.to_lowercase().as_str() {
            "cm" => Some((val * 96.0) / 2.54),
            "mm" => Some((val * 96.0) / 25.4),
            "q" => Some((val * 96.0) / 101.6),
            "in" => Some(val * 96.0),
            "pc" => Some(val * 16.0),
            "pt" => Some((val * 4.0) / 3.0),
            "px" => Some(val),
            _ => None,
        })
    };
    result.filter(|r| !r.is_nan())
}

thread_local! {
    /// minified numbers, see [`minify_number`]
    static MINIFIED_NUMBERS: RefCell<HashMap<u64, String>> = RefCell::new(HashMap::default());
}

const MINIFIED_NUMBERS_SIZE: usize = 4096;

/// run a function with the minified number: the cached string is borrowed (see [`minify_number`])
pub fn with_minified_number<R>(val: f64, f: impl FnOnce(&str) -> R) -> R {
    MINIFIED_NUMBERS.with(|cache| {
        if let Some(result) = cache.borrow().get(&val.to_bits()) {
            return f(result);
        }
        let result = minify_number_value(val);
        let output = f(&result);
        let mut cache = cache.borrow_mut();
        if cache.len() >= MINIFIED_NUMBERS_SIZE {
            cache.clear();
        }
        cache.insert(val.to_bits(), result);
        output
    })
}

/// minify a number
pub fn minify_number(val: f64) -> String {
    MINIFIED_NUMBERS.with(|cache| {
        if let Some(result) = cache.borrow().get(&val.to_bits()) {
            return result.clone();
        }
        let result = minify_number_value(val);
        let mut cache = cache.borrow_mut();
        if cache.len() >= MINIFIED_NUMBERS_SIZE {
            cache.clear();
        }
        cache.insert(val.to_bits(), result.clone());
        result
    })
}

/// minify a number given as a string
pub fn minify_number_str(val: &str) -> String {
    minify_number_value(crate::utils::number::js_parse_number(val))
}

/// minify a number
fn minify_number_value(val: f64) -> String {
    let val = js_number_to_string(to_precision_value(val, colorPrecision));
    if val == "0" {
        return "0".into();
    }
    if let Some(rest) = val.strip_prefix("-0") {
        return if rest.is_empty() { "0".into() } else { format!("-{}", rest) };
    }
    if let Some(rest) = val.strip_prefix('0') {
        return rest.into();
    }
    val
}

/// round a value to a precision (default `colorPrecision`)
pub fn to_precision_value(value: f64, precision: i32) -> f64 {
    let div = 10f64.powi(precision);
    let value = js_round(value * div) / div;
    if value.abs() < epsilon {
        0.0
    } else {
        value
    }
}

/// round an angle (default precision `anglePrecision`, corrected)
pub fn to_precision_angle(angle: f64, precision: i32, correct_value: bool) -> f64 {
    let mut angle = to_precision_value(angle, precision);
    if correct_value && angle.abs() >= 360.0 {
        angle %= 360.0;
    }
    if correct_value && angle < 0.0 {
        angle += 360.0;
    }
    angle
}

/// `toPrecisionAngle(angle)` with the default arguments
pub fn to_precision_angle_default(angle: f64) -> f64 {
    to_precision_angle(angle, anglePrecision, true)
}
