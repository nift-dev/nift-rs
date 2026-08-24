//! NR5 differential tests: @pathto / @pathtofile / @pathtopage geometry,
//! containment, the 404 rule, index-page geometry and requirement recording.
//! Expectations captured from the nift-embed CLI.

use nift::bindings::Bindings;
use nift::context::Context;
use nift::error::{ErrorKind, RenderError};
use nift::host::{RenderHost, RenderIdentity};
use nift::value::Value;
use nift::{render, FilesystemHost, Source};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn write_file(path: &std::path::Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn temp_root() -> PathBuf {
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("nift-nr5-{}-{id}", std::process::id()))
}

fn write_project(root: &std::path::Path) {
    write_file(&root.join("content/t.html"), "@content");
    write_file(&root.join("public/app.js"), "console.log('x');");
    write_file(&root.join("public/about.html"), "<p>about</p>");
}

fn fs_context(root: &std::path::Path) -> Context {
    let mut context = Context::new();
    context.set_current_output(root.join("public/t.html"));
    context
}

fn fs_render(root: &std::path::Path, template: &str) -> nift::RenderResult {
    write_file(&root.join("content/t.html"), template);
    let defaults = Bindings::new();
    let context = fs_context(root);
    let host = FilesystemHost::new(&defaults, &context, root);
    let identity = RenderIdentity::new().name("t").title("T");
    render(&host, &identity, &Source::path("content/t.html"), None).expect("render should succeed")
}

fn fs_render_err(root: &std::path::Path, template: &str) -> nift::RenderError {
    write_file(&root.join("content/t.html"), template);
    let defaults = Bindings::new();
    let context = fs_context(root);
    let host = FilesystemHost::new(&defaults, &context, root);
    let identity = RenderIdentity::new().name("t").title("T");
    render(&host, &identity, &Source::path("content/t.html"), None).expect_err("render should fail")
}

#[test]
fn pathto_concrete_and_requirements() {
    let root = temp_root();
    write_project(&root);
    // Concrete project file, relative from the current output's directory.
    let result = fs_render(&root, "<a href=\"@pathto('public/app.js')\">app</a>");
    assert_eq!(result.output, "<a href=\"./app.js\">app</a>");
    // Requirement recorded as the target's project-relative path.
    assert!(result.requirements.contains("public/app.js"));
    // @pathtofile behaves identically.
    let result = fs_render(&root, "@pathtofile('public/app.js')");
    assert_eq!(result.output, "./app.js");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn pathto_containment_and_missing() {
    let root = temp_root();
    write_project(&root);
    let error = fs_render_err(&root, "@pathto('../escape')");
    assert!(error
        .message
        .contains("path must stay inside the Nift project"));
    let error = fs_render_err(&root, "@pathto('nope.html')");
    assert!(error
        .message
        .contains("neither a tracked name nor a file that exists"));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn pathto_404_root_absolute() {
    let root = temp_root();
    write_project(&root);
    write_file(
        &root.join("content/404.html"),
        "@pathto('public/about.html')",
    );
    let defaults = Bindings::new();
    let context = fs_context(&root);
    let host = FilesystemHost::new(&defaults, &context, &root);
    let identity = RenderIdentity::new().name("404").title("Not Found");
    let result = render(&host, &identity, &Source::path("content/404.html"), None)
        .expect("render should succeed");
    assert_eq!(result.output, "/about.html");
    std::fs::remove_dir_all(&root).ok();
}

// A synthetic host exposing tracked pages (index / trailing-slash geometry).
struct TrackedHost<'a> {
    defaults: &'a Bindings,
    context: &'a Context,
    root: PathBuf,
    sources: HashMap<PathBuf, String>,
    tracked: HashMap<String, (PathBuf, bool)>,
}

impl<'a> TrackedHost<'a> {
    fn new(defaults: &'a Bindings, context: &'a Context, root: PathBuf) -> Self {
        let mut tracked = HashMap::new();
        tracked.insert("about".to_string(), (root.join("public/about.html"), false));
        tracked.insert(
            "blog/".to_string(),
            (root.join("public/blog/index.html"), true),
        );
        tracked.insert("404".to_string(), (root.join("public/404.html"), false));
        Self {
            defaults,
            context,
            root,
            sources: HashMap::new(),
            tracked,
        }
    }

    fn with_source(mut self, path: PathBuf, contents: &str) -> Self {
        self.sources.insert(path, contents.to_string());
        self
    }
}

impl<'a> RenderHost for TrackedHost<'a> {
    fn binding(&self, name: &str) -> Option<&Value> {
        nift::resolve(self.defaults, self.context, name)
    }
    fn root(&self) -> &Path {
        &self.root
    }
    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"))
    }
    fn content_path(&self, identity: &RenderIdentity) -> PathBuf {
        self.root.join(format!(
            "content/{}.html",
            identity.name.clone().unwrap_or_default()
        ))
    }
    fn output_path(&self, identity: &RenderIdentity) -> PathBuf {
        let name = identity.name.clone().unwrap_or_default();
        if name == "/" || name.ends_with('/') {
            self.root.join(format!("public/{name}index.html"))
        } else {
            self.root.join(format!("public/{name}.html"))
        }
    }
    fn read_source(&self, path: &Path) -> Result<Cow<'_, str>, RenderError> {
        self.sources
            .get(path)
            .map(|s| Cow::Borrowed(s.as_str()))
            .ok_or_else(|| RenderError::new(ErrorKind::MissingSource, "no source"))
    }
    fn source_exists(&self, path: &Path) -> bool {
        self.sources.contains_key(path)
    }
    fn source_readable(&self, path: &Path) -> bool {
        self.sources.contains_key(path)
    }
    fn tracked_output_path(&self, name: &str) -> Option<(PathBuf, bool)> {
        self.tracked.get(name).cloned()
    }

    fn has_output_context(&self) -> bool {
        true
    }
}

