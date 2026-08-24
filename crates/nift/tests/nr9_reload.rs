//! NR9 gate: reload + concurrent serving lifecycle.
//!
//! Ports the frozen C++ `engine_reload` evidence: immutable Arc generations,
//! atomic publication, last-good retention on failed reload, in-flight renders
//! finishing on the generation they started with, reload as the retry path for
//! an Engine constructed before its project existed, defaults/environment
//! provider surviving reload, zero project writes, and heavy concurrent
//! render+reload stress.

use nift::context::Context;
use nift::Engine;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Barrier;

fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "nift-nr9-{}-{}",
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

/// Atomic replace so a concurrent reload never reads a torn candidate file.
fn write_file_atomic(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents).unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

/// Atomic replace with a per-write unique temporary name, safe when several
/// threads publish candidate files concurrently (no shared `.tmp` path).
fn write_file_atomic_unique(path: &Path, contents: &str) {
    static TMP_COUNTER: AtomicUsize = AtomicUsize::new(0);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let tmp = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file"),
        TMP_COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::write(&tmp, contents).unwrap();
    std::fs::rename(&tmp, path).unwrap();
}

const KCONFIG: &str = r#"{"config":{"content-dir":"content/","content-ext":".html","output-dir":"public/","output-ext":".html","default-template":"templates/template.html","incremental-mode":"modified"}}"#;

fn tracked_with_title(title: &str) -> String {
    format!(
        r#"{{"tracked":[{{"name":"about","title":"{title}","template":"templates/template.html"}}]}}"#
    )
}

fn write_project(root: &Path) {
    write_file(&root.join(".nift/config.json"), KCONFIG);
    write_file(
        &root.join(".nift/tracked.json"),
        &tracked_with_title("Title-ALPHA"),
    );
    write_file(
        &root.join("templates/template.html"),
        "<!doctype html><title>$[title]</title>@content",
    );
    write_file(&root.join("content/about.html"), "<h1>About</h1>");
}

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
fn new_page_after_reload() {
    let root = fixture("new-page");
    write_project(&root);
    let engine = Engine::open(&root).unwrap();
    assert!(engine.is_open());
    assert!(engine.render_page("newpage", &Context::new()).is_err());

    write_file(&root.join("content/newpage.html"), "<h1>New</h1>");
    write_file(
        &root.join(".nift/tracked.json"),
        r#"{"tracked":[{"name":"about","title":"About","template":"templates/template.html"},{"name":"newpage","title":"New","template":"templates/template.html"}]}"#,
    );
    engine.reload().unwrap();
    let page = engine.render_page("newpage", &Context::new()).unwrap();
    assert!(page.output.contains("<h1>New</h1>"), "{}", page.output);
    assert!(engine.render_page("about", &Context::new()).is_ok());
}

#[test]
fn failed_reload_retains_last_good() {
    let root = fixture("failed-reload");
    write_project(&root);
    let engine = Engine::open(&root).unwrap();
    assert!(engine.is_open());
    let before = engine.render_page("about", &Context::new()).unwrap();
    assert!(before.output.contains("Title-ALPHA"));

    // The project becomes malformed; reload must fail without destroying the
    // snapshot currently being served.
    write_file(&root.join(".nift/tracked.json"), "{ not json");
    let error = engine.reload().expect_err("reload must fail");
    assert!(!error.message.is_empty());
    assert!(engine.is_open());

    let after = engine.render_page("about", &Context::new()).unwrap();
    assert!(after.output.contains("Title-ALPHA"), "{}", after.output);

    // Recovery: the malformed state is fixed and reload succeeds again.
    write_file(
        &root.join(".nift/tracked.json"),
        &tracked_with_title("Title-ALPHA"),
    );
    engine.reload().unwrap();
    assert!(engine.render_page("about", &Context::new()).is_ok());
}

