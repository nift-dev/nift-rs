# CP5.1 — Nift Embed boundary review

Design analysis (no pagination extension, no new repair machinery, no change
to the frozen C++ CP4 contract). Direct comparison of the C++ nift-embed and
Rust nift-rs surfaces.

## Surface comparison

C++ Embed (`src/Engine.cpp`, `ProjectState.h`, `Types.h`, `RenderHost.h`):
- `Engine`: project(root), reload, is_open, open_error, render(page, context),
  set_loader, set_environment_provider, set (bindings), set_json.
- public `RenderResult`: `output`, `dependencies`, `requirements`. NO
  pagination outputs.
- `ProjectState`: open, config, tracked, find, content_path, output_path,
  pagination_output_path, relative, read_shared_source, read_shared_json.
- `RenderHost`: source reads, environment, pathto, contract bindings, build
  metadata.
- NOT in Embed: ProjectInfo (CLI build), ProjectOwnership (lock/marker),
  WatchList, FileSystem writes, .info.json, build_many/build_one, repair.

Rust (`crates/nift`): `Engine` (new/open/project/reload/is_open/open_error/
render_page/render/render_partial/set/set_json/set_loader/
set_environment_provider), `ProjectState` (open/config/tracked/find/path
helpers/relative/read_shared_source/read_shared_json), `ProjectHost`,
`RenderResult` (output/dependencies/requirements, NO pagination outputs).
CP5 added `repair.rs` (Ownership lock+marker, repair_project, sweep) - the
orchestrator layer.

The Engine and ProjectState surfaces are deliberately near-identical (built to
conformance). CP5 added build-orchestrator semantics to Rust.

## Three-layer classification

```
EMBED ENGINE SEMANTICS (portable - in both, must stay)
    parse, render, context/metadata, bindings, value/expression evaluation,
    @content/@input/@pathto/@item/@paginate/@for/@if/@json/@dep,
    dependency + requirement recording, pagination RENDERING semantics,
    environment/loader host seam
    -> defines the Embed contract

PROJECT MODEL / PORTABLE LIBRARY SEMANTICS (portable convenience - in both)
    ProjectState.open (config + tracked interpretation), configured path
    computation (content/output/pagination paths), relative spellings,
    shared source/json reads, page lookup
    -> a convenience bridge between authoritative project files and an
       embedding host; usable by CLI-like hosts, not the engine core

NIFT CLI / BUILD-ORCHESTRATOR SEMANTICS (C++ only - NOT Embed)
    build ownership lock + .unfinished marker lifecycle
    filesystem mutation epochs (reconcile_watch, direct writes)
    .info.json persistence
    hash-cache persistence
    repair sweep + ownership boundary
    watch recovery
    build-thread selection, incremental staleness
    -> the CLI/build tool layer, above the Embed boundary
```

## Answers to the review questions

1. **Minimal coherent Nift Embed contract**: the template engine (parse/render/
   context/bindings/deps/reqs/pagination rendering) plus the host seam. NOT
   filesystem mutation, persistence, or recovery.

2. **C++ Engine/Parser behaviours that must be portable**: the parse/render
   semantics captured by the conformance corpus (directives, expressions,
   value composition, dependency/requirement recording, pagination rendering,
   minify format versioning is engine-adjacent). These are what a binding must
   reproduce.

3. **Does complete pagination output belong to the engine contract? YES** - and
   this is the key finding. BOTH Embed engines' public RenderResult expose only
   the PRIMARY pagination page; the full pagination pages 2..N exist only in the
   C++ internal `::RenderResult` consumed by the CLI. An embedded caller (server,
   binding) rendering a paginated template cannot currently obtain pages 2..N
   from EITHER Embed surface. This is a shared Embed semantic gap, not a
   Rust-only one. It belongs to the engine contract, independent of build
   orchestration, and should be fixed at the Embed boundary (both engines), not
   by porting repair.

4. **ProjectState functionality in Embed**: the path computation + project
   interpretation is a portable convenience for hosts that need page/path
   resolution. It is not the engine core; a host may supply its own source/path
   resolution.

5. **Exclusively CLI/build-orchestrator**: ownership lock + .unfinished marker,
   mutation epochs, direct writes, .info.json persistence, hash cache, repair
   sweep, watch recovery. None belong to the binding API.

6. **.info.json**: a Nift-BUILD concept / internal implementation detail of the
   CLI orchestrator, NOT an Embed concept and NOT a public portable contract.
   Embed callers obtain deps/reqs from the RenderResult, not from persisted
   .info.json. The Rust repair hand-writing .info.json (CP5) was recreating
   orchestrator state - evidence for the boundary, not a contract to keep.

7. **tracked.json**: the embedding API should receive an ALREADY-RESOLVED
   page/project description (via the host seam and page names), not consume
   tracked.json directly as the core contract. ProjectState is a convenience
   adapter that bridges tracked.json to the host for CLI-like callers; the
   engine contract should not depend on it.

8. **Dependency/requirement boundary**: they cross the boundary as RENDER
   OUTPUT (the caller needs to know what a rendered page depends on). Already
   in both RenderResults; portable and correct as-is.

9. **Repair in the binding API?** NO. Repair is build orchestration (lock,
   marker, filesystem mutation, sweep). A language binding rendering templates
   does not need it. The eventual binding API = engine render + host seam only.

10. **What implementing Rust repair taught us about the C++ Embed API**:
    - Both Embed engines under-expose pagination: complete pagination output
      (pages 2..N) should be added to the public RenderResult of the Embed
      engine contract before merge.
    - The Embed boundary should be drawn so .info.json/hash/watch/ownership/
      repair are CLI-orchestrator concerns; the Rust repair experiment showed
      hand-maintaining them duplicates orchestrator semantics and accumulates
      parity obligations.
    - RenderResult (output + deps + reqs + future pagination outputs) is the
      correct cross-language boundary; ProjectState is an optional convenience.

## CP5 outcome (kept as experimental evidence, not grown)

commit `4fe1078` remains: its value is that implementing it forced the
boundary to reveal itself (ownership/sweep/.info.json are orchestrator
semantics; pagination is an engine gap). It is NOT the basis for the binding
API. No pagination extension and no further repair machinery were added.

## Corpus-drift root cause (resolved)

The single pre-existing Rust failure
(`corpus_parity_pages_match_goldens`, getenv case) was NOT stale goldens and
NOT wrong Rust behaviour: the C++ conformance driver injects each case's
declared environment (`expected.json` "env", e.g. PA_CONFORMANCE_ENV) as OS
env vars before building/rendering, but the Rust corpus test rendered without
injecting it, so `@getenv("PA_CONFORMANCE_ENV")` resolved to empty. Fixed the
test harness to inject each case's declared env (mirroring the driver).

## Full Rust regression

cargo test: all suites green (181 tests, 0 failures), including
corpus_parity_pages_match_goldens and the 5 repair_parity tests.
