use sqlx::{Connection, PgConnection};

use crate::{
    catalog::{surface_exists, verify_surface, CONFIG_SURFACE, EVM_TX_SURFACE, RUN_SURFACE},
    evm_tx::{load_evm_tx_epoch, EVM_TX_SCHEMA_CONTRACT, EVM_TX_SCHEMA_SQL},
    verify_config_marker, verify_durability, verify_run_marker, AdminPostgresLocator, GateError,
    PostgresBackend, PostgresEvmTransactionAuthority, RuntimePostgresLocator, CONFIG_SCHEMA_SQL,
    RUN_SCHEMA_SQL,
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
    let owner: String = sqlx::query_scalar!("SELECT current_user AS \"value!\"")
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
        sqlx::query!("SET LOCAL synchronous_commit = on")
            .execute(&mut *transaction)
            .await
            .map_err(|_| ProvisionError::Unavailable)?;
        apply_database_acl(&mut transaction).await?;
        sqlx::query!("REVOKE ALL ON SCHEMA public FROM PUBLIC, mfm_runtime")
            .execute(&mut *transaction)
            .await
            .map_err(|_| ProvisionError::Unavailable)?;
        sqlx::query!("GRANT USAGE ON SCHEMA public TO mfm_runtime")
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

    verify_surface(&mut connection, &RUN_SURFACE, Some(&owner))
        .await
        .map_err(classify_gate)?;
    verify_run_marker(&mut connection)
        .await
        .map_err(classify_gate)?;
    verify_surface(&mut connection, &CONFIG_SURFACE, Some(&owner))
        .await
        .map_err(classify_gate)?;
    verify_config_marker(&mut connection)
        .await
        .map_err(classify_gate)?;
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
    let owner: String = sqlx::query_scalar!("SELECT current_user AS \"value!\"")
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
        sqlx::query!("SET LOCAL synchronous_commit = on")
            .execute(&mut *transaction)
            .await
            .map_err(|_| ProvisionError::Unavailable)?;
        apply_database_acl(&mut transaction).await?;
        sqlx::raw_sql(EVM_TX_SCHEMA_SQL)
            .execute(&mut *transaction)
            .await
            .map_err(|_| ProvisionError::Unavailable)?;
        sqlx::query!(
            "INSERT INTO mfm_evm_tx.mfm_evm_tx_schema (schema_contract, \
             authority_epoch) VALUES ($1, $2)",
            EVM_TX_SCHEMA_CONTRACT,
            epoch.as_slice(),
        )
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
    verify_surface(&mut connection, &EVM_TX_SURFACE, Some(&owner))
        .await
        .map_err(classify_gate)?;
    load_evm_tx_epoch(&mut connection)
        .await
        .map_err(classify_gate)?;
    verify_runtime_evm_tx_grants(&mut connection).await?;
    drop(connection);

    let authority = PostgresEvmTransactionAuthority::connect(runtime)
        .await
        .map_err(classify_open)?;
    drop(authority);
    Ok(())
}

