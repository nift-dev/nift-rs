# RUST-STACK-PLAN — jsonic-rs, minify-rs, and the nift-rs stack

Design/audit checkpoint (no implementation yet: no jsonic-rs, no minify-rs, no
pagination extension, no repair changes, no C++ modifications).

## Audit findings (from the actual repositories)

### jsonic-rs: what exists today

- `crates/nift/src/json.rs` (1069 lines) is a JSON PARSER (`struct Parser`,
  `pub fn parse_json`) that produces nift's OWN `Value` type from
  `crates/nift/src/value.rs` (425 lines: `enum Value { Null, Boolean, Number,
  String, Array, Object(IndexMap<String, Value>) }`), plus `pub fn entity`
  (HTML entity decoding) and `pub fn validate_schema` (JSON-schema subset).
- `json.rs` is NOT a general-purpose standalone JSON library: it is coupled to
  nift's `Value`. The parser is public within the crate and `pub mod json` is
  exported, but its API is shaped around nift consumption.
- Jsonic++ is vendored under `nift-embed/jsonic/` (`include/json.h`; tests:
  `json_smoke.cpp`, `json_adversarial.cpp`, `json_memory_lifetime.cpp`).
  Nift C++ uses Jsonic++ (via `Json.h`/`JsonFile.h`) for project config,
  tracked.json, contracts, and the internal `json::Document` used by bindings
  and `.info.json`.

### minify-rs: what exists today

- Minify++ is `nift-embed/minifypp/` (`Format { Html, Css, JavaScript, Jsx,
  Json, Xml, Svg }`, single `Minify.cpp` + `Json.h`).
- C++ Nift calls Minify++ ONLY from the CLI build path: `ProjectInfo.cpp:669`
  (`minify::format_for_extension`) and `:675` (`minify::run`) during build_one
  when a page should be minified. The Embed Engine does not minify.
- `nift-rs` performs NO actual minification today: only config/model support
  (`TrackedInfo.minify`, `config.minify_exts`, `supported_minify_ext`
  extension check in project.rs). Audit confirms config/model support alone is
  not minification.

## Answers to the review question sets

### JSONIC-RS

1. **What JSON functionality exists privately in nift-rs?** A full parser
   (objects/arrays/numbers/strings/booleans/null with whitespace handling),
   insertion-ordered objects (IndexMap), duplicate-key rejection, number
   parsing (int vs double), HTML entity decoding, and a JSON-schema validation
   subset — all producing `crate::value::Value`.
2. **Which parts correspond to Jsonic++ semantics?** The parser grammar,
   duplicate-key rejection, insertion-order preservation, number/string
   handling, and schema validation are the Jsonic++-derived semantics nift
   already reproduces (validated by the C++ conformance corpus).
3. **Which parts are Nift-specific and should remain in nift-rs?** The
   `Value` API surface used by bindings/expressions (number semantics, JSON
   value composition) can stay Nift-facing; the parser/schema/entity layer is
   the reusable core.
4. **Standalone Rust Jsonic public API?** `parse(&str) -> Result<Value,
   Error>`, a `Value` enum with insertion-ordered objects, duplicate-key
   rejection, number handling (int/double with a stable serialization),
   JSON output, and a `validate(schema, value)` function. Designed for Rust
   JSON users, not Nift concepts.
5. **Reusable corpora?** `jsonic/json/tests/*`: `json_smoke.cpp`,
   `json_adversarial.cpp`, `json_memory_lifetime.cpp`, plus the
   `test-jsonic`/`test-json-schema` targets and the schema/integration cases in
   nift-embed (which exercise Jsonic++ through Nift). The adversarial and
   memory-lifetime material ports directly to Rust property tests.
6. **Cross-language conformance corpus?** A `jsonic-rs` conformance harness
   comparing `jsonic-rs` output/semantics against the C++ Jsonic++ tests
   (parser round-trips, duplicate-key rejection, number formatting, schema
   accept/reject), independent of Nift rendering.
7. **External crates?** `indexmap` (already used) for insertion-ordered
   objects. No serde dependency in the core (nift avoids it deliberately);
   the public API is hand-rolled like Jsonic++'s.
8. **Extract vs redesign?** `json.rs` is a strong starting basis for the
   PARSER, but it is coupled to nift's `Value`. Recommend designing `jsonic-rs`
   with its OWN `Value` (Jsonic++-shaped), then migrating nift-rs to consume
   it — rather than extracting the coupled type. This matches "port the
   product contract, redesign the internals for Rust."

### MINIFY-RS

1. **Where does C++ call Minify++?** Only the CLI build path (build_one,
   ProjectInfo.cpp:669/:675); not the Embed Engine.
