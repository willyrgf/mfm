use mfm_ids::{ContentRef, StableId, StoreScopeId};
use mfm_journal::structured::canonical_json;
use mfm_journal::structured::HistoryObject;
use mfm_store::structured::MAX_CONFIGURATION_REVISION_BYTES;
use mfm_store::structured::{
    ConfigurationBackendAppendOutcome, ConfigurationBackendFuture, ConfigurationHistoryBackend,
    ConfigurationHistoryHead, ConfigurationRevisionObject, ConfigurationStreamKey,
    RawConfigurationHistory, StructuredStoreError, ValidatedConfigurationAppend,
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
        let object_ref = ContentRef::new(
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
            object_ref,
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
            revision_schema_id: required_text(row, "revision_schema_id")?,
            revision_digest: required_text(row, "revision_digest")?,
            canonical_revision_json: required_text(row, "canonical_revision_json")?,
        })
    }

    fn reconstruct(self) -> Result<ConfigurationRevisionObject, StructuredStoreError> {
        if self.canonical_revision_json.len() < 2
            || self.canonical_revision_json.len() > MAX_CONFIGURATION_REVISION_BYTES
        {
            return Err(invalid(
                "PostgreSQL configuration revision exceeds its byte bound",
            ));
        }
        let content_ref = ContentRef::new(
            self.revision_schema_id
                .parse()
                .map_err(|_| invalid("PostgreSQL configuration schema is invalid"))?,
            self.revision_digest
                .parse()
                .map_err(|_| invalid("PostgreSQL configuration digest is invalid"))?,
        )
        .map_err(|_| invalid("PostgreSQL configuration reference is invalid"))?;
        let revision = ConfigurationRevisionObject::from_object(HistoryObject {
            object_type: StableId::new(
                mfm_journal::structured::ADMISSION_CONFIGURATION_OBJECT_TYPE,
            )
            .map_err(|_| invalid("configuration object type is invalid"))?,
            content_ref,
            canonical_json: self.canonical_revision_json.clone(),
        })?;
        let payload = revision.revision();
        let predecessor_matches = match (
            payload.predecessor_ref(),
            self.predecessor_schema_id.as_deref(),
            self.predecessor_digest.as_deref(),
        ) {
            (None, None, None) => true,
            (Some(reference), Some(schema_id), Some(digest)) => {
                content_ref_matches(reference, schema_id, digest)
            }
            _ => false,
        };
        if payload.sequence() != self.revision_sequence
            || payload.key().store_scope_id().as_str() != self.store_scope_id
            || payload.key().tenant_scope_id().as_str() != self.tenant_scope_id
            || payload.key().entry_point_operation_id().as_str() != self.entry_point_operation_id
            || payload.key().target_id().as_str() != self.target_id
            || !predecessor_matches
            || payload.append_request_id().as_str() != self.append_request_id
            || !content_ref_matches(
                revision.content_ref(),
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

impl mfm_authority_seal::ValidatedAppendConsumerSeal for PostgresConfigurationHistoryBackend {}

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
            reconstruct_history(key, rows, head)
        })
    }

    fn load_store_snapshot(&self) -> ConfigurationBackendFuture<'_, Vec<RawConfigurationHistory>> {
        Box::pin(async move {
            let mut transaction = self.begin_read().await?;
            let mut histories = Vec::new();
            let mut after = None;
            loop {
                let page = select_stream_keys(
                    transaction.conn(),
                    &self.store_scope_id,
                    after.as_ref(),
                    256,
                )
                .await?;
                for key in &page {
                    let head = select_head(transaction.conn(), key).await?;
                    let rows = select_rows(transaction.conn(), key, None).await?;
                    histories.push(
                        reconstruct_history(key, rows, head)?
                            .ok_or_else(|| invalid("configuration snapshot key has no state"))?,
                    );
                }
                after = page.last().cloned();
                if page.len() < 256 {
                    break;
                }
            }
            transaction.commit_checked(&self.target).await?;
            Ok(histories)
        })
    }

    fn validate_authority(&self) -> ConfigurationBackendFuture<'_, ()> {
        Box::pin(async move {
            let mut transaction = self.begin_read().await?;
            transaction.validate_target(&self.target).await?;
            transaction.commit_checked(&self.target).await
        })
    }

    fn append<'a>(
        &'a self,
        command: ValidatedConfigurationAppend,
    ) -> ConfigurationBackendFuture<'a, ConfigurationBackendAppendOutcome> {
        Box::pin(async move {
            // A validated append exists only after the store proved it.
            let (revision, expected_head, successor_head) = command.into_parts(self);
            let payload = revision.revision();
            if payload.key().store_scope_id() != &self.store_scope_id {
                return Ok(ConfigurationBackendAppendOutcome::StaleHead);
            }
            let canonical_revision = revision.object().canonical_json.as_str();
            if canonical_revision.len() > MAX_CONFIGURATION_REVISION_BYTES {
                return Err(invalid("configuration revision exceeds its byte bound"));
            }

            let lock_key = canonical_json(payload.key())
                .map_err(|_| invalid("configuration stream key cannot be canonicalized"))?;
            let mut transaction = self.begin_locked_write(lock_key.as_str()).await?;

            let rows = select_rows(transaction.conn(), payload.key(), None).await?;
            let head = select_head(transaction.conn(), payload.key()).await?;
            let history = reconstruct_history(payload.key(), rows, head)?;
            if let Some(existing) = history.as_ref().and_then(|history| {
                history.revisions.iter().find(|existing| {
                    existing.revision().append_request_id() == payload.append_request_id()
                })
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
            if current != expected_head.as_ref() {
                transaction.rollback().await?;
                return Ok(ConfigurationBackendAppendOutcome::StaleHead);
            }

            transaction.validate_target(&self.target).await?;
            insert_revision(transaction.conn(), &revision, canonical_revision).await?;
            if !advance_head(transaction.conn(), payload.key(), &successor_head, current).await? {
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

async fn select_stream_keys(
    transaction: &mut Transaction<'_, Postgres>,
    store_scope_id: &StoreScopeId,
    after: Option<&ConfigurationStreamKey>,
    maximum_items: u32,
) -> Result<Vec<ConfigurationStreamKey>, StructuredStoreError> {
    let rows = sqlx::query(
        "SELECT tenant_scope_id, entry_point_operation_id, target_id FROM ( \
             SELECT tenant_scope_id, entry_point_operation_id, target_id \
               FROM configuration_revisions WHERE store_scope_id = $1 \
             UNION \
             SELECT tenant_scope_id, entry_point_operation_id, target_id \
               FROM configuration_heads WHERE store_scope_id = $1 \
         ) AS stream_keys \
         WHERE ($2::text IS NULL OR (tenant_scope_id, entry_point_operation_id, target_id) \
               > ($2::text, $3::text, $4::text)) \
         ORDER BY tenant_scope_id, entry_point_operation_id, target_id \
         LIMIT $5::bigint",
    )
    .bind(store_scope_id.as_str())
    .bind(after.map(|key| key.tenant_scope_id().as_str()))
    .bind(after.map(|key| key.entry_point_operation_id().as_str()))
    .bind(after.map(|key| key.target_id().as_str()))
    .bind(i64::from(maximum_items))
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    rows.into_iter()
        .map(|row| {
            Ok(ConfigurationStreamKey::new(
                store_scope_id.clone(),
                required_text(&row, "tenant_scope_id")?
                    .parse()
                    .map_err(|_| invalid("configuration tenant key is invalid"))?,
                required_text(&row, "entry_point_operation_id")?
                    .parse()
                    .map_err(|_| invalid("configuration operation key is invalid"))?,
                required_text(&row, "target_id")?
                    .parse()
                    .map_err(|_| invalid("configuration target key is invalid"))?,
            ))
        })
        .collect()
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
                    predecessor_digest, append_request_id, revision_schema_id, revision_digest, \
                    canonical_revision_json \
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
                    predecessor_digest, append_request_id, revision_schema_id, revision_digest, \
                    canonical_revision_json \
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
    revision: &ConfigurationRevisionObject,
    canonical_revision: &str,
) -> Result<(), StructuredStoreError> {
    let payload = revision.revision();
    let predecessor_schema_id = payload
        .predecessor_ref()
        .map(|reference| reference.schema_id().as_str());
    let predecessor_digest = payload
        .predecessor_ref()
        .map(|reference| reference.content_digest().as_str());
    sqlx::query(
        "INSERT INTO configuration_revisions ( \
            store_scope_id, tenant_scope_id, entry_point_operation_id, target_id, \
            revision_sequence, predecessor_schema_id, predecessor_digest, append_request_id, \
            revision_schema_id, revision_digest, canonical_revision_json \
         ) VALUES ( \
            $1, $2, $3, $4, $5::numeric, $6, $7, $8, $9, $10, $11 \
         )",
    )
    .bind(payload.key().store_scope_id().as_str())
    .bind(payload.key().tenant_scope_id().as_str())
    .bind(payload.key().entry_point_operation_id().as_str())
    .bind(payload.key().target_id().as_str())
    .bind(payload.sequence().to_string())
    .bind(predecessor_schema_id)
    .bind(predecessor_digest)
    .bind(payload.append_request_id().as_str())
    .bind(revision.content_ref().schema_id().as_str())
    .bind(revision.content_ref().content_digest().as_str())
    .bind(canonical_revision)
    .execute(&mut **transaction)
    .await
    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
    Ok(())
}

async fn advance_head(
    transaction: &mut Transaction<'_, Postgres>,
    key: &ConfigurationStreamKey,
    successor: &ConfigurationHistoryHead,
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
        .bind(key.store_scope_id().as_str())
        .bind(key.tenant_scope_id().as_str())
        .bind(key.entry_point_operation_id().as_str())
        .bind(key.target_id().as_str())
        .bind(successor.sequence().to_string())
        .bind(successor.object_ref().schema_id().as_str())
        .bind(successor.object_ref().content_digest().as_str())
        .bind(current.sequence().to_string())
        .bind(current.object_ref().schema_id().as_str())
        .bind(current.object_ref().content_digest().as_str())
        .execute(&mut **transaction)
        .await
    } else {
        sqlx::query(
            "INSERT INTO configuration_heads ( \
                store_scope_id, tenant_scope_id, entry_point_operation_id, target_id, \
                revision_sequence, revision_schema_id, revision_digest \
             ) VALUES ($1, $2, $3, $4, $5::numeric, $6, $7)",
        )
        .bind(key.store_scope_id().as_str())
        .bind(key.tenant_scope_id().as_str())
        .bind(key.entry_point_operation_id().as_str())
        .bind(key.target_id().as_str())
        .bind(successor.sequence().to_string())
        .bind(successor.object_ref().schema_id().as_str())
        .bind(successor.object_ref().content_digest().as_str())
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
