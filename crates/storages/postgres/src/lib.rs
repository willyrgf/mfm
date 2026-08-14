#![warn(missing_docs)]
//! Durable PostgreSQL storage for the append-only MFM frame stream.
//!
//! This crate owns only physical ordering, idempotency, and the admitted durability profile. It
//! stores one canonical frame per append and delegates semantic qualification and reduction to
//! `mfm-store`.

use mfm_ids::{AppendRequestId, ContentDigest, RunId, StoreEpoch, StoreScopeId, TenantScopeId};
use mfm_store::backend::{
    BackendAppendCommand, BackendAppendOutcome, BackendConfigurationOutcome, BackendError,
    BackendFuture, BackendResult, ConfigurationAppendCommand, RawConfigurationRevision,
    RawFactPublication, RawFactSnapshot, RawFrameBytes, RawHistoryLoadLimit, RawRunPrefix,
    StructuredStoreBackend, StructuredStoreIdentity,
};
use sqlx::{postgres::PgPoolOptions, PgPool};

const SCHEMA_CONTRACT: &str = "mfm.structured-run-history-postgres.v8";

/// The only durability claim admitted by the cutover.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurabilityProfile {
    /// Survives a crash/restart of the admitted primary.
    PrimaryCrashRestart,
}

/// Redaction-safe PostgreSQL storage error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PostgresError {
    /// The configured durability profile is not admitted.
    #[error("PostgreSQL durability profile is not admitted")]
    Durability,
    /// The writer scope or epoch does not match the opened store.
    #[error("PostgreSQL Store identity changed")]
    Identity,
    /// The required fresh schema is absent or has the wrong contract.
    #[error("PostgreSQL schema contract is invalid")]
    Schema,
    /// A retained frame failed strict canonical qualification.
    #[error("PostgreSQL retained frame is invalid")]
    InvalidRecord,
    /// The append identity or logical record conflicts with retained history.
    #[error("PostgreSQL append conflicts with retained history")]
    Conflict,
    /// The requested run is absent from this tenant partition.
    #[error("PostgreSQL run was not found")]
    NotFound,
    /// A physical bound was exceeded.
    #[error("PostgreSQL storage capacity bound exceeded")]
    Capacity,
    /// The transaction outcome was unknown after submission.
    #[error("PostgreSQL acknowledgement outcome is unknown")]
    AcknowledgementUnknown,
    /// The database operation failed without exposing driver diagnostics.
    #[error("PostgreSQL operation failed")]
    Storage,
}

/// Result type for the PostgreSQL mechanical adapter.
pub type Result<T> = std::result::Result<T, PostgresError>;

/// Opened PostgreSQL Store identity.
pub struct PostgresStore {
    pool: PgPool,
    scope: StoreScopeId,
    epoch: StoreEpoch,
    tenant: TenantScopeId,
    profile: DurabilityProfile,
}

impl PostgresStore {
    /// Connects to one caller-selected PostgreSQL database and checks its fresh target schema.
    pub async fn connect(
        database_url: &str,
        scope: StoreScopeId,
        epoch: StoreEpoch,
        tenant: TenantScopeId,
        profile: DurabilityProfile,
    ) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(database_url)
            .await
            .map_err(|_| PostgresError::Storage)?;
        let store = Self::from_pool(pool, scope, epoch, tenant, profile)?;
        store.check_ready().await?;
        Ok(store)
    }

    /// Wraps an already opened pool without exposing it to semantic callers.
    pub fn from_pool(
        pool: PgPool,
        scope: StoreScopeId,
        epoch: StoreEpoch,
        tenant: TenantScopeId,
        profile: DurabilityProfile,
    ) -> Result<Self> {
        if !matches!(profile, DurabilityProfile::PrimaryCrashRestart) {
            return Err(PostgresError::Durability);
        }
        Ok(Self {
            pool,
            scope,
            epoch,
            tenant,
            profile,
        })
    }

    /// Applies the fresh baseline and records its initial Store scope and writer epoch.
    pub async fn migrate(pool: &PgPool, scope: &StoreScopeId, epoch: StoreEpoch) -> Result<()> {
        for statement in include_str!("../migrations/0001_single_trust.sql").split(';') {
            let statement = statement.trim();
            if !statement.is_empty() {
                sqlx::query(statement)
                    .execute(pool)
                    .await
                    .map_err(|_| PostgresError::Storage)?;
            }
        }
        let epoch = i64::try_from(epoch.get()).map_err(|_| PostgresError::Identity)?;
        sqlx::query(
            "INSERT INTO mfm_store_schema (schema_contract, store_scope_id, store_epoch)
             VALUES ($1, $2, $3)
             ON CONFLICT (schema_contract) DO NOTHING",
        )
        .bind(SCHEMA_CONTRACT)
        .bind(scope.as_str())
        .bind(epoch)
        .execute(pool)
        .await
        .map_err(|_| PostgresError::Schema)?;
        let recorded: Option<(String, i64)> = sqlx::query_as(
            "SELECT store_scope_id, store_epoch
             FROM mfm_store_schema
             WHERE schema_contract = $1",
        )
        .bind(SCHEMA_CONTRACT)
        .fetch_optional(pool)
        .await
        .map_err(|_| PostgresError::Schema)?;
        match recorded {
            Some((recorded_scope, recorded_epoch))
                if recorded_scope == scope.as_str() && recorded_epoch == epoch =>
            {
                Ok(())
            }
            _ => Err(PostgresError::Identity),
        }
    }

    /// Rotates a restored database to a fresh Store scope or writer epoch.
    ///
    /// The previous pair is checked under a row lock. Existing frame partitions are not
    /// rewritten; callers must construct a new backend with the next pair after this commits.
    pub async fn rotate_identity(
        pool: &PgPool,
        previous_scope: &StoreScopeId,
        previous_epoch: StoreEpoch,
        next_scope: &StoreScopeId,
        next_epoch: StoreEpoch,
    ) -> Result<()> {
        if previous_scope == next_scope && previous_epoch == next_epoch {
            return Err(PostgresError::Identity);
        }
        let previous_epoch =
            i64::try_from(previous_epoch.get()).map_err(|_| PostgresError::Identity)?;
        let next_epoch = i64::try_from(next_epoch.get()).map_err(|_| PostgresError::Identity)?;
        let mut transaction = pool.begin().await.map_err(|_| PostgresError::Storage)?;
        let recorded: Option<(String, i64)> = sqlx::query_as(
            "SELECT store_scope_id, store_epoch
             FROM mfm_store_schema
             WHERE schema_contract = $1
             FOR UPDATE",
        )
        .bind(SCHEMA_CONTRACT)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| PostgresError::Schema)?;
        if recorded != Some((previous_scope.as_str().to_owned(), previous_epoch)) {
            return Err(PostgresError::Identity);
        }
        sqlx::query(
            "UPDATE mfm_store_schema
             SET store_scope_id = $1, store_epoch = $2
             WHERE schema_contract = $3",
        )
        .bind(next_scope.as_str())
        .bind(next_epoch)
        .bind(SCHEMA_CONTRACT)
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresError::Storage)?;
        transaction
            .commit()
            .await
            .map_err(|_| PostgresError::AcknowledgementUnknown)
    }

    /// Checks only schema, identity, and admitted primary durability; it never enumerates runs.
    pub async fn check_ready(&self) -> Result<()> {
        self.check_schema_and_durability().await?;
        self.verify_open_identity().await
    }

    async fn check_schema_and_durability(&self) -> Result<()> {
        if !matches!(self.profile, DurabilityProfile::PrimaryCrashRestart) {
            return Err(PostgresError::Durability);
        }
        let contract: Option<String> = sqlx::query_scalar(
            "SELECT schema_contract FROM mfm_store_schema WHERE schema_contract = $1",
        )
        .bind(SCHEMA_CONTRACT)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| PostgresError::Schema)?;
        if contract.as_deref() != Some(SCHEMA_CONTRACT) {
            return Err(PostgresError::Schema);
        }
        let settings: (bool, String, String, String) = sqlx::query_as(
            "SELECT pg_is_in_recovery(), current_setting('fsync'),
                    current_setting('full_page_writes'), current_setting('synchronous_commit')",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|_| PostgresError::Durability)?;
        if settings.0 || settings.1 != "on" || settings.2 != "on" || settings.3 == "off" {
            return Err(PostgresError::Durability);
        }
        Ok(())
    }

    async fn verify_open_identity(&self) -> Result<()> {
        let epoch = i64::try_from(self.epoch.get()).map_err(|_| PostgresError::Identity)?;
        let recorded: Option<(String, i64)> = sqlx::query_as(
            "SELECT store_scope_id, store_epoch
             FROM mfm_store_schema
             WHERE schema_contract = $1",
        )
        .bind(SCHEMA_CONTRACT)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| PostgresError::Schema)?;
        match recorded {
            Some((scope, recorded_epoch))
                if scope == self.scope.as_str() && recorded_epoch == epoch =>
            {
                Ok(())
            }
            _ => Err(PostgresError::Identity),
        }
    }

    async fn verify_transaction_identity(
        &self,
        transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> BackendResult<()> {
        let epoch = i64::try_from(self.epoch.get()).map_err(|_| BackendError::Capacity)?;
        let recorded: Option<(String, i64)> = sqlx::query_as(
            "SELECT store_scope_id, store_epoch
             FROM mfm_store_schema
             WHERE schema_contract = $1
             FOR SHARE",
        )
        .bind(SCHEMA_CONTRACT)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| BackendError::Storage)?;
        match recorded {
            Some((scope, recorded_epoch))
                if scope == self.scope.as_str() && recorded_epoch == epoch =>
            {
                Ok(())
            }
            _ => Err(BackendError::Identity),
        }
    }

    fn map_identity_check(error: PostgresError) -> BackendError {
        match error {
            PostgresError::Identity => BackendError::Identity,
            PostgresError::Capacity => BackendError::Capacity,
            _ => BackendError::Storage,
        }
    }

    /// Returns the admitted durability profile.
    pub const fn profile(&self) -> DurabilityProfile {
        self.profile
    }

    /// Returns the fixed Store scope.
    pub const fn scope(&self) -> &StoreScopeId {
        &self.scope
    }

    /// Returns the immutable writer epoch.
    pub const fn epoch(&self) -> StoreEpoch {
        self.epoch
    }

    /// Returns the fixed tenant partition.
    pub const fn tenant(&self) -> &TenantScopeId {
        &self.tenant
    }
}

