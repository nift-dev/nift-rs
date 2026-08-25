//! NR6 differential harness (Rust side): mirrors the frozen C++ standalone
//! Engine harness CLI and prints a stable observable JSON result
//! (output/dependencies/requirements or error) for comparison. Run by
//! tests/nr6_differential.sh against the C++ harness built from nift-embed.

use nift::context::Context;
use nift::{Engine, Source};
use std::collections::BTreeSet;
use std::io::{self, BufRead};
use std::sync::{Arc, Mutex};

fn json_escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 8 {
        eprintln!("usage: engine_harness root page template page_name current_output page_path template_path");
        std::process::exit(2);
    }
    let root = args[1].clone();
    let page_text = if args[2] == "-" {
        String::new()
    } else {
        args[2].clone()
    };
    let template_text = if args[3] == "-" {
        String::new()
    } else {
        args[3].clone()
    };
    let page_name = if args[4] == "-" {
        String::new()
    } else {
        args[4].clone()
    };
    let current_output = if args[5] == "-" {
        String::new()
    } else {
        args[5].clone()
    };
    let page_path = if args[6] == "-" {
        String::new()
    } else {
        args[6].clone()
    };
    let template_path = if args[7] == "-" {
        String::new()
    } else {
        args[7].clone()
    };
    let mode = args.get(8).map(|m| m.as_str()).unwrap_or("composed");
    let seam = args.get(9).map(|s| s.as_str()).unwrap_or("-");

    let mut engine = if mode == "page" {
        // Project-aware construction: open .nift/config.json + tracked.json so
        // render_page can resolve the tracked page (NR8/NR9 lifecycle).
        Engine::project(root.clone())
    } else {
        Engine::new()
    };
    engine.set_root(root);
    let loader_keys = Arc::new(Mutex::new(BTreeSet::new()));
    if seam == "loader" {
        let keys = Arc::clone(&loader_keys);
        engine.set_loader(move |key: &str| -> Option<String> {
            keys.lock()
                .expect("loader key lock")
                .insert(key.to_string());
            if key.ends_with("/templates/template.html") {
                Some("<main>@content</main>\n".to_string())
            } else if key.ends_with("/content/blog.html") {
                Some("<p>LOADER-CONTENT</p>\n".to_string())
            } else if key.ends_with("/content/post.html") {
                Some("@input(\"part.html\")\n".to_string())
            } else if key.ends_with("/content/part.html") {
                Some("<p>LOADER-PART</p>\n".to_string())
            } else {
                None
            }
        });
    }
    if seam == "env" {
        engine.set_environment_provider(|name: &str| -> Option<String> {
            match name {
                "NIFT_ENV_A" => Some("alpha".to_string()),
                "NIFT_ENV_B" => Some("beta".to_string()),
                _ => None,
            }
        });
    }
    for line in io::stdin().lock().lines() {
        let line = line.expect("stdin line");
        if line.is_empty() {
            continue;
        }
        if let Some(eq) = line.find('=') {
            let name = line[..eq].to_string();
            let value = line[eq + 1..].to_string();
            // A "json:" prefix binds a JSON value instead of a string, so the
            // differential can exercise arrays/objects/numbers/bools (NR10).
            if let Some(rest) = value.strip_prefix("json:") {
                engine.set_json(name, rest).ok();
            } else {
                engine.set(name, value).ok();
            }
        }
    }

    let mut context = Context::new();
    if !page_name.is_empty() {
        context.set_page_name(&page_name);
    }
    if !current_output.is_empty() {
        context.set_current_output(current_output);
    }
    let page = if page_path.is_empty() {
        Source::text(page_text)
    } else {
        Source::path(page_path)
    };
    let template = if template_path.is_empty() {
        Source::text(template_text)
    } else {
        Source::path(template_path)
    };

    let result = if mode == "page" {
        // Tracked-page render (project-aware): renders the named tracked page
        // and exposes complete pagination.
        engine.render_page(&page_name, &context)
    } else if mode == "partial" {
        engine.render_partial(&page, &context)
    } else {
        engine.render(&page, &template, &context)
    };
    match result {
        Ok(result) => {
            let deps = result
                .dependencies
                .iter()
                .map(|d| format!("\"{}\"", json_escape(d)))
                .collect::<Vec<_>>()
                .join(",");
            let reqs = result
                .requirements
                .iter()
                .map(|d| format!("\"{}\"", json_escape(d)))
                .collect::<Vec<_>>()
                .join(",");
            let pages = result
                .pagination
                .iter()
                .map(|p| {
                    format!(
                        "{{\"page\":{},\"output\":\"{}\"}}",
                        p.page,
                        json_escape(&p.output)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            if seam == "loader" {
                let keys = loader_keys
                    .lock()
                    .expect("loader key lock")
                    .iter()
                    .map(|k| format!("\"{}\"", json_escape(k)))
                    .collect::<Vec<_>>()
                    .join(",");
                println!(
                    "{{\"ok\":true,\"output\":\"{}\",\"dependencies\":[{}],\"requirements\":[{}],\"pagination\":[{}],\"loaderKeys\":[{}]}}",
                    json_escape(&result.output),
                    deps,
                    reqs,
                    pages,
                    keys
                );
            } else {
                println!(
                    "{{\"ok\":true,\"output\":\"{}\",\"dependencies\":[{}],\"requirements\":[{}],\"pagination\":[{}]}}",
                    json_escape(&result.output),
                    deps,
                    reqs,
                    pages
                );
            }
        }
        Err(error) => {
            println!(
                "{{\"ok\":false,\"error\":\"{}\"}}",
                json_escape(&error.message)
            );
        }
    }
}
