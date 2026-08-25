//! NR15: Embed host-seam failure contract (CP10.2, nift-rs equivalent).
//!
//! The environment/loader provider contract is value / absent / error
//! (`HostResult`). A host `Error` travels through the render computation: the
//! RenderResult fails with the diagnostic, identically for standalone and
//! paginated renders. `NotFound` is ordinary "unset"; `Found` with an empty
//! value is "present but empty".

use nift::context::Context;
use nift::host::HostResult;
use nift::{Engine, RenderResult};
use std::path::{Path, PathBuf};
use std::thread;

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(path, contents).expect("write file");
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nift-nr15-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn two_page_project(root: &Path) {
    write_file(
        &root.join(".nift/config.json"),
        r#"{"config":{"content-dir":"content/","output-dir":"public/","default-template":"templates/template.html","build-threads":-1,"incremental-mode":"modified"}}"#,
    );
    write_file(
        &root.join(".nift/tracked.json"),
        r#"{"tracked":[
 {"name":"blog","title":"Blog","template":"templates/template.html","paginate":{"items-per-page":1}},
 {"name":"other","title":"Other","template":"templates/template.html","paginate":{"items-per-page":1}}
]}"#,
    );
    write_file(&root.join("templates/template.html"), "<main>$[title]</main>\n@content");
    write_file(&root.join("content/blog.html"), "@item{one}@item{two}@item{three}@paginate");
    write_file(&root.join("content/other.html"), "@item{a}@item{b}@paginate");
    write_file(
        &root.join("content/blog.paginate.html"),
        "<section>@getenv(FAIL_BARRIER) page $[paginate.current]/$[paginate.total]</section>",
    );
    write_file(
        &root.join("content/other.paginate.html"),
        "<section>@getenv(OK_BARRIER) page $[paginate.current]/$[paginate.total]</section>",
    );
}

fn failing_provider(name: &str) -> HostResult {
    match name {
        "FAIL_BARRIER" => HostResult::Error("host exploded".to_string()),
        "OK_BARRIER" => HostResult::Found("ok".to_string()),
        _ => HostResult::NotFound,
    }
}

#[test]
fn standalone_env_host_failure_fails_render() {
    let mut engine = Engine::new();
    engine.set_environment_provider_result(failing_provider);

    let failed = engine.render(
        &nift::Source::text("@getenv(FAIL_BARRIER)"),
        &nift::Source::text("<main>@content</main>"),
        &Context::new(),
    );
    assert!(failed.is_err());
    assert!(failed.unwrap_err().message.contains("host exploded"));

    let unset = engine.render(
        &nift::Source::text("@getenv(MISSING)"),
        &nift::Source::text("<main>@content</main>"),
        &Context::new(),
    );
    assert!(unset.is_ok());
    assert_eq!(unset.unwrap().output, "<main></main>");
}

#[test]
fn paginated_env_host_failure_fails_render() {
    let root = temp_dir("paginated");
    two_page_project(&root);

    let mut engine = Engine::project(&root);
    assert!(engine.is_open());
    engine.set_environment_provider_result(failing_provider);

    // blog's paginate template reads @getenv(FAIL_BARRIER): the render fails.
    let failed = engine.render_page("blog", &Context::new());
    assert!(failed.is_err());
    assert!(failed.unwrap_err().message.contains("host exploded"));

    // other's paginate template reads @getenv(OK_BARRIER): it renders.
    let ok = engine.render_page("other", &Context::new()).expect("render");
    assert!(ok.output.contains("ok"), "{}", ok.output);
    assert_eq!(ok.pagination.len(), 1);

    // NotFound provider: ordinary unset, paginated render succeeds.
    let mut unset_engine = Engine::project(&root);
    unset_engine.set_environment_provider_result(|_| HostResult::NotFound);
    let unset = unset_engine.render_page("blog", &Context::new()).expect("render");
    assert_eq!(unset.pagination.len(), 2);
}

#[test]
fn concurrent_paginated_attribution() {
    let root = temp_dir("concurrent");
    two_page_project(&root);

    let mut engine = Engine::project(&root);
    assert!(engine.is_open());
    engine.set_environment_provider_result(failing_provider);
    let engine = std::sync::Arc::new(engine);

    for _ in 0..2 {
        let engine_a = std::sync::Arc::clone(&engine);
        let engine_b = std::sync::Arc::clone(&engine);
        let handle_a = thread::spawn(move || engine_a.render_page("blog", &Context::new()));
        let handle_b = thread::spawn(move || engine_b.render_page("other", &Context::new()).expect("render B"));
        let a: Result<RenderResult, nift::RenderError> = handle_a.join().expect("join A");
        let b: RenderResult = handle_b.join().expect("join B");
        assert!(a.is_err(), "blog must fail");
        assert!(a.unwrap_err().message.contains("host exploded"));
        assert!(b.output.contains("ok"), "{}", b.output);
    }
}
