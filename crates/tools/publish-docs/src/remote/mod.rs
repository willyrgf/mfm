//! Remote registry adapters.

/// docs.rs observation support.
pub(crate) mod docs_rs;
/// Shared paced/retrying HTTP execution.
pub(crate) mod http;
/// crates.io sparse-index observation support.
pub(crate) mod index;
/// Shared ordered async observation helpers.
pub(crate) mod observer;
