#!/usr/bin/env bash
# Reproducible Minify++ <-> minify-rs differential gate from a clean checkout.
# Builds the C++ reference in a temporary location, builds the minify-rs CLI
# example (release), runs scripts/minify_differential.py, propagates failure,
# and cleans the temporary C++ artifacts.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MINIFYPP="$ROOT/../nift-embed/minifypp"
JSONIC="$ROOT/../nift-embed/jsonic"
TMP=$(mktemp -d "${TMPDIR:-/tmp}/minify-diff.XXXXXX")
trap 'rm -rf "$TMP"' EXIT

echo "[minify-differential] building C++ reference..."
g++ -std=c++17 -O2 -I "$MINIFYPP/include" -I "$JSONIC/include" \
    "$ROOT/scripts/minify_cpp_cli.cpp" "$MINIFYPP/src/Minify.cpp" -o "$TMP/minify_cpp"

echo "[minify-differential] building minify-rs release runner..."
cargo build -p minify --release --example minify_cli

echo "[minify-differential] running 126-case differential..."
MINIFY_CPP="$TMP/minify_cpp" \
MINIFY_RUST="$ROOT/target/release/examples/minify_cli" \
    python3 "$ROOT/scripts/minify_differential.py" diff
