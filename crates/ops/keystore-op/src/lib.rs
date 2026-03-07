#![warn(missing_docs)]
//! Keystore op wrapper.
//!
//! Source of truth: `docs/redesign.md` (v4).
//!
//! This crate exists to enforce the boundary rule:
//! - CLI depends on `ops` crates, not directly on `core`.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_keystore::KeystoreConfig;
//!
//! let _cfg = KeystoreConfig::default();
//! ```

/// Re-exported keystore types for legacy CLI-facing integration points.
pub use mfm_core::keystore::{Keystore, KeystoreConfig};
