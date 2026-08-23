//! NR3 parser-level differential tests through the PUBLIC render path
//! (template text → parser → expression/collection evaluator → output).
//! Expectations captured from the nift-embed CLI.

use nift::bindings::Bindings;
use nift::context::Context;
use nift::{render, InMemoryHost, RenderIdentity, Source, Value};

fn make_host<'a>(defaults: &'a Bindings, context: &'a Context) -> InMemoryHost<'a> {
    InMemoryHost::new(defaults, context, "/site")
}

fn render_template(template: &str) -> String {
    let mut defaults = Bindings::new();
    defaults
        .set(
            "nums",
            Value::Array(vec![
                Value::number(3.0),
                Value::number(1.0),
                Value::number(2.0),
            ]),
        )
        .unwrap();
    defaults
        .set(
            "words",
            Value::Array(vec![Value::string("b"), Value::string("a")]),
        )
        .unwrap();
    let mut posts = Value::array();
    for (title, score, published) in [("x", 5.0, true), ("y", 3.0, false), ("z", 7.0, true)] {
        let mut post = Value::object();
        post.insert("title", Value::string(title)).unwrap();
        post.insert("score", Value::number(score)).unwrap();
        post.insert("pub", Value::boolean(published)).unwrap();
        posts.push(post).unwrap();
    }
    defaults.set("posts", posts).unwrap();
    let mut obj = Value::object();
    obj.insert("b", Value::number(2.0)).unwrap();
    obj.insert("a", Value::number(1.0)).unwrap();
    defaults.set("obj", obj).unwrap();
    let mut sort_obj = Value::object();
    sort_obj.insert("b", Value::number(3.0)).unwrap();
    sort_obj.insert("a", Value::number(1.0)).unwrap();
    sort_obj.insert("c", Value::number(2.0)).unwrap();
    defaults.set("sortobj", sort_obj).unwrap();
    let mut nested = Value::object();
    for (key, rank) in [("first", 3.0), ("second", 1.0), ("third", 2.0)] {
        let mut entry = Value::object();
        entry.insert("rank", Value::number(rank)).unwrap();
        nested.insert(key, entry).unwrap();
    }
    defaults.set("nested", nested).unwrap();
    let mut dup = Value::object();
    dup.insert("x", Value::number(1.0)).unwrap();
    dup.insert("y", Value::number(1.0)).unwrap();
    dup.insert("z", Value::number(2.0)).unwrap();
    defaults.set("dup", dup).unwrap();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("t").title("T");
    render(&host, &identity, &Source::text(template), None)
        .expect("render should succeed")
        .output
}

fn render_template_err(template: &str) -> nift::RenderError {
    let mut defaults = Bindings::new();
    defaults
        .set(
            "nums",
            Value::Array(vec![
                Value::number(3.0),
                Value::number(1.0),
                Value::number(2.0),
            ]),
        )
        .unwrap();
    let mut sort_obj = Value::object();
    sort_obj.insert("b", Value::number(3.0)).unwrap();
    sort_obj.insert("a", Value::number(1.0)).unwrap();
    sort_obj.insert("c", Value::number(2.0)).unwrap();
    defaults.set("sortobj", sort_obj).unwrap();
    let mut nested = Value::object();
    for (key, rank) in [("first", 3.0), ("second", 1.0), ("third", 2.0)] {
        let mut entry = Value::object();
        entry.insert("rank", Value::number(rank)).unwrap();
        nested.insert(key, entry).unwrap();
    }
    defaults.set("nested", nested).unwrap();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("t").title("T");
    render(&host, &identity, &Source::text(template), None).expect_err("render should fail")
}

#[test]
fn for_array_iteration_and_loop_metadata() {
    // Reference-derived: array iteration and 1-based loop.index / length.
    assert_eq!(render_template("@for(x : nums){<$[x]>}"), "<3><1><2>");
    assert_eq!(
        render_template("@for(x : nums){$[loop.index]:$[x]/$[loop.length] }"),
        "1:3/3 2:1/3 3:2/3 "
    );
}

#[test]
fn for_object_iteration() {
    // Reference-derived: (key, value) object iteration in insertion order.
    assert_eq!(
        render_template("@for((k, v) : obj){$[k]=$[v];}"),
        "b=2;a=1;"
    );
}

#[test]
fn for_sort_asc_desc() {
    // Reference-derived: `by ... desc` sorted iteration with loop metadata.
    assert_eq!(
        render_template(
            "@for(p : posts by p.score desc){$[p.title]($[loop.index]/$[loop.length])}"
        ),
        "z(1/3)x(2/3)y(3/3)"
    );
    assert_eq!(
        render_template("@for(p : posts by p.score asc){$[p.title]}"),
        "yxz"
    );
}

#[test]
fn for_interactions() {
    // @for + @if + arithmetic.
    assert_eq!(
        render_template("@for(x : nums){@if(x > 1){$[x * 2]}}"),
        "64"
    );
    // Nested @for: outer x iterates nums in order [3,1,2].
    assert_eq!(
        render_template("@for(x : nums){@for(y : nums){$[x]$[y];}}"),
        "33;31;32;13;11;12;23;21;22;"
    );
    // loop metadata inside @if.
    assert_eq!(
        render_template("@for(x : nums){@if(loop.first){F}@if(loop.last){L}}"),
        "FL"
    );
}

