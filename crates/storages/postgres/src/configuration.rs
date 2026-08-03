use mfm_ids::{ContentRef, StoreScopeId};
use mfm_journal::structured::canonical_json;
use mfm_store::structured::{
    verify_configuration_history, ConfigurationBackendAppendOutcome, ConfigurationBackendFuture,
    ConfigurationHistoryBackend, ConfigurationHistoryHead, ConfigurationRevision,
    ConfigurationStreamKey, RawConfigurationHistory, StructuredStoreError,
    ValidatedConfigurationRevision,
};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::qualification::AuthoritativeWriterContext;
use crate::schema::{APPLICATION_ROLE, CONFIGURATION_MAINTENANCE_ROLE, SCHEMA_CONTRACT_VERSION};

const MAX_REVISION_BYTES: usize = 16_777_216;

#[derive(Clone, Copy)]
enum ConfigurationRole {
    Application,
    Maintenance,
}

impl ConfigurationRole {
    const fn name(self) -> &'static str {
        match self {
            Self::Application => APPLICATION_ROLE,
            Self::Maintenance => CONFIGURATION_MAINTENANCE_ROLE,
        }
    }
}

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
            || self.canonical_revision_json.len() > MAX_REVISION_BYTES
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
    pool: PgPool,
    context: AuthoritativeWriterContext,
    store_scope_id: StoreScopeId,
    load_role: ConfigurationRole,
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
    pub(crate) fn new_application(pool: PgPool, context: AuthoritativeWriterContext) -> Self {
        Self::new(pool, context, ConfigurationRole::Application)
    }

    pub(crate) fn new_maintenance(pool: PgPool, context: AuthoritativeWriterContext) -> Self {
        Self::new(pool, context, ConfigurationRole::Maintenance)
    }

    fn new(
        pool: PgPool,
        context: AuthoritativeWriterContext,
        load_role: ConfigurationRole,
    ) -> Self {
        Self {
            store_scope_id: context.store_scope_id().clone(),
            pool,
            context,
            load_role,
        }
    }

    async fn begin(
        &self,
        role: ConfigurationRole,
    ) -> Result<Transaction<'_, Postgres>, StructuredStoreError> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
        let isolation = match role {
            ConfigurationRole::Application => "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
            // The per-stream advisory lock is the append linearization point. A
            // READ COMMITTED snapshot taken after that lock has been acquired
            // must observe the preceding lock holder's commit.
            ConfigurationRole::Maintenance => "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
        };
        sqlx::query(isolation)
            .execute(&mut *transaction)
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;
        match role {
            ConfigurationRole::Application => {
                sqlx::query("SET TRANSACTION READ ONLY")
                    .execute(&mut *transaction)
                    .await
                    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
                sqlx::query("SET LOCAL ROLE mfm_store_application")
                    .execute(&mut *transaction)
                    .await
                    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            }
            ConfigurationRole::Maintenance => {
                sqlx::query("SET TRANSACTION READ WRITE")
                    .execute(&mut *transaction)
                    .await
                    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
                sqlx::query("SET LOCAL ROLE mfm_store_configuration_maintenance")
                    .execute(&mut *transaction)
                    .await
                    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            }
        }
        sqlx::query(
            "SELECT pg_catalog.set_config( \
                 'search_path', pg_catalog.format('%I, pg_catalog', $1), TRUE \
             )",
        )
        .bind(self.context.schema_name())
        .execute(&mut *transaction)
        .await
        .map_err(|_| StructuredStoreError::BackendUnavailable)?;
        self.validate_local_authority(&mut transaction, role)
            .await?;
        Ok(transaction)
    }

    async fn validate_local_authority(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        role: ConfigurationRole,
    ) -> Result<(), StructuredStoreError> {
        let row = sqlx::query(
            "SELECT pg_catalog.current_database()::text AS database_name, \
                    pg_catalog.current_schema()::text AS schema_name, \
                    current_user::text AS role_name, \
                    database.oid::bigint AS database_oid, \
                    pg_catalog.pg_is_in_recovery() AS in_recovery, \
                    pg_catalog.current_setting('transaction_read_only') AS transaction_read_only, \
                    identity.store_scope_id, identity.store_epoch::text AS store_epoch, \
                    metadata.schema_contract_version \
               FROM pg_catalog.pg_database AS database \
               CROSS JOIN store_identity AS identity \
               CROSS JOIN store_schema_metadata AS metadata \
              WHERE database.datname = pg_catalog.current_database() \
                AND identity.singleton AND metadata.singleton",
        )
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|_| StructuredStoreError::BackendUnavailable)?
        .ok_or_else(|| invalid("PostgreSQL configuration authority is absent"))?;
        let database_oid = row
            .try_get::<i64, _>("database_oid")
            .ok()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| invalid("PostgreSQL configuration database identity is invalid"))?;
        let expected_read_only = match role {
            ConfigurationRole::Application => "on",
            ConfigurationRole::Maintenance => "off",
        };
        if row.try_get::<String, _>("database_name").ok().as_deref()
            != Some(self.context.database_name())
            || row.try_get::<String, _>("schema_name").ok().as_deref()
                != Some(self.context.schema_name())
            || row.try_get::<String, _>("role_name").ok().as_deref() != Some(role.name())
            || database_oid != self.context.database_oid()
            || row.try_get::<bool, _>("in_recovery").ok() != Some(false)
            || row
                .try_get::<String, _>("transaction_read_only")
                .ok()
                .as_deref()
                != Some(expected_read_only)
            || row.try_get::<String, _>("store_scope_id").ok().as_deref()
                != Some(self.store_scope_id.as_str())
            || row.try_get::<String, _>("store_epoch").ok().as_deref()
                != Some(self.context.store_epoch().get().to_string().as_str())
            || row
                .try_get::<String, _>("schema_contract_version")
                .ok()
                .as_deref()
                != Some(SCHEMA_CONTRACT_VERSION)
        {
            return Err(invalid(
                "PostgreSQL configuration authority changed after qualification",
            ));
        }
        Ok(())
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
            let mut transaction = self.begin(self.load_role).await?;
            let rows = select_rows(&mut transaction, key, None).await?;
            let head = select_head(&mut transaction, key).await?;
            transaction
                .commit()
                .await
                .map_err(|_| StructuredStoreError::BackendUnavailable)?;
            reconstruct_history(key, rows, head)
        })
    }

    fn append<'a>(
        &'a self,
        revision: ValidatedConfigurationRevision,
    ) -> ConfigurationBackendFuture<'a, ConfigurationBackendAppendOutcome> {
        Box::pin(async move {
            let revision = revision.into_revision();
            if revision.key().store_scope_id() != &self.store_scope_id {
                return Ok(ConfigurationBackendAppendOutcome::StaleHead);
            }
            let canonical_revision = canonical_json(&revision)
                .map_err(|_| invalid("configuration revision cannot be canonicalized"))?;
            if canonical_revision.as_bytes().len() > MAX_REVISION_BYTES {
                return Err(invalid("configuration revision exceeds its byte bound"));
            }

            let mut transaction = self.begin(ConfigurationRole::Maintenance).await?;
            let lock_key = canonical_json(revision.key())
                .map_err(|_| invalid("configuration stream key cannot be canonicalized"))?;
            sqlx::query(
                "SELECT pg_catalog.pg_advisory_xact_lock( \
                    pg_catalog.hashtextextended($1, 0) \
                 )",
            )
            .bind(lock_key.as_str())
            .execute(&mut *transaction)
            .await
            .map_err(|_| StructuredStoreError::BackendUnavailable)?;

            let rows = select_rows(&mut transaction, revision.key(), None).await?;
            let head = select_head(&mut transaction, revision.key()).await?;
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
                transaction
                    .commit()
                    .await
                    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
                return if existing == revision {
                    Ok(ConfigurationBackendAppendOutcome::ExistingSame(existing))
                } else {
                    Err(StructuredStoreError::AppendConflict)
                };
            }

            let current = history.as_ref().and_then(|history| history.head.as_ref());
            let predecessor_matches =
                current.map(ConfigurationHistoryHead::revision_ref) == revision.predecessor_ref();
            let sequence_matches = current
                .map_or(1, |current| current.sequence().saturating_add(1))
                == revision.sequence();
            if !predecessor_matches || !sequence_matches {
                transaction
                    .rollback()
                    .await
                    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
                return Ok(ConfigurationBackendAppendOutcome::StaleHead);
            }

            insert_revision(&mut transaction, &revision, canonical_revision.as_str()).await?;
            if !advance_head(&mut transaction, &revision, current).await? {
                transaction
                    .rollback()
                    .await
                    .map_err(|_| StructuredStoreError::BackendUnavailable)?;
                return Ok(ConfigurationBackendAppendOutcome::StaleHead);
            }
            if transaction.commit().await.is_err() {
                return Ok(ConfigurationBackendAppendOutcome::AcknowledgementUnknown);
            }
            Ok(ConfigurationBackendAppendOutcome::NewlyCommitted(revision))
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
              ORDER BY revision_sequence",
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
              ORDER BY revision_sequence",
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
