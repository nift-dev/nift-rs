//! NR10: expanded cross-implementation behavioural differential corpus.
//!
//! Runs a substantially expanded battery of standalone render cases through the
//! frozen C++ Engine harness (`nift-embed/.build/engine-harness`) and this
//! crate's harness example (`engine_harness`) and requires byte-identical
//! observable JSON (output / dependencies / requirements / loaderKeys, or the
//! error). The canonical project-parity corpus (CLI vs C++ Engine vs golden vs
//! Rust Engine) is gated separately (`nr8_project_engine` + the C++ conformance
//! driver); this battery is the behavioural differential corpus.
//!
//! The harnesses must be built first:
//!   cargo build -p nift --example engine_harness
//!   (nift-embed) .build/engine-harness
//! Their paths can be overridden with NIFT_CPP_HARNESS / NIFT_RUST_HARNESS.
//! When the C++ harness is absent the test prints a notice and passes, so a
//! plain `cargo test` on a Rust-only checkout stays green; the CI workflow
//! builds both harnesses and enforces the real differential run.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Case {
    name: &'static str,
    page: &'static str,
    template: &'static str,
    page_name: &'static str,
    current_output: String,
    page_path: String,
    template_path: String,
    mode: &'static str,
    seam: &'static str,
    bindings: &'static [(&'static str, &'static str)],
}

macro_rules! case {
    ($name:literal, $page:literal, $template:literal) => {
        Case {
            name: $name,
            page: $page,
            template: $template,
            page_name: "-",
            current_output: String::new(),
            page_path: String::new(),
            template_path: String::new(),
            mode: "composed",
            seam: "-",
            bindings: &[],
        }
    };
    ($name:literal, $page:literal, $template:literal, bindings = $bindings:expr) => {
        Case {
            name: $name,
            page: $page,
            template: $template,
            page_name: "-",
            current_output: String::new(),
            page_path: String::new(),
            template_path: String::new(),
            mode: "composed",
            seam: "-",
            bindings: $bindings,
        }
    };
}

const DASH: &str = "-";

fn harness_bins() -> (Option<PathBuf>, PathBuf) {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cpp = std::env::var("NIFT_CPP_HARNESS")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            let p = crate_dir.join("../../../nift-embed/.build/engine-harness");
            if p.is_file() {
                Some(p)
            } else {
                None
            }
        });
    let rust = std::env::var("NIFT_RUST_HARNESS")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let mut p = crate_dir.join("../../target/debug/examples/engine_harness");
            #[cfg(windows)]
            p.set_extension("exe");
            p
        });
    (cpp, rust)
}

fn run_harness(bin: &Path, root: &Path, case: &Case) -> String {
    let mut child = Command::new(bin)
        .arg(root)
        .arg(if case.page.is_empty() {
            DASH
        } else {
            case.page
        })
        .arg(if case.template.is_empty() {
            DASH
        } else {
            case.template
        })
        .arg(if case.page_name.is_empty() {
            DASH
        } else {
            case.page_name
        })
        .arg(if case.current_output.is_empty() {
            DASH
        } else {
            &case.current_output
        })
        .arg(if case.page_path.is_empty() {
            DASH
        } else {
            &case.page_path
        })
        .arg(if case.template_path.is_empty() {
            DASH
        } else {
            &case.template_path
        })
        .arg(case.mode)
        .arg(case.seam)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to run {}: {e}", bin.display()));
    let mut input = String::new();
    for (name, value) in case.bindings {
        input.push_str(name);
        input.push('=');
        input.push_str(value);
        input.push('\n');
    }
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    String::from_utf8(output.stdout).expect("harness stdout is UTF-8")
}

