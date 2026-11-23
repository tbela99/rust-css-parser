//! Port of `src/syntax/color/color-mix.ts`.
//!
//! The mixed values are javascript arrays that may hold `undefined` (`color[3]` of an opaque color): `Option<f64>`.

use super::color::{get_number, minmax};
use super::hsl::srgb2hslvalues;
use super::hwb::{hwbvalues, srgb2hwb};
use super::lab::srgb2labvalues;
use super::lch::{get_lch_components, srgb2lch, xyz2lchvalues};
use super::oklab::srgb2oklab;
use super::oklch::{get_oklch_components, srgb2oklch};
use super::p3::{srgb2lp3values, srgb2p3values};
use super::prophotorgb::srgb2prophotorgbvalues;
use super::rec2020::srgb2rec2020values;
use super::a98rgb::srgb2a98values;
use super::srgb::{hslvalues, srgb2lsrgbvalues, srgbvalues};
use super::utils::components::get_color_components;
use super::utils::values::{at, opt};
use super::xyz::srgb2xyz_d65;
use super::xyzd50::{xyz_d65_to_d50, xyzd502lch};
use crate::ast::arena::{AstArena, TokenValue};
use crate::ast::types::{ColorType, EnumAstNodeStatus, EnumToken};
use crate::parser::utils::text::equals_ignore_case;
use crate::syntax::syntax::{is_polar_colorspace, is_rectangular_orthogonal_colorspace};

type JsValues = Vec<Option<f64>>;

fn interpolate_hue(method: &str, mut h1: f64, mut h2: f64) -> (f64, f64) {
    match method {
        "longer" => {
            if h2 - h1 < 180.0 && h2 - h1 > 0.0 {
                h1 += 360.0;
            } else if h2 - h1 <= 0.0 && h2 - h1 > -180.0 {
                h2 += 360.0;
            }
        }
        "increasing" => {
            if h2 < h1 {
                h2 += 360.0;
            }
        }
        "decreasing" => {
            if h2 > h1 {
                h1 += 360.0;
            }
        }
        _ => {
            // shorter
            if h2 - h1 > 180.0 {
                h1 += 360.0;
            } else if h2 - h1 < -180.0 {
                h2 += 360.0;
            }
        }
    }
    (h1, h2)
}

/// `array[i] = value` (the array grows with holes)
fn set(values: &mut JsValues, i: usize, value: Option<f64>) {
    if values.len() <= i {
        values.resize(i + 1, None);
    }
    values[i] = value;
}

fn get(values: &JsValues, i: usize) -> Option<f64> {
    values.get(i).copied().flatten()
}

struct Mix {
    color: JsValues,
    alpha: f64,
}

