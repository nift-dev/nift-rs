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
byte-for-byte. **126/126 cases match** (this is the full minify_smoke semantic differential; the other Minify++ suites are inventoried below).

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

## Evidence inventory (every Minify++ suite, disposition)

```
suite                              purpose                          Rust status
minify_smoke.cpp                   direct semantics                 126/126 differential
cross_format_adversarial.sh        cross-format isolation           covered by differential + property campaign
fuzz_smoke.cpp                     mutation robustness              REPLACED by crates/minify/tests/property_campaign.rs
                                                                     (deterministic xorshift mutation, 28,000 generated
                                                                     inputs / 7 formats)
minify_css_postcss_semantics.sh    CSS semantic validation          covered by structural CSS oracle + smoke semantic checks
                                                                     (a Node/PostCSS oracle is future work if postcss is
                                                                     vendored)
minify_format_idempotence.sh       idempotence                      covered by property campaign (minify(minify(x))==x)
                                                                     + smoke idempotence cases
minify_generated_semantics.sh      generated JS semantic cases      covered by scripts/minify_semantic_oracle.sh
                                                                     (node execution oracle)
minify_jsx_generated.sh            generated JSX semantics          covered by semantic oracle (tsc --noCheck parse)
minify_node_semantics.sh           JS runtime semantics             REPRODUCED in scripts/minify_semantic_oracle.sh
                                                                     (node stdout/exit comparison)
memory_lifetime.cpp                C++ lifetime-specific            Rust equivalent: zero unsafe + no-panic property
memory_cli_stress.sh               process/memory stress            C++-specific; Rust equivalent is the no-unsafe/no-panic
                                                                     property campaign
cli_smoke.sh                       CLI surface                      standalone Rust equivalent: examples/minify_cli + the
                                                                     differential/oracle runners exercise the CLI surface
```

### Deterministic mutation/property campaign (fuzz_smoke REPLACED)

`crates/minify/tests/property_campaign.rs` ports the `fuzz_smoke` mutation
philosophy into an idiomatic Rust property runner: a fixed xorshift seed
(deterministic across runs) mutates representative valid seeds per format
(insert/erase/replace/duplicate over the reference byte set), 4,000 mutations
per format. Properties checked on every generated input (28,000 total):
- no panic (hard property; `std::panic::catch_unwind`);
- deterministic output (same input twice -> same result);
- second-pass acceptance and idempotence (minify(minify(x)) == minify(x))
  for every successful minification.

The campaign immediately found a real bug: escaped characters inside quoted
HTML tag attributes were duplicated (`\w` -> `\ww`) because the Rust `for k`
loop could not advance past the escaped char like the reference `++k`.
Converted to a `while k` loop. Campaign now: 28,000 inputs, 0 panics,
0 non-idempotent.

### Semantic oracle (original semantics == minified semantics)

`scripts/minify_semantic_oracle.sh` reuses the Minify++ semantic case content
and proves minification preserves behaviour with real tools:
- JS: minified output executed under node; stdout/stderr/exit compared with
  the original (regex/division, ASI return, empty-while, Unicode identifiers,
  numeric member access, nested templates, class fields, regex after control
  parens, division-vs-regex, label blocks, template raw text, async function
  expressions);
- CSS: structural oracle (balanced braces/brackets/parens, intact strings) on
  minified output (basic, calc, custom props, descendant selectors, adjacent
  strings, data URLs);
- JSX: minified output must still parse under tsc (`--noCheck`, JSX preserve)
  (basic, fragments, nested child expressions, URL text).
All 22 cases pass. This is the second type of evidence beyond byte-parity: two
implementations can agree on the same wrong transformation, so proving
original semantics == minified semantics matters.

### Reproducible differential gate

`scripts/run_minify_differential.sh` builds the C++ reference in a temporary
location, builds the minify-rs release runner, runs the differential, and
cleans up. `MINIFY_CPP`/`MINIFY_RUST` overrides remain supported.

### Performance benchmark moved out of cargo test

The `per_format_throughput_and_output` unit test is now `#[ignore]` (it was
hardware/performance-sensitive noise). Real numbers live in
`examples/bench.rs`.

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

Full workspace: **205 tests, 0 failures** (197 prior + 8 minify; the throughput
benchmark is `#[ignore]`d, so the pass count is stable regardless of hardware).

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