#[test]
fn reload_as_open_retry() {
    let root = fixture("open-retry");
    let empty = root.join("empty");
    std::fs::create_dir_all(&empty).unwrap();

    // Lifecycle construction never fails: the Engine exists without a project
    // and can later open through reload.
    let engine = Engine::project(&empty);
    assert!(!engine.is_open());
    assert!(engine.open_error().is_some());
    let error = engine
        .render_page("about", &Context::new())
        .expect_err("not open");
    assert!(!error.message.is_empty());

    write_project(&empty);
    engine
        .reload()
        .expect("reload establishes the first generation");
    assert!(engine.is_open());
    assert!(engine.open_error().is_none());
    let result = engine.render_page("about", &Context::new()).unwrap();
    assert!(
        result.output.contains("<h1>About</h1>"),
        "{}",
        result.output
    );
}

#[test]
fn generation_switch() {
    let root = fixture("generation-switch");
    write_project(&root);
    let engine = Engine::open(&root).unwrap();
    assert!(engine
        .render_page("about", &Context::new())
        .unwrap()
        .output
        .contains("Title-ALPHA"));

    write_file(
        &root.join(".nift/tracked.json"),
        &tracked_with_title("Title-BETA"),
    );
    engine.reload().unwrap();
    let switched = engine.render_page("about", &Context::new()).unwrap();
    assert!(
        switched.output.contains("Title-BETA"),
        "{}",
        switched.output
    );
}

#[test]
fn deterministic_lifecycle() {
    let root = fixture("deterministic");
    write_project(&root); // tracked title = Title-ALPHA
    let engine = Engine::open(&root).unwrap();

    // Opens snapshot A; renders observe A.
    assert!(engine
        .render_page("about", &Context::new())
        .unwrap()
        .output
        .contains("Title-ALPHA"));

    // Disk becomes B, but the Engine keeps serving its captured snapshot A:
    // this is snapshot semantics, not re-opening the project per render.
    write_file(
        &root.join(".nift/tracked.json"),
        &tracked_with_title("Title-BETA"),
    );
    let before_reload = engine.render_page("about", &Context::new()).unwrap();
    assert!(
        before_reload.output.contains("Title-ALPHA"),
        "{}",
        before_reload.output
    );

    // reload() atomically replaces the generation; later renders observe B.
    engine.reload().unwrap();
    let after_reload = engine.render_page("about", &Context::new()).unwrap();
    assert!(
        after_reload.output.contains("Title-BETA"),
        "{}",
        after_reload.output
    );
}

