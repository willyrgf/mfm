use std::collections::BTreeMap;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentDigest, SchemaId};
use sqlx::{Postgres, Row, Transaction};

use crate::run_store::{PostgresStore, PostgresStoreError, Result};

/// Maximum canonical catalog payload size in bytes.
pub const MAX_CATALOG_VALUE_BYTES: usize = 256 * 1024;

/// Minimal raw identity used by PostgreSQL catalog operations.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CatalogValueKey {
    /// Checked catalog name text.
    pub name: String,
    /// Schema identity supplied by application assembly.
    pub schema_id: SchemaId,
    /// Exact content digest of canonical bytes.
    pub digest: ContentDigest,
}

impl CatalogValueKey {
    /// Creates a raw catalog key after validating its name grammar.
    pub fn new(
        name: impl Into<String>,
        schema_id: SchemaId,
        digest: ContentDigest,
    ) -> Result<Self> {
        let key = Self {
            name: name.into(),
            schema_id,
            digest,
        };
        validate_catalog_key(&key)?;
        Ok(key)
    }
}

/// Raw catalog row passed between application assembly and PostgreSQL storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogValueRow {
    /// Exact catalog key.
    pub key: CatalogValueKey,
    /// Canonical JSON bytes, validated by storage before persistence or return.
    pub canonical_json: Vec<u8>,
}

impl CatalogValueRow {
    /// Creates a raw row and validates its name, canonical bytes, and digest.
    pub fn new(
        name: impl Into<String>,
        schema_id: SchemaId,
        digest: ContentDigest,
        canonical_json: impl Into<Vec<u8>>,
    ) -> Result<Self> {
        let row = Self {
            key: CatalogValueKey::new(name, schema_id, digest)?,
            canonical_json: canonical_json.into(),
        };
        validate_catalog_row(&row)?;
        Ok(row)
    }
}

/// Metadata returned by bounded catalog listing without canonical payload bytes.
pub type CatalogValueSource = CatalogValueKey;

impl PostgresStore {
    /// Appends a heterogeneous catalog batch atomically and idempotently.
    pub async fn append_catalog_values(&self, rows: &[CatalogValueRow]) -> Result<()> {
        validate_catalog_batch(rows)?;
        if rows.is_empty() {
            return Ok(());
        }

        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| PostgresStoreError::Database(database_context(error)))?;

        for row in rows {
            sqlx::query(
                "INSERT INTO catalog_values (name, schema_id, digest, canonical_json) \
                 VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
            )
            .bind(&row.key.name)
            .bind(row.key.schema_id.as_str())
            .bind(row.key.digest.as_str())
            .bind(&row.canonical_json)
            .execute(&mut *transaction)
            .await
            .map_err(|error| PostgresStoreError::Database(database_context(error)))?;

            let stored = load_catalog_value_tx(&mut transaction, &row.key)
                .await?
                .ok_or(PostgresStoreError::Corruption(
                    "catalog value disappeared during append".to_owned(),
                ))?;
            if stored != *row {
                return Err(PostgresStoreError::Corruption(
                    "catalog value identity or canonical bytes conflict".to_owned(),
                ));
            }
        }

        transaction
            .commit()
            .await
            .map_err(|error| PostgresStoreError::Database(database_context(error)))?;
        Ok(())
    }

    /// Loads one exact catalog row after raw canonical and digest verification.
    pub async fn load_catalog_value(
        &self,
        key: &CatalogValueKey,
    ) -> Result<Option<CatalogValueRow>> {
        validate_catalog_key(key)?;
        load_catalog_value_client(&self.pool, key).await
    }

    /// Exports one exact catalog row after raw canonical and digest verification.
    pub async fn export_catalog_value(
        &self,
        key: &CatalogValueKey,
    ) -> Result<Option<CatalogValueRow>> {
        self.load_catalog_value(key).await
    }

    /// Lists catalog identities in key order with bounded keyset pagination.
    pub async fn list_catalog_values(
        &self,
        after: Option<&CatalogValueKey>,
        limit: u32,
    ) -> Result<Vec<CatalogValueSource>> {
        if !(1..=100).contains(&limit) {
            return Err(PostgresStoreError::Corruption(
                "catalog list limit must be between 1 and 100".to_owned(),
            ));
        }
        if let Some(after) = after {
            validate_catalog_key(after)?;
        }

        let rows = match after {
            Some(after) => {
                sqlx::query(
                    "SELECT name, schema_id, digest FROM catalog_values \
                     WHERE (name, schema_id, digest) > ($1, $2, $3) \
                     ORDER BY name, schema_id, digest LIMIT $4",
                )
                .bind(&after.name)
                .bind(after.schema_id.as_str())
                .bind(after.digest.as_str())
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query(
                    "SELECT name, schema_id, digest FROM catalog_values \
                 ORDER BY name, schema_id, digest LIMIT $1",
                )
                .bind(i64::from(limit))
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(|error| PostgresStoreError::Database(database_context(error)))?;

        rows.into_iter()
            .map(|row| {
                let name = row.try_get::<String, _>("name").map_err(|_| {
                    PostgresStoreError::Corruption("catalog name is invalid".to_owned())
                })?;
                let schema_id = row
                    .try_get::<String, _>("schema_id")
                    .map_err(|_| {
                        PostgresStoreError::Corruption("catalog schema id is invalid".to_owned())
                    })?
                    .parse()
                    .map_err(|_| {
                        PostgresStoreError::Corruption("catalog schema id is invalid".to_owned())
                    })?;
                let digest = row
                    .try_get::<String, _>("digest")
                    .map_err(|_| {
                        PostgresStoreError::Corruption("catalog digest is invalid".to_owned())
                    })?
                    .parse()
                    .map_err(|_| {
                        PostgresStoreError::Corruption("catalog digest is invalid".to_owned())
                    })?;
                let key = CatalogValueKey {
                    name,
                    schema_id,
                    digest,
                };
                validate_catalog_key(&key)?;
                Ok(key)
            })
            .collect()
    }
}

async fn load_catalog_value_client(
    pool: &sqlx::PgPool,
    key: &CatalogValueKey,
) -> Result<Option<CatalogValueRow>> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|error| PostgresStoreError::Database(database_context(error)))?;
    let row = load_catalog_value_tx(&mut transaction, key).await?;
    transaction
        .commit()
        .await
        .map_err(|error| PostgresStoreError::Database(database_context(error)))?;
    Ok(row)
}

