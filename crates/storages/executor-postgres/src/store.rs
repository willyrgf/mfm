use std::fmt;
#[cfg(feature = "qualification-tests")]
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use mfm_executor::{
    DeliveryAuditFrontier, DeliveryAuditFrontierRef, EffectIdentity, ExecutorAppendOutcome,
    ExecutorEffectSnapshot, ExecutorError, ExecutorFuture, ExecutorLedgerAppend,
    ExecutorLedgerStore, ExecutorLedgerStoreIdentity, ExecutorResourceAppend,
    ExecutorResourceSnapshot, ExecutorStoreSnapshot, KeyedExecutorLedger, ResourceKeyRef,
    ResourceLedgerRecord, ResourceLedgerRecordRef, ResourceOwnershipRef,
    SchemaQualifiedCanonicalValue, VerifiedExecutorBinding,
};
use mfm_ids::{ContentRef, EffectKey, SchemaId};
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::fence::{hold_fence, HeldWriterGenerationFence};
use crate::schema::{validate_authoritative_schema, APPLICATION_ROLE};
use crate::{
    ExecutorWriterGenerationContext, ExecutorWriterGenerationFence, PostgresExecutorStoreError,
    Result,
};

const LEDGER_LOCK_DOMAIN: &str = "mfm.executor-postgres.ledger-generation.v1";
const EFFECT_LOCK_DOMAIN: &str = "mfm.executor-postgres.effect.v1:";
const RESOURCE_LOCK_DOMAIN: &str = "mfm.executor-postgres.resource.v1:";

/// Bounded readiness result for one already-qualified executor writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostgresExecutorReadiness {
    ledger_ready: bool,
    fence_ready: bool,
}

impl PostgresExecutorReadiness {
    /// Whether the bounded immutable-ledger probe succeeded.
    pub const fn ledger_ready(self) -> bool {
        self.ledger_ready
    }

    /// Whether the independent writer-generation fence succeeded.
    pub const fn fence_ready(self) -> bool {
        self.fence_ready
    }
}

/// Fenced raw PostgreSQL executor store.
///
/// Values can be constructed only by [`open_executor_store`]. Clones share the same pool,
/// immutable binding, and deployment fence.
#[derive(Clone)]
pub struct QualifiedPostgresExecutorStore {
    inner: Arc<StoreInner>,
}

/// Cloneable, read-only readiness capability for one qualified executor writer.
#[derive(Clone)]
pub struct QualifiedPostgresExecutorReadiness {
    inner: Arc<StoreInner>,
}

struct StoreInner {
    pool: PgPool,
    binding: VerifiedExecutorBinding,
    identity: ExecutorLedgerStoreIdentity,
    context: ExecutorWriterGenerationContext,
    fence: Arc<dyn HeldWriterGenerationFence>,
    #[cfg(feature = "qualification-tests")]
    fault: AtomicU8,
}

impl fmt::Debug for QualifiedPostgresExecutorStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QualifiedPostgresExecutorStore")
            .field("store_identity", &self.inner.identity)
            .finish_non_exhaustive()
    }
}

/// Opens the only raw PostgreSQL executor authority.
///
/// The supplied binding selects the complete immutable store identity. The deployment fence is
/// consumed, verified before any binding admission, retained by the returned store, and verified
/// again after strict whole-ledger refold. No database transaction or lock is live while fence
/// verification runs.
pub async fn open_executor_store<F>(
    writer_pool: PgPool,
    binding: VerifiedExecutorBinding,
    deployment_writer_generation_fence: F,
) -> Result<QualifiedPostgresExecutorStore>
where
    F: ExecutorWriterGenerationFence,
{
    validate_authoritative_schema(&writer_pool).await?;
    let before = probe_writer(&writer_pool).await?;
    let identity = ExecutorLedgerStoreIdentity::from_binding(&binding);
    let context = ExecutorWriterGenerationContext::new(
        before.database_name.clone(),
        before.schema_name.clone(),
        before.database_oid,
        identity.clone(),
    );
    let fence = hold_fence(deployment_writer_generation_fence);

    fence.verify(&writer_pool, &context).await?;
    let after_fence = probe_writer(&writer_pool).await?;
    if after_fence != before {
        return Err(PostgresExecutorStoreError::WriterFenceRejected);
    }

    admit_or_verify_binding(&writer_pool, &identity).await?;
    let store = QualifiedPostgresExecutorStore {
        inner: Arc::new(StoreInner {
            pool: writer_pool,
            binding: binding.clone(),
            identity,
            context,
            fence,
            #[cfg(feature = "qualification-tests")]
            fault: AtomicU8::new(PostgresExecutorFaultPoint::None as u8),
        }),
    };

    let snapshot = store
        .enumerate_snapshot()
        .await
        .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?;
    let engine = KeyedExecutorLedger::new(store.clone(), binding)
        .map_err(|_| PostgresExecutorStoreError::BindingMismatch)?;
    engine
        .strict_refold(&snapshot)
        .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?;

    store.verify_fence().await?;
    let final_probe = probe_writer(&store.inner.pool).await?;
    if final_probe != before {
        return Err(PostgresExecutorStoreError::WriterFenceRejected);
    }
    Ok(store)
}

