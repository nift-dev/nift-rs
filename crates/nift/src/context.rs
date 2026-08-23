//! Per-render request context (NR1).
//!
//! `Context` carries request-scoped state: page identity, title, the current
//! output location (used by `@pathto` later), and request-scoped value
//! bindings. Value bindings set here win over Engine defaults (the
//! foundational precedence contract, see [`crate::bindings::resolve`]); the
//! renderer resolves the rest of the chain (`@json`, contract, built-in
//! metadata) from NR2/NR8.
//!
//! `set_json` (JSON text → `Value`) is deliberately absent here: JSON parsing
//! belongs to its owning checkpoint (NR4), not to this data model.

use crate::error::BindingError;
use crate::value::Value;
use indexmap::IndexMap;
use std::path::{Path, PathBuf};

/// Request-scoped render state.
#[derive(Debug, Clone, Default)]
pub struct Context {
    page_name: Option<String>,
    title: Option<String>,
    current_output: Option<PathBuf>,
    bindings: IndexMap<String, Value>,
}

impl Context {
    pub fn new() -> Self {
        Self::default()
    }

    /// Page identity (used by the renderer and the 404 `@pathto` rule).
    pub fn set_page_name(&mut self, name: impl Into<String>) {
        self.page_name = Some(name.into());
    }

    /// The generated output location of the current page, used by `@pathto` to
    /// compute relative paths. Without it, `@pathto` has no path context.
    pub fn set_current_output(&mut self, output: impl Into<PathBuf>) {
        self.current_output = Some(output.into());
    }

    /// Request-scoped title, overriding a default/tracked title.
    pub fn set_title(&mut self, title: impl Into<String>) {
        self.title = Some(title.into());
    }

    /// Set a request-scoped value binding. Returns
    /// [`BindingError::InvalidIdentifier`] for an invalid name and
    /// [`BindingError::StructuralBuiltin`] for a structural built-in.
    pub fn set(&mut self, name: impl Into<String>, value: Value) -> Result<(), BindingError> {
        let name = name.into();
        if !crate::bindings::valid_binding_identifier(&name) {
            return Err(BindingError::InvalidIdentifier);
        }
        if crate::bindings::structural_builtin_name(&name) {
            return Err(BindingError::StructuralBuiltin);
        }
        self.bindings.insert(name, value);
        Ok(())
    }

    pub fn page_name(&self) -> Option<&str> {
        self.page_name.as_deref()
    }
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }
    pub fn current_output(&self) -> Option<&Path> {
        self.current_output.as_deref()
    }
    pub fn binding(&self, name: &str) -> Option<&Value> {
        self.bindings.get(name)
    }
    pub fn bindings(&self) -> &IndexMap<String, Value> {
        &self.bindings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setters_and_getters() {
        let mut context = Context::new();
        context.set_page_name("about");
        context.set_title("About");
        context.set_current_output("public/about.html");
        assert_eq!(context.page_name(), Some("about"));
        assert_eq!(context.title(), Some("About"));
        assert_eq!(
            context.current_output(),
            Some(Path::new("public/about.html"))
        );
    }

    #[test]
    fn set_validation_mirrors_bindings() {
        let mut context = Context::new();
        assert!(context.set("data", Value::object()).is_ok());
        assert_eq!(
            context.set("bad-name", Value::null()),
            Err(BindingError::InvalidIdentifier)
        );
        assert_eq!(
            context.set("loop", Value::null()),
            Err(BindingError::StructuralBuiltin)
        );
        assert_eq!(context.bindings().len(), 1);
    }

    #[test]
    fn binding_reads() {
        let mut context = Context::new();
        context.set("x", Value::number(5.0)).unwrap();
        assert_eq!(context.binding("x").and_then(|v| v.as_number()), Some(5.0));
        assert_eq!(context.binding("missing"), None);
    }
}
