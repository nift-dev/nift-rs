//! In-process per-format benchmark (same inputs/options as the C++ bench).
use std::time::Instant;
fn main() {
    let cases: &[(minify::Format, &str, &str)] = &[
        (minify::Format::Html, "html", "<div  class=\"a\" >  <p> hello world </p>  <span> x </span> </div>"),
        (minify::Format::Css, "css", "body { color : red ; margin : 0  10px ; } /* c */ p { padding : 1px 2px ; }"),
        (minify::Format::Json, "json", "{ \"a\" : 1, \"b\" : [ 1, 2, 3 ], \"c\" : { \"d\" : \"e\" } }"),
        (minify::Format::Xml, "xml", "<a> <b  x=\"1\" > text </b> <c/> </a>"),
        (minify::Format::Svg, "svg", "<svg> <rect  width=\"10\"  height=\"10\" /> <!-- c --> </svg>"),
        (minify::Format::JavaScript, "js", "function f ( a , b ) { return a  +  b ; } // comment"),
        (minify::Format::Jsx, "jsx", "const el = <div  className=\"a\" > hello </div>;"),
    ];
    const ROUNDS: usize = 200_000;
    for (format, name, input) in cases {
        let start = Instant::now();
        let mut out_total = 0usize;
        for _ in 0..ROUNDS {
            out_total += minify::minify(*format, input).unwrap().len();
        }
        let secs = start.elapsed().as_secs_f64();
        let mibs = input.len() as f64 * ROUNDS as f64 / secs / (1024.0 * 1024.0);
        println!("{name}: Rust {mibs:.1} MiB/s input, {} bytes/out, {secs:.2}s", out_total / ROUNDS);
    }
}
