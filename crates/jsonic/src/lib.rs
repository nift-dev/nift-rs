//! `jsonic` - native Rust JSON implementation (Jsonic++ behavioural contract).
//!
//! A standalone JSON library: parse text/bytes into an ordered [`Value`] tree,
//! serialize back to canonical JSON, and validate against a JSON Schema subset.
//!
//! Contract semantics preserved from Jsonic++:
//! - object member order is insertion/document order (`indexmap::IndexMap`);
//! - duplicate object keys are rejected at parse time;
//! - numbers are IEEE-754 doubles; strings handle JSON escapes and Unicode;
//! - errors are returned as strings (never panics on any input).

mod value;
pub use value::{Value, ValueError};

/// Idiomatic aliases: `parse`/`validate` and `parse_json`/`validate_schema`
/// are the same functions (the latter are Nift's historical names, kept for
/// the nift re-export).
pub use crate::parse_json as parse;
pub use crate::validate_schema as validate;

use indexmap::IndexMap;

struct Parser<'a> {
    text: &'a [u8],
    pos: usize,
}

pub fn parse_json(text: &str) -> Result<Value, String> {
    let mut parser = Parser {
        text: text.as_bytes(),
        pos: 0,
    };
    parser.skip_ws();
    let value = parser.value()?;
    parser.skip_ws();
    if parser.pos != parser.text.len() {
        return Err("unexpected trailing content".to_string());
    }
    Ok(value)
}

/// Parse a byte slice as JSON text.
pub fn parse_bytes(text: &[u8]) -> Result<Value, String> {
    parse_json(std::str::from_utf8(text).map_err(|_| "invalid UTF-8".to_string())?)
}

/// Canonical compact JSON serialization (deterministic). Number formatting uses
/// Rust's shortest round-trip `f64` Display.
pub fn stringify(value: &Value) -> String {
    let mut out = String::new();
    write_stringify(&mut out, value);
    out
}

fn write_stringify(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => {
            out.push('"');
            write_escaped(out, s);
            out.push('"');
        }
        Value::Array(array) => {
            out.push('[');
            for (i, item) in array.iter().enumerate() {
                if i != 0 {
                    out.push(',');
                }
                write_stringify(out, item);
            }
            out.push(']');
        }
        Value::Object(object) => {
            out.push('{');
            for (i, (key, item)) in object.iter().enumerate() {
                if i != 0 {
                    out.push(',');
                }
                out.push('"');
                write_escaped(out, key);
                out.push_str("\":");
                write_stringify(out, item);
            }
            out.push('}');
        }
    }
}

