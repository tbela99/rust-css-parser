//! Port of `src/syntax/constants.ts`.

use crate::ast::types::EnumToken;
use crate::plain::Plain;
use crate::utils::fx::{HashMap, HashSet};
use std::sync::OnceLock;

/// the units of the validation data, in their canonical spelling (`Hz`, `Q`, `dB`)
pub fn validation_units() -> &'static [String] {
    static UNITS: OnceLock<Vec<String>> = OnceLock::new();
    UNITS.get_or_init(|| {
        crate::validation::json::config_json()["units"]
            .as_array()
            .map(|a| a.iter().filter_map(|u| u.as_str().map(String::from)).collect())
            .unwrap_or_default()
    })
}

/// `/^-((webkit)|o|moz)(-repeating)?-linear-gradient$/i`
pub fn reg_match_linear_gradient(name: &str) -> bool {
    match_prefixed_gradient(name, "linear")
}

/// `/^-((webkit)|o|moz)(-repeating)?-radial-gradient$/i`
pub fn reg_match_radial_gradient(name: &str) -> bool {
    match_prefixed_gradient(name, "radial")
}

fn match_prefixed_gradient(name: &str, kind: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some(rest) = ["-webkit", "-o", "-moz"].iter().find_map(|p| lower.strip_prefix(p)) else {
        return false;
    };
    let rest = rest.strip_prefix("-repeating").unwrap_or(rest);
    rest.strip_prefix('-').and_then(|r| r.strip_prefix(kind)) == Some("-gradient")
}

pub const mFLT: [u8; 2] = [EnumToken::LtTokenType, EnumToken::LteTokenType];
pub const mFGT: [u8; 2] = [EnumToken::GtTokenType, EnumToken::GteTokenType];

/// `tokensMap`
pub fn tokens_map(typ: u8) -> Option<u8> {
    match typ {
        EnumToken::Star => Some(EnumToken::Mul),
        EnumToken::Plus => Some(EnumToken::Add),
        _ => None,
    }
}

/// `/^(["']?)[a-zA-Z0-9_/.-][a-zA-Z0-9_/:.#?-]+(\1)$/`
pub fn url_token_matcher(value: &str) -> bool {
    let bytes = value.as_bytes();
    let quote = match bytes.first() {
        Some(&q @ (b'"' | b'\'')) => Some(q),
        _ => None,
    };
    let body = match quote {
        Some(q) => {
            if bytes.len() < 2 || bytes[bytes.len() - 1] != q {
                return false;
            }
            &bytes[1..bytes.len() - 1]
        }
        None => bytes,
    };
    if body.len() < 2 {
        return false;
    }
    let first = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'/' | b'.' | b'-');
    let rest = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'/' | b':' | b'.' | b'#' | b'?' | b'-');
    first(body[0]) && body[1..].iter().all(|&b| rest(b))
}

/// `tokensfuncDefMap`: function definition token type -> function token type
pub const TOKENS_FUNC_DEF_MAP: [(u8, u8); 16] = [
    (EnumToken::FunctionTokenDefType, EnumToken::FunctionTokenType),
    (EnumToken::ColorFunctionTokenDefType, EnumToken::ColorTokenType),
    (EnumToken::UrlFunctionTokenDefType, EnumToken::UrlFunctionTokenType),
    (EnumToken::MathFunctionTokenDefType, EnumToken::MathFunctionTokenType),
    (EnumToken::ImageFunctionTokenDefType, EnumToken::ImageFunctionTokenType),
    (EnumToken::TimingFunctionTokenDefType, EnumToken::TimingFunctionTokenType),
    (EnumToken::CustomFunctionTokenDefType, EnumToken::CustomFunctionTokenType),
    (EnumToken::WildCardFunctionTokenDefType, EnumToken::WildCardFunctionTokenType),
    (EnumToken::TimelineFunctionTokenDefType, EnumToken::TimelineFunctionTokenType),
    (EnumToken::GridTemplateFuncTokenDefType, EnumToken::GridTemplateFuncTokenType),
    (EnumToken::SupportsFunctionTokenDefType, EnumToken::SupportsFunctionTokenType),
    (EnumToken::WhenElseFunctionTokenDefType, EnumToken::WhenElseFunctionTokenType),
    (EnumToken::ContainerFunctionTokenDefType, EnumToken::ContainerFunctionTokenType),
    (EnumToken::PseudoClassFunctionTokenDefType, EnumToken::PseudoClassFuncTokenType),
    (EnumToken::TransformFunctionTokenDefType, EnumToken::TransformFunctionTokenType),
    (EnumToken::GeneralEnclosedFunctionTokenDefType, EnumToken::GeneralEnclosedFunctionTokenType),
];

