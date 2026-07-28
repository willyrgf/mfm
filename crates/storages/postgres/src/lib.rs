#![warn(missing_docs)]
//! PostgreSQL representation of the recoverability-v1 committed journal.
//!
//! Authority-bearing use starts only through [`open_authoritative`]. Schema migration uses the
//! separate owner path exposed by [`PostgresSchema`].

mod configured_values;
mod error;
mod journal_store;
mod qualification;
mod schema;
mod store;

pub use error::{PostgresStoreError, Result};
#[cfg(any(test, feature = "parity-tests"))]
pub use qualification::TestAuthoritativeWriterFence;
pub use qualification::{
    open_authoritative, AuthoritativeWriterContext, AuthoritativeWriterFence,
    AuthoritativeWriterFenceFuture,
};
pub use schema::PostgresSchema;
pub use store::QualifiedPostgresStore;

#[cfg(all(test, feature = "parity-tests"))]
mod tests;
