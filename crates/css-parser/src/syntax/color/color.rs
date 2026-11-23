//! Port of `src/syntax/color/color.ts`.

use super::a98rgb::{a98rgb2srgbvalues, srgb2a98values};
use super::alpha::alpha;
use super::cmyk::*;
use super::color_mix::color_mix;
use super::hex::*;
use super::hsl::*;
use super::hwb::*;
use super::lab::*;
use super::lch::*;
use super::oklab::*;
use super::oklch::*;
use super::p3::{lp32srgbvalues, p32srgbvalues, srgb2lp3values, srgb2p3values};
use super::prophotorgb::{prophotorgb2srgbvalues, srgb2prophotorgbvalues};
use super::rec2020::{rec20202srgb, srgb2rec2020values};
use super::relative_color::parse_relative_color_components;
use super::rgb::*;
use super::srgb::{cmyk2srgbvalues, hex2srgbvalues, hsl2srgb, hwb2srgbvalues, lab2srgbvalues, lch2srgbvalues, lsrgb2srgbvalues, oklab2srgbvalues, oklch2srgbvalues, rgb2srgb, srgb2lsrgbvalues, xyz2srgb};
use super::utils::components::get_color_components;
use super::utils::values::{at, opt};
use super::xyz::{srgb2xyz, srgb2xyz_d65};
use super::xyzd50::xyzd502srgb;
use crate::ast::arena::{AstArena, TokenValue};
use crate::ast::types::{ColorType, EnumToken};
use crate::parser::utils::text::equals_ignore_case;
use crate::syntax::constants::COLOR_FUNC_COLOR_SPACE;
use crate::syntax::syntax::is_ident_color;
use crate::validation::match_::trim_array;

/// `ColorType[to].toLowerCase().replaceAll("_", "-")`
fn color_space_name(to: u8) -> String {
    ColorType::name(to).unwrap_or("").to_lowercase().replace('_', "-")
}

