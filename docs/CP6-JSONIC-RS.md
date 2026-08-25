# CP6 report — jsonic-rs implementation

Native Rust JSON implementation (Jsonic++ behavioural contract). The nift-rs
private parser/value was extracted into a standalone `crates/jsonic/` crate and
nift-rs now consumes it.

## jsonic-rs architecture / API

`crates/jsonic/` (standalone releasable library):
- `Value` (Null/Bool/Number(f64)/String/Array/Object with
  `indexmap::IndexMap` insertion order), `ValueError` — moved from nift's
  private `value.rs` (its API preserved verbatim; it was already idiomatic Rust).
- `parse_json`/`parse` (`&str -> Result<Value, String>`), `parse_bytes`
  (`&[u8]`), `stringify` (canonical compact JSON), `validate`/`validate_schema`
  (JSON Schema subset). `parse`/`validate` are idiomatic aliases;
  `parse_json`/`validate_schema` are Nift's historical names, re-exported for
  the nift migration.

## Dependencies

`indexmap` (insertion-ordered objects - the contract requires it) and `regex`
(JSON Schema `pattern` keyword). No serde_json parser; no unsafe (workspace
`unsafe_code = forbid`).

## Semantic differences / conformance gaps discovered (and fixed)

Porting exposed three genuine parser gaps versus the C++ Jsonic++ reference,
each fixed to match the reference exactly:
- **Leading zero**: the port accepted `01`; the reference rejects it
  (`fail("leading zero in JSON number")`). Fixed.
- **Non-finite numbers**: the port accepted `1e999` (infinity); the reference
  rejects it (`fail("JSON number is outside the supported finite range")`).
  Fixed, including fraction/exponent digit requirements (`1.`, `.5`, `1e`
  rejected).
- **Surrogate pairs**: the port rejected `\ud83d\ude00` (a lone `char` cannot
  hold a surrogate); the reference combines high+low surrogates into the astral
  codepoint and rejects a lone low surrogate / a high surrogate not followed by
  a low surrogate. Fixed to the reference logic.

No ambiguity required an architectural stop: all three had a clear C++ reference
behavior that affects Nift JSON compatibility, so they were matched directly.

## Entity decision

`entity` (short marker -> HTML entity) is a Nift/template rendering concern and
stays in nift (`crate::json::entity`), not in the general JSON library.

## Nift migration

- `crates/nift/src/value.rs` -> thin `pub use jsonic::{Value, ValueError};`.
- `crates/nift/src/json.rs` -> keeps `entity` + re-exports
  `jsonic::{parse_json, validate_schema}`; the private parser/validator are
  gone (no duplicate JSON parser remains).
- nift depends on `jsonic = { path = "../jsonic" }`.

## Tests + counts

- jsonic: 14 lib tests (carried from value.rs/json.rs) + 15 integration tests
  (`tests/jsonic.rs`): valid corpus, rejection corpus, duplicate keys,
  insertion order, escapes, Unicode + surrogate pairs, number boundaries
  (incl. non-finite rejection), nesting, empty structures, canonical
  stringify, parse->stringify->parse round-trips, escaping, schema
  accept/reject, and an adversarial no-panic corpus.
- Full workspace: **197 tests, 0 failures** (all nift suites incl.
  `corpus_parity_pages_match_goldens`, `repair_parity`, `nr*` suites).

## Conformance results

The Jsonic++ corpora (`json_smoke`, `json_adversarial`) are ported as
contract-equivalence tests (the C++ lifetime/memory cases translate to the
underlying guarantee: no panics on arbitrary input). The three fixed parser
gaps are directly evidenced. The full nift C++-Embed conformance corpus passes
against the migrated nift-rs, so replacing the private machinery caused no Nift
semantic regression.

## Benchmark (methodology + results)

Same representative 190-byte JSON document, 50,000 parse+serialize iterations.
C++ Jsonic++: `-O2`, `json::Document::parse` + `dump(0)`. Rust jsonic:
`--release`, `jsonic::parse` + `stringify`.

```text
Jsonic++ (release, -O2): 90.0 MiB/s (parse + dump(0))
jsonic-rs (release):     67.7 MiB/s (parse + stringify)   ~1.33x slower
```

Comparable order of magnitude; no large unexplained regression. (Debug-mode
jsonic is ~10 MiB/s, which is why the comparison must use release builds.)

## Commit / hygiene

(committed by the CP6 checkpoint in nift-rs). Clean tree: no `target/`, no
C++ artifacts (the benchmark harness was built in /tmp and removed);
nift-embed clean (no binaries/objects/.build); both git-clean.
