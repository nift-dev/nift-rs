#!/usr/bin/env python3
"""Minify++ <-> minify-rs differential runner (permanent gate).

Sends the same per-format corpus inputs through both implementations and
compares outputs byte-for-byte (the reference contract is exact output for
these cases). Reports divergences and a same-input per-format benchmark.
"""
import subprocess
import sys
import os

CPP = os.environ.get("MINIFY_CPP", "/tmp/minify_cpp")
RUST = os.environ.get("MINIFY_RUST", "target/release/examples/minify_cli")

CASES = [
    ("json", ' { "a" : 1, "s" : "a b", "x" : [ true, null ] } '),
    ("json", '{"x":[1, 2],"s":"a b"}'),
    ("css", "/*x*/ body  { color : red ; margin : 0  10px ; }"),
    ("css", "/*!license*/ .x { content: \"a  b\"; }"),
    ("css", "@laye/ *{.a{value:* /}}"),
    ("css", ".grid { grid-template-columns: 1.15fr .85fr; font: 700 .75rem sans-serif; padding: .1em .3em; }"),
    ("css", ".a .b, .a #id, .a :hover, .a [data-x], .a * { color: red; }"),
    ("css", "@media screen and (width > 10px) { .x { transform: translateX(1px) scale(2); color: color-mix(in srgb, var(--bg) 92%, transparent); } }"),
    ("css", "@media (prefers-color-scheme: dark) { .x { color: red; } }"),
    ("css", '.fonts { font-family: "A B" serif; content: "a" "b"; } * .item, [data-x] button { color: red; }'),
    ("css", "@media2 (width > 10px) { .x { color: red; } }"),
    ("css", ':root { --gap: calc(100% - 2rem); --blob: url("data:image/svg+xml,%3Csvg%20viewBox=\'0 0 1 1\'%3E%3C/svg%3E"); }'),
    ("css", "@container sidebar (width > 30rem) { .card { container-type: inline-size; } }"),
    ("css", "@layer reset, base, theme; @layer theme { .x { color: color(display-p3 1 0 0 / .5); } }"),
    ("css", "@supports selector(:has(*)) { .a:has(> .b) { width: clamp(1rem, 2vw + 1rem, 3rem); } }"),
    ("css", ".a { & > .b { --tokens: {a:b}; margin-inline: 1cqi; } }"),
    ("css", ".caf\u00e9 { --\u989c\u8272: red; }"),
    ("css", "a{/*"),
    ("css", "a{content:\"unterminated}"),
    ("html", "  <div   class=\"a  b\">  hello   world <!-- gone --> <span> x </span> </div>  "),
    ("html", "<pre>  a\n    b </pre><script> const x = ` a  b `;\n</script>"),
    ("html", "a<!--[if IE]>x<![endif]-->b"),
    ("html", "<span>a</span> <span>b</span><div> c </div>"),
    ("html", "<script><!-- not an html comment --></script><style>/* raw */ .x { a: b; }</style>"),
    ("html", "<div class=\"x\""),
    ("html", "<p>h\u00e9llo \U0001f600 \u4e16\u754c</p>"),
    ("html", "<!doctype html><template><span>A</span> <span>B</span></template>"),
    ("html", "<textarea>  alpha\n  beta &amp; gamma </textarea>"),
    ("html", "<script type=\"module\">const s='<!--'; const t='-->'; </script>"),
    ("html", "< !-- not-a-comment -->"),
    ("html", "a<!--x-->b"),
    ("html", "a <!--x--> b"),
    ("html", "<div>a</div><!--x--><div>b</div>"),
    ("html", "<span>A</span> <span>B</span>"),
    ("html", "<div>A</div>\n<div>B</div>"),
    ("html", "<script>const s=\"</scriptx>\";   const x = 1;</script>"),
    ("html", "<pre>a</prex>   b</pre>"),
    ("html", "<script>/* keep */ const x='<!-- keep -->';</script><style>/* keep */ .x { }</style>"),
    ("html", "<p>\u4f60\u597d   \U0001f600   caf\u00e9</p>"),
    ("html", "<div"),
    ("js", "const  x = 1; // comment\nconst y = x + 2;\n"),
    ("js", "const r = /https?:\\/\\/example\\.com/; /*x*/\nconst t=` a  b `;"),
    ("js", "return\n  value;"),
    ("js", "const x = value / *ptr; const y = left * /re/.test(s);"),
    ("js", "while (condition) ;\nnext();"),
    ("js", "const a=/[/]/g; const b=/a\\/\\/b/; const c=/[/*]/;"),
    ("js", "const t=`hello ${name} ${`nested ${value}`}`;\n"),
    ("js", "const el = <div className=\"x\">hello world {name}</div>;\n"),
    ("js", "type User = { name: string }; const x: User = { name: 'A' };\n"),
    ("js", "if (ok) /https?:\\/\\//.test(url);\n"),
    ("js", "while (ok) /a\\/\\/b/.test(s);\n"),
    ("js", "const z = value / /a\\/\\/b/.test(s);\n"),
    ("js", "const \u03c0 = 3.14; const \u4e16\u754c = 'ok';\n"),
    ("js", "const r = /unterminated\nx();"),
    ("js", "const x = 'unterminated"),
    ("js", "const x = `unterminated"),
    ("js", "const x = `head ${`inner ${v}`} raw // still text`;\n"),
    ("js", "try{}catch{} /https?:\\/\\//.test(s);"),
    ("js", "if(false){} /https?:\\/\\//.test(s);"),
    ("js", "const x=function(){} / 2;"),
    ("js", "const x=async function(){} / 2;"),
    ("js", "const x=class {static valueOf(){return 12}} / 2;"),
    ("js", "const x=class X {static valueOf(){return 12}} / 2;"),
    ("js", "class C{} /https?:\\/\\//.test(s);"),
    ("js", "class C{} /[/*}]/.test(s);"),
    ("js", "label:{} /https?:\\/\\//.test(s);"),
    ("js", "const x=true?1:{valueOf(){return 12}} / 2;"),
    ("js", "function f(){} /a/.test(s);"),
    ("js", "async function f(){for await(const x of xs) /a/.test(x);}"),
    ("js", "const n={valueOf(){return 12}} / 2;"),
    ("js", "const n=({valueOf(){return 12}}) / 2 / d;"),
    ("js", "while (condition);"),
    ("js", "if (x) { while (y); }"),
    ("js", "const a=/[/]/; const b=/a\\/b/g; const c=/[/*]/;"),
    ("js", "const \u03c0 = 3; return \u03c0;"),
    ("js", "const \u4f60\u597d = 1;"),
    ("js", "const s = 1 .toString();"),
    ("js", "const s = 0x1 .toString();"),
    ("js", "const s = 1e3 .toString();"),
    ("js", "const y = x / /a/.test(s);"),
    ("js", "const y = x / /[/*]/.test(s);"),
    ("js", "const x = a / b / c;"),
    ("js", "const t=`hello ${name} // not comment ${1+2}`;"),
    ("js", "const t=`outer ${`inner ${x}`}`;"),
    ("js", "function f(){return\n{x:1};}"),
    ("js", "a\n++b;"),
    ("js", "interface User { name: string }\nconst x: number = 1;"),
    ("js", "const el = <Button title=\"a  b\">{name}</Button>;"),
    ("js", "/*"),
    ("jsx", "const x=< mp<Map<string,number>> value={a} />;"),
    ("jsx", "const x=<span>https://example.com a  b</span>;\n"),
    ("jsx", "const x = <><span>a b</span><span>{value}</span></>;\n"),
    ("jsx", "const x=<p>https://example.com/a // literal text</p>;\n"),
    ("jsx", 'const el = <Widget className="card" data-id={value} aria-label="say \\"hello\\"">text</Widget>;'),
    ("jsx", "const  el = <div>https://example.com/{ name + 1 }</div>;"),
    ("jsx", "const x=<div> hello  world </div>;"),
    ("jsx", "const  el = <div className=\"x\"> hello  world {  value +  1  } </div> ;"),
    ("jsx", "const x=<><span>A</span><span>{ b + 1 }</span></>;"),
    ("jsx", "const x=<div>{a+1</div>;"),
    ("jsx", "const n=a<b&&c>d;"),
    ("jsx", "const x=foo<Bar>(baz);"),
    ("jsx", "function f(){return <Thing/>;}"),
    ("jsx", "const x=<div>{ /}/.test(s) }</div>;"),
    ("jsx", "const x=<div>{cond ? <span>https://example.com/x</span> : null}</div>;"),
    ("jsx", "const x=<Comp child={<span>{ value + 1 }</span>} />;"),
    ("jsx", "const x=<div>{ a /* } */ + b }</div>;"),
    ("jsx", "const x=<Thing value={ /}/.test(s) }/>;"),
    ("jsx", "const x = <Thing value={a > b ? x : y} />;"),
    ("jsx", "const x = <Thing value={{limit: a > b ? 2 : 1}} />;"),
    ("jsx", "const x = <Thing value={`x > ${a}`} />;"),
    ("jsx", "const x = <Thing value={ a + 1 } />;"),
    ("jsx", "const x = <><A/><B>{ {x: 1}.x }</B></>;"),
    ("xml", "< ![CDATA[a < b]]_>"),
    ("xml", "<?target  a   b?><root/>"),
    ("xml", "<?target missing"),
    ("xml", '<?xml version="1.0"?>\n<root>\n  <a x="1  2"> text  stays </a><!--x-->\n  <b><![CDATA[ a < b ]]></b>\n</root>'),
    ("xml", "<root"),
    ("xml", "<root><!--"),
    ("xml", "<p><b>A</b> <i>B</i></p>"),
    ("xml", '<root xmlns:x="urn:x"><x:item a="1 &amp; 2">A &lt; B</x:item></root>'),
    ("xml", "<root><![CDATA[ x ]]> text <![CDATA[ y < z ]]></root>"),
    ("xml", "<node data-x=\"unterminated>"),
    ("svg", "<text><tspan>A</tspan> <tspan>B</tspan></text>"),
    ("svg", '<svg viewBox="0 0 10 10"><path d="M 0 0 L 10 10 Z"/><text xml:space="preserve"> A  B </text></svg>'),
    ("svg", '<svg xmlns="http://www.w3.org/2000/svg">\n <text>hello   world</text>\n <path d="M 0 0 L 10 10" />\n</svg>'),
    ("svg", "<svg data-x=\"unterminated>"),
]


def run(binary, fmt, data):
    p = subprocess.run([binary, fmt, data], capture_output=True, text=True)
    return p.stdout


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "diff"
    divergences = 0
    for fmt, data in CASES:
        cpp = run(CPP, fmt, data)
        rust = run(RUST, fmt, data)
        if cpp != rust:
            divergences += 1
            print(f"DIVERGENCE [{fmt}]\n  in : {data!r}\n  C++ : {cpp!r}\n  Rust: {rust!r}")
    print(f"differential: {len(CASES) - divergences}/{len(CASES)} cases match, {divergences} divergences")

    return 1 if divergences else 0


if __name__ == "__main__":
    sys.exit(main())
