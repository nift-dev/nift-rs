//! NR4 differential tests through the public render path (filesystem host for
//! @input/@json/@dep; in-memory env for @getenv). Expectations captured from
//! the nift-embed CLI.

use nift::bindings::Bindings;
use nift::context::Context;
use nift::{render, FilesystemHost, InMemoryHost, RenderIdentity, Source, Value};
use std::path::PathBuf;

fn write_file(path: &std::path::Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn temp_root() -> PathBuf {
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("nift-nr4-{}-{id}", std::process::id()))
}

fn write_project(root: &std::path::Path) {
    write_file(&root.join("content/t.html"), "@content");
    write_file(&root.join("templates/head.html"), "<h1>@ent(\"&\")</h1>");
    write_file(&root.join("templates/partial.html"), "P");
    write_file(
        &root.join("templates/nested.html"),
        "A@input(\"templates/head.html\")B",
    );
    write_file(
        &root.join("data.json"),
        r#"{"items":[{"name":"b","rank":2},{"name":"a","rank":1}]}"#,
    );
    write_file(
        &root.join("schemas/items.schema.json"),
        r#"{"type":"object","required":["items"],"properties":{"items":{"type":"array","items":{"type":"object","required":["name","rank"],"properties":{"name":{"type":"string"},"rank":{"type":"integer"}},"additionalProperties":false}}},"additionalProperties":false}"#,
    );
    write_file(&root.join("extra.json"), r#"{"value":42}"#);
}

fn fs_render(root: &std::path::Path, template: &str) -> nift::RenderResult {
    write_file(&root.join("content/t.html"), template);
    let defaults = Bindings::new();
    let context = Context::new();
    let host = FilesystemHost::new(&defaults, &context, root);
    let identity = RenderIdentity::new().name("t").title("T");
    render(&host, &identity, &Source::path("content/t.html"), None).expect("render should succeed")
}

fn fs_render_err(root: &std::path::Path, template: &str) -> nift::RenderError {
    write_file(&root.join("content/t.html"), template);
    let defaults = Bindings::new();
    let context = Context::new();
    let host = FilesystemHost::new(&defaults, &context, root);
    let identity = RenderIdentity::new().name("t").title("T");
    render(&host, &identity, &Source::path("content/t.html"), None).expect_err("render should fail")
}

#[test]
fn input_directive() {
    let root = temp_root();
    write_project(&root);
    // @input relative to the project root.
    let result = fs_render(&root, "@input(\"templates/head.html\")");
    assert_eq!(result.output, "<h1>&amp;</h1>");
    assert!(result.dependencies.contains("templates/head.html"));
    // Nested @input.
    let result = fs_render(&root, "@input(\"templates/nested.html\")");
    assert_eq!(result.output, "A<h1>&amp;</h1>B");
    // @input with expressions/partial.
    let result = fs_render(&root, "<p>@input(\"templates/partial.html\")</p>");
    assert_eq!(result.output, "<p>P</p>");
    // Missing input.
    let error = fs_render_err(&root, "@input(\"templates/missing.html\")");
    assert!(error.message.contains("@input path does not exist"));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn json_directive() {
    let root = temp_root();
    write_project(&root);
    // Bind + iterate.
    let result = fs_render(
        &root,
        "@json(\"data.json\", d)@for(x : d.items by x.rank asc){$[x.name]}|",
    );
    assert_eq!(result.output, "ab|");
    assert!(result.dependencies.contains("data.json"));
    // Schema success.
    let result = fs_render(
        &root,
        "@json(\"data.json\", d, \"schemas/items.schema.json\")@for(x : d.items by x.rank asc){$[x.name]}|",
    );
    assert_eq!(result.output, "ab|");
    assert!(result.dependencies.contains("schemas/items.schema.json"));
    // Binding collision.
    let error = fs_render_err(&root, "@json(\"data.json\", d)@json(\"extra.json\", d)");
    assert!(error.message.contains("is already bound"));
    // Traversal.
    let error = fs_render_err(&root, "@json(\"../outside.json\", d)");
    assert!(error
        .message
        .contains("path must stay inside the Nift project"));
    // Missing file.
    let error = fs_render_err(&root, "@json(\"nope.json\", d)");
    assert!(error.message.contains("file does not exist"));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn getenv_directive() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = InMemoryHost::new(&defaults, &context, "/site").with_env("PA_NR4_VAR", "hello");
    let identity = RenderIdentity::new().name("t").title("T");
    let result = render(
        &host,
        &identity,
        &Source::text("e=@getenv(\"PA_NR4_VAR\")|"),
        None,
    )
    .expect("render should succeed");
    assert_eq!(result.output, "e=hello|");
    // Missing variable renders empty.
    let result = render(
        &host,
        &identity,
        &Source::text("e=@getenv(\"MISSING\")|"),
        None,
    )
    .expect("render should succeed");
    assert_eq!(result.output, "e=|");
}

#[test]
fn dep_directive() {
    let root = temp_root();
    write_project(&root);
    let result = fs_render(&root, "x@dep(\"data.json\")@dep(\"templates/head.html\")y");
    assert_eq!(result.output, "xy");
    assert!(result.dependencies.contains("data.json"));
    assert!(result.dependencies.contains("templates/head.html"));
    // Missing dependency.
    let error = fs_render_err(&root, "@dep(\"nope.json\")");
    assert!(error
        .message
        .contains("failed as dependency does not exist"));
    // Traversal.
    let error = fs_render_err(&root, "@dep(\"../outside.json\")");
    assert!(error
        .message
        .contains("dep: path must stay inside the Nift project"));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn ent_directive() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = InMemoryHost::new(&defaults, &context, "/site");
    let identity = RenderIdentity::new().name("t").title("T");
    let result = render(
        &host,
        &identity,
        &Source::text("@ent(\"&\") @ent(\"<-\")"),
        None,
    )
    .expect("render should succeed");
    assert_eq!(result.output, "&amp; &larr;");
    // Unknown entity is a controlled error.
    let error = render(&host, &identity, &Source::text("@ent(\"bogus\")"), None)
        .expect_err("unknown entity must fail");
    assert!(error
        .message
        .contains("do not currently have an entity value"));
}

#[test]
fn dependencies_survive_the_render_result() {
    let root = temp_root();
    write_project(&root);
    let result = fs_render(
        &root,
        "@json(\"data.json\", d)@dep(\"templates/partial.html\")",
    );
    assert!(result.dependencies.contains("data.json"));
    assert!(result.dependencies.contains("templates/partial.html"));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn value_bindings_still_work_through_filesystem_host() {
    let mut defaults = Bindings::new();
    defaults.set("greeting", Value::string("Hi")).unwrap();
    let root = temp_root();
    write_project(&root);
    let context = Context::new();
    let host = FilesystemHost::new(&defaults, &context, &root);
    let identity = RenderIdentity::new().name("t").title("T");
    let result = render(&host, &identity, &Source::text("$[greeting]"), None)
        .expect("render should succeed");
    assert_eq!(result.output, "Hi");
    std::fs::remove_dir_all(&root).ok();
}
