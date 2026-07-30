#![warn(missing_docs)]
//! Fenced PostgreSQL storage for the raw MFM executor ledger.
//!
//! This crate owns only database durability, raw compare-and-append, immutable-row enforcement,
//! and deployment-fence qualification. [`mfm_executor`] owns all proposal and fold semantics.

mod error;
mod fence;
mod schema;
mod store;

pub use error::{PostgresExecutorStoreError, Result};
pub use fence::{
    ExecutorWriterGenerationContext, ExecutorWriterGenerationFence,
    ExecutorWriterGenerationFenceFuture,
};
pub use schema::PostgresExecutorSchema;
#[cfg(feature = "qualification-tests")]
pub use store::PostgresExecutorFaultPoint;
pub use store::{
    open_executor_store, PostgresExecutorReadiness, QualifiedPostgresExecutorReadiness,
    QualifiedPostgresExecutorStore,
};
