#!/usr/bin/env bash
# NR12 pagination differential battery (C++ Embed <-> nift-rs Engine).
#
# Renders tracked pages (project-aware) through the frozen C++ standalone
# Engine harness (mode "page") and the Rust Engine harness, and requires
# byte-identical observable JSON results: output, dependencies, requirements,
# pagination (complete set of pages 2..N with page numbers) or error.
#
# Case inventory (10 cases):
#   1  non-paginated page                      -> empty pagination
#   2  single-page paginated (1 item)          -> empty pagination
#   3  three-page paginated (3 items, ipp 1)   -> pagination 2,3
#   4  two-page paginated (4 items, ipp 2)     -> pagination 2
#   5  partial final page (3 items, ipp 2)     -> pagination 2 (1 item)
#   6  Unicode items (CJK, emoji, combining)
#   7  @dep + @pathto(requirement) + @input partial in the paginate template
#   8  JSON binding ($[site.name]) in the paginate template
#   9  many pages (7 items, ipp 1)             -> pagination 2..7
#   10 unknown page name                       -> controlled error
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

WORK="$(mktemp -d "${TMPDIR:-/tmp}/nift-pgdiff.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

pass=0
fail=0

# run_case <name> <page_name> <project_dir> [stdin_text]
run_case() {
    local name="$1" page="$2" dir="$3" stdin_text="${4:-}"
    local cpp_out rust_out
    cpp_out="$("$CPP_HARNESS" "$dir" - - "$page" - - - page <<< "$stdin_text")"
    rust_out="$("$RUST_HARNESS" "$dir" - - "$page" - - - page <<< "$stdin_text")"
    if [ "$cpp_out" == "$rust_out" ]; then
        echo "ok   $name"
        pass=$((pass + 1))
    else
        echo "FAIL $name"
        echo "  C++ : $cpp_out"
        echo "  Rust: $rust_out"
        fail=$((fail + 1))
    fi
}

new_project() {
    local dir="$1" items_per_page="${2:-}"
    mkdir -p "$dir/"{content,templates,public,.nift}
    printf '{"config":{"content-dir":"content/","output-dir":"public/","default-template":"templates/template.html","incremental-mode":"modified"}}' > "$dir/.nift/config.json"
    printf '<main>$[title]</main>\n@content' > "$dir/templates/template.html"
    if [ -n "$items_per_page" ]; then
        printf '{"tracked":[{"name":"blog","title":"Blog","template":"templates/template.html","paginate":{"items-per-page":%s}}]}' "$items_per_page" > "$dir/.nift/tracked.json"
    else
        printf '{"tracked":[{"name":"blog","title":"Blog","template":"templates/template.html"}]}' > "$dir/.nift/tracked.json"
    fi
}

PAG_TMPL='<section>page $[paginate.current]/$[paginate.total]:[$[paginate.items]]</section>'

# 1 non-paginated
d="$WORK/1-nonpag"; new_project "$d"
printf '<p>static</p>' > "$d/content/blog.html"
run_case "1 non-paginated -> empty pagination" blog "$d"

# 2 single-page paginated
d="$WORK/2-single"; new_project "$d" 1
printf '@item{only}@paginate' > "$d/content/blog.html"
printf '%s' "$PAG_TMPL" > "$d/content/blog.paginate.html"
run_case "2 single page -> empty pagination" blog "$d"

# 3 three-page paginated
d="$WORK/3-three"; new_project "$d" 1
printf '@item{one}@item{two}@item{three}@paginate' > "$d/content/blog.html"
printf '%s' "$PAG_TMPL" > "$d/content/blog.paginate.html"
run_case "3 three pages -> pagination 2,3" blog "$d"

# 4 two-page paginated ipp 2
d="$WORK/4-two-ipp2"; new_project "$d" 2
printf '@item{a}@item{b}@item{c}@item{d}@paginate' > "$d/content/blog.html"
printf '%s' "$PAG_TMPL" > "$d/content/blog.paginate.html"
run_case "4 four items ipp2 -> pagination 2" blog "$d"

# 5 partial final page
d="$WORK/5-partial"; new_project "$d" 2
printf '@item{A}@item{B}@item{C}@paginate' > "$d/content/blog.html"
printf '%s' "$PAG_TMPL" > "$d/content/blog.paginate.html"
run_case "5 partial final page (1 item)" blog "$d"

# 6 unicode items
d="$WORK/6-unicode"; new_project "$d" 1
printf '@item{日本語}@item{émoji 😀}@item{e\\u0301 combining}@paginate' > "$d/content/blog.html"
printf '%s' "$PAG_TMPL" > "$d/content/blog.paginate.html"
run_case "6 unicode items" blog "$d"

# 7 deps + requirements + partial
d="$WORK/7-deps"; new_project "$d" 1
printf '@item{one}@item{two}@paginate' > "$d/content/blog.html"
printf '<section>@dep(\x27app.js\x27)@pathto(\x27asset.js\x27)@input(\x27part.html\x27)</section>' > "$d/content/blog.paginate.html"
printf '<p>PART</p>' > "$d/content/part.html"
run_case "7 deps + pathto requirement + input partial" blog "$d"

# 8 json binding
d="$WORK/8-json"; new_project "$d" 1
printf '@item{one}@item{two}@paginate' > "$d/content/blog.html"
printf '<section>$[site.name] page $[paginate.current]/$[paginate.total]</section>' > "$d/content/blog.paginate.html"
run_case "8 json binding in paginate template" blog "$d" 'site=json:{"name":"Acme"}'

# 9 many pages
d="$WORK/9-many"; new_project "$d" 1
printf '@item{n1}@item{n2}@item{n3}@item{n4}@item{n5}@item{n6}@item{n7}@paginate' > "$d/content/blog.html"
printf '%s' "$PAG_TMPL" > "$d/content/blog.paginate.html"
run_case "9 seven pages -> pagination 2..7" blog "$d"

# 10 unknown page
d="$WORK/10-unknown"; new_project "$d"
printf '<p>x</p>' > "$d/content/blog.html"
run_case "10 unknown page name error" nope "$d"

echo
echo "pagination differential: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