impl QualifiedPostgresExecutorStore {
    /// Narrows this executor store to its independently qualified readiness capability.
    pub fn readiness_handle(&self) -> QualifiedPostgresExecutorReadiness {
        QualifiedPostgresExecutorReadiness {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Selects one deterministic acknowledgement fault for the next append.
    #[cfg(feature = "qualification-tests")]
    pub fn inject_qualification_fault(&self, fault: PostgresExecutorFaultPoint) {
        self.inner.fault.store(fault as u8, Ordering::SeqCst);
    }

    /// Performs bounded ledger-identity and independent-fence readiness probes.
    ///
    /// The probe never opens a signer, wallet, destination, or RPC transport.
    pub async fn readiness(&self) -> Result<PostgresExecutorReadiness> {
        readiness(&self.inner).await
    }

    async fn verify_fence(&self) -> Result<()> {
        self.inner
            .fence
            .verify(&self.inner.pool, &self.inner.context)
            .await
    }

    async fn load_effect_inner(
        &self,
        effect_key: &EffectKey,
    ) -> mfm_executor::Result<Option<ExecutorEffectSnapshot>> {
        self.verify_fence()
            .await
            .map_err(|_| ExecutorError::DestinationGenerationFenced)?;
        let mut transaction = begin_read(&self.inner.pool)
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        require_binding_in_transaction(&mut transaction, &self.inner.identity)
            .await
            .map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
        let rows = fetch_effect_rows(&mut transaction, effect_key)
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        let allocation = fetch_effect_allocation(&mut transaction, effect_key)
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        decode_effect_snapshot(
            &self.inner.binding,
            &self.inner.identity,
            effect_key,
            rows,
            allocation,
        )
    }

    async fn load_resource_inner(
        &self,
        resource_ownership_ref: &ResourceOwnershipRef,
        resource_key_ref: &ResourceKeyRef,
    ) -> mfm_executor::Result<ExecutorResourceSnapshot> {
        self.verify_fence()
            .await
            .map_err(|_| ExecutorError::DestinationGenerationFenced)?;
        let mut transaction = begin_read(&self.inner.pool)
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        require_binding_in_transaction(&mut transaction, &self.inner.identity)
            .await
            .map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
        let rows = fetch_resource_rows(&mut transaction, resource_ownership_ref, resource_key_ref)
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        let records = decode_resource_records(resource_ownership_ref, resource_key_ref, rows)?;
        Ok(ExecutorResourceSnapshot::from_store_parts(
            resource_ownership_ref.clone(),
            resource_key_ref.clone(),
            records,
        ))
    }

    async fn load_content_inner(
        &self,
        content_ref: &ContentRef,
    ) -> mfm_executor::Result<Option<SchemaQualifiedCanonicalValue>> {
        self.verify_fence()
            .await
            .map_err(|_| ExecutorError::DestinationGenerationFenced)?;
        let mut transaction = begin_read(&self.inner.pool)
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        require_binding_in_transaction(&mut transaction, &self.inner.identity)
            .await
            .map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
        let row = sqlx::query(
            "SELECT schema_id, content_digest, canonical_bytes \
               FROM executor_content_records \
              WHERE schema_id = $1 AND content_digest = $2",
        )
        .bind(content_ref.schema_id().as_str())
        .bind(content_ref.content_digest().as_str())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let value = decode_content_row(row)?;
        if &value.reference()? != content_ref {
            return Err(ExecutorError::RetainedObjectMismatch);
        }
        Ok(Some(value))
    }

    async fn compare_and_append_inner(
        &self,
        append: ExecutorLedgerAppend,
    ) -> mfm_executor::Result<ExecutorAppendOutcome> {
        if append.identity().executor_binding_ref() != self.inner.identity.binding_ref()
            || append.identity().tenant_scope_id() != self.inner.identity.tenant_scope_id()
        {
            return Err(ExecutorError::WrongExecutorBinding);
        }
        append.validate_for_store_identity(&self.inner.identity)?;
        let prepared = PreparedAppend::new(&append)?;

        // Fence verification is intentionally complete before opening the write transaction.
        self.verify_fence()
            .await
            .map_err(|_| ExecutorError::DestinationGenerationFenced)?;

        let mut transaction = self
            .inner
            .pool
            .begin()
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        set_application_role(&mut transaction)
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        acquire_append_locks(&mut transaction, &prepared)
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        require_binding_in_transaction(&mut transaction, &self.inner.identity)
            .await
            .map_err(|_| ExecutorError::InvalidDurableSnapshot)?;

        if proposal_already_present(&mut transaction, &prepared)
            .await
            .map_err(|_| ExecutorError::InvalidDurableSnapshot)?
        {
            transaction
                .rollback()
                .await
                .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
            return Ok(ExecutorAppendOutcome::AlreadyApplied);
        }

        if !heads_match(&mut transaction, &prepared)
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?
        {
            transaction
                .rollback()
                .await
                .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
            return Ok(ExecutorAppendOutcome::Conflict);
        }

        if !content_is_compatible(&mut transaction, &prepared.content)
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?
        {
            transaction
                .rollback()
                .await
                .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
            return Ok(ExecutorAppendOutcome::Conflict);
        }

        insert_content(&mut transaction, &prepared.content)
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        insert_effect_frontier(&mut transaction, &prepared)
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        if prepared.resource.is_some() {
            insert_resource_and_link(&mut transaction, &prepared)
                .await
                .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        }

        match transaction.commit().await {
            Ok(()) => {
                #[cfg(feature = "qualification-tests")]
                if self
                    .inner
                    .fault
                    .compare_exchange(
                        PostgresExecutorFaultPoint::AfterCommitBeforeAcknowledgement as u8,
                        PostgresExecutorFaultPoint::None as u8,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
                    .is_ok()
                {
                    return Ok(ExecutorAppendOutcome::OutcomeUnknown);
                }
                Ok(ExecutorAppendOutcome::Applied)
            }
            // Once COMMIT has been attempted the backend deliberately does not infer whether the
            // server published it. Reconciliation must reload the exact proposal.
            Err(_) => Ok(ExecutorAppendOutcome::OutcomeUnknown),
        }
    }

    async fn enumerate_snapshot(&self) -> mfm_executor::Result<ExecutorStoreSnapshot> {
        let mut transaction = begin_read(&self.inner.pool)
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        require_binding_in_transaction(&mut transaction, &self.inner.identity)
            .await
            .map_err(|_| ExecutorError::InvalidDurableSnapshot)?;

        let effect_keys = sqlx::query(
            "SELECT DISTINCT effect_key FROM executor_effect_frontiers ORDER BY effect_key",
        )
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| ExecutorError::DurableBackendUnavailable)?
        .into_iter()
        .map(|row| {
            let value = row
                .try_get::<String, _>("effect_key")
                .map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
            EffectKey::parse(value).map_err(|_| ExecutorError::InvalidDurableSnapshot)
        })
        .collect::<mfm_executor::Result<Vec<_>>>()?;

        let mut effects = Vec::with_capacity(effect_keys.len());
        for effect_key in &effect_keys {
            let rows = fetch_effect_rows(&mut transaction, effect_key)
                .await
                .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
            let allocation = fetch_effect_allocation(&mut transaction, effect_key)
                .await
                .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
            let effect = decode_effect_snapshot(
                &self.inner.binding,
                &self.inner.identity,
                effect_key,
                rows,
                allocation,
            )?
            .ok_or(ExecutorError::InvalidDurableSnapshot)?;
            effects.push(effect);
        }

        let resource_keys = fetch_all_resource_keys(&mut transaction)
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        let admitted_owner = self.inner.identity.resource_ownership_ref().cloned();
        let mut resources = Vec::with_capacity(resource_keys.len());
        for key in resource_keys {
            let owner = admitted_owner
                .as_ref()
                .ok_or(ExecutorError::InvalidDurableSnapshot)?;
            if !key.owner.matches(owner.as_content_ref()) {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
            let first_row = fetch_resource_rows_by_parts(&mut transaction, &key.owner, &key.key)
                .await
                .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
            let first_record = first_row
                .first()
                .ok_or(ExecutorError::InvalidDurableSnapshot)?;
            let decoded = ResourceLedgerRecord::from_durable_bytes(&first_record.payload)?;
            if !key.key.matches(decoded.resource_key_ref().as_content_ref()) {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
            let resource_key_ref = decoded.resource_key_ref().clone();
            let records = decode_resource_records(owner, &resource_key_ref, first_row)?;
            resources.push(ExecutorResourceSnapshot::from_store_parts(
                owner.clone(),
                resource_key_ref,
                records,
            ));
        }

        let content_rows = sqlx::query(
            "SELECT schema_id, content_digest, canonical_bytes \
               FROM executor_content_records \
              ORDER BY schema_id, content_digest",
        )
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| ExecutorError::DurableBackendUnavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| ExecutorError::DurableBackendUnavailable)?;

        let content = content_rows
            .into_iter()
            .map(decode_content_row)
            .collect::<mfm_executor::Result<Vec<_>>>()?;
        Ok(ExecutorStoreSnapshot::from_store_parts(
            self.inner.identity.clone(),
            effects,
            resources,
            content,
        ))
    }
}

impl QualifiedPostgresExecutorReadiness {
    /// Performs bounded ledger-identity and independent-fence readiness probes.
    pub async fn readiness(&self) -> Result<PostgresExecutorReadiness> {
        readiness(&self.inner).await
    }
}

async fn readiness(inner: &StoreInner) -> Result<PostgresExecutorReadiness> {
    inner.fence.verify(&inner.pool, &inner.context).await?;
    let probe = probe_writer(&inner.pool).await?;
    if probe.database_name != inner.context.database_name()
        || probe.schema_name != inner.context.schema_name()
        || probe.database_oid != inner.context.database_oid()
    {
        return Err(PostgresExecutorStoreError::WriterFenceRejected);
    }
    verify_binding(&inner.pool, &inner.identity).await?;
    Ok(PostgresExecutorReadiness {
        ledger_ready: true,
        fence_ready: true,
    })
}

/// Deterministic qualification-only PostgreSQL acknowledgement faults.
#[cfg(feature = "qualification-tests")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PostgresExecutorFaultPoint {
    /// No fault.
    None = 0,
    /// Commit succeeds, but the caller receives `OutcomeUnknown`.
    AfterCommitBeforeAcknowledgement = 1,
}

impl ExecutorLedgerStore for QualifiedPostgresExecutorStore {
    fn store_identity(&self) -> &ExecutorLedgerStoreIdentity {
        &self.inner.identity
    }

    fn load_effect<'a>(
        &'a self,
        effect_key: &'a EffectKey,
    ) -> ExecutorFuture<'a, mfm_executor::Result<Option<ExecutorEffectSnapshot>>> {
        Box::pin(async move { self.load_effect_inner(effect_key).await })
    }