fn static_cases() -> Vec<Case> {
    vec![
        // --- literals, escaping, comments, entities -------------------------------
        case!("literal-plain", "hello world", "<main>@content</main>"),
        case!("literal-empty", "", "<main>@content</main>"),
        case!("literal-unicode", "café 東京 🚀", "<main>@content</main>"),
        case!(
            "literal-unicode-for-body",
            "@for(x : items){<p>café $[x]</p>}",
            "<main>@content</main>",
            bindings = &[("items", "json:[\"a\",\"b\"]")]
        ),
        case!(
            "literal-dollars",
            "cost $5 and $[not-a-binding]",
            "<main>@content</main>"
        ),
        case!(
            "literal-at-not-directive",
            "email a@b.c and @@ and @x",
            "<main>@content</main>"
        ),
        case!(
            "literal-backslash",
            r#"back\slash and 'quote' and \"dquote\""#,
            "<main>@content</main>"
        ),
        case!(
            "escaped-at",
            "\\@content is literal",
            "<main>@content</main>"
        ),
        case!(
            "entity-direct",
            "&lt; &amp; &#39; &#x27;",
            "<main>@content</main>"
        ),
        case!("entity-bang", "@ent('!')", "<main>@content</main>"),
        case!("entity-arrow", "@ent('->')", "<main>@content</main>"),
        case!(
            "html-comment",
            "<!-- a comment -->text",
            "<main>@content</main>"
        ),
        case!(
            "block-comment",
            "<#-- hidden -->text",
            "<main>@content</main>"
        ),
        case!("line-comment", "a@# hidden\nb", "<main>@content</main>"),
        // --- bindings: scalars, objects, arrays -----------------------------------
        case!(
            "binding-string",
            "$[name]",
            "<main>@content</main>",
            bindings = &[("name", "World")]
        ),
        case!(
            "binding-number-int",
            "$[n]",
            "<main>@content</main>",
            bindings = &[("n", "json:42")]
        ),
        case!(
            "binding-number-negative",
            "$[n]",
            "<main>@content</main>",
            bindings = &[("n", "json:-3.25")]
        ),
        case!(
            "binding-bool",
            "$[b]",
            "<main>@content</main>",
            bindings = &[("b", "json:true")]
        ),
        case!(
            "binding-null",
            "$[x]",
            "<main>@content</main>",
            bindings = &[("x", "json:null")]
        ),
        case!(
            "binding-object-member",
            "$[user.name]",
            "<main>@content</main>",
            bindings = &[("user", "json:{\"name\":\"Ada\"}")]
        ),
        case!(
            "binding-object-nested",
            "$[a.b.c]",
            "<main>@content</main>",
            bindings = &[("a", "json:{\"b\":{\"c\":7}}")]
        ),
        case!(
            "binding-array-index",
            "$[items[0]]",
            "<main>@content</main>",
            bindings = &[("items", "json:[\"a\",\"b\"]")]
        ),
        case!(
            "binding-string-concat",
            "$[greet + ' ' + name]",
            "<main>@content</main>",
            bindings = &[("greet", "Hi"), ("name", "Bob")]
        ),
        case!(
            "binding-number-arithmetic",
            "$[a + b * 2 - 1]",
            "<main>@content</main>",
            bindings = &[("a", "json:10"), ("b", "json:3")]
        ),
        case!(
            "binding-division",
            "$[a / b]",
            "<main>@content</main>",
            bindings = &[("a", "json:10"), ("b", "json:4")]
        ),
        case!(
            "binding-modulo",
            "$[a % b]",
            "<main>@content</main>",
            bindings = &[("a", "json:10"), ("b", "json:3")]
        ),
        case!(
            "binding-comparison",
            "$[a == b]",
            "<main>@content</main>",
            bindings = &[("a", "json:5"), ("b", "json:5")]
        ),
        case!(
            "binding-comparison-ne",
            "$[a != b]",
            "<main>@content</main>",
            bindings = &[("a", "json:5"), ("b", "json:6")]
        ),
        case!(
            "binding-comparison-lt",
            "$[a < b]",
            "<main>@content</main>",
            bindings = &[("a", "json:1"), ("b", "json:2")]
        ),
        case!(
            "binding-ternary",
            "$[n > 10 ? 'big' : 'small']",
            "<main>@content</main>",
            bindings = &[("n", "json:42")]
        ),
        case!(
            "binding-ternary-false-branch",
            "$[n > 10 ? 'big' : 'small']",
            "<main>@content</main>",
            bindings = &[("n", "json:5")]
        ),
        case!(
            "binding-ternary-lazy-directive",
            "$[ok ? '@getenv(NIFT_DIFF_KEY)' : 'no']",
            "<main>@content</main>",
            bindings = &[("ok", "json:true")]
        ),
        case!(
            "binding-logical-and",
            "$[a && b]",
            "<main>@content</main>",
            bindings = &[("a", "json:true"), ("b", "json:false")]
        ),
        case!(
            "binding-logical-or",
            "$[a || b]",
            "<main>@content</main>",
            bindings = &[("a", "json:false"), ("b", "json:true")]
        ),
        case!(
            "binding-not",
            "$[!a]",
            "<main>@content</main>",
            bindings = &[("a", "json:true")]
        ),
        case!(
            "binding-unary-minus",
            "$[-x]",
            "<main>@content</main>",
            bindings = &[("x", "json:7")]
        ),
        // --- metadata --------------------------------------------------------------
        case!("meta-title", "$[title]", "<main>@content</main>"),
        case!("meta-name", "$[name]", "<main>@content</main>"),
        // --- @if / @else / @else if ------------------------------------------------
        case!(
            "if-true",
            "@if(x){yes}@else{no}",
            "<main>@content</main>",
            bindings = &[("x", "json:true")]
        ),
        case!(
            "if-false",
            "@if(x){yes}@else{no}",
            "<main>@content</main>",
            bindings = &[("x", "json:false")]
        ),
        case!(
            "if-number-nonzero",
            "@if(x){yes}@else{no}",
            "<main>@content</main>",
            bindings = &[("x", "json:3")]
        ),
        case!(
            "if-zero",
            "@if(x){yes}@else{no}",
            "<main>@content</main>",
            bindings = &[("x", "json:0")]
        ),
        case!(
            "if-empty-string",
            "@if(x){yes}@else{no}",
            "<main>@content</main>",
            bindings = &[("x", "json:\"\"")]
        ),
        case!(
            "if-null",
            "@if(x){yes}@else{no}",
            "<main>@content</main>",
            bindings = &[("x", "json:null")]
        ),
        case!(
            "if-comparison",
            "@if(n >= 5){yes}@else{no}",
            "<main>@content</main>",
            bindings = &[("n", "json:5")]
        ),
        case!(
            "if-comparison-type-mismatch",
            "@if(n >= 5){yes}@else{no}",
            "<main>@content</main>",
            bindings = &[("n", "5")]
        ),
        case!(
            "if-else-if",
            "@if(a){A}@else if(b){B}@else{C}",
            "<main>@content</main>",
            bindings = &[("a", "json:false"), ("b", "json:true")]
        ),
        case!(
            "if-else-if-chain",
            "@if(a){A}@else if(b){B}@else if(c){C}@else{D}",
            "<main>@content</main>",
            bindings = &[
                ("a", "json:false"),
                ("b", "json:false"),
                ("c", "json:false")
            ]
        ),
        case!(
            "if-multiline",
            "@if(x){\n  yes\n}",
            "<main>@content</main>",
            bindings = &[("x", "json:true")]
        ),
        // --- @for arrays -----------------------------------------------------------
        case!(
            "for-array-basic",
            "@for(x : items){<$[x]>}",
            "<main>@content</main>",
            bindings = &[("items", "json:[\"a\",\"b\",\"c\"]")]
        ),
        case!(
            "for-array-loop-index",
            "@for(x : items){$[loop.index]}",
            "<main>@content</main>",
            bindings = &[("items", "json:[\"a\",\"b\"]")]
        ),
        case!(
            "for-array-loop-count",
            "@for(x : items){$[loop.count]}",
            "<main>@content</main>",
            bindings = &[("items", "json:[\"a\",\"b\",\"c\"]")]
        ),
        case!(
            "for-array-loop-first",
            "@for(x : items){$[loop.first]}",
            "<main>@content</main>",
            bindings = &[("items", "json:[\"a\",\"b\"]")]
        ),
        case!(
            "for-array-loop-last",
            "@for(x : items){$[loop.last]}",
            "<main>@content</main>",
            bindings = &[("items", "json:[\"a\",\"b\"]")]
        ),
        case!(
            "for-array-sort-string",
            "@for(x : items by x asc){<$[x]>}",
            "<main>@content</main>",
            bindings = &[("items", "json:[\"b\",\"a\",\"c\"]")]
        ),
        case!(
            "for-array-sort-desc",
            "@for(x : items by x desc){<$[x]>}",
            "<main>@content</main>",
            bindings = &[("items", "json:[\"b\",\"a\",\"c\"]")]
        ),
        case!(
            "for-array-sort-member",
            "@for(x : items by x.n desc){<$[x.n]>}",
            "<main>@content</main>",
            bindings = &[("items", "json:[{\"n\":2},{\"n\":1},{\"n\":3}]")]
        ),
        case!(
            "for-array-numeric",
            "@for(x : nums){<$[x]>}",
            "<main>@content</main>",
            bindings = &[("nums", "json:[3,1,2]")]
        ),
        case!(
            "for-array-multiline",
            "@for(x : items){\n  <$[x]>\n}",
            "<main>@content</main>",
            bindings = &[("items", "json:[\"a\",\"b\"]")]
        ),
        case!(
            "for-array-objects",
            "@for(x : items){$[x.name]-$[x.v]}",
            "<main>@content</main>",
            bindings = &[(
                "items",
                "json:[{\"name\":\"a\",\"v\":1},{\"name\":\"b\",\"v\":2}]"
            )]
        ),
        case!(
            "for-array-nested",
            "@for(x : rows){@for(y : x){<$[y]>}}",
            "<main>@content</main>",
            bindings = &[("rows", "json:[[1,2],[3,4]]")]
        ),
        // --- @for objects ----------------------------------------------------------
        case!(
            "for-object",
            "@for((k, v) : obj){$[k]=$[v];}",
            "<main>@content</main>",
            bindings = &[("obj", "json:{\"a\":1,\"b\":2}")]
        ),
        case!(
            "for-object-order",
            "@for((k, v) : obj){$[k]}",
            "<main>@content</main>",
            bindings = &[("obj", "json:{\"z\":1,\"a\":2,\"m\":3}")]
        ),
        case!(
            "for-object-loop-index",
            "@for((k, v) : obj){$[loop.index]:$[k]}",
            "<main>@content</main>",
            bindings = &[("obj", "json:{\"a\":1,\"b\":2}")]
        ),
        // --- collection directives (reference @... advanced forms) ----------------
        case!(
            "directive-sort",
            "@sort(nums)",
            "<main>@content</main>",
            bindings = &[("nums", "json:[3,1,2]")]
        ),
        case!(
            "directive-filter-in-for",
            "@for(x : @sort(items)){<$[x]>}",
            "<main>@content</main>",
            bindings = &[("items", "json:[3,1,2]")]
        ),
        case!(
            "directive-map",
            "@map(p : items => p.n * 2)",
            "<main>@content</main>",
            bindings = &[("items", "json:[{\"n\":1},{\"n\":2}]")]
        ),
        case!(
            "directive-filter",
            "@filter(p : items => p.n > 1)",
            "<main>@content</main>",
            bindings = &[("items", "json:[{\"n\":1},{\"n\":2},{\"n\":3}]")]
        ),
        case!(
            "directive-distinct",
            "@distinct(items)",
            "<main>@content</main>",
            bindings = &[("items", "json:[1,1,2,3,3]")]
        ),
        case!(
            "directive-reverse",
            "@reverse(items)",
            "<main>@content</main>",
            bindings = &[("items", "json:[1,2,3]")]
        ),
        case!(
            "directive-slice",
            "@slice(items, 1, 2)",
            "<main>@content</main>",
            bindings = &[("items", "json:[1,2,3,4]")]
        ),
        // --- @content --------------------------------------------------------------
        case!("content-text", "<p>PAGE</p>", "<main>@content</main>"),
        case!("content-empty", "", "<main>@content</main>"),
        case!(
            "content-multiline-page",
            "<p>a\nb</p>",
            "<main>@content</main>"
        ),
        case!(
            "content-trailing-newline",
            "<p>a</p>\n",
            "<main>@content</main>"
        ),
        case!(
            "content-unicode",
            "<p>café 東京</p>",
            "<main>@content</main>"
        ),
        case!("partial-no-content", "<p>fragment</p>", ""),
        // --- @getenv -----------------------------------------------------------------
        case!(
            "getenv-set",
            "@getenv(NIFT_DIFF_KEY)",
            "<main>@content</main>"
        ),
        case!(
            "getenv-unset",
            "@getenv(NIFT_DIFF_UNSET_KEY)",
            "<main>@content</main>"
        ),
        case!(
            "getenv-quoted",
            "@getenv('NIFT_DIFF_KEY')",
            "<main>@content</main>"
        ),
        // --- @dep -------------------------------------------------------------------
        case!("dep-single", "@dep('app.js')", "<main>@content</main>"),
        case!(
            "dep-multiple",
            "@dep('a.js','b.css')",
            "<main>@content</main>"
        ),
        // --- errors ------------------------------------------------------------------
        case!(
            "error-unknown-binding",
            "$[missing.member]",
            "<main>@content</main>"
        ),
        case!(
            "error-bad-json-binding",
            "$[n.x]",
            "<main>@content</main>",
            bindings = &[("n", "json:5")]
        ),
        case!(
            "error-unterminated-if",
            "@if(x){unclosed",
            "<main>@content</main>",
            bindings = &[("x", "json:true")]
        ),
        case!(
            "error-unbalanced-for",
            "@for(x : items){<$[x]>",
            "<main>@content</main>",
            bindings = &[("items", "json:[1,2]")]
        ),
        case!("error-getenv-arity", "@getenv()", "<main>@content</main>"),
        case!("error-pathto-arity", "@pathto()", "<main>@content</main>"),
        case!(
            "error-double-content",
            "@content@content",
            "<main>@content</main>"
        ),
        case!("error-partial-with-content", "@content", ""),
        case!(
            "error-item-outside-pagination",
            "@item{x}",
            "<main>@content</main>"
        ),
        case!(
            "error-paginate-outside-pagination",
            "@paginate",
            "<main>@content</main>"
        ),
        case!(
            "error-collection-bare-function",
            "@for(x : filter(items, x -> x.n > 1)){<$[x.n]>}",
            "<main>@content</main>",
            bindings = &[("items", "json:[{\"n\":1},{\"n\":2}]")]
        ),
    ]
}
fn dynamic_cases(root: &Path) -> Vec<Case> {
    // Concrete absolute paths for current_output / path sources / loader seam.
    let co = root.join("public/about.html").to_string_lossy().to_string();
    let co_index = root.join("public/index.html").to_string_lossy().to_string();
    let blog = root.join("content/blog.html").to_string_lossy().to_string();
    let post = root.join("content/post.html").to_string_lossy().to_string();
    let template = root
        .join("templates/template.html")
        .to_string_lossy()
        .to_string();

    vec![
        // @input with an on-disk source.
        Case {
            name: "input-path",
            page: "@input(\"part.html\")",
            template: "<main>@content</main>",
            page_name: "-",
            current_output: String::new(),
            page_path: blog.clone(),
            template_path: String::new(),
            mode: "composed",
            seam: "-",
            bindings: &[],
        },
        // Path-backed composed render (Source::Path page + template).
        Case {
            name: "path-composed",
            page: "-",
            template: "-",
            page_name: "-",
            current_output: String::new(),
            page_path: blog.clone(),
            template_path: template.clone(),
            mode: "composed",
            seam: "-",
            bindings: &[],
        },
        // @pathto with an explicit output context.
        Case {
            name: "pathto-concrete",
            page: "@pathto('app.js')",
            template: "<main>@content</main>",
            page_name: "-",
            current_output: co.clone(),
            page_path: String::new(),
            template_path: String::new(),
            mode: "composed",
            seam: "-",
            bindings: &[],
        },
        Case {
            name: "pathto-index-page",
            page: "@pathto('index.html')@content",
            template: "<main>@content</main>",
            page_name: "-",
            current_output: co.clone(),
            page_path: String::new(),
            template_path: String::new(),
            mode: "composed",
            seam: "-",
            bindings: &[],
        },
        Case {
            name: "pathtofile-concrete",
            page: "@pathtofile('app.js')@content",
            template: "<main>@content</main>",
            page_name: "-",
            current_output: co.clone(),
            page_path: String::new(),
            template_path: String::new(),
            mode: "composed",
            seam: "-",
            bindings: &[],
        },
        Case {
            name: "pathto-nested",
            page: "@pathto('assets/css/main.css')@content",
            template: "<main>@content</main>",
            page_name: "-",
            current_output: co.clone(),
            page_path: String::new(),
            template_path: String::new(),
            mode: "composed",
            seam: "-",
            bindings: &[],
        },
        Case {
            name: "pathto-404-absolute",
            page: "@pathto('app.js')@content",
            template: "<main>@content</main>",
            page_name: "404",
            current_output: co.clone(),
            page_path: String::new(),
            template_path: String::new(),
            mode: "composed",
            seam: "-",
            bindings: &[],
        },
        // Partial render mode.
        Case {
            name: "partial-mode",
            page: "<p>fragment</p>",
            template: "-",
            page_name: "-",
            current_output: String::new(),
            page_path: String::new(),
            template_path: String::new(),
            mode: "partial",
            seam: "-",
            bindings: &[],
        },
        // Loader seam: path keys + loader-provided content.
        Case {
            name: "loader-path-keys",
            page: "-",
            template: "-",
            page_name: "-",
            current_output: String::new(),
            page_path: blog.clone(),
            template_path: template.clone(),
            mode: "composed",
            seam: "loader",
            bindings: &[],
        },
        Case {
            name: "loader-input-relative",
            page: "-",
            template: "<main>@content</main>",
            page_name: "-",
            current_output: String::new(),
            page_path: post.clone(),
            template_path: String::new(),
            mode: "composed",
            seam: "loader",
            bindings: &[],
        },
        // Environment provider seam.
        Case {
            name: "env-provider",
            page: "@getenv(NIFT_ENV_A)|@getenv(NIFT_ENV_MISSING)",
            template: "<main>@content</main>",
            page_name: "-",
            current_output: String::new(),
            page_path: String::new(),
            template_path: String::new(),
            mode: "composed",
            seam: "env",
            bindings: &[],
        },
        // Content-path / output-path metadata with an output context.
        Case {
            name: "meta-output-path",
            page: "$[content-path]|$[output-path]@content",
            template: "<main>@content</main>",
            page_name: "-",
            current_output: co_index.clone(),
            page_path: String::new(),
            template_path: String::new(),
            mode: "composed",
            seam: "-",
            bindings: &[],
        },
        // Concrete @dep + @pathto against on-disk sources.
        Case {
            name: "dep-path-exists",
            page: "@dep('app.js')@pathto('app.js')",
            template: "<main>@content</main>",
            page_name: "-",
            current_output: co.clone(),
            page_path: String::new(),
            template_path: String::new(),
            mode: "composed",
            seam: "-",
            bindings: &[],
        },
    ]
}

