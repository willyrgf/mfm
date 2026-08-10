use mfm_ids::{ContentRef, StoreScopeId};
use mfm_journal::structured::canonical_json;
use mfm_store::structured::MAX_CONFIGURATION_REVISION_BYTES;
use mfm_store::structured::{
    verify_configuration_history, CanonicalConfigurationAppend, ConfigurationBackendAppendOutcome,
    ConfigurationBackendFuture, ConfigurationHistoryBackend, ConfigurationHistoryHead,
    ConfigurationRevision, ConfigurationStreamKey, RawConfigurationHistory, StructuredStoreError,
};
use sqlx::postgres::PgRow;
use sqlx::{Postgres, Row, Transaction};

use crate::session::{RoleSession, TargetBinding};
#[cfg(feature = "test-support")]
use crate::transaction::await_read_phase_barrier;
use crate::transaction::{
    begin_configuration_read, begin_configuration_write_locked, CommitOutcome,
    LockedConfigurationWriteTx, ReadTx,
};

struct StoredConfigurationRow {
    revision_sequence: u64,
    store_scope_id: String,
    tenant_scope_id: String,
    entry_point_operation_id: String,
    target_id: String,
    predecessor_schema_id: Option<String>,
    predecessor_digest: Option<String>,
    append_request_id: String,
    value_contract_schema_id: String,
    value_contract_digest: String,
    value_schema_id: String,
    value_digest: String,
    revision_schema_id: String,
    revision_digest: String,
    canonical_revision_json: String,
}

struct StoredConfigurationHead {
    revision_sequence: u64,
    revision_schema_id: String,
    revision_digest: String,
}

impl StoredConfigurationHead {
    fn decode(row: &PgRow) -> Result<Self, StructuredStoreError> {
        Ok(Self {
            revision_sequence: required_sequence(row, "revision_sequence")?,
            revision_schema_id: required_text(row, "revision_schema_id")?,
            revision_digest: required_text(row, "revision_digest")?,
        })
    }

    fn reconstruct(self) -> Result<ConfigurationHistoryHead, StructuredStoreError> {
        let revision_ref = ContentRef::new(
            self.revision_schema_id
                .parse()
                .map_err(|_| invalid("PostgreSQL configuration head schema is invalid"))?,
            self.revision_digest
                .parse()
                .map_err(|_| invalid("PostgreSQL configuration head digest is invalid"))?,
        )
        .map_err(|_| invalid("PostgreSQL configuration head reference is invalid"))?;
        Ok(ConfigurationHistoryHead::new(
            self.revision_sequence,
            revision_ref,
        ))
    }
}

impl StoredConfigurationRow {
    fn decode(row: &PgRow) -> Result<Self, StructuredStoreError> {
        Ok(Self {
            revision_sequence: required_sequence(row, "revision_sequence")?,
            store_scope_id: required_text(row, "store_scope_id")?,
            tenant_scope_id: required_text(row, "tenant_scope_id")?,
            entry_point_operation_id: required_text(row, "entry_point_operation_id")?,
            target_id: required_text(row, "target_id")?,
            predecessor_schema_id: optional_text(row, "predecessor_schema_id")?,
            predecessor_digest: optional_text(row, "predecessor_digest")?,
            append_request_id: required_text(row, "append_request_id")?,
            value_contract_schema_id: required_text(row, "value_contract_schema_id")?,
            value_contract_digest: required_text(row, "value_contract_digest")?,
            value_schema_id: required_text(row, "value_schema_id")?,
            value_digest: required_text(row, "value_digest")?,
            revision_schema_id: required_text(row, "revision_schema_id")?,
            revision_digest: required_text(row, "revision_digest")?,
            canonical_revision_json: required_text(row, "canonical_revision_json")?,
        })
    }