/// convert a color to another color space. `None`: not converted
pub fn convert_color(arena: &AstArena, token: u32, to: u8) -> Option<u32> {
    let mut token = token;
    let s = |t: u32| arena.resolve_string(arena.string_value.get(t)).to_string();
    let kin = arena.color_type.get(token);
    if kin == ColorType::SYS || kin == ColorType::DPSYS || (is_ident_color(arena, token) && s(token) == "currentcolor") {
        return Some(token);
    }
    if arena.color_type.get(token) == ColorType::ALPHA {
        let mut args: Vec<u32> = arena
            .tokens(token)
            .into_iter()
            .filter(|&t| !matches!(arena.data_types.get(t), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType))
            .collect();
        let n = args.len();
        if n > 1 && arena.data_types.get(args[n - 2]) == EnumToken::LiteralTokenType && s(args[n - 2]) == "/" {
            args.remove(n - 2);
        }
        let mut values: Vec<u32> = args.get(1..).unwrap_or(&[]).to_vec();
        trim_array(arena, &mut values);
        token = alpha(arena, values.first().copied().unwrap_or(0), values.get(1).copied())?;
    }
    if arena.color_type.get(token) == ColorType::COLOR_MIX && to != ColorType::COLOR_MIX {
        // `children.flat(2)`: the tokens without the commas, whitespaces and comments
        let children: Vec<u32> = arena
            .tokens(token)
            .into_iter()
            .filter(|&t| !matches!(arena.data_types.get(t), EnumToken::CommaTokenType | EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType))
            .collect();
        token = color_mix(arena, &children)?;
    }
    if &*arena.resolve_string(arena.unit.get(token)) == "rel"
        && ["rgb", "hsl", "hwb", "lab", "lch", "oklab", "oklch", "color"].iter().any(|t| equals_ignore_case(t, &s(token)))
    {
        if let Some(chi) = get_color_components(arena, token) {
            let name = s(token);
            let offset = if name == "color" { 2 } else { 1 };
            let c = |i: usize| chi.get(i).copied().unwrap_or(0);
            let color = c(1);
            let keys = if equals_ignore_case(&name, "color") { s(c(offset)) } else { name.clone() };
            let components = parse_relative_color_components(arena, &keys, color, c(offset + 1), c(offset + 2), c(offset + 3), c(offset + 4))?;
            let kin = ColorType::value(&name.to_uppercase().replace('-', "_")).unwrap_or(0);
            let initial = if name == "color" { vec![arena.clone_node_deep(c(offset), false, None)] } else { Vec::new() };
            let tk = arena.create_color(&name, kin, Some(&initial));
            let mut seen: Vec<u32> = Vec::new();
            for (_, t) in components {
                if t == 0 {
                    continue;
                }
                // a component token may be used more than once: rgb(from #ff0 r g r)
                let token = if seen.contains(&t) || arena.parents.get(t) != 0 { arena.clone_node_deep(t, true, None) } else { t };
                arena.append_token(tk, token);
                seen.push(t);
            }
            arena.copy_location(tk, token);
            token = tk;
        }
    }
    let kin = arena.color_type.get(token);
    if kin == to {
        if kin == ColorType::HEX || kin == ColorType::LIT {
            let val = reduce_hex_value(&s(token));
            arena.string_value.set(token, arena.intern_string(&val));
            arena.color_type.set(token, if val.starts_with('#') { ColorType::HEX } else { ColorType::LIT });
        }
        return Some(token);
    }
    if kin == ColorType::COLOR {
        let color_space = arena.find_token(token, |a, t| !matches!(a.data_types.get(t), EnumToken::WhitespaceTokenType | EnumToken::CommentTokenType));
        if let Some(color_space) = color_space {
            if s(color_space) == color_space_name(to) {
                for chi in arena.tokens(token) {
                    if arena.data_types.get(chi) == EnumToken::NumberTokenType && !arena.has_fraction(chi) {
                        arena.numeric_value.set(chi, get_number(arena, chi));
                    }
                }
                return Some(token);
            }
        }
    }
    match to {
        ColorType::HSL => match kin {
            ColorType::RGB => rgb2hsl_token(arena, token),
            ColorType::HEX | ColorType::LIT => hex2hsl_token(arena, token),
            ColorType::HWB => hwb2hsl_token(arena, token),
            ColorType::DEVICE_CMYK => cmyk2hsl_token(arena, token),
            ColorType::OKLAB => oklab2hsl_token(arena, token),
            ColorType::OKLCH => oklch2hsl_token(arena, token),
            ColorType::LAB => lab2hsl_token(arena, token),
            ColorType::LCH => lch2hsl_token(arena, token),
            ColorType::COLOR => color2hsl_token(arena, token),
            _ => None,
        },
        ColorType::HWB => match kin {
            ColorType::RGB | ColorType::HEX | ColorType::LIT => rgb2hwb_token(arena, token),
            ColorType::HSL => hsl2hwb_token(arena, token),
            ColorType::OKLAB => oklab2hwb_token(arena, token),
            ColorType::OKLCH => oklch2hwb_token(arena, token),
            ColorType::LAB => lab2hwb_token(arena, token),
            ColorType::LCH => lch2hwb_token(arena, token),
            ColorType::DEVICE_CMYK => cmyk2hwb_token(arena, token),
            ColorType::COLOR => color2hwb_token(arena, token),
            _ => None,
        },
        ColorType::DEVICE_CMYK => match kin {
            ColorType::RGB | ColorType::HEX | ColorType::LIT => rgb2cmyk_token(arena, token),
            ColorType::HSL => hsl2cmyk_token(arena, token),
            ColorType::HWB => hwb2cmyk_token(arena, token),
            ColorType::OKLAB => oklab2cmyk(arena, token),
            ColorType::OKLCH => oklch2cmyk_token(arena, token),
            ColorType::LAB => lab2cmyk_token(arena, token),
            ColorType::LCH => lch2cmyk_token(arena, token),
            ColorType::COLOR => color2cmyk_token(arena, token),
            _ => None,
        },
        ColorType::HEX | ColorType::LIT => match kin {
            ColorType::HEX | ColorType::LIT => {
                let val = reduce_hex_value(&s(token));
                Some(arena.create_color(&val, if val.starts_with('#') { ColorType::HEX } else { ColorType::LIT }, None))
            }
            ColorType::HSL => hsl2hex_token(arena, token),
            ColorType::HWB => hwb2hex_token(arena, token),
            ColorType::DEVICE_CMYK => cmyk2hex_token(arena, token),
            ColorType::OKLAB => oklab2hex_token(arena, token),
            ColorType::OKLCH => oklch2hex_token(arena, token),
            ColorType::LAB => lab2hex_token(arena, token),
            ColorType::LCH => lch2hex_token(arena, token),
            ColorType::COLOR => color2hex_token(arena, token),
            ColorType::RGB => rgb2hex_token(arena, token),
            _ => None,
        },
        ColorType::RGB => match kin {
            ColorType::HEX | ColorType::LIT => hex2rgb_token(arena, token),
            ColorType::HSL => hsl2rgb_token(arena, token),
            ColorType::HWB => hwb2rgb_token(arena, token),
            ColorType::DEVICE_CMYK => cmyk2rgb_token(arena, token),
            ColorType::OKLAB => oklab2rgb_token(arena, token),
            ColorType::OKLCH => oklch2rgb_token(arena, token),
            ColorType::LAB => lab2rgb_token(arena, token),
            ColorType::LCH => lch2rgb_token(arena, token),
            ColorType::COLOR => color2rgb_token(arena, token),
            _ => None,
        },
        ColorType::LAB => match kin {
            ColorType::HEX | ColorType::LIT => hex2lab_token(arena, token),
            ColorType::RGB => rgb2lab_token(arena, token),
            ColorType::HSL => hsl2lab_token(arena, token),
            ColorType::HWB => hwb2lab_token(arena, token),
            ColorType::DEVICE_CMYK => cmyk2lab_token(arena, token),
            ColorType::LCH => lch2lab_token(arena, token),
            ColorType::OKLAB => oklab2lab_token(arena, token),
            ColorType::OKLCH => oklch2lab_token(arena, token),
            ColorType::COLOR => color2lab_token(arena, token),
            _ => None,
        },
        ColorType::LCH => match kin {
            ColorType::HEX | ColorType::LIT => hex2lch_token(arena, token),
            ColorType::RGB => rgb2lch_token(arena, token),
            ColorType::HSL => hsl2lch_token(arena, token),
            ColorType::HWB => hwb2lch_token(arena, token),
            ColorType::DEVICE_CMYK => cmyk2lch_token(arena, token),
            ColorType::LAB => lab2lch_token(arena, token),
            ColorType::OKLAB => oklab2lch_token(arena, token),
            ColorType::OKLCH => oklch2lch_token(arena, token),
            ColorType::COLOR => color2lch_token(arena, token),
            _ => None,
        },
        ColorType::OKLAB => match kin {
            ColorType::HEX | ColorType::LIT => hex2oklab_token(arena, token),
            ColorType::RGB => rgb2oklab_token(arena, token),
            ColorType::HSL => hsl2oklab_token(arena, token),
            ColorType::HWB => hwb2oklab_token(arena, token),
            ColorType::DEVICE_CMYK => cmyk2oklab_token(arena, token),
            ColorType::LAB => lab2oklab_token(arena, token),
            ColorType::LCH => lch2oklab_token(arena, token),
            ColorType::OKLCH => oklch2oklab_token(arena, token),
            ColorType::COLOR => color2oklab_token(arena, token),
            _ => None,
        },
        ColorType::OKLCH => match kin {
            ColorType::HEX | ColorType::LIT => hex2oklch_token(arena, token),
            ColorType::RGB => rgb2oklch_token(arena, token),
            ColorType::HSL => hsl2oklch_token(arena, token),
            ColorType::HWB => hwb2oklch_token(arena, token),
            ColorType::DEVICE_CMYK => cmyk2oklch_token(arena, token),
            ColorType::LAB => lab2oklch_token(arena, token),
            ColorType::OKLAB => oklab2oklch_token(arena, token),
            ColorType::LCH => lch2oklch_token(arena, token),
            ColorType::COLOR => color2oklch_token(arena, token),
            _ => None,
        },
        _ if COLOR_FUNC_COLOR_SPACE.contains(&color_space_name(to).as_str()) => {
            let values = match kin {
                ColorType::HEX | ColorType::LIT => Some(hex2srgbvalues(arena, token)),
                ColorType::RGB => rgb2srgb(arena, token),
                ColorType::HSL => hsl2srgb(arena, token),
                ColorType::HWB => hwb2srgbvalues(arena, token),
                ColorType::DEVICE_CMYK => cmyk2srgbvalues(arena, token),
                ColorType::LAB => lab2srgbvalues(arena, token),
                ColorType::OKLAB => oklab2srgbvalues(arena, token),
                ColorType::LCH => lch2srgbvalues(arena, token),
                ColorType::OKLCH => oklch2srgbvalues(arena, token),
                ColorType::COLOR => color2srgbvalues(arena, token),
                _ => return None,
            }?;
            values2colortoken(arena, &values, to)
        }
        _ => None,
    }
}