impl StructuredStoreBackend for PostgresStore {
    fn identity(&self) -> StructuredStoreIdentity {
        StructuredStoreIdentity::new(self.scope.clone(), self.epoch, self.tenant.clone())
    }

    fn check_ready<'a>(&'a self) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            PostgresStore::check_ready(self)
                .await
                .map_err(|error| match error {
                    PostgresError::Durability => BackendError::Unsupported,
                    PostgresError::Schema => BackendError::Unsupported,
                    PostgresError::Identity => BackendError::Identity,
                    _ => BackendError::Storage,
                })
        })
    }

    fn load_complete_prefix<'a>(
        &'a self,
        run_id: &'a RunId,
        limit: RawHistoryLoadLimit,
    ) -> BackendFuture<'a, Option<RawRunPrefix>> {
        Box::pin(async move {
            self.verify_open_identity()
                .await
                .map_err(PostgresStore::map_identity_check)?;
            let epoch = i64::try_from(self.epoch.get()).map_err(|_| BackendError::Capacity)?;
            let mut transaction = self.pool.begin().await.map_err(|_| BackendError::Storage)?;
            self.verify_transaction_identity(&mut transaction).await?;
            let rows: Vec<(i64, String, Vec<u8>, String, String)> = sqlx::query_as(
                "SELECT run_sequence, append_request_id, frame_bytes, frame_digest, head_digest
                 FROM mfm_run_frames
                 WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3 AND run_id = $4
                 ORDER BY run_sequence ASC
                 LIMIT $5",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .bind(run_id.as_str())
            .bind(
                i64::try_from(limit.max_frames().saturating_add(1))
                    .map_err(|_| BackendError::Capacity)?,
            )
            .fetch_all(&mut *transaction)
            .await
            .map_err(|_| BackendError::Storage)?;
            let head: Option<(i64, String)> = sqlx::query_as(
                "SELECT head_sequence, head_digest
                 FROM mfm_run_heads
                 WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3 AND run_id = $4",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .bind(run_id.as_str())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| BackendError::Storage)?;
            transaction
                .commit()
                .await
                .map_err(|_| BackendError::AcknowledgementUnknown)?;
            if rows.is_empty() {
                if head.is_some() {
                    return Err(BackendError::Storage);
                }
                return Ok(None);
            }
            if rows.len() > limit.max_frames() {
                return Err(BackendError::Capacity);
            }
            let mut total_bytes = 0usize;
            let mut frames = Vec::with_capacity(rows.len());
            for (sequence, append_request_id, bytes, frame_digest, head_digest) in rows {
                total_bytes = total_bytes
                    .checked_add(bytes.len())
                    .ok_or(BackendError::Capacity)?;
                if total_bytes > limit.max_bytes() {
                    return Err(BackendError::Capacity);
                }
                frames.push(RawFrameBytes::new(
                    u64::try_from(sequence).map_err(|_| BackendError::Storage)?,
                    AppendRequestId::new(append_request_id).map_err(|_| BackendError::Storage)?,
                    bytes,
                    ContentDigest::parse(&frame_digest).map_err(|_| BackendError::Storage)?,
                    ContentDigest::parse(&head_digest).map_err(|_| BackendError::Storage)?,
                )?);
            }
            let Some((head_sequence, head_digest)) = head else {
                return Err(BackendError::Storage);
            };
            let retained_head = frames.last().ok_or(BackendError::Storage)?;
            if head_sequence < 0
                || u64::try_from(head_sequence).map_err(|_| BackendError::Storage)?
                    != retained_head.sequence()
                || ContentDigest::parse(&head_digest).map_err(|_| BackendError::Storage)?
                    != *retained_head.head_digest()
            {
                return Err(BackendError::Storage);
            }
            RawRunPrefix::new(frames).map(Some)
        })
    }

    fn compare_and_append<'a>(
        &'a self,
        command: &'a BackendAppendCommand<'a>,
    ) -> BackendFuture<'a, BackendAppendOutcome> {
        Box::pin(async move {
            self.verify_open_identity()
                .await
                .map_err(PostgresStore::map_identity_check)?;
            if command.identity()
                != &StructuredStoreIdentity::new(
                    self.scope.clone(),
                    self.epoch,
                    self.tenant.clone(),
                )
            {
                return Err(BackendError::Identity);
            }
            if command.frame_bytes().is_empty()
                || command.frame_bytes().len() > mfm_journal::single_trust::MAX_FRAME_BYTES
                || command.expected_sequence() == 0
            {
                return Err(BackendError::Capacity);
            }
            let mut transaction = self.pool.begin().await.map_err(|_| BackendError::Storage)?;
            self.verify_transaction_identity(&mut transaction).await?;
            // A missing head row cannot be locked with `FOR UPDATE`.  Serialize only this
            // identity's short transaction so concurrent genesis commands resolve through the
            // same Found/stale-head path as an already materialized head row; this is not a
            // process lease or a retained writer lock.
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(format!(
                    "history:{}:{}:{}:{}",
                    self.scope.as_str(),
                    self.epoch.get(),
                    self.tenant.as_str(),
                    command.run_id().as_str()
                ))
                .execute(&mut *transaction)
                .await
                .map_err(|_| BackendError::Storage)?;
            let epoch = i64::try_from(self.epoch.get()).map_err(|_| BackendError::Capacity)?;
            let existing: Option<(i64, Vec<u8>, String, String)> = sqlx::query_as(
                "SELECT run_sequence, frame_bytes, frame_digest, head_digest
                 FROM mfm_run_frames
                 WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3
                   AND run_id = $4 AND append_request_id = $5",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .bind(command.run_id().as_str())
            .bind(command.append_request_id().as_str())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| BackendError::Storage)?;
            if let Some((sequence, bytes, frame_digest, head_digest)) = existing {
                let same = bytes == command.frame_bytes()
                    && frame_digest == command.frame_digest().as_str()
                    && head_digest == command.head_digest().as_str();
                let commit = transaction.commit().await;
                if commit.is_err() {
                    return Err(BackendError::AcknowledgementUnknown);
                }
                if same {
                    return Ok(BackendAppendOutcome::Found(Box::new(RawFrameBytes::new(
                        u64::try_from(sequence).map_err(|_| BackendError::Storage)?,
                        command.append_request_id().clone(),
                        bytes,
                        ContentDigest::parse(&frame_digest).map_err(|_| BackendError::Storage)?,
                        ContentDigest::parse(&head_digest).map_err(|_| BackendError::Storage)?,
                    )?)));
                }
                return Err(BackendError::Conflict);
            }

            let current: Option<(i64, String)> = sqlx::query_as(
                "SELECT head_sequence, head_digest
                 FROM mfm_run_heads
                 WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3 AND run_id = $4
                 FOR UPDATE",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .bind(command.run_id().as_str())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| BackendError::Storage)?;
            let actual = current.as_ref().map_or(0, |value| value.0);
            let expected = u64::try_from(actual)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or(BackendError::Capacity)?;
            if command.expected_sequence() != expected {
                transaction
                    .commit()
                    .await
                    .map_err(|_| BackendError::AcknowledgementUnknown)?;
                return Ok(BackendAppendOutcome::StaleHead {
                    actual_sequence: u64::try_from(actual).map_err(|_| BackendError::Storage)?,
                });
            }
            if actual as usize >= mfm_journal::single_trust::MAX_RUN_FRAMES {
                return Err(BackendError::Capacity);
            }
            let existing_bytes: i64 = sqlx::query_scalar(
                "SELECT COALESCE(SUM(octet_length(frame_bytes)), 0)::BIGINT
                 FROM mfm_run_frames
                 WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3
                   AND run_id = $4",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .bind(command.run_id().as_str())
            .fetch_one(&mut *transaction)
            .await
            .map_err(|_| BackendError::Storage)?;
            let candidate_bytes =
                i64::try_from(command.frame_bytes().len()).map_err(|_| BackendError::Capacity)?;
            if existing_bytes
                .checked_add(candidate_bytes)
                .filter(|bytes| *bytes <= mfm_journal::single_trust::MAX_RUN_FRAME_BYTES as i64)
                .is_none()
            {
                return Err(BackendError::Capacity);
            }
            let actual_head = current
                .as_ref()
                .map(|(_, digest)| ContentDigest::parse(digest))
                .transpose()
                .map_err(|_| BackendError::Storage)?;
            if command.previous_head_digest() != actual_head.as_ref() {
                transaction
                    .commit()
                    .await
                    .map_err(|_| BackendError::AcknowledgementUnknown)?;
                return Ok(BackendAppendOutcome::StaleHead {
                    actual_sequence: u64::try_from(actual).map_err(|_| BackendError::Storage)?,
                });
            }
            if command.is_admission() != (actual == 0) {
                return Err(BackendError::Conflict);
            }
            if command.fact_publication().is_some() || command.fact_frontier().is_some() {
                // Materialize the zero head before locking it. PostgreSQL cannot lock a row that
                // does not exist, so selecting an optional head alone would let concurrent first
                // publications both observe zero and race into a generic unique-key conflict.
                sqlx::query(
                    "INSERT INTO mfm_fact_heads
                     (store_scope_id, store_epoch, tenant_scope_id, publication_sequence)
                     VALUES ($1, $2, $3, 0)
                     ON CONFLICT (store_scope_id, store_epoch, tenant_scope_id) DO NOTHING",
                )
                .bind(self.scope.as_str())
                .bind(epoch)
                .bind(self.tenant.as_str())
                .execute(&mut *transaction)
                .await
                .map_err(|_| BackendError::Storage)?;
                let fact_head: i64 = sqlx::query_scalar(
                    "SELECT publication_sequence FROM mfm_fact_heads
                     WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3
                     FOR UPDATE",
                )
                .bind(self.scope.as_str())
                .bind(epoch)
                .bind(self.tenant.as_str())
                .fetch_one(&mut *transaction)
                .await
                .map_err(|_| BackendError::Storage)?;
                let actual_fact = fact_head;
                if let Some(publication) = command.fact_publication() {
                    let expected_fact = actual_fact.saturating_add(1);
                    if i64::try_from(publication.publication_sequence())
                        .map_err(|_| BackendError::Capacity)?
                        != expected_fact
                    {
                        return Err(BackendError::FactFrontierChanged);
                    }
                }
                if command
                    .fact_frontier()
                    .is_some_and(|frontier| i64::try_from(frontier).ok() != Some(actual_fact))
                {
                    return Err(BackendError::FactFrontierChanged);
                }
            }

            sqlx::query(
                "INSERT INTO mfm_run_frames
                 (store_scope_id, store_epoch, tenant_scope_id, run_id, run_sequence,
                  append_request_id, frame_bytes, frame_digest, head_digest)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .bind(command.run_id().as_str())
            .bind(i64::try_from(command.expected_sequence()).map_err(|_| BackendError::Capacity)?)
            .bind(command.append_request_id().as_str())
            .bind(command.frame_bytes())
            .bind(command.frame_digest().as_str())
            .bind(command.head_digest().as_str())
            .execute(&mut *transaction)
            .await
            .map_err(|_| BackendError::Conflict)?;

            if let Some(publication) = command.fact_publication() {
                let sequence = i64::try_from(publication.publication_sequence())
                    .map_err(|_| BackendError::Capacity)?;
                sqlx::query(
                    "INSERT INTO mfm_fact_heads
                     (store_scope_id, store_epoch, tenant_scope_id, publication_sequence)
                     VALUES ($1, $2, $3, $4)
                     ON CONFLICT (store_scope_id, store_epoch, tenant_scope_id)
                     DO UPDATE SET publication_sequence = EXCLUDED.publication_sequence",
                )
                .bind(self.scope.as_str())
                .bind(epoch)
                .bind(self.tenant.as_str())
                .bind(sequence)
                .execute(&mut *transaction)
                .await
                .map_err(|_| BackendError::Storage)?;
                sqlx::query(
                    "INSERT INTO mfm_fact_publications
                     (store_scope_id, store_epoch, tenant_scope_id, publication_sequence,
                      run_id, run_sequence, proposal_set_ref)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)",
                )
                .bind(self.scope.as_str())
                .bind(epoch)
                .bind(self.tenant.as_str())
                .bind(sequence)
                .bind(publication.run_id().as_str())
                .bind(
                    i64::try_from(publication.run_sequence())
                        .map_err(|_| BackendError::Capacity)?,
                )
                .bind(
                    serde_json::to_string(publication.proposal_set_ref())
                        .map_err(|_| BackendError::Storage)?,
                )
                .execute(&mut *transaction)
                .await
                .map_err(|_| BackendError::Conflict)?;
            }
            sqlx::query(
                "INSERT INTO mfm_run_heads
                 (store_scope_id, store_epoch, tenant_scope_id, run_id, head_sequence, head_digest)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (store_scope_id, store_epoch, tenant_scope_id, run_id)
                 DO UPDATE SET head_sequence = EXCLUDED.head_sequence, head_digest = EXCLUDED.head_digest",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .bind(command.run_id().as_str())
            .bind(i64::try_from(command.expected_sequence()).map_err(|_| BackendError::Capacity)?)
            .bind(command.head_digest().as_str())
            .execute(&mut *transaction)
            .await
            .map_err(|_| BackendError::Storage)?;
            transaction
                .commit()
                .await
                .map_err(|_| BackendError::AcknowledgementUnknown)?;
            Ok(BackendAppendOutcome::NewlyCommitted)
        })
    }

    fn load_configuration<'a>(&'a self) -> BackendFuture<'a, Vec<RawConfigurationRevision>> {
        Box::pin(async move {
            self.verify_open_identity()
                .await
                .map_err(PostgresStore::map_identity_check)?;
            let epoch = i64::try_from(self.epoch.get()).map_err(|_| BackendError::Capacity)?;
            let head: Option<(i64, i64)> = sqlx::query_as(
                "SELECT head_sequence, total_bytes
                 FROM mfm_configuration_heads
                 WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| BackendError::Storage)?;
            let expected_head = head.map_or(0, |value| value.0);
            let expected_bytes = head.map_or(0, |value| value.1);
            if expected_head < 0
                || expected_bytes < 0
                || expected_bytes > mfm_journal::single_trust::MAX_CONFIGURATION_STREAM_BYTES as i64
            {
                return Err(BackendError::Capacity);
            }
            let expected_head_usize =
                usize::try_from(expected_head).map_err(|_| BackendError::Capacity)?;
            if expected_head_usize > mfm_journal::single_trust::MAX_CONFIGURATION_REVISIONS {
                return Err(BackendError::Capacity);
            }
            let metadata: (i64, i64, i64) = sqlx::query_as(
                "SELECT COUNT(*), COALESCE(SUM(octet_length(canonical_bytes)), 0),
                        COALESCE(MAX(octet_length(canonical_bytes)), 0)::BIGINT
                 FROM mfm_configuration_revisions
                 WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .fetch_one(&self.pool)
            .await
            .map_err(|_| BackendError::Storage)?;
            if metadata.0 != expected_head
                || metadata.1 != expected_bytes
                || metadata.2 < 0
                || metadata.2 > mfm_journal::single_trust::MAX_CONFIGURATION_REVISION_BYTES as i64
            {
                return Err(BackendError::Capacity);
            }
            let raw_rows: Vec<(i64, String, Vec<u8>, String, i64)> = sqlx::query_as(
                "SELECT revision_sequence, append_request_id, canonical_bytes, content_ref, total_bytes
                 FROM mfm_configuration_revisions
                 WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3
                 ORDER BY revision_sequence ASC
                 LIMIT $4",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .bind(
                i64::try_from(
                    mfm_journal::single_trust::MAX_CONFIGURATION_REVISIONS.saturating_add(1),
                )
                .map_err(|_| BackendError::Capacity)?,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|_| BackendError::Storage)?;
            if raw_rows.len() > mfm_journal::single_trust::MAX_CONFIGURATION_REVISIONS {
                return Err(BackendError::Capacity);
            }
            let mut total_bytes = 0usize;
            let rows = raw_rows
                .into_iter()
                .map(
                    |(sequence, append_request_id, bytes, content_ref, retained_total_bytes)| {
                        total_bytes = total_bytes
                            .checked_add(bytes.len())
                            .ok_or(BackendError::Capacity)?;
                        if bytes.len() > mfm_journal::single_trust::MAX_CONFIGURATION_REVISION_BYTES
                            || total_bytes
                                > mfm_journal::single_trust::MAX_CONFIGURATION_STREAM_BYTES
                        {
                            return Err(BackendError::Capacity);
                        }
                        if usize::try_from(retained_total_bytes)
                            .map_err(|_| BackendError::Storage)?
                            != total_bytes
                        {
                            return Err(BackendError::Storage);
                        }
                        RawConfigurationRevision::new(
                            u64::try_from(sequence).map_err(|_| BackendError::Storage)?,
                            AppendRequestId::new(append_request_id)
                                .map_err(|_| BackendError::Storage)?,
                            bytes,
                            serde_json::from_str(&content_ref)
                                .map_err(|_| BackendError::Storage)?,
                            usize::try_from(retained_total_bytes)
                                .map_err(|_| BackendError::Storage)?,
                        )
                    },
                )
                .collect::<BackendResult<Vec<_>>>()?;
            if expected_head_usize != rows.len()
                || expected_bytes
                    != i64::try_from(total_bytes).map_err(|_| BackendError::Capacity)?
            {
                return Err(BackendError::Storage);
            }
            Ok(rows)
        })
    }

    fn compare_and_append_configuration<'a>(
        &'a self,
        command: &'a ConfigurationAppendCommand<'a>,
    ) -> BackendFuture<'a, BackendConfigurationOutcome> {
        Box::pin(async move {
            self.verify_open_identity()
                .await
                .map_err(PostgresStore::map_identity_check)?;
            if command.identity()
                != &StructuredStoreIdentity::new(
                    self.scope.clone(),
                    self.epoch,
                    self.tenant.clone(),
                )
            {
                return Err(BackendError::Identity);
            }
            if command.canonical_bytes().is_empty()
                || command.canonical_bytes().len()
                    > mfm_journal::single_trust::MAX_CONFIGURATION_REVISION_BYTES
            {
                return Err(BackendError::Capacity);
            }
            let mut transaction = self.pool.begin().await.map_err(|_| BackendError::Storage)?;
            self.verify_transaction_identity(&mut transaction).await?;
            // Configuration has the same absent-head race as a new run.  The advisory lock is
            // transaction-scoped and only closes that gap before the ordinary head row lock.
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(format!(
                    "configuration:{}:{}:{}",
                    self.scope.as_str(),
                    self.epoch.get(),
                    self.tenant.as_str()
                ))
                .execute(&mut *transaction)
                .await
                .map_err(|_| BackendError::Storage)?;
            let epoch = i64::try_from(self.epoch.get()).map_err(|_| BackendError::Capacity)?;
            let existing: Option<(i64, String, Vec<u8>, String, i64)> = sqlx::query_as(
                "SELECT revision_sequence, append_request_id, canonical_bytes, content_ref, total_bytes
                 FROM mfm_configuration_revisions
                 WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3
                   AND append_request_id = $4",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .bind(command.append_request_id().as_str())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| BackendError::Storage)?;
            if let Some((sequence, append_request_id, bytes, content_ref, total_bytes)) = existing {
                let recorded: mfm_ids::ContentRef =
                    serde_json::from_str(&content_ref).map_err(|_| BackendError::Storage)?;
                transaction
                    .commit()
                    .await
                    .map_err(|_| BackendError::AcknowledgementUnknown)?;
                if bytes == command.canonical_bytes() && recorded == *command.content_ref() {
                    return RawConfigurationRevision::new(
                        u64::try_from(sequence).map_err(|_| BackendError::Storage)?,
                        AppendRequestId::new(append_request_id)
                            .map_err(|_| BackendError::Storage)?,
                        bytes,
                        recorded,
                        usize::try_from(total_bytes).map_err(|_| BackendError::Storage)?,
                    )
                    .map(|revision| BackendConfigurationOutcome::Found(Box::new(revision)));
                }
                return Err(BackendError::Conflict);
            }
            let current: Option<(i64, i64)> = sqlx::query_as(
                "SELECT head_sequence, total_bytes FROM mfm_configuration_heads
                 WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3
                 FOR UPDATE",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| BackendError::Storage)?;
            let (actual, total_bytes) = current.unwrap_or((0, 0));
            if command.expected_sequence()
                != u64::try_from(actual).map_err(|_| BackendError::Storage)?
            {
                transaction
                    .commit()
                    .await
                    .map_err(|_| BackendError::AcknowledgementUnknown)?;
                return Ok(BackendConfigurationOutcome::StaleHead {
                    actual_sequence: u64::try_from(actual).map_err(|_| BackendError::Storage)?,
                });
            }
            let candidate_bytes = i64::try_from(command.canonical_bytes().len())
                .map_err(|_| BackendError::Capacity)?;
            if actual as usize >= mfm_journal::single_trust::MAX_CONFIGURATION_REVISIONS
                || total_bytes
                    .checked_add(candidate_bytes)
                    .filter(|bytes| {
                        *bytes <= mfm_journal::single_trust::MAX_CONFIGURATION_STREAM_BYTES as i64
                    })
                    .is_none()
            {
                return Err(BackendError::Capacity);
            }
            let sequence = actual.checked_add(1).ok_or(BackendError::Capacity)?;
            sqlx::query(
                "INSERT INTO mfm_configuration_revisions
                 (store_scope_id, store_epoch, tenant_scope_id, revision_sequence,
                  append_request_id, canonical_bytes, content_ref, total_bytes)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .bind(sequence)
            .bind(command.append_request_id().as_str())
            .bind(command.canonical_bytes())
            .bind(serde_json::to_string(command.content_ref()).map_err(|_| BackendError::Storage)?)
            .bind(total_bytes + candidate_bytes)
            .execute(&mut *transaction)
            .await
            .map_err(|_| BackendError::Conflict)?;
            sqlx::query(
                "INSERT INTO mfm_configuration_heads
                 (store_scope_id, store_epoch, tenant_scope_id, head_sequence, total_bytes)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (store_scope_id, store_epoch, tenant_scope_id)
                 DO UPDATE SET head_sequence = EXCLUDED.head_sequence,
                               total_bytes = EXCLUDED.total_bytes",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .bind(sequence)
            .bind(total_bytes + candidate_bytes)
            .execute(&mut *transaction)
            .await
            .map_err(|_| BackendError::Storage)?;
            transaction
                .commit()
                .await
                .map_err(|_| BackendError::AcknowledgementUnknown)?;
            Ok(BackendConfigurationOutcome::NewlyCommitted)
        })
    }

    fn load_facts<'a>(&'a self) -> BackendFuture<'a, RawFactSnapshot> {
        Box::pin(async move {
            self.verify_open_identity()
                .await
                .map_err(PostgresStore::map_identity_check)?;
            let epoch = i64::try_from(self.epoch.get()).map_err(|_| BackendError::Capacity)?;
            let head: Option<i64> = sqlx::query_scalar(
                "SELECT publication_sequence FROM mfm_fact_heads
                 WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| BackendError::Storage)?;
            let rows: Vec<(i64, String, i64, String)> = sqlx::query_as(
                "SELECT publication_sequence, run_id, run_sequence, proposal_set_ref
                 FROM mfm_fact_publications
                 WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3
                 ORDER BY publication_sequence ASC
                 LIMIT $4",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .bind(
                i64::try_from(mfm_journal::single_trust::MAX_RUN_FRAMES.saturating_add(1))
                    .map_err(|_| BackendError::Capacity)?,
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|_| BackendError::Storage)?;
            let publications = rows
                .into_iter()
                .map(
                    |(publication_sequence, run_id, run_sequence, proposal_set_ref)| {
                        RawFactPublication::new(
                            u64::try_from(publication_sequence)
                                .map_err(|_| BackendError::Storage)?,
                            RunId::parse(run_id).map_err(|_| BackendError::Storage)?,
                            u64::try_from(run_sequence).map_err(|_| BackendError::Storage)?,
                            serde_json::from_str(&proposal_set_ref)
                                .map_err(|_| BackendError::Storage)?,
                        )
                    },
                )
                .collect::<BackendResult<Vec<_>>>()?;
            let head = head.unwrap_or(0);
            if head < 0
                || head != i64::try_from(publications.len()).map_err(|_| BackendError::Capacity)?
            {
                return Err(BackendError::Storage);
            }
            RawFactSnapshot::new(head as u64, publications).map_err(|_| BackendError::Storage)
        })
    }

    fn audit_run_ids<'a>(&'a self) -> BackendFuture<'a, Vec<RunId>> {
        Box::pin(async move {
            self.verify_open_identity()
                .await
                .map_err(PostgresStore::map_identity_check)?;
            let epoch = i64::try_from(self.epoch.get()).map_err(|_| BackendError::Capacity)?;
            let rows: Vec<String> = sqlx::query_scalar(
                "SELECT run_id FROM mfm_run_heads
                 WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3
                 ORDER BY run_id ASC",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(|_| BackendError::Storage)?;
            rows.into_iter()
                .map(|run_id| RunId::parse(run_id).map_err(|_| BackendError::Storage))
                .collect()
        })
    }
}