fn prepare_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("nift-nr10-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("content")).unwrap();
    std::fs::create_dir_all(root.join("templates")).unwrap();
    std::fs::create_dir_all(root.join("public")).unwrap();
    std::fs::create_dir_all(root.join("assets/css")).unwrap();
    std::fs::write(root.join("content/part.html"), "<p>PART</p>\n").unwrap();
    std::fs::write(
        root.join("templates/template.html"),
        "<main>@content</main>\n",
    )
    .unwrap();
    std::fs::write(root.join("app.js"), "/* app */\n").unwrap();
    std::fs::write(root.join("assets/css/main.css"), "body{}\n").unwrap();
    std::fs::write(root.join("content/blog.html"), "<p>PATH-CONTENT</p>\n").unwrap();
    std::fs::write(root.join("content/post.html"), "@input(\"part.html\")\n").unwrap();
    root
}

#[test]
fn expanded_behavioural_differential() {
    let (cpp, rust) = harness_bins();
    let Some(cpp) = cpp else {
        eprintln!("NR10: C++ harness not built (NIFT_CPP_HARNESS or nift-embed/.build/engine-harness); skipping differential");
        return;
    };
    if !rust.is_file() {
        panic!(
            "Rust harness not found at {} (cargo build -p nift --example engine_harness)",
            rust.display()
        );
    }

    let root = prepare_root();
    std::env::set_var("NIFT_DIFF_KEY", "env-value-1");

    let mut cases = static_cases();
    cases.extend(dynamic_cases(&root));

    let mut mismatches: Vec<String> = Vec::new();
    for case in &cases {
        let cpp_out = run_harness(&cpp, &root, case);
        let rust_out = run_harness(&rust, &root, case);
        if cpp_out != rust_out {
            mismatches.push(format!(
                "case '{}':\n  C++ : {}\n  Rust: {}",
                case.name, cpp_out, rust_out
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "{} unexplained divergence(s) in the expanded differential corpus:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    eprintln!(
        "NR10 expanded differential: {} cases, zero divergence",
        cases.len()
    );
}
