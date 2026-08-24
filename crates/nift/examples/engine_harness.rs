//! NR6 differential harness (Rust side): mirrors the frozen C++ standalone
//! Engine harness CLI and prints a stable observable JSON result
//! (output/dependencies/requirements or error) for comparison. Run by
//! tests/nr6_differential.sh against the C++ harness built from nift-embed.

use nift::context::Context;
use nift::{Engine, Source};
use std::io::{self, BufRead};

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

    let mut engine = Engine::new();
    engine.set_root(root);
    for line in io::stdin().lock().lines() {
        let line = line.expect("stdin line");
        if line.is_empty() {
            continue;
        }
        if let Some(eq) = line.find('=') {
            engine
                .set(line[..eq].to_string(), line[eq + 1..].to_string())
                .ok();
        }
    }

    let mut context = Context::new();
    if !page_name.is_empty() {
        context.set_page_name(page_name);
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

    let result = if mode == "partial" {
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
            println!(
                "{{\"ok\":true,\"output\":\"{}\",\"dependencies\":[{}],\"requirements\":[{}]}}",
                json_escape(&result.output),
                deps,
                reqs
            );
        }
        Err(error) => {
            println!(
                "{{\"ok\":false,\"error\":\"{}\"}}",
                json_escape(&error.message)
            );
        }
    }
}