fn srgb2srgbcolorspace(val: &[f64], to: u8) -> Option<Vec<f64>> {
    let (r, g, b, a) = (at(val, 0), at(val, 1), at(val, 2), opt(val, 3));
    Some(match to {
        ColorType::SRGB => val.to_vec(),
        ColorType::SRGB_LINEAR => srgb2lsrgbvalues(r, g, b, a),
        ColorType::DISPLAY_P3 => srgb2p3values(r, g, b, a),
        ColorType::DISPLAY_P3_LINEAR => srgb2lp3values(r, g, b, a),
        ColorType::PROPHOTO_RGB => srgb2prophotorgbvalues(r, g, b, a),
        ColorType::A98_RGB => srgb2a98values(r, g, b, a),
        ColorType::REC2020 => srgb2rec2020values(r, g, b, a),
        ColorType::XYZ => srgb2xyz(r, g, b, a),
        ColorType::XYZ_D50 => srgb2xyz_d65(r, g, b, a),
        _ => return None,
    })
}

pub fn minmax(value: f64, min: f64, max: f64) -> f64 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

pub fn color2srgbvalues(arena: &AstArena, token: u32) -> Option<Vec<f64>> {
    let mut components = get_color_components(arena, token)?;
    if components.is_empty() {
        return Some(Vec::new());
    }
    let color_space = components.remove(0);
    let values: Vec<f64> = components.iter().map(|&v| get_number(arena, v)).collect();
    let (r, g, b, a) = (at(&values, 0), at(&values, 1), at(&values, 2), opt(&values, 3));
    Some(match &*arena.resolve_string(arena.string_value.get(color_space)) {
        "display-p3" => p32srgbvalues(r, g, b, a),
        "display-p3-linear" => lp32srgbvalues(r, g, b, a),
        "srgb-linear" => lsrgb2srgbvalues(r, g, b, a),
        "prophoto-rgb" => prophotorgb2srgbvalues(r, g, b, a),
        "a98-rgb" => a98rgb2srgbvalues(r, g, b, a),
        "rec2020" => rec20202srgb(r, g, b, a),
        "xyz" | "xyz-d65" => xyz2srgb(r, g, b, a),
        "xyz-d50" => xyzd502srgb(r, g, b, a),
        _ => values,
    })
}

