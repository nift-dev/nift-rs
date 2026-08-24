//! NR12 benchmark: nift-rs vs representative Rust template engines and the
//! frozen C++ Engine.
//!
//! Renders an equivalent representative SSR page (conditional greeting + a
//! 10-post loop with interpolation) through nift-rs, Tera, MiniJinja and
//! Askama, and reports nanoseconds per render after warmup. When the C++ bench
//! (`nift-embed/.build/engine-bench`, NIFT_CPP_BENCH) is present it is invoked
//! and its ns/op included for the same Nift template.
//!
//! Run in release: `cargo run --release --example bench`.

use nift::context::Context;
use nift::source::Source;
use nift::Engine;
use std::process::Command;
use std::time::Instant;

const ITERATIONS: u32 = 50_000;
const WARMUP: u32 = 2_000;

fn bench<F: FnMut() -> R, R>(mut f: F) -> f64 {
    for _ in 0..WARMUP {
        std::hint::black_box(f());
    }
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        std::hint::black_box(f());
    }
    start.elapsed().as_nanos() as f64 / ITERATIONS as f64
}

fn build_posts_json() -> String {
    let posts: Vec<serde_json::Value> = (0..10)
        .map(|i| {
            serde_json::json!({ "title": format!("Post {i}"), "body": "Some body text for the post." })
        })
        .collect();
    serde_json::to_string(&posts).unwrap()
}

fn main() {
    let nift_template = "@if(user.logged_in){<p>Hello $[user.name]</p>}@else{<p>Hello guest</p>}\n@for(post : posts){<article><h2>$[post.title]</h2><p>$[post.body]</p></article>}\n";
    let jinja_template = "{% if user.logged_in %}<p>Hello {{ user.name }}</p>{% else %}<p>Hello guest</p>{% endif %}\n{% for post in posts %}<article><h2>{{ post.title }}</h2><p>{{ post.body }}</p></article>{% endfor %}\n";
    let posts_json = build_posts_json();

    // nift-rs: Engine defaults + text sources.
    let mut nift = Engine::new();
    nift.set_json("user", r#"{"logged_in":true,"name":"Ada"}"#)
        .unwrap();
    nift.set_json("posts", &posts_json).unwrap();
    let context = Context::new();
    let nift_ns = bench(|| {
        nift.render(
            &Source::text(nift_template),
            &Source::text("@content"),
            &context,
        )
    });

    // Tera.
    let mut tera = tera::Tera::default();
    tera.add_raw_template("page", jinja_template).unwrap();
    let mut tera_ctx = tera::Context::new();
    tera_ctx.insert(
        "user",
        &serde_json::json!({"logged_in": true, "name": "Ada"}),
    );
    tera_ctx.insert(
        "posts",
        &serde_json::from_str::<serde_json::Value>(&posts_json).unwrap(),
    );
    let tera_ns = bench(|| tera.render("page", &tera_ctx));

    // MiniJinja.
    let mut env = minijinja::Environment::new();
    env.add_template("page", jinja_template).unwrap();
    let template = env.get_template("page").unwrap();
    let jinja_ctx = serde_json::json!({
        "user": {"logged_in": true, "name": "Ada"},
        "posts": serde_json::from_str::<serde_json::Value>(&posts_json).unwrap(),
    });
    let jinja_ns = bench(|| template.render(minijinja::value::Value::from_serialize(&jinja_ctx)));

    // Askama (compile-time templates).
    #[derive(askama::Template)]
    #[template(
        source = "{% if user_logged_in %}<p>Hello {{ user_name }}</p>{% else %}<p>Hello guest</p>{% endif %}\n{% for post in posts %}<article><h2>{{ post.title }}</h2><p>{{ post.body }}</p></article>{% endfor %}\n",
        ext = "txt"
    )]
    struct Page<'a> {
        user_logged_in: bool,
        user_name: &'a str,
        posts: &'a [Post<'a>],
    }
    struct Post<'a> {
        title: &'a str,
        body: &'a str,
    }
    let posts: Vec<Post> = (0..10)
        .map(|i| Post {
            title: Box::leak(format!("Post {i}").into_boxed_str()),
            body: "Some body text for the post.",
        })
        .collect();
    let page = Page {
        user_logged_in: true,
        user_name: "Ada",
        posts: &posts,
    };
    let askama_ns = bench(|| askama::Template::render(&page));

    println!("nift-rs     {:>8.0} ns/render", nift_ns);
    println!("tera        {:>8.0} ns/render", tera_ns);
    println!("minijinja   {:>8.0} ns/render", jinja_ns);
    println!("askama      {:>8.0} ns/render", askama_ns);

    // C++ Engine comparison (same Nift template), when the bench is built.
    let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cpp_bench = std::env::var("NIFT_CPP_BENCH")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let p = crate_dir.join("../../../nift-embed/.build/engine-bench");
            if p.is_file() {
                Some(p)
            } else {
                None
            }
        });
    if let Some(cpp_bench) = cpp_bench {
        if let Ok(output) = Command::new(cpp_bench).output() {
            let text = String::from_utf8_lossy(&output.stdout);
            print!("C++ engine  {}", text);
        }
    }
}
