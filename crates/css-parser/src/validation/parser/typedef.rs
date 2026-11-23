//! Port of `src/validation/parser/typedef.ts`.

#[allow(non_snake_case)]
pub mod ValidationTokenEnum {
    pub const Root: u8 = 0;
    pub const Keyword: u8 = 1;
    pub const PropertyType: u8 = 2;
    pub const DeclarationType: u8 = 3;
    pub const AtRule: u8 = 4;
    pub const FunctionDefinition: u8 = 5;
    pub const OpenBracket: u8 = 6;
    pub const CloseBracket: u8 = 7;
    pub const OpenParenthesis: u8 = 8;
    pub const CloseParenthesis: u8 = 9;
    pub const Comma: u8 = 10;
    pub const Pipe: u8 = 11;
    pub const Column: u8 = 12;
    pub const Star: u8 = 13;
    pub const OpenCurlyBrace: u8 = 14;
    pub const CloseCurlyBrace: u8 = 15;
    pub const HashMark: u8 = 16;
    pub const QuestionMark: u8 = 17;
    pub const Function: u8 = 18;
    pub const Number: u8 = 19;
    pub const Whitespace: u8 = 20;
    pub const Parenthesis: u8 = 21;
    pub const Bracket: u8 = 22;
    pub const Block: u8 = 23;
    pub const Plus: u8 = 24;
    pub const Separator: u8 = 25;
    pub const Exclamation: u8 = 26;
    pub const Ampersand: u8 = 27;
    pub const PipeToken: u8 = 28;
    pub const ColumnToken: u8 = 29;
    pub const AmpersandToken: u8 = 30;
    pub const Parens: u8 = 31;
    pub const PseudoClassToken: u8 = 32;
    pub const PseudoClassFunctionToken: u8 = 33;
    pub const StringToken: u8 = 34;
    pub const AtRuleDefinition: u8 = 35;
    pub const DeclarationNameToken: u8 = 36;
    pub const DeclarationDefinitionToken: u8 = 37;
    pub const SemiColon: u8 = 38;
    pub const Character: u8 = 39;
    pub const InfinityToken: u8 = 40;
    pub const LessThan: u8 = 41;
    pub const GreaterThan: u8 = 42;
    pub const EOF: u8 = 43;
    pub const OptionalGroupToken: u8 = 44;
    pub const Dimension: u8 = 45;
    pub const DisallowWhitespace: u8 = 46;
    pub const Colon: u8 = 47;

    pub fn name(value: u8) -> Option<&'static str> {
        match value {
            0 => Some("Root"),
            1 => Some("Keyword"),
            2 => Some("PropertyType"),
            3 => Some("DeclarationType"),
            4 => Some("AtRule"),
            5 => Some("FunctionDefinition"),
            6 => Some("OpenBracket"),
            7 => Some("CloseBracket"),
            8 => Some("OpenParenthesis"),
            9 => Some("CloseParenthesis"),
            10 => Some("Comma"),
            11 => Some("Pipe"),
            12 => Some("Column"),
            13 => Some("Star"),
            14 => Some("OpenCurlyBrace"),
            15 => Some("CloseCurlyBrace"),
            16 => Some("HashMark"),
            17 => Some("QuestionMark"),
            18 => Some("Function"),
            19 => Some("Number"),
            20 => Some("Whitespace"),
            21 => Some("Parenthesis"),
            22 => Some("Bracket"),
            23 => Some("Block"),
            24 => Some("Plus"),
            25 => Some("Separator"),
            26 => Some("Exclamation"),
            27 => Some("Ampersand"),
            28 => Some("PipeToken"),
            29 => Some("ColumnToken"),
            30 => Some("AmpersandToken"),
            31 => Some("Parens"),
            32 => Some("PseudoClassToken"),
            33 => Some("PseudoClassFunctionToken"),
            34 => Some("StringToken"),
            35 => Some("AtRuleDefinition"),
            36 => Some("DeclarationNameToken"),
            37 => Some("DeclarationDefinitionToken"),
            38 => Some("SemiColon"),
            39 => Some("Character"),
            40 => Some("InfinityToken"),
            41 => Some("LessThan"),
            42 => Some("GreaterThan"),
            43 => Some("EOF"),
            44 => Some("OptionalGroupToken"),
            45 => Some("Dimension"),
            46 => Some("DisallowWhitespace"),
            47 => Some("Colon"),
            _ => None,
        }
    }
}

/// keys of the validation config object
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValidationSyntaxGroupEnum {
    Declarations,
    Functions,
    Syntaxes,
    Types,
    Selectors,
    AtRules,
    Units,
    Languages,
    MediaFeatures,
}

impl ValidationSyntaxGroupEnum {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Declarations => "declarations",
            Self::Functions => "functions",
            Self::Syntaxes => "syntaxes",
            Self::Types => "types",
            Self::Selectors => "selectors",
            Self::AtRules => "atRules",
            Self::Units => "units",
            Self::Languages => "languages",
            Self::MediaFeatures => "mediaFeatures",
        }
    }
}

/// types of media features
#[allow(non_snake_case)]
pub mod MediaFeatureType {
    pub const BooleanType: &str = "boolean";
    pub const IntergerType: &str = "integer";
    pub const KeywordType: &str = "keyword";
    pub const LengthType: &str = "length";
    pub const NumberType: &str = "number";
    pub const RatioType: &str = "ratio";
    pub const ResolutionType: &str = "resolution";
    pub const StringType: &str = "string";
}