/// `tokensfuncDefMap.get(type)`
pub fn tokens_func_def_map(typ: u8) -> Option<u8> {
    TOKENS_FUNC_DEF_MAP.iter().find(|(d, _)| *d == typ).map(|(_, t)| *t)
}

const fn function_types() -> [u8; 256] {
    let mut types = [0u8; 256];
    let mut i = 0;
    while i < TOKENS_FUNC_DEF_MAP.len() {
        types[TOKENS_FUNC_DEF_MAP[i].0 as usize] |= 1;
        types[TOKENS_FUNC_DEF_MAP[i].1 as usize] |= 2;
        i += 1;
    }
    types
}

static FUNCTION_TYPES: [u8; 256] = function_types();

/// true if the token type is a function definition (`name(`)
#[inline]
pub fn is_function_def_type(typ: u8) -> bool {
    FUNCTION_TYPES[typ as usize] & 1 == 1
}

/// true if the token type is a function (`tokensfuncSet.has(type)`)
#[inline]
pub fn is_function_type(typ: u8) -> bool {
    FUNCTION_TYPES[typ as usize] & 2 == 2
}

pub const epsilon: f64 = 1e-5;
pub const colorDistancePrecision: f64 = epsilon;
pub const colorPrecision: i32 = 6;
pub const anglePrecision: i32 = 3;

/// `colorRange`: [min, max] of the components, by color space
pub fn color_range(space: &str, component: &str) -> Option<[f64; 2]> {
    Some(match (space, component) {
        ("rgb", "r" | "g" | "b") => [0.0, 255.0],
        ("lab", "l") => [0.0, 100.0],
        ("lab", "a" | "b") => [-125.0, 125.0],
        ("lch", "l") => [0.0, 100.0],
        ("lch", "c") => [0.0, 150.0],
        ("lch", "h") => [0.0, 360.0],
        ("oklab", "l") => [0.0, 1.0],
        ("oklab", "a" | "b") => [-0.4, 0.4],
        ("oklch", "l") => [0.0, 1.0],
        ("oklch", "a" | "b") => [0.0, 0.4],
        _ => return None,
    })
}

