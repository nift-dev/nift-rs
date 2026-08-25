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

/// Read `<case>/expected.json` and inject its declared `env` object into the
/// process environment (flat `{"KEY":"value"}` shape; matches the C++
/// conformance driver's `extra_env` handling).
fn inject_case_env(case_dir: &Path) {
    let Ok(text) = std::fs::read_to_string(case_dir.join("expected.json")) else {
        return;
    };
    let Some(env_pos) = text.find("\"env\"") else {
        return;
    };
    let rest = &text[env_pos..];
    let Some(open) = rest.find('{') else {
        return;
    };
    let close = rest[open + 1..]
        .find('}')
        .map(|i| open + 1 + i)
        .unwrap_or(rest.len());
    let body = &rest[open + 1..close];
    for pair in body.split(',') {
        let pair = pair.trim();
        let Some(colon) = pair.find(':') else {
            continue;
        };
        let key = unquote(pair[..colon].trim());
        let value = unquote(pair[colon + 1..].trim());
        if !key.is_empty() {
            std::env::set_var(key, value);
        }
    }
}

fn unquote(fragment: &str) -> String {
    let fragment = fragment.trim();
    let fragment = fragment.strip_prefix('"').unwrap_or(fragment);
    let fragment = fragment.strip_suffix('"').unwrap_or(fragment);
    fragment.to_string()
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
        // The declared env is a flat object of strings, so a minimal scanner
        // suffices (avoids pulling a JSON dependency into the test harness).
        inject_case_env(&corpus_case(case_id));
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
