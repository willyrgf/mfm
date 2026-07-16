use std::collections::BTreeSet;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentDigest, SchemaId, StableAuthorKey};
use sqlx::{postgres::PgRow, Postgres, Row, Transaction};

use crate::run_store::{PostgresStore, PostgresStoreError, Result};

/// Maximum canonical configured-value payload size in bytes.
pub const MAX_CONFIGURED_VALUE_BYTES: usize = 256 * 1024;

/// One current, target-keyed semantic configuration value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredValueRow {
    /// Stable domain target that owns this configuration.
    pub target: StableAuthorKey,
    /// Typed configuration schema identity.
    pub schema_id: SchemaId,
    /// Content digest of the canonical configuration bytes.
    pub digest: ContentDigest,
    /// Canonical JSON bytes for the current configuration.
    pub canonical_json: Vec<u8>,
}

impl ConfiguredValueRow {
    /// Creates a current configuration row after validating its canonical bytes and digest.
    pub fn new(
        target: StableAuthorKey,
        schema_id: SchemaId,
        digest: ContentDigest,
        canonical_json: impl Into<Vec<u8>>,
    ) -> Result<Self> {
        let row = Self {
            target,
            schema_id,
            digest,
            canonical_json: canonical_json.into(),
        };
        validate_configured_value_row(&row)?;
        Ok(row)
    }
}

/// The effect of publishing one current configuration value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfiguredValuePublicationStatus {
    /// The target had no prior current value.
    Created,
    /// The target's current value changed.
    Updated,
    /// The target already held the identical current value.
    Unchanged,
}

/// A verified current value and the effect of its publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredValuePublication {
    /// Current configuration row after publication.
    pub row: ConfiguredValueRow,
    /// Result of publishing this target.
    pub status: ConfiguredValuePublicationStatus,
}

impl PostgresStore {
    /// Publishes a heterogeneous current-configuration batch atomically.
    ///
    /// Every target is processed in stable key order while one transaction holds the required row
    /// locks. A duplicate target is rejected before any database mutation. Existing targets are
    /// updated only when their schema, digest, or canonical bytes differ.
    pub async fn publish_configured_values(
        &self,
        rows: &[ConfiguredValueRow],
    ) -> Result<Vec<ConfiguredValuePublication>> {
        validate_configured_value_batch(rows)?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let mut rows = rows.to_vec();
        rows.sort_by(|left, right| left.target.cmp(&right.target));
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| PostgresStoreError::Database(database_context(error)))?;
        let mut publications = Vec::with_capacity(rows.len());

        for row in &rows {
            let status = match load_configured_value_tx(&mut transaction, &row.target, true).await?
            {
                Some(existing) if existing == *row => ConfiguredValuePublicationStatus::Unchanged,
                Some(_) => {
                    update_configured_value_tx(&mut transaction, row).await?;
                    ConfiguredValuePublicationStatus::Updated
                }
                None => insert_or_update_configured_value_tx(&mut transaction, row).await?,
            };
            let stored = load_configured_value_tx(&mut transaction, &row.target, true)
                .await?
                .ok_or_else(|| {
                    PostgresStoreError::Corruption(
                        "configured value disappeared during publication".to_owned(),
                    )
                })?;
            if stored != *row {
                return Err(PostgresStoreError::Corruption(
                    "configured value did not match the published canonical value".to_owned(),
                ));
            }
            publications.push(ConfiguredValuePublication {
                row: stored,
                status,
            });
        }

        transaction
            .commit()
            .await
            .map_err(|error| PostgresStoreError::Database(database_context(error)))?;
        Ok(publications)
    }

    /// Loads the current configuration for one stable target after integrity verification.
    pub async fn load_configured_value(
        &self,
        target: &StableAuthorKey,
    ) -> Result<Option<ConfiguredValueRow>> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| PostgresStoreError::Database(database_context(error)))?;
        let row = load_configured_value_tx(&mut transaction, target, false).await?;
        transaction
            .commit()
            .await
            .map_err(|error| PostgresStoreError::Database(database_context(error)))?;
        Ok(row)
    }

    /// Lists all current configured targets in stable target order.
    pub async fn list_configured_targets(&self) -> Result<Vec<StableAuthorKey>> {
        let rows = sqlx::query("SELECT target FROM catalog_values ORDER BY target")
            .fetch_all(&self.pool)
            .await
            .map_err(|error| PostgresStoreError::Database(database_context(error)))?;
        rows.into_iter()
            .map(|row| {
                let target = row.try_get::<String, _>("target").map_err(|_| {
                    PostgresStoreError::Corruption("configured target is invalid".to_owned())
                })?;
                StableAuthorKey::new(&target).map_err(|_| {
                    PostgresStoreError::Corruption("configured target is invalid".to_owned())
                })
            })
            .collect()
    }
}