pub const PSEUDO_ELEMENTS: [&str; 8] = [":before", ":after", ":first-line", ":first-letter", "::before", "::after", "::first-line", "::first-letter"];
pub const WILD_CARD_FUNCS: [&str; 3] = ["var", "env", "if"];
pub const MATH_FUNCS: [&str; 26] = ["minmax", "repeat", "fit-content", "calc", "clamp", "min", "max", "round", "mod", "rem", "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "tan", "pow", "sqrt", "hypot", "log", "exp", "abs", "sign", "anchor"];
pub const MEDIA_TYPES: [&str; 11] = ["all", "print", "screen", "aural", "braille", "embossed", "handheld", "projection", "tty", "tv", "speech"];
pub const URL_FUNC: [&str; 1] = ["url"];
pub const TIMELINE_FUNC: [&str; 2] = ["view", "scroll"];
pub const GRID_TEMPLATE_FUNC: [&str; 3] = ["minmax", "fit-content", "repeat"];
pub const SUPPORT_FUNC: [&str; 5] = ["selector", "font-tech", "font-format", "at-rule", "named-feature"];
pub const WHEN_ELSE_FUNC: [&str; 2] = ["media", "supports"];
pub const CONTAINER_FUNC: [&str; 2] = ["style", "scroll-state"];
pub const TIMING_FUNC: [&str; 9] = ["ease", "ease-in", "ease-out", "ease-in-out", "linear", "step-start", "step-end", "steps", "cubic-bezier"];
pub const COLORS_FUNC: [&str; 16] = ["rgb", "rgba", "hsl", "hsla", "hwb", "device-cmyk", "color-mix", "color", "oklab", "lab", "oklch", "lch", "alpha", "light-dark", "color-layers", "contrast-color"];
pub const IMAGE_FUNC: [&str; 18] = ["linear-gradient", "radial-gradient", "repeating-linear-gradient", "repeating-radial-gradient", "repeating-conic-gradient", "conic-gradient", "image", "image-set", "element", "cross-fade", "paint", "-o-linear-gradient", "-moz-linear-gradient", "-webkit-linear-gradient", "-webkit-repeating-linear-gradient", "-webkit-gradient", "-webkit-radial-gradient", "-webkit-repeating-radial-gradient"];
pub const TRANSFORM_FUNCTIONS: [&str; 21] = ["translate", "scale", "rotate", "skew", "perspective", "translateX", "translateY", "translateZ", "scaleX", "scaleY", "scaleZ", "rotateX", "rotateY", "rotateZ", "skewX", "skewY", "rotate3d", "translate3d", "scale3d", "matrix", "matrix3d"];
pub const COLOR_FUNC_COLOR_SPACE: [&str; 12] = ["srgb", "srgb-linear", "display-p3", "display-p3-linear", "prophoto-rgb", "a98-rgb", "rec2020", "lab", "oklab", "xyz", "xyz-d65", "xyz-d50"];
pub const COMBINATORS: [&str; 5] = ["+", ">", "~", "||", "|"];
pub const PAGE_MARGIN_BOX_TYPE: [&str; 16] = ["top-left-corner", "top-left", "top-center", "top-right", "top-right-corner", "bottom-left-corner", "bottom-left", "bottom-center", "bottom-right", "bottom-right-corner", "left-top", "left-middle", "left-bottom", "right-top", "right-middle", "right-bottom"];

pub const FUNC_LIKE: [u8; 9] = [
    EnumToken::ParensTokenType,
    EnumToken::FunctionTokenType,
    EnumToken::UrlFunctionTokenType,
    EnumToken::StartParensTokenType,
    EnumToken::ImageFunctionTokenType,
    EnumToken::TimingFunctionTokenType,
    EnumToken::TimingFunctionTokenType,
    EnumToken::PseudoClassFuncTokenType,
    EnumToken::GridTemplateFuncTokenType,
];

pub const D50: [f64; 3] = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];
/// `Math.pow(29, 3) / Math.pow(3, 3)`
pub const k: f64 = 24389.0 / 27.0;
/// `Math.pow(6, 3) / Math.pow(29, 3)`
pub const e: f64 = 216.0 / 24389.0;

/// `nonStandardColors`
pub fn non_standard_colors() -> &'static HashSet<String> {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| {
        let syntax = crate::validation::json::config_json()["syntaxes"]["-non-standard-color"]["syntax"].as_str().unwrap_or("");
        syntax
            .split(|c: char| c.is_whitespace() || c == '|')
            .filter(|s| !s.is_empty())
            .map(|m| m.to_lowercase())
            .collect()
    })
}

/// `systemColors` (lowercase)
pub const SYSTEM_COLORS: [&str; 16] = ["activetext", "buttonborder", "buttonface", "buttontext", "canvas", "canvastext", "field", "fieldtext", "graytext", "highlight", "highlighttext", "linktext", "mark", "marktext", "visitedtext", "-webkit-focus-ring-color"];
/// `deprecatedSystemColors` (lowercase)
pub const DEPRECATED_SYSTEM_COLORS: [&str; 28] = ["activeborder", "activecaption", "appworkspace", "background", "buttonface", "buttonhighlight", "buttonshadow", "buttontext", "captiontext", "graytext", "highlight", "highlighttext", "inactiveborder", "inactivecaption", "inactivecaptiontext", "infobackground", "infotext", "menu", "menutext", "scrollbar", "threeddarkshadow", "threedface", "threedhighlight", "threedlightshadow", "threedshadow", "window", "windowframe", "windowtext"];

