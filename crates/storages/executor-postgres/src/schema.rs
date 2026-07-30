use std::collections::{BTreeMap, BTreeSet};

use sqlx::{PgConnection, PgPool, Row};

use crate::{PostgresExecutorStoreError, Result};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub(crate) const APPLICATION_ROLE: &str = "mfm_executor_application";
pub(crate) const SCHEMA_CONTRACT_VERSION: &str = "mfm.executor-postgres.v2";

/// Administrative migration surface for the dedicated executor schema.
pub struct PostgresExecutorSchema;

impl PostgresExecutorSchema {
    /// Applies the compiled baseline using a migration-owner pool whose current schema is reserved
    /// exclusively for the executor ledger.
    pub async fn migrate(pool: &PgPool) -> Result<()> {
        require_dedicated_schema(pool).await?;
        MIGRATOR
            .run(pool)
            .await
            .map_err(|_| PostgresExecutorStoreError::Database)?;
        validate_authoritative_schema(pool).await
    }
}

async fn require_dedicated_schema(pool: &PgPool) -> Result<()> {
    let row = sqlx::query(
        "SELECT current_schema()::text AS schema_name, \
                count(*) FILTER ( \
                    WHERE relation.relname NOT IN ( \
                        '_sqlx_migrations', \
                        'executor_schema_metadata', \
                        'executor_bindings', \
                        'executor_content_records', \
                        'executor_effect_frontiers', \
                        'executor_effect_resource_links', \
                        'executor_resource_records', \
                        'executor_effect_heads', \
                        'executor_resource_heads' \
                    ) \
                ) AS foreign_relation_count \
           FROM pg_catalog.pg_class AS relation \
           JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
          WHERE namespace.nspname = current_schema() \
            AND relation.relkind IN ('r', 'p', 'v', 'm', 'f')",
    )
    .fetch_one(pool)
    .await
    .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
    let schema_name = row
        .try_get::<String, _>("schema_name")
        .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
    let foreign_relation_count = row
        .try_get::<i64, _>("foreign_relation_count")
        .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
    if matches!(
        schema_name.as_str(),
        "public" | "pg_catalog" | "information_schema"
    ) || foreign_relation_count != 0
    {
        return Err(PostgresExecutorStoreError::SchemaAuthorityMismatch);
    }
    Ok(())
}

pub(crate) async fn validate_authoritative_schema(pool: &PgPool) -> Result<()> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
    let connection = &mut *transaction;
    validate_migrations(connection).await?;
    validate_relations(connection).await?;
    validate_columns(connection).await?;
    validate_indexes(connection).await?;
    validate_triggers(connection).await?;
    validate_roles_and_privileges(connection).await?;
    validate_metadata(connection).await?;
    transaction
        .commit()
        .await
        .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)
}