async fn verify_target_role(connection: &mut PgConnection) -> Result<(), ProvisionError> {
    let role: Option<(bool, bool, bool, bool, bool, bool, bool)> = sqlx::query!(
        "SELECT rolsuper AS \"superuser!\", rolinherit AS \"inherit!\", \
         rolcreaterole AS \"create_role!\", rolcreatedb AS \"create_db!\", \
         rolcanlogin AS \"login!\", rolreplication AS \"replication!\", \
         rolbypassrls AS \"bypass_rls!\" FROM pg_catalog.pg_roles WHERE rolname = \
         'mfm_runtime'",
    )
    .fetch_optional(&mut *connection)
    .await
    .map(|rows| {
        rows.map(|row| {
            (
                row.superuser,
                row.inherit,
                row.create_role,
                row.create_db,
                row.login,
                row.replication,
                row.bypass_rls,
            )
        })
    })
    .map_err(|_| ProvisionError::Unavailable)?;
    if role != Some((false, false, false, false, true, false, false)) {
        return Err(ProvisionError::Incompatible);
    }
    let memberships: i64 = sqlx::query_scalar!(
        "SELECT count(*) AS \"value!\" FROM pg_catalog.pg_auth_members m JOIN \
         pg_catalog.pg_roles r ON r.oid = m.member OR r.oid = m.roleid WHERE \
         r.rolname = 'mfm_runtime'",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| ProvisionError::Unavailable)?;
    if memberships != 0 {
        return Err(ProvisionError::Incompatible);
    }
    let owns: bool = sqlx::query_scalar!(
        "SELECT EXISTS ( SELECT 1 FROM pg_catalog.pg_database d WHERE d.datname \
         = current_database() AND pg_get_userbyid(d.datdba) = 'mfm_runtime' UNION \
         ALL SELECT 1 FROM pg_catalog.pg_namespace n WHERE \
         pg_get_userbyid(n.nspowner) = 'mfm_runtime' UNION ALL SELECT 1 FROM \
         pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid = \
         c.relnamespace WHERE pg_get_userbyid(c.relowner) = 'mfm_runtime' AND \
         n.nspname NOT IN ('pg_catalog', 'information_schema') ) AS \"value!\"",
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
    let exists = surface_exists(connection, &RUN_SURFACE)
        .await
        .map_err(classify_gate)?;
    if !exists {
        return Ok(SchemaState::Absent);
    }
    verify_surface(connection, &RUN_SURFACE, Some(owner))
        .await
        .map_err(classify_gate)?;
    verify_run_marker(connection).await.map_err(classify_gate)?;
    Ok(SchemaState::Present)
}

async fn inspect_config_schema(
    connection: &mut PgConnection,
    owner: &str,
) -> Result<SchemaState, ProvisionError> {
    let exists = surface_exists(connection, &CONFIG_SURFACE)
        .await
        .map_err(classify_gate)?;
    if !exists {
        return Ok(SchemaState::Absent);
    }
    verify_schema_owner(connection, owner, CONFIG_SURFACE.schema()).await?;
    verify_surface(connection, &CONFIG_SURFACE, Some(owner))
        .await
        .map_err(classify_gate)?;
    verify_config_marker(connection)
        .await
        .map_err(classify_gate)?;
    Ok(SchemaState::Present)
}

async fn inspect_evm_tx_schema(
    connection: &mut PgConnection,
    owner: &str,
) -> Result<SchemaState, ProvisionError> {
    let exists = surface_exists(connection, &EVM_TX_SURFACE)
        .await
        .map_err(classify_gate)?;
    if !exists {
        return Ok(SchemaState::Absent);
    }
    verify_schema_owner(connection, owner, EVM_TX_SURFACE.schema()).await?;
    verify_surface(connection, &EVM_TX_SURFACE, Some(owner))
        .await
        .map_err(classify_gate)?;
    load_evm_tx_epoch(connection).await.map_err(classify_gate)?;
    Ok(SchemaState::Present)
}

async fn verify_schema_owner(
    connection: &mut PgConnection,
    owner: &str,
    schema: &str,
) -> Result<(), ProvisionError> {
    let retained: Option<String> = sqlx::query_scalar!(
        "SELECT pg_get_userbyid(nspowner) AS \"value!\" FROM \
         pg_catalog.pg_namespace WHERE nspname = $1",
        schema,
    )
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| ProvisionError::Unavailable)?;
    if retained.as_deref() != Some(owner) {
        return Err(ProvisionError::Incompatible);
    }
    Ok(())
}

async fn verify_runtime_base_grants(connection: &mut PgConnection) -> Result<(), ProvisionError> {
    verify_runtime_database_grants(connection).await
}

async fn verify_runtime_evm_tx_grants(connection: &mut PgConnection) -> Result<(), ProvisionError> {
    verify_runtime_database_grants(connection).await
}

async fn verify_runtime_database_grants(
    connection: &mut PgConnection,
) -> Result<(), ProvisionError> {
    let database: (bool, bool, bool) = sqlx::query!(
        "SELECT has_database_privilege('mfm_runtime', current_database(), \
         'CONNECT') AS \"connect!\", has_database_privilege('mfm_runtime', \
         current_database(), 'CREATE') AS \"create!\", \
         has_database_privilege('mfm_runtime', current_database(), 'TEMPORARY') \
         AS \"temporary!\"",
    )
    .fetch_one(&mut *connection)
    .await
    .map(|rows| (rows.connect, rows.create, rows.temporary))
    .map_err(|_| ProvisionError::Unavailable)?;
    if database != (true, false, false) {
        return Err(ProvisionError::Incompatible);
    }
    Ok(())
}

async fn apply_database_acl(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), ProvisionError> {
    let revoke: String = sqlx::query_scalar!(
        "SELECT format('REVOKE ALL PRIVILEGES ON DATABASE %I FROM PUBLIC, \
         mfm_runtime', current_database()) AS \"value!\"",
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
    let grant: String = sqlx::query_scalar!(
        "SELECT format('GRANT CONNECT ON DATABASE %I TO mfm_runtime', \
         current_database()) AS \"value!\"",
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
