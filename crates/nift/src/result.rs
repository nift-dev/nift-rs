//! Render results (NR1).
//!
//! A successful render produces [`RenderResult`]: the generated text plus the
//! external inputs the renderer discovered (dependencies and requirements),
//! each a root-relative path spelling, deduplicated and sorted. Renders are
//! expressed as `Result<RenderResult, RenderError>`; the success type carries
//! only success data.
//!
//! Deliberate Rust API deviation (documented): the frozen C++ `RenderResult`
//! is a single value with `ok()/output()/error()`. Rust expresses the same
//! observable contract with the type system instead: `Result<RenderResult,
//! RenderError>`. There is no success value that also carries an error.

use crate::error::RenderError;
use std::collections::BTreeSet;

/// The outcome of a successful render.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenderResult {
    /// The generated text.
    pub output: String,
    /// External inputs the renderer read, root-relative spellings, sorted and
    /// deduplicated.
    pub dependencies: BTreeSet<String>,
    /// Outputs the rendered page requires (e.g. `@pathto` destinations),
    /// root-relative spellings, sorted and deduplicated.
    pub requirements: BTreeSet<String>,
}

impl RenderResult {
    pub fn new(output: impl Into<String>) -> Self {
        RenderResult {
            output: output.into(),
            dependencies: BTreeSet::new(),
            requirements: BTreeSet::new(),
        }
    }
}

/// A successful render carrying no output (placeholder for NR1; real renders
/// arrive from NR2). Exists so the error/result types compile and are usable
/// by consumers before the renderer lands.
pub type RenderSuccess = Result<RenderResult, RenderError>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;

    #[test]
    fn render_result_holds_output_and_sets() {
        let mut result = RenderResult::new("html");
        result
            .dependencies
            .insert("templates/template.html".to_string());
        result.dependencies.insert("content/about.html".to_string());
        result.requirements.insert("public/index.html".to_string());
        assert_eq!(result.output, "html");
        assert_eq!(result.dependencies.len(), 2);
        // BTreeSet iteration is sorted.
        assert!(result.dependencies.iter().next().is_some());
    }

    #[test]
    fn render_error_carries_kind_and_text() {
        let error = RenderError::new(ErrorKind::UnknownPage, "unknown page name 'nope'");
        assert_eq!(error.kind, ErrorKind::UnknownPage);
        assert!(error.message.contains("unknown page name"));
        assert!(error.source.is_none());

        let located = RenderError::new(ErrorKind::Parse, "bad").at("templates/x.html", 3, 7);
        assert_eq!(located.source.as_deref(), Some("templates/x.html"));
        assert_eq!(located.line, Some(3));
        assert_eq!(located.column, Some(7));
    }

    #[test]
    fn error_kind_display() {
        assert_eq!(ErrorKind::MissingSource.to_string(), "missing-source");
        assert_eq!(ErrorKind::PathEscape.to_string(), "path-escape");
    }
}
