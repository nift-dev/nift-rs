//! Expression, condition and collection evaluation (NR3).
//!
//! This module implements the full NR2/NR3 expression surface used by
//! `$[...]`, `@if` and the collection operators (`@filter`/`@map`/...):
//! value/binding/metadata resolution, scalar literals, arithmetic, comparison
//! and logical operators, and the collection functions. The parser routes all
//! external state through [`RenderHost`] and the per-render `json_bindings`
//! map (scoped `@for` bindings and `loop` metadata); nothing here touches IO
//! directly.
//!
//! Semantics mirror the frozen C++ reference exactly; differential tests pin
//! the observable behaviour.

use crate::error::{ErrorKind, RenderError};
use crate::host::{RenderHost, RenderIdentity};
use crate::parser::{built_in_metadata_name, metadata, truthy};
use crate::value::Value;
use indexmap::IndexMap;

/// Per-render scoped value bindings (from `@for` loops and, later, `@json`).
pub type JsonBindings = IndexMap<String, Value>;

/// Save the current values of `keys`, apply a mutation closure, then restore.
/// `apply` receives the same borrow the caller mutates.
pub fn with_bindings<T>(
    bindings: &mut JsonBindings,
    keys: &[&str],
    apply: impl FnOnce(&mut JsonBindings) -> T,
) -> T {
    let prior: Vec<(String, Option<Value>)> = keys
        .iter()
        .map(|key| (key.to_string(), bindings.get(*key).cloned()))
        .collect();
    let result = apply(bindings);
    for (key, value) in prior {
        match value {
            Some(value) => bindings.insert(key, value),
            None => bindings.shift_remove(&key),
        };
    }
    result
}

/// Resolve a JSON value path: host binding, then scoped bindings, then
/// navigation. `Ok(None)` means unresolvable (no binding, missing member or
/// out-of-range element); a hard type error (member/element access on the
/// wrong value type) is an `Err`.
pub fn resolve_json_value(
    bindings: &JsonBindings,
    host: &dyn RenderHost,
    text: &str,
) -> Result<Option<Value>, RenderError> {
    let bytes = text.as_bytes();
    let mut pos = 0;
    if bytes.is_empty() || !(bytes[0].is_ascii_alphabetic() || bytes[0] == b'_') {
        return Ok(None);
    }
    pos += 1;
    while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
        pos += 1;
    }
    let root = &text[..pos];

    let mut current = match host.binding(root) {
        Some(value) => value.clone(),
        None => match bindings.get(root) {
            Some(value) => value.clone(),
            None => return Ok(None),
        },
    };

    while pos < bytes.len() {
        let c = bytes[pos];
        if c == b'.' {
            pos += 1;
            let start = pos;
            while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
                pos += 1;
            }
            let member = &text[start..pos];
            match &current {
                Value::Object(map) => match map.get(member) {
                    Some(next) => current = next.clone(),
                    None => return Ok(None),
                },
                _ => {
                    return Err(RenderError::new(
                        ErrorKind::Render,
                        format!(
                            "cannot access member '{member}' because the current JSON value is not an object"
                        ),
                    ));
                }
            }
        } else if c == b'[' {
            pos += 1;
            let start = pos;
            while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                pos += 1;
            }
            if pos == start || pos >= bytes.len() || bytes[pos] != b']' {
                return Ok(None);
            }
            let index = text[start..pos].parse::<usize>().unwrap_or(usize::MAX);
            pos += 1;
            match &current {
                Value::Array(array) => match array.get(index) {
                    Some(next) => current = next.clone(),
                    None => return Ok(None),
                },
                _ => {
                    return Err(RenderError::new(
                        ErrorKind::Render,
                        format!(
                            "cannot access element {index} because the current JSON value is not an array"
                        ),
                    ));
                }
            }
        } else {
            return Ok(None);
        }
    }

    Ok(Some(current))
}

