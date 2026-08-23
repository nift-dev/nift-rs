# nift-rs development programme (NR0–NR12)

The frozen development programme for the independent Rust implementation of
Nift. One checkpoint → evidence → commit → **STOP for hard review**. Within a
multi-checkpoint group, each checkpoint is committed separately; a group may
continue only if the preceding checkpoint completed cleanly with no unresolved
finding.

## Review cadence

```
GROUP A  NR0               STOP
GROUP B  NR1               STOP
GROUP C  NR2               STOP
GROUP D  NR3 + NR4         STOP
GROUP E  NR5 + NR6         STOP
GROUP F  NR7               STOP
GROUP G  NR8               STOP
GROUP H  NR9               STOP
GROUP I  NR10              STOP
GROUP J  NR11              STOP
GROUP K  NR12              FINAL
```

NR0–NR2 each stop independently: mistakes there (wrong semantic inventory,
wrong value/error model, wrong parser/Host architecture) contaminate everything
above. NR3+NR4 extend the reviewed kernel into expression/collection and
source/data semantics. NR5+NR6 feed path/output geometry into the standalone
public Engine. NR7–NR12 are each major independent proof boundaries.

If any checkpoint surfaces an unexpected semantic ambiguity, C++ or corpus
disagreement, architectural problem, safety issue, or specification hole:
**stop immediately**, classify, provide evidence, ask for review.

## Checkpoints

- **NR0 — baseline + complete semantic inventory + canonical corpus**
  crate/workspace baseline, `#![forbid(unsafe_code)]`, CI/test skeleton,
  shared-corpus arrangement, authorities/non-authorities document, the complete
  semantic inventory (hard gate: every frozen capability has one disposition),
  checkpoint/evidence mapping. *Gate: reviewed plan + corpus + inventory; no
  renderer.*

- **NR1 — Value + Context + typed errors/results**
  `Value` (null/bool/number/string/array/object) incl. structured construction;
  `Context`; engine default map; `Source` (text/path); `RenderError` with typed
  `ErrorKind`; `RenderResult` (ok/output/error/dependencies/requirements).
  Borrowing/ownership designed deliberately. *Gate: unit/property tests for
  Value/Context precedence.*

- **NR2 — parser kernel I + Host seam**
  `trait RenderHost` introduced now (with an in-memory test host); renders are
  functions of (source, host, context) → `Result`, not a stateful parser
  object. Literals, comments, escaping, `$[...]` values, deterministic
  metadata, `@content`, `@if`. *Gate: unit + parser-only differential +
  applicable goldens.*

- **NR3 — parser kernel II**
  `@for` + loop metadata; collections; the complete expression-function
  surface (filter/map/sort/slice/find/some/every/distinct/reverse/sum/prod/
  min/max/reduce/substr/join); arithmetic/comparisons; `@item`/`@paginate`
  parse semantics (project-backed semantics gated at NR8). *Gate: unit +
  differential + applicable goldens.*

- **NR4 — source/data capabilities**
  filesystem host, environment provider, custom loader seam; `@input`,
  `@json` + JSON Schema, `@getenv`, `@dep`, `@ent`, contract host capability
  (empty in standalone), dependency discovery. Path containment adversarial.
  *Gate: applicable canonical cases (getenv, schema) + applicable reject
  classes.*

- **NR5 — path + output geometry**
  `@pathto` (concrete + tracked), `@pathtofile`, `@pathtopage`,
  output context, 404 rule, requirements discovery, containment. *Gate:
  applicable goldens + adversarial containment.*

- **NR6 — standalone public Rust Engine**
  `render`/`render(partial)` APIs, `Source`, defaults, loader + environment
  seams, `RenderResult`, controlled errors, concurrent immutable renders.
  *Gate: standalone C++ ↔ Rust differential battery + Rust-native tests.*

- **NR7 — ProjectRead + immutable ProjectState**
  config + tracking validation, one project-read authority, project geometry,
  transactional open, zero project writes. *Gate: ported invalid-state/parity
  corpus matches canonical acceptance/rejection.*

- **NR8 — project-aware Engine = implementation conformance**
  `Engine::open(root)`, `render_page("about", &context)`, `ProjectHost` over
  `ProjectState`, same kernel, contracts, Context overlays, primary pagination,
  host-vs-contract precedence. **Gate: full canonical 9/9 passes Rust**
  (CLI == golden, C++ Engine == golden, Rust Engine == golden).

- **NR9 — reload + concurrent serving lifecycle**
  immutable `Arc` generations, atomic publication, last-good retention, zero
  writes, content-not-snapshot-isolated documented; stress + Loom if practical.
  *Gate: deterministic lifecycle + heavy stress.*

- **NR10 — cross-implementation differential validation**
  C++ CLI ↔ C++ Engine ↔ Rust Engine against the canonical corpus **and** the
  substantially expanded behavioural differential corpus; Linux/macOS/Windows
  CI; disagreement protocol. *Gate: zero unexplained divergence.*

- **NR11 — hardening**
  Miri, sanitizers where applicable, fuzzing, property testing, malformed/
  adversarial inputs, path safety, recursion depth, resource behaviour,
  concurrency stress. *Gate: no unexplained sanitizer/Miri/fuzz/property
  failures.*

- **NR12 — DX + performance + docs + final cold review**
  `nift = "..."` DX; benchmarks vs Askama/Tera/MiniJinja and the C++ Engine;
  the cold independent review checklist. *Gate: final sign-off.*

## Programme-wide rules

```
canonical contract > C++ implementation accident
one rendering kernel
#![forbid(unsafe_code)] in the core crate
one authoritative semantic corpus
embedded/project-aware rendering performs zero project writes
never regenerate a golden merely to make Rust pass
disagreement goes RED first and is classified
Rust architecture may differ internally where observable guarantees agree
start each checkpoint from the contract/corpus, not from C++ source translation
correctness → architecture → safety → DX → concurrency → performance
```
