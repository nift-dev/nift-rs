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
use std::sync::{Arc, Mutex};

/// A custom source loader: `path -> HostResult` (value / absent / host error).
type SourceLoader = Arc<dyn Fn(&str) -> crate::host::HostResult + Send + Sync>;
/// A custom environment provider: `name -> HostResult`.
type EnvironmentProvider = Arc<dyn Fn(&str) -> crate::host::HostResult + Send + Sync>;

/// The public Embedded Nift rendering engine.
pub struct Engine {
    root: PathBuf,
    defaults: Bindings,
    loader: Option<SourceLoader>,
    environment_provider: Option<EnvironmentProvider>,
    /// The published immutable project snapshot generations (NR9). Publication
    /// is mutex-serialized: a render clones the current generation's `Arc`
    /// under the lock and then renders on that generation, so an in-flight
    /// render finishes on the snapshot it started with while a `reload`
    /// atomically publishes a fresh generation. `None` means no committed
    /// generation yet (the standalone Engine, or a lifecycle Engine whose
    /// project has not opened).
    project: Mutex<ProjectSlot>,
}

/// The published project state: the committed snapshot generation (if any) plus
/// the recorded open/reload failure, mirroring the frozen C++ `Impl`
/// (`project_state`/`project_open_ok`/`project_open_error`).
#[derive(Debug, Default)]
struct ProjectSlot {
    state: Option<Arc<crate::project::ProjectState>>,
    open_error: Option<crate::project::ProjectError>,
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
            project: Mutex::new(ProjectSlot::default()),
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
    /// Custom source loader (convenience form without an error channel: value
    /// -> Found, None -> NotFound).
    pub fn set_loader<F>(&mut self, loader: F)
    where
        F: Fn(&str) -> Option<String> + Send + Sync + 'static,
    {
        self.loader = Some(Arc::new(move |path| match loader(path) {
            Some(value) => crate::host::HostResult::Found(value),
            None => crate::host::HostResult::NotFound,
        }));
    }

    /// Custom source loader with the full host-result contract (Found /
    /// NotFound / Error); a host Error fails the render with the diagnostic.
    pub fn set_loader_result<F>(&mut self, loader: F)
    where
        F: Fn(&str) -> crate::host::HostResult + Send + Sync + 'static,
    {
        self.loader = Some(Arc::new(loader));
    }

    /// Custom environment provider for `@getenv` (convenience form without an
    /// error channel; None means unset).
    pub fn set_environment_provider<F>(&mut self, provider: F)
    where
        F: Fn(&str) -> Option<String> + Send + Sync + 'static,
    {
        self.environment_provider = Some(Arc::new(move |name| match provider(name) {
            Some(value) => crate::host::HostResult::Found(value),
            None => crate::host::HostResult::NotFound,
        }));
    }

    /// Custom environment provider with the full host-result contract; a host
    /// Error fails the render with the diagnostic.
    pub fn set_environment_provider_result<F>(&mut self, provider: F)
    where
        F: Fn(&str) -> crate::host::HostResult + Send + Sync + 'static,
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
    /// and `.nift/tracked.json` into an immutable snapshot. Returns an error
    /// (with the project-read semantic class) when the project does not open.
    /// For the lifecycle-aware, non-throwing construction that can later open
    /// via [`Engine::reload`], use [`Engine::project`].
    pub fn open(root: impl Into<PathBuf>) -> Result<Engine, crate::project::ProjectError> {
        let engine = Engine::project(root);
        match engine.open_error() {
            Some(error) => Err(error),
            None => Ok(engine),
        }
    }

    /// Lifecycle-aware project construction (NR9): associates the Engine with a
    /// project root and attempts to open it, but never fails. `is_open()` /
    /// [`Engine::open_error`] report the outcome, and [`Engine::reload`] can
    /// later establish the first valid generation once the project exists on
    /// disk (the frozen "Engine constructed before its project exists" case).
    /// The default Engine stays deterministic standalone and never walks the
    /// filesystem.
    pub fn project(root: impl Into<PathBuf>) -> Engine {
        let mut engine = Engine::new();
        engine.root = root.into();
        let root = engine.root.clone();
        match crate::project::ProjectState::open(&root) {
            Ok(state) => {
                let mut slot = lock(&engine.project);
                slot.state = Some(Arc::new(state));
                slot.open_error = None;
            }
            Err(error) => {
                let mut slot = lock(&engine.project);
                slot.open_error = Some(error);
            }
        }
        engine
    }

    /// Atomically replaces the published project snapshot generation (NR9):
    /// builds and fully validates a candidate snapshot, then publishes it under
    /// the publication lock so in-flight renders finish on the generation they
    /// started with and later renders observe the new one. A failed reload
    /// retains the last known-good generation (never fail-closed) and returns
    /// the candidate error; it never writes to the project. This is also how an
    /// Engine constructed before its project existed can later open it.
    ///
    /// Concurrent `reload` and `render_page` are supported; concurrent reloads
    /// serialize their publication (the last successful candidate wins). Engine
    /// defaults and the environment provider are unaffected by a reload.
    pub fn reload(&self) -> Result<(), crate::project::ProjectError> {
        let root = self.root.clone();
        let candidate = crate::project::ProjectState::open(&root)?;
        let mut slot = lock(&self.project);
        slot.state = Some(Arc::new(candidate));
        slot.open_error = None;
        Ok(())
    }