/// JSON string escaping (canonical, for [`stringify`]).
fn write_escaped(out: &mut String, value: &str) {
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.text.len() && self.text[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.pos).copied()
    }

    fn expect(&mut self, expected: u8) -> Result<(), String> {
        match self.peek() {
            Some(c) if c == expected => {
                self.pos += 1;
                Ok(())
            }
            Some(c) => Err(format!(
                "expected '{}' at position {} (found '{:?}')",
                expected as char, self.pos, c as char
            )),
            None => Err(format!("expected '{}' at end of input", expected as char)),
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        self.skip_ws();
        match self.peek() {
            None => Err("unexpected end of input".to_string()),
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Value::string(self.string()?)),
            Some(b't') => self.literal("true", Value::boolean(true)),
            Some(b'f') => self.literal("false", Value::boolean(false)),
            Some(b'n') => self.literal("null", Value::null()),
            Some(c) if c == b'-' || c.is_ascii_digit() => self.number(),
            Some(c) => Err(format!("unexpected character '{}'", c as char)),
        }
    }

    fn literal(&mut self, word: &str, value: Value) -> Result<Value, String> {
        if self.text[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(format!("expected '{}'", word))
        }
    }

    fn number(&mut self) -> Result<Value, String> {
        // Reference Jsonic++ parse_number semantics: no leading zero, at least
        // one digit in each of the integer/fraction/exponent parts, and the
        // result must be finite.
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        if self.peek() == Some(b'0') {
            self.pos += 1;
            if matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                return Err("leading zero in JSON number".to_string());
            }
        } else {
            let mut any = false;
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    self.pos += 1;
                    any = true;
                } else {
                    break;
                }
            }
            if !any {
                return Err("invalid JSON number".to_string());
            }
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            let mut any = false;
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    self.pos += 1;
                    any = true;
                } else {
                    break;
                }
            }
            if !any {
                return Err("invalid fraction".to_string());
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            let mut any = false;
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    self.pos += 1;
                    any = true;
                } else {
                    break;
                }
            }
            if !any {
                return Err("invalid exponent".to_string());
            }
        }
        let slice = std::str::from_utf8(&self.text[start..self.pos])
            .map_err(|_| "invalid number".to_string())?;
        let value: f64 = slice
            .parse()
            .map_err(|_| format!("invalid number '{}'", slice))?;
        if !value.is_finite() {
            return Err("JSON number is outside the supported finite range".to_string());
        }
        Ok(Value::number(value))
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err("unterminated string".to_string()),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.peek() {
                        None => return Err("unterminated string escape".to_string()),
                        Some(c) => {
                            self.pos += 1;
                            out.push(match c {
                                b'"' => '"',
                                b'\\' => '\\',
                                b'/' => '/',
                                b'b' => '\u{8}',
                                b'f' => '\u{c}',
                                b'n' => '\n',
                                b'r' => '\r',
                                b't' => '\t',
                                b'u' => {
                                    // Reference Jsonic++ parse_unicode_codepoint:
                                    // a high surrogate must be followed by a low
                                    // surrogate, which combine into an astral
                                    // codepoint; a lone low surrogate is an error.
                                    let first = self.take_hex4()?;
                                    let codepoint = if (0xdc00..=0xdfff).contains(&first) {
                                        return Err("unexpected low surrogate in unicode escape".to_string());
                                    } else if (0xd800..=0xdbff).contains(&first) {
                                        if self.peek() != Some(b'\\')
                                            || self.text.get(self.pos + 1) != Some(&b'u')
                                        {
                                            return Err("high surrogate must be followed by a low surrogate".to_string());
                                        }
                                        self.pos += 2;
                                        let second = self.take_hex4()?;
                                        if !(0xdc00..=0xdfff).contains(&second) {
                                            return Err("invalid low surrogate in unicode escape".to_string());
                                        }
                                        0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00)
                                    } else {
                                        first
                                    };
                                    char::from_u32(codepoint)
                                        .ok_or_else(|| "invalid unicode escape".to_string())?
                                }
                                _ => return Err(format!("invalid escape '\\{}'", c as char)),
                            });
                        }
                    }
                }
                Some(_) => {
                    // Consume a full UTF-8 sequence.
                    let rest = std::str::from_utf8(&self.text[self.pos..])
                        .map_err(|_| "invalid UTF-8 in string".to_string())?;
                    let ch = rest
                        .chars()
                        .next()
                        .ok_or_else(|| "unterminated string".to_string())?;
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn take_hex4(&mut self) -> Result<u32, String> {
        if self.pos + 4 > self.text.len() {
            return Err("truncated unicode escape".to_string());
        }
        let hex = std::str::from_utf8(&self.text[self.pos..self.pos + 4])
            .map_err(|_| "invalid unicode escape".to_string())?;
        self.pos += 4;
        u32::from_str_radix(hex, 16).map_err(|_| "invalid unicode escape".to_string())
    }

    fn array(&mut self) -> Result<Value, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err("expected ',' or ']' in array".to_string()),
            }
        }
    }

    fn object(&mut self) -> Result<Value, String> {
        self.expect(b'{')?;
        let mut map: IndexMap<String, Value> = IndexMap::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Object(map));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            if map.contains_key(&key) {
                return Err(format!("duplicate object key '{key}'"));
            }
            self.skip_ws();
            self.expect(b':')?;
            let value = self.value()?;
            map.insert(key, value);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Object(map));
                }
                _ => return Err("expected ',' or '}' in object".to_string()),
            }
        }
    }
}