#[cfg(all(test, feature = "test-support"))]
mod managed_postgres_tests {
    use super::*;
    use mfm_canonical::raw_content_digest;
    use sqlx::postgres::PgConnectOptions;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::str::FromStr;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    async fn install_abort_trigger(pool: &PgPool, table: &str, operation: &str) {
        let create = match (table, operation) {
            ("mfm_run_frames", "INSERT") => {
                "CREATE TRIGGER mfm_test_abort AFTER INSERT ON mfm_run_frames
                 FOR EACH ROW EXECUTE FUNCTION mfm_test_abort_trigger()"
            }
            ("mfm_run_heads", "INSERT OR UPDATE") => {
                "CREATE TRIGGER mfm_test_abort AFTER INSERT OR UPDATE ON mfm_run_heads
                 FOR EACH ROW EXECUTE FUNCTION mfm_test_abort_trigger()"
            }
            ("mfm_fact_heads", "INSERT OR UPDATE") => {
                "CREATE TRIGGER mfm_test_abort AFTER INSERT OR UPDATE ON mfm_fact_heads
                 FOR EACH ROW EXECUTE FUNCTION mfm_test_abort_trigger()"
            }
            ("mfm_fact_publications", "INSERT") => {
                "CREATE TRIGGER mfm_test_abort AFTER INSERT ON mfm_fact_publications
                 FOR EACH ROW EXECUTE FUNCTION mfm_test_abort_trigger()"
            }
            ("mfm_configuration_revisions", "INSERT") => {
                "CREATE TRIGGER mfm_test_abort AFTER INSERT ON mfm_configuration_revisions
                 FOR EACH ROW EXECUTE FUNCTION mfm_test_abort_trigger()"
            }
            ("mfm_configuration_heads", "INSERT OR UPDATE") => {
                "CREATE TRIGGER mfm_test_abort AFTER INSERT OR UPDATE ON mfm_configuration_heads
                 FOR EACH ROW EXECUTE FUNCTION mfm_test_abort_trigger()"
            }
            _ => panic!("unreviewed PostgreSQL failpoint"),
        };
        sqlx::query(
            "CREATE OR REPLACE FUNCTION mfm_test_abort_trigger()
             RETURNS trigger LANGUAGE plpgsql AS $$
             BEGIN
                 RAISE EXCEPTION 'test transaction failpoint';
             END;
             $$",
        )
        .execute(pool)
        .await
        .expect("failpoint function");
        sqlx::query("DROP TRIGGER IF EXISTS mfm_test_abort ON mfm_run_frames")
            .execute(pool)
            .await
            .expect("clear frame trigger");
        sqlx::query("DROP TRIGGER IF EXISTS mfm_test_abort ON mfm_run_heads")
            .execute(pool)
            .await
            .expect("clear head trigger");
        sqlx::query("DROP TRIGGER IF EXISTS mfm_test_abort ON mfm_fact_heads")
            .execute(pool)
            .await
            .expect("clear fact head trigger");
        sqlx::query("DROP TRIGGER IF EXISTS mfm_test_abort ON mfm_fact_publications")
            .execute(pool)
            .await
            .expect("clear fact publication trigger");
        sqlx::query("DROP TRIGGER IF EXISTS mfm_test_abort ON mfm_configuration_revisions")
            .execute(pool)
            .await
            .expect("clear configuration revision trigger");
        sqlx::query("DROP TRIGGER IF EXISTS mfm_test_abort ON mfm_configuration_heads")
            .execute(pool)
            .await
            .expect("clear configuration head trigger");
        sqlx::query(create)
            .execute(pool)
            .await
            .expect("install failpoint trigger");
    }

