//! NR4 repair differential tests: @json block scoping, schema-path
//! interpolation, the contract host capability, expanded schema keywords, and
//! @input error-cleanup.

use nift::bindings::Bindings;
use nift::context::Context;
use nift::error::{ErrorKind, RenderError};
use nift::host::{RenderHost, RenderIdentity};
use nift::value::Value;
use nift::{render, FilesystemHost, Source};
use std::borrow::Cow;
use std::path::{Path, PathBuf};

fn write_file(path: &std::path::Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn temp_root() -> PathBuf {
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("nift-nr4r-{}-{id}", std::process::id()))
}

fn write_project(root: &std::path::Path) {
    write_file(&root.join("content/t.html"), "@content");
    write_file(
        &root.join("data.json"),
        r#"{"value":1,"obj":{"b":2},"items":[{"name":"a"},{"name":"b"}]}"#,
    );
    write_file(&root.join("extra.json"), r#"{"value":99}"#);
    write_file(
        &root.join("schemas/max.schema.json"),
        r#"{"type":"object","properties":{"n":{"type":"number","maximum":10}}}"#,
    );
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
fn json_binding_scoping() {
    let root = temp_root();
    write_project(&root);
    // Top-level @json persists for the render.
    let result = fs_render(&root, "@json(\"data.json\", d)$[d.value]");
    assert_eq!(result.output, "1");
    // @json inside @if: visible inside, absent after the block.
    let result = fs_render(
        &root,
        "@if(true){@json(\"extra.json\", e)$[e.value]}$[e.value]",
    );
    assert_eq!(result.output, "99$[e.value]");
    // @json inside @for: fresh per iteration, absent after the loop; the same
    // binding name is allowed in each iteration.
    let result = fs_render(
        &root,
        "@json(\"data.json\", d)@for(x : d.items){@json(\"extra.json\", e)$[e.value]}$[e.value]",
    );
    assert!(result.output.contains("99"));
    assert!(result.output.ends_with("$[e.value]"));
    // Failure inside a scoped block: the scope is cleaned before the error
    // propagates (whole render fails; no stale binding survives).
    let error = fs_render_err(&root, "@if(true){@json(\"data.json\", d)$[d.obj.missing]}");
    assert!(error.message.contains("has no member"));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn json_schema_path_interpolation() {
    let root = temp_root();
    write_project(&root);
    let mut defaults = Bindings::new();
    defaults
        .set("schemafile", Value::string("schemas/max.schema.json"))
        .unwrap();
    let context = Context::new();
    let host = FilesystemHost::new(&defaults, &context, &root);
    let identity = RenderIdentity::new().name("t").title("T");
    // Schema path interpolated from a binding.
    let result = render(
        &host,
        &identity,
        &Source::text("@json(\"data.json\", d, \"$[schemafile]\")@for(x : d.items){$[x.name]}|"),
        None,
    )
    .expect("render should succeed");
    assert_eq!(result.output, "ab|");
    std::fs::remove_dir_all(&root).ok();
}

// A minimal host exposing a synthetic contract namespace.
struct ContractHost<'a> {
    defaults: &'a Bindings,
    context: &'a Context,
    root: PathBuf,
    contracts: std::collections::HashMap<String, (String, Value)>,
}

impl<'a> ContractHost<'a> {
    fn new(defaults: &'a Bindings, context: &'a Context, root: PathBuf) -> Self {
        let mut contracts = std::collections::HashMap::new();
        contracts.insert(
            "site".to_string(),
            (
                "site.json".to_string(),
                Value::Object(
                    [("name".to_string(), Value::string("Nift"))]
                        .into_iter()
                        .collect(),
                ),
            ),
        );
        Self {
            defaults,
            context,
            root,
            contracts,
        }
    }
}

impl<'a> RenderHost for ContractHost<'a> {
    fn binding(&self, name: &str) -> Option<&Value> {
        nift::resolve(self.defaults, self.context, name)
    }
    fn root(&self) -> &Path {
        &self.root
    }
    fn relative(&self, path: &Path) -> String {
        path.to_string_lossy().replace('\\', "/")
    }
    fn content_path(&self, _: &RenderIdentity) -> PathBuf {
        self.root.clone()
    }
    fn output_path(&self, _: &RenderIdentity) -> PathBuf {
        self.root.clone()
    }
    fn read_source(&self, _path: &Path) -> Result<Cow<'_, str>, RenderError> {
        Err(RenderError::new(ErrorKind::MissingSource, "no source"))
    }
    fn source_exists(&self, path: &Path) -> bool {
        self.read_json(path).is_ok()
    }
    fn source_readable(&self, path: &Path) -> bool {
        self.read_json(path).is_ok()
    }
    fn is_contract_name(&self, name: &str) -> bool {
        self.contracts.contains_key(name)
    }
    fn contract_source(&self, name: &str) -> Option<&str> {
        self.contracts.get(name).map(|(source, _)| source.as_str())
    }
    fn read_json(&self, path: &Path) -> Result<Value, RenderError> {
        let key = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        for (source_path, value) in self.contracts.values() {
            if source_path == key {
                return Ok(value.clone());
            }
        }
        Err(RenderError::new(ErrorKind::MissingSource, "no contract"))
    }
}

#[test]
fn contract_host_capability() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = ContractHost::new(&defaults, &context, PathBuf::from("/site"));
    let identity = RenderIdentity::new().name("t").title("T");
    // Contract member resolves.
    let result = render(&host, &identity, &Source::text("$[site.name]"), None)
        .expect("render should succeed");
    assert_eq!(result.output, "Nift");
    // Ordinary host binding wins over the contract namespace.
    let mut defaults = Bindings::new();
    defaults
        .set(
            "site",
            Value::Object(
                [("name".to_string(), Value::string("HostWins"))]
                    .into_iter()
                    .collect(),
            ),
        )
        .unwrap();
    let context = Context::new();
    let host = ContractHost::new(&defaults, &context, PathBuf::from("/site"));
    let result = render(&host, &identity, &Source::text("$[site.name]"), None)
        .expect("render should succeed");
    assert_eq!(result.output, "HostWins");
    // Unknown namespace behaves normally (literal fallback).
    let result = render(&host, &identity, &Source::text("$[unknown.x]"), None)
        .expect("render should succeed");
    assert_eq!(result.output, "$[unknown.x]");
}

#[test]
fn schema_keyword_coverage() {
    let root = temp_root();
    write_project(&root);
    // maximum violation (the reference keyword the reviewer flagged).
    write_file(&root.join("bad.json"), r#"{"n":11}"#);
    let error = fs_render_err(&root, "@json(\"bad.json\", d, \"schemas/max.schema.json\")");
    assert!(error.message.contains("does not satisfy schema"));
    assert!(error.message.contains("maximum"));
    // Valid value under maximum.
    write_file(&root.join("ok.json"), r#"{"n":5}"#);
    let result = fs_render(
        &root,
        "@json(\"ok.json\", d, \"schemas/max.schema.json\")$[d.n]",
    );
    assert_eq!(result.output, "5");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn input_cleanup_on_error() {
    let root = temp_root();
    write_project(&root);
    write_file(
        &root.join("templates/broken.html"),
        "@json(\"data.json\", d)$[d.obj.missing]",
    );
    // The nested parse fails; @input pops its stack entry before propagating.
    let error = fs_render_err(&root, "x@input(\"templates/broken.html\")y");
    assert!(error.message.contains("has no member"));
    std::fs::remove_dir_all(&root).ok();
}