#[test]
fn collection_directives_render_json() {
    // Reference-derived: collection directives output compact-ish JSON.
    assert_eq!(render_template("S=[@sort(nums)]"), "S=[[\n1,\n2,\n3\n]]");
    assert_eq!(render_template("S=[@sum(nums)]"), "S=[6]");
    assert_eq!(
        render_template("M=[@map(p : posts => p.title)]"),
        "M=[[\n\"x\",\n\"y\",\n\"z\"\n]]"
    );
    assert_eq!(
        render_template("R=[@reduce(n : nums & acc = 0 => acc + n)]"),
        "R=[6]"
    );
    assert_eq!(
        render_template("F=[@filter(p : posts => p.pub && p.score >= 5)]"),
        "F=[[\n{\n\"title\": \"x\",\n\"score\": 5,\n\"pub\": true\n},\n{\n\"title\": \"z\",\n\"score\": 7,\n\"pub\": true\n}\n]]"
    );
    // @for sourced from a collection operator.
    assert_eq!(
        render_template("@for(x : @filter(p : posts => p.pub)){$[x.title]}"),
        "xz"
    );
}

#[test]
fn substr_and_join_directives() {
    // Reference-derived.
    assert_eq!(render_template("s=@substr(\"hello\", 1, 3)"), "s=ell");
    assert_eq!(render_template("j=@join(words, \",\")"), "j=b,a");
    assert_eq!(render_template("j=@join(nums, \"-\")"), "j=3-1-2");
}

#[test]
fn item_and_paginate_are_controlled_errors_outside_tracked_pages() {
    assert!(render_template_err("@item{x}")
        .message
        .contains("@item requires pagination on the tracked item"));
    assert!(render_template_err("x@paginate")
        .message
        .contains("@paginate requires pagination on the tracked item"));
}

#[test]
fn for_negative_cases() {
    assert!(render_template_err("@for(x : nums)")
        .message
        .contains("must be followed by a '{...}' block"));
    assert!(render_template_err("@for(x nums){a}")
        .message
        .contains("header must contain ':'"));
    assert!(render_template_err("@for(loop : nums){a}")
        .message
        .contains("conflicts with built-in metadata"));
    assert!(render_template_err("@for(9bad : nums){a}")
        .message
        .contains("array @for syntax"));
    // @for over a non-collection is an error.
    let mut defaults = Bindings::new();
    defaults.set("n", Value::number(5.0)).unwrap();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("t").title("T");
    let error = render(&host, &identity, &Source::text("@for(x : n){a}"), None)
        .expect_err("@for over a scalar must fail");
    assert!(error
        .message
        .contains("can only iterate over JSON arrays or objects"));
}

#[test]
fn substr_and_join_errors() {
    assert!(render_template_err("@substr(\"hello\", -1, 2)")
        .message
        .contains("non-negative integer"));
    assert!(render_template_err("@join(nums)")
        .message
        .contains("expected array and separator"));
    let mut defaults = Bindings::new();
    defaults.set("n", Value::number(5.0)).unwrap();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("t").title("T");
    let error = render(&host, &identity, &Source::text("@join(n, \",\")"), None)
        .expect_err("@join over a scalar must fail");
    assert!(error
        .message
        .contains("first parameter must resolve to a JSON array"));
}

#[test]
fn for_object_sorting() {
    // Reference-derived: object loops accept and APPLY sort clauses. Insertion
    // order is b,a,c; sorted orders differ so a no-op sorter cannot pass.
    assert_eq!(
        render_template(
            "@for((k, v) : sortobj by k asc){$[k]}@for((k, v) : sortobj by k desc){$[k]}|"
        ),
        "abccba|"
    );
    assert_eq!(
        render_template(
            "@for((k, v) : sortobj by v asc){$[k]}@for((k, v) : sortobj by v desc){$[k]}|"
        ),
        "acbbca|"
    );
    // Nested value-field sort root (value_name.<path>).
    assert_eq!(
        render_template("@for((k, v) : nested by v.rank asc){$[k]}|"),
        "secondthirdfirst|"
    );
    // Equal sort keys are stable (insertion order preserved within ties).
    assert_eq!(
        render_template("@for((k, v) : dup by v asc){$[k]}|"),
        "xyz|"
    );
}

#[test]
fn for_object_sort_negative_cases() {
    // Unsupported sort root (unbounded name).
    let error = render_template_err("@for((k, v) : sortobj by q asc){$[k]}");
    assert!(error
        .message
        .contains("object sort key must begin with key/value binding"));
    // Missing sort path member (reference: hard navigation error).
    let error = render_template_err("@for((k, v) : nested by v.missing asc){$[k]}");
    assert!(error.message.contains("has no member 'missing'"));
}

#[test]
fn missing_member_navigation_is_a_hard_error() {
    // Reference: a known root with a missing member errors (unlike an unknown
    // root, which falls through to literal emission).
    let mut defaults = Bindings::new();
    let mut obj = Value::object();
    obj.insert("b", Value::number(2.0)).unwrap();
    defaults.set("obj", obj).unwrap();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("t").title("T");
    let error = render(&host, &identity, &Source::text("$[obj.missing]"), None)
        .expect_err("missing member must be a hard error");
    assert!(error
        .message
        .contains("JSON value 'obj' has no member 'missing'"));
    let error = render(&host, &identity, &Source::text("$[obj.c]"), None)
        .expect_err("missing nested member must be a hard error");
    assert!(error.message.contains("has no member 'c'"));
    // Out-of-range element is a hard error.
    let mut defaults = Bindings::new();
    defaults
        .set("arr", Value::Array(vec![Value::number(1.0)]))
        .unwrap();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let error = render(&host, &identity, &Source::text("$[arr[5]]"), None)
        .expect_err("out-of-range element must be a hard error");
    assert!(error.message.contains("out of range"));
}
