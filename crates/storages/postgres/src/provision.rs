use sqlx::{Connection, PgConnection};

use crate::{
    evm_tx::{
        verify_evm_tx_privileges, verify_evm_tx_schema, EVM_TX_SCHEMA_CONTRACT, EVM_TX_SCHEMA_SQL,
    },
    mfm_relation_count, runtime_table_privilege_mask, verify_config_schema, verify_durability,
    verify_run_schema, AdminPostgresLocator, GateError, PostgresBackend,
    PostgresEvmTransactionAuthority, RuntimePostgresLocator, CONFIG_SCHEMA_SQL, RUN_SCHEMA_SQL,
    TABLE_DELETE, TABLE_INSERT, TABLE_SELECT, TABLE_UPDATE,
};

/// Redaction-safe split-authority schema provisioning failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProvisionError {
    /// The target, role, schema, ownership, or ACL posture is incompatible.
    #[error("postgres provisioning target is incompatible")]
    Incompatible,
    /// The target could not be observed or changed.
    #[error("postgres provisioning target is unavailable")]
    Unavailable,
    /// The provisioning transaction may have committed without acknowledgement.
    #[error("postgres provisioning outcome is indeterminate")]
    Indeterminate,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SchemaState {
    Absent,
    Present,
}

/// Installs or verifies run-history and configuration schemas with split credentials.
///
/// Target equivalence and the fixed runtime-role posture are proven before any
/// DDL. Existing installations are verified exactly and are never migrated,
/// repaired, re-owned, or reset.
pub async fn provision_postgres(
    admin: &AdminPostgresLocator,
    runtime: &RuntimePostgresLocator,
) -> Result<(), ProvisionError> {
    if admin.target() != runtime.target() {
        return Err(ProvisionError::Incompatible);
    }
    let admin_options = admin
        .connect_options("mfm-schema-provisioner")
        .map_err(|_| ProvisionError::Unavailable)?;
    let mut connection = PgConnection::connect_with(&admin_options)
        .await
        .map_err(|_| ProvisionError::Unavailable)?;
    verify_durability(&mut connection)
        .await
        .map_err(classify_gate)?;
    verify_target_role(&mut connection).await?;
    let owner: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&mut connection)
        .await
        .map_err(|_| ProvisionError::Unavailable)?;
    let run_state = inspect_run_schema(&mut connection, &owner).await?;
    let config_state = inspect_config_schema(&mut connection, &owner).await?;
    let states = [run_state, config_state];
    let fresh = states.iter().all(|state| *state == SchemaState::Absent);
    if !fresh && !states.iter().all(|state| *state == SchemaState::Present) {
        return Err(ProvisionError::Incompatible);
    }

    if fresh {
        let mut transaction = connection
            .begin()
            .await
            .map_err(|_| ProvisionError::Unavailable)?;
        sqlx::query("SET LOCAL synchronous_commit = on")
            .execute(&mut *transaction)
            .await
            .map_err(|_| ProvisionError::Unavailable)?;
        apply_database_acl(&mut transaction).await?;
        sqlx::raw_sql(
            "REVOKE ALL ON SCHEMA public FROM PUBLIC, mfm_runtime; \
             GRANT USAGE ON SCHEMA public TO mfm_runtime",
        )
        .execute(&mut *transaction)
        .await
        .map_err(|_| ProvisionError::Unavailable)?;
        sqlx::raw_sql(RUN_SCHEMA_SQL)
            .execute(&mut *transaction)
            .await
            .map_err(|_| ProvisionError::Unavailable)?;
        sqlx::raw_sql(CONFIG_SCHEMA_SQL)
            .execute(&mut *transaction)
            .await
            .map_err(|_| ProvisionError::Unavailable)?;
        match transaction.commit().await {
            Ok(()) => {}
            Err(error) if error.as_database_error().is_some() => {
                return Err(ProvisionError::Unavailable)
            }
            Err(_) => return Err(ProvisionError::Indeterminate),
        }
    }

    verify_run_schema(&mut connection)
        .await
        .map_err(classify_gate)?;
    verify_config_schema(&mut connection)
        .await
        .map_err(classify_gate)?;
    verify_owned_objects(&mut connection, &owner, "public").await?;
    verify_owned_objects(&mut connection, &owner, "mfm_config").await?;
    verify_runtime_base_grants(&mut connection).await?;
    drop(connection);

    let backend = PostgresBackend::connect(runtime)
        .await
        .map_err(classify_open)?;
    drop(backend);
    Ok(())
}

