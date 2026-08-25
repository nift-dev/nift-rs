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
//! # The rendering surface
//!
//! The complete surface spans the frozen NR0–NR12 programme:
//!
//! - [`Value`] — null/bool/number/string/array/object with structured
//!   construction and insertion-order object semantics.
//! - [`Context`] — per-render overlays (bindings, page name, title, current
//!   output) resolved before Engine defaults.
//! - [`Source`] — text or path sources.
//! - [`Engine`] — the public serving surface: standalone rendering
//!   ([`Engine::render`], [`Engine::render_partial`]), project-aware
//!   construction ([`Engine::open`], [`Engine::project`]), project-aware
//!   rendering ([`Engine::render_page`]), and the atomic reload lifecycle
//!   ([`Engine::reload`]). One Engine per process, configured once and shared
//!   across concurrent renders.
//! - The parser/evaluator implements the full template surface: literals,
//!   escaping, comments, `$[...]` expressions (bindings, metadata,
//!   arithmetic/comparison/logical/ternary), `@if`/`@else if`/`@else`,
//!   `@for` over arrays and objects with loop metadata and sorting, collection
//!   directives, `@content`, `@input`, `@json` + JSON Schema, `@getenv`,
//!   `@dep`, `@ent`, `@pathto`/`@pathtofile`/`@pathtopage`, and primary
//!   pagination for tracked pages.
//!
//! # Conformance
//!
//! The crate is gated by the canonical conformance corpus (see
//! `corpus/`): every parity page renders byte-identically to its golden
//! output/dependencies/requirements, project-state and runtime reject classes
//! match, and a cross-implementation differential battery (C++ Engine vs Rust
//! Engine) runs 106 cases with zero divergence on Linux, macOS and Windows.
//!
//! # Performance note (NR12)
//!
//! `render` evaluates source templates at call time (matching the frozen C++
//! reference, which re-parses per render); there is no compiled-template
//! cache yet, so per-render cost is higher than template engines that compile
//! once (Tera/MiniJinja/Askama). The crate is faster than the reference C++
//! Engine on the representative benchmark. A compiled-template cache is the
//! natural future optimization.
//!
//! See `examples/ssr_demo.rs` for a server-side rendering walkthrough and
//! `examples/bench.rs` for the NR12 comparison benchmarks.

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
pub mod repair;
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
