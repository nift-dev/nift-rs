#![forbid(unsafe_code)]
//! Independent Rust implementation of the Nift template language and
//! project-aware rendering engine.
//!
//! This crate is an **independent implementation of the Nift semantic
//! contract**, not a wrapper around the C++ Nift library and not a line-by-line
//! transliteration of the C++ implementation. The frozen canonical conformance
//! corpus is the semantic authority; the C++ implementation is an archaeology
//! reference only (see `docs/authorities.md`).
//!
//! Safety: the core crate forbids `unsafe` code (`#![forbid(unsafe_code)]`,
//! mirrored by the workspace lint). No `unsafe` may be introduced without an
//! explicit architectural decision and review (see `docs/safety.md`).
//!
//! This is the NR0 baseline: the crate skeleton, safety policy, shared-corpus
//! arrangement, authorities document, complete semantic inventory and
//! checkpoint/evidence mapping. No template-language functionality is
//! implemented yet (NR1+).

/// Baseline placeholder so the crate has a linkable unit before NR1.
///
/// Removed once the first real API lands.
pub fn __nr0_baseline_marker() -> u32 {
    0
}

#[cfg(test)]
mod tests {
    #[test]
    fn nr0_baseline_compiles() {
        assert_eq!(super::__nr0_baseline_marker(), 0);
    }
}
