//! Rust `build --repair` parity tests (accepted C++ contract).
//!
//! The Rust engine exposes only the PRIMARY pagination page, so these tests
//! cover the reconstructible surface (non-paginated reconstruction +
//! convergence, ownership-aware sweep, marker lifecycle, failure semantics).
//! The pagination-page-2..N engine gap is documented in repair.rs.

use std::fs;
use std::path::{Path, PathBuf};

use nift::repair::{repair_project, Ownership, OwnershipState};

fn scaffold(root: &Path) {
    for d in [".nift", "content", "templates", "public"] {
        fs::create_dir_all(root.join(d)).unwrap();
    }
    fs::write(
        root.join(".nift/config.json"),
        r#"{"config":{"content-dir":"content/","output-dir":"public/",
           "default-template":"templates/template.html","build-threads":-1,
           "incremental-mode":"modified"}}"#,
    )
    .unwrap();
    fs::write(
        root.join(".nift/tracked.json"),
        r#"{"tracked":[
          {"name":"/","title":"Home","template":"templates/template.html"},
          {"name":"about","title":"About","template":"templates/template.html"}]}"#,
    )
    .unwrap();
    fs::write(
        root.join("templates/template.html"),
        "<main>@content</main>\n",
    )
    .unwrap();
    fs::write(root.join("content/index.html"), "<p>home</p>\n").unwrap();
    fs::write(root.join("content/about.html"), "<p>about</p>\n").unwrap();
}

fn marker(root: &Path) -> bool {
    root.join(".nift/.unfinished").exists()
}

fn tree(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let public = root.join("public");
    for entry in fs::read_dir(&public).unwrap() {
        let path = entry.unwrap().path();
        if path.is_file() {
            out.push((
                path.file_name().unwrap().to_string_lossy().into_owned(),
                fs::read(&path).unwrap(),
            ));
        }
    }
    out.sort();
    out
}

fn render_primary(root: &Path, page: &str) -> String {
    // The engine's render_page output is the canonical page output (conformance
    // corpus goldens validate Rust-engine == C++-build bytes).
    let engine = nift::engine::Engine::open(root).unwrap();
    let result = engine
        .render_page(page, &nift::context::Context::new())
        .unwrap();
    result.output
}

#[test]
fn non_paginated_repair_converges_and_is_idempotent() {
    let root = tempdir("nr-repair-converge");
    scaffold(&root);
    // First repair establishes the derived tree.
    repair_project(&root).expect("initial repair succeeds");
    assert!(!marker(&root), "marker cleared on success");
    let expected = tree(&root);
    assert!(!expected.is_empty(), "outputs reconstructed");

    // Delete an output and corrupt another; repair must restore and converge.
    fs::remove_file(root.join("public/about.html")).unwrap();
    fs::write(root.join("public/index.html"), "<p>TORN").unwrap();
    repair_project(&root).expect("repair after corruption succeeds");
    assert_eq!(tree(&root), expected, "tree converges after corruption");
    assert!(!marker(&root));

    // Idempotence: a second repair produces the identical tree.
    repair_project(&root).expect("second repair succeeds");
    assert_eq!(tree(&root), expected, "second repair identical");
    assert!(!marker(&root));

    // The reconstructed output matches the engine's canonical render.
    assert_eq!(
        fs::read_to_string(root.join("public/index.html")).unwrap(),
        render_primary(&root, "/")
    );
}

#[test]
fn orphan_metadata_removed_public_output_preserved() {
    let root = tempdir("nr-repair-orphan");
    scaffold(&root);
    repair_project(&root).expect("repair succeeds");
    // Simulate a removed page: an orphan .info.json whose historical output
    // path is only knowable from the (distrusted) derived metadata.
    fs::write(
        root.join(".nift/public/old.info.json"),
        r#"{"name":"old","output":"public/old.html","pagination-pages":3}"#,
    )
    .unwrap();
    fs::write(root.join("public/old.html"), "<p>old</p>\n").unwrap();
    fs::write(root.join("public/keepme.txt"), "USER FILE\n").unwrap();

    repair_project(&root).expect("repair succeeds");
    assert!(
        !root.join(".nift/public/old.info.json").exists(),
        "orphan .info.json removed"
    );
    assert!(
        root.join("public/old.html").exists(),
        "historical public output preserved (conservative orphan rule)"
    );
    assert!(
        root.join("public/keepme.txt").exists(),
        "user file preserved"
    );
}

#[test]
fn hostile_orphan_metadata_cannot_delete_user_file() {
    let root = tempdir("nr-repair-hostile");
    scaffold(&root);
    repair_project(&root).expect("repair succeeds");
    // A corrupt-but-valid orphan .info.json names a user-managed path.
    fs::write(
        root.join(".nift/public/evil.info.json"),
        r#"{"output":"public/keepme.txt"}"#,
    )
    .unwrap();
    fs::write(root.join("public/keepme.txt"), "USER DATA\n").unwrap();
    repair_project(&root).expect("repair succeeds");
    assert!(
        root.join("public/keepme.txt").exists(),
        "derived metadata never authorizes deleting a public file"
    );
    assert!(
        !root.join(".nift/public/evil.info.json").exists(),
        "orphan metadata itself is removed"
    );
}

#[test]
fn repair_refuses_live_owner() {
    let root = tempdir("nr-repair-live");
    scaffold(&root);
    let (holder, state) = Ownership::acquire(root.join(".nift/.unfinished"));
    assert_eq!(state, OwnershipState::Clean, "holder acquires clean");
    let err = repair_project(&root).unwrap_err();
    assert!(
        err.message.contains("another build appears to be running"),
        "repair refuses a live owner: {err}"
    );
    drop(holder); // simulated crash: lock released by the kernel, marker survives
    assert!(
        marker(&root),
        "marker survives process death (stale evidence)"
    );
    repair_project(&root).expect("repair may take a stale marker");
    assert!(!marker(&root), "marker cleared after repair");
}

#[test]
fn repair_failure_retains_marker() {
    let root = tempdir("nr-repair-failure");
    scaffold(&root);
    repair_project(&root).expect("initial repair succeeds");
    // Break an authoritative input: repair must fail and retain the marker.
    fs::write(root.join("templates/template.html"), "@if(never closed{\n").unwrap();
    repair_project(&root).unwrap_err();
    assert!(marker(&root), "failed repair retains the marker");
    fs::write(
        root.join("templates/template.html"),
        "<main>@content</main>\n",
    )
    .unwrap();
    repair_project(&root).expect("fixed repair succeeds");
    assert!(!marker(&root), "marker cleared after successful repair");
}

fn tempdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}
