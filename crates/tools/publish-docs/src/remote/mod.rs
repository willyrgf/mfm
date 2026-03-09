//! Remote registry adapters.

/// crates.io API observation support.
pub mod crates_io;
/// docs.rs observation support.
pub mod docs_rs;
/// Shared paced/retrying HTTP execution.
pub mod http;
/// crates.io sparse-index observation support.
pub mod index;
/// Source-agnostic remote observer traits and helpers.
pub mod observer;