/// Full expression evaluation (NR3): literals, bindings/metadata, arithmetic,
/// comparison, logical and unary operators, mirroring the reference's
/// `evaluate_expression`. `$[...]` and `@if` both route through here.
pub fn evaluate_expression(
    bindings: &JsonBindings,
    host: &dyn RenderHost,
    identity: &RenderIdentity,
    expression: &str,
) -> Result<Value, RenderError> {
    fn is_compound(text: &str) -> bool {
        [
            "&&", "||", "==", "!=", "<=", ">=", " + ", " - ", " * ", " / ", " % ", " < ", " > ",
        ]
        .iter()
        .any(|op| text.contains(op))
    }

    fn resolve_direct(
        bindings: &JsonBindings,
        host: &dyn RenderHost,
        identity: &RenderIdentity,
        raw: &str,
    ) -> Result<Option<Value>, RenderError> {
        let text = raw.trim();
        match resolve_json_value(bindings, host, text) {
            Ok(Some(value)) => return Ok(Some(value)),
            Ok(None) => {}
            Err(error) => {
                // A hard resolution error (e.g. member access on a non-object)
                // propagates unless the expression is compound, in which case
                // the operator path re-evaluates the operands and surfaces the
                // error there if it still matters.
                if !is_compound(text) {
                    return Err(error);
                }
            }
        }
        if built_in_metadata_name(text) {
            if let Some(value) = metadata(host, identity, text) {
                return Ok(Some(Value::string(value)));
            }
        }
        Ok(scalar_literal(text))
    }

    fn encloses(text: &str) -> bool {
        if text.len() < 2 || !text.starts_with('(') || !text.ends_with(')') {
            return false;
        }
        let bytes = text.as_bytes();
        let mut quoted = false;
        let mut quote = 0u8;
        let mut parens = 0;
        let mut brackets = 0;
        for (i, &c) in bytes.iter().enumerate() {
            if quoted {
                if c == b'\\' && i + 1 < text.len() {
                    // skip
                } else if c == quote {
                    quoted = false;
                }
                continue;
            }
            if c == b'\'' || c == b'"' {
                quoted = true;
                quote = c;
                continue;
            }
            if c == b'[' {
                brackets += 1;
                continue;
            }
            if c == b']' {
                if brackets > 0 {
                    brackets -= 1;
                }
                continue;
            }
            if brackets > 0 {
                continue;
            }
            if c == b'(' {
                parens += 1;
            } else if c == b')' {
                parens -= 1;
                if parens == 0 && i + 1 != text.len() {
                    return false;
                }
                if parens < 0 {
                    return false;
                }
            }
        }
        parens == 0 && !quoted
    }

    fn find_top_level_op(text: &str, op: &str) -> Option<usize> {
        let bytes = text.as_bytes();
        let mut quoted = false;
        let mut quote = 0u8;
        let mut parens = 0;
        let mut brackets = 0;
        let mut i = 0;
        while i + op.len() <= text.len() {
            let c = bytes[i];
            if quoted {
                if c == b'\\' && i + 1 < text.len() {
                    i += 1;
                } else if c == quote {
                    quoted = false;
                }
                i += 1;
                continue;
            }
            if c == b'\'' || c == b'"' {
                quoted = true;
                quote = c;
                i += 1;
                continue;
            }
            if c == b'[' {
                brackets += 1;
                i += 1;
                continue;
            }
            if c == b']' {
                if brackets > 0 {
                    brackets -= 1;
                }
                i += 1;
                continue;
            }
            if brackets > 0 {
                i += 1;
                continue;
            }
            if c == b'(' {
                parens += 1;
                i += 1;
                continue;
            }
            if c == b')' {
                if parens > 0 {
                    parens -= 1;
                }
                i += 1;
                continue;
            }
            if parens == 0 && text[i..].starts_with(op) {
                return Some(i);
            }
            i += 1;
        }
        None
    }

    fn eval(
        bindings: &JsonBindings,
        host: &dyn RenderHost,
        identity: &RenderIdentity,
        raw: &str,
    ) -> Result<Value, RenderError> {
        let mut text = raw.trim().to_string();
        if text.is_empty() {
            return Err(RenderError::new(
                ErrorKind::Parse,
                "expression cannot be empty",
            ));
        }
        while encloses(&text) {
            text = text[1..text.len() - 1].trim().to_string();
        }

        // Direct resolution first (metadata/JSON names preserved before
        // interpreting punctuation as arithmetic).
        if let Some(value) = resolve_direct(bindings, host, identity, &text)? {
            return Ok(value);
        }

        if let Some(pos) = find_top_level_op(&text, "||") {
            let left = eval(bindings, host, identity, &text[..pos])?;
            if truthy(&left) {
                return Ok(Value::boolean(true));
            }
            let right = eval(bindings, host, identity, &text[pos + 2..])?;
            return Ok(Value::boolean(truthy(&right)));
        }
        if let Some(pos) = find_top_level_op(&text, "&&") {
            let left = eval(bindings, host, identity, &text[..pos])?;
            if !truthy(&left) {
                return Ok(Value::boolean(false));
            }
            let right = eval(bindings, host, identity, &text[pos + 2..])?;
            return Ok(Value::boolean(truthy(&right)));
        }
        for op in ["==", "!=", "<=", ">=", "<", ">"] {
            if let Some(pos) = find_top_level_op(&text, op) {
                let left = eval(bindings, host, identity, &text[..pos])?;
                let right = eval(bindings, host, identity, &text[pos + op.len()..])?;
                if op == "==" || op == "!=" {
                    let equal = scalar_equal(&left, &right)?;
                    return Ok(Value::boolean(if op == "==" { equal } else { !equal }));
                }
                let ordering = scalar_ordering(&left, &right)?;
                let result = match op {
                    "<" => ordering < 0,
                    "<=" => ordering <= 0,
                    ">" => ordering > 0,
                    _ => ordering >= 0,
                };
                return Ok(Value::boolean(result));
            }
        }
        if text.starts_with('!') && !text.starts_with("!=") {
            let operand = eval(bindings, host, identity, &text[1..])?;
            return Ok(Value::boolean(!truthy(&operand)));
        }

        // Binary arithmetic: scan right-to-left for + - then * / %.
        if let Some(pos) =
            find_binary_arithmetic(&text, "+-").or_else(|| find_binary_arithmetic(&text, "*/%"))
        {
            let left = eval(bindings, host, identity, &text[..pos])?;
            let right = eval(bindings, host, identity, &text[pos + 1..])?;
            let (Value::Number(a), Value::Number(b)) = (&left, &right) else {
                return Err(RenderError::new(
                    ErrorKind::Render,
                    "arithmetic operators require numeric operands",
                ));
            };
            let op = text.as_bytes()[pos] as char;
            let result = match op {
                '+' => a + b,
                '-' => a - b,
                '*' => a * b,
                '/' => {
                    if *b == 0.0 {
                        return Err(RenderError::new(ErrorKind::Render, "division by zero"));
                    }
                    a / b
                }
                '%' => {
                    if *b == 0.0 {
                        return Err(RenderError::new(ErrorKind::Render, "modulo by zero"));
                    }
                    if a.trunc() != *a || b.trunc() != *b {
                        return Err(RenderError::new(
                            ErrorKind::Render,
                            "modulo requires integer-valued operands",
                        ));
                    }
                    a % b
                }
                _ => unreachable!(),
            };
            if !result.is_finite() {
                return Err(RenderError::new(
                    ErrorKind::Render,
                    "arithmetic result is not finite",
                ));
            }
            return Ok(Value::number(result));
        }

        // Unary +/-
        if (text.starts_with('+') || text.starts_with('-')) && text.len() > 1 {
            let operand = eval(bindings, host, identity, &text[1..])?;
            match operand {
                Value::Number(n) => {
                    return Ok(Value::number(if text.starts_with('-') { -n } else { n }));
                }
                _ => {
                    return Err(RenderError::new(
                        ErrorKind::Render,
                        "unary arithmetic operators require a numeric operand",
                    ));
                }
            }
        }

        Err(RenderError::new(
            ErrorKind::Render,
            format!("unknown value or malformed expression: {text}"),
        ))
    }

    eval(bindings, host, identity, expression)
}

