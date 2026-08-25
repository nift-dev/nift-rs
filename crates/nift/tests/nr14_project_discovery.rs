//! NR14: project discovery/opening contract (pre-CP10).
//!
//! There is no global Nift configuration. A project exists only where the
//! relevant project state exists: `.nift/config.json` AND `.nift/tracked.json`
//! at the requested root. Opening/using a project from a non-project root must
//! report a controlled `NotProject` ("not a Nift project") error and must never
//! consult a global/legacy configuration source (e.g. a historical
//! `~/.nift/config.json` with old keys such as `lolcat-default`).
//!
//! Distinctions preserved:
//! - project absent                  -> NotProject ("not a Nift project")
//! - project config malformed        -> ConfigJson error, NOT NotProject
//! - project config has unknown key  -> ConfigKey error, NOT NotProject

use nift::project::ProjectErrorKind;
use nift::Engine;
use std::path::{Path, PathBuf};

fn write_file(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(path, contents).expect("write file");
}

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nift-nr14-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn valid_project(root: &Path) {
    write_file(
        &root.join(".nift/config.json"),
        r#"{"config":{"content-dir":"content/","output-dir":"public/","default-template":"templates/template.html"}}"#,
    );
    write_file(&root.join(".nift/tracked.json"), r#"{"tracked":[]}"#);
    write_file(&root.join("templates/template.html"), "@content\n");
}

#[test]
fn absent_project_is_not_project() {
    let root = temp_dir("absent");
    std::fs::create_dir_all(&root).expect("mkdir");

    let engine = Engine::project(&root);
    let error = engine.open_error().expect("must fail to open");
    assert_eq!(error.kind, ProjectErrorKind::NotProject);
    assert_eq!(error.message, "not a Nift project");
    assert_eq!(error.kind.corpus_class(), "not-a-project");

    // render_page on an unopened project reports the same controlled error.
    let result = engine.render_page("about", &nift::context::Context::new());
    let err = result.expect_err("render on non-project must fail");
    assert!(
        err.message.contains("not a Nift project"),
        "{}",
        err.message
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn historical_global_config_is_ignored() {
    // A root that only looks like the historical global config dir: config.json
    // with old keys, but NO tracked.json. It is not a project.
    let root = temp_dir("fake-global");
    write_file(
        &root.join(".nift/config.json"),
        r#"{"config":{"lolcat-default":true,"whatever-other-old-key":true}}"#,
    );

    let engine = Engine::project(&root);
    let error = engine.open_error().expect("must fail to open");
    assert_eq!(error.kind, ProjectErrorKind::NotProject);
    assert_eq!(error.message, "not a Nift project");
    // The global-style config keys were never parsed.
    assert!(!error.message.contains("lolcat"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn config_only_without_tracking_is_not_project() {
    let root = temp_dir("config-only");
    write_file(
        &root.join(".nift/config.json"),
        r#"{"config":{"content-dir":"content/","output-dir":"public/","default-template":"templates/template.html"}}"#,
    );
    let engine = Engine::project(&root);
    let error = engine.open_error().expect("must fail to open");
    assert_eq!(error.kind, ProjectErrorKind::NotProject);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn valid_project_opens() {
    let root = temp_dir("valid");
    valid_project(&root);
    let engine = Engine::project(&root);
    assert!(engine.is_open());
    assert!(engine.open_error().is_none());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn malformed_config_is_config_error_not_not_project() {
    let root = temp_dir("malformed");
    write_file(&root.join(".nift/config.json"), "{ not json");
    write_file(&root.join(".nift/tracked.json"), r#"{"tracked":[]}"#);

    let engine = Engine::project(&root);
    let error = engine.open_error().expect("must fail to open");
    assert_eq!(error.kind, ProjectErrorKind::ConfigJson);
    assert_ne!(error.kind, ProjectErrorKind::NotProject);
    assert!(
        error.message.contains("invalid project config"),
        "{}",
        error.message
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn unknown_config_key_is_config_key_error_not_not_project() {
    let root = temp_dir("unknown-key");
    write_file(
        &root.join(".nift/config.json"),
        r#"{"config":{"lolcat-default":true}}"#,
    );
    write_file(&root.join(".nift/tracked.json"), r#"{"tracked":[]}"#);

    let engine = Engine::project(&root);
    let error = engine.open_error().expect("must fail to open");
    assert_eq!(error.kind, ProjectErrorKind::ConfigKey);
    assert_ne!(error.kind, ProjectErrorKind::NotProject);
    assert!(
        error.message.contains("lolcat-default"),
        "{}",
        error.message
    );

    let _ = std::fs::remove_dir_all(&root);
}
