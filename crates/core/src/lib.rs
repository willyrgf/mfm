#![allow(clippy::disallowed_methods)]
#![warn(missing_docs)]
//! Core primitives for MFM secret-bearing key material.
//!
//! `mfm_core` provides security-sensitive crypto and keystore implementation used by higher
//! layers.
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

/// Security-sensitive Ethereum private-key parsing and signing primitives.
pub mod crypto;
/// Security-sensitive Ethereum keystore primitives and errors.
pub mod keystore;
