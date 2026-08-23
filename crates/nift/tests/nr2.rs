//! NR2 integration tests: the parser kernel I through the public
//! [`nift::render`] + [`nift::InMemoryHost`] seam.
//!
//! Expected outputs are differential expectations captured from the frozen C++
//! reference (nift-embed CLI) — where the contract is silent the reference's
//! observable behaviour is pinned here and recorded, not silently canonized.

use nift::bindings::Bindings;
use nift::context::Context;
use nift::{render, InMemoryHost, RenderIdentity, Source, Value};
use std::path::Path;

fn identity() -> RenderIdentity {
    RenderIdentity::new()
        .name("about")
        .title("About")
        .template_path("templates/page.html")
}

fn host<'a>(defaults: &'a Bindings, context: &'a Context) -> InMemoryHost<'a> {
    InMemoryHost::new(defaults, context, "/site")
}

fn render_text(host: &InMemoryHost<'_>, template: &str) -> String {
    render(host, &identity(), &Source::text(template), None)
        .expect("render should succeed")
        .output
}

fn render_text_err(host: &InMemoryHost<'_>, template: &str) -> nift::RenderError {
    render(host, &identity(), &Source::text(template), None).expect_err("render should fail")
}

// --- literal text, escaping, comments -------------------------------------

#[test]
fn literal_text_passthrough() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    assert_eq!(render_text(&host, "hello world"), "hello world");
    assert_eq!(render_text(&host, "a\nb\tc"), "a\nb\tc");
}

#[test]
fn backslash_escaping() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    assert_eq!(render_text(&host, "a\\@b \\$c \\#d"), "a@b $c #d");
}

#[test]
fn line_comments_removed_keeping_newlines() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    assert_eq!(render_text(&host, "x@# c\ny\n@// z\nend"), "x\ny\n\nend");
}

#[test]
fn block_comments_removed() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    assert_eq!(render_text(&host, "<#-- hi --#>lit"), "lit");
    assert_eq!(render_text(&host, "a<#-- x --#>b"), "ab");
}

#[test]
fn unclosed_block_comment_is_an_error() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    let error = render_text_err(&host, "a<#-- nope");
    assert!(error.message.contains("no close '--#>'"));
}

#[test]
fn pre_blocks_escape_less_than() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    // Reference-derived: inside <pre*>, non-code <...> becomes &lt;...;
    // the outer </pre> is preserved.
    assert_eq!(
        render_text(&host, "i=<pre><b>x</b></pre>o"),
        "i=<pre>&lt;b>x&lt;/b></pre>o"
    );
}

#[test]
fn unclosed_pre_is_an_error() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    let error = render_text_err(&host, "</pre>");
    assert!(error.message.contains("</pre> close tag has no preceding"));
}

// --- $[...] value lookup ---------------------------------------------------

#[test]
fn value_lookup_from_bindings() {
    let mut defaults = Bindings::new();
    defaults.set("greeting", Value::string("Hello")).unwrap();
    defaults.set("count", Value::number(3.0)).unwrap();
    defaults.set("enabled", Value::boolean(true)).unwrap();
    defaults.set("nothing", Value::null()).unwrap();
    let context = Context::new();
    let host = host(&defaults, &context);
    assert_eq!(render_text(&host, "$[greeting]"), "Hello");
    assert_eq!(render_text(&host, "$[count]"), "3");
    assert_eq!(render_text(&host, "$[enabled]"), "true");
    assert_eq!(render_text(&host, "$[nothing]"), "null");
}

#[test]
fn value_lookup_navigation() {
    let mut defaults = Bindings::new();
    let mut user = Value::object();
    user.insert("name", Value::string("Nick")).unwrap();
    user.insert(
        "roles",
        Value::Array(vec![Value::string("a"), Value::string("b")]),
    )
    .unwrap();
    defaults.set("user", user).unwrap();
    let context = Context::new();
    let host = host(&defaults, &context);
    assert_eq!(render_text(&host, "$[user.name]"), "Nick");
    assert_eq!(render_text(&host, "$[user.roles[1]]"), "b");
    assert_eq!(render_text(&host, "$[user.roles[0]]"), "a");
}

