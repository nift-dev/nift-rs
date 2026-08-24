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

/// The public Embedded Nift rendering engine.
pub struct Engine {
    root: PathBuf,
    defaults: Bindings,
    loader: Option<SourceLoader>,
    environment_provider: Option<EnvironmentProvider>,
    /// The immutable project snapshot (NR8) when the Engine was constructed via
    /// [`Engine::open`]; `None` for the deterministic standalone Engine.
    project: Option<Arc<crate::project::ProjectState>>,
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
            project: None,
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

    /// Project-aware construction (NR8): associate the Engine with a Nift
    /// project at `project_root`, loading and validating `.nift/config.json`
    /// and `.nift/tracked.json` into an immutable snapshot. The snapshot is
    /// never mutated and never reloaded implicitly; every `render_page` call
    /// observes it for the Engine's lifetime. The default Engine stays
    /// deterministic standalone and never walks the filesystem.
    ///
    /// `Engine` remains `Send + Sync` and concurrent `render_page` calls are
    /// supported; the reload/generation lifecycle is NR9.
    pub fn open(root: impl Into<PathBuf>) -> Result<Engine, crate::project::ProjectError> {
        let state = crate::project::ProjectState::open(root.into())?;
        let mut engine = Engine::new();
        engine.root = state.root().to_path_buf();
        engine.project = Some(Arc::new(state));
        Ok(engine)
    }

    /// Whether a project snapshot is loaded (see [`Engine::open`]).
    pub fn is_open(&self) -> bool {
        self.project.is_some()
    }

    /// Project-aware rendering by tracked page name, e.g. `render_page("about")`.
    /// The page's content/template/output geometry comes from the project
    /// snapshot; `@pathto`, `@input`, `@json`, contracts, dependencies,
    /// requirements and the primary pagination output behave exactly like the
    /// CLI. The page-name argument is authoritative (the Context's page name is
    /// ignored) and the project defines the current output (a Context output is
    /// ignored). Context value overlays and the title override Engine defaults
    /// and the tracked title. A failed project open or an unknown page name is a
    /// controlled error in the returned `Result`, never a panic.
    ///
    /// For a paginated tracked page this renders the page's PRIMARY output
    /// (`render_page("blog/") == public/blog/index.html`); arbitrary
    /// pagination-page selection is outside this contract.
    pub fn render_page(
        &self,
        page_name: &str,
        context: &Context,
    ) -> Result<RenderResult, RenderError> {
        let Some(snapshot) = &self.project else {
            return Err(RenderError::new(ErrorKind::Render, "not a Nift project"));
        };
        let Some(tracked) = snapshot.find(page_name) else {
            return Err(RenderError::new(
                ErrorKind::UnknownPage,
                format!("unknown page name '{page_name}'"),
            ));
        };
        let mut info = tracked.clone();
        if let Some(title) = context.title() {
            info.title = title.to_string();
        }
        let mut identity = crate::host::RenderIdentity::new()
            .name(info.name.clone())
            .title(info.title.clone());
        if !info.template_path.is_empty() {
            identity = identity.template_path(info.template_path.clone());
        }
        let host = crate::project_host::ProjectHost::new(
            snapshot,
            &self.defaults,
            context,
            self.environment_provider.as_deref(),
        );
        let mut result = crate::parser::render_tracked(&host, &identity, info.paginate.as_ref())?;
        for dependency in host.recorded_dependencies() {
            result.dependencies.insert(dependency);
        }
        Ok(result)
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
