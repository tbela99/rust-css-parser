//! Port of the shared declarations of `src/@types/*.d.ts` and `src/types.ts`.

use std::rc::Rc;

/// `ResponseType` of `src/types.ts`
#[allow(non_snake_case)]
pub mod ResponseType {
    pub const Text: u32 = 0;
    pub const ReadableStream: u32 = 1;
    pub const ArrayBuffer: u32 = 2;
    pub const JSON: u32 = 3;
}

/// `ErrorDescription.action`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorAction {
    Drop,
    Ignore,
}

impl ErrorAction {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorAction::Drop => "drop",
            ErrorAction::Ignore => "ignore",
        }
    }
}

/// `ErrorDescription.syntax`: a syntax string or a validation token
#[derive(Clone, Debug, PartialEq)]
pub enum ErrorSyntax {
    Text(Rc<str>),
    /// validation token (plain object: its string keyed properties)
    Token(Rc<crate::validation::parser::types::ValidationToken>),
    /// object of an error created from a plain object (`fromObject()`)
    Object(Rc<crate::plain::Plain>),
}

/// `ErrorDescription`
#[derive(Clone, Debug, PartialEq)]
pub struct ErrorDescription {
    pub action: ErrorAction,
    pub message: String,
    pub syntax: Option<ErrorSyntax>,
    /// error node id
    pub node: Option<u32>,
    /// `[src, line, column]`
    pub location: Option<(Option<Rc<str>>, u32, u32)>,
    /// error message of an `Error` object
    pub error: Option<String>,
}

impl ErrorDescription {
    pub fn new(action: ErrorAction, message: impl Into<String>) -> Self {
        Self { action, message: message.into(), syntax: None, node: None, location: None, error: None }
    }

    pub fn drop(message: impl Into<String>) -> Self {
        Self::new(ErrorAction::Drop, message)
    }

    pub fn ignore(message: impl Into<String>) -> Self {
        Self::new(ErrorAction::Ignore, message)
    }

    pub fn with_node(mut self, node: u32) -> Self {
        self.node = if node == 0 { None } else { Some(node) };
        self
    }

    pub fn with_syntax(mut self, syntax: impl Into<Rc<str>>) -> Self {
        self.syntax = Some(ErrorSyntax::Text(syntax.into()));
        self
    }

    pub fn with_location(mut self, src: Option<Rc<str>>, line: u32, column: u32) -> Self {
        self.location = Some((src, line, column));
        self
    }
}

/// `SourceLocation` of `ast.d.ts`
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SourceLocation {
    pub sta: u32,
    pub end: u32,
    pub src_id: u32,
}

impl ErrorDescription {
    /// plain object of the error (`ErrorDescription`)
    pub fn to_plain(&self) -> crate::plain::Plain {
        use crate::plain::{Plain, PlainObject};
        let mut o = PlainObject::new();
        o.set("action", self.action.as_str());
        o.set("message", self.message.as_str());
        if let Some(syntax) = &self.syntax {
            match syntax {
                ErrorSyntax::Text(s) => o.set("syntax", s.clone()),
                ErrorSyntax::Token(token) => o.set("syntax", token.to_plain()),
                ErrorSyntax::Object(plain) => o.set("syntax", (**plain).clone()),
            }
        }
        if let Some(node) = self.node {
            o.set("node", node);
        }
        if let Some((src, line, column)) = &self.location {
            o.set("location", Plain::Array(vec![src.clone().into(), (*line).into(), (*column).into()]));
        }
        if let Some(error) = &self.error {
            let mut e = PlainObject::new();
            e.set("message", error.as_str());
            o.set("error", e);
        }
        Plain::Object(o)
    }

    /// error from a plain object
    pub fn from_plain(plain: &crate::plain::Plain) -> Self {
        let action = if plain.get("action").as_str() == Some("ignore") { ErrorAction::Ignore } else { ErrorAction::Drop };
        let mut error = ErrorDescription::new(action, plain.get("message").as_str().unwrap_or(""));
        match plain.get("syntax") {
            crate::plain::Plain::String(s) => error.syntax = Some(ErrorSyntax::Text(s.clone())),
            p @ crate::plain::Plain::Object(_) => error.syntax = Some(ErrorSyntax::Object(Rc::new(p.clone()))),
            _ => {}
        }
        if let Some(n) = plain.get("node").as_f64() {
            error.node = Some(n as u32);
        }
        error
    }
}