    fn reconstruct(self) -> Result<ConfigurationRevision, StructuredStoreError> {
        if self.canonical_revision_json.len() < 2
            || self.canonical_revision_json.len() > MAX_CONFIGURATION_REVISION_BYTES
        {
            return Err(invalid(
                "PostgreSQL configuration revision exceeds its byte bound",
            ));
        }
        let revision: ConfigurationRevision =
            serde_json::from_str(&self.canonical_revision_json)
                .map_err(|_| invalid("PostgreSQL configuration revision cannot be decoded"))?;
        let canonical = canonical_json(&revision)
            .map_err(|_| invalid("PostgreSQL configuration revision cannot be canonicalized"))?;
        let predecessor_matches = match (
            revision.predecessor_ref(),
            self.predecessor_schema_id.as_deref(),
            self.predecessor_digest.as_deref(),
        ) {
            (None, None, None) => true,
            (Some(reference), Some(schema_id), Some(digest)) => {
                content_ref_matches(reference, schema_id, digest)
            }
            _ => false,
        };
        if canonical.as_str() != self.canonical_revision_json
            || revision.sequence() != self.revision_sequence
            || revision.key().store_scope_id().as_str() != self.store_scope_id
            || revision.key().tenant_scope_id().as_str() != self.tenant_scope_id
            || revision.key().entry_point_operation_id().as_str() != self.entry_point_operation_id
            || revision.key().target_id().as_str() != self.target_id
            || !predecessor_matches
            || revision.append_request_id().as_str() != self.append_request_id
            || !content_ref_matches(
                revision.value_contract_ref(),
                &self.value_contract_schema_id,
                &self.value_contract_digest,
            )
            || !content_ref_matches(
                revision.value_ref(),
                &self.value_schema_id,
                &self.value_digest,
            )
            || !content_ref_matches(
                revision.revision_ref(),
                &self.revision_schema_id,
                &self.revision_digest,
            )
        {
            return Err(invalid(
                "PostgreSQL configuration columns differ from canonical revision",
            ));
        }
        Ok(revision)
    }
}

/// PostgreSQL implementation of append-only configured-value history.
pub struct PostgresConfigurationHistoryBackend {
    configuration_reader: RoleSession,
    configuration_writer: Option<RoleSession>,
    target: TargetBinding,
    store_scope_id: StoreScopeId,
}

impl std::fmt::Debug for PostgresConfigurationHistoryBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PostgresConfigurationHistoryBackend")
            .field("store_scope_id", &self.store_scope_id)
            .finish_non_exhaustive()
    }
}

impl PostgresConfigurationHistoryBackend {
    pub(crate) fn from_sessions(
        configuration_reader: RoleSession,
        configuration_writer: Option<RoleSession>,
        target: TargetBinding,
    ) -> Self {
        Self {
            store_scope_id: target.store_scope_id().clone(),
            configuration_reader,
            configuration_writer,
            target,
        }
    }

    async fn begin_read(&self) -> Result<ReadTx<'_>, StructuredStoreError> {
        begin_configuration_read(&self.configuration_reader, &self.target).await
    }

    async fn begin_locked_write(
        &self,
        stream_lock_key: &str,
    ) -> Result<LockedConfigurationWriteTx<'_>, StructuredStoreError> {
        let writer = self
            .configuration_writer
            .as_ref()
            .ok_or(StructuredStoreError::BackendUnavailable)?;
        begin_configuration_write_locked(writer, &self.target, stream_lock_key).await
    }
}

impl ConfigurationHistoryBackend for PostgresConfigurationHistoryBackend {
    fn store_scope_id(&self) -> &StoreScopeId {
        &self.store_scope_id
    }

