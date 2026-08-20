use std::future::Future;
use std::pin::Pin;

use mfm_catalog::{
    CatalogDeleteResult, CatalogEntry, CatalogError, CatalogInsertResult, CatalogPage,
    ConfigCatalog, ConfigCursor, ConfigDigest, ConfigName, PageLimit, MAX_CONFIG_ENTRIES,
};
use mfm_transport_security::LoadedTlsRoots;
use sqlx::postgres::{PgPoolOptions, PgRow};
use sqlx::{Connection, PgConnection, PgPool, Row};

use crate::{
    classify_open_error, verify_catalog_connection, RuntimePostgresLocator, StoreOpenError,
};

/// PostgreSQL config catalog after its independent connection gate succeeds.
pub struct PostgresCatalog {
    pool: PgPool,
    _roots: LoadedTlsRoots,
}

impl PostgresCatalog {
    /// Connects and verifies the independent config-catalog schema and runtime authority.
    pub async fn connect(locator: &RuntimePostgresLocator) -> Result<Self, StoreOpenError> {
        let (options, roots) = locator
            .connect_options("mfm-runtime-catalog")
            .await
            .map_err(|_| StoreOpenError::Unavailable)?;
        let mut gate_connection = PgConnection::connect_with(&options)
            .await
            .map_err(|_| StoreOpenError::Unavailable)?;
        verify_catalog_connection(&mut gate_connection)
            .await
            .map_err(crate::classify_gate_error)?;
        drop(gate_connection);
        let pool = PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_secs(2))
            .after_connect(|connection, _metadata| {
                Box::pin(async move {
                    verify_catalog_connection(connection)
                        .await
                        .map_err(|error| sqlx::Error::Protocol(error.marker().to_owned()))
                })
            })
            .connect_with(options)
            .await
            .map_err(classify_open_error)?;
        Ok(Self {
            pool,
            _roots: roots,
        })
    }

    #[cfg(test)]
    pub(crate) const fn test_pool(&self) -> &PgPool {
        &self.pool
    }
}

impl ConfigCatalog for PostgresCatalog {
    fn insert_config<'a>(
        &'a self,
        entry: &'a CatalogEntry,
    ) -> Pin<Box<dyn Future<Output = Result<CatalogInsertResult, CatalogError>> + Send + 'a>> {
        Box::pin(async move { insert_config(&self.pool, entry, MutationCommitFault::None).await })
    }

    fn load_config<'a>(
        &'a self,
        name: &'a ConfigName,
    ) -> Pin<Box<dyn Future<Output = Result<Option<CatalogEntry>, CatalogError>> + Send + 'a>> {
        Box::pin(async move { load_config(&self.pool, name).await })
    }

    fn list_configs<'a>(
        &'a self,
        cursor: Option<&'a ConfigCursor>,
        limit: PageLimit,
    ) -> Pin<Box<dyn Future<Output = Result<CatalogPage, CatalogError>> + Send + 'a>> {
        Box::pin(async move { list_configs(&self.pool, cursor, limit).await })
    }

    fn delete_config<'a>(
        &'a self,
        name: &'a ConfigName,
        digest: &'a ConfigDigest,
    ) -> Pin<Box<dyn Future<Output = Result<CatalogDeleteResult, CatalogError>> + Send + 'a>> {
        Box::pin(
            async move { delete_config(&self.pool, name, digest, MutationCommitFault::None).await },
        )
    }
}

async fn load_config(
    pool: &PgPool,
    name: &ConfigName,
) -> Result<Option<CatalogEntry>, CatalogError> {
    let row = sqlx::query(
        "SELECT config_name, config_digest, canonical \
         FROM mfm_catalog.config_entries WHERE config_name = $1",
    )
    .bind(name.as_str())
    .fetch_optional(pool)
    .await
    .map_err(|_| CatalogError::Unavailable)?;
    row.map(decode_entry).transpose()
}

