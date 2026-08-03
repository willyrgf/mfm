//! One private PostgreSQL transaction authority for run and configuration history.
//!
//! All session acquisition, role selection, `search_path` pinning, target permit
//! checks, advisory locks, and commit classification live here. DML helpers require
//! [`LockedWriteTx`] or [`LockedConfigurationWriteTx`].

use mfm_store::structured::{StructuredStoreError, StructuredStoreIdentity};
use sqlx::{AssertSqlSafe, Postgres, Row, Transaction};

use crate::schema::SCHEMA_CONTRACT_VERSION;
use crate::session::{RoleSession, SessionKind, TargetBinding};

/// Fresh per-transaction permit bound to the current target fence generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetPermit {
    fence_generation: u64,
    release_epoch: u64,
    schema_name: String,
    database_oid: u32,
    store_scope_id: String,
    store_epoch: String,
}

impl TargetPermit {
    pub(crate) fn from_binding(binding: &TargetBinding) -> Self {
        Self {
            fence_generation: binding.fence_generation(),
            release_epoch: binding.release_epoch(),
            schema_name: binding.schema_name().to_owned(),
            database_oid: binding.database_oid(),
            store_scope_id: binding.store_scope_id().as_str().to_owned(),
            store_epoch: binding.store_epoch().get().to_string(),
        }
    }
}

/// Read-only transaction under an exact-target reader role.
pub(crate) struct ReadTx<'a> {
    transaction: Transaction<'a, Postgres>,
}

impl<'a> ReadTx<'a> {
    pub(crate) fn conn(&mut self) -> &mut Transaction<'a, Postgres> {
        &mut self.transaction
    }

    pub(crate) async fn commit(self) -> Result<(), StructuredStoreError> {
        self.transaction
            .commit()
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)
    }

    pub(crate) async fn rollback(self) -> Result<(), StructuredStoreError> {
        self.transaction
            .rollback()
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)
    }
}

/// Write transaction that has not yet acquired its canonical advisory lock.
pub(crate) struct WriteTx<'a> {
    transaction: Transaction<'a, Postgres>,
}

/// Write transaction after the canonical target/run (or configuration) lock is held.
///
/// Decision-bearing head reads and all DML require this typestate.
pub(crate) struct LockedWriteTx<'a> {
    transaction: Transaction<'a, Postgres>,
}

impl<'a> LockedWriteTx<'a> {
    pub(crate) fn conn(&mut self) -> &mut Transaction<'a, Postgres> {
        &mut self.transaction
    }

    pub(crate) async fn commit(self) -> Result<(), StructuredStoreError> {
        self.transaction
            .commit()
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)
    }

    pub(crate) async fn commit_outcome(self) -> Result<CommitOutcome, StructuredStoreError> {
        match self.transaction.commit().await {
            Ok(()) => Ok(CommitOutcome::Committed),
            Err(_) => Ok(CommitOutcome::AcknowledgementUnknown),
        }
    }

    pub(crate) async fn rollback(self) -> Result<(), StructuredStoreError> {
        self.transaction
            .rollback()
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)
    }
}

/// Configuration-maintenance write transaction after its stream lock is held.
pub(crate) struct LockedConfigurationWriteTx<'a> {
    transaction: Transaction<'a, Postgres>,
}

impl<'a> LockedConfigurationWriteTx<'a> {
    pub(crate) fn conn(&mut self) -> &mut Transaction<'a, Postgres> {
        &mut self.transaction
    }

    pub(crate) async fn commit_outcome(self) -> Result<CommitOutcome, StructuredStoreError> {
        match self.transaction.commit().await {
            Ok(()) => Ok(CommitOutcome::Committed),
            Err(_) => Ok(CommitOutcome::AcknowledgementUnknown),
        }
    }

    pub(crate) async fn rollback(self) -> Result<(), StructuredStoreError> {
        self.transaction
            .rollback()
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)
    }

    pub(crate) async fn commit(self) -> Result<(), StructuredStoreError> {
        self.transaction
            .commit()
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)
    }
}

