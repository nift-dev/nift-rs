//! Nift value model.
//!
//! `Value` is the JSON-compatible tree used throughout Nift. The type and its
//! API now live in the native Rust `jsonic` crate (Jsonic++ contract) and are
//! re-exported here so Nift's call sites are unchanged.

pub use jsonic::{Value, ValueError};