/// Entity escaping (reference `entity()`): map a short marker to its named
/// entity, or `None` when unknown.
pub fn entity(value: &str) -> Option<&'static str> {
    const ENTITIES: &[(&str, &str)] = &[
        ("`", "&grave;"),
        ("~", "&tilde;"),
        ("!", "&excl;"),
        ("@", "&commat;"),
        ("#", "&num;"),
        ("$", "&dollar;"),
        ("%", "&percnt;"),
        ("^", "&Hat;"),
        ("&", "&amp;"),
        ("*", "&ast;"),
        ("?", "&quest;"),
        ("<", "&lt;"),
        (">", "&gt;"),
        ("(", "&lpar;"),
        (")", "&rpar;"),
        ("[", "&lbrack;"),
        ("]", "&rbrack;"),
        ("{", "&lbrace;"),
        ("}", "&rbrace;"),
        ("-", "&minus;"),
        ("_", "&lowbar;"),
        ("=", "&equals;"),
        ("+", "&plus;"),
        ("|", "&vert;"),
        ("\\", "&bsol;"),
        ("/", "&sol;"),
        (";", "&semi;"),
        (":", "&colon;"),
        ("'", "&apos;"),
        ("\"", "&quot;"),
        (",", "&comma;"),
        (".", "&period;"),
        ("£", "&pound;"),
        ("¥", "&yen;"),
        ("€", "&euro;"),
        ("section", "&sect;"),
        ("+-", "&pm;"),
        ("-+", "&mp;"),
        ("!=", "&ne;"),
        ("<=", "&leq;"),
        (">=", "&geq;"),
        ("->", "&rarr;"),
        ("<-", "&larr;"),
        ("<->", "&harr;"),
        ("==>", "&rArr;"),
        ("<==", "&lArr;"),
        ("<==>", "&hArr;"),
        ("<=!=>", "&nhArr;"),
        ("...", "&hellip;"),
    ];
    for (key, encoded) in ENTITIES {
        if *key == value {
            return Some(encoded);
        }
    }
    None
}

/// JSON Schema validation (NR4), mirroring the frozen C++ JsonSchema subset:
/// the supported keyword inventory is enforced (unknown keywords are rejected)
/// and both the schema shape and the instance are validated.
///
/// Supported keywords: `$schema`, `$comment`, `title`, `description`, `default`,
/// `examples` (accepted, ignored), `$defs`, `$ref`, `type`, `enum`, `const`,
/// `properties`, `required`, `additionalProperties`, `minProperties`,
/// `maxProperties`, `items`, `minItems`, `maxItems`, `uniqueItems`, `contains`,
/// `minContains`, `maxContains`, `minLength`, `maxLength`, `pattern`,
/// `minimum`, `maximum`, `exclusiveMinimum`, `exclusiveMaximum`, `multipleOf`,
/// `allOf`, `anyOf`, `oneOf`, `not`.
pub fn validate_schema(value: &Value, schema: &Value) -> Result<(), String> {
    let validator = Validator {
        root_schema: schema.clone(),
    };
    validator.validate_schema_shape(schema, "#", 0)?;
    validator.apply(value, schema, "$", 0)?;
    Ok(())
}

struct Validator {
    root_schema: Value,
}

impl Validator {
    const MAX_DEPTH: usize = 256;

    fn type_name(value: &Value) -> &'static str {
        match value {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }

    fn json_equal(a: &Value, b: &Value) -> bool {
        a == b
    }

    fn non_negative_integer(value: &Value) -> Option<usize> {
        let n = value.as_number()?;
        if n < 0.0 || n.fract() != 0.0 || n > usize::MAX as f64 {
            return None;
        }
        Some(n as usize)
    }

