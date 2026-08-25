//! jsonic standalone test suite: contract conformance, property round-trips,
//! adversarial robustness, and a representative throughput benchmark.
//!
//! Many cases are ported from the Jsonic++ corpora (`json_smoke`,
//! `json_adversarial`): the goal is contract equivalence with Jsonic++, not
//! test-source equivalence, so C++ lifetime/memory cases are translated to the
//! underlying guarantee (robustness on arbitrary input, no panics).

use jsonic::{parse, parse_bytes, stringify, validate, Value};

// --- valid parse corpus -----------------------------------------------------

#[test]
fn parses_primitives() {
    assert_eq!(parse("null").unwrap(), Value::null());
    assert_eq!(parse("true").unwrap(), Value::boolean(true));
    assert_eq!(parse("false").unwrap(), Value::boolean(false));
    assert_eq!(parse("0").unwrap(), Value::number(0.0));
    assert_eq!(parse("-1.5").unwrap(), Value::number(-1.5));
    assert_eq!(parse("1e3").unwrap(), Value::number(1000.0));
    assert_eq!(parse("\"hi\"").unwrap(), Value::string("hi"));
}

#[test]
fn parses_empty_structures() {
    assert_eq!(parse("{}").unwrap(), Value::object());
    assert_eq!(parse("[]").unwrap(), Value::array());
    assert_eq!(parse("   {   }   ").unwrap(), Value::object());
}

#[test]
#[allow(clippy::approx_constant)]
fn parses_nested_document() {
    let doc =
        r#"{"name":"nift","tags":["json","rust"],"meta":{"ok":true,"n":42,"pi":3.14,"none":null}}"#;
    let value = parse(doc).unwrap();
    let object = value.as_object().unwrap();
    assert_eq!(object.get("name").unwrap().as_str(), Some("nift"));
    assert_eq!(object.get("tags").unwrap().as_array().unwrap().len(), 2);
    assert_eq!(
        object
            .get("meta")
            .unwrap()
            .as_object()
            .unwrap()
            .get("pi")
            .unwrap()
            .as_number(),
        Some(3.14)
    );
}