/// Classified commit acknowledgement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommitOutcome {
    Committed,
    AcknowledgementUnknown,
}

/// Begins a read-only transaction on a reader session.
pub(crate) async fn begin_read<'a>(
    session: &'a RoleSession,
    binding: &TargetBinding,
) -> Result<ReadTx<'a>, StructuredStoreError> {
    if !matches!(
        session.kind(),
        SessionKind::RunReader | SessionKind::ConfigurationReader
    ) {
        return Err(StructuredStoreError::BackendUnavailable);
    }
    let permit = TargetPermit::from_binding(binding);
    let mut transaction = begin_base(session, binding, true).await?;
    validate_target_permit(&mut transaction, &permit, true).await?;
    Ok(ReadTx { transaction })
}

/// Begins a writable run transaction and returns it unlocked.
pub(crate) async fn begin_run_write<'a>(
    session: &'a RoleSession,
    binding: &TargetBinding,
) -> Result<WriteTx<'a>, StructuredStoreError> {
    if session.kind() != SessionKind::RunWriter {
        return Err(StructuredStoreError::BackendUnavailable);
    }
    let permit = TargetPermit::from_binding(binding);
    let mut transaction = begin_base(session, binding, false).await?;
    validate_target_permit(&mut transaction, &permit, false).await?;
    Ok(WriteTx { transaction })
}

/// Acquires the canonical run advisory lock and upgrades to [`LockedWriteTx`].
pub(crate) async fn lock_run<'a>(
    tx: WriteTx<'a>,
    run_id: &str,
) -> Result<LockedWriteTx<'a>, StructuredStoreError> {
    let mut transaction = tx.transaction;
    sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended($1, 0))")
        .bind(run_id)
        .execute(&mut *transaction)
        .await
        .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    Ok(LockedWriteTx { transaction })
}

/// Acquires the tenant fact lock under an already locked run write transaction.
pub(crate) async fn lock_tenant_fact(
    tx: &mut LockedWriteTx<'_>,
    lock_key: &str,
) -> Result<(), StructuredStoreError> {
    sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended($1, 1))")
        .bind(lock_key)
        .execute(&mut **tx.conn())
        .await
        .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    Ok(())
}

/// Begins a configuration write transaction and acquires the stream lock.
pub(crate) async fn begin_configuration_write_locked<'a>(
    session: &'a RoleSession,
    binding: &TargetBinding,
    stream_lock_key: &str,
) -> Result<LockedConfigurationWriteTx<'a>, StructuredStoreError> {
    if session.kind() != SessionKind::ConfigurationWriter {
        return Err(StructuredStoreError::BackendUnavailable);
    }
    let permit = TargetPermit::from_binding(binding);
    let mut transaction = begin_base(session, binding, false).await?;
    validate_target_permit(&mut transaction, &permit, false).await?;
    sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended($1, 0))")
        .bind(stream_lock_key)
        .execute(&mut *transaction)
        .await
        .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    Ok(LockedConfigurationWriteTx { transaction })
}

/// Begins a configuration read transaction.
pub(crate) async fn begin_configuration_read<'a>(
    session: &'a RoleSession,
    binding: &TargetBinding,
) -> Result<ReadTx<'a>, StructuredStoreError> {
    if session.kind() != SessionKind::ConfigurationReader {
        return Err(StructuredStoreError::BackendUnavailable);
    }
    begin_read(session, binding).await
}

