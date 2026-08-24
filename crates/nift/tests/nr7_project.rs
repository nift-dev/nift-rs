//! NR7 gate: ProjectRead + immutable ProjectState parity.
//!
//! The gate is: the ported invalid-state/parity corpus matches the canonical
//! acceptance/rejection classes. This test runs against the frozen canonical
//! corpus (corpus/cases) directly, plus an executable port of the C++
//! `project_state_parity` invalid-case table, and the open-sequence / zero-write
//! / concurrent-read properties the reference ProjectState guarantees.

use nift::json::parse_json;
use nift::{mapped_name, ProjectErrorKind, ProjectState};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const CORPUS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/cases");

fn corpus_case(id: &str) -> PathBuf {
    PathBuf::from(CORPUS).join(id)
}

fn manifest_value(id: &str) -> nift::Value {
    let text = std::fs::read_to_string(corpus_case(id).join("expected.json"))
        .unwrap_or_else(|_| panic!("{id}: no expected.json"));
    parse_json(&text).unwrap_or_else(|e| panic!("{id}: bad expected.json: {e}"))
}

fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "nift-nr7-{}-{}",
        std::process::id(),
        name.replace('/', "_")
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn write_file(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn write_valid_project(root: &Path) {
    write_file(
        &root.join(".nift/config.json"),
        r#"{"config":{
          "content-dir":"content/","content-ext":".html",
          "output-dir":"public/","output-ext":".html",
          "default-template":"templates/template.html",
          "incremental-mode":"modified","build-threads":4,
          "contracts":{"site":"content/site.json"},
          "minify-exts":[".css",".js"]}}"#,
    );
    write_file(
        &root.join(".nift/tracked.json"),
        r#"{"tracked":[
          {"name":"/","title":"Home"},
          {"name":"about","title":"About","template":"templates/page.html"},
          {"name":"blog/","title":"Blog","paginate":{"items-per-page":5,"template":"templates/blog.html","separator":"templates/sep.html"}},
          {"name":"feed","title":"Feed","content-ext":".xml","output-ext":".xml"},
          {"name":"scripts","title":"Scripts","content-ext":".js","output-ext":".js","minify":true}]}"#,
    );
    for content in [
        "content/index.html",
        "content/about.html",
        "content/blog/index.html",
        "content/feed.xml",
        "content/scripts.js",
    ] {
        write_file(&root.join(content), "<p>content</p>");
    }
    write_file(
        &root.join("content/site.json"),
        r#"{"site":{"name":"Nift"}}"#,
    );
    for tpl in [
        "templates/template.html",
        "templates/page.html",
        "templates/blog.html",
        "templates/sep.html",
    ] {
        write_file(&root.join(tpl), "<html>@content</html>");
    }
}

// ---------------------------------------------------------------------------
// Canonical corpus gate
// ---------------------------------------------------------------------------

#[test]
fn corpus_open_acceptance() {
    for id in [
        "comprehensive",
        "schema",
        "getenv",
        // Runtime invalidity (missing source, @pathto escape) is a render-time
        // concern (NR8); the project state itself opens and validates.
        "missing-source",
        "path-escape",
    ] {
        let project = corpus_case(id).join("project");
        let result = ProjectState::open(&project);
        assert!(
            result.is_ok(),
            "{id}: expected a valid snapshot but open failed: {:?}",
            result.err()
        );
    }
}

#[test]
fn corpus_open_rejection_classes() {
    for id in [
        "malformed-config",
        "bad-config",
        "malformed-tracking",
        "bad-tracking",
    ] {
        let expected = manifest_value(id)
            .get("expect")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("{id}: expected.json has no 'expect' class"))
            .to_string();
        let project = corpus_case(id).join("project");
        let error = ProjectState::open(&project)
            .err()
            .unwrap_or_else(|| panic!("{id}: expected rejection but open succeeded"));
        assert_eq!(
            error.kind.corpus_class(),
            expected,
            "{id}: rejected for the wrong semantic class"
        );
    }
}

