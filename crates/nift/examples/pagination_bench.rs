//! CP8 pagination benchmark: nift-rs Engine ns/render for a tracked paginated
//! page (3 pages, 3 items), mirroring nift-embed tests/engine_pagination_bench.cpp
//! (same fixture, same render, same median metric). Renders the primary page
//! via the project-aware render -- the render that also assembles the complete
//! pagination set. Prints the median "<median> ns/render\n" on stdout.

use nift::{Engine, RenderResult};
use std::time::Instant;

fn write_file(path: &std::path::Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(path, contents).expect("write file");
}

fn main() {
    let samples: usize = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(7);
    let iterations: u32 = 5000;
    let warmup: u32 = 200;

    let root = std::env::temp_dir().join("nift-rs-pg-bench");
    let _ = std::fs::remove_dir_all(&root);
    write_file(
        &root.join(".nift/config.json"),
        r#"{"config":{"content-dir":"content/","output-dir":"public/","default-template":"templates/template.html","incremental-mode":"modified"}}"#,
    );
    write_file(
        &root.join(".nift/tracked.json"),
        r#"{"tracked":[{"name":"blog","title":"Blog","template":"templates/template.html","paginate":{"items-per-page":1}}]}"#,
    );
    write_file(
        &root.join("templates/template.html"),
        "<main>$[title]</main>\n@content",
    );
    write_file(
        &root.join("content/blog.html"),
        "@item{one}@item{two}@item{three}@paginate",
    );
    write_file(
        &root.join("content/blog.paginate.html"),
        "<section>page $[paginate.current]/$[paginate.total]:[$[paginate.items]]</section>",
    );

    let engine = Engine::project(&root);
    let mut times: Vec<u128> = Vec::with_capacity(samples);
    for s in 0..samples {
        let count = if s == 0 { warmup } else { iterations };
        let start = Instant::now();
        for _ in 0..count {
            let result: RenderResult = engine
                .render_page("blog", &nift::context::Context::new())
                .expect("render");
            assert_eq!(result.pagination.len(), 2);
        }
        if s != 0 {
            times.push(start.elapsed().as_nanos() / u128::from(count));
        }
    }
    times.sort_unstable();
    println!("{} ns/render", times[times.len() / 2]);
}
