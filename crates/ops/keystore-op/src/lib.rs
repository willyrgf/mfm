//! Keystore op wrapper.
//!
//! Source of truth: `docs/redesign.md` (v4).
//!
//! This crate exists to enforce the boundary rule:
//! - CLI depends on `ops` crates, not directly on `core`.

pub use mfm_core::keystore::{Keystore, KeystoreConfig};