    fn validate_schema_shape(
        &self,
        schema: &Value,
        schema_path: &str,
        depth: usize,
    ) -> Result<(), String> {
        if depth > Self::MAX_DEPTH {
            return Err(format!(
                "JSON Schema nesting is too deep near {schema_path}"
            ));
        }
        if schema.is_bool() {
            return Ok(());
        }
        let Some(schema_object) = schema.as_object() else {
            return Err(format!(
                "JSON Schema at {schema_path} must be an object or boolean"
            ));
        };

        const SUPPORTED: &[&str] = &[
            "$schema",
            "$comment",
            "title",
            "description",
            "default",
            "examples",
            "$defs",
            "$ref",
            "type",
            "enum",
            "const",
            "properties",
            "required",
            "additionalProperties",
            "minProperties",
            "maxProperties",
            "items",
            "minItems",
            "maxItems",
            "uniqueItems",
            "contains",
            "minContains",
            "maxContains",
            "minLength",
            "maxLength",
            "pattern",
            "minimum",
            "maximum",
            "exclusiveMinimum",
            "exclusiveMaximum",
            "multipleOf",
            "allOf",
            "anyOf",
            "oneOf",
            "not",
        ];
        for key in schema_object.keys() {
            if !SUPPORTED.contains(&key.as_str()) {
                return Err(format!(
                    "unsupported JSON Schema keyword '{key}' at {schema_path}"
                ));
            }
        }

        if let Some(t) = schema_object.get("type") {
            match t {
                Value::String(name) => {
                    if !valid_type_name(name) {
                        return Err(format!(
                            "unknown JSON Schema type '{name}' at {schema_path}"
                        ));
                    }
                }
                Value::Array(list) => {
                    if list.is_empty() {
                        return Err(format!("type array cannot be empty at {schema_path}"));
                    }
                    let mut seen = std::collections::HashSet::new();
                    for item in list {
                        let Some(name) = item.as_str() else {
                            return Err(format!(
                                "type array must contain supported type names at {schema_path}"
                            ));
                        };
                        if !valid_type_name(name) {
                            return Err(format!(
                                "type array must contain supported type names at {schema_path}"
                            ));
                        }
                        if !seen.insert(name) {
                            return Err(format!(
                                "type array contains duplicate type '{name}' at {schema_path}"
                            ));
                        }
                    }
                }
                _ => {
                    return Err(format!(
                        "type must be a string or array of strings at {schema_path}"
                    ))
                }
            }
        }

        if let Some(required) = schema_object.get("required") {
            let Some(list) = required.as_array() else {
                return Err(format!("required must be an array at {schema_path}"));
            };
            let mut seen = std::collections::HashSet::new();
            for item in list {
                let Some(name) = item.as_str() else {
                    return Err(format!(
                        "required must contain only strings at {schema_path}"
                    ));
                };
                if !seen.insert(name) {
                    return Err(format!(
                        "required contains duplicate member '{name}' at {schema_path}"
                    ));
                }
            }
        }

        for key in [
            "minProperties",
            "maxProperties",
            "minItems",
            "maxItems",
            "minContains",
            "maxContains",
            "minLength",
            "maxLength",
        ] {
            if let Some(value) = schema_object.get(key) {
                if Self::non_negative_integer(value).is_none() {
                    return Err(format!(
                        "{key} must be a non-negative integer at {schema_path}"
                    ));
                }
            }
        }
        for key in [
            "minimum",
            "maximum",
            "exclusiveMinimum",
            "exclusiveMaximum",
            "multipleOf",
        ] {
            if let Some(value) = schema_object.get(key) {
                if !value.is_number() {
                    return Err(format!("{key} must be a number at {schema_path}"));
                }
            }
        }
        if let Some(multiple) = schema_object.get("multipleOf") {
            if multiple.as_number().unwrap_or(0.0) <= 0.0 {
                return Err(format!(
                    "multipleOf must be greater than zero at {schema_path}"
                ));
            }
        }
        if let Some(pattern) = schema_object.get("pattern") {
            let Some(pattern_text) = pattern.as_str() else {
                return Err(format!("pattern must be a string at {schema_path}"));
            };
            if regex::Regex::new(pattern_text).is_err() {
                return Err(format!(
                    "pattern is not a valid regular expression at {schema_path}"
                ));
            }
        }
        if let Some(value) = schema_object.get("uniqueItems") {
            if !value.is_bool() {
                return Err(format!("uniqueItems must be boolean at {schema_path}"));
            }
        }
        if let Some(additional) = schema_object.get("additionalProperties") {
            if !additional.is_bool() && !additional.is_object() {
                return Err(format!(
                    "additionalProperties must be a boolean or schema object at {schema_path}"
                ));
            }
        }
        if schema_object.contains_key("properties") && !schema_object["properties"].is_object() {
            return Err(format!("properties must be an object at {schema_path}"));
        }
        if schema_object.contains_key("$defs") && !schema_object["$defs"].is_object() {
            return Err(format!("$defs must be an object at {schema_path}"));
        }
        if schema_object.contains_key("$ref") && !schema_object["$ref"].is_string() {
            return Err(format!("$ref must be a string at {schema_path}"));
        }
        if let Some(enum_list) = schema_object.get("enum") {
            if !enum_list.is_array() || enum_list.as_array().map(|a| a.is_empty()).unwrap_or(true) {
                return Err(format!("enum must be a non-empty array at {schema_path}"));
            }
        }
        for key in ["allOf", "anyOf", "oneOf"] {
            if let Some(list) = schema_object.get(key) {
                let Some(array) = list.as_array() else {
                    return Err(format!(
                        "{key} must be a non-empty array of schemas at {schema_path}"
                    ));
                };
                if array.is_empty() {
                    return Err(format!(
                        "{key} must be a non-empty array of schemas at {schema_path}"
                    ));
                }
                for (index, child) in array.iter().enumerate() {
                    self.validate_schema_shape(
                        child,
                        &format!("{schema_path}/{key}/{index}"),
                        depth + 1,
                    )?;
                }
            }
        }
        for key in ["items", "contains", "not"] {
            if let Some(child) = schema_object.get(key) {
                self.validate_schema_shape(child, &format!("{schema_path}/{key}"), depth + 1)?;
            }
        }
        if let Some(additional) = schema_object.get("additionalProperties") {
            if additional.is_object() {
                self.validate_schema_shape(
                    additional,
                    &format!("{schema_path}/additionalProperties"),
                    depth + 1,
                )?;
            }
        }
        if let Some(properties) = schema_object.get("properties") {
            if let Some(object) = properties.as_object() {
                for (key, child) in object {
                    self.validate_schema_shape(
                        child,
                        &format!("{schema_path}/properties/{key}"),
                        depth + 1,
                    )?;
                }
            }
        }
        if let Some(defs) = schema_object.get("$defs") {
            if let Some(object) = defs.as_object() {
                for (key, child) in object {
                    self.validate_schema_shape(
                        child,
                        &format!("{schema_path}/$defs/{key}"),
                        depth + 1,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn apply(
        &self,
        instance: &Value,
        schema: &Value,
        instance_path: &str,
        depth: usize,
    ) -> Result<(), String> {
        if depth > Self::MAX_DEPTH {
            return Err(format!(
                "at {instance_path}: JSON Schema validation exceeded maximum nesting depth"
            ));
        }
        if schema.is_bool() {
            if schema == &Value::boolean(true) {
                return Ok(());
            }
            return Err(format!(
                "at {instance_path}: value is rejected by a false schema"
            ));
        }
        let Some(schema_object) = schema.as_object() else {
            return Err(format!(
                "at {instance_path}: internal schema is not an object or boolean"
            ));
        };

        if let Some(ref_value) = schema_object.get("$ref") {
            let ref_text = ref_value.as_str().unwrap_or("");
            let target = resolve_local_ref(&self.root_schema, ref_text)?;
            self.apply(instance, &target, instance_path, depth + 1)?;
        }

        if let Some(t) = schema_object.get("type") {
            let match_type = |expected: &str| -> bool {
                match expected {
                    "null" => instance.is_null(),
                    "boolean" => instance.is_bool(),
                    "number" => instance.is_number(),
                    "integer" => {
                        instance.is_number() && instance.as_number().unwrap_or(0.0).fract() == 0.0
                    }
                    "string" => instance.is_string(),
                    "array" => instance.is_array(),
                    "object" => instance.is_object(),
                    _ => false,
                }
            };
            let matched = match t {
                Value::String(name) => match_type(name),
                Value::Array(list) => list
                    .iter()
                    .any(|item| match_type(item.as_str().unwrap_or(""))),
                _ => false,
            };
            if !matched {
                let expected = match t {
                    Value::String(name) => name.clone(),
                    Value::Array(list) => list
                        .iter()
                        .map(|item| item.as_str().unwrap_or("").to_string())
                        .collect::<Vec<_>>()
                        .join(" or "),
                    _ => "?".to_string(),
                };
                return Err(format!(
                    "at {instance_path}: expected {expected}, received {}",
                    Self::type_name(instance)
                ));
            }
        }

        if let Some(const_value) = schema_object.get("const") {
            if const_value != instance {
                return Err(format!("at {instance_path}: value does not match const"));
            }
        }
        if let Some(enum_list) = schema_object.get("enum") {
            let found = enum_list
                .as_array()
                .map(|list| list.iter().any(|candidate| candidate == instance))
                .unwrap_or(false);
            if !found {
                return Err(format!(
                    "at {instance_path}: value is not one of the allowed enum values"
                ));
            }
        }

        if instance.is_object() {
            let object = instance.as_object().unwrap();
            if let Some(min) = schema_object.get("minProperties") {
                if object.len() < Self::non_negative_integer(min).unwrap_or(0) {
                    return Err(format!(
                        "at {instance_path}: object has fewer than {} properties",
                        Self::non_negative_integer(min).unwrap_or(0)
                    ));
                }
            }
            if let Some(max) = schema_object.get("maxProperties") {
                if object.len() > Self::non_negative_integer(max).unwrap_or(0) {
                    return Err(format!(
                        "at {instance_path}: object has more than {} properties",
                        Self::non_negative_integer(max).unwrap_or(0)
                    ));
                }
            }
            if let Some(required) = schema_object.get("required") {
                if let Some(list) = required.as_array() {
                    for requirement in list {
                        let name = requirement.as_str().unwrap_or("");
                        if !object.contains_key(name) {
                            return Err(format!(
                                "at {instance_path}: required property '{name}' is missing"
                            ));
                        }
                    }
                }
            }
            if let Some(properties) = schema_object.get("properties") {
                if let Some(properties_object) = properties.as_object() {
                    for (key, child_schema) in properties_object {
                        if let Some(property_value) = object.get(key) {
                            self.apply(
                                property_value,
                                child_schema,
                                &format!("{instance_path}.{key}"),
                                depth + 1,
                            )?;
                        }
                    }
                }
            }
            if let Some(additional) = schema_object.get("additionalProperties") {
                let declared = schema_object
                    .get("properties")
                    .and_then(|p| p.as_object())
                    .map(|p| p.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default();
                for (key, value) in object {
                    if declared.contains(key) {
                        continue;
                    }
                    if additional.is_bool() {
                        if additional != &Value::boolean(true) {
                            return Err(format!(
                                "at {instance_path}.{key}: additional property is not allowed"
                            ));
                        }
                    } else {
                        self.apply(
                            value,
                            additional,
                            &format!("{instance_path}.{key}"),
                            depth + 1,
                        )?;
                    }
                }
            }
        }

        if instance.is_array() {
            let array = instance.as_array().unwrap();
            if let Some(min) = schema_object.get("minItems") {
                if array.len() < Self::non_negative_integer(min).unwrap_or(0) {
                    return Err(format!(
                        "at {instance_path}: array has fewer than {} items",
                        Self::non_negative_integer(min).unwrap_or(0)
                    ));
                }
            }
            if let Some(max) = schema_object.get("maxItems") {
                if array.len() > Self::non_negative_integer(max).unwrap_or(0) {
                    return Err(format!(
                        "at {instance_path}: array has more than {} items",
                        Self::non_negative_integer(max).unwrap_or(0)
                    ));
                }
            }
            if schema_object.get("uniqueItems") == Some(&Value::boolean(true)) {
                for i in 0..array.len() {
                    for j in (i + 1)..array.len() {
                        if Self::json_equal(&array[i], &array[j]) {
                            return Err(format!(
                                "at {instance_path}[{j}]: array items must be unique"
                            ));
                        }
                    }
                }
            }
            if let Some(items) = schema_object.get("items") {
                for (index, item) in array.iter().enumerate() {
                    self.apply(item, items, &format!("{instance_path}[{index}]"), depth + 1)?;
                }
            }
            if let Some(contains) = schema_object.get("contains") {
                let mut matches = 0usize;
                for item in array {
                    if self.apply(item, contains, instance_path, depth + 1).is_ok() {
                        matches += 1;
                    }
                }
                let min = schema_object
                    .get("minContains")
                    .and_then(Self::non_negative_integer)
                    .unwrap_or(1);
                let max = schema_object
                    .get("maxContains")
                    .and_then(Self::non_negative_integer)
                    .unwrap_or(usize::MAX);
                if matches < min || matches > max {
                    let max_text = if max == usize::MAX {
                        "unbounded".to_string()
                    } else {
                        max.to_string()
                    };
                    return Err(format!(
                        "at {instance_path}: array contains {matches} matching items; expected between {min} and {max_text}"
                    ));
                }
            }
        }

        if let Some(string) = instance.as_str() {
            let length = string.chars().count();
            if let Some(min) = schema_object.get("minLength") {
                if length < Self::non_negative_integer(min).unwrap_or(0) {
                    return Err(format!(
                        "at {instance_path}: string is shorter than minLength"
                    ));
                }
            }
            if let Some(max) = schema_object.get("maxLength") {
                if length > Self::non_negative_integer(max).unwrap_or(0) {
                    return Err(format!(
                        "at {instance_path}: string is longer than maxLength"
                    ));
                }
            }
            if let Some(pattern) = schema_object.get("pattern") {
                let pattern_text = pattern.as_str().unwrap_or("");
                let compiled = regex::Regex::new(pattern_text)
                    .map_err(|_| "invalid regex pattern in schema".to_string())?;
                if !compiled.is_match(string) {
                    return Err(format!(
                        "at {instance_path}: string does not match required pattern"
                    ));
                }
            }
        }

        if let Some(number) = instance.as_number() {
            if let Some(bound) = schema_object.get("minimum") {
                if number < bound.as_number().unwrap_or(0.0) {
                    return Err(format!("at {instance_path}: number is less than minimum"));
                }
            }
            if let Some(bound) = schema_object.get("maximum") {
                if number > bound.as_number().unwrap_or(0.0) {
                    return Err(format!(
                        "at {instance_path}: number is greater than maximum"
                    ));
                }
            }
            if let Some(bound) = schema_object.get("exclusiveMinimum") {
                if number <= bound.as_number().unwrap_or(0.0) {
                    return Err(format!(
                        "at {instance_path}: number is not greater than exclusiveMinimum"
                    ));
                }
            }
            if let Some(bound) = schema_object.get("exclusiveMaximum") {
                if number >= bound.as_number().unwrap_or(0.0) {
                    return Err(format!(
                        "at {instance_path}: number is not less than exclusiveMaximum"
                    ));
                }
            }
            if let Some(multiple) = schema_object.get("multipleOf") {
                let divisor = multiple.as_number().unwrap_or(0.0);
                if divisor != 0.0 {
                    let quotient = number / divisor;
                    if (quotient - quotient.round()).abs() > 1e-10 * quotient.abs().max(1.0) {
                        return Err(format!(
                            "at {instance_path}: number is not a multipleOf {}",
                            multiple.as_number().unwrap_or(0.0)
                        ));
                    }
                }
            }
        }

        let count_matches = |self_ref: &Self, list: &Value| -> usize {
            list.as_array()
                .map(|array| {
                    array
                        .iter()
                        .filter(|child| {
                            self_ref
                                .apply(instance, child, instance_path, depth + 1)
                                .is_ok()
                        })
                        .count()
                })
                .unwrap_or(0)
        };
        if let Some(all_of) = schema_object.get("allOf") {
            if let Some(list) = all_of.as_array() {
                for child in list {
                    self.apply(instance, child, instance_path, depth + 1)?;
                }
            }
        }
        if let Some(any_of) = schema_object.get("anyOf") {
            if count_matches(self, any_of) == 0 {
                return Err(format!(
                    "at {instance_path}: value does not match any schema in anyOf"
                ));
            }
        }
        if let Some(one_of) = schema_object.get("oneOf") {
            if count_matches(self, one_of) != 1 {
                return Err(format!(
                    "at {instance_path}: value must match exactly one schema in oneOf"
                ));
            }
        }
        if let Some(not) = schema_object.get("not") {
            if self.apply(instance, not, instance_path, depth + 1).is_ok() {
                return Err(format!(
                    "at {instance_path}: value matches schema forbidden by not"
                ));
            }
        }

        Ok(())
    }
}

fn valid_type_name(name: &str) -> bool {
    matches!(
        name,
        "null" | "boolean" | "number" | "integer" | "string" | "array" | "object"
    )
}

fn pointer_unescape(token: &str) -> Option<String> {
    let mut out = String::new();
    let bytes = token.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'~' {
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }
        if i + 1 >= bytes.len() {
            return None;
        }
        let next = bytes[i + 1];
        if next == b'0' {
            out.push('~');
        } else if next == b'1' {
            out.push('/');
        } else {
            return None;
        }
        i += 2;
    }
    Some(out)
}

fn resolve_local_ref(root: &Value, reference: &str) -> Result<Value, String> {
    if reference == "#" {
        return Ok(root.clone());
    }
    if !reference.starts_with("#/") {
        return Err(format!(
            "only local JSON Schema $ref values beginning with '#/' are supported (got '{reference}')"
        ));
    }
    let mut current = root.clone();
    let mut start = 2;
    loop {
        let slash = reference[start..].find('/').map(|o| start + o);
        let end = slash.unwrap_or(reference.len());
        let token = &reference[start..end];
        let Some(token) = pointer_unescape(token) else {
            return Err(format!("invalid JSON Pointer escape in $ref '{reference}'"));
        };
        match &current {
            Value::Object(object) => {
                let Some(child) = object.get(&token) else {
                    return Err(format!(
                        "$ref '{reference}' does not resolve (missing member '{token}')"
                    ));
                };
                current = child.clone();
            }
            Value::Array(array) => {
                if token.is_empty() || !token.chars().all(|c| c.is_ascii_digit()) {
                    return Err(format!(
                        "$ref '{reference}' uses a non-numeric array index '{token}'"
                    ));
                }
                let index = token
                    .parse::<usize>()
                    .map_err(|_| format!("$ref array index is out of range in '{reference}'"))?;
                let Some(child) = array.get(index) else {
                    return Err(format!("$ref array index is out of range in '{reference}'"));
                };
                current = child.clone();
            }
            _ => {
                return Err(format!(
                    "$ref '{reference}' traverses through a non-container value"
                ));
            }
        }
        match slash {
            None => break,
            Some(slash) => start = slash + 1,
        }
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scalars() {
        assert_eq!(parse_json("null").unwrap(), Value::null());
        assert_eq!(parse_json("true").unwrap(), Value::boolean(true));
        assert_eq!(parse_json("42").unwrap(), Value::number(42.0));
        assert_eq!(parse_json("-3.5").unwrap(), Value::number(-3.5));
        assert_eq!(parse_json("1e3").unwrap(), Value::number(1000.0));
        assert_eq!(parse_json("\"hello\"").unwrap(), Value::string("hello"));
        assert_eq!(
            parse_json("\"a\\n\\\"b\"").unwrap(),
            Value::string("a\n\"b")
        );
    }

    #[test]
    fn parses_arrays_and_objects() {
        let value = parse_json(r#"{"b":2,"a":[1,true,null]}"#).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), 2);
        let keys: Vec<&str> = object.keys().map(|k| k.as_str()).collect();
        assert_eq!(keys, ["b", "a"]);
        assert_eq!(object["a"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn rejects_duplicate_keys_and_malformed() {
        assert!(parse_json(r#"{"a":1,"a":2}"#).is_err());
        assert!(parse_json("{").is_err());
        assert!(parse_json("[1,2,").is_err());
        assert!(parse_json("1 2").is_err());
        assert!(parse_json("").is_err());
    }
}