/// compute a color mix
pub fn color_mix(arena: &AstArena, args: &[u32]) -> Option<u32> {
    let typ = |i: usize| args.get(i).map_or(0, |&t| arena.data_types.get(t));
    let s = |i: usize| args.get(i).map_or_else(String::new, |&t| arena.resolve_string(arena.string_value.get(t)).to_string());
    // invalid color or custom color profile
    if args.is_empty() || typ(0) == EnumToken::DashedIdenTokenType {
        return None;
    }
    let mut i = 0usize;
    let mut missing_percentage_count = 0usize;
    let mut total_percentage = 0.0f64;
    let mut left_over_percentage = 0.0f64;
    let mut color_space = String::from("oklab");
    let mut hue_interpolation_method = String::from("shorter");
    let mut percentages: Vec<Option<f64>> = Vec::new();
    let mut srgb_component_values: Vec<JsValues> = Vec::new();
    let mut color_components: Vec<Vec<u32>> = Vec::new();
    let mut colors: Vec<u32> = Vec::new();
    if typ(i) == EnumToken::IdenTokenType {
        let v = s(i);
        i += 1;
        if !equals_ignore_case(&v, "in") {
            return None;
        }
        if typ(i) != EnumToken::IdenTokenType {
            return None;
        }
        if is_rectangular_orthogonal_colorspace(arena, args[i]) {
            color_space = s(i);
            i += 1;
        } else if is_polar_colorspace(arena, args[i]) {
            color_space = s(i);
            i += 1;
            if typ(i) != EnumToken::IdenTokenType && typ(i) != EnumToken::ColorTokenType {
                return None;
            }
            if typ(i) == EnumToken::IdenTokenType && !equals_ignore_case(&s(i), "hue") {
                hue_interpolation_method = s(i);
                i += 1;
                if !matches!(hue_interpolation_method.as_str(), "increasing" | "decreasing" | "longer" | "shorter") {
                    return None;
                }
                let v = s(i);
                i += 1;
                if !equals_ignore_case(&v, "hue") {
                    return None;
                }
            }
        }
    }
    while i < args.len() {
        let kin = arena.color_type.get(args[i]);
        if typ(i) != EnumToken::ColorTokenType
            || matches!(kin, ColorType::SYS | ColorType::DPSYS | ColorType::NON_STD | ColorType::CUSTOM_COLOR)
            || equals_ignore_case(&s(i), "currentcolor")
        {
            return None;
        }
        // typescript: a `null` list throws when it is read
        color_components.push(get_color_components(arena, args[i]).unwrap_or_default());
        let values = srgbvalues(arena, args[i])?;
        colors.push(process_powerless_components(arena, args[i]));
        let (r, g, b, a) = (at(&values, 0), at(&values, 1), at(&values, 2), opt(&values, 3));
        let values: Vec<f64> = match color_space.as_str() {
            "srgb" => values,
            "display-p3" => srgb2p3values(r, g, b, a),
            "display-p3-linear" => srgb2lp3values(r, g, b, a),
            "a98-rgb" => srgb2a98values(r, g, b, a),
            "prophoto-rgb" => srgb2prophotorgbvalues(r, g, b, a),
            "srgb-linear" => srgb2lsrgbvalues(r, g, b, a),
            "rec2020" => srgb2rec2020values(r, g, b, a),
            "xyz" | "xyz-d65" => srgb2xyz_d65(r, g, b, a),
            "xyz-d50" => {
                let v = srgb2xyz_d65(r, g, b, a);
                xyz_d65_to_d50(at(&v, 0), at(&v, 1), at(&v, 2), opt(&v, 3))
            }
            "hsl" => srgb2hslvalues(r, g, b, a),
            "hwb" => srgb2hwb(r, g, b, a),
            "lab" => srgb2labvalues(r, g, b, a),
            "lch" => srgb2lch(r, g, b, a),
            "oklab" => srgb2oklab(r, g, b, a),
            "oklch" => srgb2oklch(r, g, b, a),
            _ => return None,
        };
        srgb_component_values.push(values.into_iter().map(Some).collect());
        i += 1;
        if i >= args.len() {
            missing_percentage_count += 1;
            percentages.push(None);
            break;
        }
        if typ(i) == EnumToken::ColorTokenType {
            percentages.push(None);
            missing_percentage_count += 1;
            continue;
        }
        if typ(i) == EnumToken::PercentageTokenType || (typ(i) == EnumToken::NumberTokenType && arena.numeric_value.get(args[i]) == 0.0) {
            if arena.numeric_value.get(args[i]) < 0.0 {
                return None;
            }
            let p = minmax(get_number(arena, args[i]), 0.0, 1.0);
            i += 1;
            percentages.push(Some(p));
            total_percentage += p;
        }
    }
    // normalize percentages
    if missing_percentage_count > 0 {
        let normalized = if total_percentage > 1.0 { 0.0 } else { 1.0 - total_percentage };
        for p in percentages.iter_mut() {
            if p.is_none() {
                *p = Some(normalized / missing_percentage_count as f64);
            }
        }
        total_percentage = percentages.iter().map(|p| p.unwrap_or(f64::NAN)).sum();
    }
    if total_percentage != 1.0 {
        if total_percentage < 1.0 {
            left_over_percentage = 1.0 - total_percentage;
        }
        let perc = if total_percentage == 0.0 { 1.0 } else { total_percentage };
        // scale down percentages
        for p in percentages.iter_mut() {
            *p = Some(p.unwrap_or(f64::NAN) / perc);
        }
    }
    let is_lch_or_oklch = color_space == "lch" || color_space == "oklch";
    let mut stack: Vec<Mix> = (0..srgb_component_values.len())
        .rev()
        .map(|i| Mix { color: srgb_component_values[i].clone(), alpha: percentages.get(i).copied().flatten().unwrap_or(f64::NAN) })
        .collect();
    let comp = |ci: usize, idx: usize| color_components.get(ci).and_then(|c| c.get(idx)).copied();
    let is_none_iden = |t: Option<u32>| {
        t.is_some_and(|t| arena.data_types.get(t) == EnumToken::IdenTokenType && &*arena.resolve_string(arena.string_value.get(t)) == "none")
    };
    let powerless = |t: Option<u32>| t.map_or(0, |t| arena.flags.get(t)) & EnumAstNodeStatus::PowerlessColorComponent != 0;
    let l_max = |color: u32| if arena.color_type.get(color) == ColorType::LCH { 100.0 } else { 1.0 };
    let mut current_index = 0usize;
    let mut offset1 = if arena.color_type.get(colors[0]) == ColorType::COLOR { 1 } else { 0 };
    if comp(0, offset1 + 3).is_some() && is_none_iden(comp(0, offset1 + 3)) && color_components.get(1).is_some_and(|c| c.len() == offset1 + 4) {
        // fix powerless alpha for last color if previous color has alpha
        let n = stack.len();
        let v = get(&stack[n - 2].color, 3);
        set(&mut stack[n - 1].color, 3, v);
    }
    if stack.len() > 1 {
        let n = stack.len();
        // powerless hue for lch spaces
        if (get(&stack[n - 1].color, 0) == Some(0.0) && is_none_iden(comp(0, offset1))) || powerless(comp(0, 0)) {
            let v = get(&stack[n - 2].color, 0);
            set(&mut stack[n - 1].color, 0, v);
        }
        if (get(&stack[n - 1].color, 1) == Some(0.0) && is_none_iden(comp(0, offset1 + 1))) || powerless(comp(0, 1)) {
            let v = get(&stack[n - 2].color, 1);
            set(&mut stack[n - 1].color, 1, v);
        }
        let last = &stack[n - 1].color;
        if (get(last, 2) == Some(0.0) && is_none_iden(comp(0, offset1 + 2)))
            || powerless(comp(0, 2))
            || (is_lch_or_oklch
                && (get(last, 1) == Some(0.0) || get(last, 0) == Some(0.0) || get(last, 0) == Some(l_max(colors[0]))))
        {
            let v = get(&stack[n - 2].color, 2);
            set(&mut stack[n - 1].color, 2, v);
        }
    }
    while stack.len() > 1 {
        let mut r1 = stack.pop().unwrap();
        let mut r2 = stack.pop().unwrap();
        current_index += 1;
        let color = colors.get(current_index).copied().unwrap_or(0);
        offset1 = if arena.color_type.get(color) == ColorType::COLOR { 1 } else { 0 };
        let ci = current_index;
        // r2 powerless alpha
        if comp(ci, offset1 + 3).is_some() && is_none_iden(comp(ci, offset1 + 3)) {
            set(&mut r2.color, 3, get(&r1.color, 3));
        }
        // powerless hue for lch spaces
        if (get(&r2.color, 0) == Some(0.0) && is_none_iden(comp(ci, offset1))) || powerless(comp(ci, offset1)) {
            set(&mut r2.color, 0, get(&r1.color, 0));
        }
        if (get(&r2.color, 1) == Some(0.0) && is_none_iden(comp(ci, offset1 + 1))) || powerless(comp(ci, offset1 + 1)) {
            set(&mut r2.color, 1, get(&r1.color, 1));
        }
        if (get(&r2.color, 2) == Some(0.0) && is_none_iden(comp(ci, offset1 + 2)))
            || powerless(comp(ci, offset1 + 2))
            || (is_lch_or_oklch
                && (get(&r2.color, 1) == Some(0.0) || get(&r2.color, 0) == Some(0.0) || get(&r2.color, 0) == Some(l_max(color))))
        {
            set(&mut r2.color, 2, get(&r1.color, 2));
        }
        {
            let (hue_index, multiplier) = if color_space == "hwb" || color_space == "hsl" { (0, 360.0) } else { (2, 1.0) };
            let (h1, h2) = interpolate_hue(
                &hue_interpolation_method,
                get(&r1.color, hue_index).unwrap_or(f64::NAN) * multiplier,
                get(&r2.color, hue_index).unwrap_or(f64::NAN) * multiplier,
            );
            set(&mut r1.color, hue_index, Some(h1 / multiplier));
            set(&mut r2.color, hue_index, Some(h2 / multiplier));
        }
        let mult1 = get(&r1.color, 3).unwrap_or(1.0);
        let mult2 = get(&r2.color, 3).unwrap_or(1.0);
        let mult = mult1 * r1.alpha + mult2 * r2.alpha;
        let c = |m: &Mix, i: usize, k: f64| get(&m.color, i).unwrap_or(f64::NAN) * k * m.alpha;
        let mixed = [c(&r1, 0, mult1) + c(&r2, 0, mult2), c(&r1, 1, mult1) + c(&r2, 1, mult2), c(&r1, 2, mult1) + c(&r2, 2, mult2)];
        stack.push(if mult == 0.0 {
            Mix { color: vec![Some(0.0), Some(0.0), Some(0.0), Some(0.0)], alpha: 0.0 }
        } else {
            Mix { color: vec![Some(mixed[0] / mult), Some(mixed[1] / mult), Some(mixed[2] / mult)], alpha: mult }
        });
    }
    let result = stack.pop()?;
    let mut values: Vec<f64> = (0..3).map(|i| get(&result.color, i).unwrap_or(f64::NAN)).collect();
    let alpha = result.alpha * (1.0 - left_over_percentage);
    if alpha != 1.0 {
        values.push(alpha);
    }
    let number = |v: f64| arena.create_token(EnumToken::NumberTokenType, TokenValue::Num(v), None);
    match color_space.as_str() {
        "xyz" | "xyz-d65" | "xyz-d50" => {
            let values = if color_space == "xyz-d50" {
                xyzd502lch(values[0], values[1], values[2], opt(&values, 3))
            } else {
                xyz2lchvalues(values[0], values[1], values[2], opt(&values, 3))
            };
            let chi: Vec<u32> = values.iter().map(|&v| number(v)).collect();
            Some(arena.create_color("lch", ColorType::LCH, Some(&chi)))
        }
        "srgb" | "srgb-linear" | "a98-rgb" | "rec2020" | "display-p3" | "display-p3-linear" | "prophoto-rgb" => {
            let mut chi = vec![arena.create_token(EnumToken::IdenTokenType, TokenValue::Str(&color_space), None)];
            chi.extend(values.iter().map(|&v| number(v)));
            let color = arena.create_color("color", ColorType::COLOR, Some(&chi));
            arena.set_unit_string(color, Some("col"));
            Some(color)
        }
        "hsl" | "hwb" | "lab" | "lch" | "oklab" | "oklch" => {
            if color_space == "hsl" || color_space == "hwb" {
                if values[2] < 0.0 {
                    values[2] += 360.0;
                }
            } else if color_space == "lch" || color_space == "oklch" {
                if values[2] < 0.0 {
                    values[2] += 360.0;
                } else if values[2] > 360.0 {
                    values[2] %= 360.0;
                }
            }
            let components: Vec<u32> = values.iter().map(|&v| number(v)).collect();
            if color_space == "hsl" {
                arena.data_types.set(components[0], EnumToken::AngleTokenType);
                arena.set_unit_string(components[0], Some("deg"));
                arena.data_types.set(components[1], EnumToken::PercentageTokenType);
                arena.data_types.set(components[2], EnumToken::PercentageTokenType);
            }
            let kin = ColorType::value(&color_space.to_uppercase().replace('-', "_")).unwrap_or(0);
            Some(arena.create_color(&color_space, kin, Some(&components)))
        }
        _ => None,
    }
}