async fn list_configs(
    pool: &PgPool,
    cursor: Option<&ConfigCursor>,
    limit: PageLimit,
) -> Result<CatalogPage, CatalogError> {
    let after = cursor.map_or("", |cursor| cursor.after().as_str());
    let query_limit = i64::try_from(limit.get() + 1).map_err(|_| CatalogError::Corrupt)?;
    let rows = sqlx::query(
        "SELECT config_name, config_digest, canonical \
         FROM mfm_catalog.config_entries WHERE config_name > $1 \
         ORDER BY config_name COLLATE \"C\" LIMIT $2",
    )
    .bind(after)
    .bind(query_limit)
    .fetch_all(pool)
    .await
    .map_err(|_| CatalogError::Unavailable)?;
    let mut items = rows
        .into_iter()
        .map(decode_entry)
        .collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > limit.get();
    if has_more {
        items.pop();
    }
    let next_cursor = if has_more {
        let last = items.last().ok_or(CatalogError::Corrupt)?;
        Some(ConfigCursor::after_name(last.name().clone()))
    } else {
        None
    };
    CatalogPage::new(items, next_cursor)
}

async fn insert_config(
    pool: &PgPool,
    entry: &CatalogEntry,
    fault: MutationCommitFault,
) -> Result<CatalogInsertResult, CatalogError> {
    let mut transaction = pool.begin().await.map_err(|_| CatalogError::Unavailable)?;
    configure_mutation(&mut transaction).await?;
    lock_catalog(&mut transaction).await?;
    let retained = sqlx::query(
        "SELECT config_name, config_digest, canonical \
         FROM mfm_catalog.config_entries WHERE config_name = $1",
    )
    .bind(entry.name().as_str())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|_| CatalogError::Unavailable)?
    .map(decode_entry)
    .transpose()?;
    if let Some(retained) = retained {
        let _ = transaction.rollback().await;
        return Ok(
            if retained.digest() == entry.digest()
                && retained.canonical_bytes() == entry.canonical_bytes()
            {
                CatalogInsertResult::Unchanged
            } else {
                CatalogInsertResult::Conflict
            },
        );
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM mfm_catalog.config_entries")
        .fetch_one(&mut *transaction)
        .await
        .map_err(|_| CatalogError::Unavailable)?;
    if usize::try_from(count)
        .ok()
        .is_none_or(|count| count >= MAX_CONFIG_ENTRIES)
    {
        let _ = transaction.rollback().await;
        return Err(CatalogError::Capacity);
    }
    sqlx::query(
        "INSERT INTO mfm_catalog.config_entries \
         (config_name, config_digest, canonical) VALUES ($1,$2,$3)",
    )
    .bind(entry.name().as_str())
    .bind(entry.digest().as_str())
    .bind(entry.canonical_bytes())
    .execute(&mut *transaction)
    .await
    .map_err(|_| CatalogError::Unavailable)?;
    commit_mutation(transaction, fault).await?;
    Ok(CatalogInsertResult::Inserted)
}

async fn delete_config(
    pool: &PgPool,
    name: &ConfigName,
    digest: &ConfigDigest,
    fault: MutationCommitFault,
) -> Result<CatalogDeleteResult, CatalogError> {
    let mut transaction = pool.begin().await.map_err(|_| CatalogError::Unavailable)?;
    configure_mutation(&mut transaction).await?;
    lock_catalog(&mut transaction).await?;
    let retained: Option<String> = sqlx::query_scalar(
        "SELECT config_digest FROM mfm_catalog.config_entries WHERE config_name = $1",
    )
    .bind(name.as_str())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|_| CatalogError::Unavailable)?;
    let Some(retained) = retained else {
        let _ = transaction.rollback().await;
        return Ok(CatalogDeleteResult::Absent);
    };
    let retained = ConfigDigest::parse(retained).map_err(|_| CatalogError::Corrupt)?;
    if &retained != digest {
        let _ = transaction.rollback().await;
        return Ok(CatalogDeleteResult::DigestMismatch);
    }
    let affected = sqlx::query(
        "DELETE FROM mfm_catalog.config_entries \
         WHERE config_name = $1 AND config_digest = $2",
    )
    .bind(name.as_str())
    .bind(digest.as_str())
    .execute(&mut *transaction)
    .await
    .map_err(|_| CatalogError::Unavailable)?
    .rows_affected();
    if affected != 1 {
        let _ = transaction.rollback().await;
        return Err(CatalogError::Corrupt);
    }
    commit_mutation(transaction, fault).await?;
    Ok(CatalogDeleteResult::Deleted)
}