    async fn clear_abort_triggers(pool: &PgPool) {
        for drop in [
            "DROP TRIGGER IF EXISTS mfm_test_abort ON mfm_run_frames",
            "DROP TRIGGER IF EXISTS mfm_test_abort ON mfm_run_heads",
            "DROP TRIGGER IF EXISTS mfm_test_abort ON mfm_fact_heads",
            "DROP TRIGGER IF EXISTS mfm_test_abort ON mfm_fact_publications",
            "DROP TRIGGER IF EXISTS mfm_test_abort ON mfm_configuration_revisions",
            "DROP TRIGGER IF EXISTS mfm_test_abort ON mfm_configuration_heads",
        ] {
            sqlx::query(drop)
                .execute(pool)
                .await
                .expect("clear failpoint trigger");
        }
        sqlx::query("DROP FUNCTION IF EXISTS mfm_test_abort_trigger()")
            .execute(pool)
            .await
            .expect("clear failpoint function");
    }

    #[cfg(target_os = "linux")]
    struct IsolatedPrimary {
        pool: PgPool,
        store: Arc<PostgresStore>,
        data_directory: PathBuf,
        pg_ctl: PathBuf,
    }

    #[cfg(target_os = "linux")]
    async fn start_isolated_primary(
        managed_pool: &PgPool,
        identity: &StructuredStoreIdentity,
    ) -> IsolatedPrimary {
        let managed_data_directory: String =
            sqlx::query_scalar("SELECT setting FROM pg_settings WHERE name = 'data_directory'")
                .fetch_one(managed_pool)
                .await
                .expect("managed data directory");
        let managed_pid =
            fs::read_to_string(PathBuf::from(&managed_data_directory).join("postmaster.pid"))
                .expect("managed postmaster pid file")
                .lines()
                .next()
                .and_then(|line| line.parse::<i32>().ok())
                .expect("managed postmaster pid");
        let postgres_executable = fs::read_link(format!("/proc/{managed_pid}/exe"))
            .expect("managed postmaster executable");
        let binary_directory = postgres_executable
            .parent()
            .expect("postgres executable directory")
            .to_owned();
        let pg_ctl = binary_directory.join("pg_ctl");
        let initdb = binary_directory.join("initdb");
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let data_directory = std::env::temp_dir().join(format!(
            "mfm-primary-restart-{}-{stamp}",
            std::process::id()
        ));
        let setup_pg_ctl = pg_ctl.clone();
        let (data_directory, port) = tokio::task::spawn_blocking({
            let data_directory = data_directory.clone();
            move || {
                fs::create_dir(&data_directory).expect("isolated primary directory");
                let init = Command::new(&initdb)
                    .args([
                        "-D",
                        data_directory
                            .to_str()
                            .expect("isolated data directory path"),
                        "-U",
                        "postgres",
                        "--auth=trust",
                    ])
                    .output()
                    .expect("initialize isolated primary");
                assert!(init.status.success(), "initialize isolated primary failed");
                let listener =
                    std::net::TcpListener::bind("127.0.0.1:0").expect("isolated primary port");
                let port = listener
                    .local_addr()
                    .expect("isolated primary address")
                    .port();
                drop(listener);
                let options = format!(
                    "-c fsync=on -c full_page_writes=on -c synchronous_commit=on \
                     -c unix_socket_directories= -h 127.0.0.1 -p {port}"
                );
                let log_path = data_directory.join("restart.log");
                let start = Command::new(&setup_pg_ctl)
                    .args([
                        "-D",
                        data_directory
                            .to_str()
                            .expect("isolated data directory path"),
                        "-o",
                        &options,
                        "-l",
                        log_path.to_str().expect("isolated restart log path"),
                        "-w",
                        "start",
                    ])
                    .output()
                    .expect("start isolated primary");
                assert!(start.status.success(), "start isolated primary failed");
                Ok::<_, ()>((data_directory, port))
            }
        })
        .await
        .expect("isolated primary worker")
        .expect("isolated primary setup");
        let database_url = format!("postgresql://postgres@127.0.0.1:{port}/postgres");
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .expect("isolated primary connection");
        PostgresStore::migrate(&pool, identity.scope(), identity.epoch())
            .await
            .expect("isolated primary schema");
        let store = Arc::new(
            PostgresStore::from_pool(
                pool.clone(),
                identity.scope().clone(),
                identity.epoch(),
                identity.tenant().clone(),
                DurabilityProfile::PrimaryCrashRestart,
            )
            .expect("isolated primary store"),
        );
        store
            .check_ready()
            .await
            .expect("isolated primary durability");
        IsolatedPrimary {
            pool,
            store,
            data_directory,
            pg_ctl,
        }
    }