2. **Formats/features required by Nift?** The seven formats
   (Html/Css/JavaScript/Jsx/Json/Xml/Svg) via `format_for_extension` +
   `minify::run`, driven by `minify-exts` and per-page `minify` flags, with the
   minifier version stamp in `.info.json`.
3. **Does nift-rs minify today?** No — config/model support only.
4. **Standalone Rust API?** A `minify(format, input) -> Result<String,
   Error>` plus `format_for_extension(&str) -> Option<Format>`, designed as a
   natural Rust multi-format minifier (iterator/string based, no C++ buffer
   ownership imitation).
5. **Reusable corpora?** `minifypp` has its own smoke/adversarial tests
   (`make -C minifypp test-smoke`) plus the minify integration cases in
   nift-embed (`test-minify`, `test-json-schema-integration` minify paths).
6. **Guarantees needing exact parity?** The minified byte output per format
   for the corpus inputs, deterministic behaviour, and the minify-version
   stamping contract.
7. **Allowed to differ internally?** Data structures, buffers, algorithm
   organization — not the output bytes.
8. **At what Nift layer?** `minify-rs` is invoked where Nift semantics require
   it (the build path when a page is configured for minification), NOT
   unconditionally inside `Engine::render()`. The Embed render contract stays
   "template + context -> semantic rendered output"; minification remains an
   explicit project/build option.

## Proposed crate/workspace layout

```
crates/
    jsonic/         native Rust Jsonic (parse, Value, schema validation)
    minify/         native Rust Minify++ (Html/Css/JavaScript/Jsx/Json/Xml/Svg)
    nift/           native Rust Nift Embed; consumes jsonic + minify
                    (json.rs removed; value.rs migrated to consume jsonic's Value)
```

## Migration path for nift-rs

1. `jsonic-rs`: design its own `Value` + parser + schema (from Jsonic++
   contract, not a line-for-line port); port the Jsonic++ regression/
   adversarial/memory corpora as Rust tests.
2. `minify-rs`: native implementation of the seven formats; port the Minify++
   smoke/adversarial corpora.
3. Migrate `nift-rs`: replace `crate::value::Value`/`json.rs` usage with
   `jsonic`, wire `minify` where Nift minification semantics apply, delete the
   private duplicate parser.
4. Re-run the full C++-Embed conformance corpus and the new layer corpora.

## Conformance strategy (layered evidence)

```
Jsonic++ tests  ↔  jsonic-rs            (layer conformance)
Minify++ tests  ↔  minify-rs            (layer conformance)
C++ Embed       ↔  nift-rs              (Nift Embed conformance corpus)
pre-Embed Nift  ↔  Embed-era Nift       (regression suites, below)
```

If final rendered output differs, the layer corpora localize the disagreement
to JSON, minify, template semantics, or host/project semantics.

## Regression-suite destinations (concrete, in the workspace)

- `nift-regression-suite` — canonical pre-Embed suite; UNTOUCHED during the
  experiment; preserve via a permanent `pre-embed-baseline` tag/commit.
- `nift-embed-regression-suite` (local fork at the canonical commit) — the C++
  suite, expanded with Embed-era guarantees: new build/info CLI grammar,
  `.unfinished` ownership/recovery, zero-mutation failure handling,
  direct-write recovery, `build --repair`, pagination result changes, and later
  C ABI/binding cases.
- `nift-rs-regression-suite` (empty repo) — a Rust-NATIVE regression suite
  testing equivalent contracts (not a mechanical copy of the C++ shell
  scripts); same behavioural proofs where the contract is behavioural, better
  Rust-native structure where appropriate.

Eventually: run the preserved pre-Embed suite against both pre-Embed and
Embed-era Nift (historical compatibility), plus the expanded suites (new
guarantees); after Embed merges, merge the approved C++ regression additions
back into `nift-regression-suite`.

## Implementation checkpoint sequence

```
1. jsonic-rs design/implementation        2. Jsonic++ ↔ jsonic-rs conformance
3. minify-rs design/implementation        4. Minify++ ↔ minify-rs conformance
5. nift-rs uses both                      6. complete pagination Embed API
7. populate nift-rs-regression-suite      8. expand nift-embed-regression-suite
9. C++ ↔ Rust Embed conformance           10. C ABI
11. first production binding              12. binding conformance
13. final performance/sanitizer/platform campaign
14. merge nift-embed → nift
15. merge approved C++ regression additions → nift-regression-suite
16. website + release
```

## What remains deliberately different from C++

- Rust-native data structures, ownership, and APIs for jsonic/minify/nift.
- jsonic-rs and minify-rs are standalone releasable Rust libraries (not
  bindings); the production binding strategy remains canonical-C++ → C ABI →
  thin Go/Python/Node/C# bindings.
- The Rust repair module (CP5, commit `4fe1078`) remains experimental
  evidence, not part of the binding API.
