//! The rendering host seam (NR2).
//!
//! The parser never touches IO directly. Every external capability the
//! renderer needs — value bindings, project geometry for metadata, source
//! reads — is supplied through [`RenderHost`]. This mirrors the frozen C++
//! `RenderHost` seam and keeps one rendering kernel usable standalone
//! (in-memory host), project-aware (a later `ProjectHost` over the project
//! snapshot) and with custom application hosts. No parser path assumes a
//! filesystem.

use crate::error::RenderError;
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
}