async fn load_catalog_value_tx(
    transaction: &mut Transaction<'_, Postgres>,
    key: &CatalogValueKey,
) -> Result<Option<CatalogValueRow>> {
    let Some(row) = sqlx::query(
        "SELECT name, schema_id, digest, canonical_json FROM catalog_values \
         WHERE name = $1 AND schema_id = $2 AND digest = $3",
    )
    .bind(&key.name)
    .bind(key.schema_id.as_str())
    .bind(key.digest.as_str())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| PostgresStoreError::Database(database_context(error)))?
    else {
        return Ok(None);
    };

    let name = row
        .try_get::<String, _>("name")
        .map_err(|_| PostgresStoreError::Corruption("catalog name is invalid".to_owned()))?;
    let schema_id = row
        .try_get::<String, _>("schema_id")
        .map_err(|_| PostgresStoreError::Corruption("catalog schema id is invalid".to_owned()))?
        .parse()
        .map_err(|_| PostgresStoreError::Corruption("catalog schema id is invalid".to_owned()))?;
    let digest = row
        .try_get::<String, _>("digest")
        .map_err(|_| PostgresStoreError::Corruption("catalog digest is invalid".to_owned()))?
        .parse()
        .map_err(|_| PostgresStoreError::Corruption("catalog digest is invalid".to_owned()))?;
    let canonical_json = row.try_get::<Vec<u8>, _>("canonical_json").map_err(|_| {
        PostgresStoreError::Corruption("catalog canonical bytes are invalid".to_owned())
    })?;
    let result = CatalogValueRow {
        key: CatalogValueKey {
            name,
            schema_id,
            digest,
        },
        canonical_json,
    };
    validate_catalog_row(&result)?;
    Ok(Some(result))
}

fn validate_catalog_batch(rows: &[CatalogValueRow]) -> Result<()> {
    let mut identities = BTreeMap::new();
    for row in rows {
        validate_catalog_row(row)?;
        if let Some(previous) = identities.insert(row.key.clone(), row.canonical_json.clone()) {
            if previous != row.canonical_json {
                return Err(PostgresStoreError::Corruption(
                    "catalog batch contains conflicting exact identity".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

fn validate_catalog_row(row: &CatalogValueRow) -> Result<()> {
    validate_catalog_key(&row.key)?;
    if !(1..=MAX_CATALOG_VALUE_BYTES).contains(&row.canonical_json.len()) {
        return Err(PostgresStoreError::Corruption(
            "catalog canonical bytes exceed the permitted bound".to_owned(),
        ));
    }
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(&row.canonical_json)
        .map_err(|_| {
            PostgresStoreError::Corruption("catalog canonical bytes are invalid".to_owned())
        })?;
    if canonical.content_digest() != row.key.digest {
        return Err(PostgresStoreError::Corruption(
            "catalog digest does not match canonical bytes".to_owned(),
        ));
    }
    Ok(())
}

fn validate_catalog_key(key: &CatalogValueKey) -> Result<()> {
    if !valid_catalog_name(&key.name) {
        return Err(PostgresStoreError::Corruption(
            "catalog name violates the catalog grammar".to_owned(),
        ));
    }
    if key.schema_id.as_str().len() > 1024 || !key.schema_id.as_str().starts_with("schema:") {
        return Err(PostgresStoreError::Corruption(
            "catalog schema id violates the catalog bounds".to_owned(),
        ));
    }
    if !key.digest.as_str().starts_with("content:sha256-jcs-v1:") {
        return Err(PostgresStoreError::Corruption(
            "catalog digest violates the catalog bounds".to_owned(),
        ));
    }
    Ok(())
}

fn valid_catalog_name(value: &str) -> bool {
    if value.is_empty() || value.len() > 256 || !value.is_ascii() {
        return false;
    }
    value.split('/').all(|segment| {
        if segment.is_empty() || segment == "." || segment == ".." {
            return false;
        }
        let mut bytes = segment.bytes();
        let Some(first) = bytes.next() else {
            return false;
        };
        (first.is_ascii_lowercase() || first.is_ascii_digit())
            && bytes.all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'_' | b'-')
            })
    })
}

fn database_context(_error: sqlx::Error) -> &'static str {
    "catalog operation failed"
}

#[cfg(test)]
mod tests {
    use super::valid_catalog_name;

    #[test]
    fn storage_name_grammar_matches_the_catalog_contract() {
        assert!(valid_catalog_name("portfolios/treasury"));
        assert!(valid_catalog_name("a.b/c_2-d"));
        assert!(!valid_catalog_name("A/a"));
        assert!(!valid_catalog_name("a//b"));
        assert!(!valid_catalog_name("a/../b"));
    }
}
