use mfm_config::{
    ConfigDigest, ConfigImportResult, ConfigName, ConfigRepositoryError, ConfigRevision,
    ConfigRevisions, RetainedConfigRevision,
};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};

pub(super) async fn load_config(
    pool: &PgPool,
    name: &ConfigName,
    digest: Option<&ConfigDigest>,
) -> Result<Option<ConfigRevision>, ConfigRepositoryError> {
    let row = sqlx::query(
        "SELECT selected.config_name, selected.config_digest, selected.canonical, \
                state.revision_count, state.current_count \
         FROM ( \
           SELECT count(*) AS revision_count, count(*) FILTER (WHERE current) AS current_count \
           FROM mfm_config.config_revisions WHERE config_name = $1 \
         ) state \
         LEFT JOIN LATERAL ( \
           SELECT config_name, config_digest, canonical \
           FROM mfm_config.config_revisions \
           WHERE config_name = $1 \
             AND (($2::text IS NULL AND current) \
               OR ($2::text IS NOT NULL AND config_digest = $2)) \
         ) selected ON true",
    )
    .bind(name.as_str())
    .bind(digest.map(ConfigDigest::as_str))
    .fetch_one(pool)
    .await
    .map_err(|_| ConfigRepositoryError::Unavailable)?;
    let revision_count: i64 = row
        .try_get("revision_count")
        .map_err(|_| ConfigRepositoryError::Corrupt)?;
    let current_count: i64 = row
        .try_get("current_count")
        .map_err(|_| ConfigRepositoryError::Corrupt)?;
    if revision_count < 0
        || current_count < 0
        || revision_count > 0 && current_count != 1
        || revision_count == 0 && current_count != 0
    {
        return Err(ConfigRepositoryError::Corrupt);
    }
    decode_optional_revision(row)
}

pub(super) async fn list_configs(pool: &PgPool) -> Result<ConfigRevisions, ConfigRepositoryError> {
    let rows = sqlx::query(
        "SELECT config_name, config_digest, canonical, current \
         FROM mfm_config.config_revisions \
         ORDER BY config_name COLLATE \"C\", config_digest COLLATE \"C\"",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| ConfigRepositoryError::Unavailable)?;
    let items = rows
        .into_iter()
        .map(|row| {
            let current: bool = row
                .try_get("current")
                .map_err(|_| ConfigRepositoryError::Corrupt)?;
            Ok(RetainedConfigRevision::new(decode_revision(row)?, current))
        })
        .collect::<Result<Vec<_>, _>>()?;
    ConfigRevisions::new(items)
}

pub(super) async fn import_config(
    pool: &PgPool,
    revision: &ConfigRevision,
    fault: MutationCommitFault,
) -> Result<ConfigImportResult, ConfigRepositoryError> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|_| ConfigRepositoryError::Unavailable)?;
    configure_mutation(&mut transaction).await?;
    lock_config(&mut transaction).await?;
    let current = sqlx::query(
        "SELECT config_name, config_digest, canonical \
         FROM mfm_config.config_revisions WHERE config_name = $1 AND current",
    )
    .bind(revision.name().as_str())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|_| ConfigRepositoryError::Unavailable)?
    .map(decode_revision)
    .transpose()?;
    let revision_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM mfm_config.config_revisions WHERE config_name = $1",
    )
    .bind(revision.name().as_str())
    .fetch_one(&mut *transaction)
    .await
    .map_err(|_| ConfigRepositoryError::Unavailable)?;
    if revision_count < 0
        || current.is_none() && revision_count != 0
        || current.is_some() && revision_count == 0
    {
        let _ = transaction.rollback().await;
        return Err(ConfigRepositoryError::Corrupt);
    }
    let exact = sqlx::query(
        "SELECT config_name, config_digest, canonical \
         FROM mfm_config.config_revisions \
         WHERE config_name = $1 AND config_digest = $2",
    )
    .bind(revision.name().as_str())
    .bind(revision.digest().as_str())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|_| ConfigRepositoryError::Unavailable)?
    .map(decode_revision)
    .transpose()?;
    if let Some(exact) = &exact {
        if exact.canonical_bytes() != revision.canonical_bytes() {
            let _ = transaction.rollback().await;
            return Err(ConfigRepositoryError::Corrupt);
        }
    }
    if current
        .as_ref()
        .is_some_and(|current| current.digest() == revision.digest())
    {
        let _ = transaction.rollback().await;
        return Ok(ConfigImportResult::Unchanged);
    }
    if current.is_some() {
        let affected = sqlx::query(
            "UPDATE mfm_config.config_revisions SET current = false \
             WHERE config_name = $1 AND current",
        )
        .bind(revision.name().as_str())
        .execute(&mut *transaction)
        .await
        .map_err(|_| ConfigRepositoryError::Unavailable)?
        .rows_affected();
        if affected != 1 {
            let _ = transaction.rollback().await;
            return Err(ConfigRepositoryError::Corrupt);
        }
    }
    if exact.is_some() {
        let affected = sqlx::query(
            "UPDATE mfm_config.config_revisions SET current = true \
             WHERE config_name = $1 AND config_digest = $2 AND NOT current",
        )
        .bind(revision.name().as_str())
        .bind(revision.digest().as_str())
        .execute(&mut *transaction)
        .await
        .map_err(|_| ConfigRepositoryError::Unavailable)?
        .rows_affected();
        if affected != 1 {
            let _ = transaction.rollback().await;
            return Err(ConfigRepositoryError::Corrupt);
        }
    } else {
        sqlx::query(
            "INSERT INTO mfm_config.config_revisions \
             (config_name, config_digest, canonical, current) VALUES ($1,$2,$3,true)",
        )
        .bind(revision.name().as_str())
        .bind(revision.digest().as_str())
        .bind(revision.canonical_bytes())
        .execute(&mut *transaction)
        .await
        .map_err(|_| ConfigRepositoryError::Unavailable)?;
    }
    let outcome = if current.is_some() {
        ConfigImportResult::Updated
    } else {
        ConfigImportResult::Created
    };
    commit_mutation(transaction, fault).await?;
    Ok(outcome)
}