    fn load<'a>(
        &'a self,
        key: &'a ConfigurationStreamKey,
    ) -> ConfigurationBackendFuture<'a, Option<RawConfigurationHistory>> {
        Box::pin(async move {
            let mut transaction = self.begin_read().await?;
            let head = select_head(transaction.conn(), key).await?;
            #[cfg(feature = "test-support")]
            await_read_phase_barrier(self.target.schema_name(), "after_head").await;
            transaction.validate_target(&self.target).await?;
            let rows = select_rows(transaction.conn(), key, None).await?;
            transaction.commit_checked(&self.target).await?;
            let history = reconstruct_history(key, rows, head)?;
            if let Some(history) = history.as_ref() {
                // The indexed head is part of the same durable snapshot as the
                // revisions; validate the chain before returning it.
                verify_configuration_history(history.clone())?;
            }
            Ok(history)
        })
    }

    fn append<'a>(
        &'a self,
        revision: CanonicalConfigurationAppend,
    ) -> ConfigurationBackendFuture<'a, ConfigurationBackendAppendOutcome> {
        Box::pin(async move {
            if !revision.is_store_verified() {
                return Err(StructuredStoreError::InvalidHistory);
            }
            let revision = revision.into_revision();
            if revision.key().store_scope_id() != &self.store_scope_id {
                return Ok(ConfigurationBackendAppendOutcome::StaleHead);
            }
            let canonical_revision = canonical_json(&revision)
                .map_err(|_| invalid("configuration revision cannot be canonicalized"))?;
            if canonical_revision.as_bytes().len() > MAX_CONFIGURATION_REVISION_BYTES {
                return Err(invalid("configuration revision exceeds its byte bound"));
            }

            let lock_key = canonical_json(revision.key())
                .map_err(|_| invalid("configuration stream key cannot be canonicalized"))?;
            let mut transaction = self.begin_locked_write(lock_key.as_str()).await?;

            let rows = select_rows(transaction.conn(), revision.key(), None).await?;
            let head = select_head(transaction.conn(), revision.key()).await?;
            let history = reconstruct_history(revision.key(), rows, head)?;
            if let Some(history) = history.as_ref() {
                verify_configuration_history(history.clone())?;
            }
            if let Some(existing) = history.as_ref().and_then(|history| {
                history
                    .revisions
                    .iter()
                    .find(|existing| existing.append_request_id() == revision.append_request_id())
            }) {
                let existing = existing.clone();
                let outcome = transaction.commit_outcome(&self.target).await?;
                return match outcome {
                    CommitOutcome::AcknowledgementUnknown => {
                        Ok(ConfigurationBackendAppendOutcome::AcknowledgementUnknown)
                    }
                    CommitOutcome::Committed if existing == revision => {
                        Ok(ConfigurationBackendAppendOutcome::ExistingSame(existing))
                    }
                    CommitOutcome::Committed => Err(StructuredStoreError::AppendConflict),
                };
            }

            let current = history.as_ref().and_then(|history| history.head.as_ref());
            let predecessor_matches =
                current.map(ConfigurationHistoryHead::revision_ref) == revision.predecessor_ref();
            let sequence_matches = current
                .map_or(1, |current| current.sequence().saturating_add(1))
                == revision.sequence();
            if !predecessor_matches || !sequence_matches {
                transaction.rollback().await?;
                return Ok(ConfigurationBackendAppendOutcome::StaleHead);
            }

            transaction.validate_target(&self.target).await?;
            insert_revision(transaction.conn(), &revision, canonical_revision.as_str()).await?;
            if !advance_head(transaction.conn(), &revision, current).await? {
                transaction.rollback().await?;
                return Ok(ConfigurationBackendAppendOutcome::StaleHead);
            }
            match transaction.commit_outcome(&self.target).await? {
                CommitOutcome::Committed => {
                    Ok(ConfigurationBackendAppendOutcome::NewlyCommitted(revision))
                }
                CommitOutcome::AcknowledgementUnknown => {
                    Ok(ConfigurationBackendAppendOutcome::AcknowledgementUnknown)
                }
            }
        })
    }
}

