use std::collections::BTreeSet;

use mfm_canonical::sha256_digest_bytes;
use mfm_ids::{StoreEpoch, StoreScopeId};
use sqlx::{PgConnection, PgPool, Row};

use crate::error::{database_error, PostgresStoreError, Result};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub(crate) const SCHEMA_CONTRACT_VERSION: &str = "mfm.recoverability-postgres.v1";
pub(crate) const APPLICATION_ROLE: &str = "mfm_store_application";
const OWNER_ROLE: &str = "mfm_store_owner";
const AUTHORITY_CATALOG_DEFINITION_SHA256: &str =
    "f9fc1e32f85cc6c3e8f338a519313fc3fd2dbe3a6714d8e1b38777f445a58392";

/// Administrative schema management for the destructive recoverability-v1 baseline.
pub struct PostgresSchema;

impl PostgresSchema {
    /// Applies the compiled baseline through a migration-owner connection.
    ///
    /// This is deliberately separate from [`crate::open_authoritative`]. The application writer
    /// role cannot migrate, replace, or repair authority tables.
    pub async fn migrate(database_url: &str) -> Result<()> {
        let pool = PgPool::connect(database_url)
            .await
            .map_err(|error| database_error("connect migration owner", error))?;
        migrate_pool(&pool).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedStoreIdentity {
    pub(crate) store_scope_id: StoreScopeId,
    pub(crate) store_epoch: StoreEpoch,
}

pub(crate) async fn migrate_pool(pool: &PgPool) -> Result<()> {
    MIGRATOR
        .run(pool)
        .await
        .map_err(|_| PostgresStoreError::Database("apply compiled baseline"))?;
    Ok(())
}

#[cfg(test)]
pub(crate) async fn validate_authoritative_schema(pool: &PgPool) -> Result<ValidatedStoreIdentity> {
    validate_authoritative_schema_inner(pool, None).await
}

pub(crate) async fn validate_authoritative_schema_at(
    pool: &PgPool,
    expected_schema: &str,
) -> Result<ValidatedStoreIdentity> {
    validate_authoritative_schema_inner(pool, Some(expected_schema)).await
}

async fn validate_authoritative_schema_inner(
    pool: &PgPool,
    expected_schema: Option<&str>,
) -> Result<ValidatedStoreIdentity> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    let connection = &mut *transaction;
    if let Some(expected_schema) = expected_schema {
        pin_validation_schema(connection, expected_schema).await?;
    }
    validate_migrations(connection).await?;
    validate_tables_and_columns(connection).await?;
    validate_indexes(connection).await?;
    validate_constraints(connection).await?;
    validate_functions_and_triggers(connection).await?;
    validate_catalog_definitions(connection).await?;
    validate_roles_and_privileges(connection).await?;
    let identity = validate_store_identity(connection).await?;
    validate_tenant_fact_integrity(connection).await?;
    transaction
        .commit()
        .await
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    Ok(identity)
}

async fn pin_validation_schema(connection: &mut PgConnection, expected_schema: &str) -> Result<()> {
    sqlx::query(
        "SELECT pg_catalog.set_config( \
             'search_path', pg_catalog.format('%I, pg_catalog', $1), TRUE \
         )",
    )
    .bind(expected_schema)
    .execute(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    let actual_schema = sqlx::query_scalar::<_, String>("SELECT pg_catalog.current_schema()::text")
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    if actual_schema != expected_schema {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    Ok(())
}

async fn validate_migrations(connection: &mut PgConnection) -> Result<()> {
    let rows =
        sqlx::query("SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&mut *connection)
            .await
            .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;

    if rows.len() != MIGRATOR.iter().count() {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    for migration in MIGRATOR.iter() {
        let Some(row) = rows
            .iter()
            .find(|row| row.try_get::<i64, _>("version").ok() == Some(migration.version))
        else {
            return Err(PostgresStoreError::SchemaAuthorityMismatch);
        };
        if !row
            .try_get::<bool, _>("success")
            .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?
        {
            return Err(PostgresStoreError::SchemaAuthorityMismatch);
        }
        let checksum = row
            .try_get::<Vec<u8>, _>("checksum")
            .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
        if checksum.as_slice() != migration.checksum.as_ref() {
            return Err(PostgresStoreError::MigrationChecksumMismatch);
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct ColumnContract {
    table: &'static str,
    name: &'static str,
    udt_name: &'static str,
    nullable: bool,
}

async fn validate_tables_and_columns(connection: &mut PgConnection) -> Result<()> {
    let table_rows = sqlx::query(
        "SELECT table_name \
           FROM information_schema.tables \
          WHERE table_schema = current_schema() \
            AND table_type = 'BASE TABLE'",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    let actual_tables = strings_from_rows(&table_rows, "table_name")?;
    let expected_tables = REQUIRED_TABLES
        .iter()
        .copied()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if actual_tables != expected_tables {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }

    let view_rows = sqlx::query(
        "SELECT table_name \
           FROM information_schema.views \
          WHERE table_schema = current_schema()",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    if !view_rows.is_empty() {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }

    let column_rows = sqlx::query(
        "SELECT table_name, column_name, udt_name, is_nullable, ordinal_position \
           FROM information_schema.columns \
          WHERE table_schema = current_schema() \
            AND table_name <> '_sqlx_migrations' \
          ORDER BY table_name, ordinal_position",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    let mut actual = Vec::with_capacity(column_rows.len());
    for row in column_rows {
        actual.push(ColumnShape {
            table: required_string(&row, "table_name")?,
            name: required_string(&row, "column_name")?,
            udt_name: required_string(&row, "udt_name")?,
            nullable: required_string(&row, "is_nullable")? == "YES",
        });
    }
    let expected = REQUIRED_COLUMNS
        .iter()
        .map(|column| ColumnShape {
            table: column.table.to_owned(),
            name: column.name.to_owned(),
            udt_name: column.udt_name.to_owned(),
            nullable: column.nullable,
        })
        .collect::<Vec<_>>();
    if actual != expected {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct ColumnShape {
    table: String,
    name: String,
    udt_name: String,
    nullable: bool,
}

async fn validate_indexes(connection: &mut PgConnection) -> Result<()> {
    let rows = sqlx::query(
        "SELECT indexname \
           FROM pg_catalog.pg_indexes \
          WHERE schemaname = current_schema()",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    let actual = strings_from_rows(&rows, "indexname")?;
    let expected = REQUIRED_INDEXES
        .iter()
        .copied()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    Ok(())
}

async fn validate_constraints(connection: &mut PgConnection) -> Result<()> {
    let rows = sqlx::query(
        "SELECT constraint_row.conname AS constraint_name \
           FROM pg_catalog.pg_constraint AS constraint_row \
           JOIN pg_catalog.pg_class AS relation ON relation.oid = constraint_row.conrelid \
           JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
          WHERE namespace.nspname = current_schema() \
            AND relation.relname <> '_sqlx_migrations' \
            AND constraint_row.contype NOT IN ('n', 't')",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    let actual = strings_from_rows(&rows, "constraint_name")?;
    let expected = REQUIRED_CONSTRAINTS
        .iter()
        .copied()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    Ok(())
}

async fn validate_functions_and_triggers(connection: &mut PgConnection) -> Result<()> {
    let function_rows = sqlx::query(
        "SELECT procedure.proname, procedure.prosecdef, owner.rolname AS owner_name, \
                pg_catalog.pg_get_function_identity_arguments(procedure.oid) AS arguments, \
                pg_catalog.pg_get_function_result(procedure.oid) AS result_type \
           FROM pg_catalog.pg_proc AS procedure \
           JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = procedure.pronamespace \
           JOIN pg_catalog.pg_roles AS owner ON owner.oid = procedure.proowner \
          WHERE namespace.nspname = current_schema()",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    if function_rows.len() != REQUIRED_FUNCTIONS.len() {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    for contract in REQUIRED_FUNCTIONS {
        let Some(row) = function_rows
            .iter()
            .find(|row| row.try_get::<String, _>("proname").ok().as_deref() == Some(contract.name))
        else {
            return Err(PostgresStoreError::SchemaAuthorityMismatch);
        };
        if required_string(row, "owner_name")? != OWNER_ROLE
            || required_string(row, "arguments")? != contract.arguments
            || required_string(row, "result_type")? != contract.result_type
            || row
                .try_get::<bool, _>("prosecdef")
                .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?
                != contract.security_definer
        {
            return Err(PostgresStoreError::SchemaAuthorityMismatch);
        }
    }

    let trigger_rows = sqlx::query(
        "SELECT trigger.tgname, relation.relname AS table_name, procedure.proname AS function_name, \
                trigger.tgdeferrable, trigger.tginitdeferred, trigger.tgenabled::text AS enabled \
           FROM pg_catalog.pg_trigger AS trigger \
           JOIN pg_catalog.pg_class AS relation ON relation.oid = trigger.tgrelid \
           JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
           JOIN pg_catalog.pg_proc AS procedure ON procedure.oid = trigger.tgfoid \
          WHERE namespace.nspname = current_schema() \
            AND NOT trigger.tgisinternal",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    if trigger_rows.len() != REQUIRED_TRIGGERS.len() {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    for contract in REQUIRED_TRIGGERS {
        let Some(row) = trigger_rows
            .iter()
            .find(|row| row.try_get::<String, _>("tgname").ok().as_deref() == Some(contract.name))
        else {
            return Err(PostgresStoreError::SchemaAuthorityMismatch);
        };
        if required_string(row, "table_name")? != contract.table
            || required_string(row, "function_name")? != contract.function
            || required_string(row, "enabled")? != "O"
            || row
                .try_get::<bool, _>("tgdeferrable")
                .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?
                != contract.deferred
            || row
                .try_get::<bool, _>("tginitdeferred")
                .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?
                != contract.deferred
        {
            return Err(PostgresStoreError::SchemaAuthorityMismatch);
        }
    }
    Ok(())
}

async fn validate_catalog_definitions(connection: &mut PgConnection) -> Result<()> {
    let rows = sqlx::query(
        "WITH catalog_rows AS ( \
             SELECT \
                 'table'::text AS kind, \
                 relation.relname::text AS identity, \
                 concat_ws('|', relation.relkind::text, relation.relpersistence::text, \
                     relation.relrowsecurity::text, relation.relforcerowsecurity::text, \
                     relation.relreplident::text, \
                     coalesce(array_to_string(relation.reloptions, ','), '')) AS definition \
               FROM pg_catalog.pg_class AS relation \
               JOIN pg_catalog.pg_namespace AS namespace \
                 ON namespace.oid = relation.relnamespace \
              WHERE namespace.nspname = current_schema() \
                AND relation.relkind = 'r' \
                AND relation.relname <> '_sqlx_migrations' \
             UNION ALL \
             SELECT \
                 'column', \
                 relation.relname || '.' || attribute.attname, \
                 concat_ws('|', attribute.attnum::text, \
                     pg_catalog.format_type(attribute.atttypid, attribute.atttypmod), \
                     attribute.attnotnull::text, attribute.attidentity::text, \
                     attribute.attgenerated::text, \
                     coalesce(pg_catalog.pg_get_expr(default_value.adbin, \
                         default_value.adrelid, TRUE), ''), \
                     coalesce(collation_namespace.nspname || '.' \
                         || collation_row.collname, '')) \
               FROM pg_catalog.pg_attribute AS attribute \
               JOIN pg_catalog.pg_class AS relation ON relation.oid = attribute.attrelid \
               JOIN pg_catalog.pg_namespace AS namespace \
                 ON namespace.oid = relation.relnamespace \
               LEFT JOIN pg_catalog.pg_attrdef AS default_value \
                 ON default_value.adrelid = attribute.attrelid \
                AND default_value.adnum = attribute.attnum \
               LEFT JOIN pg_catalog.pg_collation AS collation_row \
                 ON collation_row.oid = attribute.attcollation \
               LEFT JOIN pg_catalog.pg_namespace AS collation_namespace \
                 ON collation_namespace.oid = collation_row.collnamespace \
              WHERE namespace.nspname = current_schema() \
                AND relation.relkind = 'r' \
                AND relation.relname <> '_sqlx_migrations' \
                AND attribute.attnum > 0 \
                AND NOT attribute.attisdropped \
             UNION ALL \
             SELECT \
                 'index', \
                 index_relation.relname, \
                 replace(pg_catalog.pg_get_indexdef(index_relation.oid), \
                     pg_catalog.quote_ident(current_schema()) || '.', '<schema>.') \
               FROM pg_catalog.pg_class AS index_relation \
               JOIN pg_catalog.pg_index AS index \
                 ON index.indexrelid = index_relation.oid \
               JOIN pg_catalog.pg_class AS relation ON relation.oid = index.indrelid \
               JOIN pg_catalog.pg_namespace AS namespace \
                 ON namespace.oid = relation.relnamespace \
              WHERE namespace.nspname = current_schema() \
                AND relation.relname <> '_sqlx_migrations' \
             UNION ALL \
             SELECT \
                 'constraint', \
                 constraint_row.conname, \
                 concat_ws('|', relation.relname, constraint_row.contype::text, \
                     constraint_row.condeferrable::text, constraint_row.condeferred::text, \
                     constraint_row.convalidated::text, \
                     pg_catalog.pg_get_constraintdef(constraint_row.oid, TRUE)) \
               FROM pg_catalog.pg_constraint AS constraint_row \
               JOIN pg_catalog.pg_class AS relation ON relation.oid = constraint_row.conrelid \
               JOIN pg_catalog.pg_namespace AS namespace \
                 ON namespace.oid = relation.relnamespace \
              WHERE namespace.nspname = current_schema() \
                AND relation.relname <> '_sqlx_migrations' \
                AND constraint_row.contype NOT IN ('n', 't') \
             UNION ALL \
             SELECT \
                 'function', \
                 procedure.proname || '(' \
                     || pg_catalog.pg_get_function_identity_arguments(procedure.oid) || ')', \
                 concat_ws('|', language.lanname, procedure.provolatile::text, \
                     procedure.proparallel::text, procedure.proisstrict::text, \
                     procedure.prosecdef::text, procedure.proleakproof::text, \
                     coalesce(array_to_string(procedure.proconfig, ','), ''), \
                     replace(pg_catalog.pg_get_functiondef(procedure.oid), \
                         pg_catalog.quote_ident(current_schema()) || '.', '<schema>.')) \
               FROM pg_catalog.pg_proc AS procedure \
               JOIN pg_catalog.pg_namespace AS namespace \
                 ON namespace.oid = procedure.pronamespace \
               JOIN pg_catalog.pg_language AS language \
                 ON language.oid = procedure.prolang \
              WHERE namespace.nspname = current_schema() \
             UNION ALL \
             SELECT \
                 'trigger', \
                 relation.relname || '.' || trigger.tgname, \
                 concat_ws('|', trigger.tgenabled::text, trigger.tgdeferrable::text, \
                     trigger.tginitdeferred::text, \
                     replace(pg_catalog.pg_get_triggerdef(trigger.oid, TRUE), \
                         pg_catalog.quote_ident(current_schema()) || '.', '<schema>.')) \
               FROM pg_catalog.pg_trigger AS trigger \
               JOIN pg_catalog.pg_class AS relation ON relation.oid = trigger.tgrelid \
               JOIN pg_catalog.pg_namespace AS namespace \
                 ON namespace.oid = relation.relnamespace \
              WHERE namespace.nspname = current_schema() \
                AND NOT trigger.tgisinternal \
         ) \
         SELECT kind, identity, definition \
           FROM catalog_rows \
          ORDER BY kind, identity",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    let mut fingerprint = Vec::new();
    for row in rows {
        for column in ["kind", "identity", "definition"] {
            let value = required_string(&row, column)?;
            fingerprint.extend_from_slice(value.len().to_string().as_bytes());
            fingerprint.push(b':');
            fingerprint.extend_from_slice(value.as_bytes());
        }
    }
    if sha256_digest_bytes(&fingerprint).to_string() != AUTHORITY_CATALOG_DEFINITION_SHA256 {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    Ok(())
}

async fn validate_roles_and_privileges(connection: &mut PgConnection) -> Result<()> {
    for role in [OWNER_ROLE, APPLICATION_ROLE] {
        let row = sqlx::query(
            "SELECT rolcanlogin, rolinherit, rolsuper, rolcreatedb, rolcreaterole, \
                    rolreplication, rolbypassrls \
               FROM pg_catalog.pg_roles \
              WHERE rolname = $1",
        )
        .bind(role)
        .fetch_optional(&mut *connection)
        .await
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?
        .ok_or(PostgresStoreError::SchemaAuthorityMismatch)?;
        if !required_bool(&row, "rolinherit")? {
            return Err(PostgresStoreError::SchemaAuthorityMismatch);
        }
        for attribute in [
            "rolcanlogin",
            "rolsuper",
            "rolcreatedb",
            "rolcreaterole",
            "rolreplication",
            "rolbypassrls",
        ] {
            if row
                .try_get::<bool, _>(attribute)
                .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?
            {
                return Err(PostgresStoreError::SchemaAuthorityMismatch);
            }
        }
    }

    let owner_rows = sqlx::query(
        "SELECT relation.relname AS object_name, owner.rolname AS owner_name \
           FROM pg_catalog.pg_class AS relation \
           JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
           JOIN pg_catalog.pg_roles AS owner ON owner.oid = relation.relowner \
          WHERE namespace.nspname = current_schema() \
            AND relation.relkind = 'r' \
            AND relation.relname <> '_sqlx_migrations'",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    if owner_rows.len() != REQUIRED_TABLES.len() - 1 {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    for row in &owner_rows {
        if required_string(row, "owner_name")? != OWNER_ROLE {
            return Err(PostgresStoreError::SchemaAuthorityMismatch);
        }
    }

    let privileges = [
        "SELECT",
        "INSERT",
        "UPDATE",
        "DELETE",
        "TRUNCATE",
        "REFERENCES",
        "TRIGGER",
        "MAINTAIN",
    ];
    for table in REQUIRED_TABLES.iter().copied() {
        for privilege in privileges {
            let expected = expected_application_table_privilege(table, privilege);
            let actual = has_table_privilege(connection, table, privilege).await?;
            if actual != expected {
                return Err(PostgresStoreError::SchemaAuthorityMismatch);
            }
        }
    }

    for function in REQUIRED_FUNCTIONS {
        let actual = has_function_execute_privilege(connection, function).await?;
        let expected = function.name == "mfm_assign_tenant_fact_coordinate";
        if actual != expected {
            return Err(PostgresStoreError::SchemaAuthorityMismatch);
        }
    }

    let schema_privileges = sqlx::query(
        "SELECT \
             pg_catalog.has_schema_privilege($1, current_schema(), 'USAGE') AS app_usage, \
             pg_catalog.has_schema_privilege($1, current_schema(), 'CREATE') AS app_create, \
             pg_catalog.has_schema_privilege($2, current_schema(), 'USAGE') AS owner_usage, \
             pg_catalog.has_schema_privilege($2, current_schema(), 'CREATE') AS owner_create",
    )
    .bind(APPLICATION_ROLE)
    .bind(OWNER_ROLE)
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    if !required_bool(&schema_privileges, "app_usage")?
        || required_bool(&schema_privileges, "app_create")?
        || !required_bool(&schema_privileges, "owner_usage")?
        || !required_bool(&schema_privileges, "owner_create")?
    {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    validate_direct_acl_shape(connection).await?;
    Ok(())
}

async fn validate_direct_acl_shape(connection: &mut PgConnection) -> Result<()> {
    let table_rows = sqlx::query(
        "SELECT relation.relname AS object_name, \
                CASE WHEN acl.grantee = 0 THEN 'PUBLIC' ELSE grantee.rolname END AS grantee, \
                acl.privilege_type, acl.is_grantable \
           FROM pg_catalog.pg_class AS relation \
           JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
          CROSS JOIN LATERAL pg_catalog.aclexplode(coalesce( \
                relation.relacl, pg_catalog.acldefault('r', relation.relowner) \
          )) AS acl \
           LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid = acl.grantee \
          WHERE namespace.nspname = current_schema() \
            AND relation.relkind = 'r' \
            AND relation.relname <> '_sqlx_migrations'",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    let actual_tables = acl_entries(&table_rows)?;
    let mut expected_tables = BTreeSet::new();
    for table in REQUIRED_TABLES
        .iter()
        .copied()
        .filter(|table| *table != "_sqlx_migrations")
    {
        for privilege in [
            "DELETE",
            "INSERT",
            "MAINTAIN",
            "REFERENCES",
            "SELECT",
            "TRIGGER",
            "TRUNCATE",
            "UPDATE",
        ] {
            expected_tables.insert((
                table.to_owned(),
                OWNER_ROLE.to_owned(),
                privilege.to_owned(),
                false,
            ));
        }
        for privilege in ["SELECT", "INSERT", "UPDATE"] {
            if expected_application_table_privilege(table, privilege) {
                expected_tables.insert((
                    table.to_owned(),
                    APPLICATION_ROLE.to_owned(),
                    privilege.to_owned(),
                    false,
                ));
            }
        }
    }
    if actual_tables != expected_tables {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }

    let function_rows = sqlx::query(
        "SELECT procedure.proname AS object_name, \
                CASE WHEN acl.grantee = 0 THEN 'PUBLIC' ELSE grantee.rolname END AS grantee, \
                acl.privilege_type, acl.is_grantable \
           FROM pg_catalog.pg_proc AS procedure \
           JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = procedure.pronamespace \
          CROSS JOIN LATERAL pg_catalog.aclexplode(coalesce( \
                procedure.proacl, pg_catalog.acldefault('f', procedure.proowner) \
          )) AS acl \
           LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid = acl.grantee \
          WHERE namespace.nspname = current_schema()",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    let actual_functions = acl_entries(&function_rows)?;
    let mut expected_functions = BTreeSet::new();
    for function in REQUIRED_FUNCTIONS {
        expected_functions.insert((
            function.name.to_owned(),
            OWNER_ROLE.to_owned(),
            "EXECUTE".to_owned(),
            false,
        ));
        if function.name == "mfm_assign_tenant_fact_coordinate" {
            expected_functions.insert((
                function.name.to_owned(),
                APPLICATION_ROLE.to_owned(),
                "EXECUTE".to_owned(),
                false,
            ));
        }
    }
    if actual_functions != expected_functions {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }

    let schema_rows = sqlx::query(
        "SELECT current_schema()::text AS object_name, \
                namespace_owner.rolname AS schema_owner, \
                CASE WHEN acl.grantee = 0 THEN 'PUBLIC' ELSE grantee.rolname END AS grantee, \
                acl.privilege_type, acl.is_grantable \
           FROM pg_catalog.pg_namespace AS namespace \
           JOIN pg_catalog.pg_roles AS namespace_owner ON namespace_owner.oid = namespace.nspowner \
          CROSS JOIN LATERAL pg_catalog.aclexplode(coalesce( \
                namespace.nspacl, pg_catalog.acldefault('n', namespace.nspowner) \
          )) AS acl \
           LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid = acl.grantee \
          WHERE namespace.nspname = current_schema()",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    let first_schema_row = schema_rows
        .first()
        .ok_or(PostgresStoreError::SchemaAuthorityMismatch)?;
    let schema_name = required_string(first_schema_row, "object_name")?;
    let schema_owner = required_string(first_schema_row, "schema_owner")?;
    let actual_schema = acl_entries(&schema_rows)?;
    let mut expected_schema = BTreeSet::new();
    for privilege in ["CREATE", "USAGE"] {
        expected_schema.insert((
            schema_name.clone(),
            schema_owner.clone(),
            privilege.to_owned(),
            false,
        ));
        expected_schema.insert((
            schema_name.clone(),
            OWNER_ROLE.to_owned(),
            privilege.to_owned(),
            false,
        ));
    }
    expected_schema.insert((
        schema_name,
        APPLICATION_ROLE.to_owned(),
        "USAGE".to_owned(),
        false,
    ));
    if actual_schema != expected_schema {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    Ok(())
}

fn acl_entries(rows: &[sqlx::postgres::PgRow]) -> Result<BTreeSet<(String, String, String, bool)>> {
    rows.iter()
        .map(|row| {
            Ok((
                required_string(row, "object_name")?,
                required_string(row, "grantee")?,
                required_string(row, "privilege_type")?,
                required_bool(row, "is_grantable")?,
            ))
        })
        .collect()
}

fn expected_application_table_privilege(table: &str, privilege: &str) -> bool {
    match privilege {
        "SELECT" => true,
        "INSERT" => matches!(
            table,
            "journal_commits"
                | "journal_records"
                | "artifact_blobs"
                | "artifact_admissions"
                | "commit_artifact_bindings"
                | "commit_object_authorities"
                | "fact_scan_attestations"
                | "qualified_support_members"
        ),
        "UPDATE" => false,
        "DELETE" | "TRUNCATE" | "REFERENCES" | "TRIGGER" | "MAINTAIN" => false,
        _ => false,
    }
}

async fn has_table_privilege(
    connection: &mut PgConnection,
    table: &str,
    privilege: &str,
) -> Result<bool> {
    sqlx::query_scalar::<_, bool>(
        "SELECT pg_catalog.has_table_privilege( \
             $1, format('%I.%I', current_schema(), $2), $3 \
         )",
    )
    .bind(APPLICATION_ROLE)
    .bind(table)
    .bind(privilege)
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)
}

async fn has_function_execute_privilege(
    connection: &mut PgConnection,
    function: &FunctionContract,
) -> Result<bool> {
    let signature = if function.arguments.is_empty() {
        format!(
            "{}.{}()",
            current_schema_name(connection).await?,
            function.name
        )
    } else {
        format!(
            "{}.{}(text,text)",
            current_schema_name(connection).await?,
            function.name
        )
    };
    sqlx::query_scalar::<_, bool>("SELECT pg_catalog.has_function_privilege($1, $2, 'EXECUTE')")
        .bind(APPLICATION_ROLE)
        .bind(signature)
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)
}

async fn current_schema_name(connection: &mut PgConnection) -> Result<String> {
    sqlx::query_scalar::<_, String>("SELECT current_schema()::text")
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)
}

async fn validate_store_identity(connection: &mut PgConnection) -> Result<ValidatedStoreIdentity> {
    let row = sqlx::query(
        "SELECT identity.store_scope_id, identity.store_epoch::text AS store_epoch, \
                metadata.schema_contract_version \
           FROM store_identity AS identity \
           CROSS JOIN store_schema_metadata AS metadata \
          WHERE identity.singleton AND metadata.singleton",
    )
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?
    .ok_or(PostgresStoreError::SchemaAuthorityMismatch)?;
    let row_count = sqlx::query_scalar::<_, i64>(
        "SELECT (SELECT count(*) FROM store_identity) \
              + (SELECT count(*) FROM store_schema_metadata)",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    if row_count != 2
        || required_string(&row, "schema_contract_version")? != SCHEMA_CONTRACT_VERSION
    {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    let store_scope_id = StoreScopeId::new(required_string(&row, "store_scope_id")?)
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    let store_epoch = StoreEpoch::parse(required_string(&row, "store_epoch")?)
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    Ok(ValidatedStoreIdentity {
        store_scope_id,
        store_epoch,
    })
}

async fn validate_tenant_fact_integrity(connection: &mut PgConnection) -> Result<()> {
    let invalid_count = sqlx::query_scalar::<_, i64>(
        "WITH tenant_universe AS ( \
             SELECT tenant_scope_id FROM tenant_fact_order_heads \
             UNION \
             SELECT tenant_scope_id FROM journal_commits \
              WHERE tenant_fact_coordinate_kind IN ( \
                  'fact_publication', 'fact_selection_barrier' \
              ) \
         ), retained AS ( \
             SELECT universe.tenant_scope_id, head.current_fact_order, \
                    count(commit.*) FILTER ( \
                        WHERE commit.tenant_fact_coordinate_kind = 'fact_publication' \
                    ) AS publication_count, \
                    coalesce(max(commit.tenant_fact_order) FILTER ( \
                        WHERE commit.tenant_fact_coordinate_kind = 'fact_publication' \
                    ), 0) AS maximum_publication, \
                    count(commit.*) FILTER ( \
                        WHERE commit.tenant_fact_coordinate_kind = 'fact_selection_barrier' \
                          AND commit.tenant_fact_order NOT BETWEEN 0 AND head.current_fact_order \
                    ) AS invalid_barrier_count, \
                    count(commit.*) AS coordinate_count \
               FROM tenant_universe AS universe \
               LEFT JOIN tenant_fact_order_heads AS head USING (tenant_scope_id) \
               LEFT JOIN journal_commits AS commit \
                 ON commit.tenant_scope_id = universe.tenant_scope_id \
                AND commit.tenant_fact_coordinate_kind IN ( \
                    'fact_publication', 'fact_selection_barrier' \
                ) \
              GROUP BY universe.tenant_scope_id, head.current_fact_order \
         ) \
         SELECT count(*)::bigint \
           FROM retained \
          WHERE current_fact_order IS NULL \
             OR coordinate_count = 0 \
             OR current_fact_order <> publication_count::numeric \
             OR current_fact_order <> maximum_publication \
             OR invalid_barrier_count <> 0",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
    if invalid_count != 0 {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    Ok(())
}

fn strings_from_rows(rows: &[sqlx::postgres::PgRow], column: &str) -> Result<BTreeSet<String>> {
    rows.iter()
        .map(|row| required_string(row, column))
        .collect()
}

fn required_string(row: &sqlx::postgres::PgRow, column: &str) -> Result<String> {
    row.try_get::<String, _>(column)
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)
}

fn required_bool(row: &sqlx::postgres::PgRow, column: &str) -> Result<bool> {
    row.try_get::<bool, _>(column)
        .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)
}

struct FunctionContract {
    name: &'static str,
    arguments: &'static str,
    result_type: &'static str,
    security_definer: bool,
}

struct TriggerContract {
    name: &'static str,
    table: &'static str,
    function: &'static str,
    deferred: bool,
}

const REQUIRED_TABLES: &[&str] = &[
    "_sqlx_migrations",
    "artifact_admissions",
    "artifact_blobs",
    "commit_artifact_bindings",
    "commit_object_authorities",
    "configured_values",
    "fact_scan_attestations",
    "journal_commits",
    "journal_records",
    "qualified_support_members",
    "store_identity",
    "store_schema_metadata",
    "tenant_fact_order_heads",
];

const REQUIRED_COLUMNS: &[ColumnContract] = &[
    column("artifact_admissions", "artifact_id", "text", false),
    column("artifact_admissions", "evidence_hash", "text", false),
    column("artifact_admissions", "content_digest", "text", false),
    column("artifact_admissions", "schema_id", "text", false),
    column("artifact_admissions", "semantic_type_id", "text", false),
    column("artifact_admissions", "role", "text", false),
    column("artifact_admissions", "byte_length", "numeric", false),
    column("artifact_admissions", "media_type", "text", false),
    column(
        "artifact_admissions",
        "evidence_contract_schema_id",
        "text",
        false,
    ),
    column(
        "artifact_admissions",
        "evidence_contract_content_digest",
        "text",
        false,
    ),
    column("artifact_admissions", "canonical_value_ref", "bytea", false),
    column("artifact_blobs", "content_digest", "text", false),
    column("artifact_blobs", "byte_length", "numeric", false),
    column("artifact_blobs", "bytes", "bytea", false),
    column("commit_artifact_bindings", "run_id", "text", false),
    column("commit_artifact_bindings", "run_sequence", "numeric", false),
    column("commit_artifact_bindings", "record_ordinal", "int4", false),
    column("commit_artifact_bindings", "field_path", "text", false),
    column("commit_artifact_bindings", "authority_use", "text", false),
    column("commit_artifact_bindings", "artifact_id", "text", false),
    column("commit_artifact_bindings", "content_digest", "text", false),
    column("commit_artifact_bindings", "evidence_hash", "text", false),
    column(
        "commit_artifact_bindings",
        "canonical_value_ref",
        "bytea",
        false,
    ),
    column("commit_object_authorities", "run_id", "text", false),
    column(
        "commit_object_authorities",
        "run_sequence",
        "numeric",
        false,
    ),
    column("commit_object_authorities", "artifact_id", "text", false),
    column("commit_object_authorities", "content_digest", "text", false),
    column("commit_object_authorities", "evidence_hash", "text", false),
    column(
        "commit_object_authorities",
        "canonical_value_ref",
        "bytea",
        false,
    ),
    column("commit_object_authorities", "admission_mode", "text", false),
    column("configured_values", "store_scope_id", "text", false),
    column("configured_values", "tenant_scope_id", "text", false),
    column("configured_values", "entry_point_id", "text", false),
    column("configured_values", "target", "text", false),
    column("configured_values", "artifact_id", "text", false),
    column("configured_values", "content_digest", "text", false),
    column("configured_values", "evidence_hash", "text", false),
    column("configured_values", "canonical_value_ref", "bytea", false),
    column("configured_values", "canonical_binding", "bytea", false),
    column("fact_scan_attestations", "tenant_scope_id", "text", false),
    column("fact_scan_attestations", "consuming_run_id", "text", false),
    column(
        "fact_scan_attestations",
        "containing_run_sequence",
        "numeric",
        false,
    ),
    column(
        "fact_scan_attestations",
        "authorization_ref",
        "bytea",
        false,
    ),
    column(
        "fact_scan_attestations",
        "attestation_artifact_id",
        "text",
        false,
    ),
    column(
        "fact_scan_attestations",
        "attestation_content_digest",
        "text",
        false,
    ),
    column(
        "fact_scan_attestations",
        "attestation_evidence_hash",
        "text",
        false,
    ),
    column("fact_scan_attestations", "attestation_ref", "bytea", false),
    column("fact_scan_attestations", "observation_ref", "bytea", false),
    column(
        "fact_scan_attestations",
        "containing_journal_head",
        "bytea",
        false,
    ),
    column("journal_commits", "run_id", "text", false),
    column("journal_commits", "run_sequence", "numeric", false),
    column("journal_commits", "append_request_id", "text", false),
    column("journal_commits", "candidate_digest", "text", false),
    column("journal_commits", "predecessor_kind", "text", false),
    column(
        "journal_commits",
        "predecessor_run_sequence",
        "numeric",
        true,
    ),
    column(
        "journal_commits",
        "predecessor_commit_digest",
        "text",
        false,
    ),
    column("journal_commits", "commit_digest", "text", false),
    column("journal_commits", "tenant_scope_id", "text", false),
    column(
        "journal_commits",
        "admission_entry_point_operation_id",
        "text",
        true,
    ),
    column(
        "journal_commits",
        "admission_invocation_identity",
        "text",
        true,
    ),
    column(
        "journal_commits",
        "tenant_fact_coordinate_kind",
        "text",
        false,
    ),
    column("journal_commits", "tenant_fact_order", "numeric", true),
    column("journal_commits", "record_count", "int2", false),
    column("journal_commits", "committed_at", "numeric", false),
    column("journal_records", "run_id", "text", false),
    column("journal_records", "run_sequence", "numeric", false),
    column("journal_records", "tenant_scope_id", "text", false),
    column("journal_records", "fact_order", "numeric", true),
    column("journal_records", "ordinal", "int4", false),
    column("journal_records", "record_id", "text", false),
    column("journal_records", "record_schema_id", "text", false),
    column("journal_records", "spec_hash", "text", false),
    column("journal_records", "logical_key", "bytea", false),
    column("journal_records", "record_hash", "text", false),
    column("journal_records", "canonical_payload", "bytea", false),
    column("journal_records", "emits_facts", "bool", false),
    column(
        "qualified_support_members",
        "qualification_scope_id",
        "text",
        false,
    ),
    column("qualified_support_members", "field_path", "text", false),
    column("qualified_support_members", "artifact_id", "text", false),
    column("qualified_support_members", "content_digest", "text", false),
    column("qualified_support_members", "evidence_hash", "text", false),
    column(
        "qualified_support_members",
        "canonical_value_ref",
        "bytea",
        false,
    ),
    column("store_identity", "singleton", "bool", false),
    column("store_identity", "store_scope_id", "text", false),
    column("store_identity", "store_epoch", "numeric", false),
    column("store_schema_metadata", "singleton", "bool", false),
    column(
        "store_schema_metadata",
        "schema_contract_version",
        "text",
        false,
    ),
    column("tenant_fact_order_heads", "tenant_scope_id", "text", false),
    column(
        "tenant_fact_order_heads",
        "current_fact_order",
        "numeric",
        false,
    ),
];

const fn column(
    table: &'static str,
    name: &'static str,
    udt_name: &'static str,
    nullable: bool,
) -> ColumnContract {
    ColumnContract {
        table,
        name,
        udt_name,
        nullable,
    }
}

const REQUIRED_INDEXES: &[&str] = &[
    "_sqlx_migrations_pkey",
    "artifact_admissions_identity_key",
    "artifact_admissions_pkey",
    "artifact_admissions_routing_idx",
    "artifact_blobs_pkey",
    "commit_artifact_bindings_identity_idx",
    "commit_artifact_bindings_pkey",
    "commit_object_authorities_identity_idx",
    "commit_object_authorities_identity_key",
    "commit_object_authorities_pkey",
    "configured_values_pkey",
    "fact_scan_attestations_commit_key",
    "fact_scan_attestations_observation_ref_key",
    "fact_scan_attestations_pkey",
    "fact_scan_attestations_run_idx",
    "journal_commits_append_request_key",
    "journal_commits_admission_logical_key",
    "journal_commits_commit_digest_key",
    "journal_commits_pkey",
    "journal_commits_tenant_barrier_idx",
    "journal_commits_tenant_publication_order_key",
    "journal_records_fact_scan_idx",
    "journal_records_logical_key_key",
    "journal_records_pkey",
    "journal_records_record_id_key",
    "qualified_support_members_identity_idx",
    "qualified_support_members_pkey",
    "store_identity_pkey",
    "store_identity_store_scope_id_key",
    "store_schema_metadata_pkey",
    "tenant_fact_order_heads_pkey",
];

const REQUIRED_FUNCTIONS: &[FunctionContract] = &[
    FunctionContract {
        name: "mfm_assign_tenant_fact_coordinate",
        arguments: "requested_tenant_scope_id text, requested_coordinate_kind text",
        result_type: "numeric",
        security_definer: true,
    },
    FunctionContract {
        name: "mfm_guard_tenant_fact_head_mutation",
        arguments: "",
        result_type: "trigger",
        security_definer: false,
    },
    FunctionContract {
        name: "mfm_reject_authority_mutation",
        arguments: "",
        result_type: "trigger",
        security_definer: false,
    },
    FunctionContract {
        name: "mfm_reject_store_identity_mutation",
        arguments: "",
        result_type: "trigger",
        security_definer: false,
    },
    FunctionContract {
        name: "mfm_validate_journal_batch",
        arguments: "",
        result_type: "trigger",
        security_definer: true,
    },
    FunctionContract {
        name: "mfm_validate_tenant_fact_heads",
        arguments: "",
        result_type: "trigger",
        security_definer: true,
    },
];

const REQUIRED_TRIGGERS: &[TriggerContract] = &[
    trigger(
        "artifact_admissions_no_update",
        "artifact_admissions",
        "mfm_reject_authority_mutation",
        false,
    ),
    trigger(
        "artifact_blobs_no_update",
        "artifact_blobs",
        "mfm_reject_authority_mutation",
        false,
    ),
    trigger(
        "commit_artifact_bindings_no_update",
        "commit_artifact_bindings",
        "mfm_reject_authority_mutation",
        false,
    ),
    trigger(
        "commit_object_authorities_no_update",
        "commit_object_authorities",
        "mfm_reject_authority_mutation",
        false,
    ),
    trigger(
        "configured_values_no_update",
        "configured_values",
        "mfm_reject_authority_mutation",
        false,
    ),
    trigger(
        "fact_scan_attestations_no_update",
        "fact_scan_attestations",
        "mfm_reject_authority_mutation",
        false,
    ),
    trigger(
        "journal_commits_batch_contract",
        "journal_commits",
        "mfm_validate_journal_batch",
        true,
    ),
    trigger(
        "journal_commits_no_update",
        "journal_commits",
        "mfm_reject_authority_mutation",
        false,
    ),
    trigger(
        "journal_commits_tenant_fact_integrity",
        "journal_commits",
        "mfm_validate_tenant_fact_heads",
        true,
    ),
    trigger(
        "journal_records_batch_contract",
        "journal_records",
        "mfm_validate_journal_batch",
        true,
    ),
    trigger(
        "journal_records_no_update",
        "journal_records",
        "mfm_reject_authority_mutation",
        false,
    ),
    trigger(
        "qualified_support_members_no_update",
        "qualified_support_members",
        "mfm_reject_authority_mutation",
        false,
    ),
    trigger(
        "store_identity_no_mutation",
        "store_identity",
        "mfm_reject_store_identity_mutation",
        false,
    ),
    trigger(
        "store_schema_metadata_no_mutation",
        "store_schema_metadata",
        "mfm_reject_store_identity_mutation",
        false,
    ),
    trigger(
        "tenant_fact_order_heads_guard",
        "tenant_fact_order_heads",
        "mfm_guard_tenant_fact_head_mutation",
        false,
    ),
    trigger(
        "tenant_fact_order_heads_integrity",
        "tenant_fact_order_heads",
        "mfm_validate_tenant_fact_heads",
        true,
    ),
];

const fn trigger(
    name: &'static str,
    table: &'static str,
    function: &'static str,
    deferred: bool,
) -> TriggerContract {
    TriggerContract {
        name,
        table,
        function,
        deferred,
    }
}

const REQUIRED_CONSTRAINTS: &[&str] = &[
    "artifact_admissions_artifact_id_v1",
    "artifact_admissions_blob_fk",
    "artifact_admissions_byte_length_v1",
    "artifact_admissions_content_digest_v1",
    "artifact_admissions_contract_digest_v1",
    "artifact_admissions_contract_schema_id_v1",
    "artifact_admissions_evidence_hash_v1",
    "artifact_admissions_identity_key",
    "artifact_admissions_media_type_v1",
    "artifact_admissions_pkey",
    "artifact_admissions_role_v1",
    "artifact_admissions_schema_id_v1",
    "artifact_admissions_semantic_type_v1",
    "artifact_admissions_value_ref_bounds_v1",
    "artifact_blobs_byte_length_v1",
    "artifact_blobs_content_digest_v1",
    "artifact_blobs_pkey",
    "commit_artifact_bindings_artifact_id_v1",
    "commit_artifact_bindings_authority_fk",
    "commit_artifact_bindings_authority_use_v1",
    "commit_artifact_bindings_commit_fk",
    "commit_artifact_bindings_content_digest_v1",
    "commit_artifact_bindings_evidence_hash_v1",
    "commit_artifact_bindings_field_path_v1",
    "commit_artifact_bindings_pkey",
    "commit_artifact_bindings_record_fk",
    "commit_artifact_bindings_value_ref_bounds_v1",
    "commit_object_authorities_admission_fk",
    "commit_object_authorities_admission_mode_v1",
    "commit_object_authorities_artifact_id_v1",
    "commit_object_authorities_commit_fk",
    "commit_object_authorities_content_digest_v1",
    "commit_object_authorities_evidence_hash_v1",
    "commit_object_authorities_identity_key",
    "commit_object_authorities_pkey",
    "commit_object_authorities_run_id_v1",
    "commit_object_authorities_sequence_v1",
    "commit_object_authorities_value_ref_bounds_v1",
    "configured_values_artifact_id_v1",
    "configured_values_binding_bounds_v1",
    "configured_values_content_digest_v1",
    "configured_values_entry_point_v1",
    "configured_values_evidence_hash_v1",
    "configured_values_object_fk",
    "configured_values_pkey",
    "configured_values_store_identity_fk",
    "configured_values_store_scope_v1",
    "configured_values_target_v1",
    "configured_values_tenant_scope_v1",
    "configured_values_value_ref_bounds_v1",
    "fact_scan_attestations_artifact_id_v1",
    "fact_scan_attestations_attestation_bounds_v1",
    "fact_scan_attestations_authorization_bounds_v1",
    "fact_scan_attestations_commit_fk",
    "fact_scan_attestations_commit_key",
    "fact_scan_attestations_content_digest_v1",
    "fact_scan_attestations_evidence_hash_v1",
    "fact_scan_attestations_head_bounds_v1",
    "fact_scan_attestations_object_fk",
    "fact_scan_attestations_observation_bounds_v1",
    "fact_scan_attestations_observation_ref_key",
    "fact_scan_attestations_pkey",
    "fact_scan_attestations_run_id_v1",
    "fact_scan_attestations_sequence_v1",
    "fact_scan_attestations_tenant_v1",
    "journal_commits_append_request_id_v1",
    "journal_commits_append_request_key",
    "journal_commits_admission_key_shape_v1",
    "journal_commits_candidate_digest_v1",
    "journal_commits_commit_digest_key",
    "journal_commits_commit_digest_v1",
    "journal_commits_committed_at_v1",
    "journal_commits_coordinate_shape_v1",
    "journal_commits_pkey",
    "journal_commits_predecessor_digest_v1",
    "journal_commits_predecessor_shape_v1",
    "journal_commits_record_count_v1",
    "journal_commits_run_id_v1",
    "journal_commits_sequence_v1",
    "journal_commits_tenant_v1",
    "journal_records_commit_fk",
    "journal_records_fact_order_v1",
    "journal_records_logical_key_key",
    "journal_records_logical_key_v1",
    "journal_records_ordinal_v1",
    "journal_records_payload_bounds_v1",
    "journal_records_pkey",
    "journal_records_record_hash_v1",
    "journal_records_record_id_key",
    "journal_records_record_id_v1",
    "journal_records_run_id_v1",
    "journal_records_schema_id_v1",
    "journal_records_sequence_v1",
    "journal_records_spec_hash_v1",
    "journal_records_tenant_v1",
    "qualified_support_members_artifact_id_v1",
    "qualified_support_members_content_digest_v1",
    "qualified_support_members_evidence_hash_v1",
    "qualified_support_members_field_path_v1",
    "qualified_support_members_object_fk",
    "qualified_support_members_pkey",
    "qualified_support_members_scope_v1",
    "qualified_support_members_value_ref_bounds_v1",
    "store_identity_epoch_v1",
    "store_identity_pkey",
    "store_identity_scope_v1",
    "store_identity_singleton_v1",
    "store_identity_store_scope_id_key",
    "store_schema_metadata_pkey",
    "store_schema_metadata_singleton_v1",
    "store_schema_metadata_version_v1",
    "tenant_fact_order_heads_order_v1",
    "tenant_fact_order_heads_pkey",
    "tenant_fact_order_heads_tenant_v1",
];
