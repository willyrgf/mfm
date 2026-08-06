//! One private PostgreSQL transaction authority for run and configuration history.
//!
//! All session acquisition, role selection, `search_path` pinning, target permit
//! checks, advisory locks, and commit classification live here. DML helpers require
//! [`LockedWriteTx`] or [`LockedConfigurationWriteTx`].

#[cfg(feature = "test-support")]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(feature = "test-support")]
use std::sync::{Mutex, OnceLock};

use mfm_canonical::sha256_digest_bytes;
use mfm_ids::{ContentDigest, DigestAlgorithm};
use mfm_store::structured::{
    PhysicalTargetIdentity, StructuredStoreError, StructuredStoreIdentity,
};
use sqlx::{Postgres, Row, Transaction};

use crate::schema::SCHEMA_CONTRACT_VERSION;
use crate::session::{RoleSession, SessionKind, TargetBinding};

#[cfg(feature = "test-support")]
static COMMIT_ACKNOWLEDGEMENT_UNKNOWN: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
#[cfg(feature = "test-support")]
static READ_PHASE_BARRIERS: OnceLock<Mutex<BTreeMap<String, ReadPhaseBarrier>>> = OnceLock::new();

/// Test-only handle for coordinating an indexed-head read interleaving.
#[cfg(feature = "test-support")]
#[doc(hidden)]
#[derive(Clone)]
pub struct ReadPhaseBarrier {
    reached: std::sync::Arc<tokio::sync::Notify>,
    release: std::sync::Arc<tokio::sync::Notify>,
}

#[cfg(feature = "test-support")]
impl ReadPhaseBarrier {
    /// Waits until the PostgreSQL read has released its external fixation.
    #[doc(hidden)]
    pub async fn wait_until_reached(&self) {
        self.reached.notified().await;
    }

    /// Releases the read to continue its fixed database snapshot.
    #[doc(hidden)]
    pub fn release(&self) {
        self.release.notify_one();
    }
}

/// Arms one test-only commit acknowledgement loss for a target schema.
///
/// The transaction is committed normally, but the next matching commit reports an
/// unknown acknowledgement so the caller must recover through its exact idempotent
/// append path. This hook is unavailable without the `test-support` feature.
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub fn arm_commit_acknowledgement_unknown(schema_name: impl Into<String>) {
    COMMIT_ACKNOWLEDGEMENT_UNKNOWN
        .get_or_init(|| Mutex::new(BTreeSet::new()))
        .lock()
        .expect("commit acknowledgement fault mutex poisoned")
        .insert(schema_name.into());
}

#[cfg(feature = "test-support")]
fn consume_commit_acknowledgement_unknown(schema_name: &str) -> bool {
    COMMIT_ACKNOWLEDGEMENT_UNKNOWN
        .get_or_init(|| Mutex::new(BTreeSet::new()))
        .lock()
        .expect("commit acknowledgement fault mutex poisoned")
        .remove(schema_name)
}

/// Arms one test-only barrier after the indexed head query of a read.
///
/// The returned handle observes the point after the external fixation is released
/// and lets the test resume the transaction while its repeatable-read snapshot is
/// still held. This hook is unavailable without `test-support`.
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub fn arm_read_phase_barrier(schema_name: impl Into<String>) -> ReadPhaseBarrier {
    let schema_name = schema_name.into();
    let barrier = ReadPhaseBarrier {
        reached: std::sync::Arc::new(tokio::sync::Notify::new()),
        release: std::sync::Arc::new(tokio::sync::Notify::new()),
    };
    READ_PHASE_BARRIERS
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .expect("read phase barrier mutex poisoned")
        .insert(schema_name, barrier.clone());
    barrier
}

#[cfg(feature = "test-support")]
pub(crate) async fn await_read_phase_barrier(schema_name: &str) {
    let barrier = READ_PHASE_BARRIERS
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .expect("read phase barrier mutex poisoned")
        .remove(schema_name);
    let Some(barrier) = barrier else {
        return;
    };
    barrier.reached.notify_one();
    barrier.release.notified().await;
}

