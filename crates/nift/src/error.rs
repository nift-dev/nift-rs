//! Typed error model (NR1).
//!
//! Public diagnostics are free-form text; the typed `ErrorKind` category is
//! what matters for classification and for matching the canonical semantic
//! rejection classes later. This is a deliberate Rust API improvement over the
//! frozen C++ `RenderError` (which carries only message/source/line/column):
//! the semantic category is preserved in addition to the text.

use std::fmt;

/// Semantic error categories.
///
/// The set grows as checkpoints land: `Parse`/`MissingSource`/`PathEscape`/
/// `Schema` become reachable from the renderer (NR2+), `InvalidConfig`/
/// `InvalidTracking` from the project reader (NR7), `UnknownPage` from the
/// project-aware engine (NR8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// The template/input source could not be parsed or evaluated.
    Parse,
    /// A required source (content/template/input/JSON/contract) is missing.
    MissingSource,
    /// Project config is invalid (malformed JSON or semantically invalid).
    InvalidConfig,
    /// Project tracking is invalid (malformed JSON or semantically invalid).
    InvalidTracking,
    /// A path escaped the project root.
    PathEscape,
    /// A tracked page name does not exist.
    UnknownPage,
    /// JSON schema validation failed.
    Schema,
    /// A render-level failure not covered by a more specific category.
    Render,
    /// A binding name was rejected (invalid identifier or structural built-in).
    InvalidBinding,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            ErrorKind::Parse => "parse",
            ErrorKind::MissingSource => "missing-source",
            ErrorKind::InvalidConfig => "invalid-config",
            ErrorKind::InvalidTracking => "invalid-tracking",
            ErrorKind::PathEscape => "path-escape",
            ErrorKind::UnknownPage => "unknown-page",
            ErrorKind::Schema => "schema",
            ErrorKind::Render => "render",
            ErrorKind::InvalidBinding => "invalid-binding",
        };
        f.write_str(name)
    }
}

/// A render/project error: a typed semantic category plus free-form diagnostic
/// text and, where known, the source location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderError {
    pub kind: ErrorKind,
    pub message: String,
    pub source: Option<String>,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

impl RenderError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        RenderError {
            kind,
            message: message.into(),
            source: None,
            line: None,
            column: None,
        }
    }

    pub fn at(mut self, source: impl Into<String>, line: usize, column: usize) -> Self {
        self.source = Some(source.into());
        self.line = Some(line);
        self.column = Some(column);
        self
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(source) = &self.source {
            write!(f, "{}: {}", source, self.message)?;
        } else {
            f.write_str(&self.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for RenderError {}

/// A binding was rejected: invalid identifier or a structural built-in name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingError {
    InvalidIdentifier,
    StructuralBuiltin,
}

impl fmt::Display for BindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindingError::InvalidIdentifier => {
                f.write_str("binding name is not a valid Nift identifier")
            }
            BindingError::StructuralBuiltin => {
                f.write_str("binding name is a structural built-in and cannot be shadowed")
            }
        }
    }
}

impl std::error::Error for BindingError {}
