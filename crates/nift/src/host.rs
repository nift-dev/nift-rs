//! The rendering host seam (NR2).
//!
//! The parser never touches IO directly. Every external capability the
//! renderer needs — value bindings, project geometry for metadata, source
//! reads — is supplied through [`RenderHost`]. This mirrors the frozen C++
//! `RenderHost` seam and keeps one rendering kernel usable standalone
//! (in-memory host), project-aware (a later `ProjectHost` over the project
//! snapshot) and with custom application hosts. No parser path assumes a
//! filesystem.

use crate::error::{ErrorKind, RenderError};
use crate::value::Value;
use std::borrow::Cow;
use std::path::{Path, PathBuf};

/// Per-render page identity: the page metadata the renderer exposes (title,
/// name, template-path) and the identity used for content/output geometry.
///
/// The caller builds this from its per-render state (e.g. a Context's page
/// name and the single per-render title slot); the parser only reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderIdentity {
    /// Page name (metadata `name`, and the identity used by geometry).
    pub name: Option<String>,
    /// Page title (metadata `title`).
    pub title: Option<String>,
    /// Template path (metadata `template-path`).
    pub template_path: Option<String>,
}

impl RenderIdentity {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn template_path(mut self, template_path: impl Into<String>) -> Self {
        self.template_path = Some(template_path.into());
        self
    }
}

/// The capability seam between the parser and the outside world.
pub trait RenderHost {
    /// A host-supplied value binding (engine defaults / context overlays),
    /// resolved before built-in metadata. The caller is responsible for
    /// implementing Context-overlay > Engine-default; the parser only asks the
    /// host.
    fn binding(&self, name: &str) -> Option<&Value>;

    /// Root directory used for relative resolution.
    fn root(&self) -> &Path;

    /// Path spelling relative to the root (generic `/` separators), used by
    /// metadata and later dependency/requirement reporting.
    fn relative(&self, path: &Path) -> String;

    /// Content source path for a page identity (metadata `content-path`).
    fn content_path(&self, identity: &RenderIdentity) -> PathBuf;

    /// Generated output path for a page identity (metadata `output-path`).
    fn output_path(&self, identity: &RenderIdentity) -> PathBuf;

    /// Read a source file. The parser routes ALL source reads through this
    /// seam; a host that cannot read the path returns a `MissingSource` error.
    fn read_source(&self, path: &Path) -> Result<Cow<'_, str>, RenderError>;

    /// Read and parse a JSON file. The default parses [`Self::read_source`];
    /// a host with structured sources (e.g. in-memory JSON) may override.
    fn read_json(&self, path: &Path) -> Result<Value, RenderError> {
        let source = self.read_source(path)?;
        crate::json::parse_json(&source).map_err(|error| {
            RenderError::new(ErrorKind::Render, format!("failed to parse JSON ({error})"))
        })
    }

    /// Environment lookup for `@getenv` (nullopt means unset). The default
    /// reads the process environment.
    fn environment(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    /// Source existence as seen by this host.
    fn source_exists(&self, path: &Path) -> bool {
        self.read_source(path).is_ok()
    }

    /// Source readability as seen by this host.
    fn source_readable(&self, path: &Path) -> bool {
        self.read_source(path).is_ok()
    }

    /// Whether `name` is a configured contract namespace (NR4 capability;
    /// project-backed contract sources arrive at NR8). Standalone hosts expose
    /// no contract namespaces by default.
    fn is_contract_name(&self, name: &str) -> bool {
        let _ = name;
        false
    }

    /// The project-relative JSON source for a contract namespace, if any.
    fn contract_source(&self, name: &str) -> Option<&str> {
        let _ = name;
        None
    }
}
