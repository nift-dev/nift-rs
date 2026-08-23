# Complete semantic inventory (NR0 hard gate)

Every frozen C++ template-language capability has exactly **one disposition**
in this matrix before NR1 starts. Authority legend:

- **corpus** — pinned by canonical goldens in `corpus/cases/` (implementation-independent oracle)
- **contract** — decided C++ API semantics recorded in the frozen handover (`nift-embed/docs/handover/EMBED.md`)
- **cli-tests** — evidenced only by the frozen C++ CLI test suite; NOT yet in the 9 canonical goldens → must be added to the expanded differential corpus at NR10 (see "corpus-expansion pending")
- **new** — defined by this programme (Rust API layer)

Every row also records: portable? / owning NR checkpoint / required evidence /
intentionally unsupported in v1?

## 1. Value and data model

| Feature | Authority | Portable? | Checkpoint | Evidence | v1 unsupported? |
|---|---|---|---|---|---|
| `Value`: null/bool/number/string/array/object | contract | portable semantics | NR1 | unit + property | no |
| Structured construction (object member set, array build, indexing) | contract | portable semantics | NR1 | unit + property | no |
| Template-level type-mismatch reads (`$[x.y]` on non-object, etc.) | contract + corpus | portable | NR2 | differential + goldens | no |
| Deep-copy / move semantics of the value object | contract | Rust API semantics (owned `Value`) | NR1 | unit | no (idiom differs) |

## 2. Template text surface

| Feature | Authority | Portable? | Checkpoint | Evidence | v1 unsupported? |
|---|---|---|---|---|---|
| Literal text emission | corpus + cli-tests | yes | NR2 | unit + differential + goldens | no |
| Comments | cli-tests | yes | NR2 | unit + differential; NR10 corpus expansion | no |
| Escaping / `@ent` | cli-tests | yes | NR2 (syntax) / NR4 (`@ent`) | unit + differential; NR10 expansion | no |
| Whitespace / indentation of `@input` output | corpus | yes | NR4 | goldens (comprehensive) | no |

## 3. Values and control flow

| Feature | Authority | Portable? | Checkpoint | Evidence | v1 unsupported? |
|---|---|---|---|---|---|
| `$[...]` value lookup | corpus + contract | yes | NR2 | unit + goldens | no |
| Deterministic metadata (title/name/content-path/output-path/template-path) | corpus + contract | yes | NR2 | unit + goldens | no |
| `loop` metadata | cli-tests | yes | NR3 | unit + differential; NR10 expansion | no |
| `@content` | corpus + contract | yes | NR2 | unit + goldens | no |
| `@if` | cli-tests | yes | NR2 | unit + differential; NR10 expansion | no |
| `@for` | cli-tests | yes | NR3 | unit + differential; NR10 expansion | no |
| Collections (arrays/objects, iteration, indexing) | cli-tests | yes | NR3 | unit + differential | no |
| Expression functions: filter/map/sort/slice/find/some/every/distinct/reverse/sum/prod/min/max/reduce/substr/join | cli-tests | yes | NR3 | unit + differential; NR10 expanded corpus | no |
| Arithmetic / comparisons / ordering | cli-tests | yes | NR3 | unit + differential; NR10 expansion | no |

## 4. Time- and platform-dependent built-ins (special disposition)

| Feature | Authority | Portable? | Checkpoint | Evidence | v1 unsupported? |
|---|---|---|---|---|---|
| `build-time` / `build-date` / `build-UTC-time` / `build-UTC-date` / `build-YYYY` / `build-YY` / `build-timezone` | contract (documented built-ins) | portable **semantics**, runtime-dependent **values** | NR2 | format/shape unit tests + controlled-clock tests; **excluded from byte goldens** | no |
| `build-OS` | contract (explicitly per-OS) | platform-dependent **by specification** | NR2 | unit; permitted variation recorded | no |

These are the only explicitly specified environment-dependent built-ins.
Unexpected path/locale differences are NOT permitted variation (see
`docs/authorities.md`).

## 5. Source/data directives

| Feature | Authority | Portable? | Checkpoint | Evidence | v1 unsupported? |
|---|---|---|---|---|---|
| `@input` | corpus | yes | NR4 | goldens (comprehensive) + unit | no |
| `@json` | corpus | yes | NR4 | goldens + unit | no |
| JSON Schema | corpus | yes | NR4 | goldens (schema) + unit | no |
| `@getenv` | corpus | yes | NR4 | goldens (getenv) + unit | no |
| `@dep` (explicit dependency declaration) | cli-tests | yes | NR4 | unit + differential; NR10 expansion | no |
| `@ent` | cli-tests | yes | NR4 | unit + differential | no |
| Contract resolution (host capability) | contract | yes | NR4 (capability) / NR8 (project-backed sources) | goldens (comprehensive) + regression | no |
| Dependency discovery | corpus | yes | NR4 | goldens (deps) | no |

## 6. Path and output geometry