#[test]
fn concurrent_render_and_reload() {
    let root = fixture("concurrent");
    write_project(&root);
    const RENDER_THREADS: usize = 8;
    const ITERATIONS: usize = 30;
    const RELOADS: usize = 40;

    // ONE shared Engine created before any thread starts: every render and
    // reload below is on the same Engine and the same snapshot lifecycle.
    let engine = Arc::new(Engine::open(&root).unwrap());
    assert!(engine.is_open());

    // Two barriers make the overlap deterministic (a pure start barrier can
    // still let fast renders finish before the reloader ever publishes the
    // other generation under CI load):
    //   start      all render threads + the reloader begin together
    //   after_beta renders render the initial generation once, the reloader
    //              publishes the alternate generation, then everyone resumes
    //              and the remaining reloads overlap with the remaining
    //              renders.
    let start = Arc::new(Barrier::new(RENDER_THREADS + 1));
    let after_beta = Arc::new(Barrier::new(RENDER_THREADS + 1));
    let renders_ok = Arc::new(AtomicBool::new(true));
    let saw_a = Arc::new(AtomicBool::new(false));
    let saw_b = Arc::new(AtomicBool::new(false));
    let mut workers = Vec::new();
    for _ in 0..RENDER_THREADS {
        let engine = Arc::clone(&engine);
        let renders_ok = Arc::clone(&renders_ok);
        let saw_a = Arc::clone(&saw_a);
        let saw_b = Arc::clone(&saw_b);
        let start = Arc::clone(&start);
        let after_beta = Arc::clone(&after_beta);
        workers.push(std::thread::spawn(move || {
            start.wait();
            // One render against the initial (ALPHA) generation.
            let result = engine.render_page("about", &Context::new()).unwrap();
            if result.output.contains("Title-ALPHA") {
                saw_a.store(true, Ordering::SeqCst);
            } else if result.output.contains("Title-BETA") {
                saw_b.store(true, Ordering::SeqCst);
            } else {
                renders_ok.store(false, Ordering::SeqCst);
            }
            // Wait for the reloader to publish the alternate generation, then
            // continue rendering while the reloader keeps reloading.
            after_beta.wait();
            for _ in 0..ITERATIONS {
                let result = match engine.render_page("about", &Context::new()) {
                    Ok(result) => result,
                    Err(_) => {
                        renders_ok.store(false, Ordering::SeqCst);
                        continue;
                    }
                };
                // Every render observes exactly one committed snapshot
                // generation: never a torn mix, never an unknown page.
                let generation_a = result.output.contains("Title-ALPHA");
                let generation_b = result.output.contains("Title-BETA");
                if generation_a == generation_b {
                    renders_ok.store(false, Ordering::SeqCst);
                }
                if generation_a {
                    saw_a.store(true, Ordering::SeqCst);
                }
                if generation_b {
                    saw_b.store(true, Ordering::SeqCst);
                }
            }
        }));
    }

    // Reload thread, on the SAME shared Engine: flips between two known-good
    // generations and injects failed reloads while renders run. A failed reload
    // must retain the last-good snapshot (renders keep succeeding) and a later
    // valid reload must recover.
    let reloads_ok = Arc::new(AtomicBool::new(true));
    let engine_reloader = Arc::clone(&engine);
    let reloads_ok_2 = Arc::clone(&reloads_ok);
    let start = Arc::clone(&start);
    let after_beta = Arc::clone(&after_beta);
    let root2 = root.clone();
    let reloader = std::thread::spawn(move || {
        start.wait();
        // Publish the alternate generation first so the renders deterministically
        // observe both ALPHA (before) and BETA (after) while reloads overlap
        // with renders.
        write_file_atomic(
            &root2.join(".nift/tracked.json"),
            &tracked_with_title("Title-BETA"),
        );
        if engine_reloader.reload().is_err() {
            reloads_ok_2.store(false, Ordering::SeqCst);
        }
        after_beta.wait();
        for i in 1..RELOADS {
            if i % 5 == 4 {
                write_file_atomic(&root2.join(".nift/tracked.json"), "{ not json");
                if engine_reloader.reload().is_ok() {
                    reloads_ok_2.store(false, Ordering::SeqCst);
                }
            } else {
                write_file_atomic(
                    &root2.join(".nift/tracked.json"),
                    &tracked_with_title(if i % 2 == 0 {
                        "Title-BETA"
                    } else {
                        "Title-ALPHA"
                    }),
                );
                if engine_reloader.reload().is_err() {
                    reloads_ok_2.store(false, Ordering::SeqCst);
                }
            }
        }
    });

    reloader.join().unwrap();
    for worker in workers {
        worker.join().unwrap();
    }
    assert!(
        renders_ok.load(Ordering::SeqCst),
        "a render observed a torn/mixed generation"
    );
    assert!(
        reloads_ok.load(Ordering::SeqCst),
        "a reload failed or succeeded unexpectedly"
    );
    // The reloads genuinely switched generations: renders saw BOTH committed
    // snapshots, so the test is not vacuous.
    assert!(
        saw_a.load(Ordering::SeqCst),
        "no render ever observed generation ALPHA"
    );
    assert!(
        saw_b.load(Ordering::SeqCst),
        "no render ever observed generation BETA"
    );

    // Recovery: after all the malformed injections and the final valid reload,
    // the shared Engine still renders a committed generation.
    assert!(engine.render_page("about", &Context::new()).is_ok());
    // The Engine never wrote anything: no .info.json anywhere in the project.
    for path in tree_snapshot(&root).keys() {
        assert!(!path.contains(".info.json"), "engine wrote {path}");
    }
}

