//! NR13 evidence: the complete pagination set of one RenderResult comes from a
//! single immutable Engine snapshot (CP8.1), including while reload() publishes
//! a new generation concurrently, and a failed reload retains the last
//! known-good pagination generation.
//!
//! Deterministic interleave (no sleeps): the pagination template renders
//! `@getenv("BARRIER")` on every page, so the environment provider callback
//! fires during pagination assembly -- after the render has captured its
//! project snapshot and before the complete multi-page RenderResult has been
//! assembled. The provider blocks on a condvar; the test thread observes
//! "entered", reloads generation B, then releases the barrier. Every page also
//! renders `$[title]` (a snapshot value), so a result that mixed generations
//! would show GEN-A on some pages and GEN-B on others. The single result must
//! be entirely GEN-A and the next render entirely GEN-B.

use nift::context::Context;
use nift::{Engine, RenderResult};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

/// One-shot deterministic barrier installed as the environment provider.
struct Barrier {
    state: Mutex<(bool, bool)>,
    cv: Condvar,
    armed: AtomicBool,
}

impl Barrier {
    fn new() -> Arc<Self> {
        Arc::new(Barrier {
            state: Mutex::new((false, false)),
            cv: Condvar::new(),
            armed: AtomicBool::new(true),
        })
    }

    /// Called from the render thread (inside the pagination loop). Blocks on
    /// the first invocation until released; later invocations pass through.
    fn enter_once(&self) {
        if !self.armed.swap(false, Ordering::SeqCst) {
            return;
        }
        let mut guard = self.state.lock().unwrap();
        guard.0 = true;
        self.cv.notify_all();
        while !guard.1 {
            guard = self.cv.wait(guard).unwrap();
        }
    }

    fn wait_entered(&self) {
        let mut guard = self.state.lock().unwrap();
        while !guard.0 {
            guard = self.cv.wait(guard).unwrap();
        }
    }

    fn release(&self) {
        let mut guard = self.state.lock().unwrap();
        guard.1 = true;
        self.cv.notify_all();
    }
}

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(path, contents).expect("write file");
}

fn tracked_with_title(title: &str) -> String {
    format!(
        r#"{{"tracked":[{{"name":"blog","title":"{}","template":"templates/template.html","paginate":{{"items-per-page":1}}}}]}}"#,
        title
    )
}

fn write_project(root: &Path) {
    write_file(
        &root.join(".nift/config.json"),
        r#"{"config":{"content-dir":"content/","content-ext":".html","output-dir":"public/","output-ext":".html","default-template":"templates/template.html","incremental-mode":"modified"}}"#,
    );
    write_file(&root.join(".nift/tracked.json"), &tracked_with_title("GEN-A"));
    write_file(&root.join("templates/template.html"), "<main>$[title]</main>\n@content");
    write_file(
        &root.join("content/blog.html"),
        "@item{A1}@item{A2}@item{A3}@paginate",
    );
    write_file(
        &root.join("content/blog.paginate.html"),
        r#"<section>@getenv("BARRIER")$[title] page $[paginate.current]/$[paginate.total]:[$[paginate.items]]</section>"#,
    );
}

/// Page 1 plus every pagination page, in order.
fn all_page_texts(result: &RenderResult) -> Vec<&str> {
    let mut texts: Vec<&str> = vec![result.output.as_str()];
    texts.extend(result.pagination.iter().map(|page| page.output.as_str()));
    texts
}

/// Every page must come from generation `yes` and never show `no`.
fn result_is_entirely(result: &RenderResult, yes: &str, no: &str, paginated: bool) -> bool {
    if paginated {
        let pages: Vec<usize> = result.pagination.iter().map(|page| page.page).collect();
        if result.pagination.len() != 2 || pages != [2, 3] {
            return false;
        }
    } else if !result.pagination.is_empty() {
        return false;
    }
    all_page_texts(result)
        .iter()
        .all(|text| text.contains(yes) && !text.contains(no))
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nift-nr13-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn concurrent_reload_single_snapshot() {
    let root = temp_dir("concurrent");
    write_project(&root);

    let mut engine = Engine::project(&root);
    assert!(engine.is_open());

    let barrier = Barrier::new();
    let barrier_provider = Arc::clone(&barrier);
    engine.set_environment_provider(move |name: &str| -> Option<String> {
        if name == "BARRIER" {
            barrier_provider.enter_once();
        }
        None
    });

    let result: RenderResult = std::thread::scope(|scope| {
        let render = scope.spawn(|| engine.render_page("blog", &Context::new()).expect("render"));
        // Render is deterministically inside the pagination loop now.
        barrier.wait_entered();
        // Publish generation B while the render is mid-flight.
        write_file(&root.join(".nift/tracked.json"), &tracked_with_title("GEN-B"));
        engine.reload().expect("reload generation B");
        barrier.release();
        render.join().expect("render thread")
    });

    // The single result is entirely generation A -- page 1 and pages 2..3.
    assert!(result_is_entirely(&result, "GEN-A", "GEN-B", true), "mixed generations");

    // The next render observes the newly published generation B.
    let next = engine.render_page("blog", &Context::new()).expect("render after reload");
    assert!(result_is_entirely(&next, "GEN-B", "GEN-A", true), "next render not entirely B");
}

#[test]
fn failed_reload_retains_last_good() {
    let root = temp_dir("failed");
    write_project(&root);

    let mut engine = Engine::project(&root);
    assert!(engine.is_open());

    let first = engine.render_page("blog", &Context::new()).expect("render");
    assert!(result_is_entirely(&first, "GEN-A", "GEN-B", true));

    // Disk becomes invalid; reload must fail and retain generation A.
    write_file(&root.join(".nift/tracked.json"), "{ not json");
    assert!(engine.reload().is_err());
    assert!(engine.is_open());

    let after_failed = engine.render_page("blog", &Context::new()).expect("render");
    assert!(result_is_entirely(&after_failed, "GEN-A", "GEN-B", true), "last-good lost");

    // A later valid reload recovers; the next render observes the new generation.
    write_file(&root.join(".nift/tracked.json"), &tracked_with_title("GEN-C"));
    engine.reload().expect("reload recovery");
    let recovered = engine.render_page("blog", &Context::new()).expect("render");
    assert!(result_is_entirely(&recovered, "GEN-C", "GEN-A", true), "recovery not entirely C");

    let _ = std::fs::remove_dir_all(&root);
}