async fn insert_or_update_configured_value_tx(
    transaction: &mut Transaction<'_, Postgres>,
    row: &ConfiguredValueRow,
) -> Result<ConfiguredValuePublicationStatus> {
    let inserted = sqlx::query(
        "INSERT INTO catalog_values (target, schema_id, digest, canonical_json) \
         VALUES ($1, $2, $3, $4) ON CONFLICT (target) DO NOTHING RETURNING target",
    )
    .bind(row.target.as_str())
    .bind(row.schema_id.as_str())
    .bind(row.digest.as_str())
    .bind(&row.canonical_json)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| PostgresStoreError::Database(database_context(error)))?;
    if inserted.is_some() {
        return Ok(ConfiguredValuePublicationStatus::Created);
    }

    let existing = load_configured_value_tx(transaction, &row.target, true)
        .await?
        .ok_or_else(|| {
            PostgresStoreError::Corruption(
                "configured value disappeared after a publication conflict".to_owned(),
            )
        })?;
    if existing == *row {
        Ok(ConfiguredValuePublicationStatus::Unchanged)
    } else {
        update_configured_value_tx(transaction, row).await?;
        Ok(ConfiguredValuePublicationStatus::Updated)
    }
}

async fn update_configured_value_tx(
    transaction: &mut Transaction<'_, Postgres>,
    row: &ConfiguredValueRow,
) -> Result<()> {
    let result = sqlx::query(
        "UPDATE catalog_values SET schema_id = $2, digest = $3, canonical_json = $4 \
         WHERE target = $1",
    )
    .bind(row.target.as_str())
    .bind(row.schema_id.as_str())
    .bind(row.digest.as_str())
    .bind(&row.canonical_json)
    .execute(&mut **transaction)
    .await
    .map_err(|error| PostgresStoreError::Database(database_context(error)))?;
    if result.rows_affected() != 1 {
        return Err(PostgresStoreError::Corruption(
            "configured value disappeared before update".to_owned(),
        ));
    }
    Ok(())
}

async fn load_configured_value_tx(
    transaction: &mut Transaction<'_, Postgres>,
    target: &StableAuthorKey,
    for_update: bool,
) -> Result<Option<ConfiguredValueRow>> {
    let query = if for_update {
        "SELECT target, schema_id, digest, canonical_json FROM catalog_values \
         WHERE target = $1 FOR UPDATE"
    } else {
        "SELECT target, schema_id, digest, canonical_json FROM catalog_values WHERE target = $1"
    };
    let Some(row) = sqlx::query(query)
        .bind(target.as_str())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(|error| PostgresStoreError::Database(database_context(error)))?
    else {
        return Ok(None);
    };
    configured_value_row_from_sql_row(&row).map(Some)
}

fn configured_value_row_from_sql_row(row: &PgRow) -> Result<ConfiguredValueRow> {
    let target = row
        .try_get::<String, _>("target")
        .map_err(|_| PostgresStoreError::Corruption("configured target is invalid".to_owned()))?;
    let target = StableAuthorKey::new(&target)
        .map_err(|_| PostgresStoreError::Corruption("configured target is invalid".to_owned()))?;
    let schema_id = row
        .try_get::<String, _>("schema_id")
        .map_err(|_| PostgresStoreError::Corruption("configured schema id is invalid".to_owned()))?
        .parse()
        .map_err(|_| {
            PostgresStoreError::Corruption("configured schema id is invalid".to_owned())
        })?;
    let digest = row
        .try_get::<String, _>("digest")
        .map_err(|_| PostgresStoreError::Corruption("configured digest is invalid".to_owned()))?
        .parse()
        .map_err(|_| PostgresStoreError::Corruption("configured digest is invalid".to_owned()))?;
    let canonical_json = row.try_get::<Vec<u8>, _>("canonical_json").map_err(|_| {
        PostgresStoreError::Corruption("configured canonical bytes are invalid".to_owned())
    })?;
    let value = ConfiguredValueRow {
        target,
        schema_id,
        digest,
        canonical_json,
    };
    validate_configured_value_row(&value)?;
    Ok(value)
}

fn validate_configured_value_batch(rows: &[ConfiguredValueRow]) -> Result<()> {
    let mut targets = BTreeSet::new();
    for row in rows {
        validate_configured_value_row(row)?;
        if !targets.insert(row.target.clone()) {
            return Err(PostgresStoreError::Corruption(
                "configured batch contains a duplicate target".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_configured_value_row(row: &ConfiguredValueRow) -> Result<()> {
    if !(1..=MAX_CONFIGURED_VALUE_BYTES).contains(&row.canonical_json.len()) {
        return Err(PostgresStoreError::Corruption(
            "configured canonical bytes exceed the permitted bound".to_owned(),
        ));
    }
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(&row.canonical_json)
        .map_err(|_| {
            PostgresStoreError::Corruption("configured canonical bytes are invalid".to_owned())
        })?;
    if canonical.content_digest() != row.digest {
        return Err(PostgresStoreError::Corruption(
            "configured digest does not match canonical bytes".to_owned(),
        ));
    }
    Ok(())
}

fn database_context(_error: sqlx::Error) -> &'static str {
    "configured-value operation failed"
}