| Feature | Authority | Portable? | Checkpoint | Evidence | v1 unsupported? |
|---|---|---|---|---|---|
| `@pathto` concrete project path | corpus | yes | NR5 | goldens (comprehensive) | no |
| `@pathto` tracked name | corpus | yes | NR5/NR8 | goldens (comprehensive) | no |
| `@pathtofile` | cli-tests | yes | NR5 | unit + differential; NR10 expansion | no |
| `@pathtopage` (pagination navigation) | cli-tests | yes | NR5 (parse/geometry) / NR8 (pagination) | unit + differential; NR8 goldens | no |
| Current-output context (`has_output_context` rule) | contract | yes | NR5 | unit + differential | no |
| 404 root-absolute rule | corpus + contract | yes | NR5/NR8 | goldens (404) | no |
| Requirements discovery | corpus | yes | NR5 | goldens (reqs) | no |
| Path containment (project-root escape) | corpus + contract | yes | NR5 | adversarial + reject class project-root-escape | no |

## 7. Project-aware

| Feature | Authority | Portable? | Checkpoint | Evidence | v1 unsupported? |
|---|---|---|---|---|---|
| `.nift/config.json` parsing/validation | reject classes + contract | yes | NR7 | ported invalid-state corpus | no |
| `.nift/tracked.json` parsing/validation | reject classes + contract | yes | NR7 | ported invalid-state corpus | no |
| Tracked-name rules (index, trailing slash) | corpus + contract | yes | NR7 | ported parity + goldens | no |
| Content/output/pagination geometry | corpus | yes | NR7 | ported parity + goldens | no |
| Transactional open (success → complete snapshot; failure → no partial state) | contract | yes | NR7 | ported parity tests | no |
| Zero project writes | contract | yes | NR7/NR8 | test | no |
| `render_page("about")`, primary pagination output | contract + corpus | yes | NR8 | full canonical 9/9 | no; arbitrary page selection unsupported |
| Host-vs-contract precedence | contract (regression) | yes | NR8 | unit + regression | no |
| Reload lifecycle (atomic generation, last-good retention) | contract (C++ serving contract) | yes, observable guarantees | NR9 | deterministic + stress | no (idiomatic Rust API) |
| Concurrent rendering / concurrent reload | contract | yes, observable guarantees | NR6/NR9 | stress | no |

## 8. Rust API layer (defined by this programme)

| Feature | Authority | Portable? | Checkpoint | Evidence | v1 unsupported? |
|---|---|---|---|---|---|
| `Context` overlays / Engine defaults precedence | contract | Rust API semantics | NR1/NR6 | unit + regression | no |
| `Source` (text/path) | contract (C++ API) | Rust API semantics | NR1/NR6 | unit | no |
| Loader seam (in-memory sources) | contract (C++ API) | Rust API semantics | NR6 | unit | no |
| Environment-provider seam | contract (C++ API) | Rust API semantics | NR6 | unit | no |
| `RenderResult` (ok/output/error/dependencies/requirements) | contract | Rust API semantics | NR1/NR6 | unit + goldens | no |
| Typed internal `ErrorKind` (Parse/MissingSource/InvalidConfig/InvalidTracking/PathEscape/UnknownPage/Schema/Render/...) | new | Rust API semantics | NR1 | unit | no |

## 9. Explicitly out of v1 (intentionally unsupported)

| Feature | Authority | Disposition |
|---|---|---|
| `build-all`/`build-updated` CLI, incremental build machinery | contract | unsupported |
| Tracking mutation (`save_tracking`) | contract | unsupported |
| `.info.json` writing | contract | unsupported |
| Watch mode / filesystem watching | contract | unsupported |
| Value serialization (`dump()`) | contract | unsupported |
| Arbitrary pagination-page API | contract | unsupported |
| C ABI / ecosystem bindings | contract | out of v1 scope |

## Corpus-expansion pending (NR10)

The following are portable Nift semantics currently evidenced **only** by the
frozen C++ CLI test suite, not by the 9 canonical goldens. They are implemented
per the frozen reference with differential evidence and **added to the
expanded differential corpus at NR10** so they become canonical goldens:

comments, escaping/`@ent`, `@if`, `@for`, `loop` metadata, collections, the
expression functions (filter/map/sort/slice/find/some/every/distinct/reverse/
sum/prod/min/max/reduce/substr/join), arithmetic/comparisons, `@dep`,
`@pathtofile`, `@pathtopage`.

## Unresolved / open dispositions (reported, not silently specified)

- **Empty-root containment**: the C++ standalone engine derives `@json`/`@dep`
  containment meaning from the process CWD when no root is configured. This is a
  recorded C++ edge. Rust must define the equivalent standalone behavior
  explicitly; disposition is a **Rust API decision at NR4/NR6** (likely: no
  relative containment without an explicit root → controlled error). Open until
  then.
- **Loader probe-then-read**: the C++ loader contract is a repeatable lookup
  (may be called twice per source). Rust's loader seam should state this
  explicitly; disposition is a Rust API decision at NR6.
- **`tracked_output_path` repeated lookup**: a C++ implementation/API-hardening
  note; has no Rust analogue to reproduce. Recorded only.
- **ASAN-FLAKE-001**: a C++ toolchain/instrumentation anomaly; not portable
  Nift semantics and not applicable to Rust. Recorded only.

## Rule reminders

- Canonical contract > C++ implementation accident.
- Never regenerate a golden to make Rust pass.
- Time/platform-dependent built-ins are handled as Section 4, not folded into
  ordinary byte goldens and not broadened beyond `build-OS`.
