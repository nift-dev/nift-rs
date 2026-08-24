//! Server-side rendering demo (NR12 DX).
//!
//! Renders a project-aware page by tracked name through the public `Engine`
//! API. Run with a corpus project:
//!
//! ```text
//! cargo run --example ssr_demo -- ../../corpus/cases/comprehensive/project about
//! ```

use nift::context::Context;
use nift::Engine;
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let default_project =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/cases/comprehensive/project");
    let project = args.next().map(PathBuf::from).unwrap_or(default_project);
    let page = args.next().unwrap_or_else(|| "about".to_string());

    let engine = Engine::open(&project).expect("open the Nift project");
    let context = Context::new();
    match engine.render_page(&page, &context) {
        Ok(result) => {
            println!("{page}:");
            println!("{}", result.output);
            println!("dependencies: {:?}", result.dependencies);
            println!("requirements: {:?}", result.requirements);
        }
        Err(error) => {
            eprintln!("render failed: {} ({})", error.message, error.kind);
            std::process::exit(1);
        }
    }
}
