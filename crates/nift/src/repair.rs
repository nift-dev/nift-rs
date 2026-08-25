//! Rust `build --repair` parity.
//!
//! Implements the accepted C++ repair contract
//! (`nift-embed/docs/handover/CP4-DESIGN.md`) idiomatically in Rust: distrust
//! regenerable build-derived state and reconstruct a known-good derived tree
//! from authoritative project inputs, deleting only files Nift can establish
//! as its own.
//!
//! Cross-language contract guarantees carried across:
//! - authoritative/derived classification (only derived state is touched);
//! - ownership boundary: never infer ownership from numeric equivalence or
//!   derived metadata; only canonical Nift-generated paths are deleted;
//! - conservative orphan handling: orphan `.info.json` metadata is removed,
//!   its historical public output is preserved;
//! - marker lifecycle: Clean/Stale proceed, Live refuses; success removes the
//!   marker, any failure retains it;
//! - required sweep failures are observable (repair fails closed), stale hash
//!   cleanup is best-effort;
//! - convergence/idempotence for the reconstructible surface.
//!
//! KNOWN PARITY GAP (reported, not hidden): the Rust render engine exposes
//! only the PRIMARY pagination page (page 1) of a paginated tracked page; the
//! C++ engine emits all pagination pages 2..N. This repair therefore
//! reconstructs the primary output and `.info.json` for every tracked page and
//! fails closed for a paginated page whose surplus pages 2..N it cannot
//! regenerate (it must not certify convergence it cannot produce). Extending
//! `assemble_primary_pagination` to emit every page is the required next step
//! for full pagination convergence parity.

use std::fs;
use std::path::{Path, PathBuf};

use crate::engine::Engine;
use crate::project::ProjectState;

/// Ownership acquisition outcome (C++ `ProjectOwnership::State`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipState {
    /// Marker created by this caller; ordinary build may proceed.
    Clean,
    /// Marker existed with no live owner; a normal build must refuse, repair
    /// may proceed.
    Stale,
    /// A live process owns the lock; everyone refuses.
    Live,
    /// I/O error; the caller must refuse.
    Failed,
}

/// Two-layer build ownership: an OS-held advisory lock (live ownership) plus a
/// persistent `.nift/.unfinished` marker (crash evidence). Uses
/// `File::try_lock` (flock on Unix, LockFileEx on Windows, kernel-released on
/// process death).
pub struct Ownership {
    marker: PathBuf,
    file: Option<fs::File>,
    owned: bool,
}

impl Ownership {
    pub fn acquire(marker: PathBuf) -> (Self, OwnershipState) {
        // Atomic exclusive create (O_EXCL): Clean when we created a fresh
        // marker, otherwise open the existing one and take the advisory lock
        // (Stale when no live owner, Live when someone holds it).
        let created = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker);
        let (file, was_created) = match created {
            Ok(file) => (file, true),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                match fs::OpenOptions::new().write(true).open(&marker) {
                    Ok(file) => (file, false),
                    Err(_) => {
                        return (
                            Self { marker, file: None, owned: false },
                            OwnershipState::Failed,
                        )
                    }
                }
            }
            Err(_) => return (Self { marker, file: None, owned: false }, OwnershipState::Failed),
        };
        match file.try_lock() {
            Ok(()) => {
                let _ = file.sync_all(); // best-effort marker durability (one op)
                let ownership = Self { marker, file: Some(file), owned: true };
                ownership.hold_for_tests();
                if was_created {
                    (ownership, OwnershipState::Clean)
                } else {
                    (ownership, OwnershipState::Stale)
                }
            }
            Err(_) => (Self { marker, file: None, owned: false }, OwnershipState::Live),
        }
    }

    /// Remove the marker strictly after the final mutation and release the
    /// lock. Only the success path calls this; dropping without finish retains
    /// the marker.
    pub fn finish(&mut self) {
        let _ = fs::remove_file(&self.marker);
        self.file = None; // releases the lock
        self.owned = false;
    }

    pub fn owned(&self) -> bool {
        self.owned
    }

    /// Test-only synchronization hook (parity with the C++ project): when
    /// NIFT_TEST_OWNERSHIP_HOLD=<dir> is set, a successfully acquired owner
    /// writes <dir>/acquired and blocks until <dir>/release appears. Never
    /// active in normal use.
    fn hold_for_tests(&self) {
        let Ok(dir) = std::env::var("NIFT_TEST_OWNERSHIP_HOLD") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let _ = fs::write(dir.join("acquired"), "");
        loop {
            if dir.join("release").exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let _ = fs::remove_file(dir.join("acquired"));
    }
}

