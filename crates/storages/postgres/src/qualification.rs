use std::future::Future;
use std::pin::Pin;

use mfm_ids::{StoreEpoch, StoreScopeId};
use mfm_store::v2::{QualifiedRunStore, RunAccessAuthorityIssuer};
use sqlx::{PgPool, Row};

use crate::error::{PostgresStoreError, Result};
use crate::schema::{validate_authoritative_schema_at, ValidatedStoreIdentity, APPLICATION_ROLE};
use crate::store::PostgresRunJournalBackend;

/// Future returned by a deployment-owned authoritative-writer fence.
pub type AuthoritativeWriterFenceFuture<'a, E> =
    Pin<Box<dyn Future<Output = std::result::Result<(), E>> + Send + 'a>>;

/// Deployment authority that proves one non-rollback writable PostgreSQL lineage.
///
/// Implementations are supplied by the deployment that owns database promotion, WAL/backup
/// lineage, and stale-primary fencing. Returning success asserts that the exact database,
/// schema, store scope, and epoch in [`AuthoritativeWriterContext`] have one writable lineage;
/// that every published run suffix and tenant fact head is present; and that stale and sibling
/// writers are permanently fenced.
///
/// This crate deliberately provides no production implementation, boolean bypass, connection
/// option, or "caught-up replica" mode. PostgreSQL recovery state and local schema checks are
/// necessary but cannot replace this external proof.
pub trait AuthoritativeWriterFence: Send + Sync {
    /// Deployment-private verification error.
    type Error: Send + Sync + 'static;

    /// Verifies the supplied writer pool and its exact retained identity.
    fn verify<'a>(
        &'a self,
        writer_pool: &'a PgPool,
        context: &'a AuthoritativeWriterContext,
    ) -> AuthoritativeWriterFenceFuture<'a, Self::Error>;
}

/// Non-secret identity presented to the deployment-owned writer fence.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthoritativeWriterContext {
    database_name: String,
    schema_name: String,
    database_oid: u32,
    store_scope_id: StoreScopeId,
    store_epoch: StoreEpoch,
}

impl AuthoritativeWriterContext {
    /// Connected PostgreSQL database name.
    pub fn database_name(&self) -> &str {
        &self.database_name
    }

    /// Schema containing the closed MFM authority catalog.
    pub fn schema_name(&self) -> &str {
        &self.schema_name
    }

    /// PostgreSQL database OID observed on the writer connection.
    pub const fn database_oid(&self) -> u32 {
        self.database_oid
    }

    /// Immutable MFM store scope.
    pub const fn store_scope_id(&self) -> &StoreScopeId {
        &self.store_scope_id
    }

    /// Immutable MFM store epoch.
    pub const fn store_epoch(&self) -> StoreEpoch {
        self.store_epoch
    }
}

pub(crate) struct WriterQualification {
    context: AuthoritativeWriterContext,
}

impl WriterQualification {
    fn new(context: AuthoritativeWriterContext) -> Self {
        Self { context }
    }

    pub(crate) fn into_context(self) -> AuthoritativeWriterContext {
        self.context
    }
}

/// Opens the only authority-bearing PostgreSQL store surface.
///
/// The pool is consumed and never exposed by the returned store. Qualification is repeated for
/// every reopen or promotion. All journal, object, fact-completeness, replay, trace, export, and
/// configuration access performed by the returned store uses this same pool.
pub async fn open_authoritative<F>(
    writer_pool: PgPool,
    deployment_writer_fence: F,
) -> Result<(
    QualifiedRunStore<PostgresRunJournalBackend>,
    RunAccessAuthorityIssuer,
)>
where
    F: AuthoritativeWriterFence,
{
    let before = probe_writer(&writer_pool).await?;
    let identity = validate_authoritative_schema_at(&writer_pool, &before.schema_name).await?;
    let context = before.with_identity(identity);

    deployment_writer_fence
        .verify(&writer_pool, &context)
        .await
        .map_err(|_| PostgresStoreError::WriterFenceRejected)?;

    let after = probe_writer(&writer_pool).await?;
    let retained_identity =
        validate_authoritative_schema_at(&writer_pool, &after.schema_name).await?;
    let after_context = after.with_identity(retained_identity);
    if after_context != context {
        return Err(PostgresStoreError::WriterFenceRejected);
    }

    let (backend, issuer) = PostgresRunJournalBackend::from_qualification(
        writer_pool,
        WriterQualification::new(context),
    );
    Ok((QualifiedRunStore::from_qualified_backend(backend), issuer))
}

