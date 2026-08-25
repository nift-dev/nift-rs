//! NR8 gate: project-aware Engine implementation conformance.
//!
//! The gate is the full canonical corpus passing through the Rust project-aware
//! Engine: every parity page renders byte-identically to its golden
//! output/dependencies/requirements, and every reject case fails with the
//! canonical semantic class. Plus the project-aware Engine contract
//! (unknown-page/open failures, Context overlays, host-vs-contract precedence,
//! title override, environment provider, concurrent renders).

use nift::context::Context;
use nift::error::ErrorKind;
use nift::Engine;
use std::path::{Path, PathBuf};

const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/cases");

fn corpus_case(id: &str) -> PathBuf {
    PathBuf::from(CORPUS).join(id)
}

/// Compare a render's lines against a golden file, ignoring the golden's
/// missing trailing newline (the golden writer and the corpus driver both treat
/// these as line sets).
fn lines_equal(path: &Path, lines: &[String]) -> bool {
    let golden = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => return false,
    };
    let golden_lines: Vec<&str> = golden.lines().collect();
    golden_lines == lines.iter().map(|s| s.as_str()).collect::<Vec<_>>()
}

fn sorted(entries: &std::collections::BTreeSet<String>) -> Vec<String> {
    entries.iter().cloned().collect()
}

/// RAII guard for process-global environment mutation in corpus tests.
///
/// Saves the previous value of each injected key, sets the declared values,
/// and restores the exact prior state on Drop (a previously-absent variable is
/// removed again). Restoration runs even if a render assertion panics, so
/// corpus cases cannot leak environment state or become order-dependent.
struct CaseEnvGuard {
    saved: Vec<(String, Option<String>)>,
}

impl CaseEnvGuard {
    /// Parse `<case>/expected.json` with serde_json (the file is real JSON;
    /// values may contain commas, colons, escapes, etc.) and inject its
    /// declared `env` object into the process environment.
    fn from_expected_json(case_dir: &Path) -> Self {
        let mut guard = Self { saved: Vec::new() };
        let Ok(text) = std::fs::read_to_string(case_dir.join("expected.json")) else {
            return guard;
        };
        let Ok(doc): Result<serde_json::Value, _> = serde_json::from_str(&text) else {
            return guard;
        };
        let Some(env) = doc.get("env").and_then(|v| v.as_object()) else {
            return guard;
        };
        for (key, value) in env {
            if let Some(value) = value.as_str() {
                guard.set(key, value);
            }
        }
        guard
    }

    fn set(&mut self, key: &str, value: &str) {
        self.saved
            .push((key.to_string(), std::env::var(key).ok()));
        std::env::set_var(key, value);
    }
}

