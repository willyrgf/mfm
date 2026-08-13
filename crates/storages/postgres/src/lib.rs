#![warn(missing_docs)]
//! Durable PostgreSQL storage for the append-only MFM frame stream.
//!
//! This crate owns only physical ordering, idempotency, and the admitted durability profile. It
//! stores one canonical frame per append and delegates semantic qualification and reduction to
//! `mfm-store`.

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{AppendRequestId, ContentDigest, RunId, StoreEpoch, StoreScopeId, TenantScopeId};
use mfm_journal::single_trust::{RunFrame, RunRecord};
use mfm_store::{
    AppendDisposition, ConfigurationAppendDisposition, ConfigurationHistory, ConfigurationRevision,
    PreparedConfigurationAppend, QualifiedRun, RunStore,
};
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, Transaction};

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

    /// Applies the destructive fresh baseline to a caller-selected database.
    pub async fn migrate(pool: &PgPool) -> Result<()> {
        for statement in include_str!("../migrations/0001_single_trust.sql").split(';') {
            let statement = statement.trim();
            if !statement.is_empty() {
                sqlx::query(statement)
                    .execute(pool)
                    .await
                    .map_err(|_| PostgresError::Storage)?;
            }
        }
        Ok(())
    }

    /// Checks only schema, identity, and admitted primary durability; it never enumerates runs.
    pub async fn check_ready(&self) -> Result<()> {
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

    /// Commits one Store-prepared configuration successor through the durable exact-head path.
    pub async fn append_configuration(
        &self,
        prepared: &PreparedConfigurationAppend,
    ) -> Result<ConfigurationAppendDisposition> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| PostgresError::Storage)?;
        let epoch = i64::try_from(self.epoch.get()).map_err(|_| PostgresError::Capacity)?;
        let existing: Option<(i64, Vec<u8>, String)> = sqlx::query_as(
            "SELECT revision_sequence, canonical_bytes, content_ref
             FROM mfm_configuration_revisions
             WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3
               AND append_request_id = $4",
        )
        .bind(self.scope.as_str())
        .bind(epoch)
        .bind(self.tenant.as_str())
        .bind(prepared.append_request_id().as_str())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| PostgresError::Storage)?;
        if let Some((sequence, bytes, recorded_ref)) = existing {
            let recorded_ref: mfm_ids::ContentRef =
                serde_json::from_str(&recorded_ref).map_err(|_| PostgresError::InvalidRecord)?;
            if bytes == prepared.revision().canonical_json().as_bytes()
                && recorded_ref == *prepared.revision().content_ref()
            {
                transaction
                    .commit()
                    .await
                    .map_err(|_| PostgresError::AcknowledgementUnknown)?;
                return Ok(ConfigurationAppendDisposition::Found {
                    sequence: u64::try_from(sequence).map_err(|_| PostgresError::InvalidRecord)?,
                });
            }
            return Err(PostgresError::Conflict);
        }

        let current: Option<i64> = sqlx::query_scalar(
            "SELECT head_sequence FROM mfm_configuration_heads
             WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3
             FOR UPDATE",
        )
        .bind(self.scope.as_str())
        .bind(epoch)
        .bind(self.tenant.as_str())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| PostgresError::Storage)?;
        let actual = current.unwrap_or(0);
        if prepared.expected_sequence()
            != u64::try_from(actual).map_err(|_| PostgresError::InvalidRecord)?
        {
            transaction
                .commit()
                .await
                .map_err(|_| PostgresError::AcknowledgementUnknown)?;
            return Ok(ConfigurationAppendDisposition::StaleHead {
                actual_sequence: u64::try_from(actual).map_err(|_| PostgresError::InvalidRecord)?,
            });
        }
        let sequence = actual.checked_add(1).ok_or(PostgresError::Capacity)?;
        let content_ref = serde_json::to_string(prepared.revision().content_ref())
            .map_err(|_| PostgresError::InvalidRecord)?;
        sqlx::query(
            "INSERT INTO mfm_configuration_revisions
             (store_scope_id, store_epoch, tenant_scope_id, revision_sequence,
              append_request_id, canonical_bytes, content_ref)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(self.scope.as_str())
        .bind(epoch)
        .bind(self.tenant.as_str())
        .bind(sequence)
        .bind(prepared.append_request_id().as_str())
        .bind(prepared.revision().canonical_json().as_bytes())
        .bind(content_ref)
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresError::Conflict)?;
        sqlx::query(
            "INSERT INTO mfm_configuration_heads
             (store_scope_id, store_epoch, tenant_scope_id, head_sequence)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (store_scope_id, store_epoch, tenant_scope_id)
             DO UPDATE SET head_sequence = EXCLUDED.head_sequence",
        )
        .bind(self.scope.as_str())
        .bind(epoch)
        .bind(self.tenant.as_str())
        .bind(sequence)
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresError::Storage)?;
        transaction
            .commit()
            .await
            .map_err(|_| PostgresError::AcknowledgementUnknown)?;
        Ok(ConfigurationAppendDisposition::NewlyCommitted {
            sequence: u64::try_from(sequence).map_err(|_| PostgresError::InvalidRecord)?,
        })
    }

    /// Loads and strictly qualifies the complete selected configuration stream.
    pub async fn load_configuration(&self) -> Result<ConfigurationHistory> {
        let head: Option<i64> = sqlx::query_scalar(
            "SELECT head_sequence FROM mfm_configuration_heads
             WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3",
        )
        .bind(self.scope.as_str())
        .bind(i64::try_from(self.epoch.get()).map_err(|_| PostgresError::Capacity)?)
        .bind(self.tenant.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| PostgresError::Storage)?;
        let rows: Vec<(i64, String, Vec<u8>, String)> = sqlx::query_as(
            "SELECT revision_sequence, append_request_id, canonical_bytes, content_ref
             FROM mfm_configuration_revisions
             WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3
             ORDER BY revision_sequence ASC",
        )
        .bind(self.scope.as_str())
        .bind(i64::try_from(self.epoch.get()).map_err(|_| PostgresError::Capacity)?)
        .bind(self.tenant.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|_| PostgresError::Storage)?;
        if head.unwrap_or(0) != i64::try_from(rows.len()).map_err(|_| PostgresError::Capacity)? {
            return Err(PostgresError::InvalidRecord);
        }
        let revisions = rows
            .into_iter()
            .map(|(sequence, append_request_id, bytes, content_ref)| {
                let canonical = std::str::from_utf8(&bytes)
                    .map_err(|_| PostgresError::InvalidRecord)?
                    .to_owned();
                let request = AppendRequestId::new(append_request_id)
                    .map_err(|_| PostgresError::InvalidRecord)?;
                let content_ref: mfm_ids::ContentRef =
                    serde_json::from_str(&content_ref).map_err(|_| PostgresError::InvalidRecord)?;
                ConfigurationRevision::from_parts(
                    u64::try_from(sequence).map_err(|_| PostgresError::InvalidRecord)?,
                    request,
                    canonical,
                    content_ref,
                )
                .map_err(|_| PostgresError::InvalidRecord)
            })
            .collect::<Result<Vec<_>>>()?;
        ConfigurationHistory::from_revisions(revisions).map_err(|error| match error {
            mfm_store::StoreError::Capacity => PostgresError::Capacity,
            _ => PostgresError::InvalidRecord,
        })
    }

    /// Loads one complete canonical prefix and lets Store qualify it once.
    pub async fn load(&self, run_id: &RunId) -> Result<QualifiedRun> {
        let rows: Vec<(Vec<u8>, String)> = sqlx::query_as(
            "SELECT frame_bytes, frame_digest FROM mfm_run_frames
             WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3 AND run_id = $4
             ORDER BY run_sequence ASC",
        )
        .bind(self.scope.as_str())
        .bind(i64::try_from(self.epoch.get()).map_err(|_| PostgresError::Capacity)?)
        .bind(self.tenant.as_str())
        .bind(run_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|_| PostgresError::Storage)?;
        if rows.is_empty() {
            return Err(PostgresError::NotFound);
        }
        let head: Option<(i64, String)> = sqlx::query_as(
            "SELECT head_sequence, head_digest FROM mfm_run_heads
             WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3 AND run_id = $4",
        )
        .bind(self.scope.as_str())
        .bind(i64::try_from(self.epoch.get()).map_err(|_| PostgresError::Capacity)?)
        .bind(self.tenant.as_str())
        .bind(run_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| PostgresError::Storage)?;
        if head.as_ref().map(|value| value.0)
            != Some(i64::try_from(rows.len()).map_err(|_| PostgresError::Capacity)?)
        {
            return Err(PostgresError::InvalidRecord);
        }
        let frames = rows
            .into_iter()
            .map(|(bytes, recorded_digest)| {
                verify_frame_digest(&bytes, &recorded_digest)?;
                decode_frame(&bytes)
            })
            .collect::<Result<Vec<_>>>()?;
        let fact_head: Option<i64> = sqlx::query_scalar(
            "SELECT publication_sequence FROM mfm_fact_heads
             WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3",
        )
        .bind(self.scope.as_str())
        .bind(i64::try_from(self.epoch.get()).map_err(|_| PostgresError::Capacity)?)
        .bind(self.tenant.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| PostgresError::Storage)?;
        let fact_stats: (i64, Option<i64>, Option<i64>) = sqlx::query_as(
            "SELECT COUNT(*), MIN(publication_sequence), MAX(publication_sequence)
             FROM mfm_fact_publications
             WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3",
        )
        .bind(self.scope.as_str())
        .bind(i64::try_from(self.epoch.get()).map_err(|_| PostgresError::Capacity)?)
        .bind(self.tenant.as_str())
        .fetch_one(&self.pool)
        .await
        .map_err(|_| PostgresError::Storage)?;
        let expected_fact_head = fact_head.unwrap_or(0);
        if fact_stats.0 != expected_fact_head
            || (expected_fact_head == 0 && (fact_stats.1.is_some() || fact_stats.2.is_some()))
            || (expected_fact_head > 0
                && (fact_stats.1 != Some(1) || fact_stats.2 != Some(expected_fact_head)))
        {
            return Err(PostgresError::InvalidRecord);
        }
        let publication_rows: Vec<(i64, i64, String)> = sqlx::query_as(
            "SELECT publication_sequence, run_sequence, selection_ref
             FROM mfm_fact_publications
             WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3
               AND run_id = $4",
        )
        .bind(self.scope.as_str())
        .bind(i64::try_from(self.epoch.get()).map_err(|_| PostgresError::Capacity)?)
        .bind(self.tenant.as_str())
        .bind(run_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|_| PostgresError::Storage)?;
        for frame in &frames {
            let Some(publication) = frame.record().fact_publication() else {
                continue;
            };
            let sequence = i64::try_from(publication.publication_sequence())
                .map_err(|_| PostgresError::Capacity)?;
            let Some(row) = publication_rows.iter().find(|row| row.0 == sequence) else {
                return Err(PostgresError::InvalidRecord);
            };
            if row.1
                != i64::try_from(frame.expected_sequence()).map_err(|_| PostgresError::Capacity)?
                || serde_json::from_str::<mfm_ids::ContentRef>(&row.2)
                    .map_err(|_| PostgresError::InvalidRecord)?
                    != *publication.selection().value_ref()
            {
                return Err(PostgresError::InvalidRecord);
            }
        }
        if publication_rows.iter().any(|row| {
            !frames.iter().any(|frame| {
                frame.expected_sequence() == u64::try_from(row.1).unwrap_or(u64::MAX)
                    && frame
                        .record()
                        .fact_publication()
                        .is_some_and(|publication| {
                            publication.publication_sequence()
                                == u64::try_from(row.0).unwrap_or(u64::MAX)
                        })
            })
        }) {
            return Err(PostgresError::InvalidRecord);
        }
        let qualified =
            RunStore::qualify_prefix(self.scope.clone(), self.epoch, self.tenant.clone(), frames)
                .map_err(map_store_error)?;
        let recorded_digest = ContentDigest::parse(
            head.as_ref()
                .map(|value| value.1.as_str())
                .ok_or(PostgresError::InvalidRecord)?,
        )
        .map_err(|_| PostgresError::InvalidRecord)?;
        if qualified.head_digest().map_err(map_store_error)? != recorded_digest {
            return Err(PostgresError::InvalidRecord);
        }
        Ok(qualified)
    }

    /// Performs one append-atomic exact-head compare-and-append transaction.
    pub async fn append(&self, frame: RunFrame) -> Result<AppendDisposition> {
        frame.validate().map_err(|_| PostgresError::InvalidRecord)?;
        if frame.store_scope_id() != &self.scope || frame.store_epoch() != self.epoch {
            return Err(PostgresError::Identity);
        }
        if let RunRecord::RunAdmitted(admission) = frame.record() {
            if admission.tenant_scope_id() != &self.tenant {
                return Err(PostgresError::Identity);
            }
        }
        let encoded = frame
            .canonical_bytes()
            .map_err(|_| PostgresError::InvalidRecord)?;
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| PostgresError::Storage)?;
        let epoch = i64::try_from(self.epoch.get()).map_err(|_| PostgresError::Capacity)?;
        let existing: Option<(i64, Vec<u8>, String)> = sqlx::query_as(
            "SELECT run_sequence, frame_bytes, frame_digest FROM mfm_run_frames
             WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3
               AND run_id = $4 AND append_request_id = $5",
        )
        .bind(self.scope.as_str())
        .bind(epoch)
        .bind(self.tenant.as_str())
        .bind(frame.run_id().as_str())
        .bind(frame.append_request_id().as_str())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| PostgresError::Storage)?;
        if let Some((sequence, bytes, recorded_digest)) = existing {
            verify_frame_digest(&bytes, &recorded_digest)?;
            if bytes == encoded.as_bytes() {
                commit_transaction(transaction).await?;
                return Ok(AppendDisposition::Found {
                    sequence: u64::try_from(sequence).map_err(|_| PostgresError::InvalidRecord)?,
                });
            }
            return Err(PostgresError::Conflict);
        }

        let current: Option<(i64, String)> = sqlx::query_as(
            "SELECT head_sequence, head_digest FROM mfm_run_heads
             WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3 AND run_id = $4
             FOR UPDATE",
        )
        .bind(self.scope.as_str())
        .bind(epoch)
        .bind(self.tenant.as_str())
        .bind(frame.run_id().as_str())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| PostgresError::Storage)?;
        let actual = current.as_ref().map_or(0, |value| value.0);
        if frame.expected_sequence() != u64::try_from(actual.saturating_add(1)).unwrap_or(u64::MAX)
        {
            commit_transaction(transaction).await?;
            return Ok(AppendDisposition::StaleHead {
                actual_sequence: u64::try_from(actual).map_err(|_| PostgresError::InvalidRecord)?,
            });
        }
        if actual == 0 && !frame.record().is_admission() {
            return Err(PostgresError::InvalidRecord);
        }
        if actual > 0 && frame.record().is_admission() {
            return Err(PostgresError::InvalidRecord);
        }

        let existing_frames: Vec<(Vec<u8>, String)> = sqlx::query_as(
            "SELECT frame_bytes, frame_digest FROM mfm_run_frames
             WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3 AND run_id = $4
             ORDER BY run_sequence ASC",
        )
        .bind(self.scope.as_str())
        .bind(epoch)
        .bind(self.tenant.as_str())
        .bind(frame.run_id().as_str())
        .fetch_all(&mut *transaction)
        .await
        .map_err(|_| PostgresError::Storage)?;
        let mut candidate_frames = existing_frames
            .into_iter()
            .map(|(bytes, recorded_digest)| {
                verify_frame_digest(&bytes, &recorded_digest)?;
                decode_frame(&bytes)
            })
            .collect::<Result<Vec<_>>>()?;
        candidate_frames.push(frame.clone());
        RunStore::qualify_prefix(
            self.scope.clone(),
            self.epoch,
            self.tenant.clone(),
            candidate_frames,
        )
        .map_err(map_store_error)?;

        let previous_digest = current
            .as_ref()
            .map(|value| ContentDigest::parse(&value.1))
            .transpose()
            .map_err(|_| PostgresError::InvalidRecord)?;
        let head_digest = frame
            .head_digest(previous_digest.as_ref())
            .map_err(|_| PostgresError::InvalidRecord)?;
        let digest = mfm_canonical::raw_content_digest(encoded.as_bytes());
        sqlx::query(
            "INSERT INTO mfm_run_frames
             (store_scope_id, store_epoch, tenant_scope_id, run_id, run_sequence,
              append_request_id, frame_bytes, frame_digest)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(self.scope.as_str())
        .bind(epoch)
        .bind(self.tenant.as_str())
        .bind(frame.run_id().as_str())
        .bind(i64::try_from(frame.expected_sequence()).map_err(|_| PostgresError::Capacity)?)
        .bind(frame.append_request_id().as_str())
        .bind(encoded.as_bytes())
        .bind(digest.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresError::Conflict)?;
        if let Some(publication) = frame.record().fact_publication() {
            let current_fact_head: Option<i64> = sqlx::query_scalar(
                "SELECT publication_sequence FROM mfm_fact_heads
                 WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3
                 FOR UPDATE",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|_| PostgresError::Storage)?;
            let expected = current_fact_head.unwrap_or(0).saturating_add(1);
            if i64::try_from(publication.publication_sequence())
                .map_err(|_| PostgresError::Capacity)?
                != expected
            {
                return Err(PostgresError::Conflict);
            }
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
            .bind(expected)
            .execute(&mut *transaction)
            .await
            .map_err(|_| PostgresError::Storage)?;
            sqlx::query(
                "INSERT INTO mfm_fact_publications
                 (store_scope_id, store_epoch, tenant_scope_id, publication_sequence,
                  run_id, run_sequence, selection_ref)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(self.scope.as_str())
            .bind(epoch)
            .bind(self.tenant.as_str())
            .bind(expected)
            .bind(frame.run_id().as_str())
            .bind(i64::try_from(frame.expected_sequence()).map_err(|_| PostgresError::Capacity)?)
            .bind(
                serde_json::to_string(publication.selection().value_ref())
                    .map_err(|_| PostgresError::InvalidRecord)?,
            )
            .execute(&mut *transaction)
            .await
            .map_err(|_| PostgresError::Storage)?;
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
        .bind(frame.run_id().as_str())
        .bind(i64::try_from(frame.expected_sequence()).map_err(|_| PostgresError::Capacity)?)
        .bind(head_digest.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresError::Storage)?;
        if commit_transaction(transaction).await.is_err() {
            return Ok(AppendDisposition::AcknowledgementUnknown);
        }
        Ok(AppendDisposition::NewlyCommitted {
            sequence: frame.expected_sequence(),
        })
    }

    /// Returns the physical append identity used for an acknowledgement-resolution lookup.
    pub async fn contains_append(
        &self,
        run_id: &RunId,
        append_request_id: &AppendRequestId,
    ) -> Result<bool> {
        let found: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM mfm_run_frames
             WHERE store_scope_id = $1 AND store_epoch = $2 AND tenant_scope_id = $3
               AND run_id = $4 AND append_request_id = $5",
        )
        .bind(self.scope.as_str())
        .bind(i64::try_from(self.epoch.get()).map_err(|_| PostgresError::Capacity)?)
        .bind(self.tenant.as_str())
        .bind(run_id.as_str())
        .bind(append_request_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| PostgresError::Storage)?;
        Ok(found.is_some())
    }
}

async fn commit_transaction(transaction: Transaction<'_, Postgres>) -> Result<()> {
    transaction
        .commit()
        .await
        .map_err(|_| PostgresError::Storage)
}

fn decode_frame(bytes: &[u8]) -> Result<RunFrame> {
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|_| PostgresError::InvalidRecord)?;
    let frame: RunFrame =
        serde_json::from_slice(canonical.as_bytes()).map_err(|_| PostgresError::InvalidRecord)?;
    if frame
        .canonical_bytes()
        .map_err(|_| PostgresError::InvalidRecord)?
        .as_bytes()
        != bytes
    {
        return Err(PostgresError::InvalidRecord);
    }
    frame.validate().map_err(|_| PostgresError::InvalidRecord)?;
    Ok(frame)
}

fn verify_frame_digest(bytes: &[u8], recorded_digest: &str) -> Result<()> {
    if mfm_canonical::raw_content_digest(bytes).as_str() == recorded_digest {
        Ok(())
    } else {
        Err(PostgresError::InvalidRecord)
    }
}

fn map_store_error(error: mfm_store::StoreError) -> PostgresError {
    match error {
        mfm_store::StoreError::NotFound => PostgresError::NotFound,
        mfm_store::StoreError::Capacity => PostgresError::Capacity,
        mfm_store::StoreError::Identity => PostgresError::Identity,
        mfm_store::StoreError::InvalidRecord
        | mfm_store::StoreError::Conflict
        | mfm_store::StoreError::NotActionable
        | mfm_store::StoreError::InvalidHistory => PostgresError::InvalidRecord,
    }
}