/// `COLORS_NAMES`: name to color
pub const COLORS_NAMES: [(&str, &str); 149] = [
    ("aliceblue", "#f0f8ff"),
    ("antiquewhite", "#faebd7"),
    ("aqua", "#00ffff"),
    ("aquamarine", "#7fffd4"),
    ("azure", "#f0ffff"),
    ("beige", "#f5f5dc"),
    ("bisque", "#ffe4c4"),
    ("black", "#000000"),
    ("blanchedalmond", "#ffebcd"),
    ("blue", "#0000ff"),
    ("blueviolet", "#8a2be2"),
    ("brown", "#a52a2a"),
    ("burlywood", "#deb887"),
    ("cadetblue", "#5f9ea0"),
    ("chartreuse", "#7fff00"),
    ("chocolate", "#d2691e"),
    ("coral", "#ff7f50"),
    ("cornflowerblue", "#6495ed"),
    ("cornsilk", "#fff8dc"),
    ("crimson", "#dc143c"),
    ("cyan", "#00ffff"),
    ("darkblue", "#00008b"),
    ("darkcyan", "#008b8b"),
    ("darkgoldenrod", "#b8860b"),
    ("darkgray", "#a9a9a9"),
    ("darkgrey", "#a9a9a9"),
    ("darkgreen", "#006400"),
    ("darkkhaki", "#bdb76b"),
    ("darkmagenta", "#8b008b"),
    ("darkolivegreen", "#556b2f"),
    ("darkorange", "#ff8c00"),
    ("darkorchid", "#9932cc"),
    ("darkred", "#8b0000"),
    ("darksalmon", "#e9967a"),
    ("darkseagreen", "#8fbc8f"),
    ("darkslateblue", "#483d8b"),
    ("darkslategray", "#2f4f4f"),
    ("darkslategrey", "#2f4f4f"),
    ("darkturquoise", "#00ced1"),
    ("darkviolet", "#9400d3"),
    ("deeppink", "#ff1493"),
    ("deepskyblue", "#00bfff"),
    ("dimgray", "#696969"),
    ("dimgrey", "#696969"),
    ("dodgerblue", "#1e90ff"),
    ("firebrick", "#b22222"),
    ("floralwhite", "#fffaf0"),
    ("forestgreen", "#228b22"),
    ("fuchsia", "#ff00ff"),
    ("gainsboro", "#dcdcdc"),
    ("ghostwhite", "#f8f8ff"),
    ("gold", "#ffd700"),
    ("goldenrod", "#daa520"),
    ("gray", "#808080"),
    ("grey", "#808080"),
    ("green", "#008000"),
    ("greenyellow", "#adff2f"),
    ("honeydew", "#f0fff0"),
    ("hotpink", "#ff69b4"),
    ("indianred", "#cd5c5c"),
    ("indigo", "#4b0082"),
    ("ivory", "#fffff0"),
    ("khaki", "#f0e68c"),
    ("lavender", "#e6e6fa"),
    ("lavenderblush", "#fff0f5"),
    ("lawngreen", "#7cfc00"),
    ("lemonchiffon", "#fffacd"),
    ("lightblue", "#add8e6"),
    ("lightcoral", "#f08080"),
    ("lightcyan", "#e0ffff"),
    ("lightgoldenrodyellow", "#fafad2"),
    ("lightgray", "#d3d3d3"),
    ("lightgrey", "#d3d3d3"),
    ("lightgreen", "#90ee90"),
    ("lightpink", "#ffb6c1"),
    ("lightsalmon", "#ffa07a"),
    ("lightseagreen", "#20b2aa"),
    ("lightskyblue", "#87cefa"),
    ("lightslategray", "#778899"),
    ("lightslategrey", "#778899"),
    ("lightsteelblue", "#b0c4de"),
    ("lightyellow", "#ffffe0"),
    ("lime", "#00ff00"),
    ("limegreen", "#32cd32"),
    ("linen", "#faf0e6"),
    ("magenta", "#ff00ff"),
    ("maroon", "#800000"),
    ("mediumaquamarine", "#66cdaa"),
    ("mediumblue", "#0000cd"),
    ("mediumorchid", "#ba55d3"),
    ("mediumpurple", "#9370d8"),
    ("mediumseagreen", "#3cb371"),
    ("mediumslateblue", "#7b68ee"),
    ("mediumspringgreen", "#00fa9a"),
    ("mediumturquoise", "#48d1cc"),
    ("mediumvioletred", "#c71585"),
    ("midnightblue", "#191970"),
    ("mintcream", "#f5fffa"),
    ("mistyrose", "#ffe4e1"),
    ("moccasin", "#ffe4b5"),
    ("navajowhite", "#ffdead"),
    ("navy", "#000080"),
    ("oldlace", "#fdf5e6"),
    ("olive", "#808000"),
    ("olivedrab", "#6b8e23"),
    ("orange", "#ffa500"),
    ("orangered", "#ff4500"),
    ("orchid", "#da70d6"),
    ("palegoldenrod", "#eee8aa"),
    ("palegreen", "#98fb98"),
    ("paleturquoise", "#afeeee"),
    ("palevioletred", "#d87093"),
    ("papayawhip", "#ffefd5"),
    ("peachpuff", "#ffdab9"),
    ("peru", "#cd853f"),
    ("pink", "#ffc0cb"),
    ("plum", "#dda0dd"),
    ("powderblue", "#b0e0e6"),
    ("purple", "#800080"),
    ("red", "#ff0000"),
    ("rosybrown", "#bc8f8f"),
    ("royalblue", "#4169e1"),
    ("saddlebrown", "#8b4513"),
    ("salmon", "#fa8072"),
    ("sandybrown", "#f4a460"),
    ("seagreen", "#2e8b57"),
    ("seashell", "#fff5ee"),
    ("sienna", "#a0522d"),
    ("silver", "#c0c0c0"),
    ("skyblue", "#87ceeb"),
    ("slateblue", "#6a5acd"),
    ("slategray", "#708090"),
    ("slategrey", "#708090"),
    ("snow", "#fffafa"),
    ("springgreen", "#00ff7f"),
    ("steelblue", "#4682b4"),
    ("tan", "#d2b48c"),
    ("teal", "#008080"),
    ("thistle", "#d8bfd8"),
    ("tomato", "#ff6347"),
    ("turquoise", "#40e0d0"),
    ("violet", "#ee82ee"),
    ("wheat", "#f5deb3"),
    ("white", "#ffffff"),
    ("whitesmoke", "#f5f5f5"),
    ("yellow", "#ffff00"),
    ("yellowgreen", "#9acd32"),
    ("rebeccapurple", "#663399"),
    ("transparent", "#00000000"),
];

/// `COLORS_NAMES[name]`
pub fn color_name_to_hex(name: &str) -> Option<&'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| COLORS_NAMES.iter().copied().collect()).get(name).copied()
}

/// `NAMES_COLORS[hex]`: color to name (the last name of a color wins, like the typescript reduce)
pub fn hex_to_color_name(hex: &str) -> Option<&'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| COLORS_NAMES.iter().map(|&(name, hex)| (hex, name)).collect()).get(hex).copied()
}

/// `trimTokenSpace`
pub fn trim_token_space(typ: u8) -> bool {
    matches!(
        typ,
        EnumToken::Add
            | EnumToken::Mul
            | EnumToken::Div
            | EnumToken::Sub
            | EnumToken::ColonTokenType
            | EnumToken::CommaTokenType
            | EnumToken::SemiColonTokenType
            | EnumToken::DashedIdenTokenType
            | EnumToken::DeclarationNodeType
            | EnumToken::SupportsQueryConditionTokenType
            | EnumToken::SupportsQueryUnaryConditionTokenType
    )
}

#[allow(dead_code)]
fn _plain(_: Plain) {}
