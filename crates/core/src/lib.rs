#![warn(missing_docs)]
//! Core primitives for MFM configuration and secret-bearing key material.
//!
//! `mfm_core` provides the typed configuration models that describe networks, tokens, DEXes,
//! and authentication methods, together with the security-sensitive keystore implementation used
//! by higher layers.
//!
//! # Examples
//!
//! ```rust,no_run
//! use mfm_core::keystore::{Keystore, KeystoreConfig};
//!
//! let path = std::env::temp_dir().join("mfm-core-doc-example.json");
//! let mut keystore =
//!     Keystore::new_with_config(&path, KeystoreConfig::insecure_integration_test())?;
//! keystore.unlock("correct horse battery staple")?;
//! # Ok::<(), mfm_core::keystore::KeystoreError>(())
//! ```

/// Typed YAML configuration models shared across the workspace.
pub mod config;
/// Security-sensitive Ethereum keystore primitives and errors.
pub mod keystore;
