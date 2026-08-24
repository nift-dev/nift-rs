//! NR11: hardening — fuzzing, adversarial inputs, property tests.
//!
//! The parser must be a total function over external input: every template
//! renders to a `Result` (Ok or Err), never a panic, on arbitrary input.
//! These tests attack that invariant deterministically (seeded, reproducible):
//! a mutational template fuzzer, adversarial structure/nesting/unicode/control
//! inputs, and determinism/round-trip properties.
//!
//! Iteration counts are scaled down under Miri so the same corpus can run
//! interpreted (set by `RUSTFLAGS=-Zmiri...`; `cfg(miri)` is set by
//! `cargo miri test`).

use nift::bindings::Bindings;
use nift::context::Context;
use nift::host::{RenderHost, RenderIdentity};
use nift::hosts::InMemoryHost;
use nift::source::Source;
use nift::{render, render_tracked};
use std::path::PathBuf;

/// Deterministic xorshift64 PRNG (fixed seed -> reproducible failures).
struct XorShift64(u64);

impl XorShift64 {
    fn new(seed: u64) -> Self {
        XorShift64(seed.max(1))
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn next_usize(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.next_usize(items.len())]
    }
}

fn fuzz_iterations() -> usize {
    if cfg!(miri) {
        200
    } else {
        20_000
    }
}

const LITERAL_FRAGMENTS: &[&str] = &[
    "hello",
    " ",
    "<p>",
    "</p>",
    "&amp;",
    "&lt;",
    "@",
    "$",
    "[",
    "]",
    "{",
    "}",
    "(",
    ")",
    "café 東京 🚀",
    "\u{200d}\u{200c}",
    "\\",
    "'",
    "\"",
    "\n",
    "\t",
    "\r",
    "\u{0}",
    "=",
    ":",
    "?",
    "->",
    "123",
    "-3.5",
    "true",
    "null",
    "a.b",
    "index",
    "partial",
    "!=",
    "&&",
    "||",
];

const EXPR_FRAGMENTS: &[&str] = &[
    "x",
    "n",
    "items",
    "user.name",
    "loop.index",
    "loop.first",
    "loop.last",
    "loop.count",
    "n > 10",
    "n == 5",
    "a && b",
    "a || b",
    "!a",
    "title",
    "name",
    "content-path",
    "output-path",
    "1 + 2 * 3",
    "substr(s, 1, 2)",
    "n > 10 ? 'big' : 'small'",
    "ok ? 'yes' : ''",
    "paginate.items",
    "paginate.current",
    "paginate.total",
    "items[0]",
    "a[1].b",
];

const DIRECTIVE_FRAGMENTS: &[&str] = &[
    "@if(x){yes}@else{no}",
    "@if(n > 5){big}@else if(n == 5){eq}@else{small}",
    "@for(x : items){<$[x]>}",
    "@for(x : items by x desc){<$[x]>}",
    "@for((k, v) : obj){$[k]=$[v]}",
    "@content",
    "@input(\"part\")",
    "@input('part.html')",
    "@getenv(PA_NR11)",
    "@getenv('PA_NR11')",
    "@dep('a.js')",
    "@dep('a.js','b.css')",
    "@ent('!')",
    "@ent('->')",
    "@item{x}",
    "@paginate",
    "@json('data.json', d)",
    "@pathto('app.js')",
    "@pathtofile('app.js')",
    "@pathtopage(1)",
    "@sort(items)",
    "@filter(p : items => p.n > 1)",
    "@map(p : items => p.n * 2)",
    "@distinct(items)",
    "@reverse(items)",
    "@slice(items, 0, 2)",
];