#[test]
fn unknown_value_renders_literally() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    // Reference-derived: an unresolvable $[...] is emitted literally.
    assert_eq!(render_text(&host, "a$[unknown]b"), "a$[unknown]b");
    assert_eq!(render_text(&host, "x$[a.b.c]y"), "x$[a.b.c]y");
}

#[test]
fn array_or_object_direct_render_is_an_error() {
    let mut defaults = Bindings::new();
    defaults
        .set("arr", Value::Array(vec![Value::number(1.0)]))
        .unwrap();
    defaults.set("obj", Value::object()).unwrap();
    let context = Context::new();
    let host = host(&defaults, &context);
    let array_error = render_text_err(&host, "$[arr]");
    assert!(array_error.message.contains("cannot render JSON array"));
    let object_error = render_text_err(&host, "$[obj]");
    assert!(object_error.message.contains("cannot render JSON object"));
}

#[test]
fn non_object_navigation_is_an_error() {
    let mut defaults = Bindings::new();
    defaults.set("s", Value::string("x")).unwrap();
    let context = Context::new();
    let host = host(&defaults, &context);
    let error = render_text_err(&host, "$[s.member]");
    assert!(error.message.contains("cannot access member 'member'"));
}

#[test]
fn metadata_lookup() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    assert_eq!(render_text(&host, "$[title]"), "About");
    assert_eq!(render_text(&host, "$[name]"), "about");
    assert_eq!(render_text(&host, "$[content-path]"), "content/about.html");
    assert_eq!(render_text(&host, "$[output-path]"), "public/about.html");
    assert_eq!(
        render_text(&host, "$[template-path]"),
        "templates/page.html"
    );
}

#[test]
fn build_time_metadata_has_expected_shape() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    // Time-dependent built-ins are excluded from byte goldens; verify shape.
    let date = render_text(&host, "$[build-date]");
    assert_eq!(date.len(), 10);
    assert_eq!(&date[4..5], "-");
    assert_eq!(&date[7..8], "-");
    let year = render_text(&host, "$[build-YYYY]");
    assert_eq!(year.len(), 4);
    let time = render_text(&host, "$[build-time]");
    assert_eq!(time.len(), 8);
    assert_eq!(&time[2..3], ":");
    assert_eq!(&time[5..6], ":");
    let os = render_text(&host, "$[build-OS]");
    assert!(os == "Linux" || os == "macOS" || os == "Windows");
}

// --- @content --------------------------------------------------------------

#[test]
fn content_injects_text_page() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    let result = render(
        &host,
        &identity(),
        &Source::text("<p>@content</p>"),
        Some(&Source::text("BODY")),
    )
    .expect("render should succeed");
    assert_eq!(result.output, "<p>BODY</p>");
}

#[test]
fn content_injects_path_page_through_host_seam() {
    let defaults = Bindings::new();
    let context = Context::new();
    let absolute =
        std::path::absolute(Path::new("/site/content/about.html")).expect("absolute path");
    let host = host(&defaults, &context).with_source(absolute.clone(), "<b>body</b>");
    let result = render(
        &host,
        &identity(),
        &Source::text("<p>@content</p>"),
        Some(&Source::path("content/about.html")),
    )
    .expect("render should succeed");
    assert_eq!(result.output, "<p><b>body</b></p>");
}

#[test]
fn content_parses_nested_directives() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    let result = render(
        &host,
        &identity(),
        &Source::text("<p>@content</p>"),
        Some(&Source::text("@if(true){inner}")),
    )
    .expect("render should succeed");
    assert_eq!(result.output, "<p>inner</p>");
}

#[test]
fn content_without_page_is_an_error() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    let error = render_text_err(&host, "@content");
    assert!(error.message.contains("@content requires a page source"));
}

#[test]
fn composed_render_requires_exactly_one_content() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    // No @content in a composed render is an error.
    let error = render(
        &host,
        &identity(),
        &Source::text("<p>no content</p>"),
        Some(&Source::text("BODY")),
    )
    .expect_err("composed render without @content must fail");
    assert!(error.message.contains("exactly one @content"));

    // Two @content in one render is an error.
    let error = render(
        &host,
        &identity(),
        &Source::text("@content@content"),
        Some(&Source::text("BODY")),
    )
    .expect_err("two @content must fail");
    assert!(error.message.contains("exactly once"));
}