/// Installs or verifies the optional EVM transaction-authority schema.
pub async fn provision_evm_transaction_authority(
    admin: &AdminPostgresLocator,
    runtime: &RuntimePostgresLocator,
) -> Result<(), ProvisionError> {
    if admin.target() != runtime.target() {
        return Err(ProvisionError::Incompatible);
    }
    let admin_options = admin
        .connect_options("mfm-evm-authority-provisioner")
        .map_err(|_| ProvisionError::Unavailable)?;
    let mut connection = PgConnection::connect_with(&admin_options)
        .await
        .map_err(|_| ProvisionError::Unavailable)?;
    verify_durability(&mut connection)
        .await
        .map_err(classify_gate)?;
    verify_target_role(&mut connection).await?;
    let owner: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&mut connection)
        .await
        .map_err(|_| ProvisionError::Unavailable)?;
    let state = inspect_evm_tx_schema(&mut connection, &owner).await?;
    if state == SchemaState::Absent {
        let mut epoch = [0_u8; 32];
        getrandom::fill(&mut epoch).map_err(|_| ProvisionError::Unavailable)?;
        let mut transaction = connection
            .begin()
            .await
            .map_err(|_| ProvisionError::Unavailable)?;
        sqlx::query("SET LOCAL synchronous_commit = on")
            .execute(&mut *transaction)
            .await
            .map_err(|_| ProvisionError::Unavailable)?;
        apply_database_acl(&mut transaction).await?;
        sqlx::raw_sql(EVM_TX_SCHEMA_SQL)
            .execute(&mut *transaction)
            .await
            .map_err(|_| ProvisionError::Unavailable)?;
        sqlx::query(
            "INSERT INTO mfm_evm_tx.mfm_evm_tx_schema (schema_contract, authority_epoch) \
             VALUES ($1, $2)",
        )
        .bind(EVM_TX_SCHEMA_CONTRACT)
        .bind(epoch.as_slice())
        .execute(&mut *transaction)
        .await
        .map_err(|_| ProvisionError::Unavailable)?;
        match transaction.commit().await {
            Ok(()) => {}
            Err(error) if error.as_database_error().is_some() => {
                return Err(ProvisionError::Unavailable)
            }
            Err(_) => return Err(ProvisionError::Indeterminate),
        }
    }
    verify_evm_tx_schema(&mut connection)
        .await
        .map_err(classify_gate)?;
    verify_owned_objects(&mut connection, &owner, "mfm_evm_tx").await?;
    verify_runtime_evm_tx_grants(&mut connection).await?;
    drop(connection);

    let authority = PostgresEvmTransactionAuthority::connect(runtime)
        .await
        .map_err(classify_open)?;
    drop(authority);
    Ok(())
}

async fn verify_target_role(connection: &mut PgConnection) -> Result<(), ProvisionError> {
    let role: Option<(bool, bool, bool, bool, bool, bool, bool)> = sqlx::query_as(
        "SELECT rolsuper, rolinherit, rolcreaterole, rolcreatedb, rolcanlogin, \
                rolreplication, rolbypassrls \
         FROM pg_catalog.pg_roles WHERE rolname = 'mfm_runtime'",
    )
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| ProvisionError::Unavailable)?;
    if role != Some((false, false, false, false, true, false, false)) {
        return Err(ProvisionError::Incompatible);
    }
    let memberships: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_auth_members m \
         JOIN pg_catalog.pg_roles r ON r.oid = m.member OR r.oid = m.roleid \
         WHERE r.rolname = 'mfm_runtime'",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| ProvisionError::Unavailable)?;
    if memberships != 0 {
        return Err(ProvisionError::Incompatible);
    }
    let owns: bool = sqlx::query_scalar(
        "SELECT EXISTS ( \
           SELECT 1 FROM pg_catalog.pg_database d \
            WHERE d.datname = current_database() AND pg_get_userbyid(d.datdba) = 'mfm_runtime' \
           UNION ALL \
           SELECT 1 FROM pg_catalog.pg_namespace n \
            WHERE pg_get_userbyid(n.nspowner) = 'mfm_runtime' \
           UNION ALL \
           SELECT 1 FROM pg_catalog.pg_class c \
            JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
            WHERE pg_get_userbyid(c.relowner) = 'mfm_runtime' \
              AND n.nspname NOT IN ('pg_catalog', 'information_schema') \
         )",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| ProvisionError::Unavailable)?;
    if owns {
        return Err(ProvisionError::Incompatible);
    }
    Ok(())
}