impl Drop for CaseEnvGuard {
    fn drop(&mut self) {
        for (key, previous) in self.saved.drain(..).rev() {
            match previous {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn page_base(name: &str) -> String {
    if name == "/" {
        "ROOT".to_string()
    } else {
        name.replace('/', "_")
    }
}

#[test]
fn corpus_parity_pages_match_goldens() {
    let cases: &[(&str, &[&str])] = &[
        ("comprehensive", &["/", "about", "404", "blog/"]),
        ("schema", &["/"]),
        ("getenv", &["/"]),
    ];
    for (case_id, pages) in cases {
        // Mirror the C++ conformance driver: each case's expected.json may
        // declare an injected environment (e.g. the getenv case's
        // PA_CONFORMANCE_ENV), which must be present for @getenv renders.
        // The guard injects via serde_json and restores the prior process
        // environment when the case completes (even on panic).
        let _env = CaseEnvGuard::from_expected_json(&corpus_case(case_id));
        let project = corpus_case(case_id).join("project");
        let engine = Engine::open(&project)
            .unwrap_or_else(|e| panic!("{case_id}: open failed: {:?}", e.message));
        for &page in *pages {
            let context = Context::new();
            let result = engine
                .render_page(page, &context)
                .unwrap_or_else(|e| panic!("{case_id}/{page}: render failed: {}", e.message));
            let base = page_base(page);
            let expected_dir = corpus_case(case_id).join("expected");
            assert!(
                std::fs::read_to_string(expected_dir.join(format!("{base}.out"))).unwrap()
                    == result.output,
                "{case_id}/{page}: output does not match the golden"
            );
            assert!(
                lines_equal(
                    &expected_dir.join(format!("{base}.deps")),
                    &sorted(&result.dependencies)
                ),
                "{case_id}/{page}: dependencies do not match the golden"
            );
            assert!(
                lines_equal(
                    &expected_dir.join(format!("{base}.reqs")),
                    &sorted(&result.requirements)
                ),
                "{case_id}/{page}: requirements do not match the golden"
            );
        }
    }
}

#[test]
fn corpus_project_state_rejects() {
    for (case_id, class) in [
        ("malformed-config", "invalid-config-json"),
        ("bad-config", "unknown-config-key"),
        ("malformed-tracking", "invalid-tracking-json"),
        ("bad-tracking", "duplicate-tracked-name"),
    ] {
        let error = Engine::open(corpus_case(case_id).join("project"))
            .err()
            .unwrap_or_else(|| panic!("{case_id}: expected rejection"));
        assert_eq!(error.kind.corpus_class(), class, "{case_id}");
    }
}

#[test]
fn corpus_runtime_rejects() {
    // Missing source: config/tracking validate, the page render fails with the
    // missing-source class ("content file").
    let engine = Engine::open(corpus_case("missing-source").join("project")).unwrap();
    let error = engine
        .render_page("gone", &Context::new())
        .err()
        .unwrap_or_else(|| panic!("missing-source: expected render failure"));
    assert_eq!(error.kind, ErrorKind::MissingSource);
    assert!(
        error.message.contains("content file"),
        "missing-source: wrong class message: {}",
        error.message
    );

    // @pathto escape: the render fails with the project-root-escape class.
    let engine = Engine::open(corpus_case("path-escape").join("project")).unwrap();
    let error = engine
        .render_page("/", &Context::new())
        .err()
        .unwrap_or_else(|| panic!("path-escape: expected render failure"));
    assert!(
        error
            .message
            .contains("path must stay inside the Nift project"),
        "path-escape: wrong class message: {}",
        error.message
    );
}

// ---------------------------------------------------------------------------
// Project-aware Engine contract
// ---------------------------------------------------------------------------

#[test]
fn open_is_open_and_unknown_page() {
    let project = corpus_case("comprehensive").join("project");
    let engine = Engine::open(&project).unwrap();
    assert!(engine.is_open());

    // Unknown page name is a controlled error.
    let error = engine
        .render_page("nope", &Context::new())
        .expect_err("unknown page must fail");
    assert_eq!(error.kind, ErrorKind::UnknownPage);
    assert!(
        error.message.contains("unknown page name 'nope'"),
        "{}",
        error.message
    );
}

#[test]
fn standalone_render_without_project_is_controlled() {
    // A default Engine has no project: render_page is a controlled error.
    let engine = Engine::new();
    assert!(!engine.is_open());
    let error = engine
        .render_page("about", &Context::new())
        .expect_err("not-open render must fail");
    assert_eq!(error.message, "not a Nift project");
}

#[test]
fn context_overlay_and_title_override() {
    let project = corpus_case("comprehensive").join("project");
    let engine = Engine::open(&project).unwrap();

    // Context overlay wins over the tracked title for the root page.
    let mut context = Context::new();
    context.set_title("Custom Title");
    let result = engine.render_page("/", &context).unwrap();
    assert!(
        result.output.contains("<title>Custom Title</title>"),
        "{}",
        result.output
    );
    assert!(!result.output.contains("<title>Home</title>"));

    // The page-name argument is authoritative: a Context page name is ignored.
    let mut misleading = Context::new();
    misleading.set_page_name("about");
    let result = engine.render_page("/", &misleading).unwrap();
    assert!(result.output.contains("<h1>Home</h1>"), "{}", result.output);
}

#[test]
fn host_binding_wins_over_contract() {
    let project = corpus_case("comprehensive").join("project");
    let mut engine = Engine::open(&project).unwrap();

    // Engine default "site" (host binding) resolves before the contract.
    engine
        .set(
            "site",
            nift::Value::Object(
                [("name".to_string(), nift::Value::string("HostWins"))]
                    .into_iter()
                    .collect(),
            ),
        )
        .unwrap();

    let result = engine.render_page("about", &Context::new()).unwrap();
    assert!(result.output.contains("site=HostWins"), "{}", result.output);

    // Without the host binding the contract supplies site=Nift.
    let plain = Engine::open(&project).unwrap();
    let result = plain.render_page("about", &Context::new()).unwrap();
    assert!(result.output.contains("site=Nift"), "{}", result.output);
}

#[test]
fn environment_provider_is_consulted() {
    let project = corpus_case("getenv").join("project");
    let mut engine = Engine::open(&project).unwrap();
    engine.set_environment_provider(|name: &str| -> Option<String> {
        if name == "PA_CONFORMANCE_ENV" {
            Some("provider-value".to_string())
        } else {
            None
        }
    });
    let result = engine.render_page("/", &Context::new()).unwrap();
    assert!(
        result.output.contains("env=provider-value"),
        "{}",
        result.output
    );
}

#[test]
fn concurrent_project_renders() {
    let project = corpus_case("comprehensive").join("project");
    let engine = std::sync::Arc::new(Engine::open(&project).unwrap());
    let mut workers = Vec::new();
    for _ in 0..8 {
        let engine = std::sync::Arc::clone(&engine);
        workers.push(std::thread::spawn(move || {
            for _ in 0..20 {
                let result = engine.render_page("about", &Context::new()).unwrap();
                assert!(
                    result.output.contains("<h1>About</h1>"),
                    "{}",
                    result.output
                );
                let result = engine.render_page("/", &Context::new()).unwrap();
                assert!(result.output.contains("<h1>Home</h1>"), "{}", result.output);
            }
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
}

#[test]
fn primary_pagination_output() {
    let project = corpus_case("comprehensive").join("project");
    let engine = Engine::open(&project).unwrap();
    // render_page("blog/") is the CLI primary output (public/blog/index.html).
    let result = engine.render_page("blog/", &Context::new()).unwrap();
    assert!(
        result.output.contains("class=\"page-1\""),
        "{}",
        result.output
    );
    assert!(result.output.contains("onetwo"), "{}", result.output);
    // The pagination template and item source are dependencies.
    assert!(result
        .dependencies
        .contains("content/blog/index.paginate.html"));
    assert!(result.dependencies.contains("data/items.json"));
}

#[test]
fn contract_dependencies_are_recorded() {
    // Resolving a contract records .nift/config.json and the contract source as
    // dependencies (reference value_of), matching the ROOT/about/blog goldens.
    let project = corpus_case("comprehensive").join("project");
    let engine = Engine::open(&project).unwrap();
    let result = engine.render_page("about", &Context::new()).unwrap();
    assert!(result.dependencies.contains(".nift/config.json"));
    assert!(result.dependencies.contains("content/site.json"));
    assert!(result.dependencies.contains("content/about.html"));
    assert!(result.dependencies.contains("templates/page.html"));

    // A page that never resolves a contract carries neither dependency.
    let result = engine.render_page("404", &Context::new()).unwrap();
    assert!(!result.dependencies.contains(".nift/config.json"));
    assert!(!result.dependencies.contains("content/site.json"));
}

#[test]
fn case_env_guard_parses_full_json_values_and_restores() {
    // 1. An env value containing a comma, a colon and an escaped quote must be
    //    read back exactly (the manual scanner this replaces would split on
    //    the comma and colon and mis-handle the escape).
    let dir = std::env::temp_dir().join(format!("nr8-env-guard-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("expected.json"),
        r#"{"env":{"WEIRD":"a,b: \"c\""},"pages":{}}"#,
    )
    .unwrap();

    std::env::remove_var("WEIRD");
    {
        let guard = CaseEnvGuard::from_expected_json(&dir);
        assert_eq!(
            std::env::var("WEIRD").unwrap_or_default(),
            "a,b: \"c\"",
            "comma/colon/escaped-quote env value must be exact"
        );
        drop(guard);
    }
    assert!(
        std::env::var("WEIRD").is_err(),
        "previously-absent variable must be absent again after the guard"
    );

    // 2. A pre-existing variable is restored to its prior value.
    std::env::set_var("WEIRD", "original");
    {
        let guard = CaseEnvGuard::from_expected_json(&dir);
        assert_eq!(std::env::var("WEIRD").unwrap_or_default(), "a,b: \"c\"");
        drop(guard);
    }
    assert_eq!(
        std::env::var("WEIRD").unwrap_or_default(),
        "original",
        "pre-existing value must be restored after the guard"
    );
    std::env::remove_var("WEIRD");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pagination_renders_complete_page_set() {
    let dir = std::env::temp_dir().join(format!("nift-pagination-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for d in [".nift", "content", "templates", "public"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    std::fs::write(
        dir.join(".nift/config.json"),
        r#"{"config":{"content-dir":"content/","output-dir":"public/","default-template":"templates/template.html","build-threads":-1,"incremental-mode":"modified"}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join(".nift/tracked.json"),
        r#"{"tracked":[{"name":"blog","title":"Blog","template":"templates/template.html","paginate":{"items-per-page":1}}]}"#,
    )
    .unwrap();
    std::fs::write(dir.join("templates/template.html"), "<main>$[title]</main>\n@content\n").unwrap();
    std::fs::write(
        dir.join("content/blog.html"),
        "@item{one}@item{two}@item{three}@paginate\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("content/blog.paginate.html"),
        "<section>page $[paginate.current]/$[paginate.total]</section>\n",
    )
    .unwrap();

    let engine = Engine::open(&dir).unwrap();
    let result = engine.render_page("blog", &Context::new()).unwrap();

    // output = page 1 (primary).
    assert_eq!(result.output, "<main>Blog</main>\n<section>page 1/3</section>\n\n");
    // complete pagination: pages 2..N ascending.
    let pages: Vec<(usize, &str)> = result
        .pagination
        .iter()
        .map(|p| (p.page, p.output.as_str()))
        .collect();
    assert_eq!(
        pages,
        vec![
            (2, "<main>Blog</main>\n<section>page 2/3</section>\n\n"),
            (3, "<main>Blog</main>\n<section>page 3/3</section>\n\n"),
        ]
    );
    // pagination template is a dependency.
    assert!(result.dependencies.contains("content/blog.paginate.html"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn non_paginated_render_has_empty_pagination() {
    let dir = std::env::temp_dir().join(format!("nift-nonpag-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for d in [".nift", "content", "templates", "public"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    std::fs::write(
        dir.join(".nift/config.json"),
        r#"{"config":{"content-dir":"content/","output-dir":"public/","default-template":"templates/template.html","build-threads":-1,"incremental-mode":"modified"}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join(".nift/tracked.json"),
        r#"{"tracked":[{"name":"/","title":"Home","template":"templates/template.html"}]}"#,
    )
    .unwrap();
    std::fs::write(dir.join("templates/template.html"), "<main>@content</main>\n").unwrap();
    std::fs::write(dir.join("content/index.html"), "<p>home</p>\n").unwrap();
    let engine = Engine::open(&dir).unwrap();
    let result = engine.render_page("/", &Context::new()).unwrap();
    assert_eq!(result.output, "<main><p>home</p></main>\n");
    assert!(result.pagination.is_empty(), "non-paginated render has empty pagination");
    let _ = std::fs::remove_dir_all(&dir);
}