#[test]
fn corpus_comprehensive_geometry() {
    let project = corpus_case("comprehensive").join("project");
    let state = ProjectState::open(&project).unwrap();

    assert_eq!(state.tracked().len(), 4);
    for name in ["/", "about", "404", "blog/"] {
        assert!(state.find(name).is_some(), "missing tracked '{name}'");
    }
    assert!(state.find("nope").is_none());

    let root = state.root().to_path_buf();
    let home = state.find("/").unwrap();
    assert_eq!(state.content_path(home), root.join("content/index.html"));
    assert_eq!(state.output_path(home), root.join("public/index.html"));
    let blog = state.find("blog/").unwrap();
    assert_eq!(
        state.content_path(blog),
        root.join("content/blog/index.html")
    );
    assert_eq!(state.output_path(blog), root.join("public/blog/index.html"));
    assert_eq!(
        state.pagination_output_path(blog, 1),
        root.join("public/blog/index.html")
    );
    assert_eq!(
        state.pagination_output_path(blog, 2),
        root.join("public/blog/2.html")
    );
    assert_eq!(mapped_name("/"), "index");
    assert_eq!(mapped_name("blog/"), "blog/index");
    assert_eq!(mapped_name("about"), "about");

    assert_eq!(state.config().contracts.len(), 1);
    assert_eq!(
        state.config().contracts.get("site").map(|s| s.as_str()),
        Some("content/site.json")
    );

    let site = state
        .read_shared_json(&root.join("content/site.json"))
        .unwrap();
    assert_eq!(
        site.get("name").and_then(|name| name.as_str()),
        Some("Nift")
    );
}

// ---------------------------------------------------------------------------
// Ported C++ project_state_parity invalid-case table
// ---------------------------------------------------------------------------

const KCONFIG: &str = r#"{"config":{
    "content-dir":"content/","content-ext":".html",
    "output-dir":"public/","output-ext":".html",
    "default-template":"templates/template.html",
    "incremental-mode":"modified","build-threads":4,
    "contracts":{"site":"content/site.json"},
    "minify-exts":[".css",".js"]}}"#;

const KTRACKED: &str = r#"{"tracked":[
    {"name":"/","title":"Home"},
    {"name":"about","title":"About","template":"templates/page.html"},
    {"name":"blog/","title":"Blog","paginate":{"items-per-page":5,"template":"templates/blog.html","separator":"templates/sep.html"}},
    {"name":"feed","title":"Feed","content-ext":".xml","output-ext":".xml"},
    {"name":"scripts","title":"Scripts","content-ext":".js","output-ext":".js","minify":true}]}"#;

struct InvalidCase {
    name: &'static str,
    config: Option<&'static str>,
    tracked: Option<&'static str>,
}

