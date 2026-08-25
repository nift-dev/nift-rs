//! The Nift value data model (NR1).
//!
//! `Value` is a JSON-compatible tree: null, bool, number (f64, matching the
//! frozen reference's double numbers), string, array and object.
//!
//! Object member order is **insertion/document order** (`indexmap::IndexMap`),
//! matching the frozen reference, whose JSON documents store object members as
//! an ordered sequence and iterate them in document order. This matters for
//! template rendering once objects are iterated (NR2+). Duplicate JSON object
//! keys are rejected by the reference *at parse time*; that rule belongs to the
//! JSON parser (NR4), not to this data model, where `insert` overwrites.
//!
//! Deliberate Rust API deviations from the frozen C++ `Value` (documented,
//! observable template semantics are unaffected):
//! - mutating member/element access returns `Result` (e.g.
//!   [`Value::insert`]/[`Value::push`]) instead of throwing
//!   `std::runtime_error`; the panic policy forbids panics on any input.
//! - typed reads return `Option` instead of silently returning type defaults.
//! - `Value` is an owned tree (Rust ownership), so "deep copy" is structural
//!   and moves do not leave a valid-null source behind.

use indexmap::IndexMap;
use std::fmt;

/// A JSON-compatible Nift value.
#[derive(Debug, Clone, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    Object(IndexMap<String, Value>),
}

/// Why a mutating member/element operation on a `Value` was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueError {
    /// Member access on a non-object, non-null value.
    NotObject,
    /// Element access on a non-array value.
    NotArray,
}

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueError::NotObject => f.write_str("value is not an object"),
            ValueError::NotArray => f.write_str("value is not an array"),
        }
    }
}

impl std::error::Error for ValueError {}

impl Value {
    pub fn null() -> Self {
        Value::Null
    }

    pub fn boolean(value: bool) -> Self {
        Value::Bool(value)
    }

    pub fn number(value: f64) -> Self {
        Value::Number(value)
    }

    pub fn string(value: impl Into<String>) -> Self {
        Value::String(value.into())
    }

    pub fn array() -> Self {
        Value::Array(Vec::new())
    }

