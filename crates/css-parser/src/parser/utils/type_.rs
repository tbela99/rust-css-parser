//! Port of `src/parser/utils/type.ts` (`type` is a rust keyword: the module is `type_`).

use crate::ast::arena::AstArena;
use crate::ast::types::EnumToken;
use crate::syntax::constants::is_function_type;
use crate::syntax::syntax::is_color;

const MATH_TOKEN_TYPES: [u8; 15] = [
    EnumToken::Add,
    EnumToken::Mul,
    EnumToken::Div,
    EnumToken::Sub,
    EnumToken::LiteralTokenType,
    EnumToken::CommaTokenType,
    EnumToken::WhitespaceTokenType,
    EnumToken::DimensionTokenType,
    EnumToken::NumberTokenType,
    EnumToken::LengthTokenType,
    EnumToken::AngleTokenType,
    EnumToken::PercentageTokenType,
    EnumToken::ResolutionTokenType,
    EnumToken::TimeTokenType,
    EnumToken::BinaryExpressionTokenType,
];

thread_local! {
    /// the token types of the shorthand configurations (`EnumToken[name]` of `types`), by configuration address: the
    /// configuration is static, the names are resolved once
    static TYPES: std::cell::RefCell<crate::utils::fx::HashMap<usize, std::rc::Rc<[u8]>>> = std::cell::RefCell::new(Default::default());
}

/// `properties.types` resolved to token types
fn config_types(properties: &serde_json::Value) -> std::rc::Rc<[u8]> {
    let key = properties as *const serde_json::Value as usize;
    if let Some(types) = TYPES.with(|t| t.borrow().get(&key).cloned()) {
        return types;
    }
    let types: std::rc::Rc<[u8]> = properties["types"]
        .as_array()
        .map(|a| a.iter().filter_map(|t| t.as_str().and_then(EnumToken::value)).collect::<Vec<u8>>())
        .unwrap_or_default()
        .into();
    TYPES.with(|t| t.borrow_mut().insert(key, types.clone()));
    types
}

/// check if a token matches a shorthand property type (`PropertyMapType`: `keywords`, `types`)
pub fn match_type(arena: &AstArena, val: u32, properties: &serde_json::Value) -> bool {
    let typ = arena.data_types.get(val);
    let types = config_types(properties);
    let keyword = typ == EnumToken::IdenTokenType
        && properties["keywords"]
            .as_array()
            .is_some_and(|k| arena.with_string(arena.string_value.get(val), |v| k.iter().any(|x| x.as_str() == Some(v))));
    if keyword || types.iter().any(|&t| (typ == EnumToken::IdenTokenType && t == EnumToken::ColorTokenType && is_color(arena, val, None)) || t == typ) {
        return true;
    }
    if typ == EnumToken::NumberTokenType && arena.numeric_value.get(val) == 0.0 {
        return types.iter().any(|&t| t == EnumToken::LengthTokenType || t == EnumToken::AngleTokenType);
    }
    if is_function_type(typ) && typ == EnumToken::MathFunctionTokenType {
        return arena.token_iter(val).all(|t| MATH_TOKEN_TYPES.contains(&arena.data_types.get(t)) || match_type(arena, t, properties));
    }
    false
}
