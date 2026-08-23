//! Render inputs (NR1).
//!
//! A `Source` is either in-memory text (with an optional logical identity for
//! diagnostics and relative resolution) or a filesystem path. This mirrors the
//! frozen C++ `nift::Source`; the renderer consumes it from NR2.

use std::path::{Path, PathBuf};

/// A render input: in-memory text or a filesystem path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// In-memory text. `logical_name` provides an identity for diagnostics and,
    /// later, relative `@input` resolution.
    Text {
        text: String,
        logical_name: Option<String>,
    },
    /// Filesystem-backed source.
    Path(PathBuf),
}

impl Source {
    /// In-memory text with no logical identity.
    pub fn text(text: impl Into<String>) -> Self {
        Source::Text {
            text: text.into(),
            logical_name: None,
        }
    }

    /// In-memory text with a logical identity.
    pub fn text_named(text: impl Into<String>, logical_name: impl Into<String>) -> Self {
        Source::Text {
            text: text.into(),
            logical_name: Some(logical_name.into()),
        }
    }

    /// A filesystem path source.
    pub fn path(path: impl Into<PathBuf>) -> Self {
        Source::Path(path.into())
    }

    /// The in-memory text, if this is a text source.
    pub fn as_text(&self) -> Option<&str> {
        if let Source::Text { text, .. } = self {
            Some(text)
        } else {
            None
        }
    }

    /// The logical identity of a text source, if any.
    pub fn logical_name(&self) -> Option<&str> {
        if let Source::Text { logical_name, .. } = self {
            logical_name.as_deref()
        } else {
            None
        }
    }

    /// The path, if this is a path source.
    pub fn as_path(&self) -> Option<&Path> {
        if let Source::Path(path) = self {
            Some(path)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_source() {
        let source = Source::text("hello");
        assert_eq!(source.as_text(), Some("hello"));
        assert_eq!(source.logical_name(), None);
        assert_eq!(source.as_path(), None);
    }

    #[test]
    fn text_source_with_logical_name() {
        let source = Source::text_named("hello", "templates/part.html");
        assert_eq!(source.as_text(), Some("hello"));
        assert_eq!(source.logical_name(), Some("templates/part.html"));
    }

    #[test]
    fn path_source() {
        let source = Source::path("content/about.html");
        assert_eq!(source.as_text(), None);
        assert_eq!(source.as_path(), Some(Path::new("content/about.html")));
    }

    #[test]
    fn clone_and_equality() {
        let a = Source::text("x");
        assert_eq!(a.clone(), a);
        assert_ne!(a, Source::path("x"));
    }
}
