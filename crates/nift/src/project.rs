//! Project reading and the immutable project snapshot (NR7).
//!
//! This module is the Rust port of the frozen C++ `project_read` authority
//! (nift-embed `ProjectRead`/`ProjectState`): it is the single implementation
//! of Nift project-read semantics — config parsing/validation, tracking
//! parsing/validation, tracked-name rules and path geometry. It performs
//! **zero project writes** by construction: the only filesystem operations are
//! reads of `.nift/config.json`, `.nift/tracked.json` and (in the shared read
//! caches) project sources/JSON.
//!
//! The observable contract (established by archaeology against the frozen C++
//! reference):
//!
//! - `root` is absolutised and lexically normalized on open; a failed open is
//!   transactional (no partial config/tracked state is observable) and the
//!   object stays empty/unopened, valid for another open.
//! - Config defaults: `content-dir=content/`, `content-ext=.html`,
//!   `output-dir=public/`, `output-ext=.html`,
//!   `default-template=templates/template.html`, `incremental-mode=modified`,
//!   `build-threads=-1`, empty `minify-exts`/`contracts`.
//! - Config validation rejects malformed JSON, a non-object/`config` shape,
//!   non-string string fields, an empty `content-dir`, bad extensions, a
//!   non-integral `build-threads`, invalid/reserved contract names, contract
//!   paths escaping the project root, unsupported `minify-exts` entries, an
//!   invalid `incremental-mode`, and **unknown config keys** (a project must
//!   never believe a setting is honoured when it is not).
//! - Tracking validation requires `.nift/tracked.json` to exist and hold an
//!   object with a `tracked` array; every entry needs string `name`/`title`, an
//!   optional string `template`, optional `content-ext`/`output-ext` overrides,
//!   an optional boolean `minify` and an optional `paginate` object with a
//!   positive integer `items-per-page`. Tracked names must be project-relative
//!   with no `..` parent component. No duplicate tracked names and no two
//!   entries resolving to the same content or output path are allowed.
//! - Path geometry: `/` maps to `index`, a trailing `/` maps to `name + index`;
//!   content/output paths are `root/dir/mapped_name + ext`; pagination page 1
//!   is the primary output, later pages are `stem-N.ext` (or `N.ext` under a
//!   directory page).
//!
//! The reject classes that matter for conformance are preserved in
//! [`ProjectErrorKind`] rather than in diagnostic prose:
//! `invalid-config-json`, `invalid-tracking-json`, `unknown-config-key` and
//! `duplicate-tracked-name`. Runtime invalidity (missing sources,
//! `@pathto` escapes) is the project-aware Engine's render-time concern (NR8).
//!
//! The project **reading/discovery** layer stays distinct from mutation, build
//! and watch concerns: there is no tracking mutation, no `.info.json` writer,
//! no hash tracking, no build decision and no watcher here.

use crate::json::parse_json;
use crate::value::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Pagination configuration for a tracked page (reference `PaginationConfig`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaginationConfig {
    /// Number of items rendered per pagination page.
    pub items_per_page: usize,
    /// Optional pagination template path (project-relative).
    pub template_path: Option<String>,
    /// Optional pagination separator path (project-relative).
    pub separator_path: Option<String>,
}

/// A tracked page record (reference `TrackedInfo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedInfo {
    /// Tracked page name; `/` is the root page, a trailing `/` a directory page.
    pub name: String,
    /// Page title (metadata `title`).
    pub title: String,
    /// Template path (project-relative); empty means the config default.
    pub template_path: String,
    /// Per-page content extension override; empty means the config default.
    pub content_ext: String,
    /// Per-page output extension override; empty means the config default.
    pub output_ext: String,
    /// Per-page minification override.
    pub minify: Option<bool>,
    /// Pagination configuration, when the page is paginated.
    pub paginate: Option<PaginationConfig>,
}

