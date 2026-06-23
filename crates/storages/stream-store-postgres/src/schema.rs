use std::collections::BTreeSet;

use sqlx::{PgPool, Row};

use crate::run_store::{PostgresStoreError, Result};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// PostgreSQL schema management for the run store.
pub struct PostgresSchema;

impl PostgresSchema {
    /// Applies all pending run-store migrations.
    pub async fn migrate(database_url: &str) -> Result<()> {
        let pool = connect_pool(database_url).await?;
        migrate_pool(&pool).await?;
        Ok(())
    }

    /// Applies migrations using the `DATABASE_URL` environment variable.
    pub async fn migrate_env() -> Result<()> {
        let database_url = database_url_env()?;
        Self::migrate(&database_url).await
    }

    /// Verifies that all expected migrations are applied and the schema contains required objects.
    pub async fn validate(database_url: &str) -> Result<()> {
        let pool = connect_pool(database_url).await?;
        validate_pool(&pool).await
    }

    /// Verifies schema compatibility using the `DATABASE_URL` environment variable.
    pub async fn validate_env() -> Result<()> {
        let database_url = database_url_env()?;
        Self::validate(&database_url).await
    }
}

pub(crate) async fn migrate_pool(pool: &PgPool) -> Result<()> {
    MIGRATOR
        .run(pool)
        .await
        .map_err(|_| PostgresStoreError::Database("migration failed"))?;
    Ok(())
}

pub(crate) async fn connect_pool(database_url: &str) -> Result<PgPool> {
    PgPool::connect(database_url)
        .await
        .map_err(|_| PostgresStoreError::Database("connect failed"))
}

pub(crate) async fn validate_pool(pool: &PgPool) -> Result<()> {
    validate_migrations(pool).await?;
    validate_catalog(pool).await?;
    validate_store_metadata(pool).await
}

fn database_url_env() -> Result<String> {
    std::env::var("DATABASE_URL").map_err(|_| PostgresStoreError::Database("missing DATABASE_URL"))
}