#[test]
fn invalid_state_parity() {
    let cases = [
        // Config-stage rejections (tracked.json is valid/absent; config fails).
        InvalidCase {
            name: "unknown-key",
            config: Some(r#"{"config":{"bogus":1}}"#),
            tracked: Some(KTRACKED),
        },
        InvalidCase {
            name: "non-string-field",
            config: Some(r#"{"config":{"content-dir":3}}"#),
            tracked: Some(KTRACKED),
        },
        InvalidCase {
            name: "empty-content-dir",
            config: Some(r#"{"config":{"content-dir":""}}"#),
            tracked: Some(KTRACKED),
        },
        InvalidCase {
            name: "bad-extension",
            config: Some(r#"{"config":{"content-ext":"html"}}"#),
            tracked: Some(KTRACKED),
        },
        InvalidCase {
            name: "bad-build-threads",
            config: Some(r#"{"config":{"build-threads":1.5}}"#),
            tracked: Some(KTRACKED),
        },
        InvalidCase {
            name: "bad-contract-name",
            config: Some(r#"{"config":{"contracts":{"9bad":"content/site.json"}}}"#),
            tracked: Some(KTRACKED),
        },
        InvalidCase {
            name: "reserved-contract",
            config: Some(r#"{"config":{"contracts":{"title":"content/site.json"}}}"#),
            tracked: Some(KTRACKED),
        },
        InvalidCase {
            name: "contract-outside-root",
            config: Some(r#"{"config":{"contracts":{"site":"../secret.json"}}}"#),
            tracked: Some(KTRACKED),
        },
        InvalidCase {
            name: "contract-non-string",
            config: Some(r#"{"config":{"contracts":{"site":5}}}"#),
            tracked: Some(KTRACKED),
        },
        InvalidCase {
            name: "minify-exts-not-array",
            config: Some(r#"{"config":{"minify-exts":"css"}}"#),
            tracked: Some(KTRACKED),
        },
        InvalidCase {
            name: "minify-exts-unsupported",
            config: Some(r#"{"config":{"minify-exts":[".ts"]}}"#),
            tracked: Some(KTRACKED),
        },
        InvalidCase {
            name: "bad-incremental-mode",
            config: Some(r#"{"config":{"incremental-mode":"wat"}}"#),
            tracked: Some(KTRACKED),
        },
        InvalidCase {
            name: "config-not-object",
            config: Some(r#"{"config":[]}"#),
            tracked: Some(KTRACKED),
        },
        InvalidCase {
            name: "no-config-member",
            config: Some(r#"{"settings":{}}"#),
            tracked: Some(KTRACKED),
        },
        InvalidCase {
            name: "missing-config",
            config: None,
            tracked: Some(KTRACKED),
        },
        // Tracking-stage rejections (config is valid; tracked.json fails).
        InvalidCase {
            name: "missing-tracked",
            config: Some(KCONFIG),
            tracked: None,
        },
        InvalidCase {
            name: "malformed-tracked",
            config: Some(KCONFIG),
            tracked: Some("{ not json"),
        },
        InvalidCase {
            name: "tracked-not-array",
            config: Some(KCONFIG),
            tracked: Some(r#"{"tracked":{"name":"x"}}"#),
        },
        InvalidCase {
            name: "entry-missing-name",
            config: Some(KCONFIG),
            tracked: Some(r#"{"tracked":[{"title":"X"}]}"#),
        },
        InvalidCase {
            name: "entry-non-string-title",
            config: Some(KCONFIG),
            tracked: Some(r#"{"tracked":[{"name":"a","title":3}]}"#),
        },
        InvalidCase {
            name: "bad-name-parent-component",
            config: Some(KCONFIG),
            tracked: Some(r#"{"tracked":[{"name":"../escape","title":"X"}]}"#),
        },
        InvalidCase {
            name: "bad-name-absolute",
            config: Some(KCONFIG),
            tracked: Some(r#"{"tracked":[{"name":"/abs","title":"X"}]}"#),
        },
        InvalidCase {
            name: "duplicate-name",
            config: Some(KCONFIG),
            tracked: Some(
                r#"{"tracked":[{"name":"about","title":"A"},{"name":"about","title":"B"}]}"#,
            ),
        },
        InvalidCase {
            name: "duplicate-path",
            config: Some(KCONFIG),
            tracked: Some(r#"{"tracked":[{"name":"/","title":"A"},{"name":"index","title":"B"}]}"#),
        },
        InvalidCase {
            name: "bad-extension-override",
            config: Some(KCONFIG),
            tracked: Some(r#"{"tracked":[{"name":"feed","title":"F","content-ext":"xml"}]}"#),
        },
        InvalidCase {
            name: "template-equals-content",
            config: Some(KCONFIG),
            tracked: Some(
                r#"{"tracked":[{"name":"about","title":"A","template":"content/about.html"}]}"#,
            ),
        },
        InvalidCase {
            name: "bad-paginate",
            config: Some(KCONFIG),
            tracked: Some(
                r#"{"tracked":[{"name":"blog/","title":"B","paginate":{"items-per-page":0}}]}"#,
            ),
        },
        InvalidCase {
            name: "paginate-non-int",
            config: Some(KCONFIG),
            tracked: Some(
                r#"{"tracked":[{"name":"blog/","title":"B","paginate":{"items-per-page":2.5}}]}"#,
            ),
        },
        InvalidCase {
            name: "minify-non-bool",
            config: Some(KCONFIG),
            tracked: Some(r#"{"tracked":[{"name":"scripts","title":"S","minify":"yes"}]}"#),
        },
    ];

    for test in cases {
        // A leading-slash tracked name is absolute only on POSIX; on Windows a
        // bare "/abs" has no drive prefix, so both implementations accept it
        // (see nr10_portability). Skip the platform-specific case there.
        if cfg!(windows) && test.name == "bad-name-absolute" {
            continue;
        }
        let root = fixture(&format!("invalid-{}", test.name));
        if let Some(config) = test.config {
            write_file(&root.join(".nift/config.json"), config);
        }
        if let Some(tracked) = test.tracked {
            write_file(&root.join(".nift/tracked.json"), tracked);
        }

        let error = match ProjectState::open(&root) {
            Ok(_) => panic!("{}: expected rejection but open succeeded", test.name),
            Err(error) => error,
        };
        assert!(
            !error.message.is_empty(),
            "{}: error message must be non-empty",
            test.name
        );
    }
}

#[test]
fn duplicate_literal_name_is_duplicate_class() {
    let root = fixture("invalid-duplicate-literal");
    write_file(&root.join(".nift/config.json"), KCONFIG);
    write_file(
        &root.join(".nift/tracked.json"),
        r#"{"tracked":[{"name":"about","title":"A"},{"name":"about","title":"B"}]}"#,
    );
    let error = ProjectState::open(&root).unwrap_err();
    assert_eq!(error.kind, ProjectErrorKind::DuplicateName);
    assert_eq!(error.kind.corpus_class(), "duplicate-tracked-name");
}

fn collision_fixture(name: &str, tracked: &str) -> ProjectErrorKind {
    let root = fixture(name);
    write_file(&root.join(".nift/config.json"), KCONFIG);
    write_file(&root.join(".nift/tracked.json"), tracked);
    let error = ProjectState::open(&root).unwrap_err();
    assert_eq!(error.kind.corpus_class(), "invalid-tracking-json");
    error.kind
}

#[test]
fn resolved_content_path_collision_is_not_duplicate_class() {
    // "/" and "index" both resolve to content/index.html and public/index.html:
    // distinct tracked names, same resolved paths -> ordinary invalid tracking,
    // NOT duplicate-tracked-name.
    let kind = collision_fixture(
        "collision-both",
        r#"{"tracked":[{"name":"/","title":"A"},{"name":"index","title":"B"}]}"#,
    );
    assert_eq!(kind, ProjectErrorKind::PathCollision);
    assert_ne!(kind, ProjectErrorKind::DuplicateName);
}

#[test]
fn resolved_content_path_collision_only() {
    // "a/" maps to a/index; "a/index" is its own mapped name. With different
    // output extensions the content paths collide while the outputs differ.
    let kind = collision_fixture(
        "collision-content",
        r#"{"tracked":[
            {"name":"a/","title":"A","output-ext":".html"},
            {"name":"a/index","title":"B","output-ext":".txt"}]}"#,
    );
    assert_eq!(kind, ProjectErrorKind::PathCollision);
    assert_ne!(kind, ProjectErrorKind::DuplicateName);
}

#[test]
fn resolved_output_path_collision_only() {
    // With different content extensions the content paths differ while both
    // resolve to public/a/index.html.
    let kind = collision_fixture(
        "collision-output",
        r#"{"tracked":[
            {"name":"a/","title":"A","content-ext":".html"},
            {"name":"a/index","title":"B","content-ext":".txt"}]}"#,
    );
    assert_eq!(kind, ProjectErrorKind::PathCollision);
    assert_ne!(kind, ProjectErrorKind::DuplicateName);
}

#[test]
fn valid_parity() {
    let root = fixture("valid");
    write_valid_project(&root);

    let state = ProjectState::open(&root).unwrap();
    assert_eq!(state.tracked().len(), 5);
    assert_eq!(state.config().build_threads, 4);
    assert_eq!(state.config().minify_exts.len(), 2);
    assert!(state.config().minify_exts.contains(".css"));
    assert!(state.config().minify_exts.contains(".js"));

    for name in ["/", "about", "blog/", "feed", "scripts"] {
        assert!(state.find(name).is_some(), "missing tracked '{name}'");
    }
    assert!(state.find("nope").is_none());

    let home = state.find("/").unwrap();
    assert_eq!(state.content_path(home), root.join("content/index.html"));
    assert_eq!(state.output_path(home), root.join("public/index.html"));
    let blog = state.find("blog/").unwrap();
    assert_eq!(
        state.content_path(blog),
        root.join("content/blog/index.html")
    );
    assert_eq!(
        state.pagination_output_path(blog, 3),
        root.join("public/blog/3.html")
    );
    let feed = state.find("feed").unwrap();
    assert_eq!(state.output_path(feed), root.join("public/feed.xml"));
    let about = state.find("about").unwrap();
    assert_eq!(
        state.pagination_output_path(about, 3),
        root.join("public/about-3.html")
    );

    assert_eq!(
        state.relative(&root.join("content/about.html")),
        "content/about.html"
    );
    assert_eq!(
        state.relative(&root.join("public/feed.xml")),
        "public/feed.xml"
    );
}

// ---------------------------------------------------------------------------
// Open sequences, zero writes, concurrency
// ---------------------------------------------------------------------------

#[test]
fn open_sequences_are_transactional() {
    // Successful open then a failed open: the failure discards nothing (each
    // open constructs a fresh snapshot; a failure simply returns an error).
    let valid = fixture("seq-valid");
    write_valid_project(&valid);
    let opened = ProjectState::open(&valid).unwrap();
    assert!(opened.find("about").is_some());

    let broken = fixture("seq-broken");
    write_file(&broken.join(".nift/config.json"), KCONFIG);
    write_file(&broken.join(".nift/tracked.json"), "{ not json");
    let error = ProjectState::open(&broken).unwrap_err();
    assert_eq!(error.kind, ProjectErrorKind::TrackingJson);
    // The previously opened snapshot is unaffected.
    assert!(opened.find("about").is_some());
    assert_eq!(opened.tracked().len(), 5);

    // Failed open then successful open: the next open recovers to a complete
    // validated snapshot.
    let error = ProjectState::open(&broken).unwrap_err();
    assert_eq!(error.kind, ProjectErrorKind::TrackingJson);
    let recovered = ProjectState::open(&valid).unwrap();
    assert!(recovered.find("blog/").is_some());
    let home = recovered.find("/").unwrap();
    assert_eq!(
        recovered.content_path(home),
        valid.join("content/index.html")
    );
}

/// Recursive tree snapshot: relative path -> contents.
fn tree_snapshot(root: &Path) -> BTreeMap<String, String> {
    let mut snapshot = BTreeMap::new();
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                out.insert(relative, std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }
    walk(root, root, &mut snapshot);
    snapshot
}

#[test]
fn zero_writes() {
    let root = fixture("zero-writes");
    write_valid_project(&root);
    let before = tree_snapshot(&root);
    assert!(!before.keys().any(|path| path.contains(".info.json")));

    let state = ProjectState::open(&root).unwrap();
    for info in state.tracked() {
        state.content_path(info);
        state.output_path(info);
        state.pagination_output_path(info, 2);
        let _ = state.read_shared_source(&state.content_path(info));
    }
    let _ = state.read_shared_json(&root.join("content/site.json"));

    let after = tree_snapshot(&root);
    assert_eq!(after, before);
    assert!(!after.keys().any(|path| path.contains(".info.json")));
}

#[test]
fn concurrent_reads_are_stable() {
    let root = fixture("concurrent");
    write_valid_project(&root);
    let state = Arc::new(ProjectState::open(&root).unwrap());

    let ok = Arc::new(AtomicBool::new(true));
    let mut workers = Vec::new();
    for _ in 0..8 {
        let state = Arc::clone(&state);
        let ok = Arc::clone(&ok);
        let thread_root = root.clone();
        workers.push(std::thread::spawn(move || {
            for _ in 0..50 {
                if state.find("about").is_none() {
                    ok.store(false, Ordering::SeqCst);
                }
                if let Some(source) =
                    state.read_shared_source(&thread_root.join("content/about.html"))
                {
                    if &*source != "<p>content</p>" {
                        ok.store(false, Ordering::SeqCst);
                    }
                } else {
                    ok.store(false, Ordering::SeqCst);
                }
                if state
                    .read_shared_json(&thread_root.join("content/site.json"))
                    .is_err()
                {
                    ok.store(false, Ordering::SeqCst);
                }
            }
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    assert!(ok.load(Ordering::SeqCst));
}

// ---------------------------------------------------------------------------
// Adversarial project state
// ---------------------------------------------------------------------------

#[test]
fn root_normalization_and_relative_root() {
    let root = fixture("root-normal");
    write_valid_project(&root);

    // A root spelled with "." / trailing components normalizes to the same
    // geometry as the clean root.
    let dotted = root.join(".").join("content").join("..");
    let state = ProjectState::open(&dotted).unwrap();
    assert_eq!(state.root(), &root);
    let home = state.find("/").unwrap();
    assert_eq!(state.content_path(home), root.join("content/index.html"));

    // A relative root is absolutised against the current directory.
    fn path_relative_to(cwd: &Path, target: &Path) -> PathBuf {
        let cwd_components: Vec<_> = cwd.components().collect();
        let target_components: Vec<_> = target.components().collect();
        let mut common = 0;
        while common < cwd_components.len()
            && common < target_components.len()
            && cwd_components[common] == target_components[common]
        {
            common += 1;
        }
        let mut out = PathBuf::new();
        for _ in common..cwd_components.len() {
            out.push("..");
        }
        for component in &target_components[common..] {
            out.push(component.as_os_str());
        }
        out
    }
    let relative_root = path_relative_to(&std::env::current_dir().unwrap(), &root);
    let state = ProjectState::open(&relative_root).unwrap();
    assert_eq!(state.root(), &root);
}

#[test]
fn relative_generic_spelling() {
    let root = fixture("relative-spelling");
    write_valid_project(&root);
    let state = ProjectState::open(&root).unwrap();

    // Nested relative path uses generic `/` spelling.
    assert_eq!(
        state.relative(&root.join("content/blog/index.html")),
        "content/blog/index.html"
    );

    // Root-equal spelling is ".": the C++ reference's lexically_relative(root,
    // root) returns "." (probed against the frozen compiler), so relative_of
    // spells the root itself as "." rather than an absolute path.
    assert_eq!(state.relative(state.root()), ".");

    // Outside-root spelling is the lexical `..` form.
    assert_eq!(
        state.relative(&root.join("..").join("outside.html")),
        "../outside.html"
    );
}

#[cfg(unix)]
#[test]
fn relative_preserves_literal_backslash_in_filename() {
    // On POSIX a backslash is an ordinary filename character: C++
    // generic_string() does not reinterpret it, so it must survive spelling.
    let root = fixture("relative-backslash");
    write_file(
        &root.join(".nift/config.json"),
        r#"{"config":{"content-dir":"content/"}}"#,
    );
    write_file(&root.join(".nift/tracked.json"), KTRACKED);
    write_file(&root.join("content/back\\slash.html"), "<p>x</p>");
    let state = ProjectState::open(&root).unwrap();
    assert_eq!(
        state.relative(&root.join("content/back\\slash.html")),
        "content/back\\slash.html"
    );
    // The shared source read key must keep the same spelling and hit the cache.
    assert_eq!(
        &*state
            .read_shared_source(&root.join("content/back\\slash.html"))
            .unwrap(),
        "<p>x</p>"
    );
}

#[cfg(windows)]
#[test]
fn relative_uses_forward_slashes_on_windows() {
    let root = fixture("relative-windows");
    write_valid_project(&root);
    let state = ProjectState::open(&root).unwrap();
    // Native PathBuf spelling is `\`-separated; generic spelling must be `/`.
    let spelled = state.relative(&root.join("content/about.html"));
    assert_eq!(spelled, "content/about.html");
    assert!(!spelled.contains('\\'));
}

#[test]
fn tracked_name_backslash_parent_rejected() {
    let root = fixture("backslash-parent");
    write_file(&root.join(".nift/config.json"), KCONFIG);
    write_file(
        &root.join(".nift/tracked.json"),
        r#"{"tracked":[{"name":"..\\escape","title":"X"}]}"#,
    );
    // The reference normalises `\` to `/` for the parent-component rule, so
    // `..\escape` is rejected on all platforms.
    let error = ProjectState::open(&root).unwrap_err();
    assert_eq!(error.kind, ProjectErrorKind::TrackingValue);
}

/// Portable symlink creation (unix `symlink`; Windows `symlink_file`). Returns
/// `false` on platforms/permits where symlinks cannot be created, so the test
/// is preserved everywhere and only skipped where the capability is absent.
#[cfg(unix)]
fn make_symlink(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

#[cfg(windows)]
fn make_symlink(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

#[test]
fn symlink_contract_escape_rejected() {
    let root = fixture("symlink-escape");
    std::fs::create_dir_all(root.join("content")).unwrap();
    let outside = fixture("symlink-outside");
    std::fs::write(outside.join("secret.json"), "{}").unwrap();
    let _ = std::fs::remove_file(root.join("content/link.json"));
    if !make_symlink(
        &outside.join("secret.json"),
        &root.join("content/link.json"),
    ) {
        // Platform without symlink support/privilege: skip.
        return;
    }
    write_file(
        &root.join(".nift/config.json"),
        r#"{"config":{"contracts":{"site":"content/link.json"}}}"#,
    );
    write_file(&root.join(".nift/tracked.json"), KTRACKED);
    // The lexical check passes (content/link.json is lexically inside the
    // root); the symlink-aware containment check must reject the escape.
    let error = ProjectState::open(&root).unwrap_err();
    assert_eq!(error.kind, ProjectErrorKind::ConfigValue);
    assert!(error.message.contains("stay inside"), "{}", error.message);
}

#[test]
fn absolute_content_dir_geometry() {
    // The reference joins `root / content-dir` (PathBuf::join semantics: an
    // absolute RHS replaces the root) and performs no containment check on the
    // directories; this documents the frozen observable behaviour.
    let root = fixture("absolute-content-dir");
    let abs = std::env::temp_dir().join(format!("nift-nr7-abs-content-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&abs);
    std::fs::create_dir_all(&abs).unwrap();
    // The path is embedded in JSON, so backslashes (Windows) must be escaped.
    let content_dir = abs.to_string_lossy().replace('\\', "\\\\");
    write_file(
        &root.join(".nift/config.json"),
        &format!(r#"{{"config":{{"content-dir":"{content_dir}"}}}}"#),
    );
    write_file(
        &root.join(".nift/tracked.json"),
        r#"{"tracked":[{"name":"/","title":"Home"}]}"#,
    );
    let state = ProjectState::open(&root).unwrap();
    let home = state.find("/").unwrap();
    assert_eq!(state.content_path(home), abs.join("index.html"));
}
