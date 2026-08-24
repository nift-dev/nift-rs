#!/usr/bin/env bash
# Canonical conformance corpus arrangement for nift-rs.
#
# The authoritative semantic corpus lives in the frozen C++ reference worktree
# (nift-embed tests/conformance/): fixture projects, canonical output goldens,
# canonical dependency/requirement goldens, semantic rejection classes and
# corpus metadata. There is exactly ONE authoritative representation of that
# semantic data; nift-rs keeps a byte-identical, hash-verified mirror here and
# never edits it in place. Implementation-specific runners/adapters live next to
# each implementation, not inside the corpus.
#
# A semantic change flows:
#   semantic decision
#     -> canonical corpus changes once (in nift-embed, under review)
#     -> every implementation's runner tests it
#
# Modes:
#   sync    copy the canonical corpus into ./corpus and regenerate MANIFEST.sha256
#   verify  check the committed mirror is byte-identical to MANIFEST.sha256
#           (self-contained; does not need the sibling worktree)
#
# The sibling source is overridable with NIFT_EMBED_CORPUS (e.g. for CI).
set -euo pipefail

SOURCE="${NIFT_EMBED_CORPUS:-$(cd "$(dirname "$0")/../.." && pwd)/nift-embed/tests/conformance}"
HERE="$(cd "$(dirname "$0")" && pwd)"
MODE="${1:-sync}"

# The canonical semantic assets (adapters like run_conformance.py /
# gen_golden.py / cpp_runner.cpp are implementation-specific and are NOT
# mirrored).
CANONICAL_ASSETS="cases manifest.json"

# Deterministic ordering: LC_ALL=C forces a byte-order, locale-independent
# sort so the manifest line order is identical in every environment and under
# every locale. (The default `sort` uses LC_COLLATE, which reorders paths
# between locales and made verify fail falsely cross-locale.)
#
# Portable hash tool: sha256sum (GNU) with a shasum -a 256 fallback (macOS/BSD);
# both emit "<hash>  <path>".
HASH_TOOL="sha256sum"
if ! command -v "$HASH_TOOL" >/dev/null 2>&1; then
    if command -v shasum >/dev/null 2>&1; then
        HASH_TOOL="shasum -a 256"
    else
        echo "error: no sha-256 tool available (need sha256sum or shasum)" >&2
        exit 1
    fi
fi

# `tr -d '\r'` strips the CR that the MSYS/Git-Bash runtime injects into pipe
# output on Windows (LF -> CRLF in text mode), so the manifest comparison stays
# byte-exact with the LF committed manifest on every platform.
hash_tree() {
    (cd "$HERE" && find $CANONICAL_ASSETS -type f | LC_ALL=C sort | xargs $HASH_TOOL | tr -d '\r')
}

case "$MODE" in
    sync)
        if [[ ! -d "$SOURCE" ]]; then
            echo "error: canonical corpus source not found at $SOURCE" >&2
            echo "       set NIFT_EMBED_CORPUS to the nift-embed tests/conformance directory" >&2
            exit 2
        fi
        for asset in $CANONICAL_ASSETS; do
            rm -rf "$HERE/$asset"
            cp -a "$SOURCE/$asset" "$HERE/$asset"
        done
        hash_tree > "$HERE/MANIFEST.sha256"
        echo "synced canonical corpus from $SOURCE"
        echo "wrote MANIFEST.sha256 ($(wc -l < "$HERE/MANIFEST.sha256") entries)"
        ;;
    verify)
        if [[ ! -f "$HERE/MANIFEST.sha256" ]]; then
            echo "error: MANIFEST.sha256 missing (run 'sync')" >&2
            exit 1
        fi
        if diff -u "$HERE/MANIFEST.sha256" <(hash_tree) >/dev/null; then
            echo "corpus mirror verified: byte-identical to MANIFEST.sha256"
        else
            echo "corpus mirror MISMATCH: files differ from MANIFEST.sha256" >&2
            diff -u "$HERE/MANIFEST.sha256" <(hash_tree) >&2 || true
            echo "  - if the canonical corpus changed deliberately, run 'sync' and commit" >&2
            echo "  - otherwise this is an accidental fork of the canonical data" >&2
            exit 1
        fi
        ;;
    *)
        echo "usage: $0 [sync|verify]" >&2
        exit 2
        ;;
esac