#[test]
fn zero_writes_across_reload() {
    let root = fixture("zero-writes");
    write_project(&root);
    let engine = Engine::open(&root).unwrap();

    write_file(
        &root.join(".nift/tracked.json"),
        &tracked_with_title("About2"),
    );
    let before_success = tree_snapshot(&root);
    engine.reload().unwrap();
    assert!(engine.render_page("about", &Context::new()).is_ok());
    assert_eq!(tree_snapshot(&root), before_success);

    write_file(&root.join(".nift/tracked.json"), "{ not json");
    let before_failure = tree_snapshot(&root);
    assert!(engine.reload().is_err());
    assert_eq!(tree_snapshot(&root), before_failure);
    for path in tree_snapshot(&root).keys() {
        assert!(!path.contains(".info.json"));
    }
}

#[test]
fn defaults_survive_reload() {
    let root = fixture("defaults-survive");
    write_project(&root);
    let mut engine = Engine::open(&root).unwrap();
    engine.set_json("app", r#"{"name":"Kept"}"#).unwrap();
    engine.set_environment_provider(|_| Some("KeptEnv".to_string()));

    write_file(
        &root.join("content/about.html"),
        "<h1>$[app.name]</h1><p>env=@getenv(\"PA_ENV_VAR\")</p>",
    );
    write_file(
        &root.join(".nift/tracked.json"),
        &tracked_with_title("About2"),
    );
    engine.reload().unwrap();

    let result = engine.render_page("about", &Context::new()).unwrap();
    assert!(result.output.contains("Kept"), "{}", result.output);
    assert!(result.output.contains("KeptEnv"), "{}", result.output);
}

#[test]
fn concurrent_reload_and_reload() {
    let root = fixture("concurrent-reloads");
    write_project(&root); // open on generation Title-ALPHA
    let engine = Arc::new(Engine::open(&root).unwrap());
    assert!(engine.is_open());

    // Two reload workers on the SAME Engine. Candidates may build concurrently;
    // publication is serialized by the Engine's publication lock and the last
    // successful candidate to acquire it wins (no "newest invocation" or
    // timestamp ordering is imposed). Successful candidates only ever publish a
    // fully validated ProjectState generation.
    const WORKERS: usize = 2;
    const RELOADS: usize = 250;

    let barrier = Arc::new(Barrier::new(WORKERS));
    let mut workers = Vec::new();
    for w in 0..WORKERS {
        let engine = Arc::clone(&engine);
        let barrier = Arc::clone(&barrier);
        let root = root.clone();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            for i in 0..RELOADS {
                // Mix one invalid candidate every five iterations. Which reload
                // observes the invalid file is racy (the other worker may have
                // republished a valid one), so no success/failure count is
                // asserted; the point is that concurrent reloads never
                // deadlock, panic, or publish a torn generation, and a failed
                // candidate can never erase a successfully published one.
                if (i + w) % 5 == 4 {
                    write_file_atomic_unique(&root.join(".nift/tracked.json"), "{ not json");
                } else {
                    write_file_atomic_unique(
                        &root.join(".nift/tracked.json"),
                        &tracked_with_title(if (i + w) % 2 == 0 {
                            "Title-BETA"
                        } else {
                            "Title-ALPHA"
                        }),
                    );
                }
                let _ = engine.reload();
            }
        }));
    }
    for worker in workers {
        // Joining proves neither worker deadlocked or panicked.
        worker.join().unwrap();
    }

    // The Engine is still serving one complete, valid generation.
    assert!(engine.is_open());
    assert!(engine.render_page("about", &Context::new()).is_ok());

    // Deterministic recovery: publish a known-good tracked.json, reload, and
    // confirm the rendered result corresponds to one complete valid generation.
    write_file_atomic(
        &root.join(".nift/tracked.json"),
        &tracked_with_title("Title-FINAL"),
    );
    engine
        .reload()
        .expect("deterministic reload after the workers");
    assert!(engine.is_open());
    let result = engine.render_page("about", &Context::new()).unwrap();
    assert!(
        result.output.contains("<h1>About</h1>"),
        "{}",
        result.output
    );
    assert!(result.output.contains("Title-FINAL"), "{}", result.output);

    // Zero project writes: no .info.json anywhere.
    for path in tree_snapshot(&root).keys() {
        assert!(!path.contains(".info.json"), "engine wrote {path}");
    }
}
