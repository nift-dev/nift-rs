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

## Conformance — differential gate (126/126 byte-for-byte)

`scripts/minify_differential.py` (permanent gate) sends the same 126 corpus
inputs (every `minify_smoke.cpp` semantic case, plus malformed/adversarial
inputs) through BOTH the C++ reference (`/tmp/minify_cpp`, built from
minifypp/Minify.cpp) and minify-rs (a CLI example), comparing output
byte-for-byte. **126/126 cases match.**

Porting against the full corpus exposed and fixed six real gaps in the first
pass (this is why the full corpus matters):
1. JSX was copying regions verbatim instead of recursively minifying
   `{...}` expressions and preserving JSX text (`https://` was being treated
   as a JS comment). Replaced with a full JSX region processor (tags,
   attribute-brace expressions, nested JSX, fragments, self-closing tags,
   TSX-generic-arrow disambiguation).
2. JS `can_start_regex` was wrong after operators (`=`, `:`, etc.) and blocks;
   regex flags (`/[/]/g`) were dropped. Ported the block-brace stack and the
   operator/brace can_start_regex rules.
3. JS/HTML/XML/CSS non-ASCII bytes were pushed as single chars, corrupting
   UTF-8 (e.g. `café`). Now copies full characters; `word_char` returns true
   for `>= 0x80` so identifier-adjacent whitespace is preserved.
4. HTML/XML whitespace emission indexed the INPUT by the output length
   (`bytes[output.len()-1]`) instead of the output's last byte, dropping
   inter-element/inside-tag spaces. Fixed.
5. JS label `label:{}` block classification used the wrong statement-boundary
   check (looked at the char before `:` instead of before the identifier).
   Fixed.
6. CSS `css_needs_space`/string handling for non-ASCII. Fixed.

Semantic difference confirmed (NOT a bug): CSS keeps a trailing `;` before the
closing `}` (only whitespace is trimmed) - the smoke case
`body{color:red;margin:0 10px;}` confirms this is the reference contract.

## Additional evidence

- Idempotence: minify(minify(x)) == minify(x) over all seven formats.
- No-panic adversarial corpus over all seven formats.
- The minify_smoke.cpp idempotence cases (json/css/html/js/jsx/xml/svg) pass.

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

Full workspace: **205 tests, 0 failures** (197 prior + 8 minify incl. the
per-format benchmark test).

## C++ vs Rust per-format benchmark (in-process, same inputs, same options,
release builds, 200k iterations)

```text
format   C++ input MiB/s   Rust input MiB/s   ratio (Rust/C++)   output bytes/iter (identical)
Html        188.2              229.1            1.22x faster          60
Css         264.0              303.6            1.15x faster          49
Json         91.0              115.7            1.27x faster          33
Xml         140.2              300.1            2.14x faster          33
Svg         268.4              366.1            1.36x faster          44
JavaScript  104.8              216.1            2.06x faster          28
Jsx         117.2              235.3            2.01x faster          44
```

Same per-format inputs/options; output sizes are byte-identical between
implementations, so the input-throughput comparison is apples-to-apples. Rust
is faster on every format (no large unexplained regression - the reverse).

## Roadmap correction applied

`nift-rs-regression-suite` removed from the active roadmap: one
implementation-neutral `nift-embed-regression-suite` runs against both C++ and
Rust via `NIFT_BIN`-style binary selection / neutral harnesses. Layer-specific
conformance (Jsonic++, Minify++, Embed) stays separate.

## Commit / hygiene

(committed by the CP7 checkpoint in nift-rs). Clean tree: no `target/`, no C++
artifacts; nift-embed clean; both git-clean.