/// `==`/`!=` scalar comparison (matching the reference; non-scalar types are an
/// error).
fn scalar_equal(left: &Value, right: &Value) -> Result<bool, RenderError> {
    if std::mem::discriminant(left) != std::mem::discriminant(right) {
        return Ok(false);
    }
    match (left, right) {
        (Value::Null, Value::Null) => Ok(true),
        (Value::Bool(a), Value::Bool(b)) => Ok(a == b),
        (Value::Number(a), Value::Number(b)) => Ok(a == b),
        (Value::String(a), Value::String(b)) => Ok(a == b),
        _ => Err(RenderError::new(
            ErrorKind::Render,
            "comparisons are only supported for scalar values",
        )),
    }
}

/// Ordering comparison; requires two numbers or two strings of the same type.
fn scalar_ordering(left: &Value, right: &Value) -> Result<i32, RenderError> {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => Ok(if a < b {
            -1
        } else if a > b {
            1
        } else {
            0
        }),
        (Value::String(a), Value::String(b)) => Ok(if a < b {
            -1
        } else if a > b {
            1
        } else {
            0
        }),
        _ => Err(RenderError::new(
            ErrorKind::Render,
            "ordering comparisons require two numbers or two strings of the same type",
        )),
    }
}

/// Right-to-left top-level scan for binary arithmetic operators, skipping
/// unary +/- (a preceding operator character).
fn find_binary_arithmetic(text: &str, ops: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut quoted = false;
    let mut quote = 0u8;
    let mut parens = 0;
    let mut brackets = 0;
    let mut i = text.len();
    while i > 0 {
        i -= 1;
        let c = bytes[i];
        if quoted {
            if c == quote && (i == 0 || bytes[i - 1] != b'\\') {
                quoted = false;
            }
            continue;
        }
        if c == b'\'' || c == b'"' {
            quoted = true;
            quote = c;
            continue;
        }
        if c == b']' {
            brackets += 1;
            continue;
        }
        if c == b'[' {
            if brackets > 0 {
                brackets -= 1;
            }
            continue;
        }
        if brackets > 0 {
            continue;
        }
        if c == b')' {
            parens += 1;
            continue;
        }
        if c == b'(' {
            if parens > 0 {
                parens -= 1;
            }
            continue;
        }
        if parens > 0 || !ops.contains(c as char) {
            continue;
        }
        if (c == b'+' || c == b'-') && (i == 0 || "+-*/%(<>=!&|?:,".contains(bytes[i - 1] as char))
        {
            continue;
        }
        return Some(i);
    }
    None
}