fn decode_revision(row: PgRow) -> Result<ConfigRevision, ConfigRepositoryError> {
    let name: &str = row
        .try_get("config_name")
        .map_err(|_| ConfigRepositoryError::Corrupt)?;
    let digest: &str = row
        .try_get("config_digest")
        .map_err(|_| ConfigRepositoryError::Corrupt)?;
    let canonical: &[u8] = row
        .try_get("canonical")
        .map_err(|_| ConfigRepositoryError::Corrupt)?;
    ConfigRevision::new(
        ConfigName::new(name).map_err(|_| ConfigRepositoryError::Corrupt)?,
        ConfigDigest::parse(digest).map_err(|_| ConfigRepositoryError::Corrupt)?,
        canonical.to_vec(),
    )
    .map_err(|_| ConfigRepositoryError::Corrupt)
}

fn decode_optional_revision(row: PgRow) -> Result<Option<ConfigRevision>, ConfigRepositoryError> {
    let name: Option<&str> = row
        .try_get("config_name")
        .map_err(|_| ConfigRepositoryError::Corrupt)?;
    let digest: Option<&str> = row
        .try_get("config_digest")
        .map_err(|_| ConfigRepositoryError::Corrupt)?;
    let canonical: Option<&[u8]> = row
        .try_get("canonical")
        .map_err(|_| ConfigRepositoryError::Corrupt)?;
    match (name, digest, canonical) {
        (Some(name), Some(digest), Some(canonical)) => ConfigRevision::new(
            ConfigName::new(name).map_err(|_| ConfigRepositoryError::Corrupt)?,
            ConfigDigest::parse(digest).map_err(|_| ConfigRepositoryError::Corrupt)?,
            canonical.to_vec(),
        )
        .map(Some)
        .map_err(|_| ConfigRepositoryError::Corrupt),
        (None, None, None) => Ok(None),
        _ => Err(ConfigRepositoryError::Corrupt),
    }
}

async fn configure_mutation(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), ConfigRepositoryError> {
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED, READ WRITE")
        .execute(&mut **transaction)
        .await
        .map_err(|_| ConfigRepositoryError::Unavailable)?;
    sqlx::query("SET LOCAL synchronous_commit = on")
        .execute(&mut **transaction)
        .await
        .map_err(|_| ConfigRepositoryError::Unavailable)?;
    Ok(())
}

async fn lock_config(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), ConfigRepositoryError> {
    sqlx::query("SELECT pg_advisory_xact_lock(573463700271197453)")
        .execute(&mut **transaction)
        .await
        .map_err(|_| ConfigRepositoryError::Unavailable)?;
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
pub(crate) async fn import_config_with_fault(
    pool: &PgPool,
    revision: &ConfigRevision,
    fault: MutationCommitFault,
) -> Result<ConfigImportResult, ConfigRepositoryError> {
    import_config(pool, revision, fault).await
}

async fn commit_mutation(
    transaction: sqlx::Transaction<'_, sqlx::Postgres>,
    fault: MutationCommitFault,
) -> Result<(), ConfigRepositoryError> {
    #[cfg(test)]
    match fault {
        MutationCommitFault::BeforeSubmission => {
            transaction
                .rollback()
                .await
                .map_err(|_| ConfigRepositoryError::Unavailable)?;
            return Err(ConfigRepositoryError::Unavailable);
        }
        MutationCommitFault::UnknownRolledBack => {
            let _ = transaction.rollback().await;
            return Err(ConfigRepositoryError::Indeterminate);
        }
        MutationCommitFault::UnknownCommitted => {
            transaction
                .commit()
                .await
                .map_err(|_| ConfigRepositoryError::Indeterminate)?;
            return Err(ConfigRepositoryError::Indeterminate);
        }
        MutationCommitFault::None => {}
    }
    #[cfg(not(test))]
    let _ = fault;
    match transaction.commit().await {
        Ok(()) => Ok(()),
        Err(error) if error.as_database_error().is_some() => {
            Err(ConfigRepositoryError::Unavailable)
        }
        Err(_) => Err(ConfigRepositoryError::Indeterminate),
    }
}
