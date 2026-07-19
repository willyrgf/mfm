#![warn(missing_docs)]
//! Postgres-backed run store.
//!
//! This crate exposes the certified Postgres run-store implementation for typed runs.
//!
//! # Examples
//!
//! ```no_run
//! # async fn example() -> Result<(), mfm_storage_postgres::PostgresStoreError> {
//! mfm_storage_postgres::PostgresSchema::migrate(
//!     "postgres://postgres:postgres@localhost/mfm",
//! )
//! .await?;
//! let _authority = mfm_storage_postgres::PostgresSchema::validate(
//!     "postgres://postgres:postgres@localhost/mfm",
//! )
//! .await?;
//! let _store = mfm_storage_postgres::PostgresStore::connect(
//!     "postgres://postgres:postgres@localhost/mfm",
//! )
//! .await?;
//! # Ok(())
//! # }
//! ```

mod configured_values;
mod run_store;
mod schema;

pub use configured_values::{
    ConfiguredValuePublication, ConfiguredValuePublicationStatus, ConfiguredValueRow,
    MAX_CONFIGURED_VALUE_BYTES, MAX_CONFIGURED_VALUE_SCHEMA_ID_BYTES,
};
pub use run_store::{
    PostgresFactQueryResult, PostgresFactQueryRow, PostgresStore, PostgresStoreAuthority,
    PostgresStoreAuthorityError, PostgresStoreError,
};
pub use schema::PostgresSchema;