fn values2colortoken(arena: &AstArena, values: &[f64], to: u8) -> Option<u32> {
    let values = srgb2srgbcolorspace(values, to)?;
    let number = |v: f64| arena.create_token(EnumToken::NumberTokenType, TokenValue::Num(v), None);
    let mut chi = vec![number(at(&values, 0)), number(at(&values, 1)), number(at(&values, 2))];
    if values.len() == 4 {
        chi.push(arena.create_token(EnumToken::LiteralTokenType, TokenValue::Str("/"), None));
        chi.push(arena.create_token(EnumToken::PercentageTokenType, TokenValue::Num(values[3] * 100.0), None));
    }
    let color_space = color_space_name(to);
    Some(if COLOR_FUNC_COLOR_SPACE.contains(&color_space.as_str()) {
        let mut tokens = vec![arena.create_token(EnumToken::IdenTokenType, TokenValue::Str(&color_space), None)];
        tokens.extend(chi);
        arena.create_color("color", ColorType::COLOR, Some(&tokens))
    } else {
        arena.create_color(&color_space, to, Some(&chi))
    })
}

/// clamp color values
pub fn clamp(arena: &AstArena, token: u32) -> u32 {
    if arena.color_type.get(token) == ColorType::RGB {
        let mut index = 0;
        let mut t = arena.first_token.get(token);
        while t != 0 {
            let typ = arena.data_types.get(t);
            if !matches!(typ, EnumToken::LiteralTokenType | EnumToken::CommaTokenType | EnumToken::WhitespaceTokenType) {
                let v = arena.numeric_value.get(t);
                if typ == EnumToken::NumberTokenType {
                    arena.numeric_value.set(t, minmax(v, 0.0, if index <= 2 { 255.0 } else { 1.0 }));
                } else if typ == EnumToken::PercentageTokenType {
                    arena.numeric_value.set(t, minmax(v, 0.0, 100.0));
                }
                index += 1;
            }
            t = arena.next_sibling.get(t);
        }
    }
    token
}

/// numeric value of a number, percentage, fraction or `none` token. percentages are divided by 100
pub fn get_number(arena: &AstArena, token: u32) -> f64 {
    let typ = arena.data_types.get(token);
    if typ == EnumToken::IdenTokenType && &*arena.resolve_string(arena.string_value.get(token)) == "none" {
        return 0.0;
    }
    let fraction = arena.get_fraction(token);
    let val = if fraction != 0 {
        arena.numeric_value.get(arena.first_child.get(fraction)) / arena.numeric_value.get(arena.last_child.get(fraction))
    } else {
        arena.numeric_value.get(token)
    };
    if typ == EnumToken::PercentageTokenType {
        val / 100.0
    } else {
        val
    }
}

/// convert an angle to turns
pub fn get_angle(arena: &AstArena, token: u32) -> f64 {
    let typ = arena.data_types.get(token);
    if typ == EnumToken::IdenTokenType && &*arena.resolve_string(arena.string_value.get(token)) == "none" {
        return 0.0;
    }
    let val = get_number(arena, token);
    if typ == EnumToken::AngleTokenType {
        match &*arena.resolve_string(arena.unit.get(token)) {
            "deg" => return val / 360.0,
            "rad" => return val / (2.0 * std::f64::consts::PI),
            "grad" => return val / 400.0,
            "turn" => return val,
            _ => {}
        }
    }
    val / 360.0
}
