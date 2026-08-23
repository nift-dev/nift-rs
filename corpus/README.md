# Canonical conformance corpus (mirror)

This directory is a **hash-verified mirror** of the authoritative Nift
semantic corpus, which lives in the frozen C++ reference worktree at
`nift-embed/tests/conformance/` (fixture projects, canonical output goldens,
canonical dependency/requirement goldens, semantic rejection classes and corpus
metadata).

Rules (NR0):

- There is **one authoritative representation** of the semantic data. It is
  edited in the C++ reference worktree, under review. It is never edited here.
- The mirror is byte-identical to the source; `MANIFEST.sha256` records every
  file's hash. `corpus/sync_corpus.sh verify` proves the mirror has not been
  accidentally forked.
- A semantic change flows: decision → canonical corpus changes once → every
  implementation's runner (C++ CLI, C++ Engine, Rust Engine) tests it.
- Implementation-specific runners/adapters are NOT part of the corpus. The Rust
  conformance runner will be built under `crates/` during NR8/NR10.

Commands:

```sh
bash corpus/sync_corpus.sh sync    # refresh the mirror from the canonical source
bash corpus/sync_corpus.sh verify  # prove the mirror is intact (CI does this)
```

The source path defaults to the sibling `nift-embed/tests/conformance` and is
overridable with `NIFT_EMBED_CORPUS`.
