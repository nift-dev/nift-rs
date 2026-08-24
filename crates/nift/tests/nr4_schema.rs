//! NR4 JSON-Schema completeness tests: $defs/$ref, contains/minContains/
//! maxContains, pattern, unknown-keyword rejection and schema-shape
//! validation. Expected behaviours mirror the frozen C++ JsonSchema.

use nift::json::{parse_json, validate_schema};
use nift::Value;

fn schema(text: &str) -> Value {
    parse_json(text).expect("valid schema JSON")
}

fn instance(text: &str) -> Value {
    parse_json(text).expect("valid instance JSON")
}

fn check(instance_text: &str, schema_text: &str) -> Result<(), String> {
    validate_schema(&instance(instance_text), &schema(schema_text))
}

#[test]
fn ref_and_defs() {
    let s = r##"{"$defs":{"positive":{"type":"number","minimum":0}},"type":"object","properties":{"n":{"$ref":"#/$defs/positive"}}}"##;
    assert!(check(r#"{"n":5}"#, s).is_ok());
    assert!(check(r#"{"n":-1}"#, s).is_err());
    // Degenerate self-reference ($ref "#" to the whole schema) exceeds the
    // depth limit rather than looping forever.
    assert!(check("5", r##"{"$ref":"#"}"##).is_err());
    // Non-local ref is rejected.
    assert!(check("5", r##"{"$ref":"http://x/y.json"}"##).is_err());
    // Missing definition.
    assert!(check("5", r##"{"$ref":"#/$defs/nope"}"##).is_err());
    // Invalid pointer escape.
    assert!(check("5", r##"{"$ref":"#/$defs~2x"}"##).is_err());
}

#[test]
fn contains_and_counts() {
    let s = r##"{"type":"array","contains":{"type":"number","minimum":10},"minContains":1,"maxContains":3}"##;
    assert!(check("[1,20,3]", s).is_ok());
    assert!(check("[1,2,3]", s).is_err());
    assert!(check("[1,20,30,40,50]", s).is_err()); // 4 matches > 3
                                                   // minContains only.
    let s = r##"{"type":"array","contains":{"type":"string"},"minContains":2}"##;
    assert!(check(r#"["a","b",1]"#, s).is_ok());
    assert!(check(r#"["a",1,2]"#, s).is_err());
    // Empty array fails when minContains defaults to 1.
    let s = r##"{"type":"array","contains":{"type":"string"}}"##;
    assert!(check("[]", s).is_err());
}

#[test]
fn pattern_keyword() {
    let s = r##"{"type":"string","pattern":"^[a-z]+$"}"##;
    assert!(check("\"hello\"", s).is_ok());
    assert!(check("\"Hello1\"", s).is_err());
    assert!(check("5", s).is_err()); // type mismatch (string required)
                                     // Without a type constraint, pattern is only applied to strings.
    let s = r##"{"pattern":"^[a-z]+$"}"##;
    assert!(check("5", s).is_ok());
    assert!(check("\"NO\"", s).is_err());
    // Malformed regex is a schema-shape error.
    assert!(check("5", r##"{"pattern":"["}"##).is_err());
    // Non-string pattern is a schema-shape error.
    assert!(check("5", r##"{"pattern":5}"##).is_err());
}

#[test]
fn unknown_keyword_rejection() {
    let s = r##"{"type":"string","definitelyNotANiftSchemaKeyword":true}"##;
    let error = check("\"x\"", s).expect_err("unknown keyword must be rejected");
    assert!(error.contains("unsupported JSON Schema keyword 'definitelyNotANiftSchemaKeyword'"));
}

#[test]
fn schema_shape_validation() {
    assert!(check("5", r##"{"type":"strin"}"##).is_err());
    assert!(check("5", r##"{"type":[]}"##).is_err());
    assert!(check("{}", r##"{"required":"x"}"##).is_err());
    assert!(check("[]", r##"{"minItems":-1}"##).is_err());
    assert!(check("4", r##"{"multipleOf":0}"##).is_err());
    assert!(check("5", "5").is_err());
}

#[test]
fn unique_items_boolean_boundaries() {
    // uniqueItems: true -> enforce.
    let s = r##"{"type":"array","uniqueItems":true}"##;
    assert!(check("[1,2,3]", s).is_ok());
    assert!(check("[1,1,2]", s).is_err());
    // uniqueItems: false -> valid, do not enforce (duplicates pass).
    let s = r##"{"type":"array","uniqueItems":false}"##;
    assert!(check("[1,1,2]", s).is_ok());
    // non-boolean uniqueItems is a schema-shape error.
    assert!(check("[]", r##"{"uniqueItems":1}"##).is_err());
    assert!(check("[]", r##"{"uniqueItems":"yes"}"##).is_err());
}

#[test]
fn shape_audit_multivalued_rules() {
    // additionalProperties accepts boolean (true AND false) or a schema object.
    assert!(check(
        r#"{"a":1,"b":2}"#,
        r##"{"type":"object","additionalProperties":true}"##
    )
    .is_ok());
    assert!(check(
        r#"{"a":1}"#,
        r##"{"type":"object","additionalProperties":false}"##
    )
    .is_err());
    assert!(check(
        r#"{"a":1}"#,
        r##"{"type":"object","additionalProperties":{"type":"number"}}"##
    )
    .is_ok());
    assert!(check("[]", r##"{"additionalProperties":5}"##).is_err());
    // Boolean schemas: both true and false are valid shapes.
    assert!(check("5", "true").is_ok());
    assert!(check("5", "false").is_err());
    // type accepts a string or an array of strings.
    assert!(check("5", r##"{"type":"number"}"##).is_ok());
    assert!(check("5", r##"{"type":["number","string"]}"##).is_ok());
    assert!(check("5", r##"{"type":["boolean"]}"##).is_err());
}