/// A required repair operation failed (repair must not certify convergence).
#[derive(Debug)]
pub struct RepairError {
    pub message: String,
}

impl RepairError {
    fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
}

impl std::fmt::Display for RepairError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RepairError {}

/// Reconstruct derived state for a project from authoritative inputs.
///
/// Ownership semantics: Clean and Stale proceed; Live and Failed refuse. On
/// success the marker is removed; on ANY failure it is retained (the caller
/// must not clear it).
pub fn repair_project(root: &Path) -> Result<(), RepairError> {
    let state = ProjectState::open(root).map_err(|e: crate::project::ProjectError| {
        RepairError::new(format!("project open failed: {}", e.message))
    })?;

    let marker = root.join(".nift").join(".unfinished");
    let (mut ownership, ownership_state) = Ownership::acquire(marker.clone());
    match ownership_state {
        OwnershipState::Live => {
            return Err(RepairError::new(
                "another build appears to be running; refusing to mutate build state",
            ))
        }
        OwnershipState::Failed => {
            return Err(RepairError::new("could not acquire the build lock"))
        }
        OwnershipState::Clean | OwnershipState::Stale => {}
    }

    // Reconstruct every tracked page (authoritative -> derived).
    let engine = Engine::open(root).map_err(|e: crate::project::ProjectError| {
        RepairError::new(format!("engine open failed: {}", e.message))
    })?;
    for info in state.tracked() {
        let context = crate::context::Context::new();
        let result = engine.render_page(&info.name, &context).map_err(|e| {
            RepairError::new(format!("page '{}' render failed: {}", info.name, e.message))
        })?;
        if info.paginate.is_some() {
            // PARITY GAP: the Rust engine exposes only the primary pagination
            // page; pages 2..N cannot be regenerated, so convergence cannot be
            // certified. Fail closed and retain the marker.
            return Err(RepairError::new(format!(
                "page '{}' is paginated; the Rust engine reconstructs only the primary \
                 pagination page, so pages 2..N cannot be regenerated (pagination-engine \
                 parity gap); refusing to certify repair",
                info.name
            )));
        }
        write_primary_output(&state, info, &result.output)?;
        write_page_info(&state, info, &result)?;
    }

    // Ownership-aware sweep (required operations propagate failure).
    sweep(&state)?;

    ownership.finish();
    Ok(())
}

fn write_primary_output(
    state: &ProjectState,
    info: &crate::project::TrackedInfo,
    contents: &str,
) -> Result<(), RepairError> {
    let output = state.output_path(info);
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|e| RepairError::new(format!("mkdir {}: {}", parent.display(), e)))?;
    }
    // Direct in-place write (the accepted CP3 contract for regenerable state).
    if fs::write(&output, contents).is_err() {
        // Existing read-only output (0644 writes fail on POSIX): make it
        // writable and retry.
        let mut perms = fs::metadata(&output)
            .map_err(|_| RepairError::new(format!("cannot write {}", output.display())))?
            .permissions();
        perms.set_readonly(false);
        let _ = fs::set_permissions(&output, perms);
        fs::write(&output, contents)
            .map_err(|e| RepairError::new(format!("failed to write {}: {}", output.display(), e)))?;
    }
    // Preserve the source content file's permissions (reference write_direct_file).
    let content = state.content_path(info);
    if let Ok(metadata) = fs::metadata(&content) {
        let _ = fs::set_permissions(&output, metadata.permissions());
    }
    Ok(())
}

