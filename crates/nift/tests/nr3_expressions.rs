//! NR3 expression-surface differential tests: arithmetic, comparison, logical
//! and unary operators in `$[...]` and `@if`, with reference-derived
//! expectations captured from the nift-embed CLI.

use nift::bindings::Bindings;
use nift::context::Context;
use nift::{render, InMemoryHost, RenderIdentity, Source, Value};

fn make_host<'a>(defaults: &'a Bindings, context: &'a Context) -> InMemoryHost<'a> {
    InMemoryHost::new(defaults, context, "/site")
}

fn render_with(identity: &RenderIdentity, template: &str) -> String {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    render(&host, identity, &Source::text(template), None)
        .expect("render should succeed")
        .output
}

#[test]
fn arithmetic_in_value_lookup() {
    let mut defaults = Bindings::new();
    defaults.set("a", Value::number(5.0)).unwrap();
    defaults.set("b", Value::number(3.0)).unwrap();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("t").title("TT");
    assert_eq!(
        render(&host, &identity, &Source::text("$[a + b]"), None)
            .unwrap()
            .output,
        "8"
    );
    assert_eq!(
        render(&host, &identity, &Source::text("$[a * b]"), None)
            .unwrap()
            .output,
        "15"
    );
    assert_eq!(
        render(&host, &identity, &Source::text("$[10 / 4]"), None)
            .unwrap()
            .output,
        "2.5"
    );
    assert_eq!(
        render(&host, &identity, &Source::text("$[7 % 3]"), None)
            .unwrap()
            .output,
        "1"
    );
    assert_eq!(
        render(&host, &identity, &Source::text("$[(1 + 2) * 3]"), None)
            .unwrap()
            .output,
        "9"
    );
    assert_eq!(
        render(&host, &identity, &Source::text("$[-b]"), None)
            .unwrap()
            .output,
        "-3"
    );
}

#[test]
fn comparison_and_logic_in_value_lookup() {
    let mut defaults = Bindings::new();
    defaults.set("a", Value::number(5.0)).unwrap();
    defaults.set("b", Value::number(3.0)).unwrap();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("t").title("TT");
    assert_eq!(
        render(&host, &identity, &Source::text("$[a < 10]"), None)
            .unwrap()
            .output,
        "true"
    );
    assert_eq!(
        render(&host, &identity, &Source::text("$[a == 5]"), None)
            .unwrap()
            .output,
        "true"
    );
    assert_eq!(
        render(&host, &identity, &Source::text("$[a != 5]"), None)
            .unwrap()
            .output,
        "false"
    );
    assert_eq!(
        render(&host, &identity, &Source::text("$[b >= 3]"), None)
            .unwrap()
            .output,
        "true"
    );
    assert_eq!(
        render(&host, &identity, &Source::text("$[b > 3]"), None)
            .unwrap()
            .output,
        "false"
    );
    assert_eq!(
        render(&host, &identity, &Source::text("$[a > 4 && b < 5]"), None)
            .unwrap()
            .output,
        "true"
    );
    assert_eq!(
        render(&host, &identity, &Source::text("$[a == 1 || b == 3]"), None)
            .unwrap()
            .output,
        "true"
    );
}

#[test]
fn logic_and_negation_in_if() {
    let mut defaults = Bindings::new();
    defaults.set("a", Value::number(5.0)).unwrap();
    defaults.set("b", Value::number(3.0)).unwrap();
    defaults.set("flag", Value::boolean(true)).unwrap();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("t").title("TT");
    assert_eq!(
        render(
            &host,
            &identity,
            &Source::text("@if(a > 4 && b < 5){Y} else {N}"),
            None
        )
        .unwrap()
        .output,
        "Y"
    );
    assert_eq!(
        render(
            &host,
            &identity,
            &Source::text("@if(a > 9 || b == 3){Y} else {N}"),
            None
        )
        .unwrap()
        .output,
        "Y"
    );
    assert_eq!(
        render(
            &host,
            &identity,
            &Source::text("@if(a + b == 8){EIGHT} else {OTHER}"),
            None
        )
        .unwrap()
        .output,
        "EIGHT"
    );
}

#[test]
fn string_comparisons_and_truthiness() {
    let mut defaults = Bindings::new();
    defaults.set("word", Value::string("hello")).unwrap();
    defaults.set("empty", Value::string("")).unwrap();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("t").title("TT");
    assert_eq!(
        render(
            &host,
            &identity,
            &Source::text("@if(word == \"hello\"){Y} else {N}"),
            None
        )
        .unwrap()
        .output,
        "Y"
    );
    assert_eq!(
        render(
            &host,
            &identity,
            &Source::text("@if(word){Y} else {N}"),
            None
        )
        .unwrap()
        .output,
        "Y"
    );
    assert_eq!(
        render(
            &host,
            &identity,
            &Source::text("@if(empty){Y} else {N}"),
            None
        )
        .unwrap()
        .output,
        "N"
    );
}

#[test]
fn arithmetic_errors_are_controlled() {
    let mut defaults = Bindings::new();
    defaults.set("n", Value::number(1.0)).unwrap();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("t").title("TT");
    let division = render(&host, &identity, &Source::text("$[1 / 0]"), None)
        .expect_err("division by zero must fail");
    assert!(division.message.contains("division by zero"));
    let non_numeric = render(&host, &identity, &Source::text("$[n + \"x\"]"), None)
        .expect_err("non-numeric arithmetic must fail");
    assert!(non_numeric
        .message
        .contains("arithmetic operators require numeric operands"));
}

#[test]
fn if_uses_full_expression_evaluation() {
    let identity = RenderIdentity::new().name("t").title("TT");
    assert_eq!(
        render_with(&identity, "@if(2 * 3 == 6){SIX} else {N}"),
        "SIX"
    );
    assert_eq!(
        render_with(&identity, "@if(1 < 2 && 3 > 2){Y} else {N}"),
        "Y"
    );
    assert_eq!(
        render_with(&identity, "@if((2 + 2) == 4){FOUR} else {N}"),
        "FOUR"
    );
}