async fn validate_migrations(pool: &PgPool) -> Result<()> {
    let rows =
        sqlx::query!("SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .map_err(|_| PostgresStoreError::Database("schema migrations missing"))?;

    if rows.len() != MIGRATOR.iter().count() {
        return Err(PostgresStoreError::Database(
            "schema migration count mismatch",
        ));
    }

    for migration in MIGRATOR.iter() {
        let row = rows
            .iter()
            .find(|row| row.version == migration.version)
            .ok_or(PostgresStoreError::Database("schema migration missing"))?;
        if !row.success {
            return Err(PostgresStoreError::Database("schema migration failed"));
        }
        if row.checksum.as_slice() != migration.checksum.as_ref() {
            return Err(PostgresStoreError::Database(
                "schema migration checksum mismatch",
            ));
        }
    }

    Ok(())
}

async fn validate_catalog(pool: &PgPool) -> Result<()> {
    let table_rows = sqlx::query!(
        "SELECT table_name as \"table_name!\" \
         FROM information_schema.tables \
         WHERE table_schema = current_schema() AND table_type = 'BASE TABLE'"
    )
    .fetch_all(pool)
    .await
    .map_err(|_| PostgresStoreError::Database("failed to inspect schema tables"))?;
    let tables = table_rows
        .into_iter()
        .map(|row| row.table_name)
        .collect::<BTreeSet<_>>();

    for table in REQUIRED_TABLES {
        if !tables.contains(*table) {
            return Err(PostgresStoreError::Database(
                "required schema table missing",
            ));
        }
    }
    for table in FORBIDDEN_TABLES {
        if tables.contains(*table) {
            return Err(PostgresStoreError::Database("stale schema table present"));
        }
    }

    let view_rows = sqlx::query(
        "SELECT table_name \
         FROM information_schema.views \
         WHERE table_schema = current_schema()",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| PostgresStoreError::Database("failed to inspect schema views"))?;
    let views = view_rows
        .into_iter()
        .map(|row| row.try_get::<String, _>("table_name"))
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(|_| PostgresStoreError::Database("failed to decode schema views"))?;
    for view in REQUIRED_VIEWS {
        if !views.contains(*view) {
            return Err(PostgresStoreError::Database("required schema view missing"));
        }
    }

    let trigger_rows = sqlx::query(
        "SELECT trigger_name \
         FROM information_schema.triggers \
         WHERE trigger_schema = current_schema()",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| PostgresStoreError::Database("failed to inspect schema triggers"))?;
    let triggers = trigger_rows
        .into_iter()
        .map(|row| row.try_get::<String, _>("trigger_name"))
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(|_| PostgresStoreError::Database("failed to decode schema triggers"))?;
    for trigger in REQUIRED_TRIGGERS {
        if !triggers.contains(*trigger) {
            return Err(PostgresStoreError::Database(
                "required schema trigger missing",
            ));
        }
    }

    let function_rows = sqlx::query(
        "SELECT p.proname \
         FROM pg_proc p \
         INNER JOIN pg_namespace n ON n.oid = p.pronamespace \
         WHERE n.nspname = current_schema()",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| PostgresStoreError::Database("failed to inspect schema functions"))?;
    let functions = function_rows
        .into_iter()
        .map(|row| row.try_get::<String, _>("proname"))
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(|_| PostgresStoreError::Database("failed to decode schema functions"))?;
    for function in REQUIRED_FUNCTIONS {
        if !functions.contains(*function) {
            return Err(PostgresStoreError::Database(
                "required schema function missing",
            ));
        }
    }

    let column_rows = sqlx::query(
        "SELECT column_name \
         FROM information_schema.columns \
         WHERE table_schema = current_schema() \
           AND table_name = 'run_observation_change_summaries'",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| PostgresStoreError::Database("failed to inspect read-model columns"))?;
    let columns = column_rows
        .into_iter()
        .map(|row| row.try_get::<String, _>("column_name"))
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(|_| PostgresStoreError::Database("failed to decode read-model columns"))?;
    for column in REQUIRED_OBSERVATION_SUMMARY_COLUMNS {
        if !columns.contains(*column) {
            return Err(PostgresStoreError::Database(
                "required read-model column missing",
            ));
        }
    }

    Ok(())
}

async fn validate_store_metadata(pool: &PgPool) -> Result<()> {
    let row = sqlx::query(
        "SELECT COUNT(*)::bigint AS row_count, \
          MIN(schema_contract_version) AS schema_contract_version, \
          MIN(octet_length(cursor_secret)) AS cursor_secret_len \
         FROM store_metadata WHERE singleton",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| PostgresStoreError::Database("failed to inspect store metadata"))?;
    let row_count: i64 = row
        .try_get("row_count")
        .map_err(|_| PostgresStoreError::Database("failed to decode store metadata"))?;
    let schema_contract_version: Option<String> = row
        .try_get("schema_contract_version")
        .map_err(|_| PostgresStoreError::Database("failed to decode store metadata"))?;
    let cursor_secret_len: Option<i32> = row
        .try_get("cursor_secret_len")
        .map_err(|_| PostgresStoreError::Database("failed to decode store metadata"))?;
    if row_count != 1
        || schema_contract_version.as_deref() != Some("mfm.postgres.run_store.v1")
        || cursor_secret_len != Some(32)
    {
        return Err(PostgresStoreError::Database("invalid store metadata"));
    }
    Ok(())
}

const REQUIRED_TABLES: &[&str] = &[
    "_sqlx_migrations",
    "store_metadata",
    "commits",
    "run_events",
    "artifact_blobs",
    "artifact_admissions",
    "commit_artifact_evidence",
    "run_artifact_admissions",
    "resource_lane_claim_events",
    "resource_lane_release_events",
    "resource_lane_transitions",
    "run_commit_log",
    "run_observation_change_summaries",
];

const REQUIRED_VIEWS: &[&str] = &["current_run_observations"];

const REQUIRED_FUNCTIONS: &[&str] = &["mfm_set_append_xid", "mfm_reject_authority_mutation"];

const REQUIRED_TRIGGERS: &[&str] = &[
    "commits_set_append_xid",
    "run_commit_log_set_append_xid",
    "store_metadata_no_update",
    "commits_no_update",
    "run_events_no_update",
    "artifact_blobs_no_update",
    "artifact_admissions_no_update",
    "commit_artifact_evidence_no_update",
    "run_artifact_admissions_no_update",
    "resource_lane_claim_events_no_update",
    "resource_lane_release_events_no_update",
    "resource_lane_transitions_no_update",
    "run_commit_log_no_update",
    "run_observation_change_summaries_no_update",
];

const REQUIRED_OBSERVATION_SUMMARY_COLUMNS: &[&str] = &[
    "projection_version",
    "commit_id",
    "summary_kind",
    "run_id",
    "head_seq",
    "observed_status",
    "source_authority_hash",
    "source_event_count",
    "summary_row_hash",
    "summary_row_canonical_json",
];

const FORBIDDEN_TABLES: &[&str] = &[
    "typed_run_heads",
    "typed_run_events",
    "typed_commit_keys",
    "typed_artifacts",
    "typed_run_artifacts",
    "typed_logical_keys",
    "typed_unique_logical_payloads",
    "typed_resource_lane_locks",
    "typed_retention_projection",
    "typed_retention_manifests",
    "typed_run_projection",
    "typed_run_completion_projection",
    "typed_saga_engagement_projection",
    "typed_manual_resolution_projection",
    "typed_attempt_projection",
    "typed_cell_projection",
    "typed_fact_projection",
    "typed_side_effect_projection",
    "typed_resource_lane_projection",
    "public_output_projection",
];
