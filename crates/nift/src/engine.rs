//! The public standalone Embedded Nift Engine (NR6).
//!
//! `Engine` is the coherent public API that orchestrates the pieces built in
//! NR1–NR5: Context/Bindings/Source/RenderIdentity feed the rendering kernel
//! through a per-render `RenderHost`. The Engine does **not** implement parser
//! or Host semantics itself; it wires them together.
//!
//! Standalone only: this Engine performs no project tracking, no
//! `.nift/config.json` discovery, no output generation and no pagination.
//! Those are the project-aware layer's responsibilities (NR7/NR8).
//!
//! One Engine per process: configure it (root, loader, environment provider,
//! default bindings) and then share it across concurrent renders; each render
//! supplies request-scoped state through a per-render `Context`.

use crate::bindings::{resolve, Bindings};
use crate::context::Context;
use crate::error::{BindingError, ErrorKind, RenderError};
use crate::host::{RenderHost, RenderIdentity};
use crate::result::RenderResult;
use crate::source::Source;
use crate::value::Value;
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A custom source loader: `path -> Option<source>`.
type SourceLoader = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
/// A custom environment provider: `name -> Option<value>`.
type EnvironmentProvider = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// The public standalone Embedded Nift rendering engine.
pub struct Engine {
    root: PathBuf,
    defaults: Bindings,
    loader: Option<SourceLoader>,
    environment_provider: Option<EnvironmentProvider>,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// A new standalone Engine with no root, no loader, no environment
    /// provider and no default bindings.
    pub fn new() -> Self {
        Self {
            root: PathBuf::new(),
            defaults: Bindings::new(),
            loader: None,
            environment_provider: None,
        }
    }

    /// Base directory used to resolve relative path sources, relative `@input`
    /// paths and metadata geometry.
    pub fn set_root(&mut self, root: impl Into<PathBuf>) {
        self.root = root.into();
    }

    /// Custom source loader: `path -> Option<source>`. When set, all source
    /// reads (content, template, `@input`, `@json`, `@dep`) route through it.
    /// It must be thread-safe (called concurrently by renders).
    pub fn set_loader<F>(&mut self, loader: F)
    where
        F: Fn(&str) -> Option<String> + Send + Sync + 'static,
    {
        self.loader = Some(Arc::new(loader));
    }

    /// Custom environment provider for `@getenv` (`nullopt` means unset).
    /// When absent, the process environment is read.
    pub fn set_environment_provider<F>(&mut self, provider: F)
    where
        F: Fn(&str) -> Option<String> + Send + Sync + 'static,
    {
        self.environment_provider = Some(Arc::new(provider));
    }

    /// Set a long-lived default binding. Returns
    /// [`BindingError::InvalidIdentifier`] for an invalid name and
    /// [`BindingError::StructuralBuiltin`] for a structural built-in.
    pub fn set(
        &mut self,
        name: impl Into<String>,
        value: impl Into<Value>,
    ) -> Result<(), BindingError> {
        self.defaults.set(name, value.into())
    }

    /// Set a long-lived default binding from JSON text. Returns an error for
    /// malformed JSON or an invalid binding name.
    pub fn set_json(
        &mut self,
        name: impl Into<String>,
        json_text: &str,
    ) -> Result<(), RenderError> {
        let value = crate::json::parse_json(json_text)
            .map_err(|e| RenderError::new(ErrorKind::Render, format!("invalid JSON: {e}")))?;
        self.defaults
            .set(name, value)
            .map_err(|e| RenderError::new(ErrorKind::Render, e.to_string()))
    }

    /// Render a page composed into a template (the template must execute
    /// exactly one `@content`). The page and template may each be text or path
    /// sources.
    pub fn render(
        &self,
        page: &Source,
        template: &Source,
        context: &Context,
    ) -> Result<RenderResult, RenderError> {
        self.render_inner(page, Some(template), context)
    }

    /// Render a standalone partial/fragment (no content slot; `@content` is an
    /// error).
    pub fn render_partial(
        &self,
        partial: &Source,
        context: &Context,
    ) -> Result<RenderResult, RenderError> {
        self.render_inner(partial, None, context)
    }

    fn render_inner(
        &self,
        page: &Source,
        template: Option<&Source>,
        context: &Context,
    ) -> Result<RenderResult, RenderError> {
        let mut identity = RenderIdentity::new();
        if let Some(name) = context.page_name() {
            identity = identity.name(name.to_string());
        }
        if let Some(title) = context.title() {
            identity = identity.title(title.to_string());
        }
        let host = EngineHost {
            engine: self,
            context,
        };
        match template {
            Some(template) => crate::render(&host, &identity, template, Some(page)),
            None => crate::render(&host, &identity, page, None),
        }
    }
}

/// The per-render Host that routes the Engine's configuration and the render's
/// Context into the rendering kernel.
struct EngineHost<'a> {
    engine: &'a Engine,
    context: &'a Context,
}

impl<'a> RenderHost for EngineHost<'a> {
    fn binding(&self, name: &str) -> Option<&Value> {
        resolve(&self.engine.defaults, self.context, name)
    }

    fn root(&self) -> &Path {
        &self.engine.root
    }

    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.engine.root)
            .map(|rel| {
                let rel = rel.to_string_lossy().replace('\\', "/");
                if rel.is_empty() {
                    ".".to_string()
                } else {
                    rel
                }
            })
            .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"))
    }

    fn content_path(&self, identity: &RenderIdentity) -> PathBuf {
        self.engine
            .root
            .join(identity.name.clone().unwrap_or_default())
    }

    fn output_path(&self, identity: &RenderIdentity) -> PathBuf {
        // Standalone authority: an explicit Context.current_output wins;
        // otherwise the synthesized page output geometry.
        if let Some(current) = self.context.current_output() {
            return current.to_path_buf();
        }
        self.engine
            .root
            .join(identity.name.clone().unwrap_or_default())
    }

    fn read_source(&self, path: &Path) -> Result<Cow<'_, str>, RenderError> {
        if let Some(loader) = &self.engine.loader {
            let key = path.to_string_lossy().to_string();
            return loader(&key).map(Cow::Owned).ok_or_else(|| {
                RenderError::new(
                    ErrorKind::MissingSource,
                    format!("source file is not readable: {}", path.display()),
                )
            });
        }
        match std::fs::read_to_string(path) {
            Ok(contents) => Ok(Cow::Owned(contents)),
            Err(_) => Err(RenderError::new(
                ErrorKind::MissingSource,
                format!("source file is not readable: {}", path.display()),
            )),
        }
    }

    fn environment(&self, name: &str) -> Option<String> {
        if let Some(provider) = &self.engine.environment_provider {
            return provider(name);
        }
        std::env::var(name).ok()
    }

    fn source_exists(&self, path: &Path) -> bool {
        if self.engine.loader.is_some() {
            return self.read_source(path).is_ok();
        }
        path.exists()
    }

    fn source_readable(&self, path: &Path) -> bool {
        if self.engine.loader.is_some() {
            return self.read_source(path).is_ok();
        }
        path.is_file()
    }

    fn has_output_context(&self) -> bool {
        self.context.current_output().is_some()
    }

    fn output_dir(&self) -> String {
        "public/".to_string()
    }
}