    fn load_resource<'a>(
        &'a self,
        resource_ownership_ref: &'a ResourceOwnershipRef,
        resource_key_ref: &'a ResourceKeyRef,
    ) -> ExecutorFuture<'a, mfm_executor::Result<ExecutorResourceSnapshot>> {
        Box::pin(async move {
            self.load_resource_inner(resource_ownership_ref, resource_key_ref)
                .await
        })
    }

    fn load_content<'a>(
        &'a self,
        content_ref: &'a ContentRef,
    ) -> ExecutorFuture<'a, mfm_executor::Result<Option<SchemaQualifiedCanonicalValue>>> {
        Box::pin(async move { self.load_content_inner(content_ref).await })
    }

    fn compare_and_append<'a>(
        &'a self,
        append: ExecutorLedgerAppend,
    ) -> ExecutorFuture<'a, mfm_executor::Result<ExecutorAppendOutcome>> {
        Box::pin(async move { self.compare_and_append_inner(append).await })
    }
}

fn decode_content_row(
    row: sqlx::postgres::PgRow,
) -> mfm_executor::Result<SchemaQualifiedCanonicalValue> {
    let schema = row
        .try_get::<String, _>("schema_id")
        .map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
    let digest = row
        .try_get::<String, _>("content_digest")
        .map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
    let bytes = row
        .try_get::<Vec<u8>, _>("canonical_bytes")
        .map_err(|_| ExecutorError::InvalidDurableSnapshot)?;
    let value = SchemaQualifiedCanonicalValue::new(
        SchemaId::parse(schema).map_err(|_| ExecutorError::InvalidDurableSnapshot)?,
        &bytes,
    )?;
    if value.reference()?.content_digest().as_str() != digest {
        return Err(ExecutorError::RetainedObjectMismatch);
    }
    Ok(value)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WriterProbe {
    database_name: String,
    schema_name: String,
    database_oid: u32,
}

async fn probe_writer(pool: &PgPool) -> Result<WriterProbe> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|_| PostgresExecutorStoreError::WriterRequired)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresExecutorStoreError::WriterRequired)?;
    sqlx::query("SET TRANSACTION READ WRITE")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresExecutorStoreError::WriterRequired)?;
    let row = sqlx::query(
        "SELECT current_database()::text AS database_name, \
                current_schema()::text AS schema_name, \
                (SELECT oid::bigint FROM pg_catalog.pg_database \
                  WHERE datname = current_database()) AS database_oid, \
                pg_catalog.pg_is_in_recovery() AS in_recovery, \
                current_setting('transaction_read_only') AS transaction_read_only, \
                pg_catalog.pg_has_role(current_user, $1, 'MEMBER') AS application_member",
    )
    .bind(APPLICATION_ROLE)
    .fetch_one(&mut *transaction)
    .await
    .map_err(|_| PostgresExecutorStoreError::WriterRequired)?;
    if required_bool(
        &row,
        "in_recovery",
        PostgresExecutorStoreError::WriterRequired,
    )? || required_string(
        &row,
        "transaction_read_only",
        PostgresExecutorStoreError::WriterRequired,
    )? != "off"
        || !required_bool(
            &row,
            "application_member",
            PostgresExecutorStoreError::WriterRequired,
        )?
    {
        return Err(PostgresExecutorStoreError::WriterRequired);
    }
    let database_oid = row
        .try_get::<i64, _>("database_oid")
        .ok()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(PostgresExecutorStoreError::WriterRequired)?;
    let probe = WriterProbe {
        database_name: required_string(
            &row,
            "database_name",
            PostgresExecutorStoreError::WriterRequired,
        )?,
        schema_name: required_string(
            &row,
            "schema_name",
            PostgresExecutorStoreError::WriterRequired,
        )?,
        database_oid,
    };
    transaction
        .rollback()
        .await
        .map_err(|_| PostgresExecutorStoreError::WriterRequired)?;
    Ok(probe)
}

