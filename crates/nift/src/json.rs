//! A JSON text parser into [`Value`] (NR4).
//!
//! Mirrors the reference JSON semantics that matter for `@json`/`set_json`:
//! object member order is insertion order, and duplicate object keys are
//! rejected. Errors are returned as strings (never panics).

use crate::value::Value;
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
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }
        let slice = std::str::from_utf8(&self.text[start..self.pos])
            .map_err(|_| "invalid number".to_string())?;
        slice
            .parse::<f64>()
            .map(Value::number)
            .map_err(|_| format!("invalid number '{}'", slice))
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
                                    let hex = self.take_hex4()?;
                                    char::from_u32(hex).ok_or("invalid unicode escape")?
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

/// A focused JSON Schema validator (NR4) covering the reference surface used
/// by `@json(..., schema)`: type, required, properties, items, and
/// additionalProperties. Returns an error string with a JSON pointer-style
/// path on failure.
pub fn validate_schema(value: &Value, schema: &Value) -> Result<(), String> {
    validate_at(value, schema, "$")
}

fn validate_at(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    let Some(schema_object) = schema.as_object() else {
        return Ok(());
    };

    if let Some(ty) = schema_object.get("type") {
        let Some(expected) = ty.as_str() else {
            return Err(format!("{path}: schema 'type' must be a string"));
        };
        let matches = match expected {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => value.is_number() && value.as_number().unwrap_or(0.0).fract() == 0.0,
            "boolean" => value.is_bool(),
            "null" => value.is_null(),
            other => return Err(format!("{path}: unsupported schema type '{other}'")),
        };
        if !matches {
            return Err(format!(
                "{path}: expected {expected}, got {}",
                json_type_name(value)
            ));
        }
    }

    if let Some(required) = schema_object.get("required") {
        let Some(required_list) = required.as_array() else {
            return Err(format!("{path}: schema 'required' must be an array"));
        };
        if let Some(object) = value.as_object() {
            for requirement in required_list {
                let Some(key) = requirement.as_str() else {
                    return Err(format!("{path}: 'required' entries must be strings"));
                };
                if !object.contains_key(key) {
                    return Err(format!("{path}: missing required property '{key}'"));
                }
            }
        }
    }

    if let Some(properties) = schema_object.get("properties") {
        let Some(properties_object) = properties.as_object() else {
            return Err(format!("{path}: schema 'properties' must be an object"));
        };
        if let Some(object) = value.as_object() {
            for (key, property_schema) in properties_object {
                if let Some(property_value) = object.get(key) {
                    validate_at(property_value, property_schema, &format!("{path}.{key}"))?;
                }
            }
        }
    }

    if let Some(additional) = schema_object.get("additionalProperties") {
        if let Some(object) = value.as_object() {
            if additional == &Value::boolean(false) {
                if let Some(properties) = schema_object.get("properties") {
                    if let Some(properties_object) = properties.as_object() {
                        for key in object.keys() {
                            if !properties_object.contains_key(key) {
                                return Err(format!("{path}: unexpected property '{key}'"));
                            }
                        }
                    }
                }
            } else if additional.is_object() {
                let additional_value = additional.clone();
                if let Some(properties) = schema_object.get("properties") {
                    if let Some(properties_object) = properties.as_object() {
                        for (key, property_value) in object {
                            if !properties_object.contains_key(key) {
                                validate_at(
                                    property_value,
                                    &additional_value,
                                    &format!("{path}.{key}"),
                                )?;
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some(count) = schema_object.get("minProperties") {
        if let Some(object) = value.as_object() {
            let limit = count.as_number().unwrap_or(0.0);
            if object.len() < limit as usize {
                return Err(format!("{path}: object has fewer than {limit} properties"));
            }
        }
    }
    if let Some(count) = schema_object.get("maxProperties") {
        if let Some(object) = value.as_object() {
            let limit = count.as_number().unwrap_or(0.0);
            if object.len() > limit as usize {
                return Err(format!("{path}: object has more than {limit} properties"));
            }
        }
    }

    if let Some(items) = schema_object.get("items") {
        if let Some(array) = value.as_array() {
            if let Some(items_object) = items.as_object() {
                let items_value = Value::Object(items_object.clone());
                for (index, item) in array.iter().enumerate() {
                    validate_at(item, &items_value, &format!("{path}[{index}]"))?;
                }
            }
        }
    }
    if let Some(count) = schema_object.get("minItems") {
        if let Some(array) = value.as_array() {
            let limit = count.as_number().unwrap_or(0.0);
            if array.len() < limit as usize {
                return Err(format!("{path}: array has fewer than {limit} items"));
            }
        }
    }
    if let Some(count) = schema_object.get("maxItems") {
        if let Some(array) = value.as_array() {
            let limit = count.as_number().unwrap_or(0.0);
            if array.len() > limit as usize {
                return Err(format!("{path}: array has more than {limit} items"));
            }
        }
    }
    if schema_object.get("uniqueItems") == Some(&Value::boolean(true)) {
        if let Some(array) = value.as_array() {
            let mut seen = std::collections::HashSet::new();
            for item in array {
                if !seen.insert(crate::expr::dump_compact(item)) {
                    return Err(format!("{path}: array items must be unique"));
                }
            }
        }
    }

    if let Some(count) = schema_object.get("minLength") {
        if let Some(string) = value.as_str() {
            let limit = count.as_number().unwrap_or(0.0);
            if string.chars().count() < limit as usize {
                return Err(format!("{path}: string is shorter than {limit} characters"));
            }
        }
    }
    if let Some(count) = schema_object.get("maxLength") {
        if let Some(string) = value.as_str() {
            let limit = count.as_number().unwrap_or(0.0);
            if string.chars().count() > limit as usize {
                return Err(format!("{path}: string is longer than {limit} characters"));
            }
        }
    }

    for key in ["minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"] {
        if let Some(bound) = schema_object.get(key) {
            if let Some(number) = value.as_number() {
                let bound = bound.as_number().unwrap_or(f64::NAN);
                let violation = match key {
                    "minimum" => number < bound,
                    "maximum" => number > bound,
                    "exclusiveMinimum" => number <= bound,
                    "exclusiveMaximum" => number >= bound,
                    _ => false,
                };
                if violation {
                    return Err(format!("{path}: {number} is not within {key} {bound}"));
                }
            }
        }
    }
    if let Some(multiple) = schema_object.get("multipleOf") {
        if let Some(number) = value.as_number() {
            let multiple = multiple.as_number().unwrap_or(0.0);
            if multiple != 0.0 {
                let remainder = number / multiple;
                if remainder.fract().abs() > 1e-9 {
                    return Err(format!("{path}: {number} is not a multiple of {multiple}"));
                }
            }
        }
    }

    if let Some(enum_list) = schema_object.get("enum") {
        let Some(enum_array) = enum_list.as_array() else {
            return Err(format!("{path}: schema 'enum' must be an array"));
        };
        if !enum_array.iter().any(|candidate| candidate == value) {
            return Err(format!(
                "{path}: value is not one of the allowed enum values"
            ));
        }
    }
    if let Some(const_value) = schema_object.get("const") {
        if const_value != value {
            return Err(format!("{path}: value does not equal the const value"));
        }
    }

    for key in ["allOf", "anyOf", "oneOf"] {
        if let Some(combinators) = schema_object.get(key) {
            let Some(combinator_array) = combinators.as_array() else {
                return Err(format!("{path}: schema '{key}' must be an array"));
            };
            let mut matched = 0usize;
            for sub_schema in combinator_array {
                if validate_at(value, sub_schema, path).is_ok() {
                    matched += 1;
                }
            }
            let ok = match key {
                "allOf" => matched == combinator_array.len(),
                "anyOf" => matched >= 1,
                "oneOf" => matched == 1,
                _ => false,
            };
            if !ok {
                return Err(format!("{path}: does not satisfy '{key}'"));
            }
        }
    }
    if let Some(not_schema) = schema_object.get("not") {
        if validate_at(value, not_schema, path).is_ok() {
            return Err(format!("{path}: value matches the 'not' schema"));
        }
    }

    Ok(())
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
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
