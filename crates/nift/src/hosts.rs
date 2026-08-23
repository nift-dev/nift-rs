//! In-memory host (NR2): the standalone/test host that proves the parser never
//! touches IO directly.
//!
//! [`InMemoryHost`] supplies value bindings (Engine defaults overlaid by a
//! Context, via the foundational precedence contract), project geometry over an
//! explicit root and configurable directories/extensions, and sources from an
//! in-memory map. It never touches the filesystem. Later checkpoints add a
//! filesystem host (NR4) and a project host over the immutable snapshot
//! (NR7/NR8); the parser sees the same seam for all of them.

use crate::bindings::{resolve, Bindings};
use crate::context::Context;
use crate::error::{ErrorKind, RenderError};
use crate::host::{RenderHost, RenderIdentity};
use crate::value::Value;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A pure in-memory render host.
pub struct InMemoryHost<'a> {
    defaults: &'a Bindings,
    context: &'a Context,
    root: PathBuf,
    content_dir: String,
    output_dir: String,
    content_ext: String,
    output_ext: String,
    sources: HashMap<PathBuf, String>,
}

impl<'a> InMemoryHost<'a> {
    pub fn new(defaults: &'a Bindings, context: &'a Context, root: impl Into<PathBuf>) -> Self {
        Self {
            defaults,
            context,
            root: root.into(),
            content_dir: "content/".to_string(),
            output_dir: "public/".to_string(),
            content_ext: ".html".to_string(),
            output_ext: ".html".to_string(),
            sources: HashMap::new(),
        }
    }

    pub fn with_source(mut self, path: impl Into<PathBuf>, contents: impl Into<String>) -> Self {
        self.sources.insert(path.into(), contents.into());
        self
    }

    pub fn with_content_ext(mut self, ext: impl Into<String>) -> Self {
        self.content_ext = ext.into();
        self
    }

    pub fn with_output_ext(mut self, ext: impl Into<String>) -> Self {
        self.output_ext = ext.into();
        self
    }
}

fn mapped_name(name: &str) -> String {
    if name == "/" {
        "index".to_string()
    } else if name.ends_with('/') {
        format!("{}index", name)
    } else {
        name.to_string()
    }
}

impl<'a> RenderHost for InMemoryHost<'a> {
    fn binding(&self, name: &str) -> Option<&Value> {
        resolve(self.defaults, self.context, name)
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn relative(&self, path: &Path) -> String {
        let normalized = path;
        match normalized.strip_prefix(&self.root) {
            Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
            Err(_) => normalized.to_string_lossy().replace('\\', "/"),
        }
    }

    fn content_path(&self, identity: &RenderIdentity) -> PathBuf {
        match &identity.name {
            Some(name) => self.root.join(&self.content_dir).join(format!(
                "{}{}",
                mapped_name(name),
                self.content_ext
            )),
            None => self.root.join(&self.content_dir),
        }
    }

    fn output_path(&self, identity: &RenderIdentity) -> PathBuf {
        match &identity.name {
            Some(name) => self.root.join(&self.output_dir).join(format!(
                "{}{}",
                mapped_name(name),
                self.output_ext
            )),
            None => self.root.join(&self.output_dir),
        }
    }

    fn read_source(&self, path: &Path) -> Result<Cow<'_, str>, RenderError> {
        match self.sources.get(path) {
            Some(source) => Ok(Cow::Borrowed(source.as_str())),
            None => Err(RenderError::new(
                ErrorKind::MissingSource,
                format!("source file is not readable: {}", path.display()),
            )),
        }
    }
}