/// Validated project configuration (reference `Config`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectConfig {
    /// Content directory, project-relative.
    pub content_dir: String,
    /// Default content extension, including the leading `.`.
    pub content_ext: String,
    /// Output directory, project-relative.
    pub output_dir: String,
    /// Default output extension, including the leading `.`.
    pub output_ext: String,
    /// Default template path, project-relative.
    pub default_template: String,
    /// Incremental build mode: `modified`, `hash` or `hybrid`.
    pub incremental_mode: String,
    /// Supported (minifiable) extensions to minify, lowercased.
    pub minify_exts: BTreeSet<String>,
    /// Contract name -> project-relative JSON path.
    pub contracts: BTreeMap<String, String>,
    /// Build threads: `-1` means unspecified.
    pub build_threads: i32,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            content_dir: "content/".to_string(),
            content_ext: ".html".to_string(),
            output_dir: "public/".to_string(),
            output_ext: ".html".to_string(),
            default_template: "templates/template.html".to_string(),
            incremental_mode: "modified".to_string(),
            minify_exts: BTreeSet::new(),
            contracts: BTreeMap::new(),
            build_threads: -1,
        }
    }
}

/// Semantic project-read rejection classes (NR7 gate).
///
/// These mirror the conformance corpus classes: prose is not a public contract,
/// the class is. Config/tracking syntax invalidity (`ConfigJson`/`TrackingJson`)
/// is project-state JSON that cannot be parsed; `ConfigKey` is a semantically
/// valid JSON object with an unknown key; `DuplicateName` is the tracking
/// uniqueness rule; `ConfigValue`/`TrackingValue` are other semantically
/// invalid project state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectErrorKind {
    /// Syntax-invalid `config.json` (corpus class `invalid-config-json`).
    ConfigJson,
    /// Unknown config key (corpus class `unknown-config-key`).
    ConfigKey,
    /// Semantically invalid config value (reject; class `invalid-config-json`).
    ConfigValue,
    /// Syntax-invalid `tracked.json` (corpus class `invalid-tracking-json`).
    TrackingJson,
    /// Semantically invalid tracked entry (reject; class `invalid-tracking-json`).
    TrackingValue,
    /// Duplicate literal tracked name (corpus class `duplicate-tracked-name`).
    DuplicateName,
    /// Two distinct tracked names resolve to the same content or output path.
    /// This is ordinary invalid tracking (class `invalid-tracking-json`), NOT
    /// a duplicate-name failure.
    PathCollision,
    /// A project JSON document read through the shared cache failed
    /// (render-support read; not a project-state reject class).
    JsonRead,
}

impl ProjectErrorKind {
    /// The corpus semantic rejection class for this error.
    pub fn corpus_class(self) -> &'static str {
        match self {
            ProjectErrorKind::ConfigJson => "invalid-config-json",
            ProjectErrorKind::ConfigKey => "unknown-config-key",
            ProjectErrorKind::ConfigValue => "invalid-config-json",
            ProjectErrorKind::TrackingJson => "invalid-tracking-json",
            ProjectErrorKind::TrackingValue => "invalid-tracking-json",
            ProjectErrorKind::DuplicateName => "duplicate-tracked-name",
            ProjectErrorKind::PathCollision => "invalid-tracking-json",
            ProjectErrorKind::JsonRead => "json-read",
        }
    }
}

/// A project-read failure: a semantic class plus free-form diagnostic text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectError {
    pub kind: ProjectErrorKind,
    pub message: String,
}