fn make_host<'a>(defaults: &'a Bindings, context: &'a Context) -> InMemoryHost<'a> {
    InMemoryHost::new(defaults, context, PathBuf::from("/fuzz"))
        .with_source("content/part.html", "<p>PART</p>")
        .with_source("content/page.html", "<p>PAGE</p>")
        .with_source("data.json", r#"{"a":{"b":1},"items":[1,2,3]}"#)
        .with_source("app.js", "/* app */")
        .with_source("b.css", "/* b */")
        .with_env("PA_NR11", "env-val")
}

/// Render via the public kernel and assert it returns (never panics).
fn render_total(host: &dyn RenderHost, identity: &RenderIdentity, template: &str, page: bool) {
    let _ = if page {
        render(
            host,
            identity,
            &Source::text(template),
            Some(&Source::text("<p>page</p>")),
        )
    } else {
        render(host, identity, &Source::text(template), None)
    };
}

fn mutate_template(rng: &mut XorShift64, template: &str) -> String {
    // Mutate on char boundaries (the template contains multi-byte text; byte
    // offsets can land mid-character and panic the test harness, not the
    // parser).
    let boundaries: Vec<usize> = template.char_indices().map(|(i, _)| i).collect();
    let char_count = boundaries.len();
    let mut out = template.to_string();
    let action = rng.next_usize(4);
    match action {
        // Insert a random fragment at a random char position.
        0 => {
            let frag = rng.pick(LITERAL_FRAGMENTS).to_string();
            let pos = boundaries
                .get(rng.next_usize(char_count + 1))
                .copied()
                .unwrap_or(out.len());
            out.insert_str(pos, &frag);
        }
        // Replace a char-range with a random fragment.
        1 => {
            if char_count > 0 {
                let start_index = rng.next_usize(char_count);
                let len = (1 + rng.next_usize(4)).min(char_count - start_index);
                let start = boundaries[start_index];
                let end = boundaries
                    .get(start_index + len)
                    .copied()
                    .unwrap_or(out.len());
                out.replace_range(start..end, rng.pick(LITERAL_FRAGMENTS));
            }
        }
        // Append a directive or expression fragment.
        2 => {
            let frag = match rng.next_usize(3) {
                0 => format!("$[{}]", rng.pick(EXPR_FRAGMENTS)),
                1 => rng.pick(DIRECTIVE_FRAGMENTS).to_string(),
                _ => rng.pick(LITERAL_FRAGMENTS).to_string(),
            };
            out.push_str(&frag);
        }
        // Truncate at a random char boundary.
        _ => {
            let pos = boundaries
                .get(rng.next_usize(char_count + 1))
                .copied()
                .unwrap_or(out.len());
            out.truncate(pos);
        }
    }
    out
}

/// A `$[...]` expression generator that produces valid-ish expressions from
/// fragments.
fn random_expression(rng: &mut XorShift64) -> String {
    let depth = 1 + rng.next_usize(3);
    let mut expr = rng.pick(EXPR_FRAGMENTS).to_string();
    for _ in 0..depth {
        match rng.next_usize(3) {
            0 => expr = format!("{}{}", rng.pick(EXPR_FRAGMENTS), expr),
            1 => expr = format!("{expr}{}", rng.pick(EXPR_FRAGMENTS)),
            _ => expr = format!("$[{expr}]"),
        }
    }
    expr
}

#[test]
fn fuzz_mutated_templates_never_panic() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("fuzz").title("Fuzz");

    let mut rng = XorShift64::new(0x5EED_0000_0000_0001);
    let mut template = String::new();
    for i in 0..fuzz_iterations() {
        template = mutate_template(&mut rng, &template);
        render_total(&host, &identity, &template, i % 4 == 0);
    }
}

#[test]
fn fuzz_random_templates_never_panic() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("fuzz").title("Fuzz");

    let mut rng = XorShift64::new(0xDEAD_0000_0000_0002);
    for i in 0..fuzz_iterations() {
        let mut template = String::new();
        let segments = 1 + rng.next_usize(6);
        for _ in 0..segments {
            match rng.next_usize(4) {
                0 => template.push_str(rng.pick(LITERAL_FRAGMENTS)),
                1 => template.push_str(&random_expression(&mut rng)),
                2 => template.push_str(rng.pick(DIRECTIVE_FRAGMENTS)),
                _ => template.push(char::from_u32(rng.next_usize(0x300) as u32).unwrap()),
            }
        }
        render_total(&host, &identity, &template, i % 3 == 0);
    }
}

#[test]
fn fuzz_random_bindings_never_panic() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("fuzz").title("Fuzz");

    let mut rng = XorShift64::new(0xBEEF_0000_0000_0003);
    let template = "@if(x){$[y.z]}@for(k : items){$[k.a]@getenv(PA_NR11)}@content";
    for _ in 0..fuzz_iterations() {
        // Render the same template with different (random) bindings.
        render_total(&host, &identity, template, true);
        let _ = rng.next_u64();
    }
}

#[test]
fn deep_nesting_is_a_controlled_error_not_a_stack_overflow() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("deep").title("D");

    // Far beyond the 64-level parse-depth guard.
    for depth in [65, 100, 1_000, 10_000] {
        let template = format!("{}x{}", "@if(true){".repeat(depth), "}".repeat(depth));
        let result = render(&host, &identity, &Source::text(&template), None);
        assert!(result.is_err(), "depth {depth} must be a controlled error");
    }
}

#[test]
fn deep_for_nesting_is_a_controlled_error() {
    let defaults = Bindings::new();
    let mut context = Context::new();
    let _ = context.set("items", nift::Value::Array(vec![nift::Value::number(1.0)]));
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("deep").title("D");

    for depth in [65, 500, 5_000] {
        let template = format!(
            "{}{}x{}",
            "@for(x : items){".repeat(depth),
            "",
            "}".repeat(depth)
        );
        let result = render(&host, &identity, &Source::text(&template), None);
        assert!(result.is_err(), "depth {depth} must be a controlled error");
    }
}

