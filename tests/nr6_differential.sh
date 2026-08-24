#!/usr/bin/env bash
# NR6 standalone Engine C++ <-> Rust differential battery.
#
# Runs a battery of cases through the frozen C++ standalone Engine harness
# (built from nift-embed) and the Rust Engine harness (cargo build --example
# engine_harness) and requires byte-identical observable JSON results
# (output/dependencies/requirements or error). Compare observable results, not
# merely that both succeeded.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
CPP_HARNESS="${CPP_HARNESS:-$REPO/../nift-embed/.build/engine-harness}"
RUST_HARNESS="${RUST_HARNESS:-$REPO/target/debug/examples/engine_harness}"

if [ ! -x "$CPP_HARNESS" ]; then
    echo "error: C++ harness not found at $CPP_HARNESS (build nift-embed .build/engine-harness)" >&2
    exit 2
fi
if [ ! -x "$RUST_HARNESS" ]; then
    echo "error: Rust harness not found at $RUST_HARNESS (cargo build --example engine_harness)" >&2
    exit 2
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/content" "$TMP/templates" "$TMP/public"
printf '<p>PATH-CONTENT</p>\n' >"$TMP/content/blog.html"
printf 'P\n' >"$TMP/content/part.html"
printf '<main>@content</main>\n' >"$TMP/templates/template.html"
printf 'x\n' >"$TMP/public/app.js"

fail=0
run_case() {
    local page="$1" tpl="$2" pname="$3" co="$4" ppath="$5" tpath="$6" mode="$7" bindings="$8"
    local cpp_out rust_out
    cpp_out="$(printf '%s' "$bindings" | "$CPP_HARNESS" "$TMP" "$page" "$tpl" "$pname" "$co" "$ppath" "$tpath" "$mode")"
    rust_out="$(printf '%s' "$bindings" | "$RUST_HARNESS" "$TMP" "$page" "$tpl" "$pname" "$co" "$ppath" "$tpath" "$mode")"
    if [ "$cpp_out" != "$rust_out" ]; then
        echo "MISMATCH page='$page' tpl='$tpl' name='$pname' co='$co' ppath='$ppath' tpath='$tpath' mode='$mode'"
        echo "  C++ : $cpp_out"
        echo "  Rust: $rust_out"
        fail=1
    else
        echo "PASS name='$pname' co='$co' mode='$mode'"
    fi
}

CO="$TMP/public/about.html"

# Composed text render.
run_case '<h2>P</h2>' '<main>@content</main>' - - - - composed ''
# Composed path render (loading via Source::Path).
run_case - - - - content/blog.html templates/template.html composed ''
# Partial render.
run_case '<p>x</p>' - - - - - partial ''
# Engine defaults binding.
run_case 'site=$[site]@content' '<main>@content</main>' - - - - composed 'site=hello'
# Explicit output context -> @pathto resolves.
run_case '@pathto("public/app.js")@content' - about "$CO" - - composed ''
# Absent output context -> @pathto controlled error.
run_case '@pathto("public/app.js")@content' - about - - - composed ''
# Source/content path authority: page_name authoritative for metadata;
# Source::Path authoritative for loading.
run_case 'cp=$[content-path] op=$[output-path]@content' - blog "$CO" content/blog.html - composed ''
run_case 'cp=$[content-path]@content' - - - content/blog.html - composed ''
# @input relative resolves against the loaded source path.
run_case '@input("part.html")@content' - - - content/blog.html - composed ''
# Dependencies/requirements spelling.
run_case '@content' - blog - content/blog.html templates/template.html composed ''
# Controlled error: missing JSON file.
run_case '@json("data.json", d)$[d.v]@content' - - - - - composed ''

echo "---"
if [ "$fail" -ne 0 ]; then
    echo "NR6 differential: FAILED"
    exit 1
fi
echo "NR6 differential: PASS"