impl ProjectError {
    fn new(kind: ProjectErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ProjectError {}

/// The immutable read-only snapshot of a Nift project (reference `ProjectState`).
///
/// `open` builds the candidate snapshot entirely in locals and only constructs
/// the returned state once the whole snapshot validates, so a failure can never
/// expose partial config or a partial tracked registry. Once opened the
/// snapshot is immutable; shared source/JSON reads are cached behind mutexes
/// and safe under concurrent readers. The snapshot never writes to the project.
#[derive(Debug)]
pub struct ProjectState {
    root: PathBuf,
    config: ProjectConfig,
    tracked: Vec<TrackedInfo>,
    tracked_index: HashMap<String, usize>,
    source_cache: Mutex<HashMap<String, Arc<str>>>,
    json_cache: Mutex<HashMap<String, Arc<Value>>>,
}

impl ProjectState {
    /// Opens and validates the snapshot rooted at `root`. On any failure
    /// (missing/malformed config or tracking, invalid entries, duplicate
    /// names, conflicting paths) returns an error and never writes to disk.
    pub fn open(root: impl AsRef<Path>) -> Result<ProjectState, ProjectError> {
        let candidate_root = absolute_normalized(root.as_ref());
        let mut config = ProjectConfig::default();
        load_config(&candidate_root, &mut config)?;
        let mut tracked = Vec::new();
        load_tracking(&candidate_root, &config, &mut tracked)?;
        let mut tracked_index = HashMap::with_capacity(tracked.len());
        for (i, info) in tracked.iter().enumerate() {
            tracked_index.insert(info.name.clone(), i);
        }
        Ok(ProjectState {
            root: candidate_root,
            config,
            tracked,
            tracked_index,
            source_cache: Mutex::new(HashMap::new()),
            json_cache: Mutex::new(HashMap::new()),
        })
    }

    /// The normalized absolute project root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The validated project configuration.
    pub fn config(&self) -> &ProjectConfig {
        &self.config
    }

    /// The tracked registry, preserving `tracked.json` order.
    pub fn tracked(&self) -> &[TrackedInfo] {
        &self.tracked
    }

    /// Tracked-name lookup (the same lookup the C++ reference performs).
    pub fn find(&self, name: &str) -> Option<&TrackedInfo> {
        self.tracked_index.get(name).map(|&i| &self.tracked[i])
    }

    /// Content source path for a tracked page.
    pub fn content_path(&self, info: &TrackedInfo) -> PathBuf {
        content_path_of(&self.root, &self.config, info)
    }

    /// Generated output path for a tracked page.
    pub fn output_path(&self, info: &TrackedInfo) -> PathBuf {
        output_path_of(&self.root, &self.config, info)
    }

    /// Generated output path of pagination page `page` (page 1 = primary).
    pub fn pagination_output_path(&self, info: &TrackedInfo, page: usize) -> PathBuf {
        pagination_output_path_of(&self.root, &self.config, info, page)
    }

    /// Path spelling relative to the project root (reference `relative_of`).
    pub fn relative(&self, path: &Path) -> String {
        relative_of(&self.root, path)
    }

    /// Thread-safe shared source read with the reference caching. `None` when
    /// the source cannot be read. Never writes to the paths read here.
    pub fn read_shared_source(&self, path: &Path) -> Option<Arc<str>> {
        let key = generic_string(&lexically_normal(path));
        {
            let cache = lock(&self.source_cache);
            if let Some(hit) = cache.get(&key) {
                return Some(hit.clone());
            }
        }
        let contents = Arc::<str>::from(std::fs::read_to_string(path).ok()?);
        let mut cache = lock(&self.source_cache);
        let stored = cache.entry(key).or_insert_with(|| contents.clone()).clone();
        Some(stored)
    }

    /// Thread-safe shared JSON read with the reference caching. On failure
    /// returns an error string (the cache only stores successful parses).
    pub fn read_shared_json(&self, path: &Path) -> Result<Arc<Value>, ProjectError> {
        let key = generic_string(&absolute_normalized(path));
        {
            let cache = lock(&self.json_cache);
            if let Some(hit) = cache.get(&key) {
                return Ok(hit.clone());
            }
        }
        let document = Arc::new(read_shared_json_document(path)?);
        let mut cache = lock(&self.json_cache);
        let stored = cache.entry(key).or_insert_with(|| document.clone()).clone();
        Ok(stored)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ---------------------------------------------------------------------------
// Path helpers (reference `FileSystem` + `std::filesystem` semantics)
// ---------------------------------------------------------------------------

/// `fs::absolute(root).lexically_normal()`.
fn absolute_normalized(path: &Path) -> PathBuf {
    std::path::absolute(path)
        .map(|p| lexically_normal(&p))
        .unwrap_or_else(|_| lexically_normal(path))
}

/// `fs::path::lexically_normal()`: resolve `.`/`..` components lexically.
fn lexically_normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let popped = out.pop();
                if !popped {
                    out.push(component.as_os_str());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `std::filesystem::path::generic_string()`: the path spelled with `/`
/// separators regardless of platform.
///
/// The native separator is converted to `/`; any other character is preserved
/// verbatim. In particular on POSIX the native and generic forms coincide and a
/// literal backslash inside a filename is an ordinary character that must NOT
/// be reinterpreted as a separator (so this is not a blind `\\` -> `/` text
/// replace).
fn generic_string(path: &Path) -> String {
    let native = path.to_string_lossy().to_string();
    if std::path::MAIN_SEPARATOR == '/' {
        // POSIX: native spelling already uses `/`; preserve everything else.
        native
    } else {
        // Windows: `\` is the native separator (and cannot occur inside a
        // filename), so converting it to `/` yields the generic form.
        native.replace('\\', "/")
    }
}

/// `fs::path::lexically_relative()`: the relative path from `base` to `path`.
/// Returns `.` when the paths are equivalent (the C++ reference returns `.`,
/// not an empty path, for equal paths), `..`-prefixed when outside.
fn lexical_relative(path: &Path, base: &Path) -> PathBuf {
    let path_comps: Vec<Component<'_>> = path.components().collect();
    let base_comps: Vec<Component<'_>> = base.components().collect();
    let mut common = 0;
    while common < path_comps.len()
        && common < base_comps.len()
        && path_comps[common] == base_comps[common]
    {
        common += 1;
    }
    let mut out = PathBuf::new();
    for _ in common..base_comps.len() {
        out.push("..");
    }
    for component in &path_comps[common..] {
        out.push(component.as_os_str());
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// `filesystem::has_parent_component`: a `..` component after normalising `\`
/// to `/`.
fn has_parent_component(path: &str) -> bool {
    let normalised = path.replace('\\', "/");
    normalised.split('/').any(|component| component == "..")
}

/// `filesystem::valid_extension`: begins with `.`, no path separators.
fn valid_extension(extension: &str) -> bool {
    !extension.is_empty()
        && extension.starts_with('.')
        && !extension.contains('/')
        && !extension.contains('\\')
}

/// `project_read::valid_contract_name`: identifier with letters/digits/underscores.
fn valid_contract_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    if !(bytes[0].is_ascii_alphabetic() || bytes[0] == b'_') {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

/// `project_read::reserved_contract_name`: conflicts with built-in metadata.
fn reserved_contract_name(name: &str) -> bool {
    matches!(
        name,
        "title"
            | "name"
            | "content-path"
            | "output-path"
            | "template-path"
            | "build-timezone"
            | "build-time"
            | "build-UTC-time"
            | "build-date"
            | "build-UTC-date"
            | "build-YYYY"
            | "build-YY"
            | "build-OS"
            | "loop"
    )
}

/// `project_read::valid_tracked_name`: `/` root, otherwise project-relative
/// with no `..` parent component.
fn valid_tracked_name(name: &str) -> bool {
    if name == "/" {
        return true;
    }
    if name.is_empty() {
        return false;
    }
    let path = Path::new(name);
    !path.is_absolute() && !has_parent_component(name)
}

/// `minify::format_for_extension` supported extensions, lowercased.
fn supported_minify_ext(extension: &str) -> bool {
    matches!(
        extension.to_ascii_lowercase().as_str(),
        ".html" | ".htm" | ".css" | ".js" | ".mjs" | ".cjs" | ".jsx" | ".json" | ".xml" | ".svg"
    )
}

/// `filesystem::weakly_canonical`: resolve symlinks in the existing prefix,
/// keeping a non-existent leaf path lexical.
fn weakly_canonical(path: &Path) -> std::io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::path::absolute(path)?
    };
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        resolved.push(component.as_os_str());
        let is_symlink = std::fs::symlink_metadata(&resolved)
            .map(|meta| meta.file_type().is_symlink())
            .unwrap_or(false);
        if is_symlink {
            let target = std::fs::read_link(&resolved)?;
            resolved.pop();
            if target.is_absolute() {
                resolved = weakly_canonical(&target)?;
            } else {
                resolved = weakly_canonical(&resolved.join(&target))?;
            }
        }
    }
    Ok(resolved)
}

/// `filesystem::path_within`: lexical containment first, then symlink-aware.
fn path_within(base: &Path, candidate: &Path) -> bool {
    let normalized_base = absolute_normalized(base);
    let normalized_candidate = absolute_normalized(candidate);

    // Reject obvious lexical escapes first. (lexically_relative returns "." for
    // equal paths, so only the ".." prefix can reject here, exactly like the
    // C++ reference.)
    let lexical = lexical_relative(&normalized_candidate, &normalized_base);
    if lexical.components().next() == Some(Component::ParentDir) {
        return false;
    }

    let Ok(canonical_base) = weakly_canonical(&normalized_base) else {
        return false;
    };
    let Ok(canonical_candidate) = weakly_canonical(&normalized_candidate) else {
        return false;
    };

    let canonical_relative = lexical_relative(&canonical_candidate, &canonical_base);
    if canonical_relative.as_os_str().is_empty() {
        canonical_candidate == canonical_base
    } else {
        canonical_relative.components().next() != Some(Component::ParentDir)
    }
}

// ---------------------------------------------------------------------------
// Geometry (reference `project_read`)
// ---------------------------------------------------------------------------

/// The file-system name for a tracked page: `/` and trailing `/` map to `index`.
pub fn mapped_name(name: &str) -> String {
    if name == "/" {
        "index".to_string()
    } else if !name.is_empty() && name.ends_with('/') {
        format!("{}index", name)
    } else {
        name.to_string()
    }
}

fn content_path_of(root: &Path, config: &ProjectConfig, info: &TrackedInfo) -> PathBuf {
    let extension = if info.content_ext.is_empty() {
        &config.content_ext
    } else {
        &info.content_ext
    };
    root.join(&config.content_dir)
        .join(format!("{}{}", mapped_name(&info.name), extension))
}

fn output_path_of(root: &Path, config: &ProjectConfig, info: &TrackedInfo) -> PathBuf {
    let extension = if info.output_ext.is_empty() {
        &config.output_ext
    } else {
        &info.output_ext
    };
    root.join(&config.output_dir)
        .join(format!("{}{}", mapped_name(&info.name), extension))
}

fn pagination_output_path_of(
    root: &Path,
    config: &ProjectConfig,
    info: &TrackedInfo,
    page: usize,
) -> PathBuf {
    if page <= 1 {
        return output_path_of(root, config, info);
    }
    let extension = if info.output_ext.is_empty() {
        &config.output_ext
    } else {
        &info.output_ext
    };
    let primary = output_path_of(root, config, info);
    let parent = primary.parent().unwrap_or(&primary);
    if info.name == "/" || (!info.name.is_empty() && info.name.ends_with('/')) {
        parent.join(format!("{page}{extension}"))
    } else {
        let stem = primary
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("");
        parent.join(format!("{stem}-{page}{extension}"))
    }
}

fn relative_of(root: &Path, path: &Path) -> String {
    // The reference returns lexically_relative(path, root).generic_string();
    // lexically_relative yields "." for a path equal to the root (probed C++
    // behavior), so the root itself spells as ".".
    generic_string(&lexical_relative(&lexically_normal(path), root))
}

// ---------------------------------------------------------------------------
// Config reading (reference `project_read::load_config`)
// ---------------------------------------------------------------------------

/// `ProjectState::read_shared_json` document read with the reference error
/// messages.
fn read_shared_json_document(path: &Path) -> Result<Value, ProjectError> {
    if !path.exists() {
        return Err(ProjectError::new(
            ProjectErrorKind::JsonRead,
            "JSON file does not exist",
        ));
    }
    if !path.is_file() {
        return Err(ProjectError::new(
            ProjectErrorKind::JsonRead,
            "JSON file is not readable",
        ));
    }
    let text = std::fs::read_to_string(path)
        .map_err(|_| ProjectError::new(ProjectErrorKind::JsonRead, "JSON file is not readable"))?;
    parse_json(&text).map_err(|error| {
        ProjectError::new(
            ProjectErrorKind::JsonRead,
            format!("failed to parse JSON ({error})"),
        )
    })
}

fn load_json_document(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Err("file does not exist".to_string());
    }
    if !path.is_file() {
        return Err("file is not readable".to_string());
    }
    let text = std::fs::read_to_string(path).map_err(|_| "file is not readable".to_string())?;
    parse_json(&text)
}

fn read_string_field(
    config: &indexmap::IndexMap<String, Value>,
    key: &str,
) -> Result<Option<String>, ProjectError> {
    match config.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(|text| Some(text.to_string()))
            .ok_or_else(|| {
                ProjectError::new(
                    ProjectErrorKind::ConfigValue,
                    "config string fields must contain JSON strings",
                )
            }),
    }
}

fn load_config(root: &Path, config: &mut ProjectConfig) -> Result<(), ProjectError> {
    let path = root.join(".nift/config.json");
    let document = match load_json_document(&path) {
        Ok(document) => document,
        Err(parse_error) => {
            return Err(ProjectError::new(
                ProjectErrorKind::ConfigJson,
                format!("invalid project config ({parse_error})"),
            ))
        }
    };
    let Some(value) = document.as_object() else {
        return Err(ProjectError::new(
            ProjectErrorKind::ConfigJson,
            "invalid project config",
        ));
    };
    let Some(config_object) = value.get("config").and_then(|config| config.as_object()) else {
        return Err(ProjectError::new(
            ProjectErrorKind::ConfigJson,
            "invalid project config",
        ));
    };

    for key in [
        "content-dir",
        "content-ext",
        "output-dir",
        "output-ext",
        "default-template",
        "incremental-mode",
    ] {
        let field = read_string_field(config_object, key)?;
        let destination = match key {
            "content-dir" => &mut config.content_dir,
            "content-ext" => &mut config.content_ext,
            "output-dir" => &mut config.output_dir,
            "output-ext" => &mut config.output_ext,
            "default-template" => &mut config.default_template,
            _ => &mut config.incremental_mode,
        };
        if let Some(field) = field {
            *destination = field;
        }
    }

    if config.content_dir.is_empty() {
        return Err(ProjectError::new(
            ProjectErrorKind::ConfigValue,
            "content-dir must be non-empty",
        ));
    }
    if !valid_extension(&config.content_ext) || !valid_extension(&config.output_ext) {
        return Err(ProjectError::new(
            ProjectErrorKind::ConfigValue,
            "content-ext and output-ext must begin with '.' and cannot contain path separators",
        ));
    }

    if let Some(build_threads) = config_object.get("build-threads") {
        let valid = build_threads
            .as_number()
            .map(|number| {
                number.is_finite()
                    && number.fract() == 0.0
                    && number >= i32::MIN as f64
                    && number <= i32::MAX as f64
            })
            .unwrap_or(false);
        if !valid {
            return Err(ProjectError::new(
                ProjectErrorKind::ConfigValue,
                "build-threads must be an integer",
            ));
        }
        config.build_threads = build_threads.as_number().unwrap() as i32;
    }

    config.contracts.clear();
    if let Some(contracts) = config_object.get("contracts") {
        let Some(contract_object) = contracts.as_object() else {
            return Err(ProjectError::new(
                ProjectErrorKind::ConfigValue,
                "contracts must be an object mapping names to project-relative JSON paths",
            ));
        };
        for (name, source) in contract_object {
            if !valid_contract_name(name) {
                return Err(ProjectError::new(
                    ProjectErrorKind::ConfigValue,
                    format!(
                        "contract name '{name}' must be an identifier using letters, digits and underscores"
                    ),
                ));
            }
            if reserved_contract_name(name) {
                return Err(ProjectError::new(
                    ProjectErrorKind::ConfigValue,
                    format!(
                        "contract name '{name}' conflicts with built-in metadata/reserved bindings"
                    ),
                ));
            }
            let Some(source_path) = source.as_str() else {
                return Err(ProjectError::new(
                    ProjectErrorKind::ConfigValue,
                    format!("contract '{name}' must map to a non-empty JSON path string"),
                ));
            };
            if source_path.is_empty() {
                return Err(ProjectError::new(
                    ProjectErrorKind::ConfigValue,
                    format!("contract '{name}' must map to a non-empty JSON path string"),
                ));
            }
            let contract_path = lexically_normal(&root.join(source_path));
            if !path_within(root, &contract_path) {
                return Err(ProjectError::new(
                    ProjectErrorKind::ConfigValue,
                    format!(
                        "contract '{name}' path must stay inside the Nift project: {source_path}"
                    ),
                ));
            }
            config
                .contracts
                .insert(name.clone(), source_path.to_string());
        }
    }

    config.minify_exts.clear();
    if let Some(minify_exts) = config_object.get("minify-exts") {
        let Some(extensions) = minify_exts.as_array() else {
            return Err(ProjectError::new(
                ProjectErrorKind::ConfigValue,
                "minify-exts must be an array of extension strings",
            ));
        };
        for item in extensions {
            let Some(extension) = item.as_str() else {
                return Err(ProjectError::new(
                    ProjectErrorKind::ConfigValue,
                    "every minify-exts entry must be an extension string beginning with '.'",
                ));
            };
            if !valid_extension(extension) {
                return Err(ProjectError::new(
                    ProjectErrorKind::ConfigValue,
                    "every minify-exts entry must be an extension string beginning with '.'",
                ));
            }
            if !supported_minify_ext(extension) {
                return Err(ProjectError::new(
                    ProjectErrorKind::ConfigValue,
                    format!("unsupported minify-exts entry: {extension}"),
                ));
            }
            config.minify_exts.insert(extension.to_ascii_lowercase());
        }
    }

    if config.incremental_mode != "modified"
        && config.incremental_mode != "hash"
        && config.incremental_mode != "hybrid"
    {
        return Err(ProjectError::new(
            ProjectErrorKind::ConfigValue,
            "incremental-mode must be modified, hash or hybrid",
        ));
    }

    const KNOWN_CONFIG_KEYS: &[&str] = &[
        "content-dir",
        "content-ext",
        "output-dir",
        "output-ext",
        "default-template",
        "incremental-mode",
        "build-threads",
        "contracts",
        "minify-exts",
    ];
    for key in config_object.keys() {
        if !KNOWN_CONFIG_KEYS.contains(&key.as_str()) {
            return Err(ProjectError::new(
                ProjectErrorKind::ConfigKey,
                format!("unknown config key '{key}'"),
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tracking reading (reference `project_read::load_tracking`)
// ---------------------------------------------------------------------------

fn integer_field(value: &Value) -> Option<i64> {
    let number = value.as_number()?;
    if !number.is_finite() || number.fract() != 0.0 {
        return None;
    }
    Some(number as i64)
}

fn load_tracking(
    root: &Path,
    config: &ProjectConfig,
    tracked: &mut Vec<TrackedInfo>,
) -> Result<(), ProjectError> {
    let path = root.join(".nift/tracked.json");
    if !path.exists() {
        return Err(ProjectError::new(
            ProjectErrorKind::TrackingJson,
            "invalid tracked.json (file does not exist)",
        ));
    }
    let source = std::fs::read_to_string(&path).map_err(|_| {
        ProjectError::new(
            ProjectErrorKind::TrackingJson,
            "invalid tracked.json (file is not readable)",
        )
    })?;
    let document = parse_json(&source).map_err(|parse_error| {
        ProjectError::new(
            ProjectErrorKind::TrackingJson,
            format!("invalid tracked.json ({parse_error})"),
        )
    })?;
    let Some(tracking_object) = document.as_object() else {
        return Err(ProjectError::new(
            ProjectErrorKind::TrackingJson,
            "invalid tracked.json (expected root JSON object)",
        ));
    };
    let Some(entries) = tracking_object.get("tracked").and_then(|t| t.as_array()) else {
        return Err(ProjectError::new(
            ProjectErrorKind::TrackingJson,
            "invalid tracked.json (expected an array for member 'tracked')",
        ));
    };

    tracked.clear();
    for entry in entries {
        let Some(entry_object) = entry.as_object() else {
            return Err(tracking_entry_error(
                "every tracked entry must be an object with string name/title fields and an optional string template field",
            ));
        };
        let Some(name) = entry_object.get("name").and_then(|value| value.as_str()) else {
            return Err(tracking_entry_error(
                "every tracked entry must be an object with string name/title fields and an optional string template field",
            ));
        };
        let Some(title) = entry_object.get("title").and_then(|value| value.as_str()) else {
            return Err(tracking_entry_error(
                "every tracked entry must be an object with string name/title fields and an optional string template field",
            ));
        };
        if entry_object.contains_key("template") && !entry_object["template"].is_string() {
            return Err(tracking_entry_error(
                "every tracked entry must be an object with string name/title fields and an optional string template field",
            ));
        }

        let mut info = TrackedInfo {
            name: name.to_string(),
            title: title.to_string(),
            template_path: entry_object
                .get("template")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string(),
            content_ext: String::new(),
            output_ext: String::new(),
            minify: None,
            paginate: None,
        };

        if let Some(content_ext) = entry_object.get("content-ext") {
            let Some(extension) = content_ext.as_str() else {
                return Err(tracking_entry_error("tracked content-ext must be a string"));
            };
            info.content_ext = extension.to_string();
        }
        if let Some(output_ext) = entry_object.get("output-ext") {
            let Some(extension) = output_ext.as_str() else {
                return Err(tracking_entry_error("tracked output-ext must be a string"));
            };
            info.output_ext = extension.to_string();
        }
        if let Some(minify) = entry_object.get("minify") {
            let Some(flag) = minify.as_bool() else {
                return Err(tracking_entry_error(
                    "tracked minify override must be a boolean",
                ));
            };
            info.minify = Some(flag);
        }
        if let Some(paginate) = entry_object.get("paginate") {
            let Some(pagination_object) = paginate.as_object() else {
                return Err(tracking_entry_error(
                    "tracked paginate must be an object with positive integer items-per-page",
                ));
            };
            let Some(items) = pagination_object.get("items-per-page") else {
                return Err(tracking_entry_error(
                    "tracked paginate must be an object with positive integer items-per-page",
                ));
            };
            let Some(items) = integer_field(items) else {
                return Err(tracking_entry_error(
                    "tracked paginate must be an object with positive integer items-per-page",
                ));
            };
            if items < 1 {
                return Err(tracking_entry_error(
                    "tracked paginate must be an object with positive integer items-per-page",
                ));
            }
            let mut pagination = PaginationConfig {
                items_per_page: items as usize,
                template_path: None,
                separator_path: None,
            };
            if let Some(template) = pagination_object.get("template") {
                let Some(path) = template.as_str() else {
                    return Err(tracking_entry_error("paginate template must be a string"));
                };
                pagination.template_path = Some(path.to_string());
            }
            if let Some(separator) = pagination_object.get("separator") {
                let Some(path) = separator.as_str() else {
                    return Err(tracking_entry_error("paginate separator must be a string"));
                };
                pagination.separator_path = Some(path.to_string());
            }
            info.paginate = Some(pagination);
        }

        if !valid_tracked_name(&info.name) {
            return Err(tracking_entry_error(
                "tracked names must be project-relative and cannot contain '..' path components",
            ));
        }
        if (!info.content_ext.is_empty() && !valid_extension(&info.content_ext))
            || (!info.output_ext.is_empty() && !valid_extension(&info.output_ext))
        {
            return Err(tracking_entry_error(
                "tracked content-ext/output-ext overrides must begin with '.' and cannot contain path separators",
            ));
        }

        let derived_content = lexically_normal(&content_path_of(root, config, &info));
        let derived_output = lexically_normal(&output_path_of(root, config, &info));
        let template_path = lexically_normal(&root.join(&info.template_path));
        if !info.template_path.is_empty()
            && (derived_content == template_path || derived_output == template_path)
        {
            return Err(tracking_entry_error(
                "tracked template path cannot be the same as its content or output path",
            ));
        }
        tracked.push(info);
    }

    let mut names: Vec<&str> = tracked.iter().map(|info| info.name.as_str()).collect();
    names.sort_unstable();
    for pair in names.windows(2) {
        if pair[0] == pair[1] {
            return Err(ProjectError::new(
                ProjectErrorKind::DuplicateName,
                format!(
                    "invalid tracked.json (duplicate tracked name '{}')",
                    pair[0]
                ),
            ));
        }
    }

    let mut content_paths: Vec<String> = tracked
        .iter()
        .map(|info| generic_string(&lexically_normal(&content_path_of(root, config, info))))
        .collect();
    content_paths.sort_unstable();
    if content_paths.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(ProjectError::new(
            ProjectErrorKind::PathCollision,
            "invalid tracked.json (tracked entries resolve to the same content or output path)",
        ));
    }

    let mut output_paths: Vec<String> = tracked
        .iter()
        .map(|info| generic_string(&lexically_normal(&output_path_of(root, config, info))))
        .collect();
    output_paths.sort_unstable();
    if output_paths.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(ProjectError::new(
            ProjectErrorKind::PathCollision,
            "invalid tracked.json (tracked entries resolve to the same content or output path)",
        ));
    }

    Ok(())
}

fn tracking_entry_error(message: &str) -> ProjectError {
    ProjectError::new(
        ProjectErrorKind::TrackingValue,
        format!("invalid tracked.json ({message})"),
    )
}
