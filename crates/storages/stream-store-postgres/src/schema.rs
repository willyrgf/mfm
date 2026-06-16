use std::collections::BTreeSet;

use sqlx::PgPool;

use crate::typed::{PostgresTypedStoreError, Result};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// PostgreSQL schema management for the typed run-event store.
pub struct PostgresSchema;

impl PostgresSchema {
    /// Applies all pending typed run-event store migrations.
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
        .map_err(|_| PostgresTypedStoreError::Database("migration failed"))?;
    Ok(())
}

pub(crate) async fn connect_pool(database_url: &str) -> Result<PgPool> {
    PgPool::connect(database_url)
        .await
        .map_err(|_| PostgresTypedStoreError::Database("connect failed"))
}

pub(crate) async fn validate_pool(pool: &PgPool) -> Result<()> {
    validate_migrations(pool).await?;
    validate_catalog(pool).await
}

fn database_url_env() -> Result<String> {
    std::env::var("DATABASE_URL")
        .map_err(|_| PostgresTypedStoreError::Database("missing DATABASE_URL"))
}

async fn validate_migrations(pool: &PgPool) -> Result<()> {
    let rows =
        sqlx::query!("SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .map_err(|_| PostgresTypedStoreError::Database("schema migrations missing"))?;

    if rows.len() != MIGRATOR.iter().count() {
        return Err(PostgresTypedStoreError::Database(
            "schema migration count mismatch",
        ));
    }

    for migration in MIGRATOR.iter() {
        let row = rows
            .iter()
            .find(|row| row.version == migration.version)
            .ok_or(PostgresTypedStoreError::Database(
                "schema migration missing",
            ))?;
        if !row.success {
            return Err(PostgresTypedStoreError::Database("schema migration failed"));
        }
        if row.checksum.as_slice() != migration.checksum.as_ref() {
            return Err(PostgresTypedStoreError::Database(
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
    .map_err(|_| PostgresTypedStoreError::Database("failed to inspect schema tables"))?;
    let tables = table_rows
        .into_iter()
        .map(|row| row.table_name)
        .collect::<BTreeSet<_>>();

    for table in REQUIRED_TABLES {
        if !tables.contains(*table) {
            return Err(PostgresTypedStoreError::Database(
                "required schema table missing",
            ));
        }
    }
    for table in FORBIDDEN_TABLES {
        if tables.contains(*table) {
            return Err(PostgresTypedStoreError::Database(
                "stale schema table present",
            ));
        }
    }

    let index_exists = sqlx::query_scalar!(
        "SELECT EXISTS ( \
           SELECT 1 \
           FROM pg_indexes \
           WHERE schemaname = current_schema() \
             AND tablename = 'typed_resource_lane_projection' \
             AND indexname = 'typed_resource_lane_projection_run_idx' \
         )"
    )
    .fetch_one(pool)
    .await
    .map_err(|_| PostgresTypedStoreError::Database("failed to inspect schema indexes"))?;
    if index_exists != Some(true) {
        return Err(PostgresTypedStoreError::Database(
            "required schema index missing",
        ));
    }

    Ok(())
}

const REQUIRED_TABLES: &[&str] = &[
    "_sqlx_migrations",
    "typed_run_heads",
    "typed_run_events",
    "typed_commit_keys",
    "typed_artifacts",
    "typed_run_artifacts",
    "typed_logical_keys",
    "typed_unique_logical_payloads",
    "typed_run_projection",
    "typed_run_completion_projection",
    "typed_saga_engagement_projection",
    "typed_manual_resolution_projection",
    "typed_attempt_projection",
    "typed_cell_projection",
    "typed_fact_projection",
    "typed_side_effect_projection",
    "typed_resource_lane_projection",
    "typed_public_output_projection",
];

const FORBIDDEN_TABLES: &[&str] = &["typed_retention_projection", "typed_retention_manifests"];
