#![warn(missing_docs)]
//! Postgres-backed run store.
//!
//! This crate exposes only the certified Postgres run-store implementation. The old dynamic
//! stream-store surface was removed with the typed-core rewrite so it cannot act as semantic
//! authority for certified runs.
//!
//! # Examples
//!
//! ```no_run
//! # async fn example() -> Result<(), mfm_stream_store_postgres::PostgresStoreError> {
//! mfm_stream_store_postgres::PostgresSchema::migrate(
//!     "postgres://postgres:postgres@localhost/mfm",
//! )
//! .await?;
//! let _authority = mfm_stream_store_postgres::PostgresSchema::validate(
//!     "postgres://postgres:postgres@localhost/mfm",
//! )
//! .await?;
//! let _store = mfm_stream_store_postgres::PostgresRunStore::connect(
//!     "postgres://postgres:postgres@localhost/mfm",
//! )
//! .await?;
//! # Ok(())
//! # }
//! ```

mod run_store;
mod schema;

pub use run_store::{
    PostgresRunStore, PostgresStoreAuthority, PostgresStoreAuthorityError, PostgresStoreError,
};
pub use schema::PostgresSchema;