async fn admit_or_verify_binding(
    pool: &PgPool,
    identity: &ExecutorLedgerStoreIdentity,
) -> Result<()> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|_| PostgresExecutorStoreError::Database)?;
    set_application_role(&mut transaction).await?;
    acquire_ledger_lock(&mut transaction, false).await?;
    let row = sqlx::query("SELECT * FROM executor_bindings WHERE singleton")
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| PostgresExecutorStoreError::Database)?;
    if let Some(row) = row {
        require_binding_row(&row, identity)?;
    } else {
        let binding = identity.binding_ref().as_content_ref();
        let generation = identity.durable_ledger_generation_ref();
        let evidence = identity.evidence_authority_ref();
        let owner = identity
            .resource_ownership_ref()
            .map(ResourceOwnershipRef::as_content_ref);
        sqlx::query(
            "INSERT INTO executor_bindings ( \
                 singleton, tenant_scope_id, \
                 executor_binding_schema_id, executor_binding_content_digest, \
                 durable_generation_schema_id, durable_generation_content_digest, \
                 evidence_authority_schema_id, evidence_authority_content_digest, \
                 resource_ownership_schema_id, resource_ownership_content_digest \
             ) VALUES (TRUE, $1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(identity.tenant_scope_id().as_str())
        .bind(binding.schema_id().as_str())
        .bind(binding.content_digest().as_str())
        .bind(generation.schema_id().as_str())
        .bind(generation.content_digest().as_str())
        .bind(evidence.schema_id().as_str())
        .bind(evidence.content_digest().as_str())
        .bind(owner.map(|value| value.schema_id().as_str()))
        .bind(owner.map(|value| value.content_digest().as_str()))
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresExecutorStoreError::Database)?;
    }
    transaction
        .commit()
        .await
        .map_err(|_| PostgresExecutorStoreError::Database)
}

async fn verify_binding(pool: &PgPool, identity: &ExecutorLedgerStoreIdentity) -> Result<()> {
    let mut transaction = begin_read(pool).await?;
    require_binding_in_transaction(&mut transaction, identity).await?;
    transaction
        .commit()
        .await
        .map_err(|_| PostgresExecutorStoreError::Database)
}

async fn require_binding_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    identity: &ExecutorLedgerStoreIdentity,
) -> Result<()> {
    let row = sqlx::query("SELECT * FROM executor_bindings WHERE singleton")
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| PostgresExecutorStoreError::Database)?
        .ok_or(PostgresExecutorStoreError::BindingMismatch)?;
    require_binding_row(&row, identity)
}

fn require_binding_row(
    row: &sqlx::postgres::PgRow,
    identity: &ExecutorLedgerStoreIdentity,
) -> Result<()> {
    let binding = identity.binding_ref().as_content_ref();
    let generation = identity.durable_ledger_generation_ref();
    let evidence = identity.evidence_authority_ref();
    let expected_owner = identity
        .resource_ownership_ref()
        .map(ResourceOwnershipRef::as_content_ref);
    let actual_owner_schema = row
        .try_get::<Option<String>, _>("resource_ownership_schema_id")
        .map_err(|_| PostgresExecutorStoreError::BindingMismatch)?;
    let actual_owner_digest = row
        .try_get::<Option<String>, _>("resource_ownership_content_digest")
        .map_err(|_| PostgresExecutorStoreError::BindingMismatch)?;
    let owner_matches = match (
        expected_owner,
        actual_owner_schema.as_deref(),
        actual_owner_digest.as_deref(),
    ) {
        (None, None, None) => true,
        (Some(owner), Some(schema), Some(digest)) => {
            schema == owner.schema_id().as_str() && digest == owner.content_digest().as_str()
        }
        _ => false,
    };
    if !row
        .try_get::<bool, _>("singleton")
        .map_err(|_| PostgresExecutorStoreError::BindingMismatch)?
        || row
            .try_get::<String, _>("tenant_scope_id")
            .map_err(|_| PostgresExecutorStoreError::BindingMismatch)?
            != identity.tenant_scope_id().as_str()
        || !row_ref_matches(
            row,
            "executor_binding_schema_id",
            "executor_binding_content_digest",
            binding,
        )?
        || !row_ref_matches(
            row,
            "durable_generation_schema_id",
            "durable_generation_content_digest",
            generation,
        )?
        || !row_ref_matches(
            row,
            "evidence_authority_schema_id",
            "evidence_authority_content_digest",
            evidence,
        )?
        || !owner_matches
    {
        return Err(PostgresExecutorStoreError::BindingMismatch);
    }
    Ok(())
}

fn row_ref_matches(
    row: &sqlx::postgres::PgRow,
    schema_column: &str,
    digest_column: &str,
    reference: &ContentRef,
) -> Result<bool> {
    Ok(row
        .try_get::<String, _>(schema_column)
        .map_err(|_| PostgresExecutorStoreError::BindingMismatch)?
        == reference.schema_id().as_str()
        && row
            .try_get::<String, _>(digest_column)
            .map_err(|_| PostgresExecutorStoreError::BindingMismatch)?
            == reference.content_digest().as_str())
}

async fn begin_read(pool: &PgPool) -> Result<Transaction<'_, Postgres>> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|_| PostgresExecutorStoreError::Database)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresExecutorStoreError::Database)?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresExecutorStoreError::Database)?;
    set_application_role(&mut transaction).await?;
    Ok(transaction)
}

async fn set_application_role(transaction: &mut Transaction<'_, Postgres>) -> Result<()> {
    sqlx::query("SET LOCAL ROLE mfm_executor_application")
        .execute(&mut **transaction)
        .await
        .map_err(|_| PostgresExecutorStoreError::Database)?;
    Ok(())
}

async fn acquire_ledger_lock(
    transaction: &mut Transaction<'_, Postgres>,
    shared: bool,
) -> Result<()> {
    let sql = if shared {
        "SELECT pg_catalog.pg_advisory_xact_lock_shared( \
             pg_catalog.hashtextextended(current_schema() || ':' || $1, 0) \
         )"
    } else {
        "SELECT pg_catalog.pg_advisory_xact_lock( \
             pg_catalog.hashtextextended(current_schema() || ':' || $1, 0) \
         )"
    };
    sqlx::query(sql)
        .bind(LEDGER_LOCK_DOMAIN)
        .execute(&mut **transaction)
        .await
        .map_err(|_| PostgresExecutorStoreError::Database)?;
    Ok(())
}

