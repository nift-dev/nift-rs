#!/usr/bin/env bash
# NR12 downstream-DX smoke test: prove a normal external crate (outside this
# workspace) can depend on `nift` by path, resolve it, compile, use the
# documented public re-export surface, and perform a standalone render.
#
# Usage: bash scripts/consumer_smoke.sh
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
NIFT_CRATE="$REPO/crates/nift"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/consumer/src"
cat >"$TMP/consumer/Cargo.toml" <<EOF
[package]
name = "consumer"
version = "0.0.0"
edition = "2021"

[dependencies]
nift = { path = "$NIFT_CRATE" }
EOF

cat >"$TMP/consumer/src/main.rs" <<'EOF'
// A normal downstream crate using only the documented public API: the crate
// root re-exports Engine/RenderError; Context and Source are public modules
// (nift::context / nift::source), matching the README.
use nift::context::Context;
use nift::source::Source;
use nift::{Engine, RenderError};

fn main() -> Result<(), RenderError> {
    // Standalone render through the public re-export surface.
    let mut engine = Engine::new();
    engine.set("greeting", "hi").expect("valid binding");
    let result = engine.render(
        &Source::text("<p>$[greeting] world</p>"),
        &Source::text("@content"),
        &Context::new(),
    )?;
    assert_eq!(result.output, "<p>hi world</p>");
    println!("standalone render OK: {}", result.output);
    Ok(())
}
EOF

(cd "$TMP/consumer" && cargo run --quiet)

# The downstream build must not pull the NR12 benchmark-only dev-dependencies.
if grep -q '"tera"\|"minijinja"\|"askama"\|"serde_json"' "$TMP/consumer/Cargo.lock"; then
    echo "error: downstream build pulled benchmark dev-dependencies" >&2
    exit 1
fi
echo "consumer smoke test passed (no dev-only deps leaked)"
