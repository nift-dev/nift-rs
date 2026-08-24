//! NR10: platform portability scrutiny.
//!
//! Hostile toward Unix-only assumptions: generic path spelling across native
//! separators, Unicode project paths and content filenames, atomic
//! replace-by-rename publication (the fixture helper the reload tests rely on),
//! and path spellings that must stay stable on Windows (`/` not `\`).

use nift::context::Context;
use nift::Engine;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "nift-nr10-port-{}-{}",
        std::process::id(),
        name.replace('/', "_")
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn write_file(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

const KCONFIG: &str = r#"{"config":{"content-dir":"content/","content-ext":".html","output-dir":"public/","output-ext":".html","default-template":"templates/template.html","incremental-mode":"modified"}}"#;

fn tracked(entries: &str) -> String {
    format!(r#"{{"tracked":[{entries}]}}"#)
}

#[test]
fn unicode_project_and_content_paths() {
    // A project whose root, content file and template all contain non-ASCII
    // characters must open, reload and render identically on every platform.
    let root = fixture("unicode-プロジェクト");
    write_file(&root.join(".nift/config.json"), KCONFIG);
    write_file(
        &root.join(".nift/tracked.json"),
        &tracked(r#"{"name":"/","title":"ホーム","template":"templates/template.html"}"#),
    );
    write_file(
        &root.join("templates/template.html"),
        "<main>@content</main>",
    );
    write_file(&root.join("content/index.html"), "<p>こんにちは 世界</p>");

    let engine = Engine::open(&root).unwrap();
    let result = engine.render_page("/", &Context::new()).unwrap();
    assert_eq!(result.output, "<main><p>こんにちは 世界</p></main>");

    // Reload round-trips the same Unicode paths.
    engine.reload().unwrap();
    let result = engine.render_page("/", &Context::new()).unwrap();
    assert_eq!(result.output, "<main><p>こんにちは 世界</p></main>");

    // The content-path metadata spellings are generic (`/`) even on Windows.
    assert!(result.dependencies.contains("content/index.html"));
    assert!(result.dependencies.contains("templates/template.html"));
}

#[test]
fn generic_relative_spelling_on_native_separators() {
    // ProjectState::relative must emit generic `/` spelling regardless of the
    // platform-native separator, matching C++ generic_string().
    let root = fixture("generic");
    write_file(&root.join(".nift/config.json"), KCONFIG);
    write_file(
        &root.join(".nift/tracked.json"),
        &tracked(r#"{"name":"about","title":"About","template":"templates/template.html"}"#),
    );
    write_file(
        &root.join("templates/template.html"),
        "<main>@content</main>",
    );
    write_file(&root.join("content/about.html"), "<p>a</p>");

    let engine = Engine::open(&root).unwrap();
    let result = engine.render_page("about", &Context::new()).unwrap();
    for dependency in &result.dependencies {
        assert!(
            !dependency.contains('\\'),
            "dependency '{dependency}' uses a native backslash separator"
        );
    }
    for requirement in &result.requirements {
        assert!(
            !requirement.contains('\\'),
            "requirement '{requirement}' uses a backslash"
        );
    }
    assert!(result.dependencies.contains("content/about.html"));
}

#[test]
fn atomic_rename_publication_is_a_single_file_operation() {
    // The reload fixture helper relies on rename() replacing the destination
    // atomically. On Windows this is the documented behaviour for files, but
    // the helper must not leave its temporary file behind and must preserve the
    // guarantee under repeated use.
    let root = fixture("atomic-rename");
    std::fs::create_dir_all(root.join(".nift")).unwrap();
    let path = root.join(".nift/tracked.json");
    std::fs::write(&path, "one").unwrap();

    for i in 0..20 {
        let tmp = path.with_extension(format!("tmp{i}"));
        std::fs::write(&tmp, format!("value-{i}")).unwrap();
        std::fs::rename(&tmp, &path).unwrap();
        assert!(!tmp.exists(), "temporary publication file was left behind");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("value-{i}")
        );
    }

    // No stray temporary files remain in the directory.
    let leftovers: Vec<_> = std::fs::read_dir(root.join(".nift"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(leftovers, vec!["tracked.json".to_string()]);
}

#[test]
fn dotdot_and_dot_spellings_in_tracked_names_are_rejected() {
    // Tracked-name validation is lexical and platform-neutral: `..`, absolute
    // spellings and backslash-parent spellings are rejected on every OS.
    let root = fixture("name-hostility");
    write_file(&root.join(".nift/config.json"), KCONFIG);
    for (name, expected_class) in [
        ("../escape", "invalid-tracking-json"),
        ("/abs", "invalid-tracking-json"),
        ("..\\escape", "invalid-tracking-json"),
    ] {
        let case_root = root.join(name.replace(['/', '\\'], "_"));
        std::fs::create_dir_all(case_root.join(".nift")).unwrap();
        write_file(&case_root.join(".nift/config.json"), KCONFIG);
        write_file(
            &case_root.join(".nift/tracked.json"),
            &tracked(&format!(r#"{{"name":"{name}","title":"X"}}"#)),
        );
        let error = Engine::open(&case_root)
            .err()
            .expect("expected rejection for tracked name");
        assert_eq!(error.kind.corpus_class(), expected_class, "name '{name}'");
    }
}

#[test]
fn platform_independent_output_path_geometry() {
    // Output path geometry uses `/`-joined generic spelling in metadata and
    // requirements regardless of the host filesystem.
    let root = fixture("output-geometry");
    write_file(&root.join(".nift/config.json"), KCONFIG);
    write_file(
        &root.join(".nift/tracked.json"),
        &tracked(
            r#"{"name":"/","title":"Home","template":"templates/template.html"},{"name":"blog/","title":"Blog","template":"templates/template.html"}"#,
        ),
    );
    write_file(
        &root.join("templates/template.html"),
        "<main>@content</main>",
    );
    write_file(&root.join("content/index.html"), "<p>home</p>");
    write_file(
        &root.join("content/blog/index.html"),
        "<p>home=@pathto(\"/\")</p>",
    );

    let engine = Engine::open(&root).unwrap();
    let result = engine.render_page("blog/", &Context::new()).unwrap();
    assert!(result.requirements.contains("public/index.html"));
    for requirement in &result.requirements {
        assert!(!requirement.contains('\\'));
    }
}