    /// Whether a project snapshot generation is currently published (see
    /// [`Engine::open`] / [`Engine::reload`]).
    pub fn is_open(&self) -> bool {
        lock(&self.project).state.is_some()
    }

    /// The recorded open/reload failure, when no generation is published.
    pub fn open_error(&self) -> Option<crate::project::ProjectError> {
        lock(&self.project).open_error.clone()
    }

    /// Project-aware rendering by tracked page name, e.g. `render_page("about")`.
    /// The page's content/template/output geometry comes from the published
    /// project snapshot generation; `@pathto`, `@input`, `@json`, contracts,
    /// dependencies, requirements and the primary pagination output behave
    /// exactly like the CLI. The page-name argument is authoritative (the
    /// Context's page name is ignored) and the project defines the current
    /// output (a Context output is ignored). Context value overlays and the
    /// title override Engine defaults and the tracked title. An unopened
    /// project or an unknown page name is a controlled error in the returned
    /// `Result`, never a panic.
    ///
    /// For a paginated tracked page this renders the page's PRIMARY output
    /// (`render_page("blog/") == public/blog/index.html`); arbitrary
    /// pagination-page selection is outside this contract.
    pub fn render_page(
        &self,
        page_name: &str,
        context: &Context,
    ) -> Result<RenderResult, RenderError> {
        let snapshot = {
            let slot = lock(&self.project);
            match &slot.state {
                Some(snapshot) => Arc::clone(snapshot),
                None => {
                    let open_error = slot.open_error.clone();
                    return Err(RenderError::new(
                        ErrorKind::Render,
                        open_error
                            .map(|error| error.message)
                            .unwrap_or_else(|| "not a Nift project".to_string()),
                    ));
                }
            }
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
            &snapshot,
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

    /// Render a standalone filesystem source (the file at `path`) as a
    /// partial. The path is ALWAYS a filesystem path: a missing path is a
    /// controlled missing-path error and is never reinterpreted as literal
    /// template text.
    pub fn render_path(
        &self,
        path: impl Into<std::path::PathBuf>,
        context: &Context,
    ) -> Result<RenderResult, RenderError> {
        self.render_partial(&Source::path(path), context)
    }

    /// Render the supplied bytes as a standalone in-memory source (partial).
    /// The text is ALWAYS template source: it is never checked against the
    /// filesystem, so it cannot be misinterpreted as a page or path name.
    pub fn render_text(
        &self,
        text: impl Into<String>,
        context: &Context,
    ) -> Result<RenderResult, RenderError> {
        self.render_partial(&Source::text(text), context)
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
            // The reference loader key is lexically_normal().generic_string():
            // `/` separators even on Windows. Using native separators here
            // would diverge from the C++ harness keys on Windows.
            let key = crate::parser::generic(path);
            return match loader(&key) {
                crate::host::HostResult::Found(value) => Ok(Cow::Owned(value)),
                crate::host::HostResult::Error(message) => {
                    Err(RenderError::new(ErrorKind::Render, message))
                }
                crate::host::HostResult::NotFound => Err(RenderError::new(
                    ErrorKind::MissingSource,
                    format!("source file is not readable: {}", path.display()),
                )),
            };
        }
        match std::fs::read_to_string(path) {
            Ok(contents) => Ok(Cow::Owned(contents)),
            Err(_) => Err(RenderError::new(
                ErrorKind::MissingSource,
                format!("source file is not readable: {}", path.display()),
            )),
        }
    }

    fn environment(&self, name: &str) -> crate::host::HostResult {
        if let Some(provider) = &self.engine.environment_provider {
            return provider(name);
        }
        match std::env::var(name) {
            Ok(value) => crate::host::HostResult::Found(value),
            Err(_) => crate::host::HostResult::NotFound,
        }
    }

    fn source_exists(&self, path: &Path) -> bool {
        if let Some(loader) = &self.engine.loader {
            // A host Error is treated as "exists" so the subsequent read
            // surfaces the distinct host-error diagnostic.
            return !matches!(
                loader(&crate::parser::generic(path)),
                crate::host::HostResult::NotFound
            );
        }
        path.exists()
    }

    fn source_readable(&self, path: &Path) -> bool {
        if let Some(loader) = &self.engine.loader {
            return !matches!(
                loader(&crate::parser::generic(path)),
                crate::host::HostResult::NotFound
            );
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

/// Lock a mutex, recovering from poisoning (the "never panics on external
/// input" rule: a poisoned lock is a panicking thread, not external input, but
/// recovery keeps the serving contract intact).
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
