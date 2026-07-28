use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_executor::ExecutorLedgerStoreIdentity;
use sqlx::PgPool;

use crate::{PostgresExecutorStoreError, Result};

/// Future returned by an executor deployment's writer-generation fence.
pub type ExecutorWriterGenerationFenceFuture<'a, E> =
    Pin<Box<dyn Future<Output = std::result::Result<(), E>> + Send + 'a>>;

/// Independent deployment authority for one non-rollback executor writer generation.
///
/// Successful verification asserts that the exact database, schema, and executor-ledger identity
/// in [`ExecutorWriterGenerationContext`] name the sole writable lineage, that the lineage has not
/// rolled back, and that stale and sibling writers are permanently fenced. Implementations must
/// not treat the MFM run-store writer fence, PostgreSQL recovery status, or apparent WAL position
/// as this proof.
pub trait ExecutorWriterGenerationFence: Send + Sync + 'static {
    /// Deployment-private verification error.
    type Error: Send + Sync + 'static;

    /// Acquires or revalidates the independent generation fence.
    fn verify<'a>(
        &'a self,
        writer_pool: &'a PgPool,
        context: &'a ExecutorWriterGenerationContext,
    ) -> ExecutorWriterGenerationFenceFuture<'a, Self::Error>;
}

/// Non-secret database and ledger identity presented to the deployment fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorWriterGenerationContext {
    database_name: String,
    schema_name: String,
    database_oid: u32,
    store_identity: ExecutorLedgerStoreIdentity,
}

impl ExecutorWriterGenerationContext {
    pub(crate) fn new(
        database_name: String,
        schema_name: String,
        database_oid: u32,
        store_identity: ExecutorLedgerStoreIdentity,
    ) -> Self {
        Self {
            database_name,
            schema_name,
            database_oid,
            store_identity,
        }
    }

    /// Connected PostgreSQL database name.
    pub fn database_name(&self) -> &str {
        &self.database_name
    }

    /// Dedicated executor schema name.
    pub fn schema_name(&self) -> &str {
        &self.schema_name
    }

    /// PostgreSQL database OID observed on the writer connection.
    pub const fn database_oid(&self) -> u32 {
        self.database_oid
    }

    /// Exact immutable executor-ledger identity.
    pub const fn store_identity(&self) -> &ExecutorLedgerStoreIdentity {
        &self.store_identity
    }
}

pub(crate) trait HeldWriterGenerationFence: Send + Sync {
    fn verify<'a>(
        &'a self,
        writer_pool: &'a PgPool,
        context: &'a ExecutorWriterGenerationContext,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
}

struct HeldFence<F>(F);

impl<F> HeldWriterGenerationFence for HeldFence<F>
where
    F: ExecutorWriterGenerationFence,
{
    fn verify<'a>(
        &'a self,
        writer_pool: &'a PgPool,
        context: &'a ExecutorWriterGenerationContext,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.0
                .verify(writer_pool, context)
                .await
                .map_err(|_| PostgresExecutorStoreError::WriterFenceRejected)
        })
    }
}

pub(crate) fn hold_fence<F>(fence: F) -> Arc<dyn HeldWriterGenerationFence>
where
    F: ExecutorWriterGenerationFence,
{
    Arc::new(HeldFence(fence))
}
