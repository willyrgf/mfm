//! Reusable execution states for keystore-backed workflows.
//!
//! Use [`admin`] for metadata-oriented keystore actions such as import, list, and delete. Use
//! [`tx`] for local signing flows that bridge local key material into durable transaction outputs.
//! Remote raw-transaction submission lives in `mfm-state-keystore-submit` so this crate can remain
//! free of remote-capable runtime dependencies.
//!
//! These modules are intended for thin op planners: they expose stable config structs and state
//! types without pushing keystore execution logic back into the CLI or API layers.

/// States that implement keystore administration flows such as import, list, and delete.
pub mod admin;
/// States that implement local transaction signing flows.
pub mod tx;
