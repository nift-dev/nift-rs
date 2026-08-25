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

// --- Pagination separator host-error semantics -----------------------------

use nift::error::{ErrorKind, RenderError};
use nift::host::{RenderHost, RenderIdentity};
use nift::parser::render_tracked;
use nift::project::{PaginationConfig, ProjectState};
use nift::value::Value;
use std::borrow::Cow;

#[derive(Clone)]
enum SeparatorOutcome {
    Error(&'static str),
    NotFound,
    FoundEmpty,
}

struct SeparatorProbeHost<'a> {
    state: &'a ProjectState,
    outcome: SeparatorOutcome,
}

impl<'a> RenderHost for SeparatorProbeHost<'a> {
    fn binding(&self, _name: &str) -> Option<&Value> {
        None
    }
    fn root(&self) -> &std::path::Path {
        self.state.root()
    }
    fn relative(&self, path: &std::path::Path) -> String {
        self.state.relative(path)
    }
    fn content_path(&self, identity: &RenderIdentity) -> std::path::PathBuf {
        let config = self.state.config();
        match &identity.name {
            Some(name) => match self.state.find(name) {
                Some(info) => self.state.content_path(info),
                None => self
                    .state
                    .root()
                    .join(&config.content_dir)
                    .join(format!("{}{}", name, config.content_ext)),
            },
            None => self.state.root().join(&config.content_dir),
        }
    }
    fn output_path(&self, identity: &RenderIdentity) -> std::path::PathBuf {
        let config = self.state.config();
        match &identity.name {
            Some(name) => match self.state.find(name) {
                Some(info) => self.state.output_path(info),
                None => self
                    .state
                    .root()
                    .join(&config.output_dir)
                    .join(format!("{}{}", name, config.output_ext)),
            },
            None => self.state.root().join(&config.output_dir),
        }
    }
    fn source_exists(&self, path: &std::path::Path) -> bool {
        self.is_separator(path) || self.state.read_shared_source(path).is_some()
    }
    fn source_readable(&self, path: &std::path::Path) -> bool {
        self.is_separator(path) || self.state.read_shared_source(path).is_some()
    }
    fn read_source(&self, path: &std::path::Path) -> Result<Cow<'_, str>, RenderError> {
        let is_separator = self.is_separator(path);
        if is_separator {
            return match &self.outcome {
                SeparatorOutcome::Error(message) => {
                    Err(RenderError::new(ErrorKind::Render, message.to_string()))
                }
                SeparatorOutcome::FoundEmpty => Ok(Cow::Owned(String::new())),
                SeparatorOutcome::NotFound => Err(RenderError::new(
                    ErrorKind::MissingSource,
                    "separator missing",
                )),
            };
        }
        match self.state.read_shared_source(path) {
            Some(source) => Ok(Cow::Owned(source.to_string())),
            None => Err(RenderError::new(
                ErrorKind::MissingSource,
                format!("source file is not readable: {}", path.display()),
            )),
        }
    }
}

impl SeparatorProbeHost<'_> {
    fn is_separator(&self, path: &std::path::Path) -> bool {
        path.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.contains(".separator.html"))
            .unwrap_or(false)
    }
}

fn separator_project(root: &Path) -> ProjectState {
    write_file(
        &root.join(".nift/config.json"),
        r#"{"config":{"content-dir":"content/","output-dir":"public/","default-template":"templates/template.html","build-threads":1,"incremental-mode":"modified"}}"#,
    );
    write_file(
        &root.join(".nift/tracked.json"),
        r#"{"tracked":[{"name":"blog","title":"Blog","template":"templates/template.html","paginate":{"items-per-page":1}}]}"#,
    );
    write_file(&root.join("templates/template.html"), "<main>$[title]</main>\n@content");
    write_file(&root.join("content/blog.html"), "@item{one}@item{two}@paginate");
    write_file(
        &root.join("content/blog.paginate.html"),
        "<section>$[paginate.items]-$[paginate.current]/$[paginate.total]</section>",
    );
    write_file(&root.join("content/blog.separator.html"), "--sep--");
    ProjectState::open(root).expect("open project")
}

fn render_with_host(state: &ProjectState, outcome: SeparatorOutcome) -> Result<nift::result::RenderResult, nift::RenderError> {
    let info = state.find("blog").expect("blog tracked");
    let identity = RenderIdentity {
        name: Some(info.name.clone()),
        title: Some(info.title.clone()),
        template_path: if info.template_path.is_empty() {
            None
        } else {
            Some(info.template_path.clone())
        },
    };
    let paginate = PaginationConfig {
        items_per_page: info.paginate.as_ref().map(|p| p.items_per_page).unwrap_or(1),
        template_path: None,
        separator_path: None,
    };
    let host = SeparatorProbeHost { state, outcome };
    render_tracked(&host, &identity, Some(&paginate))
}

#[test]
fn pagination_separator_host_error_semantics() {
    let root = temp_dir("separator");
    let state = separator_project(&root);

    // Error -> render fails with the host diagnostic (Error != NotFound).
    let failed = render_with_host(&state, SeparatorOutcome::Error("separator backend failed"));
    let failed_message = failed.unwrap_err().message;
    assert!(
        failed_message.contains("separator backend failed"),
        "{failed_message}"
    );

    // NotFound -> no separator; render succeeds without "--sep--".
    let no_sep = render_with_host(&state, SeparatorOutcome::NotFound).expect("render");
    assert!(!no_sep.output.contains("--sep--"));

    // Found empty -> valid empty separator; render succeeds.
    let empty_sep = render_with_host(&state, SeparatorOutcome::FoundEmpty).expect("render");
    assert!(!empty_sep.output.contains("--sep--"));
}

#[test]
fn pagination_error_selection_is_page_order() {
    let root = temp_dir("order");
    write_file(
        &root.join(".nift/config.json"),
        r#"{"config":{"content-dir":"content/","output-dir":"public/","default-template":"templates/template.html","build-threads":1,"incremental-mode":"modified"}}"#,
    );
    write_file(
        &root.join(".nift/tracked.json"),
        r#"{"tracked":[{"name":"blog","title":"Blog","template":"templates/template.html","paginate":{"items-per-page":1}}]}"#,
    );
    write_file(&root.join("templates/template.html"), "<main>$[title]</main>\n@content");
    write_file(&root.join("content/blog.html"), "@item{one}@item{two}@paginate");
    write_file(
        &root.join("content/blog.paginate.html"),
        "<section>@getenv(FAIL) $[paginate.current]</section>",
    );

    let mut engine = Engine::project(&root);
    assert!(engine.is_open());
    let calls = std::sync::atomic::AtomicUsize::new(0);
    engine.set_environment_provider_result(move |_| {
        let call = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if call == 0 {
            HostResult::Error("error-one".to_string())
        } else {
            HostResult::Error("error-two".to_string())
        }
    });
    let result = engine.render_page("blog", &Context::new());
    assert!(result.is_err());
    let message = result.unwrap_err().message;
    assert!(message.contains("error-one"), "{message}");
    assert!(!message.contains("error-two"), "{message}");
}
