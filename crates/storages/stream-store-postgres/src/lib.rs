#![warn(missing_docs)]
//! Postgres-backed typed run-event store.
//!
//! This crate exposes only the certified typed store implementation. The old dynamic stream-store
//! surface was removed with the typed-core rewrite so it cannot act as semantic authority for
//! certified typed runs.
//!
//! # Examples
//!
//! ```no_run
//! # async fn main() -> Result<(), mfm_stream_store_postgres::PostgresTypedStoreError> {
//! let _store = mfm_stream_store_postgres::PostgresTypedRunEventStore::connect(
//!     "postgres://postgres:postgres@localhost/mfm",
//! )
//! .await?;
//! # Ok(())
//! # }
//! ```

mod typed;

pub use typed::{PostgresTypedRunEventStore, PostgresTypedStoreError};
