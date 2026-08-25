#!/usr/bin/env bash
# CSS semantic differential against minify-rs using the actual PostCSS oracle
# (adapted from Minify++ tests/minify_css_postcss_semantics.sh).
#
# This is the second type of evidence: Minify++ output == minify-rs output
# proves implementation parity, but only parsing BOTH with PostCSS and comparing
# the semantic trees proves the transformation preserves CSS semantics.
set -euo pipefail
RUST="${MINIFY_RUST:-target/release/examples/minify_cli}"
TMP=$(mktemp -d "${TMPDIR:-/tmp}/minify-postcss.XXXXXX")
trap 'rm -rf "$TMP"' EXIT

if ! node -e 'require("postcss"); require("postcss-selector-parser"); require("postcss-value-parser")' >/dev/null 2>&1; then
    echo "PostCSS not installed; CSS PostCSS semantic oracle SKIPPED"
    exit 0
fi

css_cases=(
'.grid { grid-template-columns: 1.15fr .85fr; font: 700 .75rem sans-serif; padding: .1em .3em; }'
'.a .b, .a #id, .a :hover, .a [data-x], .a * { color: red; }'
'* .item, [data-x] button, :is(.a, .b) > span { color: red; }'
'.fonts { font-family: "A B" serif; content: "a" "b"; }'
'.x { transform: translateX(1px) scale(2); filter: blur(1px) contrast(2); }'
'.x { color: color-mix(in srgb, var(--bg) 92%, transparent); }'
'.x { width: calc(100% - 2rem); height: min(10px + 2vw, 30px); }'
'@supports selector(:has(*)) { .a:has(> .b) { display: block; } }'
'@container sidebar (width > 30rem) { .card { container-type: inline-size; } }'
'@layer reset, base; @layer base { .a { color: display-p3(1 0 0); } }'
'.a { grid-template: "a a" 1fr "b c" 2fr / minmax(0, 1fr) auto; }'
'.a { --tokens: alpha  beta / gamma; animation: foo 1s steps(2, jump-none); }'
'.a { background: linear-gradient(45deg, red 0%, blue 100%); }'
'.a { & > .b { margin-inline: 1cqi; } }'
'@font-face { font-family: "Demo"; src: url(demo.woff2) format("woff2"); }'
)

for i in "${!css_cases[@]}"; do
  "$RUST" css "${css_cases[$i]}" >"$TMP/minified-$i.css" 2>/dev/null || {
    echo "FAIL case $i: minify-rs rejected valid CSS"; exit 1; }
  printf '%s' "${css_cases[$i]}" >"$TMP/original-$i.css"
done

node - "$TMP" "${#css_cases[@]}" <<'JS'
const fs = require("fs");
const postcss = require("postcss");
const selectorParser = require("postcss-selector-parser");
const valueParser = require("postcss-value-parser");
const directory = process.argv[2];
const caseCount = Number(process.argv[3]);

function semantic(node) {
  if (node.type === "comment") return null;
  const result = {type: node.type};
  for (const key of ["name", "prop", "important"]) {
    if (node[key] !== undefined) result[key] = node[key];
  }
  if (node.selector !== undefined) {
    result.selector = selectorParser().processSync(node.selector, {lossless: false});
  }
  for (const key of ["params", "value"]) {
    if (node[key] === undefined) continue;
    const parsed = valueParser(node[key]);
    parsed.walk(part => {
      if (part.type === "space") part.value = " ";
      if (part.before !== undefined) part.before = "";
      if (part.after !== undefined) part.after = "";
    });
    result[key] = parsed.toString().replace(/\s*([><=,\/])\s*/g, "$1");
  }
  if (node.nodes) result.nodes = node.nodes.map(semantic).filter(Boolean);
  return result;
}

for (let count = 0; count < caseCount; ++count) {
  const source = fs.readFileSync(`${directory}/original-${count}.css`, "utf8");
  const output = fs.readFileSync(`${directory}/minified-${count}.css`, "utf8");
  const before = semantic(postcss.parse(source));
  const after = semantic(postcss.parse(output));
  if (JSON.stringify(before) !== JSON.stringify(after)) {
    throw new Error(`CSS semantic tree changed\nsource: ${source}\noutput: ${output}\n` +
                    `before: ${JSON.stringify(before)}\nafter: ${JSON.stringify(after)}`);
  }
}
console.log(`CSS PostCSS semantic differential corpus passed (${caseCount} stylesheets)`);
JS
