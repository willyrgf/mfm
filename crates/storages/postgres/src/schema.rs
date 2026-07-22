use std::collections::BTreeSet;

use mfm_store::v1::StoreScopeId;
use sqlx::{PgPool, Row};

use crate::run_store::{
    PostgresStoreAuthority, PostgresStoreAuthorityError, PostgresStoreError, Result,
};

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

    /// Verifies store authority and returns the validated Postgres run-store authority.
    pub async fn validate(database_url: &str) -> Result<PostgresStoreAuthority> {
        let pool = connect_pool(database_url)
            .await
            .map_err(|_| PostgresStoreError::Authority(PostgresStoreAuthorityError::Connection))?;
        validate_pool(&pool).await
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

pub(crate) async fn validate_pool(pool: &PgPool) -> Result<PostgresStoreAuthority> {
    validate_migrations(pool).await?;
    validate_catalog(pool).await?;
    validate_store_metadata(pool).await
}

async fn validate_migrations(pool: &PgPool) -> Result<()> {
    let rows =
        sqlx::query!("SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .map_err(|_| authority_mismatch())?;

    if rows.len() != MIGRATOR.iter().count() {
        return Err(authority_mismatch());
    }

    for migration in MIGRATOR.iter() {
        let row = rows
            .iter()
            .find(|row| row.version == migration.version)
            .ok_or_else(authority_mismatch)?;
        if !row.success {
            return Err(authority_mismatch());
        }
        if row.checksum.as_slice() != migration.checksum.as_ref() {
            return Err(store_authority_error(
                PostgresStoreAuthorityError::MigrationChecksumMismatch,
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
    .map_err(|_| authority_mismatch())?;
    let tables = table_rows
        .into_iter()
        .map(|row| row.table_name)
        .collect::<BTreeSet<_>>();

    for table in REQUIRED_TABLES {
        if !tables.contains(*table) {
            return Err(authority_mismatch());
        }
    }
    for table in FORBIDDEN_TABLES {
        if tables.contains(*table) {
            return Err(authority_mismatch());
        }
    }

    let view_rows = sqlx::query(
        "SELECT table_name \
         FROM information_schema.views \
         WHERE table_schema = current_schema()",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| authority_mismatch())?;
    let views = view_rows
        .into_iter()
        .map(|row| row.try_get::<String, _>("table_name"))
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(|_| authority_mismatch())?;
    for view in REQUIRED_VIEWS {
        if !views.contains(*view) {
            return Err(authority_mismatch());
        }
    }

    let index_rows = sqlx::query(
        "SELECT indexname \
         FROM pg_indexes \
         WHERE schemaname = current_schema()",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| authority_mismatch())?;
    let indexes = index_rows
        .into_iter()
        .map(|row| row.try_get::<String, _>("indexname"))
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(|_| authority_mismatch())?;
    for index in REQUIRED_INDEXES {
        if !indexes.contains(*index) {
            return Err(authority_mismatch());
        }
    }

    let trigger_rows = sqlx::query(
        "SELECT trigger_name \
         FROM information_schema.triggers \
         WHERE trigger_schema = current_schema()",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| authority_mismatch())?;
    let triggers = trigger_rows
        .into_iter()
        .map(|row| row.try_get::<String, _>("trigger_name"))
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(|_| authority_mismatch())?;
    for trigger in REQUIRED_TRIGGERS {
        if !triggers.contains(*trigger) {
            return Err(authority_mismatch());
        }
    }
    validate_trigger_contracts(pool).await?;

    let function_rows = sqlx::query(
        "SELECT p.proname \
         FROM pg_proc p \
         INNER JOIN pg_namespace n ON n.oid = p.pronamespace \
         WHERE n.nspname = current_schema()",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| authority_mismatch())?;
    let functions = function_rows
        .into_iter()
        .map(|row| row.try_get::<String, _>("proname"))
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(|_| authority_mismatch())?;
    for function in REQUIRED_FUNCTIONS {
        if !functions.contains(*function) {
            return Err(authority_mismatch());
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
    .map_err(|_| authority_mismatch())?;
    let constraints = constraint_rows
        .into_iter()
        .map(|row| row.try_get::<String, _>("constraint_name"))
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(|_| authority_mismatch())?;
    for constraint in REQUIRED_CONSTRAINTS {
        if !constraints.contains(*constraint) {
            return Err(authority_mismatch());
        }
    }

    let cursor_column_rows = sqlx::query(
        "SELECT column_name \
         FROM information_schema.columns \
         WHERE table_schema = current_schema() \
           AND table_name = 'run_observation_cursors'",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| authority_mismatch())?;
    let cursor_columns = cursor_column_rows
        .into_iter()
        .map(|row| row.try_get::<String, _>("column_name"))
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(|_| authority_mismatch())?;
    for column in REQUIRED_CURSOR_COLUMNS {
        if !cursor_columns.contains(*column) {
            return Err(authority_mismatch());
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
    .map_err(|_| authority_mismatch())?;

    for contract in REQUIRED_FUNCTION_CONTRACTS {
        let row = rows
            .iter()
            .find(|row| row.try_get::<String, _>("proname").ok().as_deref() == Some(contract.name))
            .ok_or_else(authority_mismatch)?;
        let result_type: String = row
            .try_get("result_type")
            .map_err(|_| authority_mismatch())?;
        let arguments: String = row.try_get("arguments").map_err(|_| authority_mismatch())?;
        let definition: String = row
            .try_get("definition")
            .map_err(|_| authority_mismatch())?;
        if result_type != "trigger" || !arguments.is_empty() {
            return Err(authority_mismatch());
        }
        for snippet in contract.required_definition_snippets {
            if !definition.contains(snippet) {
                return Err(authority_mismatch());
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
    .map_err(|_| authority_mismatch())?;

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
        .ok_or_else(authority_mismatch)?;
    let table_name: String = row
        .try_get("table_name")
        .map_err(|_| authority_mismatch())?;
    let function_name: String = row
        .try_get("function_name")
        .map_err(|_| authority_mismatch())?;
    let trigger_type: i32 = row
        .try_get("trigger_type")
        .map_err(|_| authority_mismatch())?;
    let trigger_enabled: String = row
        .try_get("trigger_enabled")
        .map_err(|_| authority_mismatch())?;

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
        return Err(authority_mismatch());
    }

    Ok(())
}

fn trigger_type_has(trigger_type: i32, bit: i32) -> bool {
    trigger_type & bit == bit
}

async fn validate_store_metadata(pool: &PgPool) -> Result<PostgresStoreAuthority> {
    let row = sqlx::query(
        "SELECT COUNT(*)::bigint AS row_count, \
          MIN(store_epoch) AS store_epoch, \
          MIN(store_scope_id) AS store_scope_id, \
          MIN(schema_contract_version) AS schema_contract_version \
         FROM store_metadata WHERE singleton",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| authority_mismatch())?;
    let row_count: i64 = row.try_get("row_count").map_err(|_| authority_mismatch())?;
    let store_epoch: Option<String> = row
        .try_get("store_epoch")
        .map_err(|_| authority_mismatch())?;
    let schema_contract_version: Option<String> = row
        .try_get("schema_contract_version")
        .map_err(|_| authority_mismatch())?;
    let store_scope_id: Option<String> = row
        .try_get("store_scope_id")
        .map_err(|_| authority_mismatch())?;
    if row_count != 1
        || schema_contract_version.as_deref() != Some("mfm.postgres.store.v4")
        || !valid_store_epoch(store_epoch.as_deref())
    {
        return Err(authority_mismatch());
    }
    let store_scope_id = store_scope_id
        .ok_or_else(authority_mismatch)
        .and_then(|value| StoreScopeId::new(value).map_err(|_| authority_mismatch()))?;
    Ok(PostgresStoreAuthority::new(store_scope_id))
}

fn valid_store_epoch(value: Option<&str>) -> bool {
    const PREFIX: &str = "mfm.store.epoch.v1:";
    let Some(value) = value else {
        return false;
    };
    let Some(hex) = value.strip_prefix(PREFIX) else {
        return false;
    };
    hex.len() == 32 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn store_authority_error(kind: PostgresStoreAuthorityError) -> PostgresStoreError {
    PostgresStoreError::Authority(kind)
}

fn authority_mismatch() -> PostgresStoreError {
    store_authority_error(PostgresStoreAuthorityError::StoreAuthorityMismatch)
}

const REQUIRED_TABLES: &[&str] = &[
    "_sqlx_migrations",
    "store_metadata",
    "store_commit_order",
    "commits",
    "run_events",
    "artifact_blobs",
    "artifact_admissions",
    "commit_artifact_evidence",
    "run_artifact_admissions",
    "fact_descriptor_catalog",
    "run_fact_descriptor_admissions",
    "fact_query_projection",
    "fact_query_terms",
    "fact_query_metadata",
    "admission_lane",
    "admission_waiter",
    "run_observation_cursors",
    "configured_values",
];

const REQUIRED_VIEWS: &[&str] = &[];

const REQUIRED_INDEXES: &[&str] = &[
    "admission_lane_expired_execution_claim_idx",
    "admission_waiter_live_fifo_idx",
    "admission_waiter_waiting_expiry_idx",
    "fact_descriptor_catalog_kind_idx",
    "fact_query_projection_descriptor_order_idx",
    "fact_query_projection_fact_key_idx",
    "fact_query_projection_response_artifact_idx",
    "fact_query_terms_text_idx",
    "fact_query_terms_bool_idx",
    "fact_query_terms_i64_idx",
    "fact_query_terms_u64_idx",
    "fact_query_terms_decimal_idx",
    "fact_query_terms_timestamp_idx",
    "fact_query_terms_digest_idx",
];

const REQUIRED_FUNCTIONS: &[&str] = &["mfm_reject_authority_mutation"];

const REQUIRED_TRIGGERS: &[&str] = &[
    "store_metadata_no_update",
    "commits_no_update",
    "run_events_no_update",
    "artifact_blobs_no_update",
    "artifact_admissions_no_update",
    "commit_artifact_evidence_no_update",
    "run_artifact_admissions_no_update",
    "run_observation_cursors_no_update",
];

const REQUIRED_CONSTRAINTS: &[&str] = &[
    "artifact_blobs_byte_len_max",
    "commits_store_commit_order_positive",
    "commits_store_commit_order_key",
    "store_commit_order_nonnegative",
    "store_metadata_store_scope_id_v1",
    "admission_lane_id_len",
    "admission_lane_class_v1",
    "admission_lane_mode_v1",
    "admission_lane_class_mode_v1",
    "admission_lane_wait_fifo_shape",
    "admission_lane_nowait_skip_shape",
    "admission_lane_execution_run_id_shape",
    "admission_lane_execution_run_id_nonempty",
    "admission_waiter_lane_fk",
    "admission_waiter_resource_wait_fifo_v1",
    "admission_waiter_lane_id_len",
    "admission_waiter_lane_ticket_positive",
    "admission_waiter_id_nonempty",
    "admission_waiter_token_nonempty",
    "admission_waiter_status_v1",
    "fact_descriptor_catalog_artifact_fk",
    "fact_descriptor_catalog_artifact_unique",
    "fact_descriptor_catalog_descriptor_artifact_unique",
    "run_fact_descriptor_admissions_descriptor_artifact_fk",
    "run_fact_descriptor_admissions_event_fk",
    "run_fact_descriptor_admissions_event_id_fk",
    "run_fact_descriptor_admissions_commit_fk",
    "run_fact_descriptor_admissions_run_artifact_fk",
    "run_fact_descriptor_admissions_seq_positive",
    "run_fact_descriptor_admissions_ordinal_nonnegative",
    "fact_query_projection_event_fk",
    "fact_query_projection_event_id_fk",
    "fact_query_projection_commit_fk",
    "fact_query_projection_descriptor_fk",
    "fact_query_projection_response_artifact_fk",
    "fact_query_projection_seq_positive",
    "fact_query_projection_ordinal_nonnegative",
    "fact_query_projection_store_commit_order_positive",
    "fact_query_terms_claim_fk",
    "fact_query_terms_descriptor_fk",
    "fact_query_terms_seq_positive",
    "fact_query_terms_ordinal_nonnegative",
    "fact_query_terms_source_v1",
    "fact_query_terms_value_type_v1",
    "fact_query_terms_u64_range",
    "fact_query_terms_value_shape",
    "fact_query_metadata_generation_positive",
    "run_observation_cursors_version_v3",
    "run_observation_cursors_store_commit_order_nonnegative",
    "configured_values_target_bounds",
    "configured_values_schema_id_bounds",
    "configured_values_digest_bounds",
    "configured_values_canonical_json_bounds",
];

const REQUIRED_CURSOR_COLUMNS: &[&str] = &[
    "token_hash",
    "cursor_version",
    "store_epoch",
    "store_commit_order",
    "issued_at",
];

const IMMUTABLE_TABLES: &[&str] = &[
    "store_metadata",
    "commits",
    "run_events",
    "artifact_blobs",
    "artifact_admissions",
    "commit_artifact_evidence",
    "run_artifact_admissions",
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

const REQUIRED_FUNCTION_CONTRACTS: &[FunctionContract<'_>] = &[FunctionContract {
    name: "mfm_reject_authority_mutation",
    required_definition_snippets: &["RAISE EXCEPTION", "mfm authority tables are append-only"],
}];

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

const REQUIRED_TRIGGER_CONTRACTS: &[TriggerContract<'_>] = &[];

const FORBIDDEN_TABLES: &[&str] = &[
    "catalog_values",
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
    "fact_projection",
    "fact_projections",
    "fact_records",
    "fact_terms",
    "fact_descriptor_index",
    "fact_index",
    "fact_index_terms",
    "fact_projection_metadata",
    "typed_side_effect_projection",
    "typed_resource_lane_projection",
    "public_output_projection",
    "resource_lane_waiter_counters",
    "resource_lane_waiters",
];