/// Read-only transaction under an exact-target reader role.
pub(crate) struct ReadTx<'a> {
    transaction: Transaction<'a, Postgres>,
}

impl<'a> ReadTx<'a> {
    pub(crate) fn conn(&mut self) -> &mut Transaction<'a, Postgres> {
        &mut self.transaction
    }

    pub(crate) async fn commit_checked(
        mut self,
        binding: &TargetBinding,
    ) -> Result<(), StructuredStoreError> {
        validate_target(&mut self.transaction, binding, true).await?;
        self.transaction
            .commit()
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)
    }

    /// Validates the bound deployment target after the caller's indexed-head
    /// query has established the repeatable-read snapshot.
    pub(crate) async fn validate_target(
        &mut self,
        binding: &TargetBinding,
    ) -> Result<(), StructuredStoreError> {
        validate_target(&mut self.transaction, binding, true).await
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

    pub(crate) async fn validate_target(
        &mut self,
        binding: &TargetBinding,
    ) -> Result<(), StructuredStoreError> {
        validate_target(&mut self.transaction, binding, false).await
    }

    pub(crate) async fn commit_outcome(
        mut self,
        binding: &TargetBinding,
    ) -> Result<CommitOutcome, StructuredStoreError> {
        // Revalidate after all advisory locks and DML.  A deployment promotion
        // can happen while this transaction is open; an admission that was valid
        // at begin/prepare must not commit under a newer fence or release.
        validate_target(&mut self.transaction, binding, false).await?;
        match self.transaction.commit().await {
            Ok(()) => {
                #[cfg(feature = "test-support")]
                if consume_commit_acknowledgement_unknown(binding.schema_name()) {
                    return Ok(CommitOutcome::AcknowledgementUnknown);
                }
                Ok(CommitOutcome::Committed)
            }
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

    pub(crate) async fn validate_target(
        &mut self,
        binding: &TargetBinding,
    ) -> Result<(), StructuredStoreError> {
        validate_target(&mut self.transaction, binding, false).await
    }

    pub(crate) async fn commit_outcome(
        mut self,
        binding: &TargetBinding,
    ) -> Result<CommitOutcome, StructuredStoreError> {
        validate_target(&mut self.transaction, binding, false).await?;
        match self.transaction.commit().await {
            Ok(()) => {
                #[cfg(feature = "test-support")]
                if consume_commit_acknowledgement_unknown(binding.schema_name()) {
                    return Ok(CommitOutcome::AcknowledgementUnknown);
                }
                Ok(CommitOutcome::Committed)
            }
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
    let transaction = begin_base(session, binding, true).await?;
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
    let mut transaction = begin_base(session, binding, false).await?;
    validate_target(&mut transaction, binding, false).await?;
    Ok(WriteTx { transaction })
}

/// Acquires the complete stream lock set in canonical key order.
pub(crate) async fn lock_run_and_tenant<'a>(
    tx: WriteTx<'a>,
    run_id: &str,
    tenant_key: Option<&str>,
) -> Result<LockedWriteTx<'a>, StructuredStoreError> {
    let mut transaction = tx.transaction;
    let mut keys = vec![(0_u8, run_id), (1_u8, tenant_key.unwrap_or(""))];
    if tenant_key.is_none() {
        keys.pop();
    }
    keys.sort_unstable();
    for (namespace, key) in keys {
        sqlx::query("SELECT pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended($1, $2))")
            .bind(key)
            .bind(i32::from(namespace))
            .execute(&mut *transaction)
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    }
    Ok(LockedWriteTx { transaction })
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
    let mut transaction = begin_base(session, binding, false).await?;
    validate_target(&mut transaction, binding, false).await?;
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
    // Every run/configuration read is fixed to one snapshot. Writes retain their
    // read-committed lock protocol below; readers never observe a mixed prefix.
    let isolation = if read_only {
        crate::sql_catalog::TransactionIsolation::RepeatableRead
    } else {
        crate::sql_catalog::TransactionIsolation::ReadCommitted
    };
    sqlx::query(crate::sql_catalog::transaction_isolation(isolation))
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
    // session issuance by the private catalog bridge.
    sqlx::query(crate::sql_catalog::transaction_set_role(
        session.managed_role(),
    ))
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

async fn validate_target(
    transaction: &mut Transaction<'_, Postgres>,
    binding: &TargetBinding,
    read_only: bool,
) -> Result<(), StructuredStoreError> {
    binding
        .checkpoint()
        .validate_target(
            binding.store_scope_id(),
            binding.store_epoch(),
            binding.target_key().as_str(),
            binding.database_oid(),
            binding.schema_name(),
            binding.fence_generation(),
            binding.release_epoch(),
        )
        .map_err(|_| StructuredStoreError::InvalidHistory)?;
    let row = sqlx::query(
        "SELECT pg_catalog.current_database()::text AS database_name, \
                pg_catalog.current_schema()::text AS schema_name, \
                database.oid::bigint AS database_oid, \
                pg_catalog.pg_is_in_recovery() AS in_recovery, \
                pg_catalog.current_setting('transaction_read_only') AS transaction_read_only, \
                identity.store_scope_id, identity.store_epoch::text AS store_epoch, \
                metadata.schema_contract_version, \
                authority.fence_generation::text AS fence_generation, \
                authority.release_epoch::text AS release_epoch, \
                authority.target_key \
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
    let expected_store_epoch = binding.store_epoch().get().to_string();
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
    if row.try_get::<String, _>("schema_name").ok().as_deref() != Some(binding.schema_name())
        || database_oid != binding.database_oid()
        || row.try_get::<bool, _>("in_recovery").ok() != Some(false)
        || row
            .try_get::<String, _>("transaction_read_only")
            .ok()
            .as_deref()
            != Some(expected_read_only)
        || row.try_get::<String, _>("store_scope_id").ok().as_deref()
            != Some(binding.store_scope_id().as_str())
        || row.try_get::<String, _>("store_epoch").ok().as_deref()
            != Some(expected_store_epoch.as_str())
        || row
            .try_get::<String, _>("schema_contract_version")
            .ok()
            .as_deref()
            != Some(SCHEMA_CONTRACT_VERSION)
        || fence_generation != binding.fence_generation()
        || release_epoch != binding.release_epoch()
        || row.try_get::<String, _>("target_key").ok().as_deref()
            != Some(binding.target_key().as_str())
    {
        return Err(StructuredStoreError::InvalidHistory);
    }
    Ok(())
}

pub(crate) fn store_identity_from_binding(binding: &TargetBinding) -> StructuredStoreIdentity {
    let incarnation_preimage = format!(
        "mfm.postgres.physical-target-incarnation.v1\0{}\0{}\0{}\0{}\0{}\0{}\0{}",
        binding.target_key().as_str(),
        binding.database_oid(),
        binding.schema_name(),
        binding.store_scope_id().as_str(),
        binding.store_epoch(),
        binding.fence_generation(),
        binding.release_epoch(),
    );
    StructuredStoreIdentity {
        store_scope_id: binding.store_scope_id().clone(),
        store_epoch: binding.store_epoch(),
        physical_target: Some(PhysicalTargetIdentity {
            target_key: binding.target_key().as_str().to_owned(),
            database_oid: binding.database_oid(),
            fence_generation: binding.fence_generation(),
            release_epoch: binding.release_epoch(),
            current_incarnation_ref: ContentDigest::from_digest(
                DigestAlgorithm::Sha256V1,
                sha256_digest_bytes(incarnation_preimage.as_bytes()),
            ),
        }),
    }
}
