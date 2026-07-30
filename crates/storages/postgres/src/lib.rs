#![warn(missing_docs)]
//! PostgreSQL representation of the recoverability-v2 committed journal.
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
#[doc(hidden)]
pub use store::PostgresRunJournalBackend;
#[cfg(any(test, feature = "parity-tests"))]
#[doc(hidden)]
pub use store::{TestAdmissionRunLockHook, TestCommitFailurePoint};

#[cfg(all(test, feature = "parity-tests"))]
mod tests;