async fn acquire_append_locks(
    transaction: &mut Transaction<'_, Postgres>,
    append: &PreparedAppend,
) -> Result<()> {
    acquire_ledger_lock(transaction, true).await?;
    let effect_lock = format!("{EFFECT_LOCK_DOMAIN}{}", append.effect_key);
    sqlx::query(
        "SELECT pg_catalog.pg_advisory_xact_lock( \
             pg_catalog.hashtextextended(current_schema() || ':' || $1, 0) \
         )",
    )
    .bind(effect_lock)
    .execute(&mut **transaction)
    .await
    .map_err(|_| PostgresExecutorStoreError::Database)?;
    if let Some(resource) = &append.resource {
        let resource_lock = format!(
            "{RESOURCE_LOCK_DOMAIN}{}:{}:{}:{}",
            resource.owner.schema, resource.owner.digest, resource.key.schema, resource.key.digest
        );
        sqlx::query(
            "SELECT pg_catalog.pg_advisory_xact_lock( \
                 pg_catalog.hashtextextended(current_schema() || ':' || $1, 0) \
             )",
        )
        .bind(resource_lock)
        .execute(&mut **transaction)
        .await
        .map_err(|_| PostgresExecutorStoreError::Database)?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredRef {
    schema: String,
    digest: String,
}

impl StoredRef {
    fn from_content_ref(reference: &ContentRef) -> Self {
        Self {
            schema: reference.schema_id().as_str().to_owned(),
            digest: reference.content_digest().as_str().to_owned(),
        }
    }

    fn matches(&self, reference: &ContentRef) -> bool {
        self.schema == reference.schema_id().as_str()
            && self.digest == reference.content_digest().as_str()
    }
}

#[derive(Debug)]
struct PreparedContent {
    reference: StoredRef,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct PreparedResource {
    owner: StoredRef,
    key: StoredRef,
    expected: Option<StoredRef>,
    record: StoredRef,
    predecessor: Option<StoredRef>,
    payload: Vec<u8>,
}

#[derive(Debug)]
struct PreparedAppend {
    effect_key: String,
    expected_effect: Option<StoredRef>,
    frontier: StoredRef,
    predecessor: Option<StoredRef>,
    frontier_payload: Vec<u8>,
    resource: Option<PreparedResource>,
    content: Vec<PreparedContent>,
}

impl PreparedAppend {
    fn new(append: &ExecutorLedgerAppend) -> mfm_executor::Result<Self> {
        let frontier = append.effect_frontier();
        if frontier.effect_key() != append.identity().effect_key()
            || frontier.executor_binding_ref() != append.identity().executor_binding_ref()
            || frontier.request_digest() != append.identity().request_digest()
        {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        let expected_effect = append
            .expected_effect_head()
            .map(DeliveryAuditFrontierRef::as_content_ref)
            .map(StoredRef::from_content_ref);
        let predecessor = frontier
            .predecessor_frontier_ref()
            .map(DeliveryAuditFrontierRef::as_content_ref)
            .map(StoredRef::from_content_ref);
        if expected_effect != predecessor {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        let frontier_ref = frontier.reference()?;
        let resource = append.resource().map(PreparedResource::new).transpose()?;
        let content = append
            .content_objects()
            .iter()
            .map(|value| {
                Ok(PreparedContent {
                    reference: StoredRef::from_content_ref(&value.reference()?),
                    bytes: value.as_bytes().to_vec(),
                })
            })
            .collect::<mfm_executor::Result<Vec<_>>>()?;
        Ok(Self {
            effect_key: append.identity().effect_key().as_str().to_owned(),
            expected_effect,
            frontier: StoredRef::from_content_ref(frontier_ref.as_content_ref()),
            predecessor,
            frontier_payload: frontier.to_durable_bytes()?,
            resource,
            content,
        })
    }
}

impl PreparedResource {
    fn new(resource: &ExecutorResourceAppend) -> mfm_executor::Result<Self> {
        let record = resource.record();
        let expected = resource
            .expected_head()
            .map(ResourceLedgerRecordRef::as_content_ref)
            .map(StoredRef::from_content_ref);
        let predecessor = record
            .predecessor()
            .map(ResourceLedgerRecordRef::as_content_ref)
            .map(StoredRef::from_content_ref);
        if expected != predecessor {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        Ok(Self {
            owner: StoredRef::from_content_ref(record.resource_ownership_ref().as_content_ref()),
            key: StoredRef::from_content_ref(record.resource_key_ref().as_content_ref()),
            expected,
            record: StoredRef::from_content_ref(record.reference()?.as_content_ref()),
            predecessor,
            payload: record.to_durable_bytes()?,
        })
    }
}

#[derive(Debug)]
struct StoredEffectRow {
    ordinal: i64,
    frontier: StoredRef,
    predecessor: Option<StoredRef>,
    payload: Vec<u8>,
}

#[derive(Debug)]
struct StoredResourceRow {
    owner: StoredRef,
    key: StoredRef,
    ordinal: i64,
    record: StoredRef,
    predecessor: Option<StoredRef>,
    linked_effect_key: String,
    payload: Vec<u8>,
}

#[derive(Debug)]
struct StoredAllocation {
    effect_frontier: StoredRef,
    row: StoredResourceRow,
}

async fn fetch_effect_rows(
    transaction: &mut Transaction<'_, Postgres>,
    effect_key: &EffectKey,
) -> Result<Vec<StoredEffectRow>> {
    sqlx::query(
        "SELECT ordinal, frontier_schema_id, frontier_content_digest, \
                predecessor_schema_id, predecessor_content_digest, durable_payload \
           FROM executor_effect_frontiers \
          WHERE effect_key = $1 \
          ORDER BY ordinal",
    )
    .bind(effect_key.as_str())
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| PostgresExecutorStoreError::Database)?
    .into_iter()
    .map(decode_stored_effect_row)
    .collect()
}

fn decode_stored_effect_row(row: sqlx::postgres::PgRow) -> Result<StoredEffectRow> {
    Ok(StoredEffectRow {
        ordinal: row
            .try_get("ordinal")
            .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
        frontier: StoredRef {
            schema: row
                .try_get("frontier_schema_id")
                .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
            digest: row
                .try_get("frontier_content_digest")
                .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
        },
        predecessor: optional_ref(&row, "predecessor_schema_id", "predecessor_content_digest")?,
        payload: row
            .try_get("durable_payload")
            .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
    })
}

async fn fetch_effect_allocation(
    transaction: &mut Transaction<'_, Postgres>,
    effect_key: &EffectKey,
) -> Result<Option<StoredAllocation>> {
    let row = sqlx::query(
        "SELECT \
             link.effect_frontier_schema_id, link.effect_frontier_content_digest, \
             record.resource_ownership_schema_id, record.resource_ownership_content_digest, \
             record.resource_key_schema_id, record.resource_key_content_digest, \
             record.ordinal, record.record_schema_id, record.record_content_digest, \
             record.predecessor_schema_id, record.predecessor_content_digest, \
             record.linked_effect_key, record.durable_payload \
         FROM executor_effect_resource_links AS link \
         JOIN executor_resource_records AS record \
           ON record.resource_ownership_schema_id = link.resource_ownership_schema_id \
          AND record.resource_ownership_content_digest = link.resource_ownership_content_digest \
          AND record.resource_key_schema_id = link.resource_key_schema_id \
          AND record.resource_key_content_digest = link.resource_key_content_digest \
          AND record.record_schema_id = link.resource_record_schema_id \
          AND record.record_content_digest = link.resource_record_content_digest \
        WHERE link.effect_key = $1",
    )
    .bind(effect_key.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| PostgresExecutorStoreError::Database)?;
    row.map(|row| {
        Ok(StoredAllocation {
            effect_frontier: StoredRef {
                schema: row
                    .try_get("effect_frontier_schema_id")
                    .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
                digest: row
                    .try_get("effect_frontier_content_digest")
                    .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
            },
            row: decode_stored_resource_row(row)?,
        })
    })
    .transpose()
}

async fn fetch_resource_rows(
    transaction: &mut Transaction<'_, Postgres>,
    owner: &ResourceOwnershipRef,
    key: &ResourceKeyRef,
) -> Result<Vec<StoredResourceRow>> {
    let owner = StoredRef::from_content_ref(owner.as_content_ref());
    let key = StoredRef::from_content_ref(key.as_content_ref());
    fetch_resource_rows_by_parts(transaction, &owner, &key).await
}

async fn fetch_resource_rows_by_parts(
    transaction: &mut Transaction<'_, Postgres>,
    owner: &StoredRef,
    key: &StoredRef,
) -> Result<Vec<StoredResourceRow>> {
    sqlx::query(
        "SELECT resource_ownership_schema_id, resource_ownership_content_digest, \
                resource_key_schema_id, resource_key_content_digest, ordinal, \
                record_schema_id, record_content_digest, \
                predecessor_schema_id, predecessor_content_digest, \
                linked_effect_key, durable_payload \
           FROM executor_resource_records \
          WHERE resource_ownership_schema_id = $1 \
            AND resource_ownership_content_digest = $2 \
            AND resource_key_schema_id = $3 \
            AND resource_key_content_digest = $4 \
          ORDER BY ordinal",
    )
    .bind(&owner.schema)
    .bind(&owner.digest)
    .bind(&key.schema)
    .bind(&key.digest)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| PostgresExecutorStoreError::Database)?
    .into_iter()
    .map(decode_stored_resource_row)
    .collect()
}

fn decode_stored_resource_row(row: sqlx::postgres::PgRow) -> Result<StoredResourceRow> {
    Ok(StoredResourceRow {
        owner: StoredRef {
            schema: row
                .try_get("resource_ownership_schema_id")
                .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
            digest: row
                .try_get("resource_ownership_content_digest")
                .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
        },
        key: StoredRef {
            schema: row
                .try_get("resource_key_schema_id")
                .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
            digest: row
                .try_get("resource_key_content_digest")
                .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
        },
        ordinal: row
            .try_get("ordinal")
            .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
        record: StoredRef {
            schema: row
                .try_get("record_schema_id")
                .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
            digest: row
                .try_get("record_content_digest")
                .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
        },
        predecessor: optional_ref(&row, "predecessor_schema_id", "predecessor_content_digest")?,
        linked_effect_key: row
            .try_get("linked_effect_key")
            .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
        payload: row
            .try_get("durable_payload")
            .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
    })
}

async fn fetch_all_resource_keys(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<Vec<ResourceParts>> {
    sqlx::query(
        "SELECT DISTINCT \
             resource_ownership_schema_id, resource_ownership_content_digest, \
             resource_key_schema_id, resource_key_content_digest \
         FROM executor_resource_records \
        ORDER BY resource_ownership_schema_id, resource_ownership_content_digest, \
                 resource_key_schema_id, resource_key_content_digest",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| PostgresExecutorStoreError::Database)?
    .into_iter()
    .map(|row| {
        Ok(ResourceParts {
            owner: StoredRef {
                schema: row
                    .try_get("resource_ownership_schema_id")
                    .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
                digest: row
                    .try_get("resource_ownership_content_digest")
                    .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
            },
            key: StoredRef {
                schema: row
                    .try_get("resource_key_schema_id")
                    .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
                digest: row
                    .try_get("resource_key_content_digest")
                    .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
            },
        })
    })
    .collect()
}

struct ResourceParts {
    owner: StoredRef,
    key: StoredRef,
}

fn decode_effect_snapshot(
    binding: &VerifiedExecutorBinding,
    store_identity: &ExecutorLedgerStoreIdentity,
    effect_key: &EffectKey,
    rows: Vec<StoredEffectRow>,
    allocation: Option<StoredAllocation>,
) -> mfm_executor::Result<Option<ExecutorEffectSnapshot>> {
    if rows.is_empty() {
        if allocation.is_some() {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        return Ok(None);
    }
    let mut frontiers = Vec::with_capacity(rows.len());
    for (expected_ordinal, row) in rows.into_iter().enumerate() {
        if usize::try_from(row.ordinal).ok() != Some(expected_ordinal) {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        let frontier = DeliveryAuditFrontier::from_durable_bytes(&row.payload)?;
        if frontier.effect_key() != effect_key
            || !row.frontier.matches(frontier.reference()?.as_content_ref())
            || row.predecessor
                != frontier
                    .predecessor_frontier_ref()
                    .map(DeliveryAuditFrontierRef::as_content_ref)
                    .map(StoredRef::from_content_ref)
        {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        frontiers.push(frontier);
    }
    let first = frontiers
        .first()
        .ok_or(ExecutorError::InvalidDurableSnapshot)?;
    let identity = EffectIdentity::reconstruct(
        binding,
        store_identity.tenant_scope_id().clone(),
        first.executor_binding_ref().clone(),
        effect_key.clone(),
        first.request_digest().clone(),
    )?;
    if frontiers.iter().any(|frontier| {
        frontier.executor_binding_ref() != identity.executor_binding_ref()
            || frontier.request_digest() != identity.request_digest()
    }) {
        return Err(ExecutorError::InvalidDurableSnapshot);
    }
    let allocation = allocation
        .map(|allocation| {
            let mut frontier_linked = false;
            for frontier in &frontiers {
                if allocation
                    .effect_frontier
                    .matches(frontier.reference()?.as_content_ref())
                {
                    frontier_linked = true;
                    break;
                }
            }
            if !frontier_linked {
                return Err(ExecutorError::InvalidDurableSnapshot);
            }
            let record = ResourceLedgerRecord::from_durable_bytes(&allocation.row.payload)?;
            validate_resource_row(&allocation.row, &record)?;
            Ok(record)
        })
        .transpose()?;
    Ok(Some(ExecutorEffectSnapshot::from_store_parts(
        identity, frontiers, allocation,
    )))
}

fn decode_resource_records(
    owner: &ResourceOwnershipRef,
    key: &ResourceKeyRef,
    rows: Vec<StoredResourceRow>,
) -> mfm_executor::Result<Vec<ResourceLedgerRecord>> {
    let mut records = Vec::with_capacity(rows.len());
    for (expected_ordinal, row) in rows.into_iter().enumerate() {
        if usize::try_from(row.ordinal).ok() != Some(expected_ordinal)
            || !row.owner.matches(owner.as_content_ref())
            || !row.key.matches(key.as_content_ref())
        {
            return Err(ExecutorError::InvalidDurableSnapshot);
        }
        let record = ResourceLedgerRecord::from_durable_bytes(&row.payload)?;
        validate_resource_row(&row, &record)?;
        records.push(record);
    }
    Ok(records)
}

fn validate_resource_row(
    row: &StoredResourceRow,
    record: &ResourceLedgerRecord,
) -> mfm_executor::Result<()> {
    if !row
        .owner
        .matches(record.resource_ownership_ref().as_content_ref())
        || !row.key.matches(record.resource_key_ref().as_content_ref())
        || !row.record.matches(record.reference()?.as_content_ref())
        || row.predecessor
            != record
                .predecessor()
                .map(ResourceLedgerRecordRef::as_content_ref)
                .map(StoredRef::from_content_ref)
        || row.linked_effect_key != record.effect_key().as_str()
    {
        return Err(ExecutorError::InvalidDurableSnapshot);
    }
    Ok(())
}

async fn proposal_already_present(
    transaction: &mut Transaction<'_, Postgres>,
    append: &PreparedAppend,
) -> Result<bool> {
    let row = sqlx::query(
        "SELECT ordinal, frontier_schema_id, frontier_content_digest, \
                predecessor_schema_id, predecessor_content_digest, durable_payload \
           FROM executor_effect_frontiers \
          WHERE effect_key = $1 \
            AND frontier_schema_id = $2 \
            AND frontier_content_digest = $3",
    )
    .bind(&append.effect_key)
    .bind(&append.frontier.schema)
    .bind(&append.frontier.digest)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| PostgresExecutorStoreError::Database)?;
    let Some(row) = row else {
        return Ok(false);
    };
    let effect = decode_stored_effect_row(row)?;
    if effect.frontier != append.frontier
        || effect.predecessor != append.predecessor
        || effect.payload != append.frontier_payload
        || !content_is_exactly_present(transaction, &append.content).await?
    {
        return Err(PostgresExecutorStoreError::CorruptLedger);
    }
    match &append.resource {
        None => {
            if fetch_effect_allocation_by_string(transaction, &append.effect_key)
                .await?
                .is_some()
            {
                return Err(PostgresExecutorStoreError::CorruptLedger);
            }
            Ok(true)
        }
        Some(resource) => {
            let allocation = fetch_effect_allocation_by_string(transaction, &append.effect_key)
                .await?
                .ok_or(PostgresExecutorStoreError::CorruptLedger)?;
            if allocation.effect_frontier != append.frontier
                || allocation.row.owner != resource.owner
                || allocation.row.key != resource.key
                || allocation.row.record != resource.record
                || allocation.row.predecessor != resource.predecessor
                || allocation.row.linked_effect_key != append.effect_key
                || allocation.row.payload != resource.payload
            {
                return Err(PostgresExecutorStoreError::CorruptLedger);
            }
            Ok(true)
        }
    }
}

async fn fetch_effect_allocation_by_string(
    transaction: &mut Transaction<'_, Postgres>,
    effect_key: &str,
) -> Result<Option<StoredAllocation>> {
    let parsed =
        EffectKey::parse(effect_key).map_err(|_| PostgresExecutorStoreError::CorruptLedger)?;
    fetch_effect_allocation(transaction, &parsed).await
}

async fn heads_match(
    transaction: &mut Transaction<'_, Postgres>,
    append: &PreparedAppend,
) -> Result<bool> {
    let effect_head = sqlx::query(
        "SELECT frontier_schema_id, frontier_content_digest \
           FROM executor_effect_heads \
          WHERE effect_key = $1",
    )
    .bind(&append.effect_key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| PostgresExecutorStoreError::Database)?
    .map(|row| {
        Ok(StoredRef {
            schema: row
                .try_get("frontier_schema_id")
                .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
            digest: row
                .try_get("frontier_content_digest")
                .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
        })
    })
    .transpose()?;
    if effect_head != append.expected_effect {
        return Ok(false);
    }
    let Some(resource) = &append.resource else {
        return Ok(true);
    };
    let resource_head = current_resource_head(transaction, resource).await?;
    if resource_head != resource.expected {
        return Ok(false);
    }
    let existing_link = sqlx::query(
        "SELECT TRUE AS present FROM executor_effect_resource_links WHERE effect_key = $1",
    )
    .bind(&append.effect_key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| PostgresExecutorStoreError::Database)?;
    Ok(existing_link.is_none())
}

async fn current_resource_head(
    transaction: &mut Transaction<'_, Postgres>,
    resource: &PreparedResource,
) -> Result<Option<StoredRef>> {
    sqlx::query(
        "SELECT record_schema_id, record_content_digest \
           FROM executor_resource_heads \
          WHERE resource_ownership_schema_id = $1 \
            AND resource_ownership_content_digest = $2 \
            AND resource_key_schema_id = $3 \
            AND resource_key_content_digest = $4",
    )
    .bind(&resource.owner.schema)
    .bind(&resource.owner.digest)
    .bind(&resource.key.schema)
    .bind(&resource.key.digest)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| PostgresExecutorStoreError::Database)?
    .map(|row| {
        Ok(StoredRef {
            schema: row
                .try_get("record_schema_id")
                .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
            digest: row
                .try_get("record_content_digest")
                .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
        })
    })
    .transpose()
}

async fn content_is_compatible(
    transaction: &mut Transaction<'_, Postgres>,
    content: &[PreparedContent],
) -> Result<bool> {
    for value in content {
        let existing = sqlx::query(
            "SELECT canonical_bytes \
               FROM executor_content_records \
              WHERE schema_id = $1 AND content_digest = $2",
        )
        .bind(&value.reference.schema)
        .bind(&value.reference.digest)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| PostgresExecutorStoreError::Database)?;
        if let Some(existing) = existing {
            let bytes = existing
                .try_get::<Vec<u8>, _>("canonical_bytes")
                .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?;
            if bytes != value.bytes {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

async fn content_is_exactly_present(
    transaction: &mut Transaction<'_, Postgres>,
    content: &[PreparedContent],
) -> Result<bool> {
    for value in content {
        let existing = sqlx::query(
            "SELECT canonical_bytes \
               FROM executor_content_records \
              WHERE schema_id = $1 AND content_digest = $2",
        )
        .bind(&value.reference.schema)
        .bind(&value.reference.digest)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| PostgresExecutorStoreError::Database)?;
        let Some(existing) = existing else {
            return Ok(false);
        };
        if existing
            .try_get::<Vec<u8>, _>("canonical_bytes")
            .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?
            != value.bytes
        {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn insert_content(
    transaction: &mut Transaction<'_, Postgres>,
    content: &[PreparedContent],
) -> Result<()> {
    for value in content {
        sqlx::query(
            "INSERT INTO executor_content_records (schema_id, content_digest, canonical_bytes) \
             VALUES ($1, $2, $3) \
             ON CONFLICT (schema_id, content_digest) DO NOTHING",
        )
        .bind(&value.reference.schema)
        .bind(&value.reference.digest)
        .bind(&value.bytes)
        .execute(&mut **transaction)
        .await
        .map_err(|_| PostgresExecutorStoreError::Database)?;
    }
    Ok(())
}

async fn insert_effect_frontier(
    transaction: &mut Transaction<'_, Postgres>,
    append: &PreparedAppend,
) -> Result<()> {
    let ordinal = next_effect_ordinal(transaction, &append.effect_key).await?;
    sqlx::query(
        "INSERT INTO executor_effect_frontiers ( \
             effect_key, ordinal, frontier_schema_id, frontier_content_digest, \
             predecessor_schema_id, predecessor_content_digest, durable_payload \
         ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&append.effect_key)
    .bind(ordinal)
    .bind(&append.frontier.schema)
    .bind(&append.frontier.digest)
    .bind(
        append
            .predecessor
            .as_ref()
            .map(|value| value.schema.as_str()),
    )
    .bind(
        append
            .predecessor
            .as_ref()
            .map(|value| value.digest.as_str()),
    )
    .bind(&append.frontier_payload)
    .execute(&mut **transaction)
    .await
    .map_err(|_| PostgresExecutorStoreError::Database)?;
    Ok(())
}

async fn next_effect_ordinal(
    transaction: &mut Transaction<'_, Postgres>,
    effect_key: &str,
) -> Result<i64> {
    let current = sqlx::query("SELECT ordinal FROM executor_effect_heads WHERE effect_key = $1")
        .bind(effect_key)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| PostgresExecutorStoreError::Database)?
        .map(|row| {
            row.try_get::<i64, _>("ordinal")
                .map_err(|_| PostgresExecutorStoreError::CorruptLedger)
        })
        .transpose()?;
    current
        .map_or(Some(0), |value| value.checked_add(1))
        .ok_or(PostgresExecutorStoreError::CorruptLedger)
}

async fn insert_resource_and_link(
    transaction: &mut Transaction<'_, Postgres>,
    append: &PreparedAppend,
) -> Result<()> {
    let resource = append
        .resource
        .as_ref()
        .ok_or(PostgresExecutorStoreError::CorruptLedger)?;
    let ordinal = next_resource_ordinal(transaction, resource).await?;
    sqlx::query(
        "INSERT INTO executor_resource_records ( \
             resource_ownership_schema_id, resource_ownership_content_digest, \
             resource_key_schema_id, resource_key_content_digest, ordinal, \
             record_schema_id, record_content_digest, \
             predecessor_schema_id, predecessor_content_digest, \
             linked_effect_key, durable_payload \
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(&resource.owner.schema)
    .bind(&resource.owner.digest)
    .bind(&resource.key.schema)
    .bind(&resource.key.digest)
    .bind(ordinal)
    .bind(&resource.record.schema)
    .bind(&resource.record.digest)
    .bind(
        resource
            .predecessor
            .as_ref()
            .map(|value| value.schema.as_str()),
    )
    .bind(
        resource
            .predecessor
            .as_ref()
            .map(|value| value.digest.as_str()),
    )
    .bind(&append.effect_key)
    .bind(&resource.payload)
    .execute(&mut **transaction)
    .await
    .map_err(|_| PostgresExecutorStoreError::Database)?;

    sqlx::query(
        "INSERT INTO executor_effect_resource_links ( \
             effect_key, effect_frontier_schema_id, effect_frontier_content_digest, \
             resource_ownership_schema_id, resource_ownership_content_digest, \
             resource_key_schema_id, resource_key_content_digest, \
             resource_record_schema_id, resource_record_content_digest \
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(&append.effect_key)
    .bind(&append.frontier.schema)
    .bind(&append.frontier.digest)
    .bind(&resource.owner.schema)
    .bind(&resource.owner.digest)
    .bind(&resource.key.schema)
    .bind(&resource.key.digest)
    .bind(&resource.record.schema)
    .bind(&resource.record.digest)
    .execute(&mut **transaction)
    .await
    .map_err(|_| PostgresExecutorStoreError::Database)?;
    Ok(())
}

async fn next_resource_ordinal(
    transaction: &mut Transaction<'_, Postgres>,
    resource: &PreparedResource,
) -> Result<i64> {
    let current = sqlx::query(
        "SELECT ordinal \
           FROM executor_resource_heads \
          WHERE resource_ownership_schema_id = $1 \
            AND resource_ownership_content_digest = $2 \
            AND resource_key_schema_id = $3 \
            AND resource_key_content_digest = $4",
    )
    .bind(&resource.owner.schema)
    .bind(&resource.owner.digest)
    .bind(&resource.key.schema)
    .bind(&resource.key.digest)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| PostgresExecutorStoreError::Database)?
    .map(|row| {
        row.try_get::<i64, _>("ordinal")
            .map_err(|_| PostgresExecutorStoreError::CorruptLedger)
    })
    .transpose()?;
    current
        .map_or(Some(0), |value| value.checked_add(1))
        .ok_or(PostgresExecutorStoreError::CorruptLedger)
}

fn optional_ref(
    row: &sqlx::postgres::PgRow,
    schema_column: &str,
    digest_column: &str,
) -> Result<Option<StoredRef>> {
    match (
        row.try_get::<Option<String>, _>(schema_column)
            .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
        row.try_get::<Option<String>, _>(digest_column)
            .map_err(|_| PostgresExecutorStoreError::CorruptLedger)?,
    ) {
        (None, None) => Ok(None),
        (Some(schema), Some(digest)) => Ok(Some(StoredRef { schema, digest })),
        _ => Err(PostgresExecutorStoreError::CorruptLedger),
    }
}

fn required_string(
    row: &sqlx::postgres::PgRow,
    column: &str,
    error: PostgresExecutorStoreError,
) -> Result<String> {
    row.try_get(column).map_err(|_| error)
}

fn required_bool(
    row: &sqlx::postgres::PgRow,
    column: &str,
    error: PostgresExecutorStoreError,
) -> Result<bool> {
    row.try_get(column).map_err(|_| error)
}
