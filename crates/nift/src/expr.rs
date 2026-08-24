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
/// contract, then navigation. `Ok(None)` means an unresolvable ROOT (no
/// binding, no contract); a known root with a missing member or an out-of-range
/// element is a hard `Err` (reference navigation semantics).
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
            None => {
                // Contract resolution (NR4 host capability; project-backed
                // sources arrive at NR8). Precedence: host binding > scoped
                // binding > contract > metadata (checked by the caller).
                if host.is_contract_name(root) {
                    match host.contract_source(root) {
                        Some(source) => {
                            let path = crate::parser::lexically_normal(&host.root().join(source));
                            host.read_json(&path).map_err(|e| {
                                RenderError::new(
                                    ErrorKind::Render,
                                    format!(
                                        "contract '{root}': failed to parse {source} ({})",
                                        e.message
                                    ),
                                )
                            })?
                        }
                        None => return Ok(None),
                    }
                } else {
                    return Ok(None);
                }
            }
        },
    };

    while pos < bytes.len() {
        let c = bytes[pos];
        if c == b'.' {
            let dot_pos = pos;
            pos += 1;
            let start = pos;
            while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
                pos += 1;
            }
            let member = &text[start..pos];
            match &current {
                Value::Object(map) => match map.get(member) {
                    Some(next) => current = next.clone(),
                    None => {
                        // Reference message: the path prefix up to the '.'.
                        return Err(RenderError::new(
                            ErrorKind::Render,
                            format!(
                                "JSON value '{}' has no member '{}'",
                                &text[..dot_pos],
                                member
                            ),
                        ));
                    }
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
                return Err(RenderError::new(
                    ErrorKind::Render,
                    format!("JSON array indices must be non-negative integers in '{text}'"),
                ));
            }
            let index = text[start..pos].parse::<usize>().unwrap_or(usize::MAX);
            pos += 1;
            match &current {
                Value::Array(array) => match array.get(index) {
                    Some(next) => current = next.clone(),
                    None => {
                        return Err(RenderError::new(
                            ErrorKind::Render,
                            format!("JSON array index {index} is out of range in '{text}'"),
                        ));
                    }
                },
                _ => {
                    return Err(RenderError::new(
                        ErrorKind::Render,
                        format!("cannot index JSON value in '{text}' because it is not an array"),
                    ));
                }
            }
        } else {
            return Err(RenderError::new(
                ErrorKind::Render,
                format!("invalid JSON access syntax in '{text}'"),
            ));
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
        let mut i = 0;
        while i < text.len() {
            let c = bytes[i];
            if quoted {
                if c == b'\\' && i + 1 < text.len() {
                    // Skip the escaped character so an escaped quote cannot
                    // terminate quote mode (unified with the other scanners).
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
            } else if c == b')' {
                parens -= 1;
                if parens == 0 && i + 1 != text.len() {
                    return false;
                }
                if parens < 0 {
                    return false;
                }
            }
            i += 1;
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

/// Split a parameter list on top-level commas, matching the reference
/// `parse_parameters`: quoted strings are unquoted and unescaped, whitespace is
/// collapsed (trailing whitespace trimmed), nesting is respected.
pub fn parse_parameters(text: &str) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut quote = 0u8;
    let mut parens = 0;
    let mut brackets = 0;
    let mut braces = 0;
    let mut significant_end = 0usize;
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        let c = bytes[i];
        if quoted {
            if c == b'\\' && i + 1 < text.len() {
                i += 1;
                let escaped = bytes[i];
                if escaped == b'$' {
                    current.push('\\');
                }
                current.push(escaped as char);
                significant_end = current.len();
            } else if c == quote {
                quoted = false;
            } else {
                current.push(c as char);
                significant_end = current.len();
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
            b')' => parens -= 1,
            b'[' => brackets += 1,
            b']' => brackets -= 1,
            b'{' => braces += 1,
            b'}' => braces -= 1,
            _ => {}
        }
        if parens < 0 || brackets < 0 || braces < 0 {
            return Vec::new();
        }
        if c == b',' && parens == 0 && brackets == 0 && braces == 0 {
            current.truncate(significant_end);
            result.push(std::mem::take(&mut current));
            significant_end = 0;
            i += 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            if !current.is_empty() {
                current.push(' ');
            }
        } else {
            current.push(c as char);
            significant_end = current.len();
        }
        i += 1;
    }
    if quoted || parens != 0 || brackets != 0 || braces != 0 {
        return Vec::new();
    }
    if !text.is_empty() || !current.is_empty() {
        current.truncate(significant_end);
        result.push(current);
    }
    result
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

/// Reserved binding names: built-in metadata, `loop`, and structural built-ins.
pub fn reserved_binding_name(name: &str) -> bool {
    matches!(
        name,
        "title"
            | "name"
            | "content-path"
            | "output-path"
            | "template-path"
            | "build-timezone"
            | "build-time"
            | "build-UTC-time"
            | "build-date"
            | "build-UTC-date"
            | "build-YYYY"
            | "build-YY"
            | "build-OS"
            | "loop"
    )
}

/// Collection operator evaluation (NR3): `@filter`/`@map`/`@sort`/`@slice`/
/// `@find`/`@some`/`@every`/`@distinct`/`@reverse`/`@sum`/`@prod`/`@min`/
/// `@max`/`@reduce`, with simple and advanced (`binding : collection =>
/// expression`) forms. Mirrors the reference; used as the `@for` collection
/// source and by the collection directives.
pub fn evaluate_collection_value(
    bindings: &mut JsonBindings,
    host: &dyn RenderHost,
    identity: &RenderIdentity,
    expression: &str,
) -> Result<Value, RenderError> {
    let text = expression.trim();
    if text.is_empty() {
        return Err(RenderError::new(
            ErrorKind::Render,
            "collection expression cannot be empty",
        ));
    }
    if !text.starts_with('@') {
        return evaluate_expression(bindings, host, identity, text);
    }

    let mut name_end = 1;
    let bytes = text.as_bytes();
    while name_end < text.len() && bytes[name_end].is_ascii_lowercase() {
        name_end += 1;
    }
    if name_end == 1 || name_end >= text.len() || bytes[name_end] != b'(' {
        return Err(RenderError::new(
            ErrorKind::Render,
            format!("malformed collection operation: {text}"),
        ));
    }
    let function = &text[1..name_end];
    let supported = [
        "filter", "map", "sort", "slice", "find", "some", "every", "distinct", "reverse", "sum",
        "prod", "min", "max", "reduce",
    ];
    if !supported.contains(&function) {
        return Err(RenderError::new(
            ErrorKind::Render,
            format!("unsupported collection operation: @{function}"),
        ));
    }

    let Some(close) = crate::parser::find_balanced(text, name_end, b'(', b')') else {
        return Err(RenderError::new(
            ErrorKind::Render,
            format!("malformed @{function} call"),
        ));
    };
    if close + 1 != text.len() {
        return Err(RenderError::new(
            ErrorKind::Render,
            format!("malformed @{function} call"),
        ));
    }
    let body = text[name_end + 1..close].trim().to_string();

    let error = |message: String| RenderError::new(ErrorKind::Render, message);

    let collection_arg = |bindings: &mut JsonBindings, raw: &str| -> Result<Value, RenderError> {
        let value = evaluate_collection_value(bindings, host, identity, raw)?;
        if !value.is_array() {
            return Err(error(format!(
                "{function}: collection must resolve to an array"
            )));
        }
        Ok(value)
    };

    let comparable = |a: &Value, b: &Value| -> Result<i32, RenderError> {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => Ok(if x < y {
                -1
            } else if x > y {
                1
            } else {
                0
            }),
            (Value::String(x), Value::String(y)) => Ok(if x < y {
                -1
            } else if x > y {
                1
            } else {
                0
            }),
            _ => Err(error(format!(
                "{function}: values must be numbers or strings of the same type"
            ))),
        }
    };

    // Simple forms.
    if function == "slice" {
        let params = parse_parameters(&body);
        if params.len() != 3 {
            return Err(error(
                "slice: expected collection, position and length".into(),
            ));
        }
        let source = collection_arg(bindings, &params[0])?;
        let index =
            |bindings: &mut JsonBindings, raw: &str, label: &str| -> Result<usize, RenderError> {
                let v = evaluate_expression(bindings, host, identity, raw)?;
                match v {
                    Value::Number(n) if n >= 0.0 && n.trunc() == n && n <= usize::MAX as f64 => {
                        Ok(n as usize)
                    }
                    _ => Err(error(format!(
                        "slice: {label} must be a non-negative integer"
                    ))),
                }
            };
        let pos = index(bindings, &params[1], "position")?;
        let len = index(bindings, &params[2], "length")?;
        let source_array = source.as_array().map(|a| a.to_vec()).unwrap_or_default();
        let end = (pos + len.min(source_array.len().saturating_sub(pos))).min(source_array.len());
        return Ok(Value::Array(if pos < source_array.len() {
            source_array[pos..end].to_vec()
        } else {
            Vec::new()
        }));
    }
    if function == "reverse" || function == "distinct" {
        let params = parse_parameters(&body);
        if params.len() != 1 {
            return Err(error(format!("{function}: expected one collection")));
        }
        let source = collection_arg(bindings, &params[0])?;
        let source_array = source.as_array().map(|a| a.to_vec()).unwrap_or_default();
        if function == "reverse" {
            return Ok(Value::Array(source_array.into_iter().rev().collect()));
        }
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        for item in source_array {
            let key = dump_compact(&item);
            if seen.insert(key) {
                result.push(item);
            }
        }
        return Ok(Value::Array(result));
    }
    if (function == "sort"
        || function == "sum"
        || function == "prod"
        || function == "min"
        || function == "max")
        && find_top_level(&body, "=>").is_none()
    {
        let params = parse_parameters(&body);
        if params.len() != 1 {
            return Err(error(format!(
                "{function}: expected one collection or binding : collection => expression"
            )));
        }
        let source = collection_arg(bindings, &params[0])?;
        let mut source_array = source.as_array().map(|a| a.to_vec()).unwrap_or_default();
        if function == "sort" {
            if source_array.is_empty() {
                return Ok(source);
            }
            let ty = std::mem::discriminant(&source_array[0]);
            if !source_array[0].is_number() && !source_array[0].is_string() {
                return Err(error(
                    "sort: simple form requires an array of numbers or strings".into(),
                ));
            }
            for item in &source_array {
                if std::mem::discriminant(item) != ty || (!item.is_number() && !item.is_string()) {
                    return Err(error(
                        "sort: values must all have the same sortable type".into(),
                    ));
                }
            }
            source_array.sort_by(|a, b| {
                let _ = comparable(a, b);
                if a.is_number() {
                    a.as_number()
                        .unwrap()
                        .partial_cmp(&b.as_number().unwrap())
                        .unwrap_or(std::cmp::Ordering::Equal)
                } else {
                    a.as_str().unwrap().cmp(b.as_str().unwrap())
                }
            });
            return Ok(Value::Array(source_array));
        }
        if function == "sum" || function == "prod" {
            let mut aggregate = if function == "sum" { 0.0 } else { 1.0 };
            for item in &source_array {
                let Some(n) = item.as_number() else {
                    return Err(error(format!("{function}: values must be numeric")));
                };
                aggregate = if function == "sum" {
                    aggregate + n
                } else {
                    aggregate * n
                };
                if !aggregate.is_finite() {
                    return Err(error(format!("{function}: result is not finite")));
                }
            }
            return Ok(Value::number(aggregate));
        }
        if source_array.is_empty() {
            return Err(error(format!(
                "{function}: cannot aggregate an empty collection"
            )));
        }
        let mut extreme = source_array[0].clone();
        if !extreme.is_number() && !extreme.is_string() {
            return Err(error(format!(
                "{function}: values must be numbers or strings"
            )));
        }
        for item in &source_array[1..] {
            let ordering = comparable(item, &extreme)?;
            if (function == "min" && ordering < 0) || (function == "max" && ordering > 0) {
                extreme = item.clone();
            }
        }
        return Ok(extreme);
    }

    // Advanced forms require `binding : collection => expression`.
    let Some(arrow) = find_top_level(&body, "=>") else {
        return Err(error(format!(
            "{function}: advanced form requires '=>' between binding/source and expression"
        )));
    };
    let left = body[..arrow].trim().to_string();
    let expr = body[arrow + 2..].trim().to_string();
    if expr.is_empty() {
        return Err(error("expression cannot be empty".into()));
    }

    if function == "reduce" {
        let Some(amp) = find_top_level(&left, "&") else {
            return Err(error(
                "reduce: expected binding : collection & accumulator = initial => expression"
                    .into(),
            ));
        };
        let (binding_text, source_text) = split_binding(&left[..amp], function)?;
        let accumulator_clause = left[amp + 1..].trim().to_string();
        let Some(equals) = find_top_level(&accumulator_clause, "=") else {
            return Err(error(
                "reduce: expected accumulator = initial expression".into(),
            ));
        };
        let accumulator = accumulator_clause[..equals].trim().to_string();
        let initial = accumulator_clause[equals + 1..].trim().to_string();
        let bindings_list = parse_bindings(&binding_text, function)?;
        if !crate::bindings::valid_binding_identifier(&accumulator)
            || reserved_binding_name(&accumulator)
        {
            return Err(error(
                "reduce: accumulator must be a non-reserved identifier".into(),
            ));
        }
        if bindings_list.contains(&accumulator) {
            return Err(error(
                "reduce: accumulator must be distinct from item bindings".into(),
            ));
        }
        let source = collection_arg(bindings, &source_text)?;
        let source_array = source.as_array().map(|a| a.to_vec()).unwrap_or_default();
        let mut accumulator_value = evaluate_expression(bindings, host, identity, &initial)?;
        for element in &source_array {
            accumulator_value = with_scoped(
                bindings,
                &bindings_list,
                &[(&accumulator, accumulator_value.clone())],
                |bindings| {
                    let mut next = Value::Null;
                    with_iteration_binding(
                        bindings,
                        &bindings_list,
                        element,
                        |bindings| -> Result<(), RenderError> {
                            next = evaluate_expression(bindings, host, identity, &expr)?;
                            Ok(())
                        },
                    )?;
                    Ok(next)
                },
            )?;
        }
        return Ok(accumulator_value);
    }

    let (binding_text, source_text) = split_binding(&left, function)?;
    let bindings_list = parse_bindings(&binding_text, function)?;
    let source = collection_arg(bindings, &source_text)?;
    let source_array = source.as_array().map(|a| a.to_vec()).unwrap_or_default();

    let mut descending = false;
    let mut expr_owned = expr.clone();
    if function == "sort" {
        if expr_owned.len() > 5 && expr_owned.ends_with(" desc") {
            descending = true;
            expr_owned = expr_owned[..expr_owned.len() - 5].trim().to_string();
        } else if expr_owned.len() > 4 && expr_owned.ends_with(" asc") {
            expr_owned = expr_owned[..expr_owned.len() - 4].trim().to_string();
        }
    }

    let mut result = match function {
        "filter" | "map" => Value::array(),
        "some" => Value::boolean(false),
        "every" => Value::boolean(true),
        "find" => Value::null(),
        "sum" | "prod" => Value::number(if function == "sum" { 0.0 } else { 1.0 }),
        _ => Value::null(),
    };
    let mut have_extreme = false;
    let mut keys: Vec<Value> = Vec::new();

    for element in &source_array {
        let evaluated = with_iteration_binding(bindings, &bindings_list, element, |bindings| {
            evaluate_expression(bindings, host, identity, &expr_owned)
        })?;
        match function {
            "filter" => {
                if truthy(&evaluated) {
                    result.as_array_mut().unwrap().push(element.clone());
                }
            }
            "map" => result.as_array_mut().unwrap().push(evaluated),
            "find" => {
                if truthy(&evaluated) {
                    return Ok(element.clone());
                }
            }
            "some" => {
                if truthy(&evaluated) {
                    return Ok(Value::boolean(true));
                }
            }
            "every" => {
                if !truthy(&evaluated) {
                    return Ok(Value::boolean(false));
                }
            }
            "sum" | "prod" => {
                let Some(n) = evaluated.as_number() else {
                    return Err(error(format!(
                        "{function}: expression must produce numeric values"
                    )));
                };
                let next = if function == "sum" {
                    result.as_number().unwrap() + n
                } else {
                    result.as_number().unwrap() * n
                };
                if !next.is_finite() {
                    return Err(error(format!("{function}: result is not finite")));
                }
                *result.as_number_mut().unwrap() = next;
            }
            "min" | "max" => {
                if !evaluated.is_number() && !evaluated.is_string() {
                    return Err(error(format!(
                        "{function}: expression must produce numbers or strings"
                    )));
                }
                if !have_extreme {
                    result = evaluated;
                    have_extreme = true;
                } else {
                    let ordering = comparable(&evaluated, &result)?;
                    if (function == "min" && ordering < 0) || (function == "max" && ordering > 0) {
                        result = evaluated;
                    }
                }
            }
            _ => keys.push(evaluated),
        }
    }

    if function != "sort" {
        if (function == "min" || function == "max") && !have_extreme {
            return Err(error(format!(
                "{function}: cannot aggregate an empty collection"
            )));
        }
        return Ok(result);
    }

    if keys.is_empty() {
        return Ok(source);
    }
    let ty = std::mem::discriminant(&keys[0]);
    if !keys[0].is_number() && !keys[0].is_string() {
        return Err(error("sort: keys must be numbers or strings".into()));
    }
    for key in &keys {
        if std::mem::discriminant(key) != ty {
            return Err(error("sort: keys must all have the same type".into()));
        }
    }
    let mut order: Vec<usize> = (0..keys.len()).collect();
    order.sort_by(|&a, &b| {
        let ordering = comparable(&keys[a], &keys[b]).unwrap_or(0);
        let ordering = if descending { -ordering } else { ordering };
        ordering.cmp(&0)
    });
    Ok(Value::Array(
        order.into_iter().map(|i| source_array[i].clone()).collect(),
    ))
}

fn split_binding(raw: &str, function: &str) -> Result<(String, String), RenderError> {
    match find_top_level(raw, ":") {
        Some(pos) => {
            let binding_text = raw[..pos].trim().to_string();
            let collection_text = raw[pos + 1..].trim().to_string();
            if binding_text.is_empty() || collection_text.is_empty() {
                Err(RenderError::new(
                    ErrorKind::Render,
                    format!("{function}: expected binding : collection"),
                ))
            } else {
                Ok((binding_text, collection_text))
            }
        }
        None => Err(RenderError::new(
            ErrorKind::Render,
            format!("{function}: expected binding : collection"),
        )),
    }
}

fn parse_bindings(raw: &str, function: &str) -> Result<Vec<String>, RenderError> {
    let binding = raw.trim().to_string();
    let mut bindings: Vec<String> =
        if binding.len() >= 2 && binding.starts_with('(') && binding.ends_with(')') {
            let inner = &binding[1..binding.len() - 1];
            let parsed = parse_parameters(inner);
            if parsed.len() < 2 {
                return Err(RenderError::new(
                    ErrorKind::Render,
                    format!("{function}: tuple binding requires at least two identifiers"),
                ));
            }
            parsed
        } else {
            vec![binding.clone()]
        };
    let mut seen = std::collections::HashSet::new();
    for name in &mut bindings {
        *name = name.trim().to_string();
        if !crate::bindings::valid_binding_identifier(name) {
            return Err(RenderError::new(
                ErrorKind::Render,
                format!("{function}: binding must be an identifier"),
            ));
        }
        if reserved_binding_name(name) {
            return Err(RenderError::new(
                ErrorKind::Render,
                format!("{function}: binding conflicts with a reserved namespace: {name}"),
            ));
        }
        if !seen.insert(name.clone()) {
            return Err(RenderError::new(
                ErrorKind::Render,
                format!("{function}: bindings must be distinct identifiers"),
            ));
        }
    }
    Ok(bindings)
}

/// Bind a single element (or tuple of elements) under `bindings` for one
/// iteration, then restore prior bindings.
fn with_iteration_binding<T>(
    bindings: &mut JsonBindings,
    names: &[String],
    element: &Value,
    apply: impl FnOnce(&mut JsonBindings) -> Result<T, RenderError>,
) -> Result<T, RenderError> {
    let prior: Vec<(String, Option<Value>)> = names
        .iter()
        .map(|name| (name.clone(), bindings.get(name).cloned()))
        .collect();
    if names.len() == 1 {
        bindings.insert(names[0].clone(), element.clone());
    } else {
        let Value::Array(array) = element else {
            return Err(RenderError::new(
                ErrorKind::Render,
                "tuple binding arity must match each array item",
            ));
        };
        if array.len() != names.len() {
            return Err(RenderError::new(
                ErrorKind::Render,
                "tuple binding arity must match each array item",
            ));
        }
        for (name, value) in names.iter().zip(array.iter()) {
            bindings.insert(name.clone(), value.clone());
        }
    }
    let result = apply(bindings);
    for (name, value) in prior {
        match value {
            Some(value) => bindings.insert(name, value),
            None => bindings.shift_remove(&name),
        };
    }
    result
}

/// Bind extra scoped values (e.g. a reduce accumulator) alongside `names`.
fn with_scoped<T>(
    bindings: &mut JsonBindings,
    names: &[String],
    extra: &[(&str, Value)],
    apply: impl FnOnce(&mut JsonBindings) -> Result<T, RenderError>,
) -> Result<T, RenderError> {
    let mut extra_keys: Vec<(&str, Option<Value>)> = extra
        .iter()
        .map(|(name, _)| (*name, bindings.get(*name).cloned()))
        .collect();
    let prior_names: Vec<(String, Option<Value>)> = names
        .iter()
        .map(|name| (name.clone(), bindings.get(name).cloned()))
        .collect();
    for (name, value) in extra {
        bindings.insert((*name).to_string(), value.clone());
    }
    for name in names {
        bindings.insert(name.clone(), Value::Null);
    }
    let result = apply(bindings);
    for (name, value) in extra_keys.drain(..) {
        match value {
            Some(value) => bindings.insert(name.to_string(), value),
            None => bindings.shift_remove(name),
        };
    }
    for (name, value) in prior_names {
        match value {
            Some(value) => bindings.insert(name, value),
            None => bindings.shift_remove(&name),
        };
    }
    result
}

/// Compact-ish JSON dump matching the reference `dump(0)`: strings/number/
/// bool/null scalars inline; arrays/objects use newlines (indent level 0).
pub fn dump_compact(value: &Value) -> String {
    let mut out = String::new();
    write_dump(&mut out, value);
    out
}

fn write_dump(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&crate::parser::format_number(*n)),
        Value::String(s) => {
            out.push('"');
            write_escaped(out, s);
            out.push('"');
        }
        Value::Array(array) => {
            out.push('[');
            if !array.is_empty() {
                out.push('\n');
                for (i, item) in array.iter().enumerate() {
                    write_dump(out, item);
                    if i + 1 != array.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
            }
            out.push(']');
        }
        Value::Object(object) => {
            out.push('{');
            if !object.is_empty() {
                out.push('\n');
                for (i, (key, item)) in object.iter().enumerate() {
                    out.push('"');
                    write_escaped(out, key);
                    out.push_str("\": ");
                    write_dump(out, item);
                    if i + 1 != object.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
            }
            out.push('}');
        }
    }
}

fn write_escaped(out: &mut String, value: &str) {
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
}

/// Scan from the start of `text` (which begins immediately after `$[`) for the
/// matching `]`, honouring nested brackets and quoted strings. Returns the
/// relative position of the closing `]`.
pub fn scan_balanced_bracket(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut nested = 0usize;
    let mut quoted = false;
    let mut quote = 0u8;
    let mut i = 0;
    while i < text.len() {
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
            nested += 1;
        } else if c == b']' {
            if nested == 0 {
                return Some(i);
            }
            nested -= 1;
        }
        i += 1;
    }
    None
}