/// Canonical `.info.json` (reference `ProjectInfo::write_page_info` schema).
fn write_page_info(
    state: &ProjectState,
    info: &crate::project::TrackedInfo,
    result: &crate::result::RenderResult,
) -> Result<(), RepairError> {
    let output = state.output_path(info);
    let info_path = info_path_of(state, info);
    let content_name = state.relative(&state.content_path(info));
    let output_name = state.relative(&output);
    let mut json = String::new();
    json.push_str("{\n  \"name\": \"");
    json.push_str(&escape(&info.name));
    json.push_str("\",\n  \"title\": \"");
    json.push_str(&escape(&info.title));
    json.push_str("\",\n  \"template\": \"");
    json.push_str(&escape(&info.template_path));
    json.push_str("\",\n  \"content\": \"");
    json.push_str(&escape(&content_name));
    json.push_str("\",\n  \"output\": \"");
    json.push_str(&escape(&output_name));
    json.push_str("\",\n  \"minify\": false,\n  \"minify-version\": 0,\n  \"pagination\": ");
    json.push_str(if info.paginate.is_some() { "true" } else { "false" });
    json.push_str(",\n  \"pagination-items-per-page\": ");
    json.push_str(&info.paginate.as_ref().map(|p| p.items_per_page.to_string()).unwrap_or_else(|| "0".to_string()));
    json.push_str(",\n  \"pagination-template\": \"\",\n  \"pagination-separator\": \"\",\n  \"pagination-pages\": ");
    json.push_str(if info.paginate.is_some() { "1" } else { "0" });
    json.push_str(",\n  \"dependencies\": [");
    for (index, dependency) in result.dependencies.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        json.push('\n');
        json.push_str("    \"");
        json.push_str(&escape(dependency));
        json.push('"');
    }
    if !result.dependencies.is_empty() {
        json.push('\n');
        json.push_str("  ");
    }
    json.push_str("],\n  \"reqs\": [");
    for (index, requirement) in result.requirements.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        json.push('\n');
        json.push_str("    \"");
        json.push_str(&escape(requirement));
        json.push('"');
    }
    if !result.requirements.is_empty() {
        json.push('\n');
        json.push_str("  ");
    }
    json.push_str("]\n}\n");
    if let Some(parent) = info_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| RepairError::new(format!("mkdir {}: {}", parent.display(), e)))?;
    }
    fs::write(&info_path, json).map_err(|e| {
        RepairError::new(format!("failed to write {}: {}", info_path.display(), e))
    })
}

/// `.nift/<output-relative-without-ext>.info.json` (reference
/// `ProjectInfo::info_path`).
fn info_path_of(state: &ProjectState, info: &crate::project::TrackedInfo) -> PathBuf {
    let output = state.output_path(info);
    let relative = state.relative(&output);
    let without_ext = match relative.rfind('.') {
        Some(dot) if dot > 0 => &relative[..dot],
        _ => &relative[..],
    };
    state.root().join(".nift").join(format!("{without_ext}.info.json"))
}

/// Ownership-aware sweep (reference `ProjectInfo::repair_derived_state`):
/// - orphan `.info.json` metadata is removed (its historical public output is
///   PRESERVED - path only knowable from distrustable derived metadata);
/// - stale stored hashes are best-effort cache hygiene.
fn sweep(state: &ProjectState) -> Result<(), RepairError> {
    let nift_root = state.root().join(".nift");
    let info_root = nift_root.join(&state.config().output_dir);
    let current_infos: std::collections::HashSet<PathBuf> = state
        .tracked()
        .iter()
        .map(|info| info_path_of(state, info))
        .collect();

    // Orphan .info.json (REQUIRED).
    if let Ok(entries) = fs::read_dir(&info_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(filename) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !filename.ends_with(".info.json") {
                continue;
            }
            if current_infos.contains(&path) {
                continue;
            }
            fs::remove_file(&path).map_err(|e| {
                RepairError::new(format!(
                    "failed to remove orphan metadata {}: {}",
                    path.display(),
                    e
                ))
            })?;
        }
    }

    // Stale stored hashes (BEST-EFFORT): hashes are regenerable and invalidated
    // independently, so failures are ignored.
    if let Ok(entries) = fs::read_dir(&nift_root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().map(|e| e == "hash").unwrap_or(false) {
                let mirrored = path.with_extension("");
                if !mirrored.exists() {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }
    Ok(())
}

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