    #[cfg(target_os = "linux")]
    async fn crash_and_restart_primary(primary: &IsolatedPrimary) {
        let port: String = sqlx::query_scalar("SELECT current_setting('port')")
            .fetch_one(&primary.pool)
            .await
            .expect("isolated primary port");
        let pid_path = primary.data_directory.join("postmaster.pid");
        let postmaster_pid = fs::read_to_string(&pid_path)
            .expect("isolated postmaster pid file")
            .lines()
            .next()
            .and_then(|line| line.parse::<i32>().ok())
            .expect("isolated postmaster pid");
        assert!(Command::new("kill")
            .args(["-KILL", &postmaster_pid.to_string()])
            .status()
            .expect("kill isolated primary")
            .success());

        let restart_data_directory = primary.data_directory.clone();
        let restart_pg_ctl = primary.pg_ctl.clone();
        let restart_port = port.clone();
        tokio::task::spawn_blocking(move || {
            for _ in 0..100 {
                let data_directory_text = restart_data_directory.to_string_lossy();
                let still_running = fs::read_dir("/proc")
                    .ok()
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| entry.ok())
                    .filter_map(|entry| entry.file_name().into_string().ok())
                    .filter(|name| name.bytes().all(|byte| byte.is_ascii_digit()))
                    .any(|pid| {
                        fs::read_to_string(format!("/proc/{pid}/cmdline"))
                            .map(|command| {
                                command.contains("postgres")
                                    && command.contains(data_directory_text.as_ref())
                            })
                            .unwrap_or(false)
                    });
                if !still_running {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            if fs::read_to_string(restart_data_directory.join("postmaster.pid"))
                .ok()
                .and_then(|contents| contents.lines().next().map(str::to_owned))
                .and_then(|pid| pid.parse::<i32>().ok())
                == Some(postmaster_pid)
            {
                fs::remove_file(restart_data_directory.join("postmaster.pid"))
                    .expect("remove crashed isolated pid file");
            }
            let options = format!(
                "-c fsync=on -c full_page_writes=on -c synchronous_commit=on \
                 -c unix_socket_directories= -h 127.0.0.1 -p {restart_port}"
            );
            let log_path = restart_data_directory.join("restart.log");
            let output = Command::new(&restart_pg_ctl)
                .args([
                    "-D",
                    restart_data_directory
                        .to_str()
                        .expect("isolated data directory path"),
                    "-o",
                    &options,
                    "-l",
                    log_path.to_str().expect("isolated restart log path"),
                    "-w",
                    "start",
                ])
                .output()
                .expect("restart isolated primary");
            assert!(output.status.success(), "restart isolated primary failed");
        })
        .await
        .expect("isolated primary restart worker");

        let mut ready = false;
        for _ in 0..100 {
            if sqlx::query_scalar::<_, i32>("SELECT 1")
                .fetch_one(&primary.pool)
                .await
                .is_ok()
            {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(ready, "restarted isolated primary did not become ready");
    }

    #[cfg(target_os = "linux")]
    async fn stop_isolated_primary(primary: IsolatedPrimary) {
        primary.pool.close().await;
        let data_directory = primary.data_directory;
        let pg_ctl = primary.pg_ctl;
        tokio::task::spawn_blocking(move || {
            let stopped = Command::new(&pg_ctl)
                .args([
                    "-D",
                    data_directory
                        .to_str()
                        .expect("isolated data directory path"),
                    "-m",
                    "immediate",
                    "-w",
                    "stop",
                ])
                .output()
                .expect("stop isolated primary");
            assert!(stopped.status.success(), "stop isolated primary failed");
            fs::remove_dir_all(data_directory).expect("remove isolated primary directory");
        })
        .await
        .expect("isolated primary shutdown worker");
    }

    fn process_race_identity() -> StructuredStoreIdentity {
        StructuredStoreIdentity::new(
            StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("scope"),
            StoreEpoch::new(1),
            TenantScopeId::new("mfm.tenant_scope.v1:0123456789abcdef0123456789abcdef")
                .expect("tenant"),
        )
    }

    async fn run_process_cas_child(stage: mfm_store::backend_conformance::ProcessRaceStage) {
        let database_url = std::env::var("DATABASE_URL").expect("child database URL");
        let identity = process_race_identity();
        let candidate = std::env::var("MFM_POSTGRES_CAS_CANDIDATE").expect("child CAS candidate");
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .expect("child PostgreSQL service");
        let store = Arc::new(
            PostgresStore::from_pool(
                pool,
                identity.scope().clone(),
                identity.epoch(),
                identity.tenant().clone(),
                DurabilityProfile::PrimaryCrashRestart,
            )
            .expect("child store"),
        );
        store.check_ready().await.expect("child store readiness");
        let backend: Arc<dyn StructuredStoreBackend> = store;
        let (sequence, previous_head) =
            mfm_store::backend_conformance::process_race_candidate_head(
                Arc::clone(&backend),
                stage,
            )
            .await
            .expect("child process CAS head");
        await_process_race_barrier(&candidate).await;
        let outcome = mfm_store::backend_conformance::append_process_race_candidate_at_head(
            backend,
            identity,
            stage,
            &candidate,
            sequence,
            previous_head,
        )
        .await
        .expect("child process CAS append");
        let outcome_name = match &outcome {
            BackendAppendOutcome::NewlyCommitted => "newly-committed",
            BackendAppendOutcome::StaleHead { .. } => "stale-head",
            BackendAppendOutcome::Found(_) => "found",
            BackendAppendOutcome::AcknowledgementUnknown => "acknowledgement-unknown",
        };
        let barrier = std::env::var_os("MFM_POSTGRES_CAS_BARRIER")
            .map(PathBuf::from)
            .expect("child CAS barrier");
        fs::write(barrier.join(format!("{candidate}.outcome")), outcome_name)
            .expect("record child CAS outcome");
        assert!(matches!(
            outcome,
            BackendAppendOutcome::NewlyCommitted | BackendAppendOutcome::StaleHead { .. }
        ));
    }

    async fn await_process_race_barrier(candidate: &str) {
        assert!(matches!(candidate, "left" | "right"));
        let barrier = std::env::var_os("MFM_POSTGRES_CAS_BARRIER")
            .map(PathBuf::from)
            .expect("child CAS barrier");
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(barrier.join(format!("{candidate}.ready")))
            .expect("create child CAS barrier marker");
        let other = if candidate == "left" { "right" } else { "left" };
        for _ in 0..400 {
            if barrier.join(format!("{other}.ready")).is_file() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("process CAS barrier did not release both children");
    }

    #[tokio::test]
    async fn managed_primary_schema_meets_the_admitted_profile() {
        let Ok(database_url) = std::env::var("DATABASE_URL") else {
            return;
        };
        if std::env::var_os("MFM_POSTGRES_CAS_CHILD").is_some() {
            let stage = std::env::var("MFM_POSTGRES_CAS_STAGE")
                .ok()
                .and_then(|value| mfm_store::backend_conformance::ProcessRaceStage::parse(&value))
                .expect("child CAS stage");
            run_process_cas_child(stage).await;
            return;
        }
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .expect("managed PostgreSQL service");
        let identity = process_race_identity();
        sqlx::query(
            "DROP TABLE IF EXISTS mfm_run_frames, mfm_run_heads, mfm_fact_heads,
             mfm_fact_publications, mfm_configuration_revisions, mfm_configuration_heads,
             mfm_store_schema CASCADE",
        )
        .execute(&pool)
        .await
        .expect("isolated PostgreSQL baseline");
        PostgresStore::migrate(&pool, identity.scope(), identity.epoch())
            .await
            .expect("fresh PostgreSQL baseline");
        sqlx::query(
            "TRUNCATE mfm_run_frames, mfm_run_heads, mfm_fact_heads,
             mfm_fact_publications, mfm_configuration_revisions, mfm_configuration_heads",
        )
        .execute(&pool)
        .await
        .expect("isolated conformance schema");
        let store = Arc::new(
            PostgresStore::from_pool(
                pool.clone(),
                identity.scope().clone(),
                identity.epoch(),
                identity.tenant().clone(),
                DurabilityProfile::PrimaryCrashRestart,
            )
            .expect("store"),
        );
        store
            .check_ready()
            .await
            .expect("admitted PostgreSQL durability");
        let backend: Arc<dyn StructuredStoreBackend> = store.clone();
        mfm_store::backend_conformance::exercise(backend.clone(), identity.clone())
            .await
            .expect("PostgreSQL backend contract");
        let executable = std::env::current_exe().expect("PostgreSQL test executable");
        let child_database_url = database_url.clone();
        for stage in mfm_store::backend_conformance::ProcessRaceStage::ALL {
            mfm_store::backend_conformance::seed_process_race(
                backend.clone(),
                identity.clone(),
                stage,
            )
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "seed PostgreSQL process CAS stage {}: {error:?}",
                    stage.as_str()
                )
            });
            let barrier = std::env::temp_dir().join(format!(
                "mfm-postgres-cas-{}-{}-{}",
                std::process::id(),
                stage.as_str(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("CAS barrier clock")
                    .as_nanos()
            ));
            fs::create_dir(&barrier).expect("create PostgreSQL CAS barrier");
            let executable = executable.clone();
            let child_database_url = child_database_url.clone();
            let child_barrier = barrier.clone();
            let stage_name = stage.as_str();
            tokio::task::spawn_blocking(move || {
                let mut children = ["left", "right"]
                    .into_iter()
                    .map(|candidate| {
                        Command::new(&executable)
                            .args([
                                "--exact",
                                "managed_postgres_tests::managed_primary_schema_meets_the_admitted_profile",
                                "--nocapture",
                            ])
                            .env("DATABASE_URL", &child_database_url)
                            .env("MFM_POSTGRES_CAS_CHILD", "1")
                            .env("MFM_POSTGRES_CAS_STAGE", stage_name)
                            .env("MFM_POSTGRES_CAS_CANDIDATE", candidate)
                            .env("MFM_POSTGRES_CAS_BARRIER", &child_barrier)
                            .spawn()
                            .expect("spawn PostgreSQL CAS child")
                    })
                    .collect::<Vec<_>>();
                for child in &mut children {
                    assert!(child
                        .wait()
                        .expect("wait for PostgreSQL CAS child")
                        .success());
                }
            })
            .await
            .expect("PostgreSQL process CAS worker");
            let outcomes = ["left", "right"]
                .into_iter()
                .map(|candidate| {
                    fs::read_to_string(barrier.join(format!("{candidate}.outcome")))
                        .expect("read PostgreSQL CAS child outcome")
                })
                .collect::<Vec<_>>();
            fs::remove_dir_all(&barrier).expect("remove PostgreSQL CAS barrier");
            assert_eq!(
                outcomes
                    .iter()
                    .filter(|outcome| outcome.as_str() == "newly-committed")
                    .count(),
                1,
                "process CAS stage {} outcomes: {outcomes:?}",
                stage.as_str()
            );
            assert_eq!(
                outcomes
                    .iter()
                    .filter(|outcome| outcome.as_str() == "stale-head")
                    .count(),
                1,
                "process CAS stage {} outcomes: {outcomes:?}",
                stage.as_str()
            );
            mfm_store::backend_conformance::verify_process_race(backend.clone(), stage)
                .await
                .unwrap_or_else(|error| {
                    panic!(
                        "verify PostgreSQL process CAS stage {}: {error:?}",
                        stage.as_str()
                    )
                });
        }

        let process_race_run = RunId::parse(
            "run:sha256-jcs-v1:c123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("process race run");
        let process_prefix = backend
            .load_complete_prefix(
                &process_race_run,
                RawHistoryLoadLimit::new(2, mfm_journal::single_trust::MAX_FRAME_BYTES * 2),
            )
            .await
            .expect("load process CAS prefix")
            .expect("process CAS prefix");
        assert_eq!(process_prefix.frames().len(), 1);
        let recorded_process_head = process_prefix.frames()[0].head_digest().as_str().to_owned();
        let corrupt_process_head =
            "content:sha256-v1:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
        sqlx::query(
            "UPDATE mfm_run_heads
             SET head_digest = $1
             WHERE store_scope_id = $2 AND store_epoch = $3 AND tenant_scope_id = $4 AND run_id = $5",
        )
        .bind(corrupt_process_head)
        .bind(identity.scope().as_str())
        .bind(i64::try_from(identity.epoch().get()).expect("epoch"))
        .bind(identity.tenant().as_str())
        .bind(process_race_run.as_str())
        .execute(&pool)
        .await
        .expect("corrupt process CAS head");
        assert!(matches!(
            backend
                .load_complete_prefix(
                    &process_race_run,
                    RawHistoryLoadLimit::new(2, mfm_journal::single_trust::MAX_FRAME_BYTES * 2),
                )
                .await,
            Err(BackendError::Storage)
        ));
        sqlx::query(
            "UPDATE mfm_run_heads
             SET head_digest = $1
             WHERE store_scope_id = $2 AND store_epoch = $3 AND tenant_scope_id = $4 AND run_id = $5",
        )
        .bind(&recorded_process_head)
        .bind(identity.scope().as_str())
        .bind(i64::try_from(identity.epoch().get()).expect("epoch"))
        .bind(identity.tenant().as_str())
        .bind(process_race_run.as_str())
        .execute(&pool)
        .await
        .expect("restore process CAS head");

        #[cfg(target_os = "linux")]
        {
            let primary = start_isolated_primary(&pool, &identity).await;
            let primary_backend: Arc<dyn StructuredStoreBackend> = primary.store.clone();
            let restart_run = mfm_store::backend_conformance::append_primary_restart_probe(
                primary_backend.clone(),
                identity.clone(),
            )
            .await
            .expect("primary restart probe append");
            mfm_store::backend_conformance::verify_primary_restart_probe(
                primary.store.clone(),
                restart_run.clone(),
            )
            .await
            .expect("primary restart probe before crash");
            crash_and_restart_primary(&primary).await;
            primary
                .store
                .check_ready()
                .await
                .expect("restarted PostgreSQL durability");
            mfm_store::backend_conformance::verify_primary_restart_probe(
                primary_backend,
                restart_run,
            )
            .await
            .expect("primary restart probe recovery");
            stop_isolated_primary(primary).await;
        }

        for (table, operation) in [
            ("mfm_run_frames", "INSERT"),
            ("mfm_run_heads", "INSERT OR UPDATE"),
        ] {
            install_abort_trigger(&pool, table, operation).await;
            mfm_store::backend_conformance::exercise_atomic_rollback(
                backend.clone(),
                identity.clone(),
                mfm_store::backend_conformance::AtomicRollbackProbe::History,
            )
            .await
            .expect("history rollback");
            clear_abort_triggers(&pool).await;
        }
        for (table, operation) in [
            ("mfm_fact_heads", "INSERT OR UPDATE"),
            ("mfm_fact_publications", "INSERT"),
        ] {
            install_abort_trigger(&pool, table, operation).await;
            mfm_store::backend_conformance::exercise_atomic_rollback(
                backend.clone(),
                identity.clone(),
                mfm_store::backend_conformance::AtomicRollbackProbe::FactPublication,
            )
            .await
            .expect("fact rollback");
            clear_abort_triggers(&pool).await;
        }
        mfm_store::backend_conformance::exercise_first_fact_publication_race(
            backend.clone(),
            identity.clone(),
        )
        .await
        .expect("PostgreSQL first fact publication race");
        for (table, operation) in [
            ("mfm_configuration_revisions", "INSERT"),
            ("mfm_configuration_heads", "INSERT OR UPDATE"),
        ] {
            install_abort_trigger(&pool, table, operation).await;
            mfm_store::backend_conformance::exercise_atomic_rollback(
                backend.clone(),
                identity.clone(),
                mfm_store::backend_conformance::AtomicRollbackProbe::Configuration,
            )
            .await
            .expect("configuration rollback");
            clear_abort_triggers(&pool).await;
        }

        // A killed client connection must leave its uncommitted frame absent after reconnect.
        let first_bytes = br#"{"kind":"connection-loss"}"#;
        let first_digest = raw_content_digest(first_bytes);
        let first_head = ContentDigest::parse(
            "content:sha256-v1:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        )
        .expect("connection-loss head");
        let mut target = pool.acquire().await.expect("target connection");
        sqlx::query("BEGIN")
            .execute(&mut *target)
            .await
            .expect("begin target transaction");
        let lost_run_id = RunId::parse(
            "run:sha256-jcs-v1:6123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("lost run");
        sqlx::query(
            "INSERT INTO mfm_run_frames
             (store_scope_id, store_epoch, tenant_scope_id, run_id, run_sequence,
              append_request_id, frame_bytes, frame_digest, head_digest)
             VALUES ($1, $2, $3, $4, 1, $5, $6, $7, $8)",
        )
        .bind(identity.scope().as_str())
        .bind(i64::try_from(identity.epoch().get()).expect("epoch"))
        .bind(identity.tenant().as_str())
        .bind(lost_run_id.as_str())
        .bind("postgres-connection-loss-0123456789")
        .bind(first_bytes)
        .bind(first_digest.as_str())
        .bind(first_head.as_str())
        .execute(&mut *target)
        .await
        .expect("uncommitted frame");
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *target)
            .await
            .expect("target pid");
        let killed: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
            .bind(pid)
            .fetch_one(&pool)
            .await
            .expect("terminate target");
        assert!(killed);
        assert!(sqlx::query("COMMIT").execute(&mut *target).await.is_err());
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM mfm_run_frames WHERE run_id = $1")
                .bind(lost_run_id.as_str())
                .fetch_one(&pool)
                .await
                .expect("reconnected count");
        assert_eq!(remaining, 0);
        clear_abort_triggers(&pool).await;

        let source_database: String = sqlx::query_scalar("SELECT current_database()")
            .fetch_one(&pool)
            .await
            .expect("source database name");
        drop(target);
        drop(backend);
        drop(store);
        pool.close().await;

        let admin_options = PgConnectOptions::from_str(&database_url)
            .expect("parse managed database URL")
            .database("template1");
        let admin_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(admin_options)
            .await
            .expect("managed PostgreSQL administrator");
        let clone_database = format!(
            "mfm_restore_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("restore clock")
                .as_nanos()
        );
        let quote_identifier = |value: &str| format!("\"{}\"", value.replace('"', "\"\""));
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE DATABASE {} TEMPLATE {}",
            quote_identifier(&clone_database),
            quote_identifier(&source_database),
        )))
        .execute(&admin_pool)
        .await
        .expect("copy database for restore");

        let clone_options = PgConnectOptions::from_str(&database_url)
            .expect("parse clone database URL")
            .database(&clone_database);
        let clone_pool = PgPoolOptions::new()
            .max_connections(2)
            .connect_with(clone_options)
            .await
            .expect("connect copied database");
        let old_clone = PostgresStore::from_pool(
            clone_pool.clone(),
            identity.scope().clone(),
            identity.epoch(),
            identity.tenant().clone(),
            DurabilityProfile::PrimaryCrashRestart,
        )
        .expect("old copied identity");
        old_clone
            .check_ready()
            .await
            .expect("copied database retains old identity");
        let restored_scope =
            StoreScopeId::new("mfm.store_scope.v1:abcdefabcdefabcdefabcdefabcdefab")
                .expect("restored scope");
        let restored_epoch = StoreEpoch::new(2);
        PostgresStore::rotate_identity(
            &clone_pool,
            identity.scope(),
            identity.epoch(),
            &restored_scope,
            restored_epoch,
        )
        .await
        .expect("rotate copied database identity");
        assert_eq!(old_clone.check_ready().await, Err(PostgresError::Identity));
        let restored = PostgresStore::from_pool(
            clone_pool.clone(),
            restored_scope.clone(),
            restored_epoch,
            identity.tenant().clone(),
            DurabilityProfile::PrimaryCrashRestart,
        )
        .expect("restored store");
        restored.check_ready().await.expect("restored readiness");
        let restored_backend: Arc<dyn StructuredStoreBackend> = Arc::new(restored);
        assert!(
            restored_backend
                .load_complete_prefix(
                    &process_race_run,
                    RawHistoryLoadLimit::new(2, mfm_journal::single_trust::MAX_FRAME_BYTES * 2),
                )
                .await
                .expect("new identity lookup")
                .is_none(),
            "old run partitions are not resumable under the fresh identity"
        );
        let restored_identity =
            StructuredStoreIdentity::new(restored_scope, restored_epoch, identity.tenant().clone());
        let restored_run = mfm_store::backend_conformance::append_primary_restart_probe(
            restored_backend.clone(),
            restored_identity.clone(),
        )
        .await
        .expect("append after restore");
        mfm_store::backend_conformance::verify_primary_restart_probe(
            restored_backend,
            restored_run,
        )
        .await
        .expect("verify append after restore");
        let second_restored = PostgresStore::from_pool(
            clone_pool.clone(),
            restored_identity.scope().clone(),
            restored_identity.epoch(),
            restored_identity.tenant().clone(),
            DurabilityProfile::PrimaryCrashRestart,
        )
        .expect("second restored open");
        second_restored
            .check_ready()
            .await
            .expect("same restored identity can open twice");
        clone_pool.close().await;
        let reopened_options = PgConnectOptions::from_str(&database_url)
            .expect("parse reopened clone URL")
            .database(&clone_database);
        let reopened_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(reopened_options)
            .await
            .expect("reconnect copied database");
        let reopened = PostgresStore::from_pool(
            reopened_pool.clone(),
            restored_identity.scope().clone(),
            restored_identity.epoch(),
            restored_identity.tenant().clone(),
            DurabilityProfile::PrimaryCrashRestart,
        )
        .expect("reopened restored store");
        reopened
            .check_ready()
            .await
            .expect("rotated identity survives reconnect");
        let reused = PostgresStore::from_pool(
            reopened_pool.clone(),
            identity.scope().clone(),
            identity.epoch(),
            identity.tenant().clone(),
            DurabilityProfile::PrimaryCrashRestart,
        )
        .expect("old identity construction");
        assert_eq!(
            reused.check_ready().await,
            Err(PostgresError::Identity),
            "a restored deployment cannot reuse the old scope/epoch"
        );
        reopened_pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE {}",
            quote_identifier(&clone_database)
        )))
        .execute(&admin_pool)
        .await
        .expect("remove copied restore database");
        admin_pool.close().await;
    }
}
