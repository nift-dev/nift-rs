// CP19 render API conformance: render_page(name) / render_path(path) /
// render_text(text) with stable meanings and no existence-based dispatch.
use nift::{Context, Engine, Source};

fn write(path: &std::path::Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

#[test]
fn render_path_and_text_are_typed() {
    let root = std::env::temp_dir().join(format!("nift-cp19-rs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    write(
        &root.join(".nift/config.json"),
        r#"{"config":{"content-dir":"content/","content-ext":".html","output-dir":"public/","output-ext":".html","default-template":"templates/template.html","incremental-mode":"modified"}}"#,
    );
    write(
        &root.join(".nift/tracked.json"),
        r#"{"tracked":[{"name":"about","title":"About","template":"templates/template.html"}]}"#,
    );
    write(
        &root.join("templates/template.html"),
        "<main>@content</main>",
    );
    write(&root.join("content/about.html"), "<p>about</p>");

    let engine = Engine::open(&root).expect("project opens");
    let ctx = Context::new();

    // render_page: tracked page by name.
    let by_name = engine.render_page("about", &ctx).expect("render");
    assert_eq!(by_name.output, "<main><p>about</p></main>");
    let unknown = engine.render_page("no-such-page", &ctx);
    assert!(unknown.is_err(), "unknown tracked name must error");

    // render_path: always a filesystem path; missing path is an error, never
    // reinterpreted as text.
    let via_path = engine
        .render_path(root.join("content/about.html"), &ctx)
        .expect("render_path existing");
    assert_eq!(via_path.output, "<p>about</p>");
    assert!(
        engine.render_path(root.join("nope.html"), &ctx).is_err(),
        "render_path missing must error"
    );

    // render_text: never checks the filesystem; a string naming an existing
    // file still renders as literal text.
    let via_text = engine
        .render_text("<p>literal</p>", &ctx)
        .expect("render_text");
    assert_eq!(via_text.output, "<p>literal</p>");
    let names_a_file = root
        .join("content/about.html")
        .to_string_lossy()
        .to_string();
    let literal = engine
        .render_text(&names_a_file, &ctx)
        .expect("render_text literal");
    assert_eq!(literal.output, names_a_file);

    // Typed Source composition still works (path/path, text/text).
    let pp = engine
        .render(
            &Source::path(root.join("content/about.html")),
            &Source::path(root.join("templates/template.html")),
            &ctx,
        )
        .expect("path/path composition");
    assert_eq!(pp.output, "<main><p>about</p></main>");
    let tt = engine
        .render(
            &Source::text("<p>hi</p>"),
            &Source::text("<main>@content</main>"),
            &ctx,
        )
        .expect("text/text composition");
    assert_eq!(tt.output, "<main><p>hi</p></main>");

    let _ = std::fs::remove_dir_all(&root);
}