async fn validate_migrations(connection: &mut PgConnection) -> Result<()> {
    let rows =
        sqlx::query("SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&mut *connection)
            .await
            .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
    if rows.len() != MIGRATOR.iter().count() {
        return Err(PostgresExecutorStoreError::SchemaAuthorityMismatch);
    }
    for migration in MIGRATOR.iter() {
        let row = rows
            .iter()
            .find(|row| row.try_get::<i64, _>("version").ok() == Some(migration.version))
            .ok_or(PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
        let success = row
            .try_get::<bool, _>("success")
            .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
        let checksum = row
            .try_get::<Vec<u8>, _>("checksum")
            .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
        if !success || checksum.as_slice() != migration.checksum.as_ref() {
            return Err(PostgresExecutorStoreError::MigrationChecksumMismatch);
        }
    }
    Ok(())
}

async fn validate_relations(connection: &mut PgConnection) -> Result<()> {
    let rows = sqlx::query(
        "SELECT table_name, table_type \
           FROM information_schema.tables \
          WHERE table_schema = current_schema() \
            AND table_name <> '_sqlx_migrations'",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
    let actual = rows
        .into_iter()
        .map(|row| {
            Ok((
                row.try_get::<String, _>("table_name")
                    .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?,
                row.try_get::<String, _>("table_type")
                    .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let expected = [
        ("executor_bindings", "BASE TABLE"),
        ("executor_content_records", "BASE TABLE"),
        ("executor_effect_frontiers", "BASE TABLE"),
        ("executor_effect_heads", "VIEW"),
        ("executor_effect_resource_links", "BASE TABLE"),
        ("executor_resource_heads", "VIEW"),
        ("executor_resource_records", "BASE TABLE"),
        ("executor_schema_metadata", "BASE TABLE"),
    ]
    .into_iter()
    .map(|(name, kind)| (name.to_owned(), kind.to_owned()))
    .collect::<BTreeMap<_, _>>();
    if actual != expected {
        return Err(PostgresExecutorStoreError::SchemaAuthorityMismatch);
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct ColumnShape {
    table: String,
    name: String,
    data_type: String,
    nullable: bool,
}

type ExpectedColumn = (&'static str, &'static str, bool);
type ExpectedTableColumns = (&'static str, &'static [ExpectedColumn]);

async fn validate_columns(connection: &mut PgConnection) -> Result<()> {
    let rows = sqlx::query(
        "SELECT table_name, column_name, udt_name, is_nullable \
           FROM information_schema.columns \
          WHERE table_schema = current_schema() \
            AND table_name LIKE 'executor_%' \
          ORDER BY table_name, ordinal_position",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
    let actual = rows
        .into_iter()
        .map(|row| {
            Ok(ColumnShape {
                table: row
                    .try_get("table_name")
                    .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?,
                name: row
                    .try_get("column_name")
                    .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?,
                data_type: row
                    .try_get("udt_name")
                    .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?,
                nullable: row
                    .try_get::<String, _>("is_nullable")
                    .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?
                    == "YES",
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let expected = expected_columns();
    if actual != expected {
        return Err(PostgresExecutorStoreError::SchemaAuthorityMismatch);
    }
    Ok(())
}

fn expected_columns() -> Vec<ColumnShape> {
    let contracts: &[ExpectedTableColumns] = &[
        (
            "executor_bindings",
            &[
                ("singleton", "bool", false),
                ("tenant_scope_id", "text", false),
                ("executor_binding_schema_id", "text", false),
                ("executor_binding_content_digest", "text", false),
                ("durable_generation_schema_id", "text", false),
                ("durable_generation_content_digest", "text", false),
                ("evidence_authority_schema_id", "text", false),
                ("evidence_authority_content_digest", "text", false),
                ("resource_ownership_schema_id", "text", true),
                ("resource_ownership_content_digest", "text", true),
            ],
        ),
        (
            "executor_content_records",
            &[
                ("schema_id", "text", false),
                ("content_digest", "text", false),
                ("canonical_bytes", "bytea", false),
            ],
        ),
        (
            "executor_effect_frontiers",
            &[
                ("effect_key", "text", false),
                ("ordinal", "int8", false),
                ("frontier_schema_id", "text", false),
                ("frontier_content_digest", "text", false),
                ("predecessor_schema_id", "text", true),
                ("predecessor_content_digest", "text", true),
                ("durable_payload", "bytea", false),
            ],
        ),
        (
            "executor_effect_heads",
            &[
                ("effect_key", "text", true),
                ("ordinal", "int8", true),
                ("frontier_schema_id", "text", true),
                ("frontier_content_digest", "text", true),
            ],
        ),
        (
            "executor_effect_resource_links",
            &[
                ("effect_key", "text", false),
                ("effect_frontier_schema_id", "text", false),
                ("effect_frontier_content_digest", "text", false),
                ("resource_ownership_schema_id", "text", false),
                ("resource_ownership_content_digest", "text", false),
                ("resource_key_schema_id", "text", false),
                ("resource_key_content_digest", "text", false),
                ("resource_record_schema_id", "text", false),
                ("resource_record_content_digest", "text", false),
            ],
        ),
        (
            "executor_resource_heads",
            &[
                ("resource_ownership_schema_id", "text", true),
                ("resource_ownership_content_digest", "text", true),
                ("resource_key_schema_id", "text", true),
                ("resource_key_content_digest", "text", true),
                ("ordinal", "int8", true),
                ("record_schema_id", "text", true),
                ("record_content_digest", "text", true),
            ],
        ),
        (
            "executor_resource_records",
            &[
                ("resource_ownership_schema_id", "text", false),
                ("resource_ownership_content_digest", "text", false),
                ("resource_key_schema_id", "text", false),
                ("resource_key_content_digest", "text", false),
                ("ordinal", "int8", false),
                ("record_schema_id", "text", false),
                ("record_content_digest", "text", false),
                ("predecessor_schema_id", "text", true),
                ("predecessor_content_digest", "text", true),
                ("linked_effect_key", "text", false),
                ("durable_payload", "bytea", false),
            ],
        ),
        (
            "executor_schema_metadata",
            &[
                ("singleton", "bool", false),
                ("schema_contract_version", "text", false),
            ],
        ),
    ];
    contracts
        .iter()
        .flat_map(|(table, columns)| {
            columns
                .iter()
                .map(|(name, data_type, nullable)| ColumnShape {
                    table: (*table).to_owned(),
                    name: (*name).to_owned(),
                    data_type: (*data_type).to_owned(),
                    nullable: *nullable,
                })
        })
        .collect()
}

async fn validate_indexes(connection: &mut PgConnection) -> Result<()> {
    let rows = sqlx::query(
        "SELECT indexname \
           FROM pg_catalog.pg_indexes \
          WHERE schemaname = current_schema() \
            AND tablename LIKE 'executor_%'",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
    let actual = rows
        .into_iter()
        .map(|row| {
            row.try_get::<String, _>("indexname")
                .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)
        })
        .collect::<Result<BTreeSet<_>>>()?;
    let expected = [
        "executor_bindings_pkey",
        "executor_content_records_pkey",
        "executor_effect_frontiers_pkey",
        "executor_effect_frontiers_ref_key",
        "executor_effect_resource_links_pkey",
        "executor_resource_records_linked_effect_idx",
        "executor_resource_records_pkey",
        "executor_resource_records_ref_key",
        "executor_schema_metadata_pkey",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(PostgresExecutorStoreError::SchemaAuthorityMismatch);
    }
    Ok(())
}

async fn validate_triggers(connection: &mut PgConnection) -> Result<()> {
    let rows = sqlx::query(
        "SELECT trigger.tgname, trigger.tgenabled::text AS enabled, \
                procedure.proname AS function_name \
           FROM pg_catalog.pg_trigger AS trigger \
           JOIN pg_catalog.pg_class AS relation ON relation.oid = trigger.tgrelid \
           JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
           JOIN pg_catalog.pg_proc AS procedure ON procedure.oid = trigger.tgfoid \
          WHERE namespace.nspname = current_schema() \
            AND NOT trigger.tgisinternal",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
    let mut actual = BTreeSet::new();
    for row in rows {
        if row
            .try_get::<String, _>("enabled")
            .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?
            != "O"
        {
            return Err(PostgresExecutorStoreError::SchemaAuthorityMismatch);
        }
        let function_name = row
            .try_get::<String, _>("function_name")
            .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
        if !matches!(
            function_name.as_str(),
            "mfm_executor_reject_authority_mutation" | "mfm_executor_reject_metadata_mutation"
        ) {
            return Err(PostgresExecutorStoreError::SchemaAuthorityMismatch);
        }
        actual.insert(
            row.try_get::<String, _>("tgname")
                .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?,
        );
    }
    let expected = [
        "executor_bindings_no_update",
        "executor_content_records_no_update",
        "executor_effect_frontiers_no_update",
        "executor_effect_resource_links_no_update",
        "executor_resource_records_no_update",
        "executor_schema_metadata_no_mutation",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(PostgresExecutorStoreError::SchemaAuthorityMismatch);
    }
    Ok(())
}

async fn validate_roles_and_privileges(connection: &mut PgConnection) -> Result<()> {
    let role_rows = sqlx::query(
        "SELECT rolname, rolsuper, rolinherit, rolcreaterole, rolcreatedb, \
                rolcanlogin, rolreplication, rolbypassrls \
           FROM pg_catalog.pg_roles \
          WHERE rolname IN ('mfm_executor_owner', 'mfm_executor_application')",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
    if role_rows.len() != 2 {
        return Err(PostgresExecutorStoreError::SchemaAuthorityMismatch);
    }
    for row in role_rows {
        if required_bool(&row, "rolsuper")?
            || !required_bool(&row, "rolinherit")?
            || required_bool(&row, "rolcreaterole")?
            || required_bool(&row, "rolcreatedb")?
            || required_bool(&row, "rolcanlogin")?
            || required_bool(&row, "rolreplication")?
            || required_bool(&row, "rolbypassrls")?
        {
            return Err(PostgresExecutorStoreError::SchemaAuthorityMismatch);
        }
    }

    let owner_rows = sqlx::query(
        "SELECT relation.relname, owner.rolname AS owner_name \
           FROM pg_catalog.pg_class AS relation \
           JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
           JOIN pg_catalog.pg_roles AS owner ON owner.oid = relation.relowner \
          WHERE namespace.nspname = current_schema() \
            AND relation.relname LIKE 'executor_%' \
            AND relation.relkind IN ('r', 'p', 'v')",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
    if owner_rows.len() != 8
        || owner_rows.into_iter().any(|row| {
            row.try_get::<String, _>("owner_name").ok().as_deref() != Some("mfm_executor_owner")
        })
    {
        return Err(PostgresExecutorStoreError::SchemaAuthorityMismatch);
    }

    let function_rows = sqlx::query(
        "SELECT procedure.proname, owner.rolname AS owner_name, procedure.prosecdef \
           FROM pg_catalog.pg_proc AS procedure \
           JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = procedure.pronamespace \
           JOIN pg_catalog.pg_roles AS owner ON owner.oid = procedure.proowner \
          WHERE namespace.nspname = current_schema()",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
    if function_rows.len() != 2
        || function_rows.into_iter().any(|row| {
            row.try_get::<String, _>("owner_name").ok().as_deref() != Some("mfm_executor_owner")
                || row.try_get::<bool, _>("prosecdef").ok() != Some(false)
        })
    {
        return Err(PostgresExecutorStoreError::SchemaAuthorityMismatch);
    }

    let row = sqlx::query(
        "SELECT \
             pg_catalog.pg_has_role(current_user, $1, 'MEMBER') AS application_member, \
             pg_catalog.pg_has_role($1, 'mfm_executor_owner', 'MEMBER') AS application_is_owner, \
             pg_catalog.has_schema_privilege($1, current_schema(), 'USAGE') AS schema_usage, \
             pg_catalog.has_schema_privilege($1, current_schema(), 'CREATE') AS schema_create",
    )
    .bind(APPLICATION_ROLE)
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
    if !required_bool(&row, "application_member")?
        || required_bool(&row, "application_is_owner")?
        || !required_bool(&row, "schema_usage")?
        || required_bool(&row, "schema_create")?
    {
        return Err(PostgresExecutorStoreError::SchemaAuthorityMismatch);
    }

    for table in [
        "executor_bindings",
        "executor_content_records",
        "executor_effect_frontiers",
        "executor_resource_records",
        "executor_effect_resource_links",
    ] {
        let row = sqlx::query(
            "SELECT \
                 pg_catalog.has_table_privilege($1, format('%I.%I', current_schema(), $2), \
                                                'SELECT') AS can_select, \
                 pg_catalog.has_table_privilege($1, format('%I.%I', current_schema(), $2), \
                                                'INSERT') AS can_insert, \
                 pg_catalog.has_table_privilege($1, format('%I.%I', current_schema(), $2), \
                                                'UPDATE') AS can_update, \
                 pg_catalog.has_table_privilege($1, format('%I.%I', current_schema(), $2), \
                                                'DELETE') AS can_delete, \
                 pg_catalog.has_table_privilege($1, format('%I.%I', current_schema(), $2), \
                                                'TRUNCATE') AS can_truncate",
        )
        .bind(APPLICATION_ROLE)
        .bind(table)
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
        if !required_bool(&row, "can_select")?
            || !required_bool(&row, "can_insert")?
            || required_bool(&row, "can_update")?
            || required_bool(&row, "can_delete")?
            || required_bool(&row, "can_truncate")?
        {
            return Err(PostgresExecutorStoreError::SchemaAuthorityMismatch);
        }
    }

    for table in [
        "executor_schema_metadata",
        "executor_effect_heads",
        "executor_resource_heads",
    ] {
        let row = sqlx::query(
            "SELECT \
                 pg_catalog.has_table_privilege($1, format('%I.%I', current_schema(), $2), \
                                                'SELECT') AS can_select, \
                 pg_catalog.has_table_privilege($1, format('%I.%I', current_schema(), $2), \
                                                'INSERT') AS can_insert, \
                 pg_catalog.has_table_privilege($1, format('%I.%I', current_schema(), $2), \
                                                'UPDATE') AS can_update, \
                 pg_catalog.has_table_privilege($1, format('%I.%I', current_schema(), $2), \
                                                'DELETE') AS can_delete, \
                 pg_catalog.has_table_privilege($1, format('%I.%I', current_schema(), $2), \
                                                'TRUNCATE') AS can_truncate",
        )
        .bind(APPLICATION_ROLE)
        .bind(table)
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
        if !required_bool(&row, "can_select")?
            || required_bool(&row, "can_insert")?
            || required_bool(&row, "can_update")?
            || required_bool(&row, "can_delete")?
            || required_bool(&row, "can_truncate")?
        {
            return Err(PostgresExecutorStoreError::SchemaAuthorityMismatch);
        }
    }
    Ok(())
}

async fn validate_metadata(connection: &mut PgConnection) -> Result<()> {
    let row = sqlx::query(
        "SELECT singleton, schema_contract_version \
           FROM executor_schema_metadata",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?;
    if !required_bool(&row, "singleton")?
        || row
            .try_get::<String, _>("schema_contract_version")
            .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)?
            != SCHEMA_CONTRACT_VERSION
    {
        return Err(PostgresExecutorStoreError::SchemaAuthorityMismatch);
    }
    Ok(())
}

fn required_bool(row: &sqlx::postgres::PgRow, column: &str) -> Result<bool> {
    row.try_get(column)
        .map_err(|_| PostgresExecutorStoreError::SchemaAuthorityMismatch)
}
