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
    validate_trigger_contracts(pool).await?;
    validate_append_xid_columns(pool).await?;

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
    validate_function_contracts(pool).await?;

    let constraint_rows = sqlx::query(
        "SELECT constraint_name \
         FROM information_schema.table_constraints \
         WHERE table_schema = current_schema()",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| PostgresStoreError::Database("failed to inspect schema constraints"))?;
    let constraints = constraint_rows
        .into_iter()
        .map(|row| row.try_get::<String, _>("constraint_name"))
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(|_| PostgresStoreError::Database("failed to decode schema constraints"))?;
    for constraint in REQUIRED_CONSTRAINTS {
        if !constraints.contains(*constraint) {
            return Err(PostgresStoreError::Database(
                "required schema constraint missing",
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

async fn validate_function_contracts(pool: &PgPool) -> Result<()> {
    let rows = sqlx::query(
        "SELECT p.proname, pg_get_function_result(p.oid) AS result_type, \
          pg_get_function_arguments(p.oid) AS arguments, pg_get_functiondef(p.oid) AS definition \
         FROM pg_proc p \
         INNER JOIN pg_namespace n ON n.oid = p.pronamespace \
         WHERE n.nspname = current_schema()",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| PostgresStoreError::Database("failed to inspect schema function contracts"))?;

    for contract in REQUIRED_FUNCTION_CONTRACTS {
        let row = rows
            .iter()
            .find(|row| row.try_get::<String, _>("proname").ok().as_deref() == Some(contract.name))
            .ok_or(PostgresStoreError::Database(
                "required schema function missing",
            ))?;
        let result_type: String = row
            .try_get("result_type")
            .map_err(|_| PostgresStoreError::Database("failed to decode schema function result"))?;
        let arguments: String = row.try_get("arguments").map_err(|_| {
            PostgresStoreError::Database("failed to decode schema function arguments")
        })?;
        let definition: String = row.try_get("definition").map_err(|_| {
            PostgresStoreError::Database("failed to decode schema function definition")
        })?;
        if result_type != "trigger" || !arguments.is_empty() {
            return Err(PostgresStoreError::Database(
                "required schema function contract mismatch",
            ));
        }
        for snippet in contract.required_definition_snippets {
            if !definition.contains(snippet) {
                return Err(PostgresStoreError::Database(
                    "required schema function contract mismatch",
                ));
            }
        }
    }

    Ok(())
}

async fn validate_trigger_contracts(pool: &PgPool) -> Result<()> {
    let rows = sqlx::query(
        "SELECT t.tgname, c.relname AS table_name, p.proname AS function_name, \
          t.tgtype::int AS trigger_type, t.tgenabled::text AS trigger_enabled \
         FROM pg_trigger t \
         INNER JOIN pg_class c ON c.oid = t.tgrelid \
         INNER JOIN pg_namespace n ON n.oid = c.relnamespace \
         INNER JOIN pg_proc p ON p.oid = t.tgfoid \
         WHERE n.nspname = current_schema() AND NOT t.tgisinternal",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| PostgresStoreError::Database("failed to inspect schema trigger contracts"))?;

    for contract in REQUIRED_TRIGGER_CONTRACTS {
        validate_trigger_contract_row(&rows, contract)?;
    }

    for table in IMMUTABLE_TABLES {
        let trigger_name = format!("{table}_no_update");
        validate_trigger_contract_row(
            &rows,
            &TriggerContract {
                name: &trigger_name,
                table,
                function: "mfm_reject_authority_mutation",
                row_level: false,
                before: true,
                insert: false,
                update: true,
                delete: true,
                truncate: true,
            },
        )?;
    }

    Ok(())
}

fn validate_trigger_contract_row(
    rows: &[sqlx::postgres::PgRow],
    contract: &TriggerContract<'_>,
) -> Result<()> {
    let row = rows
        .iter()
        .find(|row| row.try_get::<String, _>("tgname").ok().as_deref() == Some(contract.name))
        .ok_or(PostgresStoreError::Database(
            "required schema trigger missing",
        ))?;
    let table_name: String = row
        .try_get("table_name")
        .map_err(|_| PostgresStoreError::Database("failed to decode schema trigger table"))?;
    let function_name: String = row
        .try_get("function_name")
        .map_err(|_| PostgresStoreError::Database("failed to decode schema trigger function"))?;
    let trigger_type: i32 = row
        .try_get("trigger_type")
        .map_err(|_| PostgresStoreError::Database("failed to decode schema trigger type"))?;
    let trigger_enabled: String = row
        .try_get("trigger_enabled")
        .map_err(|_| PostgresStoreError::Database("failed to decode schema trigger enabled"))?;

    if table_name != contract.table
        || function_name != contract.function
        || trigger_enabled != "O"
        || trigger_type_has(trigger_type, TRIGGER_TYPE_ROW) != contract.row_level
        || trigger_type_has(trigger_type, TRIGGER_TYPE_BEFORE) != contract.before
        || trigger_type_has(trigger_type, TRIGGER_TYPE_INSERT) != contract.insert
        || trigger_type_has(trigger_type, TRIGGER_TYPE_UPDATE) != contract.update
        || trigger_type_has(trigger_type, TRIGGER_TYPE_DELETE) != contract.delete
        || trigger_type_has(trigger_type, TRIGGER_TYPE_TRUNCATE) != contract.truncate
    {
        return Err(PostgresStoreError::Database(
            "required schema trigger contract mismatch",
        ));
    }

    Ok(())
}

fn trigger_type_has(trigger_type: i32, bit: i32) -> bool {
    trigger_type & bit == bit
}

async fn validate_append_xid_columns(pool: &PgPool) -> Result<()> {
    let rows = sqlx::query(
        "SELECT table_name, column_default \
         FROM information_schema.columns \
         WHERE table_schema = current_schema() \
           AND column_name = 'append_xid' \
           AND table_name IN ('commits', 'run_commit_log')",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| PostgresStoreError::Database("failed to inspect append xid columns"))?;
    for table in APPEND_XID_TABLES {
        let row = rows
            .iter()
            .find(|row| row.try_get::<String, _>("table_name").ok().as_deref() == Some(*table))
            .ok_or(PostgresStoreError::Database(
                "required append_xid column missing",
            ))?;
        let default: Option<String> = row
            .try_get("column_default")
            .map_err(|_| PostgresStoreError::Database("failed to decode append xid default"))?;
        if default.as_deref() != Some("pg_current_xact_id()") {
            return Err(PostgresStoreError::Database(
                "required append_xid default mismatch",
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
    "observation_derivations",
    "observation_derivation_sources",
    "run_observation_cursors",
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
    "observation_derivations_no_update",
    "observation_derivation_sources_no_update",
    "run_observation_cursors_no_update",
];

const REQUIRED_CONSTRAINTS: &[&str] = &["artifact_blobs_byte_len_max"];

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

const APPEND_XID_TABLES: &[&str] = &["commits", "run_commit_log"];

const IMMUTABLE_TABLES: &[&str] = &[
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
    "observation_derivations",
    "observation_derivation_sources",
    "run_observation_cursors",
];

const TRIGGER_TYPE_ROW: i32 = 1;
const TRIGGER_TYPE_BEFORE: i32 = 2;
const TRIGGER_TYPE_INSERT: i32 = 4;
const TRIGGER_TYPE_DELETE: i32 = 8;
const TRIGGER_TYPE_UPDATE: i32 = 16;
const TRIGGER_TYPE_TRUNCATE: i32 = 32;

struct FunctionContract<'a> {
    name: &'a str,
    required_definition_snippets: &'a [&'a str],
}

const REQUIRED_FUNCTION_CONTRACTS: &[FunctionContract<'_>] = &[
    FunctionContract {
        name: "mfm_set_append_xid",
        required_definition_snippets: &["NEW.append_xid", "pg_current_xact_id()"],
    },
    FunctionContract {
        name: "mfm_reject_authority_mutation",
        required_definition_snippets: &["RAISE EXCEPTION", "mfm authority tables are append-only"],
    },
];

struct TriggerContract<'a> {
    name: &'a str,
    table: &'a str,
    function: &'a str,
    row_level: bool,
    before: bool,
    insert: bool,
    update: bool,
    delete: bool,
    truncate: bool,
}

const REQUIRED_TRIGGER_CONTRACTS: &[TriggerContract<'_>] = &[
    TriggerContract {
        name: "commits_set_append_xid",
        table: "commits",
        function: "mfm_set_append_xid",
        row_level: true,
        before: true,
        insert: true,
        update: false,
        delete: false,
        truncate: false,
    },
    TriggerContract {
        name: "run_commit_log_set_append_xid",
        table: "run_commit_log",
        function: "mfm_set_append_xid",
        row_level: true,
        before: true,
        insert: true,
        update: false,
        delete: false,
        truncate: false,
    },
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