#[test]
fn parses_escapes_and_unicode() {
    let value = parse(r#""a\"b\\c\/d\b\f\n\r\t\u0041\u00e9\ud83d\ude00""#).unwrap();
    assert_eq!(
        value.as_str().unwrap(),
        "a\"b\\c/d\u{8}\u{c}\n\r\tA\u{e9}\u{1f600}"
    );
    // Raw unicode survives verbatim.
    assert_eq!(parse("\"λ日本語🙂\"").unwrap().as_str(), Some("λ日本語🙂"));
}

// --- invalid / rejection corpus --------------------------------------------

#[test]
fn rejects_malformed() {
    for bad in [
        "",
        "{",
        "[",
        "\"unterminated",
        "tru",
        "nul",
        "01",
        "1.",
        ".5",
        "{\"a\":}",
        "[1,]",
        "{\"a\" 1}",
        "truX",
        "{'a':1}",
        "1 2",
        "}",
        "]",
        "\"\\x\"",
        "\"\\u12\"",
        "\"\\uZZZZ\"",
    ] {
        assert!(parse(bad).is_err(), "expected rejection for {bad:?}");
    }
}

#[test]
fn rejects_duplicate_keys() {
    assert!(parse(r#"{"a":1,"a":2}"#).is_err());
    assert!(parse(r#"{"a":1,"b":2,"a":3}"#).is_err());
}

#[test]
fn rejects_invalid_utf8_bytes() {
    assert!(parse_bytes(&[0x22, 0xff, 0x22]).is_err());
    assert!(parse_bytes(b"{\"a\":1}").is_ok());
}

// --- semantics: insertion order, numbers ------------------------------------

#[test]
fn preserves_object_member_order() {
    let value = parse(r#"{"z":1,"a":2,"m":3}"#).unwrap();
    let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
    assert_eq!(keys, vec!["z", "a", "m"]);
}

#[test]
fn number_boundaries() {
    assert_eq!(parse("0").unwrap().as_number(), Some(0.0));
    assert_eq!(parse("-0").unwrap().as_number(), Some(-0.0));
    assert_eq!(parse("1.5").unwrap().as_number(), Some(1.5));
    assert_eq!(parse("1e10").unwrap().as_number(), Some(1e10));
    // Non-finite results are rejected by the reference (finite-range rule).
    assert!(
        parse("1e999").is_err(),
        "non-finite number must be rejected"
    );
}

// --- stringify / round-trip property ----------------------------------------

#[test]
fn stringify_canonical() {
    let value = parse(r#"{"b":true,"n":1.5,"s":"a\"b","a":[1,null,{}],"z":0}"#).unwrap();
    assert_eq!(
        stringify(&value),
        r#"{"b":true,"n":1.5,"s":"a\"b","a":[1,null,{}],"z":0}"#
    );
    assert_eq!(stringify(&Value::object()), "{}");
    assert_eq!(stringify(&Value::array()), "[]");
    assert_eq!(stringify(&Value::null()), "null");
}

#[test]
fn parse_stringify_parse_round_trips() {
    let documents = [
        r#"{"a":1,"b":[true,false,null],"c":{"d":"e","f":[1.5,-2,0]}}"#,
        r#"["", "\u00e9", "λ", "a\nb", "quote\"here"]"#,
        r#"{"nested":{"deep":{"deeper":[{"x":1},{"y":null}]}}}"#,
        r#"{"empty_obj":{},"empty_arr":[],"zero":0,"neg":-0.0,"big":1e300}"#,
    ];
    for doc in documents {
        let first = parse(doc).unwrap();
        let serialized = stringify(&first);
        let second = parse(&serialized).unwrap();
        assert_eq!(first, second, "round-trip failed for {doc}");
    }
}

#[test]
fn stringify_escapes_control_and_quotes() {
    let value = Value::string("a\"b\\c\u{1}\u{2}\n\t");
    assert_eq!(stringify(&value), "\"a\\\"b\\\\c\\u0001\\u0002\\n\\t\"");
}

// --- schema validation ------------------------------------------------------

#[test]
fn schema_accept_and_reject() {
    let schema = parse(
        r#"{"type":"object","required":["name","rank"],
           "properties":{"name":{"type":"string"},"rank":{"type":"integer","minimum":1}}}"#,
    )
    .unwrap();
    let ok = parse(r#"{"name":"x","rank":2}"#).unwrap();
    let bad = parse(r#"{"name":"x","rank":0}"#).unwrap();
    let missing = parse(r#"{"name":"x"}"#).unwrap();
    let wrong_type = parse(r#"{"name":1,"rank":2}"#).unwrap();
    assert!(validate(&ok, &schema).is_ok());
    assert!(validate(&bad, &schema).is_err());
    assert!(validate(&missing, &schema).is_err());
    assert!(validate(&wrong_type, &schema).is_err());
}

// --- adversarial robustness (no panic on arbitrary input) --------------------

#[test]
fn adversarial_inputs_never_panic() {
    let corpus = [
        "{{{{{{{{{{",
        "]]]][[[[",
        "\"\\u",
        "1e",
        "-",
        "--1",
        "{\"\":}",
        "[1,,2]",
        "\"\\ud800\"",
        "\"\\ud800\\ud800\"",
        "\x00\x01\x02",
        "\"a",
        "tru",
        "nulll",
        "{\"a\":1,\"a\":1}",
        "[[[]]]",
        "1e999999999",
        "\"\\\"",
        "{\"a\" : 1}",
    ];
    for input in corpus {
        let _ = parse(input); // must not panic
    }
}

// --- representative throughput benchmark -------------------------------------

#[test]
fn benchmark_parse_throughput() {
    let document = r#"{"name":"nift","version":"0.1.0","tags":["json","rust","embed"],
        "count":42,"ratio":3.14159,"ok":true,"meta":{"a":1,"b":[1,2,3],"c":{"d":null}}}"#;
    const ROUNDS: usize = 50_000;
    let start = std::time::Instant::now();
    let mut checksum = 0usize;
    for _ in 0..ROUNDS {
        let value = parse(document).unwrap();
        checksum += stringify(&value).len();
    }
    let elapsed = start.elapsed();
    let bytes = document.len() as f64 * ROUNDS as f64;
    let mb_per_s = bytes / elapsed.as_secs_f64() / (1024.0 * 1024.0);
    println!(
        "jsonic parse+stringify: {ROUNDS} docs in {elapsed:?} ({mb_per_s:.1} MiB/s), checksum={checksum}"
    );
    assert!(mb_per_s > 1.0, "suspiciously slow parse throughput");
}
