# Authorities and non-authorities

`nift-rs` is an independent implementation of the Nift semantic contract. This
document records what is authoritative for that contract and what is not.

## Authorities

1. **The canonical conformance corpus** (mirrored in `corpus/`; authoritative
   source: the frozen C++ reference worktree `nift-embed/tests/conformance/`).
   Fixture projects, canonical output goldens, canonical dependency/requirement
   goldens, semantic rejection classes and corpus metadata. The goldens are the
   implementation-independent oracle: "these are the answers".

2. **Decided C++ API semantics** recorded in the frozen C++ handover
   (`nift-embed/docs/handover/EMBED.md`), i.e.:
   - the frozen `nift::Value` / `nift::Context` / `nift::Engine` /
     `nift::RenderResult` contracts;
   - the precedence order (Context overlay > Engine default > `@json` >
     contract binding > built-in metadata) and host-vs-contract precedence;
   - structural built-ins (name, content-path, output-path, template-path,
     loop) not overridable by `set`;
   - `@pathto` semantics incl. the 404 root-absolute rule and the
     current-output requirement;
   - the primary-pagination decision (`render("blog/")` == CLI primary output;
     arbitrary page selection is outside v1);
   - immutable project snapshot semantics, transactional open, atomic
     metadata-generation reload with last-good retention, zero project writes;
   - the concurrency contract (concurrent `render`; reload safe with renders;
     other mutators are a pre-serve boundary).

3. **The frozen rejection taxonomy** (syntax-invalid project state /
   semantically-invalid project state / source-runtime invalidity) and the
   canonical reject classes (invalid-config-json, invalid-tracking-json,
   unknown-config-key, duplicate-tracked-name, missing-source,
   project-root-escape).

4. **The frozen documentation of what is intentionally outside v1**:
   `build --all` CLI/build machinery, tracking mutation, `.info.json` writing,
   watch mode, incremental builds, arbitrary pagination-page API, value
   serialization, the C ABI and ecosystem bindings.

## Non-authorities (never silently inherited)

- **C++ implementation internals**: `Parser` object layout/state, `RenderHost`
  method spelling, `shared_ptr`/mutex patterns, `EngineHost`/`ProjectHost`
  concrete shapes, JSONic++ `Document` internals. Rust architecture may differ
  wherever observable guarantees stay equivalent.
- **Undocumented C++ behaviour** not reflected in the corpus, the decided-API
  record or the C++ CLI test suite. When the contract does not answer a
  semantic question, `nift-rs` STOPS and classifies (see below) rather than
  declaring whatever C++ happens to do to be the specification.
- **Permitted platform variation beyond what is explicitly specified**.
  Only `build-OS` is explicitly platform-dependent by specification.
  Time-dependent built-ins (`build-time`/`build-date`/...) have portable
  semantics but runtime-dependent values and are excluded from byte goldens.
  An unexpected Windows `\` path or a locale-dependent render is a conformance
  finding first (it goes RED), never an automatic "platform variation".

## Disagreement protocol (hard gate)

When Rust and C++ (or Rust and a canonical golden) disagree:

1. STOP. Neither side changes to copy the other.
2. Classify:
   - Rust bug
   - C++ bug
   - canonical-golden violation
   - unspecified behaviour
   - permitted implementation difference
   - specification defect
3. Provide evidence, ask for review.

Never regenerate a golden merely to make Rust pass. If the canonical semantics
are intentionally changed, the contract changes first and both implementations
are updated.

## C++ archaeology rule

When the frozen documentation/corpus does not answer a semantic question:
STOP, classify the ambiguity, provide evidence, ask for review. Do not inspect
the C++ implementation and silently declare its current behaviour to be the
specification.