struct WriterProbe {
    database_name: String,
    schema_name: String,
    database_oid: u32,
}

impl WriterProbe {
    fn with_identity(self, identity: ValidatedStoreIdentity) -> AuthoritativeWriterContext {
        AuthoritativeWriterContext {
            database_name: self.database_name,
            schema_name: self.schema_name,
            database_oid: self.database_oid,
            store_scope_id: identity.store_scope_id,
            store_epoch: identity.store_epoch,
        }
    }
}

async fn probe_writer(pool: &PgPool) -> Result<WriterProbe> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|_| PostgresStoreError::Connection)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    sqlx::query("SET TRANSACTION READ WRITE")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresStoreError::WriterRequired)?;

    let row = sqlx::query(
        "SELECT current_database()::text AS database_name, \
                current_schema()::text AS schema_name, \
                (SELECT oid::bigint FROM pg_catalog.pg_database \
                  WHERE datname = current_database()) AS database_oid, \
                pg_catalog.pg_is_in_recovery() AS in_recovery, \
                current_setting('transaction_read_only') AS transaction_read_only, \
                pg_catalog.pg_has_role(current_user, $1, 'SET') AS application_role_settable, \
                pg_catalog.has_table_privilege( \
                    current_user, \
                    format('%I.journal_commits', current_schema()), \
                    'SELECT' \
                ) AS journal_select, \
                pg_catalog.has_table_privilege( \
                    current_user, \
                    format('%I.journal_commits', current_schema()), \
                    'INSERT' \
                ) AS journal_insert",
    )
    .bind(APPLICATION_ROLE)
    .fetch_one(&mut *transaction)
    .await
    .map_err(|_| PostgresStoreError::WriterRequired)?;

    let in_recovery = row
        .try_get::<bool, _>("in_recovery")
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    let transaction_read_only = row
        .try_get::<String, _>("transaction_read_only")
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    let application_role_settable = row
        .try_get::<bool, _>("application_role_settable")
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    let journal_select = row
        .try_get::<bool, _>("journal_select")
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    let journal_insert = row
        .try_get::<bool, _>("journal_insert")
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    if in_recovery
        || transaction_read_only != "off"
        || !application_role_settable
        || !journal_select
        || !journal_insert
    {
        return Err(PostgresStoreError::WriterRequired);
    }

    let database_name = row
        .try_get::<String, _>("database_name")
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    let schema_name = row
        .try_get::<String, _>("schema_name")
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    let database_oid = row
        .try_get::<i64, _>("database_oid")
        .ok()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(PostgresStoreError::WriterRequired)?;

    transaction
        .rollback()
        .await
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    Ok(WriterProbe {
        database_name,
        schema_name,
        database_oid,
    })
}

#[cfg(any(test, feature = "parity-tests"))]
/// Test-only fence for database conformance tests.
pub struct TestAuthoritativeWriterFence;

#[cfg(any(test, feature = "parity-tests"))]
impl AuthoritativeWriterFence for TestAuthoritativeWriterFence {
    type Error = std::convert::Infallible;

    fn verify<'a>(
        &'a self,
        _writer_pool: &'a PgPool,
        _context: &'a AuthoritativeWriterContext,
    ) -> AuthoritativeWriterFenceFuture<'a, Self::Error> {
        Box::pin(async { Ok(()) })
    }
}
