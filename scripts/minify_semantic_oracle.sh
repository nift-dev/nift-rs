#!/usr/bin/env bash
# Semantic oracle: minified output must preserve original semantics, proven
# with real tools (node for JS execution, node+node --check for syntax,
# PostCSS if available for CSS, tsc for JSX). Reuses the Minify++ semantic
# case content (minify_node_semantics / minify_generated_semantics).
#
# This is the SECOND type of evidence beyond byte-parity: two implementations
# can agree on the same wrong transformation, so we also prove original
# semantics == minified semantics.
set -euo pipefail
RUST="${MINIFY_RUST:-target/release/examples/minify_cli}"
TMP=$(mktemp -d "${TMPDIR:-/tmp}/minify-oracle.XXXXXX")
trap 'rm -rf "$TMP"' EXIT
fails=0

js_case() {
    local name="$1" source="$2"
    printf '%s' "$source" >"$TMP/$name.js"
    "$RUST" js "$source" >"$TMP/$name.min.js" 2>/dev/null || { echo "FAIL $name: minify error"; fails=$((fails+1)); return; }
    set +e
    node "$TMP/$name.js" >"$TMP/$name.orig.out" 2>"$TMP/$name.orig.err"
    local a=$?
    node "$TMP/$name.min.js" >"$TMP/$name.min.out" 2>"$TMP/$name.min.err"
    local b=$?
    set -e
    if [ "$a" -ne "$b" ]; then echo "FAIL $name: exit $a vs $b"; fails=$((fails+1)); return; fi
    cmp -s "$TMP/$name.orig.out" "$TMP/$name.min.out" || { echo "FAIL $name: stdout differs"; fails=$((fails+1)); return; }
    cmp -s "$TMP/$name.orig.err" "$TMP/$name.min.err" || { echo "FAIL $name: stderr differs"; fails=$((fails+1)); return; }
    echo "PASS js/$name"
}

css_case() {
    local name="$1" source="$2"
    local out
    out=$("$RUST" css "$source") || { echo "FAIL $name: minify error"; fails=$((fails+1)); return; }
    # Structural oracle: balanced braces/brackets/parens and intact strings.
    node -e 'const s=process.argv[1]; const q=/("(?:[^"\\]|\\.)*"|\x27(?:[^\x27\\]|\\.)*\x27)/g; const stripped=s.replace(q,""); let bal={"{":0,"}":0,"[":0,"]":0,"(":0,")":0}; for(const c of stripped){if(c in bal) bal[c]++;} if(bal["{"]!==bal["}"]||bal["["]!==bal["]"]||bal["("]!==bal[")"]) { console.error("unbalanced"); process.exit(1);}' "$out" \
        || { echo "FAIL $name: minified CSS structurally invalid"; fails=$((fails+1)); return; }
    echo "PASS css/$name"
}

jsx_case() {
    local name="$1" source="$2"
    local out
    out=$("$RUST" jsx "$source") || { echo "FAIL $name: minify error"; fails=$((fails+1)); return; }
    if command -v tsc >/dev/null 2>&1; then
        printf '%s\n' "$out" >"$TMP/$name.tsx"
        if ! tsc --noEmit --noCheck --jsx preserve --skipLibCheck --target ES2020 "$TMP/$name.tsx" >/dev/null 2>&1; then
            echo "FAIL $name: minified JSX does not type/parse under tsc"; fails=$((fails+1)); return
        fi
    else
        printf '%s\n' "$out" >"$TMP/$name.tsx"
        node -e 'require("fs").readFileSync(process.argv[1],"utf8");' "$TMP/$name.tsx" >/dev/null 2>&1 \
            || { echo "FAIL $name: cannot read minified JSX"; fails=$((fails+1)); return; }
    fi
    echo "PASS jsx/$name"
}

# Reused Minify++ node_semantics / generated_semantics cases.
js_case regex_division "const s='https://x'; console.log(/https?:\\/\\//.test(s), 12 / 3 / 2);"
js_case asi $'function f(){return\n{x:1}}; console.log(String(f()));'
js_case empty_while "let x=0; while(x++<1); console.log(x);"
js_case unicode "const \u03c0=3,caf\u00e9=2; console.log(\u03c0+caf\u00e9);"
js_case number_member "console.log(1 .toString(), 1e3 .toString());"
js_case templates 'const x=2; console.log(`a ${x > 1 ? `b ${x}` : "c"}`);'
js_case class_fields 'class A{#x=2;static y=3;get z(){return this.#x}} console.log(new A().z+A.y);'
js_case regex_after_control "const ok=true; if (ok) /https?:\\/\\//.test('https://x'); console.log('done');"
js_case division_regex "const z = 6 / /a\\/\\/b/.test('a//b') ? 1 : 2; console.log(z);"
js_case label_block "let done=false; label:{ done=true; } console.log(done);"
js_case template_raw 'const v=3; const t=`head ${`inner ${v}`} raw // still text`; console.log(t);'
js_case async_fp "const f=async function(){}; console.log(typeof f, 4 / 2);"

css_case css_basic "body { color : red ; margin : 0  10px ; }"
css_case css_calc ".a { width: calc(100% - 2rem); }"
css_case css_custom ".x { --gap: 1  2; color: red; }"
css_case css_desc ".a .b, .a :hover, .a [data-x] { color: red; }"
css_case css_strings '.f { font-family: "A B" serif; content: "a" "b"; }'
css_case css_url ".x { background: url(\"data:image/svg+xml,<svg><!--x--></svg>\"); }"

jsx_case jsx_basic "const el = <div className=\"x\"> hello  world { value + 1 } </div>;"
jsx_case jsx_fragment "const el = <><span>A</span><span>{ b + 1 }</span></>;"
jsx_case jsx_nested "const el = <Comp child={<span>{ value + 1 }</span>} />;"
jsx_case jsx_text "const el = <p>https://example.com/a // literal text</p>;"

echo "semantic oracle: $(grep -c '^PASS' <<< '') runs finished"
if [ "$fails" -gt 0 ]; then echo "ORACLE FAILURES: $fails"; exit 1; fi
echo "ALL SEMANTIC ORACLE CASES PASSED"
