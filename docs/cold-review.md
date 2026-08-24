# NR12 cold independent review checklist

This is the final cold-review checklist for the nift-rs programme (NR0–NR12).
The reviewer signs off each line with evidence from the committed repository,
not from session summaries. Every line should be independently re-checked.

## 1. Architecture

- [ ] One rendering kernel serves standalone and project-aware rendering
      through a single `RenderHost` seam; there is no second parser.
- [ ] The public surface is complete and coherent: `Value`, `Context`,
      `Source`, `Engine` (standalone + project-aware + reload), `RenderError`
      with typed `ErrorKind`, `RenderResult`.
- [ ] Project reading/discovery (`ProjectState`) is distinct from the Engine
      and performs zero project writes by construction.
- [ ] The Engine publishes immutable snapshot generations; reload is
      transactional and retains last-good on failure.
- [ ] `#![forbid(unsafe_code)]` holds in the crate and the workspace lint.

## 2. Conformance

- [ ] Every canonical parity page (comprehensive/schema/getenv) renders
      byte-identically to its golden output through the Rust Engine.
- [ ] Dependency and requirement sets match the goldens (line-identical).
- [ ] Project-state reject classes (invalid-config-json, invalid-tracking-json,
      unknown-config-key, duplicate-tracked-name) match the manifest.
- [ ] Runtime reject classes (missing-source, project-root-escape) match.
- [ ] The C++ conformance suite passes 9/9 (CLI == C++ Engine == golden).
- [ ] The expanded differential battery runs 106 cases with zero divergence
      against the C++ Engine on Linux, macOS and Windows.
- [ ] The standalone NR6 differential battery passes 16/16.

## 3. Portability

- [ ] `relative`/generic path spelling uses `/` separators on every platform.
- [ ] The corpus mirror is byte-identical on Windows checkouts (no CRLF/autocrlf
      or hash-format drift).
- [ ] Loader keys use `generic_string()` spelling (no native `\` on Windows).
- [ ] The differential harness is UTF-8-clean on Windows (wmain + binary
      stdout).
- [ ] Tracked-name validation matches platform `is_absolute` semantics.

## 4. Safety and hardening

- [ ] The parser is a total function over external input: arbitrary templates
      and bindings produce `Ok` or a typed `Err`, never a panic (fuzz suite).
- [ ] Deep nesting is a controlled error before the stack is exhausted (the
      depth guard is safe on a 1 MiB thread stack).
- [ ] Multi-byte text is handled on character boundaries everywhere the parser
      slices.
- [ ] Miri passes the lib unit tests and the hardening suite.
- [ ] ASan passes the lib unit tests.
- [ ] TSan passes the reload lifecycle suite race-free.

## 5. Reliability and lifecycle

- [ ] Concurrent `render_page` on one Engine is safe and observes complete
      snapshot generations.
- [ ] Concurrent `reload` + `reload` and `reload` + render are safe; failed
      reloads retain the last good generation.
- [ ] Engine defaults and the environment provider survive reload.
- [ ] Reload and rendering perform zero project writes.

## 6. Documentation and DX

- [ ] Crate-level rustdoc describes the full surface, conformance evidence and
      the performance note.
- [ ] `examples/ssr_demo.rs` demonstrates project-aware server-side rendering.
- [ ] `examples/bench.rs` compares nift-rs against Tera, MiniJinja, Askama and
      the C++ Engine.
- [ ] The semantic inventory and authorities documents remain accurate.

## 7. Repository hygiene

- [ ] Working tree is clean at the reviewed commit.
- [ ] The canonical corpus mirror verifies byte-identical.
- [ ] `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`
      pass on stable and MSRV 1.97.

## Sign-off

Reviewer: ____________  Date: ____________  Result: [ ] PASS  [ ] HOLD
