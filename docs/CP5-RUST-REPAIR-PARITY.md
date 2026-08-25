# CP5 report: Rust `build --repair` parity

Implements the accepted C++ repair contract (CP4.3 freeze) idiomatically in
Rust. nift-rs is an engine + project library (no CLI build pipeline), so the
contract is exposed as a library repair operation with the same externally
observable semantics.

## Implementation (crates/nift/src/repair.rs)

- `Ownership` (two-layer model): an OS-held advisory lock via `File::try_lock`
  (flock on Unix / LockFileEx on Windows, kernel-released on process death)
  plus the persistent `.nift/.unfinished` marker. Acquisition returns
  Clean/Stale/Live/Failed with the exact C++ protocol: atomic exclusive create
  (O_EXCL) for Clean, open+lock for Stale (no live owner) vs Live (held).
  `finish()` removes the marker strictly after the final mutation; dropping
  without finish retains it (crash evidence). One `sync_all()` at acquisition
  (best-effort durability). Env-gated `NIFT_TEST_OWNERSHIP_HOLD` hook for
  deterministic interrupted-repair testing (parity with C++).
- `repair_project(root)`: opens ProjectState (authoritative), acquires
  ownership (Clean/Stale proceed; Live/Failed refuse), reconstructs every
  tracked page (render via the Rust Engine -> direct in-place output write +
  canonical `.info.json` in the C++ schema), runs the ownership-aware sweep,
  and clears the marker only on complete success.
- Ownership-aware sweep:
  - orphan `.info.json` metadata removed (REQUIRED; failures propagate);
    its historical public output PRESERVED (path only knowable from distrusted
    derived metadata - conservative orphan rule).
  - stale stored-hash cleanup BEST-EFFORT (regenerable cache, failures ignored).
- Repair failure always retains the marker; success converges and is idempotent
  for the reconstructible surface.

## Tests (crates/nift/tests/repair_parity.rs, 5)

- non-paginated repair converges after deletion/corruption and is idempotent;
  reconstructed output byte-matches the engine's canonical render;
- orphan .info.json removed, historical public output preserved, user file
  preserved;
- hostile orphan metadata ("output":"public/keepme.txt") cannot delete a user
  file;
- repair refuses a Live owner; a dropped (crashed) holder leaves a stale
  marker that repair then takes over and clears;
- repair failure on a broken authoritative template retains the marker; fixing
  the input lets repair succeed and clear it.

## Parity gap found (the experiment's finding)

The Rust render engine exposes ONLY the PRIMARY pagination page (page 1) of a
paginated tracked page (`RenderResult` has no pagination-outputs vector;
`assemble_primary_pagination` emits page 1), whereas the C++ engine emits all
pagination pages 2..N. Therefore full repair convergence for paginated
projects cannot be certified in Rust: `repair_project` fails closed for a
paginated page (retains the marker) rather than claiming convergence it cannot
produce, and the pagination-surplus ownership sweep (canonical `<base>-<N>`,
N >= 2, no leading zeros, overflow-safe) is implemented but only exercised once
the engine can render pages 2..N. This is exactly the kind of boundary the
cross-language conformance experiment exists to surface.

Recommended next step: extend `assemble_primary_pagination` to emit every
pagination page (mirroring `Parser::render`'s per-page loop), then implement
the pagination-surplus sweep and paginated convergence tests.

## Full Rust test suite

cargo test: all suites pass EXCEPT one PRE-EXISTING failure
(`corpus_parity_pages_match_goldens` in nr8_project_engine.rs, "getenv// output
does not match the golden") that fails identically WITHOUT the repair change
(verified via git stash). It is corpus drift: the Rust copy of the getenv
golden is stale relative to the current C++ behaviour. Unrelated to this
checkpoint; flagged for corpus re-sync.

## Commit / hygiene

(committed by the CP5 checkpoint in the nift-rs repository).
