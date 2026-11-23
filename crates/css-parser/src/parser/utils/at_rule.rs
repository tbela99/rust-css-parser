//! Port of `src/parser/utils/at-rule.ts` and `at-rule-font-feature-values.ts`.

use crate::ast::arena::AstArena;
use crate::options::ParserOptions;
use crate::types::ErrorDescription;
use crate::validation::config::get_syntax_rule;
use crate::validation::match_::{create_validation_context, match_all_syntaxes, trim_array};
use crate::validation::parser::typedef::ValidationSyntaxGroupEnum;

/// match the prelude of an at-rule with its syntax
pub fn match_at_rule_syntax(arena: &AstArena, at_rule: u32, stream: &mut Vec<u32>, options: &ParserOptions) -> (bool, Vec<ErrorDescription>) {
    let rules = get_syntax_rule(ValidationSyntaxGroupEnum::AtRules, &format!("@{}", arena.get_name(at_rule)));
    let syntax: Vec<_> = rules.and_then(|r| r.get_prelude_rules()).map(|p| p.iter().skip(1).cloned().collect()).unwrap_or_default();
    trim_array(arena, stream);
    if syntax.is_empty() {
        return (true, Vec::new());
    }
    let result = match_all_syntaxes(arena, Some(&syntax), &create_validation_context(arena, stream), options);
    (result.success, result.errors)
}

/// match the prelude of `@font-feature-values` (`at-rule-font-feature-values.ts`)
pub fn parse_at_rule_font_feature_values(arena: &AstArena, stream: &mut Vec<u32>, context: u32, options: &ParserOptions) -> (bool, Vec<ErrorDescription>) {
    let rules = get_syntax_rule(ValidationSyntaxGroupEnum::AtRules, &format!("@{}", arena.get_name(context)));
    let syntax: Option<Vec<_>> = rules.and_then(|r| r.get_prelude_rules()).map(|p| p.iter().skip(1).cloned().collect());
    trim_array(arena, stream);
    let result = match_all_syntaxes(arena, syntax.as_deref(), &create_validation_context(arena, stream), options);
    (result.success, result.errors)
}