#[test]
fn pathto_tracked_and_index_geometry() {
    let defaults = Bindings::new();
    let context = Context::new();
    let root = PathBuf::from("/site");
    let host = TrackedHost::new(&defaults, &context, root.clone())
        .with_source(root.join("public/about.html"), "<p>about</p>")
        .with_source(root.join("public/blog/index.html"), "<p>blog</p>")
        .with_source(root.join("public/404.html"), "<p>404</p>");
    let identity = RenderIdentity::new().name("about").title("About");
    // From about -> a tracked index page emits the directory path.
    let result = render(
        &host,
        &identity,
        &Source::text("<p>blog=@pathto('blog/')</p>"),
        None,
    )
    .expect("render should succeed");
    assert_eq!(result.output, "<p>blog=blog/</p>");
    // From about -> a tracked non-index page emits the file path.
    let result = render(&host, &identity, &Source::text("@pathto('404')"), None)
        .expect("render should succeed");
    assert_eq!(result.output, "./404.html");
    // Requirement recorded for the tracked target's output.
    let result = render(&host, &identity, &Source::text("@pathto('blog/')"), None)
        .expect("render should succeed");
    assert!(result.requirements.contains("public/blog/index.html"));
}

#[test]
fn pathtopage_requires_pagination() {
    let root = temp_root();
    write_project(&root);
    // A valid page number (1) reaches the pagination-context guard.
    let error = fs_render_err(&root, "@pathtopage(1)");
    assert!(error
        .message
        .contains("only available while rendering pagination"));
    // Out-of-range and malformed page numbers are validated before that.
    let error = fs_render_err(&root, "@pathtopage(2)");
    assert!(error
        .message
        .contains("resolved page must be between 1 and 1"));
    let error = fs_render_err(&root, "@pathtopage(0)");
    assert!(error.message.contains("positive integer"));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn pathto_requires_explicit_output_context() {
    let root = temp_root();
    write_project(&root);
    // No current_output -> @pathto is a controlled requires-context error.
    let defaults = Bindings::new();
    let context = Context::new();
    let host = FilesystemHost::new(&defaults, &context, &root);
    let identity = RenderIdentity::new().name("t").title("T");
    let error = render(
        &host,
        &identity,
        &Source::text("@pathto('public/app.js')"),
        None,
    )
    .expect_err("@pathto without an output context must fail");
    assert!(error.message.contains("requires a path context"));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn pathto_uses_explicit_current_output() {
    let root = temp_root();
    write_project(&root);
    // Explicit root-level current output.
    let defaults = Bindings::new();
    let mut context = Context::new();
    context.set_current_output(root.join("public/custom.html"));
    let host = FilesystemHost::new(&defaults, &context, &root);
    let identity = RenderIdentity::new().name("t").title("T");
    let result = render(
        &host,
        &identity,
        &Source::text("@pathto('public/app.js')"),
        None,
    )
    .expect("render should succeed");
    assert_eq!(result.output, "./app.js");
    // Nested current output -> ../../app.js.
    let mut context = Context::new();
    context.set_current_output(root.join("public/a/b/page.html"));
    let host = FilesystemHost::new(&defaults, &context, &root);
    let result = render(
        &host,
        &identity,
        &Source::text("@pathto('public/app.js')"),
        None,
    )
    .expect("render should succeed");
    assert_eq!(result.output, "../../app.js");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn current_output_differs_from_identity_and_wins() {
    let root = temp_root();
    write_project(&root);
    let defaults = Bindings::new();
    let mut context = Context::new();
    context.set_current_output(root.join("runtime/cache/foo/index.html"));
    let host = FilesystemHost::new(&defaults, &context, &root);
    let identity = RenderIdentity::new().name("about").title("About");
    let result = render(
        &host,
        &identity,
        &Source::text("@pathto('public/app.js')"),
        None,
    )
    .expect("render should succeed");
    assert_eq!(result.output, "../../../public/app.js");
    // Changing only current_output changes @pathto output.
    let mut context = Context::new();
    context.set_current_output(root.join("public/about.html"));
    let host = FilesystemHost::new(&defaults, &context, &root);
    let result = render(
        &host,
        &identity,
        &Source::text("@pathto('public/app.js')"),
        None,
    )
    .expect("render should succeed");
    assert_eq!(result.output, "./app.js");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn pathto_404_with_explicit_context_keeps_root_absolute() {
    let root = temp_root();
    write_project(&root);
    let defaults = Bindings::new();
    let context = fs_context(&root);
    let host = FilesystemHost::new(&defaults, &context, &root);
    let identity = RenderIdentity::new().name("404").title("Not Found");
    let result = render(
        &host,
        &identity,
        &Source::text("@pathto('public/app.js')"),
        None,
    )
    .expect("render should succeed");
    assert_eq!(result.output, "/app.js");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn tracked_geometry_works_without_context_current_output() {
    let defaults = Bindings::new();
    let context = Context::new(); // no current_output
    let root = PathBuf::from("/site");
    let host = TrackedHost::new(&defaults, &context, root.clone())
        .with_source(root.join("public/about.html"), "<p>about</p>")
        .with_source(root.join("public/blog/index.html"), "<p>blog</p>");
    let identity = RenderIdentity::new().name("about").title("About");
    // The tracked host's output geometry is its own authority.
    let result = render(&host, &identity, &Source::text("@pathto('blog/')"), None)
        .expect("render should succeed");
    assert_eq!(result.output, "blog/");
    std::fs::remove_dir_all(&root).ok();
}