async fn select_rows(
    transaction: &mut Transaction<'_, Postgres>,
    key: &ConfigurationStreamKey,
    append_request_id: Option<&str>,
) -> Result<Vec<StoredConfigurationRow>, StructuredStoreError> {
    let rows = if let Some(append_request_id) = append_request_id {
        sqlx::query(
            "SELECT store_scope_id, tenant_scope_id, entry_point_operation_id, target_id, \
                    revision_sequence::text AS revision_sequence, predecessor_schema_id, \
                    predecessor_digest, append_request_id, value_contract_schema_id, \
                    value_contract_digest, value_schema_id, value_digest, revision_schema_id, \
                    revision_digest, canonical_revision_json \
               FROM configuration_revisions \
              WHERE store_scope_id = $1 AND tenant_scope_id = $2 \
                AND entry_point_operation_id = $3 AND target_id = $4 \
                AND append_request_id = $5 \
              ORDER BY configuration_revisions.revision_sequence",
        )
        .bind(key.store_scope_id().as_str())
        .bind(key.tenant_scope_id().as_str())
        .bind(key.entry_point_operation_id().as_str())
        .bind(key.target_id().as_str())
        .bind(append_request_id)
        .fetch_all(&mut **transaction)
        .await
    } else {
        sqlx::query(
            "SELECT store_scope_id, tenant_scope_id, entry_point_operation_id, target_id, \
                    revision_sequence::text AS revision_sequence, predecessor_schema_id, \
                    predecessor_digest, append_request_id, value_contract_schema_id, \
                    value_contract_digest, value_schema_id, value_digest, revision_schema_id, \
                    revision_digest, canonical_revision_json \
               FROM configuration_revisions \
              WHERE store_scope_id = $1 AND tenant_scope_id = $2 \
                AND entry_point_operation_id = $3 AND target_id = $4 \
              ORDER BY configuration_revisions.revision_sequence",
        )
        .bind(key.store_scope_id().as_str())
        .bind(key.tenant_scope_id().as_str())
        .bind(key.entry_point_operation_id().as_str())
        .bind(key.target_id().as_str())
        .fetch_all(&mut **transaction)
        .await
    }
    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    rows.iter().map(StoredConfigurationRow::decode).collect()
}

async fn select_head(
    transaction: &mut Transaction<'_, Postgres>,
    key: &ConfigurationStreamKey,
) -> Result<Option<StoredConfigurationHead>, StructuredStoreError> {
    let row = sqlx::query(
        "SELECT revision_sequence::text AS revision_sequence, revision_schema_id, revision_digest \
           FROM configuration_heads \
          WHERE store_scope_id = $1 AND tenant_scope_id = $2 \
            AND entry_point_operation_id = $3 AND target_id = $4",
    )
    .bind(key.store_scope_id().as_str())
    .bind(key.tenant_scope_id().as_str())
    .bind(key.entry_point_operation_id().as_str())
    .bind(key.target_id().as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    row.as_ref()
        .map(StoredConfigurationHead::decode)
        .transpose()
}

fn reconstruct_history(
    key: &ConfigurationStreamKey,
    rows: Vec<StoredConfigurationRow>,
    head: Option<StoredConfigurationHead>,
) -> Result<Option<RawConfigurationHistory>, StructuredStoreError> {
    if rows.is_empty() && head.is_none() {
        return Ok(None);
    }
    let revisions = rows
        .into_iter()
        .map(StoredConfigurationRow::reconstruct)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(RawConfigurationHistory {
        key: key.clone(),
        head: head.map(StoredConfigurationHead::reconstruct).transpose()?,
        revisions,
    }))
}

