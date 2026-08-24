#!/usr/bin/env bash
# NR6 standalone Engine C++ <-> Rust differential battery.
#
# Runs a battery of cases through the frozen C++ standalone Engine harness
# (built from nift-embed) and the Rust Engine harness (cargo build --example
# engine_harness) and requires byte-identical observable JSON results
# (output/dependencies/requirements/loaderKeys or error). Compare observable
# results, not merely that both succeeded.
#
# Case inventory (16 cases):
#   1  composed text render
#   2  composed path render (Source::Path loading)
#   3  partial render
#   4  defaults + Context precedence
#   5  explicit output context -> @pathto
#   6  absent output context -> @pathto controlled error
#   7  content-path/output-path authority with page_name
#   8  content-path authority with no page_name
#   9  @input relative to the loaded source path
#   10 dependencies/requirements spelling
#   11 missing JSON file controlled error
#   12 loader: path keys reaching the custom loader (page + template)
#   13 loader: @input path key resolving through the custom loader
#   14 environment provider: @getenv values
#   15 environment provider: missing value -> @getenv emits nothing
#   16 no provider (process-env fallback): unset variable -> nothing
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
    local page="$1" tpl="$2" pname="$3" co="$4" ppath="$5" tpath="$6" mode="$7" bindings="$8" seam="${9:-}"
    local cpp_out rust_out
    cpp_out="$(printf '%s' "$bindings" | "$CPP_HARNESS" "$TMP" "$page" "$tpl" "$pname" "$co" "$ppath" "$tpath" "$mode" "$seam")"
    rust_out="$(printf '%s' "$bindings" | "$RUST_HARNESS" "$TMP" "$page" "$tpl" "$pname" "$co" "$ppath" "$tpath" "$mode" "$seam")"
    if [ "$cpp_out" != "$rust_out" ]; then
        echo "MISMATCH page='$page' tpl='$tpl' name='$pname' co='$co' ppath='$ppath' tpath='$tpath' mode='$mode' seam='$seam'"
        echo "  C++ : $cpp_out"
        echo "  Rust: $rust_out"
        fail=1
    else
        echo "PASS name='$pname' co='$co' mode='$mode' seam='$seam'"
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
# Loader seam: Source::Path page + template reach the custom loader with the
# same resolved path keys and loader-provided content renders identically.
run_case - - - - content/blog.html templates/template.html composed '' loader
# Loader seam: @input resolves relative to the source and reaches the loader.
run_case - '<main>@content</main>' - - content/post.html - composed '' loader
# Environment provider: @getenv obtains provider-supplied values.
run_case '@getenv(NIFT_ENV_A)|@getenv(NIFT_ENV_B)' '<main>@content</main>' - - - - composed '' env
# Environment provider: missing value renders nothing.
run_case '@getenv(NIFT_ENV_MISSING)' '<main>@content</main>' - - - - composed '' env
# No provider: process-environment fallback, unset variable renders nothing.
run_case '@getenv(NIFT_DIFF_UNSET_VAR)' '<main>@content</main>' - - - - composed '' -

echo "---"
if [ "$fail" -ne 0 ]; then
    echo "NR6 differential: FAILED"
    exit 1
fi
echo "NR6 differential: PASS"