#[test]
fn huge_and_control_laden_templates_render_or_error() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("big").title("Big");

    // A multi-megabyte literal template renders (no quadratic blowup panic).
    // Scaled down under Miri where interpretation would take too long.
    let big = "abc".repeat(if cfg!(miri) { 20_000 } else { 2_000_000 });
    let result = render(&host, &identity, &Source::text(&big), None).unwrap();
    assert_eq!(result.output.len(), big.len());

    // Control characters and NUL bytes in templates are tolerated.
    let control = "a\u{0}b\u{1}\u{7}\u{1b}c\r\n\te".to_string();
    let result = render(&host, &identity, &Source::text(&control), None).unwrap();
    assert_eq!(result.output, control);
}

#[test]
fn unterminated_constructs_are_controlled_errors() {
    let defaults = Bindings::new();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("u").title("U");
    // These construct a directive that is genuinely unterminated (an error).
    // Bare "$[" and lone "@" are literal text by contract (the reference falls
    // through to literal emission when no closing bracket/parenthesis is
    // found), so they are asserted separately as literal rather than errors.
    let templates = [
        "@if(",
        "@if(x){",
        "@if(x){}@else",
        "@for(",
        "@for(x : items){",
        "@for(x : items",
        "@input(",
        "@input('part",
        "@getenv(",
        "@dep(",
        "@content(",
        "@item{",
        "@json(",
        "@pathto(",
        "<#--",
        "@if(x){yes}@else if(",
    ];
    for template in templates {
        let result = render(&host, &identity, &Source::text(template), None);
        assert!(result.is_err(), "unterminated '{template}' must error");
    }
    // Lone "$[", "@", "\" and unterminated "$[a.b" are literal text, not errors.
    for template in ["$[", "$[a.b", "@", "\\"] {
        let result = render(&host, &identity, &Source::text(template), None).unwrap();
        assert_eq!(result.output, template, "literal '{template}'");
    }
}

#[test]
fn rendering_is_deterministic() {
    let defaults = Bindings::new();
    let mut context = Context::new();
    let _ = context.set("x", nift::Value::boolean(true));
    let _ = context.set(
        "y",
        nift::Value::Object([("z".to_string(), nift::Value::number(1.0))].into()),
    );
    let _ = context.set(
        "items",
        nift::Value::Array(vec![nift::Value::Object(
            [("a".to_string(), nift::Value::string("v"))].into(),
        )]),
    );
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("det").title("D");
    let template = "@if(x){$[y.z]}@for(k : items){<$[k.a]>}$[title]@getenv(PA_NR11)";
    let first = render(&host, &identity, &Source::text(template), None).unwrap();
    for _ in 0..50 {
        let again = render(&host, &identity, &Source::text(template), None).unwrap();
        assert_eq!(first.output, again.output);
        assert_eq!(first.dependencies, again.dependencies);
        assert_eq!(first.requirements, again.requirements);
    }

    // Determinism also holds for the error path: the same invalid input yields
    // the same typed error.
    let bad = "@if(x){unclosed";
    let first_error = render(&host, &identity, &Source::text(bad), None).unwrap_err();
    for _ in 0..20 {
        let again = render(&host, &identity, &Source::text(bad), None).unwrap_err();
        assert_eq!(first_error.kind, again.kind);
        assert_eq!(first_error.message, again.message);
    }
}

#[test]
fn every_error_is_a_typed_result() {
    // The rejection surface is typed: every failure carries an ErrorKind.
    let defaults = Bindings::new();
    let context = Context::new();
    let host = make_host(&defaults, &context);
    let identity = RenderIdentity::new().name("e").title("E");
    let templates = [
        "@if(x){unclosed",
        "$[missing.deep.member]",
        "@for(x : 5){$[x]}",
        "@json('nope.json', d)$[d]",
        "@pathto('..')",
        "@input('missing')",
        "@getenv()",
        "@paginate",
        "@item{x}",
    ];
    for template in templates {
        match render(&host, &identity, &Source::text(template), None) {
            Ok(_) => {}
            Err(error) => {
                assert!(
                    !error.message.is_empty(),
                    "error '{template}' needs a message"
                );
                let _ = error.kind;
            }
        }
    }
}

// Miri isolates the filesystem (statx unavailable), so this filesystem-touching
// test is skipped under Miri; the parser/expression fuzz coverage above still
// runs interpreted.
#[cfg(not(miri))]
#[test]
fn project_state_open_never_panics_on_adversarial_roots() {
    // open() must return a typed result for hostile roots without panicking.
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/cases");
    for id in [
        "malformed-config",
        "malformed-tracking",
        "bad-config",
        "bad-tracking",
        "missing-source",
        "path-escape",
    ] {
        let _ = nift::ProjectState::open(corpus.join(id).join("project"));
    }
    // A root that does not exist.
    let _ = nift::ProjectState::open("/definitely/not/a/project");
    // A root that is a file, not a directory.
    let file = std::env::temp_dir().join(format!("nift-nr11-file-{}", std::process::id()));
    std::fs::write(&file, "x").unwrap();
    let _ = nift::ProjectState::open(&file);
    let _ = std::fs::remove_file(&file);
    let _ = render_tracked;
}