async fn insert_revision(
    transaction: &mut Transaction<'_, Postgres>,
    revision: &ConfigurationRevision,
    canonical_revision: &str,
) -> Result<(), StructuredStoreError> {
    let predecessor_schema_id = revision
        .predecessor_ref()
        .map(|reference| reference.schema_id().as_str());
    let predecessor_digest = revision
        .predecessor_ref()
        .map(|reference| reference.content_digest().as_str());
    sqlx::query(
        "INSERT INTO configuration_revisions ( \
            store_scope_id, tenant_scope_id, entry_point_operation_id, target_id, \
            revision_sequence, predecessor_schema_id, predecessor_digest, append_request_id, \
            value_contract_schema_id, value_contract_digest, value_schema_id, value_digest, \
            revision_schema_id, revision_digest, canonical_revision_json \
         ) VALUES ( \
            $1, $2, $3, $4, $5::numeric, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15 \
         )",
    )
    .bind(revision.key().store_scope_id().as_str())
    .bind(revision.key().tenant_scope_id().as_str())
    .bind(revision.key().entry_point_operation_id().as_str())
    .bind(revision.key().target_id().as_str())
    .bind(revision.sequence().to_string())
    .bind(predecessor_schema_id)
    .bind(predecessor_digest)
    .bind(revision.append_request_id().as_str())
    .bind(revision.value_contract_ref().schema_id().as_str())
    .bind(revision.value_contract_ref().content_digest().as_str())
    .bind(revision.value_ref().schema_id().as_str())
    .bind(revision.value_ref().content_digest().as_str())
    .bind(revision.revision_ref().schema_id().as_str())
    .bind(revision.revision_ref().content_digest().as_str())
    .bind(canonical_revision)
    .execute(&mut **transaction)
    .await
    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    Ok(())
}

async fn advance_head(
    transaction: &mut Transaction<'_, Postgres>,
    revision: &ConfigurationRevision,
    current: Option<&ConfigurationHistoryHead>,
) -> Result<bool, StructuredStoreError> {
    let result = if let Some(current) = current {
        sqlx::query(
            "UPDATE configuration_heads \
                SET revision_sequence = $5::numeric, revision_schema_id = $6, revision_digest = $7 \
              WHERE store_scope_id = $1 AND tenant_scope_id = $2 \
                AND entry_point_operation_id = $3 AND target_id = $4 \
                AND revision_sequence = $8::numeric \
                AND revision_schema_id = $9 AND revision_digest = $10",
        )
        .bind(revision.key().store_scope_id().as_str())
        .bind(revision.key().tenant_scope_id().as_str())
        .bind(revision.key().entry_point_operation_id().as_str())
        .bind(revision.key().target_id().as_str())
        .bind(revision.sequence().to_string())
        .bind(revision.revision_ref().schema_id().as_str())
        .bind(revision.revision_ref().content_digest().as_str())
        .bind(current.sequence().to_string())
        .bind(current.revision_ref().schema_id().as_str())
        .bind(current.revision_ref().content_digest().as_str())
        .execute(&mut **transaction)
        .await
    } else {
        sqlx::query(
            "INSERT INTO configuration_heads ( \
                store_scope_id, tenant_scope_id, entry_point_operation_id, target_id, \
                revision_sequence, revision_schema_id, revision_digest \
             ) VALUES ($1, $2, $3, $4, $5::numeric, $6, $7)",
        )
        .bind(revision.key().store_scope_id().as_str())
        .bind(revision.key().tenant_scope_id().as_str())
        .bind(revision.key().entry_point_operation_id().as_str())
        .bind(revision.key().target_id().as_str())
        .bind(revision.sequence().to_string())
        .bind(revision.revision_ref().schema_id().as_str())
        .bind(revision.revision_ref().content_digest().as_str())
        .execute(&mut **transaction)
        .await
    }
    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    Ok(result.rows_affected() == 1)
}

fn content_ref_matches(reference: &ContentRef, schema_id: &str, digest: &str) -> bool {
    reference.schema_id().as_str() == schema_id && reference.content_digest().as_str() == digest
}

fn required_sequence(row: &PgRow, column: &str) -> Result<u64, StructuredStoreError> {
    required_text(row, column)?
        .parse()
        .map_err(|_| invalid("PostgreSQL configuration sequence is invalid"))
}

fn required_text(row: &PgRow, column: &str) -> Result<String, StructuredStoreError> {
    row.try_get(column)
        .map_err(|_| invalid("PostgreSQL configuration text is absent"))
}

fn optional_text(row: &PgRow, column: &str) -> Result<Option<String>, StructuredStoreError> {
    row.try_get(column)
        .map_err(|_| invalid("PostgreSQL configuration optional text is invalid"))
}

fn invalid(_message: &'static str) -> StructuredStoreError {
    StructuredStoreError::InvalidHistory
}
