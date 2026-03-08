//! Reusable execution states for keystore-backed workflows.
//!
//! Use [`admin`] for metadata-oriented keystore actions such as import, list, and delete. Use
//! [`tx`] for signing and raw-transaction submission flows that bridge local key material and EVM
//! transport namespaces.
//!
//! These modules are intended for thin op planners: they expose stable config structs and state
//! types without pushing keystore execution logic back into the CLI or API layers.

/// States that implement keystore administration flows such as import, list, and delete.
pub mod admin;
/// States that implement transaction signing and raw-transaction submission flows.
pub mod tx;
