//! Differential-testing CLI: `minify_cli <format> <input>` -> minified output
//! or `ERR:<message>`. Format names: html css js jsx json xml svg.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        std::process::exit(2);
    }
    let format = match args[1].as_str() {
        "html" => minify::Format::Html,
        "css" => minify::Format::Css,
        "js" => minify::Format::JavaScript,
        "jsx" => minify::Format::Jsx,
        "json" => minify::Format::Json,
        "xml" => minify::Format::Xml,
        "svg" => minify::Format::Svg,
        _ => std::process::exit(2),
    };
    match minify::minify(format, &args[2]) {
        Ok(out) => print!("{out}"),
        Err(e) => println!("ERR:{e}"),
    }
}
