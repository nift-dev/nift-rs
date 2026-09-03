# nift

An independent Rust implementation of the Nift template language and
project-aware rendering engine.

```toml
[dependencies]
nift = "0.1"
```

Render a Nift template from a project by tracked page name:

```rust
use nift::context::Context;
use nift::Engine;

let engine = Engine::open("path/to/project")?;
let result = engine.render_page("about", &Context::new())?;
println!("{}", result.output);
```

Or render standalone sources:

```rust
use nift::context::Context;
use nift::source::Source;
use nift::Engine;

let mut engine = Engine::new();
engine.set_json("user", r#"{"name":"Ada"}"#)?;
let result = engine.render(
    &Source::text("<p>Hello $[user.name]</p>"),
    &Source::text("@content"),
    &Context::new(),
)?;
```

## What this crate provides

- `Value` / `Context` / `Source` / `Engine` with a typed `RenderError`
  (`ErrorKind`) and `RenderResult` (output, dependencies, requirements).
- The full Nift template surface: `$[...]` expressions, `@if`/`@else if`/
  `@else`, `@for` over arrays and objects with loop metadata and sorting,
  collection directives, `@content`, `@input`, `@json` + JSON Schema,
  `@getenv`, `@dep`, `@ent`, `@pathto`/`@pathtofile`/`@pathtopage`, and
  primary pagination for tracked pages.
- Project-aware rendering: immutable project snapshots, contracts,
  transactional `reload` with last-good retention, and concurrent renders.
- Zero `unsafe` (`#![forbid(unsafe_code)]`).

The crate reads existing Nift projects but does not implement the `nift init`
or build CLI. The shared CLI terminology contract calls tracked outputs
**files**, so a future Rust build command must report summaries such as
`3 files built successfully`, and status must describe `tracked files` rather
than `tracked pages`. Current
project conventions keep ordinary CSS, JavaScript and other static
assets directly in the configured output tree without tracked entries. The Rust
engine continues to support deliberately tracked/generated text assets because
that remains part of the project format and rendering contract.

## Conformance

This is an independent implementation of the Nift semantic contract, gated by
the canonical conformance corpus and a cross-implementation differential
battery (see `docs/cold-review.md` in the repository for the full evidence
matrix). It is not a wrapper around the C++ implementation.

## Examples

- `cargo run --example ssr_demo` — project-aware server-side rendering.
- `cargo run --release --example bench` — NR12 comparison benchmarks.