/// A scalar literal: true/false/null, a quoted string, or a number.
pub fn scalar_literal(text: &str) -> Option<Value> {
    match text {
        "true" => return Some(Value::boolean(true)),
        "false" => return Some(Value::boolean(false)),
        "null" => return Some(Value::null()),
        _ => {}
    }
    let quoted = text.len() >= 2
        && ((text.starts_with('"') && text.ends_with('"'))
            || (text.starts_with('\'') && text.ends_with('\'')));
    if quoted {
        return Some(Value::string(&text[1..text.len() - 1]));
    }
    text.parse::<f64>().ok().map(Value::number)
}

/// Split a parameter list on top-level commas (respecting nesting/quotes).
pub fn parse_parameters(text: &str) -> Vec<String> {
    let mut params = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    let mut quoted = false;
    let mut quote = 0u8;
    let bytes = text.as_bytes();
    for (i, &c) in bytes.iter().enumerate() {
        if quoted {
            if c == b'\\' && i + 1 < text.len() {
                // skip next via loop
            } else if c == quote {
                quoted = false;
            }
            continue;
        }
        if c == b'\'' || c == b'"' {
            quoted = true;
            quote = c;
            continue;
        }
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            b',' if depth == 0 => {
                params.push(text[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    params.push(text[start..].trim().to_string());
    params
}

/// Top-level position of `needle`, or `None`.
pub fn find_top_level(text: &str, needle: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut quoted = false;
    let mut quote = 0u8;
    let mut parens = 0;
    let mut brackets = 0;
    let mut braces = 0;
    let mut i = 0;
    while i + needle.len() <= text.len() {
        let c = bytes[i];
        if quoted {
            if c == b'\\' && i + 1 < text.len() {
                i += 1;
            } else if c == quote {
                quoted = false;
            }
            i += 1;
            continue;
        }
        if c == b'\'' || c == b'"' {
            quoted = true;
            quote = c;
            i += 1;
            continue;
        }
        match c {
            b'(' => parens += 1,
            b')' => {
                if parens > 0 {
                    parens -= 1;
                }
            }
            b'[' => brackets += 1,
            b']' => {
                if brackets > 0 {
                    brackets -= 1;
                }
            }
            b'{' => braces += 1,
            b'}' if braces > 0 => braces -= 1,
            _ => {}
        }
        if parens == 0 && brackets == 0 && braces == 0 && text[i..].starts_with(needle) {
            return Some(i);
        }
        i += 1;
    }
    None
}
