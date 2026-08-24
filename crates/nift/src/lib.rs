#![forbid(unsafe_code)]
//! Independent Rust implementation of the Nift template language and
//! project-aware rendering engine.
//!
//! This crate is an **independent implementation of the Nift semantic
//! contract**, not a wrapper around the C++ Nift library and not a line-by-line
//! transliteration of the C++ implementation. The frozen canonical conformance
//! corpus is the semantic authority; the C++ implementation is an archaeology
//! reference only (see `docs/authorities.md` in the repository root).
//!
//! Safety: the core crate forbids `unsafe` code (`#![forbid(unsafe_code)]`,
//! mirrored by the workspace lint). No `unsafe` may be introduced without an
//! explicit architectural decision and review. Malformed/external input is
//! handled as `Result`/error values, never panics.
//!
//! # NR1 status
//!
//! This checkpoint implements only the frozen foundational Rust model:
//! the value data model, request context, source input, typed errors/results,
//! engine-default bindings and the foundational precedence contract. No Nift
//! template parser or directive is implemented yet (NR2+). Notably absent
//! until their owning checkpoints: template/JSON parsing (NR4), `@json`-style
//! `set_json` text handling (NR4), and the `Engine` serving surface (NR6).

pub mod bindings;
pub mod context;
pub mod engine;
pub mod error;
pub mod expr;
pub mod host;
pub mod hosts;
pub mod parser;
pub mod project;
pub mod project_host;
pub mod result;
pub mod source;
pub mod value;

pub use bindings::{resolve, structural_builtin_name, valid_binding_identifier, Bindings};
pub use context::Context;
pub use engine::Engine;
pub use error::{BindingError, ErrorKind, RenderError};
pub use host::{RenderHost, RenderIdentity};
pub use hosts::{FilesystemHost, InMemoryHost};
pub use parser::{render, render_tracked};
pub use project::{
    mapped_name, ProjectConfig, ProjectError, ProjectErrorKind, ProjectState, TrackedInfo,
};
pub use result::RenderResult;
pub use source::Source;
pub use value::{Value, ValueError};
pub mod json;