fn decode_entry(row: PgRow) -> Result<CatalogEntry, CatalogError> {
    let name: &str = row
        .try_get("config_name")
        .map_err(|_| CatalogError::Corrupt)?;
    let digest: &str = row
        .try_get("config_digest")
        .map_err(|_| CatalogError::Corrupt)?;
    let canonical: &[u8] = row
        .try_get("canonical")
        .map_err(|_| CatalogError::Corrupt)?;
    CatalogEntry::new(
        ConfigName::new(name).map_err(|_| CatalogError::Corrupt)?,
        ConfigDigest::parse(digest).map_err(|_| CatalogError::Corrupt)?,
        canonical.to_vec(),
    )
    .map_err(|_| CatalogError::Corrupt)
}

async fn configure_mutation(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), CatalogError> {
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED, READ WRITE")
        .execute(&mut **transaction)
        .await
        .map_err(|_| CatalogError::Unavailable)?;
    sqlx::query("SET LOCAL synchronous_commit = on")
        .execute(&mut **transaction)
        .await
        .map_err(|_| CatalogError::Unavailable)?;
    Ok(())
}

async fn lock_catalog(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), CatalogError> {
    sqlx::query("SELECT pg_advisory_xact_lock(573463700271197453)")
        .execute(&mut **transaction)
        .await
        .map_err(|_| CatalogError::Unavailable)?;
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) enum MutationCommitFault {
    None,
    #[cfg(test)]
    BeforeSubmission,
    #[cfg(test)]
    UnknownRolledBack,
    #[cfg(test)]
    UnknownCommitted,
}

#[cfg(test)]
pub(crate) async fn insert_config_with_fault(
    pool: &PgPool,
    entry: &CatalogEntry,
    fault: MutationCommitFault,
) -> Result<CatalogInsertResult, CatalogError> {
    insert_config(pool, entry, fault).await
}

#[cfg(test)]
pub(crate) async fn delete_config_with_fault(
    pool: &PgPool,
    name: &ConfigName,
    digest: &ConfigDigest,
    fault: MutationCommitFault,
) -> Result<CatalogDeleteResult, CatalogError> {
    delete_config(pool, name, digest, fault).await
}

async fn commit_mutation(
    transaction: sqlx::Transaction<'_, sqlx::Postgres>,
    fault: MutationCommitFault,
) -> Result<(), CatalogError> {
    #[cfg(test)]
    match fault {
        MutationCommitFault::BeforeSubmission => {
            transaction
                .rollback()
                .await
                .map_err(|_| CatalogError::Unavailable)?;
            return Err(CatalogError::Unavailable);
        }
        MutationCommitFault::UnknownRolledBack => {
            let _ = transaction.rollback().await;
            return Err(CatalogError::Indeterminate);
        }
        MutationCommitFault::UnknownCommitted => {
            transaction
                .commit()
                .await
                .map_err(|_| CatalogError::Indeterminate)?;
            return Err(CatalogError::Indeterminate);
        }
        MutationCommitFault::None => {}
    }
    #[cfg(not(test))]
    let _ = fault;
    match transaction.commit().await {
        Ok(()) => Ok(()),
        Err(error) if error.as_database_error().is_some() => Err(CatalogError::Unavailable),
        Err(_) => Err(CatalogError::Indeterminate),
    }
}
