//! NR6: the public standalone Engine API. An SSR-style render entirely through
//! the public Engine surface - template, request/runtime bindings, JSON/data,
//! a partial, @if/@for, and an explicit current output - plus configuration
//! seams (root, loader, environment provider), errors, dependencies/
//! requirements, and concurrent rendering.

use nift::context::Context;
use nift::{Engine, RenderResult, Source, Value};
use std::path::PathBuf;

fn write_file(path: &std::path::Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn temp_root() -> PathBuf {
    let id = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    std::env::temp_dir().join(format!("nift-nr6-{}-{id}", std::process::id()))
}

fn write_ssr_project(root: &std::path::Path) {
    write_file(
        &root.join("templates/template.html"),
        "<!doctype html><html><body>@content</body></html>",
    );
    write_file(&root.join("templates/head.html"), "<h1>@ent(\"&\")</h1>");
    write_file(
        &root.join("content/blog.html"),
        "@json(\"data.json\", posts)@for(p : posts.items by p.rank asc){@if(p.published){<article>$[p.title]</article>}}@input(\"templates/head.html\")<a href=\"@pathto('public/app.js')\">app</a>",
    );
    write_file(
        &root.join("data.json"),
        r#"{"items":[{"title":"one","rank":2,"published":true},{"title":"two","rank":1,"published":true},{"title":"three","rank":3,"published":false}]}"#,
    );
    write_file(&root.join("public/app.js"), "x");
}

#[test]
fn ssr_render_through_public_engine() {
    let root = temp_root();
    write_ssr_project(root.as_path());
    let mut engine = Engine::new();
    engine.set_root(&root);
    engine
        .set_json("site", r#"{"name":"Nift"}"#)
        .expect("valid json");
    let mut context = Context::new();
    context.set_current_output(root.join("public/blog.html"));
    context.set("request", Value::string("req-1")).unwrap();
    let result: RenderResult = engine
        .render(
            &Source::path("content/blog.html"),
            &Source::path("templates/template.html"),
            &context,
        )
        .expect("render should succeed");
    assert_eq!(
        result.output,
        "<!doctype html><html><body><article>two</article><article>one</article><h1>&amp;</h1><a href=\"./app.js\">app</a></body></html>"
    );
    assert!(result.dependencies.contains("data.json"));
    assert!(result.dependencies.contains("templates/head.html"));
    assert!(result.requirements.contains("public/app.js"));
    std::fs::remove_dir_all(root.as_path()).ok();
}

#[test]
fn engine_render_from_text_and_partial() {
    let root = temp_root();
    write_ssr_project(root.as_path());
    let mut engine = Engine::new();
    engine.set_root(&root);
    // Composed render from text sources.
    let result = engine
        .render(
            &Source::text("<h2>Page</h2>"),
            &Source::text("<main>@content</main>"),
            &Context::new(),
        )
        .expect("render should succeed");
    assert_eq!(result.output, "<main><h2>Page</h2></main>");
    // Partial render.
    let result = engine
        .render_partial(&Source::text("@if(true){ok}"), &Context::new())
        .expect("render should succeed");
    assert_eq!(result.output, "ok");
    // @content in a partial is an error.
    let error = engine
        .render_partial(&Source::text("@content"), &Context::new())
        .expect_err("@content in a partial must fail");
    assert!(error.message.contains("requires a page source"));
    std::fs::remove_dir_all(root.as_path()).ok();
}

#[test]
fn engine_pathto_requires_current_output() {
    let root = temp_root();
    write_ssr_project(root.as_path());
    let mut engine = Engine::new();
    engine.set_root(&root);
    let error = engine
        .render(
            &Source::text("@pathto('public/app.js')"),
            &Source::text("@content"),
            &Context::new(),
        )
        .expect_err("@pathto without current output must fail");
    assert!(error.message.contains("requires a path context"));
    std::fs::remove_dir_all(root.as_path()).ok();
}

#[test]
fn engine_loader_and_environment_seams() {
    // Loader-backed engine: sources come from memory, no filesystem. The
    // loader receives root-resolved absolute path keys (matching the
    // reference standalone Engine host).
    let root = temp_root();
    let template_key = root
        .join("templates/template.html")
        .to_string_lossy()
        .to_string();
    let page_key = root.join("content/page.html").to_string_lossy().to_string();
    let sources = [
        (template_key, "<main>@content</main>".to_string()),
        (
            page_key,
            "<p>$[greeting]</p>@getenv(\"PA_NR6\")".to_string(),
        ),
    ]
    .into_iter()
    .collect::<std::collections::HashMap<_, _>>();
    let mut engine = Engine::new();
    engine.set_root(&root);
    engine.set_loader(move |path| sources.get(path).cloned());
    engine.set_environment_provider(|name| {
        if name == "PA_NR6" {
            Some("env-val".to_string())
        } else {
            None
        }
    });
    engine.set("greeting", "hi").expect("binding");
    let context = Context::new();
    let result = engine
        .render(
            &Source::path("content/page.html"),
            &Source::path("templates/template.html"),
            &context,
        )
        .expect("render should succeed");
    assert_eq!(result.output, "<main><p>hi</p>env-val</main>");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn engine_concurrent_renders() {
    let root = std::sync::Arc::new(temp_root());
    write_ssr_project(root.as_path());
    let mut engine = Engine::new();
    engine.set_root(root.as_path());
    engine
        .set_json("site", r#"{"name":"Nift"}"#)
        .expect("valid json");
    let engine = std::sync::Arc::new(engine);
    let mut workers = Vec::new();
    for thread_id in 0..8 {
        let engine = engine.clone();
        let root = root.clone();
        workers.push(std::thread::spawn(move || {
            for _ in 0..20 {
                let mut context = Context::new();
                context.set_current_output(root.join("public/blog.html"));
                context
                    .set("request", Value::number(thread_id as f64))
                    .unwrap();
                let result = engine
                    .render(
                        &Source::path("content/blog.html"),
                        &Source::path("templates/template.html"),
                        &context,
                    )
                    .expect("render should succeed");
                assert!(result.output.contains("<article>two</article>"));
            }
        }));
    }
    for worker in workers {
        worker.join().expect("worker panicked");
    }
    std::fs::remove_dir_all(root.as_path()).ok();
}