#[test]
fn content_loop_is_an_error() {
    let defaults = Bindings::new();
    let context = Context::new();
    let absolute =
        std::path::absolute(Path::new("/site/content/about.html")).expect("absolute path");
    let host = host(&defaults, &context).with_source(absolute.clone(), "@content");
    let error = render(
        &host,
        &identity(),
        &Source::path("content/about.html"),
        Some(&Source::path("content/about.html")),
    )
    .expect_err("content loop must fail");
    assert!(error.message.contains("input loop"));
}

// --- @if -------------------------------------------------------------------

#[test]
fn if_constant_conditions() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    assert_eq!(render_text(&host, "@if(true){yes}@if(false){no}"), "yes");
}

#[test]
fn if_truthiness() {
    let mut defaults = Bindings::new();
    defaults.set("empty", Value::string("")).unwrap();
    defaults.set("nonempty", Value::string("x")).unwrap();
    defaults.set("zero", Value::number(0.0)).unwrap();
    defaults.set("one", Value::number(1.0)).unwrap();
    defaults.set("flag", Value::boolean(true)).unwrap();
    defaults.set("unflag", Value::boolean(false)).unwrap();
    defaults.set("nil", Value::null()).unwrap();
    let context = Context::new();
    let host = host(&defaults, &context);
    assert_eq!(render_text(&host, "@if(empty){T} else {F}"), "F");
    assert_eq!(render_text(&host, "@if(nonempty){T} else {F}"), "T");
    assert_eq!(render_text(&host, "@if(zero){T} else {F}"), "F");
    assert_eq!(render_text(&host, "@if(one){T} else {F}"), "T");
    assert_eq!(render_text(&host, "@if(flag){T} else {F}"), "T");
    assert_eq!(render_text(&host, "@if(unflag){T} else {F}"), "F");
    assert_eq!(render_text(&host, "@if(nil){T} else {F}"), "F");
}

#[test]
fn if_else_and_else_if_chain() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    assert_eq!(render_text(&host, "@if(false){x} else {y}"), "y");
    assert_eq!(
        render_text(&host, "@if(false){x} else if(true){y} else {z}"),
        "y"
    );
    assert_eq!(
        render_text(&host, "@if(false){x} else if(false){y} else {z}"),
        "z"
    );
}

#[test]
fn if_nested_and_adjacent() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    assert_eq!(render_text(&host, "@if(true){@if(true){deep}}"), "deep");
    assert_eq!(render_text(&host, "@if(true){a}@if(false){b}"), "a");
}

#[test]
fn if_block_indentation_is_structural_not_output() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    // Reference-derived: multiline control blocks are dedented; the structural
    // indentation is readability, not output.
    assert_eq!(
        render_text(&host, "start\n@if(true){\n    <p>ok</p>\n}\nend"),
        "start\n<p>ok</p>\nend"
    );
    assert_eq!(
        render_text(&host, "a\n@if(false){\n    skip\n} else {\n    keep\n}\nz"),
        "a\nkeep\nz"
    );
}

#[test]
fn if_condition_errors() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = host(&defaults, &context);
    assert!(render_text_err(&host, "@if(missing){x}")
        .message
        .contains("unknown value or malformed expression"));
    assert!(render_text_err(&host, "@if(true) hello")
        .message
        .contains("must be followed by a '{...}' block"));
    assert!(render_text_err(&host, "@if(")
        .message
        .contains("no matching ')'"));
}

// --- errors carry location -------------------------------------------------

#[test]
fn errors_carry_source_location() {
    let mut defaults = Bindings::new();
    defaults.set("s", Value::string("x")).unwrap();
    let context = Context::new();
    let host = host(&defaults, &context);
    let error = render_text_err(&host, "line1\nline2 $[s.member]");
    assert_eq!(error.line, Some(2));
    assert!(error.column.is_some());
    assert_eq!(error.kind, nift::ErrorKind::Render);
}