async fn inspect_run_schema(
    connection: &mut PgConnection,
    owner: &str,
) -> Result<SchemaState, ProvisionError> {
    verify_schema_owner(connection, owner, "public").await?;
    let count = mfm_relation_count(connection)
        .await
        .map_err(classify_gate)?;
    if count == 0 {
        return Ok(SchemaState::Absent);
    }
    verify_run_schema(connection).await.map_err(classify_gate)?;
    verify_owned_objects(connection, owner, "public").await?;
    verify_runtime_table_grants(connection, true).await?;
    Ok(SchemaState::Present)
}

async fn inspect_config_schema(
    connection: &mut PgConnection,
    owner: &str,
) -> Result<SchemaState, ProvisionError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname = 'mfm_config')",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| ProvisionError::Unavailable)?;
    if !exists {
        return Ok(SchemaState::Absent);
    }
    verify_config_schema(connection)
        .await
        .map_err(classify_gate)?;
    verify_owned_objects(connection, owner, "mfm_config").await?;
    verify_runtime_table_grants(connection, false).await?;
    Ok(SchemaState::Present)
}

async fn inspect_evm_tx_schema(
    connection: &mut PgConnection,
    owner: &str,
) -> Result<SchemaState, ProvisionError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname = 'mfm_evm_tx')",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| ProvisionError::Unavailable)?;
    if !exists {
        return Ok(SchemaState::Absent);
    }
    verify_evm_tx_schema(connection)
        .await
        .map_err(classify_gate)?;
    verify_owned_objects(connection, owner, "mfm_evm_tx").await?;
    verify_runtime_evm_tx_grants(connection).await?;
    Ok(SchemaState::Present)
}

async fn verify_owned_objects(
    connection: &mut PgConnection,
    owner: &str,
    schema: &str,
) -> Result<(), ProvisionError> {
    let wrong_owner: bool = sqlx::query_scalar(
        "SELECT EXISTS( \
           SELECT 1 FROM pg_catalog.pg_class c \
           JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
           WHERE n.nspname = $1 \
             AND c.relname IN ('mfm_store_schema','mfm_run_frames','mfm_run_heads', \
                               'mfm_config_schema','config_revisions','mfm_evm_tx_schema', \
                               'nonce_domains','nonce_reservations','prepared_transactions', \
                               'transaction_settlements') \
             AND pg_get_userbyid(c.relowner) <> $2)",
    )
    .bind(schema)
    .bind(owner)
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| ProvisionError::Unavailable)?;
    if wrong_owner {
        return Err(ProvisionError::Incompatible);
    }
    verify_schema_owner(connection, owner, schema).await
}

async fn verify_schema_owner(
    connection: &mut PgConnection,
    owner: &str,
    schema: &str,
) -> Result<(), ProvisionError> {
    let retained: Option<String> = sqlx::query_scalar(
        "SELECT pg_get_userbyid(nspowner) FROM pg_catalog.pg_namespace WHERE nspname = $1",
    )
    .bind(schema)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| ProvisionError::Unavailable)?;
    if retained.as_deref() != Some(owner) {
        return Err(ProvisionError::Incompatible);
    }
    Ok(())
}

async fn verify_runtime_base_grants(connection: &mut PgConnection) -> Result<(), ProvisionError> {
    let database: (bool, bool, bool) = sqlx::query_as(
        "SELECT has_database_privilege('mfm_runtime', current_database(), 'CONNECT'), \
                has_database_privilege('mfm_runtime', current_database(), 'CREATE'), \
                has_database_privilege('mfm_runtime', current_database(), 'TEMPORARY')",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| ProvisionError::Unavailable)?;
    if database != (true, false, false) {
        return Err(ProvisionError::Incompatible);
    }
    for schema in ["public", "mfm_config"] {
        let privileges: (bool, bool) = sqlx::query_as(
            "SELECT has_schema_privilege('mfm_runtime', $1, 'USAGE'), \
                    has_schema_privilege('mfm_runtime', $1, 'CREATE')",
        )
        .bind(schema)
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| ProvisionError::Unavailable)?;
        if privileges != (true, false) {
            return Err(ProvisionError::Incompatible);
        }
    }
    verify_runtime_table_grants(connection, true).await?;
    verify_runtime_table_grants(connection, false).await
}

