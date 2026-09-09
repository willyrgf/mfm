use mfm_config::{ConfigDigest, ConfigImportResult, ConfigRepositoryError, ConfigRevision};
use mfm_ids::ConfigName;
use sqlx::PgPool;

pub(super) async fn load_config(
    pool: &PgPool,
    name: &ConfigName,
    digest: &ConfigDigest,
) -> Result<Option<ConfigRevision>, ConfigRepositoryError> {
    sqlx::query_as!(
        ConfigRow,
        "SELECT config_name, config_digest, canonical FROM ONLY \
         mfm_config.config_revisions WHERE config_name = $1 AND config_digest = \
         $2",
        name.as_str(),
        digest.as_str(),
    )
    .fetch_optional(pool)
    .await
    .map_err(classify_revision_query)?
    .map(decode_revision)
    .transpose()
}

pub(super) async fn list_configs(
    pool: &PgPool,
) -> Result<Vec<ConfigRevision>, ConfigRepositoryError> {
    sqlx::query_as!(
        ConfigRow,
        "SELECT config_name, config_digest, canonical FROM ONLY \
         mfm_config.config_revisions ORDER BY config_name COLLATE \"C\", \
         config_digest COLLATE \"C\"",
    )
    .fetch_all(pool)
    .await
    .map_err(classify_revision_query)?
    .into_iter()
    .map(decode_revision)
    .collect()
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
    let affected = sqlx::query!(
        "INSERT INTO mfm_config.config_revisions (config_name, config_digest, \
         canonical) VALUES ($1,$2,$3) ON CONFLICT (config_name, config_digest) DO \
         NOTHING",
        revision.name().as_str(),
        revision.digest().as_str(),
        revision.canonical_bytes(),
    )
    .execute(&mut *transaction)
    .await
    .map_err(|_| ConfigRepositoryError::Unavailable)?
    .rows_affected();
    if affected == 1 {
        commit_mutation(transaction, fault).await?;
        return Ok(ConfigImportResult::Created);
    }
    let retained = sqlx::query_as!(
        ConfigRow,
        "SELECT config_name, config_digest, canonical FROM ONLY \
         mfm_config.config_revisions WHERE config_name = $1 AND config_digest = \
         $2",
        revision.name().as_str(),
        revision.digest().as_str(),
    )
    .fetch_optional(&mut *transaction)
    .await
    .map_err(classify_revision_query)?
    .map(decode_revision)
    .transpose()?
    .ok_or(ConfigRepositoryError::Corrupt)?;
    let _ = transaction.rollback().await;
    if retained.canonical_bytes() == revision.canonical_bytes() {
        Ok(ConfigImportResult::Unchanged)
    } else {
        Err(ConfigRepositoryError::Corrupt)
    }
}

pub(super) async fn delete_config(
    pool: &PgPool,
    name: &ConfigName,
    digest: &ConfigDigest,
    fault: MutationCommitFault,
) -> Result<(), ConfigRepositoryError> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|_| ConfigRepositoryError::Unavailable)?;
    configure_mutation(&mut transaction).await?;
    let affected = sqlx::query!(
        "DELETE FROM ONLY mfm_config.config_revisions WHERE config_name = $1 AND \
         config_digest = $2",
        name.as_str(),
        digest.as_str(),
    )
    .execute(&mut *transaction)
    .await
    .map_err(|_| ConfigRepositoryError::Unavailable)?
    .rows_affected();
    if affected == 0 {
        let _ = transaction.rollback().await;
        return Ok(());
    }
    commit_mutation(transaction, fault).await
}

struct ConfigRow {
    config_name: String,
    config_digest: String,
    canonical: Vec<u8>,
}

fn decode_revision(row: ConfigRow) -> Result<ConfigRevision, ConfigRepositoryError> {
    ConfigRevision::new(
        ConfigName::new(row.config_name).map_err(|_| ConfigRepositoryError::Corrupt)?,
        ConfigDigest::parse(row.config_digest).map_err(|_| ConfigRepositoryError::Corrupt)?,
        row.canonical,
    )
    .map_err(|_| ConfigRepositoryError::Corrupt)
}

async fn configure_mutation(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), ConfigRepositoryError> {
    sqlx::query!("SET LOCAL synchronous_commit = on")
        .execute(&mut **transaction)
        .await
        .map_err(|_| ConfigRepositoryError::Unavailable)?;
    Ok(())
}

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

#[cfg(test)]
pub(crate) async fn delete_config_with_fault(
    pool: &PgPool,
    name: &ConfigName,
    digest: &ConfigDigest,
    fault: MutationCommitFault,
) -> Result<(), ConfigRepositoryError> {
    delete_config(pool, name, digest, fault).await
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

fn classify_revision_query(error: sqlx::Error) -> ConfigRepositoryError {
    match error {
        sqlx::Error::ColumnDecode { .. }
        | sqlx::Error::Decode(_)
        | sqlx::Error::ColumnNotFound(_)
        | sqlx::Error::ColumnIndexOutOfBounds { .. } => ConfigRepositoryError::Corrupt,
        _ => ConfigRepositoryError::Unavailable,
    }
}
