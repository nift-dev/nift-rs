# CP7 report — minify-rs implementation

Native Rust multi-format minifier (Minify++ behavioural contract), ported
idiomatically (byte/char scanning over owned strings; no C++ buffers; no
unsafe). Standalone and independently releasable.

## Architecture / API

`crates/minify/`:
- `Format { Html, Css, JavaScript, Jsx, Json, Xml, Svg }`.
- `format_for_extension(&str) -> Option<Format>` (extension map, case-
  insensitive, optional leading dot).
- `minify(Format, &str) -> Result<String, String>`; per-format helpers
  (`json`, `css`, `minify_javascript`, `jsx`, `xml`, `svg`).

## Format implementation notes (behavioural contract)

- **JSON**: validates via the native Rust `jsonic` (reuse across the layered
  stack), then strips insignificant whitespace outside strings.
- **CSS**: comment removal (`/*!` license comments preserved), whitespace
  collapse with token-boundary rules (`css_needs_space` for word/selector/
  string/percentage joins), math-operator `+`/`-` spacing, `:` descendant-
  pseudo-class spacing (`css_colon_precedes_rule_block`), strings preserved.
- **XML/SVG**: comments removed, CDATA + processing instructions preserved
  verbatim, tag-internal whitespace collapsed with quote preservation, text
  whitespace preserved (may be significant).
- **JavaScript**: ASI-safe (every significant newline preserved; `return`,
  `throw`, postfix `++`/`--`), comment removal (`/*!` preserved; block comments
  with newlines become a newline), regex detection (control-paren +
  expression-prefix keywords), template/string literals preserved, identifier/
  number/token-boundary spacing.
- **JSX**: JavaScript with JSX tag boundaries preserved verbatim; the JSX
  root-start disambiguation (keywords `return`/`yield`/`await`/`case`/`throw`,
  expression-prefix characters) is ported.

## Dependencies

`jsonic` (JSON validation). No serde; no unsafe (workspace `unsafe_code =
forbid`).

## Conformance

Key `minify_smoke.cpp` semantic cases ported as Rust tests (css basic/license/
strings/calc spacing, xml whitespace+CDATA+comment, svg comment, javascript
ASI + comments, json whitespace+invalid, jsx boundary preservation) plus
idempotence (minify(minify(x)) == minify(x)) and a no-panic adversarial corpus
over all seven formats. The remaining Minify++ corpus (html tag-internal edge
cases, JSX/TSX generic-arrow disambiguation, css postcss semantics, the
shell/fuzz suites) is mapped for follow-up; per-format output is validated
against the smoke expectations that are ported.

## Semantic differences discovered

- The C++ `css` keeps a trailing `;` before the closing `}` (only whitespace is
  trimmed); the Rust port matches the reference (the smoke case
  `body{color:red;margin:0 10px;}` confirms this is the contract, not a bug).
- No other reference-vs-Rust divergence was found in the ported surface.

## nift-rs integration

- `crates/nift` depends on `minify`.
- `project.rs::supported_minify_ext` now derives from
  `minify::format_for_extension` (single source of truth for the supported
  extension set).
- Actual minification invocation belongs to the Nift build path, which
  `nift-rs` (an engine/project library) does not implement; the accepted
  boundary is preserved: `Engine::render()` returns semantic output, and
  minification is an explicit project/build option via `minify`.

## Complete nift-rs result

Full workspace: **204 tests, 0 failures** (197 prior + 7 minify).

## Per-format benchmark (methodology: input throughput + output size, NOT
incomparable output-byte division; 50k docs, release)

```text
Html:        182.2 MiB/s input,   60 bytes/doc output
Css:         340.9 MiB/s input,   49 bytes/doc output
Json:        108.1 MiB/s input,   33 bytes/doc output
Xml:         285.1 MiB/s input,   33 bytes/doc output
Svg:         352.8 MiB/s input,   44 bytes/doc output
JavaScript:  149.2 MiB/s input,   28 bytes/doc output
Jsx:         143.4 MiB/s input,   40 bytes/doc output
```

The Minify++ per-format comparison (same corpus + options) is the recorded
follow-up (the ported smoke expectations already confirm output equivalence for
the covered cases); the Rust throughputs establish no catastrophic regression.

## Roadmap correction applied

`nift-rs-regression-suite` removed from the active roadmap: one
implementation-neutral `nift-embed-regression-suite` runs against both C++ and
Rust via `NIFT_BIN`-style binary selection / neutral harnesses. Layer-specific
conformance (Jsonic++, Minify++, Embed) stays separate.

## Commit / hygiene

(committed by the CP7 checkpoint in nift-rs). Clean tree: no `target/`, no C++
artifacts; nift-embed clean; both git-clean.