fn process_powerless_components(arena: &AstArena, color: u32) -> u32 {
    let kin = arena.color_type.get(color);
    match kin {
        ColorType::LCH | ColorType::OKLCH => {
            let values = if kin == ColorType::LCH { get_lch_components(arena, color) } else { get_oklch_components(arena, color) };
            let components = get_color_components(arena, color);
            let (Some(mut values), Some(components)) = (values, components) else {
                return color;
            };
            if values.len() < 3 {
                return color;
            }
            let max = if kin == ColorType::LCH { 100.0 } else { 1.0 };
            // https://www.w3.org/TR/css-color-4/#specifying-lab-lch
            let epsilon = if kin == ColorType::LCH { 0.0015 } else { 0.000004 };
            let none = |t: u32| arena.reset_token(t, EnumToken::IdenTokenType, TokenValue::Str("none"), None);
            let flag = |t: u32| arena.flags.set(t, arena.flags.get(t) | EnumAstNodeStatus::PowerlessColorComponent);
            if values[0] < epsilon || (values[0] - max).abs() < epsilon {
                arena.reset_token(components[0], EnumToken::NumberTokenType, TokenValue::Num(if values[0] < epsilon { 0.0 } else { max }), None);
                none(components[1]);
                none(components[2]);
                flag(components[1]);
                flag(components[2]);
            } else if values[1] < epsilon {
                values[1] = 0.0;
                arena.reset_token(components[1], EnumToken::NumberTokenType, TokenValue::Num(0.0), None);
                none(components[2]);
                flag(components[2]);
            }
        }
        ColorType::HSL | ColorType::HWB => {
            // `Object.values(hslvalues() ?? {})`
            let values: Vec<f64> = if kin == ColorType::HSL {
                hslvalues(arena, color).map_or_else(Vec::new, |v| [Some(v.h), Some(v.s), Some(v.l), v.a].into_iter().flatten().collect())
            } else {
                hwbvalues(arena, color).map_or_else(Vec::new, |v| [Some(v.h), Some(v.w), Some(v.b), v.a].into_iter().flatten().collect())
            };
            let Some(components) = get_color_components(arena, color) else {
                return color;
            };
            if values.len() < 3 {
                return color;
            }
            // https://www.w3.org/TR/css-color-4/#the-hsl-notation
            if (kin == ColorType::HSL && values[1] <= 0.001) || (kin == ColorType::HWB && values[1] + values[2] >= 99.999) {
                arena.reset_token(components[0], EnumToken::IdenTokenType, TokenValue::Str("none"), None);
                arena.flags.set(components[0], arena.flags.get(components[0]) | EnumAstNodeStatus::PowerlessColorComponent);
            }
        }
        _ => {}
    }
    color
}
