//! Engine-default value bindings and the foundational precedence contract
//! (NR1).
//!
//! `Bindings` is the long-lived (engine default) binding store. The
//! foundational precedence rule is: **Context overlay > Engine default**. The
//! renderer extends this chain with `@json`, contract and built-in-metadata
//! bindings from NR2/NR8, but the data-model contract settled here is the base
//! of that chain.
//!
//! A binding name must be a valid Nift identifier and must not be one of the
//! structural built-ins (`name`, `content-path`, `output-path`, `template-path`,
//! `loop`), which describe the render's own geometry and must never be
//! shadowed. This mirrors the frozen reference `set` rule; the C++ `bool` is
//! expressed here as `Result<(), BindingError>`.

use crate::context::Context;
use crate::error::BindingError;
use crate::value::Value;
use indexmap::IndexMap;

/// A validated collection of long-lived (engine default) value bindings.
#[derive(Debug, Clone, Default)]
pub struct Bindings {
    map: IndexMap<String, Value>,
}

/// A valid Nift binding identifier: ASCII letter or `_`, then ASCII
/// alphanumerics or `_`.
pub fn valid_binding_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Structural built-ins that describe the render's own geometry and must never
/// be shadowed by host values.
pub fn structural_builtin_name(name: &str) -> bool {
    matches!(
        name,
        "name" | "content-path" | "output-path" | "template-path" | "loop"
    )
}

impl Bindings {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert (or overwrite) a binding. Returns
    /// [`BindingError::StructuralBuiltin`] for a structural built-in (checked
    /// first, so hyphenated structural names such as `content-path` are
    /// classified correctly) and [`BindingError::InvalidIdentifier`] for any
    /// other invalid name.
    pub fn set(&mut self, name: impl Into<String>, value: Value) -> Result<(), BindingError> {
        let name = name.into();
        if structural_builtin_name(&name) {
            return Err(BindingError::StructuralBuiltin);
        }
        if !valid_binding_identifier(&name) {
            return Err(BindingError::InvalidIdentifier);
        }
        self.map.insert(name, value);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.map.get(name)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.map.iter()
    }
}

/// Foundational precedence: a Context overlay wins over an Engine default.
pub fn resolve<'a>(defaults: &'a Bindings, context: &'a Context, name: &str) -> Option<&'a Value> {
    context.binding(name).or_else(|| defaults.get(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_validation() {
        for valid in ["a", "_", "abc123", "_under", "x_y"] {
            assert!(valid_binding_identifier(valid), "{valid:?} should be valid");
        }
        for invalid in ["", "9abc", "a-b", "a b", "a.b", "é", "has space"] {
            assert!(
                !valid_binding_identifier(invalid),
                "{invalid:?} should be invalid"
            );
        }
    }

    #[test]
    fn structural_builtins_are_rejected() {
        for builtin in [
            "name",
            "content-path",
            "output-path",
            "template-path",
            "loop",
        ] {
            assert!(structural_builtin_name(builtin));
        }
        assert!(!structural_builtin_name("site"));
        assert!(!structural_builtin_name("routes"));
    }

    #[test]
    fn set_validation() {
        let mut bindings = Bindings::new();
        assert!(bindings.set("app", Value::object()).is_ok());
        assert_eq!(
            bindings.set("9bad", Value::null()),
            Err(BindingError::InvalidIdentifier)
        );
        assert_eq!(
            bindings.set("name", Value::null()),
            Err(BindingError::StructuralBuiltin)
        );
        // Hyphenated structural built-ins are StructuralBuiltin, not
        // InvalidIdentifier.
        assert_eq!(
            bindings.set("content-path", Value::null()),
            Err(BindingError::StructuralBuiltin)
        );
        assert_eq!(bindings.len(), 1);
    }

    #[test]
    fn set_overwrites() {
        let mut bindings = Bindings::new();
        bindings.set("app", Value::number(1.0)).unwrap();
        bindings.set("app", Value::number(2.0)).unwrap();
        assert_eq!(bindings.get("app").and_then(|v| v.as_number()), Some(2.0));
        assert_eq!(bindings.len(), 1);
    }

    #[test]
    fn precedence_context_overlay_wins_over_default() {
        let mut defaults = Bindings::new();
        defaults.set("app", Value::string("default")).unwrap();
        let mut context = Context::new();
        context.set("app", Value::string("overlay")).unwrap();

        assert_eq!(
            resolve(&defaults, &context, "app").and_then(|v| v.as_str()),
            Some("overlay")
        );
        assert_eq!(
            resolve(&defaults, &Context::new(), "app").and_then(|v| v.as_str()),
            Some("default")
        );
        assert_eq!(resolve(&defaults, &context, "missing"), None);
    }

    #[test]
    fn context_alone_without_default() {
        let mut context = Context::new();
        context.set("page", Value::number(7.0)).unwrap();
        assert_eq!(
            resolve(&Bindings::new(), &context, "page").and_then(|v| v.as_number()),
            Some(7.0)
        );
        assert_eq!(resolve(&Bindings::new(), &context, "nope"), None);
    }
}