async fn begin_base<'a>(
    session: &'a RoleSession,
    binding: &TargetBinding,
    read_only: bool,
) -> Result<Transaction<'a, Postgres>, StructuredStoreError> {
    let mut transaction = session
        .pool()
        .begin()
        .await
        .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    // Commit 5 introduces the shared transaction owner. Commit 6 switches decision
    // reads to READ COMMITTED under the lock; use READ COMMITTED for all writes now
    // so configuration and run share one isolation choice.
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *transaction)
        .await
        .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    if read_only {
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *transaction)
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    } else {
        sqlx::query("SET TRANSACTION READ WRITE")
            .execute(&mut *transaction)
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    }
    // Role names are generated from the closed 16-hex target key and validated at
    // session issuance; quote_ident still double-quotes them before SET LOCAL ROLE.
    let set_role = format!("SET LOCAL ROLE {}", quote_ident(session.managed_role()));
    sqlx::query(AssertSqlSafe(set_role))
        .execute(&mut *transaction)
        .await
        .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    sqlx::query(
        "SELECT pg_catalog.set_config( \
             'search_path', pg_catalog.format('%I, pg_catalog', $1), TRUE \
         )",
    )
    .bind(binding.schema_name())
    .execute(&mut *transaction)
    .await
    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    Ok(transaction)
}

async fn validate_target_permit(
    transaction: &mut Transaction<'_, Postgres>,
    permit: &TargetPermit,
    read_only: bool,
) -> Result<(), StructuredStoreError> {
    if !read_only {
        // Serialize fence-generation observation for writers without requiring UPDATE
        // privilege on the authority row itself.
        sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended($1, 2))")
            .bind(format!(
                "mfm.target-fence:{}:{}",
                permit.schema_name, permit.fence_generation
            ))
            .execute(&mut **transaction)
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    }

    let row = sqlx::query(
        "SELECT pg_catalog.current_database()::text AS database_name, \
                pg_catalog.current_schema()::text AS schema_name, \
                database.oid::bigint AS database_oid, \
                pg_catalog.pg_is_in_recovery() AS in_recovery, \
                pg_catalog.current_setting('transaction_read_only') AS transaction_read_only, \
                identity.store_scope_id, identity.store_epoch::text AS store_epoch, \
                metadata.schema_contract_version, \
                authority.fence_generation::text AS fence_generation, \
                authority.release_epoch::text AS release_epoch \
           FROM pg_catalog.pg_database AS database \
           CROSS JOIN store_identity AS identity \
           CROSS JOIN store_schema_metadata AS metadata \
           CROSS JOIN target_authority AS authority \
          WHERE database.datname = pg_catalog.current_database() \
            AND identity.singleton AND metadata.singleton AND authority.singleton",
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| StructuredStoreError::BackendUnavailable)?
    .ok_or(StructuredStoreError::InvalidHistory)?;

    let database_oid = row
        .try_get::<i64, _>("database_oid")
        .ok()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(StructuredStoreError::InvalidHistory)?;
    let expected_read_only = if read_only { "on" } else { "off" };
    let fence_generation = row
        .try_get::<String, _>("fence_generation")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or(StructuredStoreError::InvalidHistory)?;
    let release_epoch = row
        .try_get::<String, _>("release_epoch")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or(StructuredStoreError::InvalidHistory)?;
    if row.try_get::<String, _>("schema_name").ok().as_deref() != Some(permit.schema_name.as_str())
        || database_oid != permit.database_oid
        || row.try_get::<bool, _>("in_recovery").ok() != Some(false)
        || row
            .try_get::<String, _>("transaction_read_only")
            .ok()
            .as_deref()
            != Some(expected_read_only)
        || row.try_get::<String, _>("store_scope_id").ok().as_deref()
            != Some(permit.store_scope_id.as_str())
        || row.try_get::<String, _>("store_epoch").ok().as_deref()
            != Some(permit.store_epoch.as_str())
        || row
            .try_get::<String, _>("schema_contract_version")
            .ok()
            .as_deref()
            != Some(SCHEMA_CONTRACT_VERSION)
        || fence_generation != permit.fence_generation
        || release_epoch != permit.release_epoch
    {
        return Err(StructuredStoreError::InvalidHistory);
    }
    Ok(())
}

pub(crate) fn store_identity_from_binding(binding: &TargetBinding) -> StructuredStoreIdentity {
    StructuredStoreIdentity {
        store_scope_id: binding.store_scope_id().clone(),
        store_epoch: binding.store_epoch(),
    }
}

fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
