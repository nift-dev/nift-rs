//! Per-render request context (NR1).
//!
//! `Context` carries request-scoped state: page identity, the current output
//! location (used by `@pathto` later), and request-scoped value bindings. Value
//! bindings set here win over Engine defaults (the foundational precedence
//! contract, see [`crate::bindings::resolve`]); the renderer resolves the rest
//! of the chain (`@json`, contract, built-in metadata) from NR2/NR8.
//!
//! **Title is a single per-render slot.** The frozen reference specifies that
//! `set_title` and `set("title", ...)` share one per-render slot, last write
//! wins in both orderings. The C++ implementation stores that slot as the
//! `"title"` binding (`set_title` writes the binding; the parser resolves the
//! host `"title"` binding before built-in title metadata, so the binding is the
//! effective title). This Rust model follows the same observable contract: the
//! title slot IS `bindings["title"]`, `set_title` writes it as a string, and
//! `set("title", value)` writes it as any value. `binding("title")` and
//! `resolve` therefore observe the same effective title.
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

    /// Request-scoped title, overriding a default/tracked title. Writes the
    /// single per-render title slot, i.e. `set_title(s)` is equivalent to
    /// `set("title", Value::string(s))`; the most recent write of either wins.
    pub fn set_title(&mut self, title: impl Into<String>) {
        self.bindings
            .insert("title".to_string(), Value::string(title));
    }

    /// Set a request-scoped value binding. Returns
    /// [`BindingError::StructuralBuiltin`] for a structural built-in (checked
    /// first, so hyphenated structural names such as `content-path` are
    /// classified correctly) and [`BindingError::InvalidIdentifier`] for any
    /// other invalid name. `set("title", value)` writes the single per-render
    /// title slot.
    pub fn set(&mut self, name: impl Into<String>, value: Value) -> Result<(), BindingError> {
        let name = name.into();
        if crate::bindings::structural_builtin_name(&name) {
            return Err(BindingError::StructuralBuiltin);
        }
        if !crate::bindings::valid_binding_identifier(&name) {
            return Err(BindingError::InvalidIdentifier);
        }
        self.bindings.insert(name, value);
        Ok(())
    }

    pub fn page_name(&self) -> Option<&str> {
        self.page_name.as_deref()
    }

    /// The effective per-render title (the single title slot), as a string.
    /// `None` when no title was set or the slot holds a non-string value.
    pub fn title(&self) -> Option<&str> {
        self.bindings.get("title").and_then(|value| value.as_str())
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
        // Hyphenated structural built-ins are classified as StructuralBuiltin,
        // not InvalidIdentifier.
        assert_eq!(
            context.set("content-path", Value::null()),
            Err(BindingError::StructuralBuiltin)
        );
        assert_eq!(
            context.set("output-path", Value::null()),
            Err(BindingError::StructuralBuiltin)
        );
        assert_eq!(
            context.set("template-path", Value::null()),
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

    #[test]
    fn title_is_one_per_render_slot() {
        // set_title writes the "title" binding; set("title", ...) writes the
        // same slot. Last write wins in both orderings, and binding lookup /
        // resolution observes the effective title.
        let mut context = Context::new();
        context.set_title("A");
        assert_eq!(context.title(), Some("A"));
        assert_eq!(context.binding("title").and_then(|v| v.as_str()), Some("A"));

        context.set("title", Value::string("B")).unwrap();
        assert_eq!(context.title(), Some("B"));
        assert_eq!(context.binding("title").and_then(|v| v.as_str()), Some("B"));

        context.set("title", Value::string("C")).unwrap();
        context.set_title("D");
        assert_eq!(context.title(), Some("D"));
        assert_eq!(context.binding("title").and_then(|v| v.as_str()), Some("D"));
    }

    #[test]
    fn title_slot_holds_any_value_via_set() {
        let mut context = Context::new();
        context.set("title", Value::number(7.0)).unwrap();
        // String accessor is None for a non-string title slot; the binding
        // itself holds the value (rendered by the renderer from NR2).
        assert_eq!(context.title(), None);
        assert_eq!(
            context.binding("title").and_then(|v| v.as_number()),
            Some(7.0)
        );
    }

    #[test]
    fn title_resolves_through_precedence() {
        use crate::bindings::{resolve, Bindings};
        let mut defaults = Bindings::new();
        defaults
            .set("title", Value::string("default-title"))
            .unwrap();

        let mut context = Context::new();
        context.set_title("context-title");
        // Context overlay wins over the engine default title.
        assert_eq!(
            resolve(&defaults, &context, "title").and_then(|v| v.as_str()),
            Some("context-title")
        );
        assert_eq!(
            resolve(&defaults, &Context::new(), "title").and_then(|v| v.as_str()),
            Some("default-title")
        );
    }
}