    pub fn object() -> Self {
        Value::Object(IndexMap::new())
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
    pub fn is_bool(&self) -> bool {
        matches!(self, Value::Bool(_))
    }
    pub fn is_number(&self) -> bool {
        matches!(self, Value::Number(_))
    }
    pub fn is_string(&self) -> bool {
        matches!(self, Value::String(_))
    }
    pub fn is_array(&self) -> bool {
        matches!(self, Value::Array(_))
    }
    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }

    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Bool(value) = self {
            Some(*value)
        } else {
            None
        }
    }
    pub fn as_number(&self) -> Option<f64> {
        if let Value::Number(value) = self {
            Some(*value)
        } else {
            None
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Value::String(value) = self {
            Some(value)
        } else {
            None
        }
    }
    pub fn as_array(&self) -> Option<&[Value]> {
        if let Value::Array(value) = self {
            Some(value)
        } else {
            None
        }
    }
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Value>> {
        if let Value::Array(value) = self {
            Some(value)
        } else {
            None
        }
    }
    pub fn as_object(&self) -> Option<&IndexMap<String, Value>> {
        if let Value::Object(value) = self {
            Some(value)
        } else {
            None
        }
    }
    /// Mutable number read.
    pub fn as_number_mut(&mut self) -> Option<&mut f64> {
        if let Value::Number(value) = self {
            Some(value)
        } else {
            None
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut IndexMap<String, Value>> {
        if let Value::Object(value) = self {
            Some(value)
        } else {
            None
        }
    }

    /// Number of elements for an array/object, `None` for other types.
    pub fn len(&self) -> Option<usize> {
        match self {
            Value::Array(value) => Some(value.len()),
            Value::Object(value) => Some(value.len()),
            _ => None,
        }
    }

    /// True for an empty array/object.
    pub fn is_empty(&self) -> Option<bool> {
        self.len().map(|len| len == 0)
    }

    /// Member read. `None` when the value is not an object or the key is
    /// absent.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object()?.get(key)
    }

    /// Member read (mutable). `None` when the value is not an object or the
    /// key is absent.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Value> {
        self.as_object_mut()?.get_mut(key)
    }

    /// Element read. `None` when the value is not an array or the index is out
    /// of range.
    pub fn at(&self, index: usize) -> Option<&Value> {
        self.as_array()?.get(index)
    }

    /// Element read (mutable).
    pub fn at_mut(&mut self, index: usize) -> Option<&mut Value> {
        self.as_array_mut()?.get_mut(index)
    }

    /// Insert a member, overwriting an existing key. Matching the frozen
    /// reference, a `Null` is materialised into an Object first; any other
    /// non-object type is a [`ValueError::NotObject`]. Never panics.
    pub fn insert(&mut self, key: impl Into<String>, value: Value) -> Result<(), ValueError> {
        let map = match self {
            Value::Null => {
                *self = Value::Object(IndexMap::new());
                if let Value::Object(map) = self {
                    map
                } else {
                    return Err(ValueError::NotObject);
                }
            }
            Value::Object(map) => map,
            _ => return Err(ValueError::NotObject),
        };
        map.insert(key.into(), value);
        Ok(())
    }

    /// Append an element. Requires an Array; a `Null` is never materialised
    /// into an Array (matching the frozen reference, where element access and
    /// `push_back` require an Array). Never panics.
    pub fn push(&mut self, value: Value) -> Result<(), ValueError> {
        if let Value::Array(array) = self {
            array.push(value);
            Ok(())
        } else {
            Err(ValueError::NotArray)
        }
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::boolean(value)
    }
}
impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Value::number(value as f64)
    }
}
impl From<u64> for Value {
    fn from(value: u64) -> Self {
        Value::number(value as f64)
    }
}
impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Value::number(value)
    }
}
impl From<String> for Value {
    fn from(value: String) -> Self {
        Value::string(value)
    }
}
impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Value::string(value)
    }
}
impl From<Vec<Value>> for Value {
    fn from(value: Vec<Value>) -> Self {
        Value::Array(value)
    }
}
impl From<IndexMap<String, Value>> for Value {
    fn from(value: IndexMap<String, Value>) -> Self {
        Value::Object(value)
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => a == b,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Array(a), Value::Array(b)) => a == b,
            (Value::Object(a), Value::Object(b)) => {
                // Map equality: same keys mapping to equal values, independent
                // of member order (member order is still preserved for
                // iteration).
                a.len() == b.len() && a.iter().all(|(key, value)| b.get(key) == Some(value))
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_and_type_predicates() {
        assert!(Value::null().is_null());
        assert!(Value::boolean(true).is_bool());
        assert!(Value::number(3.5).is_number());
        assert!(Value::string("x").is_string());
        assert!(Value::array().is_array());
        assert!(Value::object().is_object());
        assert!(!Value::null().is_object());
        assert!(Value::default().is_null());
    }

    #[test]
    fn typed_reads_return_options() {
        assert_eq!(Value::boolean(true).as_bool(), Some(true));
        assert_eq!(Value::number(2.0).as_number(), Some(2.0));
        assert_eq!(Value::string("s").as_str(), Some("s"));
        assert_eq!(Value::null().as_bool(), None);
        assert_eq!(Value::string("s").as_number(), None);
        assert_eq!(Value::null().as_str(), None);
    }

    #[test]
    fn structured_construction_documented_example() {
        let mut user = Value::object();
        user.insert("name", Value::string("Nick")).unwrap();
        let mut projects = Value::array();
        projects.push(Value::string("nift")).unwrap();
        projects.push(Value::string("tscc")).unwrap();
        user.insert("projects", projects).unwrap();
        assert_eq!(user.get("name").and_then(|v| v.as_str()), Some("Nick"));
        assert_eq!(
            user.get("projects")
                .and_then(|v| v.at(1))
                .and_then(|v| v.as_str()),
            Some("tscc")
        );
    }

    #[test]
    fn insert_materialises_null_into_object() {
        let mut value = Value::null();
        value.insert("a", Value::number(1.0)).unwrap();
        assert!(value.is_object());
        assert_eq!(value.get("a").and_then(|v| v.as_number()), Some(1.0));
    }

    #[test]
    fn insert_overwrites_existing_key() {
        let mut object = Value::object();
        object.insert("a", Value::number(1.0)).unwrap();
        object.insert("a", Value::number(2.0)).unwrap();
        assert_eq!(object.get("a").and_then(|v| v.as_number()), Some(2.0));
        assert_eq!(object.len(), Some(1));
    }

    #[test]
    fn insert_rejects_non_objects() {
        // Member insert on any non-object, non-null value (including an
        // array) is NotObject.
        let mut value = Value::number(1.0);
        assert_eq!(value.insert("a", Value::null()), Err(ValueError::NotObject));
        let mut array = Value::array();
        assert_eq!(array.insert("a", Value::null()), Err(ValueError::NotObject));
        let mut text = Value::string("s");
        assert_eq!(text.insert("a", Value::null()), Err(ValueError::NotObject));
    }

    #[test]
    fn push_requires_array() {
        let mut value = Value::object();
        assert_eq!(value.push(Value::null()), Err(ValueError::NotArray));
        let mut array = Value::array();
        array.push(Value::boolean(true)).unwrap();
        array.push(Value::boolean(false)).unwrap();
        assert_eq!(array.len(), Some(2));
        assert_eq!(array.at(1).and_then(|v| v.as_bool()), Some(false));
    }

    #[test]
    fn object_member_order_is_insertion_order() {
        let mut object = Value::object();
        object.insert("zebra", Value::null()).unwrap();
        object.insert("apple", Value::null()).unwrap();
        object.insert("mango", Value::null()).unwrap();
        let keys: Vec<&str> = object
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        assert_eq!(keys, ["zebra", "apple", "mango"]);
    }

    #[test]
    fn equality_is_value_semantics_and_order_insensitive_for_objects() {
        assert_eq!(Value::number(1.0), Value::number(1.0));
        assert_ne!(Value::number(1.0), Value::string("1"));

        let mut a = Value::object();
        a.insert("x", Value::number(1.0)).unwrap();
        a.insert("y", Value::number(2.0)).unwrap();
        let mut b = Value::object();
        b.insert("y", Value::number(2.0)).unwrap();
        b.insert("x", Value::number(1.0)).unwrap();
        assert_eq!(a, b);

        b.insert("z", Value::null()).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn clone_is_structural() {
        let mut original = Value::object();
        original.insert("a", Value::number(1.0)).unwrap();
        let copy = original.clone();
        assert_eq!(original, copy);
        let mut mutated = original.clone();
        mutated.insert("a", Value::number(99.0)).unwrap();
        assert_ne!(original, mutated);
    }

    #[test]
    fn from_impls() {
        let value: Value = true.into();
        assert_eq!(value.as_bool(), Some(true));
        let value: Value = 3_i64.into();
        assert_eq!(value.as_number(), Some(3.0));
        let value: Value = "text".into();
        assert_eq!(value.as_str(), Some("text"));
    }
}
