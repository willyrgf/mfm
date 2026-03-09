//! Remote registry adapters.

/// docs.rs observation support.
pub mod docs_rs;
/// Shared paced/retrying HTTP execution.
pub mod http;
/// crates.io sparse-index observation support.
pub mod index;
/// Shared ordered async observation helpers.
pub mod observer;
