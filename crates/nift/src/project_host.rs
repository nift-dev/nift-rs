//! Project-aware render host over the immutable project snapshot (NR8).
//!
//! [`ProjectHost`] is the per-render adapter that lets the existing rendering
//! kernel consume real Nift project knowledge: tracked content/template/input
//! geometry, contracts, tracked output lookup for `@pathto` (including the 404
//! rule), pagination geometry and the shared source/JSON read caches. It mirrors
//! the frozen C++ `ProjectHost`: stateless over the snapshot, constructed per
//! render, and with `has_output_context()` always true (a project-backed render
//! is a tracked page with a real output location). It owns none of the
//! CLI/build responsibilities: no writes, no build decisions, no tracking
//! repair, no watch machinery.

use crate::bindings::{resolve, Bindings};
use crate::context::Context;
use crate::error::{ErrorKind, RenderError};
use crate::host::{RenderHost, RenderIdentity};
use crate::project::{mapped_name, ProjectState};
use crate::value::Value;
use std::borrow::Cow;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// A custom environment lookup: `name -> Option<value>` (nullopt means unset).
type EnvironmentProvider = dyn Fn(&str) -> crate::host::HostResult + Send + Sync;

/// A per-render host over an immutable [`ProjectState`] snapshot.
pub struct ProjectHost<'a> {
    state: &'a ProjectState,
    defaults: &'a Bindings,
    context: &'a Context,
    environment_provider: Option<&'a EnvironmentProvider>,
    recorded_dependencies: RefCell<Vec<String>>,
}

impl<'a> ProjectHost<'a> {
    pub fn new(
        state: &'a ProjectState,
        defaults: &'a Bindings,
        context: &'a Context,
        environment_provider: Option<&'a EnvironmentProvider>,
    ) -> Self {
        Self {
            state,
            defaults,
            context,
            environment_provider,
            recorded_dependencies: RefCell::new(Vec::new()),
        }
    }

    /// Root-relative dependencies the render discovered through project-backed
    /// value resolution (contracts), merged into the render result afterwards.
    pub fn recorded_dependencies(&self) -> Vec<String> {
        self.recorded_dependencies.borrow().clone()
    }
}

impl RenderHost for ProjectHost<'_> {
    fn binding(&self, name: &str) -> Option<&Value> {
        resolve(self.defaults, self.context, name)
    }

    fn root(&self) -> &Path {
        self.state.root()
    }

    fn relative(&self, path: &Path) -> String {
        self.state.relative(path)
    }

    fn content_path(&self, identity: &RenderIdentity) -> PathBuf {
        let config = self.state.config();
        match &identity.name {
            Some(name) => match self.state.find(name) {
                Some(info) => self.state.content_path(info),
                None => self.state.root().join(&config.content_dir).join(format!(
                    "{}{}",
                    mapped_name(name),
                    config.content_ext
                )),
            },
            None => self.state.root().join(&config.content_dir),
        }
    }

    fn output_path(&self, identity: &RenderIdentity) -> PathBuf {
        let config = self.state.config();
        match &identity.name {
            Some(name) => match self.state.find(name) {
                Some(info) => self.state.output_path(info),
                None => self.state.root().join(&config.output_dir).join(format!(
                    "{}{}",
                    mapped_name(name),
                    config.output_ext
                )),
            },
            None => self.state.root().join(&config.output_dir),
        }
    }

    fn read_source(&self, path: &Path) -> Result<Cow<'_, str>, RenderError> {
        match self.state.read_shared_source(path) {
            Some(source) => Ok(Cow::Owned(source.to_string())),
            None => Err(RenderError::new(
                ErrorKind::MissingSource,
                format!("source file is not readable: {}", path.display()),
            )),
        }
    }

    fn read_json(&self, path: &Path) -> Result<Value, RenderError> {
        self.state
            .read_shared_json(path)
            .map(|doc| (*doc).clone())
            .map_err(|error| RenderError::new(ErrorKind::Render, error.message))
    }

    fn environment(&self, name: &str) -> crate::host::HostResult {
        if let Some(provider) = self.environment_provider {
            return provider(name);
        }
        match std::env::var(name) {
            Ok(value) => crate::host::HostResult::Found(value),
            Err(_) => crate::host::HostResult::NotFound,
        }
    }

    fn source_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn source_readable(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_contract_name(&self, name: &str) -> bool {
        self.state.config().contracts.contains_key(name)
    }

    fn contract_source(&self, name: &str) -> Option<&str> {
        self.state.config().contracts.get(name).map(|s| s.as_str())
    }

    fn record_dependencies(&self, paths: &[String]) {
        self.recorded_dependencies
            .borrow_mut()
            .extend(paths.iter().cloned());
    }

    fn has_output_context(&self) -> bool {
        true
    }

    fn output_dir(&self) -> String {
        self.state.config().output_dir.clone()
    }

    fn tracked_output_path(&self, name: &str) -> Option<(PathBuf, bool)> {
        let info = self.state.find(name)?;
        let index_page = info.name == "/" || (!info.name.is_empty() && info.name.ends_with('/'));
        Some((self.state.output_path(info), index_page))
    }

    fn pagination_output_path(&self, identity: &RenderIdentity, page: usize) -> PathBuf {
        match &identity.name {
            Some(name) => match self.state.find(name) {
                Some(info) => self.state.pagination_output_path(info, page),
                None => crate::host::RenderHost::pagination_output_path(self, identity, page),
            },
            None => crate::host::RenderHost::pagination_output_path(self, identity, page),
        }
    }
}