async fn verify_runtime_evm_tx_grants(connection: &mut PgConnection) -> Result<(), ProvisionError> {
    let database: (bool, bool, bool) = sqlx::query_as(
        "SELECT has_database_privilege('mfm_runtime', current_database(), 'CONNECT'), \
                has_database_privilege('mfm_runtime', current_database(), 'CREATE'), \
                has_database_privilege('mfm_runtime', current_database(), 'TEMPORARY')",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| ProvisionError::Unavailable)?;
    let schema: (bool, bool) = sqlx::query_as(
        "SELECT has_schema_privilege('mfm_runtime', 'mfm_evm_tx', 'USAGE'), \
                has_schema_privilege('mfm_runtime', 'mfm_evm_tx', 'CREATE')",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| ProvisionError::Unavailable)?;
    if database != (true, false, false) || schema != (true, false) {
        return Err(ProvisionError::Incompatible);
    }
    if !verify_evm_tx_privileges(connection)
        .await
        .map_err(classify_gate)?
    {
        return Err(ProvisionError::Incompatible);
    }
    Ok(())
}

async fn verify_runtime_table_grants(
    connection: &mut PgConnection,
    run: bool,
) -> Result<(), ProvisionError> {
    let expected = if run {
        [
            ("public.mfm_run_frames", TABLE_SELECT | TABLE_INSERT),
            (
                "public.mfm_run_heads",
                TABLE_SELECT | TABLE_INSERT | TABLE_UPDATE,
            ),
            ("public.mfm_store_schema", TABLE_SELECT),
        ]
        .as_slice()
    } else {
        [
            (
                "mfm_config.config_revisions",
                TABLE_SELECT | TABLE_INSERT | TABLE_DELETE,
            ),
            ("mfm_config.mfm_config_schema", TABLE_SELECT),
        ]
        .as_slice()
    };
    for (table, expected_mask) in expected {
        let retained = runtime_table_privilege_mask(connection, table)
            .await
            .map_err(classify_gate)?;
        if retained != *expected_mask {
            return Err(ProvisionError::Incompatible);
        }
    }
    Ok(())
}

async fn apply_database_acl(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), ProvisionError> {
    let revoke: String = sqlx::query_scalar(
        "SELECT format('REVOKE ALL PRIVILEGES ON DATABASE %I FROM PUBLIC, mfm_runtime', current_database())",
    )
    .fetch_one(&mut **transaction)
    .await
    .map_err(|_| ProvisionError::Unavailable)?;
    // PostgreSQL itself quotes the observed database identifier with `%I`; no caller text is
    // interpolated into either statement.
    sqlx::query(sqlx::AssertSqlSafe(revoke.as_str()))
        .execute(&mut **transaction)
        .await
        .map_err(|_| ProvisionError::Unavailable)?;
    let grant: String = sqlx::query_scalar(
        "SELECT format('GRANT CONNECT ON DATABASE %I TO mfm_runtime', current_database())",
    )
    .fetch_one(&mut **transaction)
    .await
    .map_err(|_| ProvisionError::Unavailable)?;
    sqlx::query(sqlx::AssertSqlSafe(grant.as_str()))
        .execute(&mut **transaction)
        .await
        .map_err(|_| ProvisionError::Unavailable)?;
    Ok(())
}

const fn classify_gate(error: GateError) -> ProvisionError {
    match error {
        GateError::Incompatible => ProvisionError::Incompatible,
        GateError::Unavailable => ProvisionError::Unavailable,
    }
}

const fn classify_open(error: crate::PostgresOpenError) -> ProvisionError {
    match error {
        crate::PostgresOpenError::Incompatible => ProvisionError::Incompatible,
        crate::PostgresOpenError::Unavailable => ProvisionError::Unavailable,
    }
}
