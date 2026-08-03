//! Administrative schema installation and closed production qualification.

use ring::digest::{digest, SHA256};
use sqlx::{PgConnection, PgPool, Row};

use crate::error::{PostgresEvmWalletError, Result};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub(crate) const SCHEMA_CONTRACT_VERSION: &str = "mfm.evm.wallet-authority-postgres.v1";
pub(crate) const ACTIVATION_ADMIN_ROLE: &str = "mfm_evm_wallet_activation_admin";
pub(crate) const ACTIVATION_PUBLIC_ROLE: &str = "mfm_evm_wallet_activation_public";
pub(crate) const NONCE_APPLICATION_ROLE: &str = "mfm_evm_wallet_nonce_application";
const OWNER_ROLE: &str = "mfm_evm_wallet_owner";
const TEST_ROLE: &str = "mfm_evm_wallet_test";
const STORE_APPLICATION_ROLE: &str = "mfm_store_application";
const WALLET_ROLES: [&str; 5] = [
    OWNER_ROLE,
    ACTIVATION_ADMIN_ROLE,
    ACTIVATION_PUBLIC_ROLE,
    NONCE_APPLICATION_ROLE,
    TEST_ROLE,
];
const WALLET_RUNTIME_ROLES: [&str; 3] = [
    ACTIVATION_ADMIN_ROLE,
    ACTIVATION_PUBLIC_ROLE,
    NONCE_APPLICATION_ROLE,
];

const REQUIRED_TABLES: &[&str] = &[
    "wallet_domain_activations",
    "wallet_nonce_candidates",
    "wallet_nonce_completions",
    "wallet_nonce_domains",
    "wallet_nonce_reservations",
    "wallet_store_incarnations",
    "wallet_store_lineage_heads",
    "wallet_store_schema_metadata",
];

// These manifests cover every field returned by the corresponding catalog query. They are
// generated from the sole compiled baseline under PostgreSQL 16, whose version is pinned by the
// development and verification environment. A same-name weakened constraint, trigger, function,
// owner, or column therefore cannot pass qualification.
const COLUMN_MANIFEST_SHA256: &str =
    "3c98650a03fafdfd43072de71528b8f00f620834caee26812fb7183fd1c97542";
const CONSTRAINT_MANIFEST_SHA256: &str =
    "f2685f075dab63c2d088590533f20364c189ef329a9781b0672c8d3549d7f8ba";
const TRIGGER_MANIFEST_SHA256: &str =
    "e19f09ad27003b64c8aa75c0a0ba0c3450414e9ca5a3f3487a227c4f0c3c1b22";
const FUNCTION_MANIFEST_SHA256: &str =
    "c12159cefc0c3714f9245f09c17e4165cacfa530e3b943871fa27070cf5066ac";
const TABLE_MANIFEST_SHA256: &str =
    "3061b01017092161b7f967f5a5a0938c9539411931036ab26b6817694ac84a4b";
const ACL_MANIFEST_SHA256: &str =
    "ad9916df4a5ab866bc30f3a39d66d9498bf9f5d0a0d99b121a7296d33d88b35e";
const MIGRATION_LEDGER_SHAPE_SHA256: &str =
    "9ebcbe4772ee5d894dcf92eff3f53aa6a7027d418c80e35c4486cd0540aae8eb";

struct QualifiedSession {
    login_role: String,
    schema_owner: String,
}

/// Administrative installer for the one current wallet-authority baseline.
pub struct PostgresEvmWalletSchema;

impl PostgresEvmWalletSchema {
    /// Applies the compiled migration through a migration-owner connection.
    pub async fn migrate(database_url: &str) -> Result<()> {
        let pool = PgPool::connect(database_url)
            .await
            .map_err(|_| PostgresEvmWalletError::Unavailable)?;
        MIGRATOR
            .run(&pool)
            .await
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)
    }
}

pub(crate) async fn validate_wallet_schema(
    pool: &PgPool,
    expected_schema: &str,
    expected_role: &str,
) -> Result<()> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    sqlx::query("SET TRANSACTION READ WRITE")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    let connection = &mut *transaction;
    let session = pin_and_probe_primary(connection, expected_schema, expected_role).await?;
    validate_migration_ledger(connection).await?;
    validate_catalog_shape(connection).await?;
    validate_roles_and_privileges(connection, expected_role, &session).await?;
    validate_closed_acls(connection).await?;
    validate_metadata(connection).await?;
    validate_prefix_integrity(connection, expected_role).await?;
    transaction
        .commit()
        .await
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)
}

async fn pin_and_probe_primary(
    connection: &mut PgConnection,
    expected_schema: &str,
    expected_role: &str,
) -> Result<QualifiedSession> {
    sqlx::query(
        "SELECT pg_catalog.set_config( \
             'search_path', pg_catalog.format('%I, pg_catalog', $1), TRUE \
         )",
    )
    .bind(expected_schema)
    .execute(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    let row = sqlx::query(
        "SELECT current_database()::text AS database_name, \
                current_schema()::text AS schema_name, \
                session_user::text AS login_role, current_user::text AS role_name, \
                owner.rolname AS schema_owner, \
                (SELECT oid::bigint FROM pg_catalog.pg_database \
                  WHERE datname = current_database()) AS database_oid, \
                pg_catalog.pg_is_in_recovery() AS in_recovery, \
                current_setting('transaction_read_only') AS transaction_read_only \
         FROM pg_catalog.pg_namespace AS namespace \
         JOIN pg_catalog.pg_roles AS owner ON owner.oid = namespace.nspowner \
         WHERE namespace.nspname = current_schema()",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    let database_name = text(&row, "database_name")?;
    let database_oid = row
        .try_get::<i64, _>("database_oid")
        .ok()
        .and_then(|value| u32::try_from(value).ok());
    let login_role = text(&row, "login_role")?;
    let schema_owner = text(&row, "schema_owner")?;
    if database_name.is_empty()
        || database_name.len() > 63
        || database_oid.is_none()
        || !WALLET_RUNTIME_ROLES.contains(&expected_role)
        || text(&row, "schema_name")? != expected_schema
        || text(&row, "role_name")? != expected_role
        || login_role == expected_role
        || login_role == schema_owner
        || WALLET_ROLES.iter().any(|role| *role == login_role)
        || row.try_get::<bool, _>("in_recovery").unwrap_or(true)
        || text(&row, "transaction_read_only")? != "off"
    {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    Ok(QualifiedSession {
        login_role,
        schema_owner,
    })
}

async fn validate_migration_ledger(connection: &mut PgConnection) -> Result<()> {
    let rows =
        sqlx::query("SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&mut *connection)
            .await
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    if rows.len() != MIGRATOR.iter().count() {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    for migration in MIGRATOR.iter() {
        let row = rows
            .iter()
            .find(|row| row.try_get::<i64, _>("version").ok() == Some(migration.version))
            .ok_or(PostgresEvmWalletError::InvalidAuthority)?;
        let success = row
            .try_get::<bool, _>("success")
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        let checksum = row
            .try_get::<Vec<u8>, _>("checksum")
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
        if !success || checksum.as_slice() != migration.checksum.as_ref() {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
    }
    Ok(())
}

async fn validate_catalog_shape(connection: &mut PgConnection) -> Result<()> {
    let tables = sqlx::query_scalar::<_, String>(
        "SELECT relation.relname FROM pg_catalog.pg_class AS relation \
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
         WHERE namespace.nspname = current_schema() AND relation.relkind = 'r' \
           AND relation.relname <> '_sqlx_migrations' ORDER BY relation.relname",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    if tables != REQUIRED_TABLES {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    let extra_relations = sqlx::query_scalar::<_, i64>(
        "SELECT count(*)::bigint FROM pg_catalog.pg_class AS relation \
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
         WHERE namespace.nspname = current_schema() \
           AND relation.relkind IN ('v', 'm', 'S', 'f', 'p')",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    if extra_relations != 0 {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }

    let columns = sqlx::query(
        "SELECT relation.relname AS table_name, attribute.attname AS column_name, \
                column_type.typname AS udt_name, \
                CASE WHEN attribute.attnotnull THEN 'NO' ELSE 'YES' END AS is_nullable \
         FROM pg_catalog.pg_attribute AS attribute \
         JOIN pg_catalog.pg_class AS relation ON relation.oid = attribute.attrelid \
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
         JOIN pg_catalog.pg_type AS column_type ON column_type.oid = attribute.atttypid \
         WHERE namespace.nspname = current_schema() AND relation.relkind = 'r' \
           AND relation.relname <> '_sqlx_migrations' \
           AND attribute.attnum > 0 AND NOT attribute.attisdropped \
         ORDER BY relation.relname, attribute.attnum",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    require_manifest(
        columns.iter().map(|row| {
            vec![
                text(row, "table_name"),
                text(row, "column_name"),
                text(row, "udt_name"),
                text(row, "is_nullable"),
            ]
        }),
        COLUMN_MANIFEST_SHA256,
    )?;

    let constraints = sqlx::query(
        "SELECT constraint_row.conname, constraint_row.contype::text AS contype, \
                pg_catalog.pg_get_constraintdef(constraint_row.oid, TRUE) AS definition \
         FROM pg_catalog.pg_constraint AS constraint_row \
         JOIN pg_catalog.pg_class AS relation ON relation.oid = constraint_row.conrelid \
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
         WHERE namespace.nspname = current_schema() \
           AND relation.relname <> '_sqlx_migrations' ORDER BY constraint_row.conname",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    require_manifest(
        constraints.iter().map(|row| {
            vec![
                text(row, "conname"),
                text(row, "contype"),
                text(row, "definition"),
            ]
        }),
        CONSTRAINT_MANIFEST_SHA256,
    )?;

    let triggers = sqlx::query(
        "SELECT trigger_row.tgname, relation.relname, trigger_row.tgenabled::text AS enabled, \
                pg_catalog.pg_get_triggerdef(trigger_row.oid, TRUE) AS definition, \
                procedure.proname \
         FROM pg_catalog.pg_trigger AS trigger_row \
         JOIN pg_catalog.pg_class AS relation ON relation.oid = trigger_row.tgrelid \
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
         JOIN pg_catalog.pg_proc AS procedure ON procedure.oid = trigger_row.tgfoid \
         WHERE namespace.nspname = current_schema() AND NOT trigger_row.tgisinternal \
         ORDER BY trigger_row.tgname",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    require_manifest(
        triggers.iter().map(|row| {
            vec![
                text(row, "tgname"),
                text(row, "relname"),
                text(row, "enabled"),
                text(row, "definition"),
                text(row, "proname"),
            ]
        }),
        TRIGGER_MANIFEST_SHA256,
    )?;

    let functions = sqlx::query(
        "SELECT procedure.proname, language.lanname, owner.rolname, \
                pg_catalog.pg_get_function_result(procedure.oid) AS result, \
                pg_catalog.pg_get_function_identity_arguments(procedure.oid) AS arguments, \
                procedure.provolatile::text AS volatility, \
                procedure.prosecdef::text AS security_definer, procedure.prosrc \
         FROM pg_catalog.pg_proc AS procedure \
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = procedure.pronamespace \
         JOIN pg_catalog.pg_language AS language ON language.oid = procedure.prolang \
         JOIN pg_catalog.pg_roles AS owner ON owner.oid = procedure.proowner \
         WHERE namespace.nspname = current_schema() ORDER BY procedure.proname",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    require_manifest(
        functions.iter().map(|row| {
            vec![
                text(row, "proname"),
                text(row, "lanname"),
                text(row, "rolname"),
                text(row, "result"),
                text(row, "arguments"),
                text(row, "volatility"),
                text(row, "security_definer"),
                text(row, "prosrc"),
            ]
        }),
        FUNCTION_MANIFEST_SHA256,
    )?;

    let table_manifest = sqlx::query(
        "SELECT relation.relname, owner.rolname, relation.relkind::text AS relkind, \
                relation.relpersistence::text AS persistence, \
                relation.relrowsecurity::text AS row_security, \
                relation.relforcerowsecurity::text AS force_row_security \
         FROM pg_catalog.pg_class AS relation \
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
         JOIN pg_catalog.pg_roles AS owner ON owner.oid = relation.relowner \
         WHERE namespace.nspname = current_schema() AND relation.relkind = 'r' \
           AND relation.relname <> '_sqlx_migrations' ORDER BY relation.relname",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    require_manifest(
        table_manifest.iter().map(|row| {
            vec![
                text(row, "relname"),
                text(row, "rolname"),
                text(row, "relkind"),
                text(row, "persistence"),
                text(row, "row_security"),
                text(row, "force_row_security"),
            ]
        }),
        TABLE_MANIFEST_SHA256,
    )
}

fn require_manifest<I>(rows: I, expected: &str) -> Result<()>
where
    I: IntoIterator<Item = Vec<Result<String>>>,
{
    let mut manifest = Vec::new();
    for row in rows {
        let mut first = true;
        for field in row {
            if !first {
                manifest.push(0x1f);
            }
            first = false;
            manifest.extend_from_slice(field?.as_bytes());
        }
        manifest.push(b'\n');
    }
    let actual = hex::encode(digest(&SHA256, &manifest).as_ref());
    if actual != expected {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    Ok(())
}

async fn validate_roles_and_privileges(
    connection: &mut PgConnection,
    expected_role: &str,
    session: &QualifiedSession,
) -> Result<()> {
    let managed_roles = [
        ACTIVATION_ADMIN_ROLE,
        ACTIVATION_PUBLIC_ROLE,
        NONCE_APPLICATION_ROLE,
        OWNER_ROLE,
        STORE_APPLICATION_ROLE,
        TEST_ROLE,
    ];
    let roles = sqlx::query(
        "SELECT rolname, rolsuper, rolinherit, rolcreaterole, rolcreatedb, rolcanlogin, \
                rolreplication, rolbypassrls FROM pg_catalog.pg_roles \
         WHERE rolname = ANY($1) ORDER BY rolname",
    )
    .bind(
        managed_roles
            .iter()
            .map(|role| (*role).to_owned())
            .collect::<Vec<_>>(),
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    if roles.len() != managed_roles.len() {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    for role in roles {
        if role.try_get::<bool, _>("rolsuper").unwrap_or(true)
            || !role.try_get::<bool, _>("rolinherit").unwrap_or(false)
            || role.try_get::<bool, _>("rolcreaterole").unwrap_or(true)
            || role.try_get::<bool, _>("rolcreatedb").unwrap_or(true)
            || role.try_get::<bool, _>("rolcanlogin").unwrap_or(true)
            || role.try_get::<bool, _>("rolreplication").unwrap_or(true)
            || role.try_get::<bool, _>("rolbypassrls").unwrap_or(true)
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
    }
    validate_role_memberships(connection, expected_role, session).await?;

    for role in managed_roles {
        let expected_usage = role != STORE_APPLICATION_ROLE;
        let expected_create = role == OWNER_ROLE;
        for (privilege, expected) in [("USAGE", expected_usage), ("CREATE", expected_create)] {
            let granted = sqlx::query_scalar::<_, bool>(
                "SELECT pg_catalog.has_schema_privilege($1, current_schema(), $2)",
            )
            .bind(role)
            .bind(privilege)
            .fetch_one(&mut *connection)
            .await
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
            if granted != expected {
                return Err(PostgresEvmWalletError::InvalidAuthority);
            }
        }
    }

    for role in [
        ACTIVATION_ADMIN_ROLE,
        ACTIVATION_PUBLIC_ROLE,
        NONCE_APPLICATION_ROLE,
        STORE_APPLICATION_ROLE,
        TEST_ROLE,
    ] {
        for table in REQUIRED_TABLES.iter().copied().chain(["_sqlx_migrations"]) {
            for privilege in [
                "SELECT",
                "INSERT",
                "UPDATE",
                "DELETE",
                "TRUNCATE",
                "REFERENCES",
                "TRIGGER",
            ] {
                let granted = sqlx::query_scalar::<_, bool>(
                    "SELECT pg_catalog.has_table_privilege( \
                         $1, pg_catalog.format('%I.%I', current_schema(), $2), $3 \
                     )",
                )
                .bind(role)
                .bind(table)
                .bind(privilege)
                .fetch_one(&mut *connection)
                .await
                .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
                if granted != expected_table_privilege(role, table, privilege) {
                    return Err(PostgresEvmWalletError::InvalidAuthority);
                }
            }
        }
    }
    for role in managed_roles {
        for function in [
            "enforce_wallet_lineage_head_cas()",
            "enforce_wallet_nonce_domain_update()",
            "reject_immutable_wallet_row_change()",
        ] {
            let granted = sqlx::query_scalar::<_, bool>(
                "SELECT pg_catalog.has_function_privilege( \
                     $1, pg_catalog.format('%I.%s', current_schema(), $2), 'EXECUTE' \
                 )",
            )
            .bind(role)
            .bind(function)
            .fetch_one(&mut *connection)
            .await
            .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
            if granted != (role == OWNER_ROLE) {
                return Err(PostgresEvmWalletError::InvalidAuthority);
            }
        }
    }
    Ok(())
}

async fn validate_role_memberships(
    connection: &mut PgConnection,
    expected_role: &str,
    session: &QualifiedSession,
) -> Result<()> {
    let wallet_roles = WALLET_ROLES
        .iter()
        .map(|role| (*role).to_owned())
        .collect::<Vec<_>>();
    let memberships = sqlx::query(
        "SELECT granted.rolname AS granted_role, member.rolname AS member_role, \
                membership.admin_option, membership.inherit_option, membership.set_option, \
                member.rolsuper, member.rolinherit, member.rolcreaterole, \
                member.rolcreatedb, member.rolcanlogin, member.rolreplication, \
                member.rolbypassrls \
         FROM pg_catalog.pg_auth_members AS membership \
         JOIN pg_catalog.pg_roles AS granted ON granted.oid = membership.roleid \
         JOIN pg_catalog.pg_roles AS member ON member.oid = membership.member \
         WHERE granted.rolname = ANY($1) \
         ORDER BY granted.rolname, member.rolname",
    )
    .bind(&wallet_roles)
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;

    let expected_membership_count = WALLET_ROLES
        .len()
        .checked_add(WALLET_RUNTIME_ROLES.len())
        .ok_or(PostgresEvmWalletError::InvalidAuthority)?;
    if memberships.len() != expected_membership_count {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }

    let mut runtime_principals = Vec::with_capacity(WALLET_RUNTIME_ROLES.len());
    for role in WALLET_ROLES {
        let role_memberships = memberships
            .iter()
            .filter(|membership| {
                membership
                    .try_get::<String, _>("granted_role")
                    .ok()
                    .as_deref()
                    == Some(role)
            })
            .collect::<Vec<_>>();
        let expected_count = if WALLET_RUNTIME_ROLES.contains(&role) {
            2
        } else {
            1
        };
        if role_memberships.len() != expected_count {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }

        let mut runtime_principal = None;
        for membership in role_memberships {
            let member_role = text(membership, "member_role")?;
            if member_role == session.schema_owner {
                if membership
                    .try_get::<bool, _>("admin_option")
                    .unwrap_or(true)
                    || !membership
                        .try_get::<bool, _>("inherit_option")
                        .unwrap_or(false)
                    || !membership.try_get::<bool, _>("set_option").unwrap_or(false)
                {
                    return Err(PostgresEvmWalletError::InvalidAuthority);
                }
                continue;
            }
            if !WALLET_RUNTIME_ROLES.contains(&role)
                || member_role == STORE_APPLICATION_ROLE
                || WALLET_ROLES
                    .iter()
                    .any(|candidate| *candidate == member_role)
                || membership.try_get::<bool, _>("rolsuper").unwrap_or(true)
                || membership.try_get::<bool, _>("rolinherit").unwrap_or(true)
                || membership
                    .try_get::<bool, _>("rolcreaterole")
                    .unwrap_or(true)
                || membership.try_get::<bool, _>("rolcreatedb").unwrap_or(true)
                || !membership
                    .try_get::<bool, _>("rolcanlogin")
                    .unwrap_or(false)
                || membership
                    .try_get::<bool, _>("rolreplication")
                    .unwrap_or(true)
                || membership
                    .try_get::<bool, _>("rolbypassrls")
                    .unwrap_or(true)
                || membership
                    .try_get::<bool, _>("admin_option")
                    .unwrap_or(true)
                || membership
                    .try_get::<bool, _>("inherit_option")
                    .unwrap_or(true)
                || !membership.try_get::<bool, _>("set_option").unwrap_or(false)
                || runtime_principal.replace(member_role).is_some()
            {
                return Err(PostgresEvmWalletError::InvalidAuthority);
            }
        }
        if let Some(principal) = runtime_principal {
            runtime_principals.push((role, principal));
        } else if WALLET_RUNTIME_ROLES.contains(&role) {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
    }

    runtime_principals.sort_by(|left, right| left.1.cmp(&right.1));
    if runtime_principals
        .windows(2)
        .any(|pair| pair[0].1 == pair[1].1)
        || !runtime_principals
            .iter()
            .any(|(role, principal)| *role == expected_role && principal == &session.login_role)
    {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }

    let incoming_wallet_memberships = sqlx::query_scalar::<_, i64>(
        "SELECT count(*)::bigint FROM pg_catalog.pg_auth_members AS membership \
         JOIN pg_catalog.pg_roles AS member ON member.oid = membership.member \
         WHERE member.rolname = ANY($1)",
    )
    .bind(&wallet_roles)
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    if incoming_wallet_memberships != 0 {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }

    let principal_names = runtime_principals
        .iter()
        .map(|(_, principal)| principal.clone())
        .collect::<Vec<_>>();
    let principal_memberships = sqlx::query(
        "SELECT granted.rolname AS granted_role, member.rolname AS member_role, \
                membership.admin_option, membership.inherit_option, membership.set_option \
         FROM pg_catalog.pg_auth_members AS membership \
         JOIN pg_catalog.pg_roles AS granted ON granted.oid = membership.roleid \
         JOIN pg_catalog.pg_roles AS member ON member.oid = membership.member \
         WHERE member.rolname = ANY($1) \
         ORDER BY member.rolname, granted.rolname",
    )
    .bind(&principal_names)
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    if principal_memberships.len() != runtime_principals.len() {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    for membership in principal_memberships {
        let granted_role = text(&membership, "granted_role")?;
        let member_role = text(&membership, "member_role")?;
        if !runtime_principals
            .iter()
            .any(|(role, principal)| *role == granted_role && principal == &member_role)
            || membership
                .try_get::<bool, _>("admin_option")
                .unwrap_or(true)
            || membership
                .try_get::<bool, _>("inherit_option")
                .unwrap_or(true)
            || !membership.try_get::<bool, _>("set_option").unwrap_or(false)
        {
            return Err(PostgresEvmWalletError::InvalidAuthority);
        }
    }
    Ok(())
}

fn expected_table_privilege(role: &str, table: &str, privilege: &str) -> bool {
    if table == "_sqlx_migrations" {
        return privilege == "SELECT"
            && matches!(
                role,
                ACTIVATION_ADMIN_ROLE | ACTIVATION_PUBLIC_ROLE | NONCE_APPLICATION_ROLE | TEST_ROLE
            );
    }
    match role {
        ACTIVATION_ADMIN_ROLE => match table {
            "wallet_store_schema_metadata" => privilege == "SELECT",
            "wallet_store_incarnations" | "wallet_domain_activations" => {
                matches!(privilege, "SELECT" | "INSERT")
            }
            "wallet_store_lineage_heads" => {
                matches!(privilege, "SELECT" | "INSERT" | "UPDATE")
            }
            _ => false,
        },
        ACTIVATION_PUBLIC_ROLE => {
            matches!(
                table,
                "wallet_store_schema_metadata"
                    | "wallet_store_incarnations"
                    | "wallet_store_lineage_heads"
                    | "wallet_domain_activations"
            ) && privilege == "SELECT"
        }
        NONCE_APPLICATION_ROLE => match table {
            "wallet_store_schema_metadata" => privilege == "SELECT",
            "wallet_nonce_domains" => matches!(privilege, "SELECT" | "INSERT" | "UPDATE"),
            "wallet_nonce_reservations"
            | "wallet_nonce_candidates"
            | "wallet_nonce_completions" => matches!(privilege, "SELECT" | "INSERT"),
            _ => false,
        },
        TEST_ROLE => privilege == "SELECT",
        STORE_APPLICATION_ROLE | "PUBLIC" => false,
        _ => false,
    }
}

async fn validate_closed_acls(connection: &mut PgConnection) -> Result<()> {
    let schema_acl = sqlx::query(
        "SELECT 'schema'::text AS object_kind, '__schema__'::text AS object_name, \
                CASE WHEN acl.grantor = namespace.nspowner THEN '__owner__' \
                     ELSE grantor.rolname END AS grantor_name, \
                CASE WHEN acl.grantee = 0 THEN 'PUBLIC' \
                     WHEN acl.grantee = namespace.nspowner THEN '__owner__' \
                     ELSE grantee.rolname END AS grantee_name, \
                acl.privilege_type, acl.is_grantable::text AS is_grantable \
         FROM pg_catalog.pg_namespace AS namespace \
         CROSS JOIN LATERAL pg_catalog.aclexplode( \
             COALESCE(namespace.nspacl, pg_catalog.acldefault('n', namespace.nspowner)) \
         ) AS acl \
         LEFT JOIN pg_catalog.pg_roles AS grantor ON grantor.oid = acl.grantor \
         LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid = acl.grantee \
         WHERE namespace.nspname = current_schema() \
         ORDER BY object_kind, object_name, grantor_name, grantee_name, \
                  acl.privilege_type, acl.is_grantable",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    let table_acl = sqlx::query(
        "SELECT 'table'::text AS object_kind, relation.relname AS object_name, \
                CASE WHEN acl.grantor = relation.relowner THEN '__owner__' \
                     ELSE grantor.rolname END AS grantor_name, \
                CASE WHEN acl.grantee = 0 THEN 'PUBLIC' \
                     WHEN acl.grantee = relation.relowner THEN '__owner__' \
                     ELSE grantee.rolname END AS grantee_name, \
                acl.privilege_type, acl.is_grantable::text AS is_grantable \
         FROM pg_catalog.pg_class AS relation \
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
         CROSS JOIN LATERAL pg_catalog.aclexplode( \
             COALESCE(relation.relacl, pg_catalog.acldefault('r', relation.relowner)) \
         ) AS acl \
         LEFT JOIN pg_catalog.pg_roles AS grantor ON grantor.oid = acl.grantor \
         LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid = acl.grantee \
         WHERE namespace.nspname = current_schema() AND relation.relkind = 'r' \
           AND (relation.relname = ANY($1) OR relation.relname = '_sqlx_migrations') \
         ORDER BY object_kind, object_name, grantor_name, grantee_name, \
                  acl.privilege_type, acl.is_grantable",
    )
    .bind(
        REQUIRED_TABLES
            .iter()
            .map(|table| (*table).to_owned())
            .collect::<Vec<_>>(),
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    let function_acl = sqlx::query(
        "SELECT 'function'::text AS object_kind, procedure.proname AS object_name, \
                CASE WHEN acl.grantor = procedure.proowner THEN '__owner__' \
                     ELSE grantor.rolname END AS grantor_name, \
                CASE WHEN acl.grantee = 0 THEN 'PUBLIC' \
                     WHEN acl.grantee = procedure.proowner THEN '__owner__' \
                     ELSE grantee.rolname END AS grantee_name, \
                acl.privilege_type, acl.is_grantable::text AS is_grantable \
         FROM pg_catalog.pg_proc AS procedure \
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = procedure.pronamespace \
         CROSS JOIN LATERAL pg_catalog.aclexplode( \
             COALESCE(procedure.proacl, pg_catalog.acldefault('f', procedure.proowner)) \
         ) AS acl \
         LEFT JOIN pg_catalog.pg_roles AS grantor ON grantor.oid = acl.grantor \
         LEFT JOIN pg_catalog.pg_roles AS grantee ON grantee.oid = acl.grantee \
         WHERE namespace.nspname = current_schema() \
         ORDER BY object_kind, object_name, grantor_name, grantee_name, \
                  acl.privilege_type, acl.is_grantable",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    let acl_rows = schema_acl
        .iter()
        .chain(table_acl.iter())
        .chain(function_acl.iter());
    require_manifest(
        acl_rows.map(|row| {
            vec![
                text(row, "object_kind"),
                text(row, "object_name"),
                text(row, "grantor_name"),
                text(row, "grantee_name"),
                text(row, "privilege_type"),
                text(row, "is_grantable"),
            ]
        }),
        ACL_MANIFEST_SHA256,
    )?;

    let ledger_owner_matches_schema = sqlx::query_scalar::<_, bool>(
        "SELECT relation.relowner = namespace.nspowner \
         FROM pg_catalog.pg_class AS relation \
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
         WHERE namespace.nspname = current_schema() \
           AND relation.relname = '_sqlx_migrations' AND relation.relkind = 'r'",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    if !ledger_owner_matches_schema {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    let ledger_shape = sqlx::query(
        "SELECT 'column'::text AS shape_kind, attribute.attnum::text AS ordinal, \
                attribute.attname AS name, column_type.typname AS detail, \
                attribute.attnotnull::text AS required, \
                COALESCE(pg_catalog.pg_get_expr(default_value.adbin, default_value.adrelid), '') \
                    AS expression \
         FROM pg_catalog.pg_attribute AS attribute \
         JOIN pg_catalog.pg_class AS relation ON relation.oid = attribute.attrelid \
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
         JOIN pg_catalog.pg_type AS column_type ON column_type.oid = attribute.atttypid \
         LEFT JOIN pg_catalog.pg_attrdef AS default_value \
           ON default_value.adrelid = attribute.attrelid \
          AND default_value.adnum = attribute.attnum \
         WHERE namespace.nspname = current_schema() \
           AND relation.relname = '_sqlx_migrations' \
           AND attribute.attnum > 0 AND NOT attribute.attisdropped \
         UNION ALL \
         SELECT 'constraint', '0', constraint_row.conname, \
                constraint_row.contype::text, constraint_row.convalidated::text, \
                pg_catalog.pg_get_constraintdef(constraint_row.oid, TRUE) \
         FROM pg_catalog.pg_constraint AS constraint_row \
         JOIN pg_catalog.pg_class AS relation ON relation.oid = constraint_row.conrelid \
         JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace \
         WHERE namespace.nspname = current_schema() \
           AND relation.relname = '_sqlx_migrations' \
         ORDER BY shape_kind, ordinal, name",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    require_manifest(
        ledger_shape.iter().map(|row| {
            vec![
                text(row, "shape_kind"),
                text(row, "ordinal"),
                text(row, "name"),
                text(row, "detail"),
                text(row, "required"),
                text(row, "expression"),
            ]
        }),
        MIGRATION_LEDGER_SHAPE_SHA256,
    )
}

async fn validate_metadata(connection: &mut PgConnection) -> Result<()> {
    let row = sqlx::query(
        "SELECT schema_contract_version, \
                (SELECT count(*)::bigint FROM wallet_store_schema_metadata) AS metadata_count \
         FROM wallet_store_schema_metadata WHERE singleton = TRUE",
    )
    .fetch_optional(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?
    .ok_or(PostgresEvmWalletError::InvalidAuthority)?;
    if row.try_get::<i64, _>("metadata_count").ok() != Some(1)
        || text(&row, "schema_contract_version")? != SCHEMA_CONTRACT_VERSION
    {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    Ok(())
}

async fn validate_prefix_integrity(
    connection: &mut PgConnection,
    expected_role: &str,
) -> Result<()> {
    match expected_role {
        ACTIVATION_ADMIN_ROLE | ACTIVATION_PUBLIC_ROLE => {
            validate_registry_prefix_integrity(connection).await
        }
        NONCE_APPLICATION_ROLE => validate_nonce_prefix_integrity(connection).await,
        _ => Err(PostgresEvmWalletError::InvalidAuthority),
    }
}

async fn validate_registry_prefix_integrity(connection: &mut PgConnection) -> Result<()> {
    let invalid = sqlx::query_scalar::<_, i64>(
        "WITH ordered_incarnations AS ( \
             SELECT incarnation.*, \
                    row_number() OVER ( \
                        PARTITION BY wallet_nonce_store_lineage_id ORDER BY writer_epoch \
                    )::numeric AS dense_epoch, \
                    lag(incarnation_ref) OVER ( \
                        PARTITION BY wallet_nonce_store_lineage_id ORDER BY writer_epoch \
                    ) AS prior_ref \
             FROM wallet_store_incarnations AS incarnation \
         ), invalid_incarnations AS ( \
             SELECT 1 FROM ordered_incarnations \
             WHERE writer_epoch <> dense_epoch \
                OR predecessor_incarnation_ref IS DISTINCT FROM prior_ref \
                OR (writer_epoch = 1 AND promotion_successor_ref IS NOT NULL) \
                OR (writer_epoch > 1 AND promotion_successor_ref IS NULL) \
                OR (writer_epoch > 1 AND ( \
                    promotion_attestation_json::jsonb -> 'previous_incarnation_ref' \
                        <> predecessor_incarnation_ref::jsonb \
                    OR promotion_attestation_json::jsonb -> 'current_incarnation_ref' \
                        <> incarnation_ref::jsonb \
                    OR promotion_attestation_json::jsonb -> 'successor_ref' \
                        <> promotion_successor_ref::jsonb \
                    OR promotion_attestation_json::jsonb ->> 'writer_epoch' \
                        <> writer_epoch::text \
                )) \
         ), invalid_heads AS ( \
             SELECT 1 FROM wallet_store_lineage_heads AS head \
             LEFT JOIN LATERAL ( \
                 SELECT wallet_nonce_store_lineage_id, writer_epoch, incarnation_ref \
                 FROM wallet_store_incarnations \
                 WHERE wallet_nonce_store_lineage_id = head.wallet_nonce_store_lineage_id \
                 ORDER BY writer_epoch DESC LIMIT 1 \
             ) AS last_incarnation ON TRUE \
             WHERE last_incarnation.wallet_nonce_store_lineage_id IS NULL \
                OR head.current_writer_epoch <> last_incarnation.writer_epoch \
                OR head.current_incarnation_ref <> last_incarnation.incarnation_ref \
         ), orphan_lineages AS ( \
             SELECT 1 FROM wallet_store_incarnations AS incarnation \
             LEFT JOIN wallet_store_lineage_heads AS head USING (wallet_nonce_store_lineage_id) \
             WHERE head.wallet_nonce_store_lineage_id IS NULL \
         ), invalid_activations AS ( \
             SELECT 1 FROM wallet_domain_activations AS activation \
             LEFT JOIN wallet_store_lineage_heads AS head USING (wallet_nonce_store_lineage_id) \
             WHERE head.wallet_nonce_store_lineage_id IS NULL \
                OR activation.activation_record_json::jsonb \
                       #>> '{wallet_nonce_domain,digest}' <> activation.wallet_nonce_domain_id \
                OR activation.activation_record_json::jsonb \
                       ->> 'wallet_nonce_store_lineage_id' <> activation.wallet_nonce_store_lineage_id \
                OR activation.activation_attestation_json::jsonb \
                       -> 'current_schema_record' <> activation.activation_record_json::jsonb \
                OR activation.activation_attestation_json::jsonb \
                       -> 'activation_record_ref' <> activation.activation_record_ref::jsonb \
                OR activation.activation_attestation_json::jsonb \
                       -> 'observed_store_incarnation_ref' \
                       <> activation.observed_store_incarnation_ref::jsonb \
                OR activation.activation_attestation_json::jsonb \
                       -> 'registry_issuance_ref' <> activation.registry_issuance_ref::jsonb \
         ) \
         SELECT (SELECT count(*) FROM invalid_incarnations) \
              + (SELECT count(*) FROM invalid_heads) \
              + (SELECT count(*) FROM orphan_lineages) \
              + (SELECT count(*) FROM invalid_activations)",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    if invalid != 0 {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    Ok(())
}

async fn validate_nonce_prefix_integrity(connection: &mut PgConnection) -> Result<()> {
    let invalid = sqlx::query_scalar::<_, i64>(
        "WITH ordered_reservations AS ( \
             SELECT reservation.*, \
                    row_number() OVER ( \
                        PARTITION BY wallet_nonce_domain_id ORDER BY nonce \
                    )::numeric AS dense_offset, \
                    count(*) OVER (PARTITION BY wallet_nonce_domain_id) AS reservation_count \
             FROM wallet_nonce_reservations AS reservation \
         ), invalid_domains AS ( \
             SELECT 1 FROM wallet_nonce_domains AS domain \
             LEFT JOIN LATERAL ( \
                 SELECT count(*)::numeric AS retained_count, max(nonce) AS maximum_nonce \
                 FROM wallet_nonce_reservations \
                 WHERE wallet_nonce_domain_id = domain.wallet_nonce_domain_id \
             ) AS reservations ON TRUE \
             WHERE domain.activation_record_json::jsonb \
                       #>> '{wallet_nonce_domain,digest}' <> domain.wallet_nonce_domain_id \
                OR domain.activation_record_json::jsonb \
                       ->> 'wallet_nonce_store_lineage_id' <> domain.wallet_nonce_store_lineage_id \
                OR domain.activation_attestation_json::jsonb \
                       -> 'current_schema_record' <> domain.activation_record_json::jsonb \
                OR domain.activation_attestation_json::jsonb \
                       -> 'activation_record_ref' <> domain.activation_record_ref::jsonb \
                OR (reservations.retained_count = 0 AND domain.local_high_water_nonce IS NOT NULL) \
                OR (reservations.retained_count > 0 \
                    AND domain.local_high_water_nonce IS DISTINCT FROM reservations.maximum_nonce) \
         ), invalid_reservations AS ( \
             SELECT 1 FROM ordered_reservations AS reservation \
             JOIN wallet_nonce_domains AS domain USING (wallet_nonce_domain_id) \
             WHERE reservation.nonce <> \
                   (domain.activation_record_json::jsonb \
                        ->> 'finalized_sender_nonce_floor')::numeric \
                   + reservation.dense_offset - 1 \
                OR reservation.request_json::jsonb \
                       #>> '{nonce_domain,digest}' <> reservation.wallet_nonce_domain_id \
                OR reservation.request_json::jsonb \
                       #> '{domain_activation_attestation,activation_record_ref}' \
                       <> domain.activation_record_ref::jsonb \
                OR reservation.request_json::jsonb \
                       #>> '{submission_intent_id,digest}' \
                       <> reservation.submission_intent_id \
                OR reservation.request_json::jsonb \
                       #>> '{reservation_key,digest}' \
                       <> reservation.semantic_reservation_key \
                OR reservation.transaction_intent_json::jsonb \
                       ->> 'digest' <> reservation.transaction_intent_digest \
                OR reservation.candidate_family_json::jsonb \
                       ->> 'digest' <> reservation.candidate_family_ref \
                OR reservation.reservation_json::jsonb \
                       #>> '{nonce_domain,digest}' <> reservation.wallet_nonce_domain_id \
                OR reservation.reservation_json::jsonb \
                       #>> '{semantic_reservation_key,digest}' \
                       <> reservation.semantic_reservation_key \
                OR reservation.reservation_json::jsonb \
                       #>> '{submission_intent_id,digest}' \
                       <> reservation.submission_intent_id \
                OR reservation.reservation_json::jsonb \
                       ->> 'transaction_intent_digest' <> reservation.transaction_intent_digest \
                OR reservation.reservation_json::jsonb \
                       ->> 'candidate_family_ref' <> reservation.candidate_family_ref \
                OR reservation.reservation_json::jsonb \
                       ->> 'nonce' <> reservation.nonce::text \
         ), invalid_candidates AS ( \
             SELECT 1 FROM ( \
                 SELECT candidate.*, row_number() OVER ( \
                     PARTITION BY semantic_reservation_key ORDER BY candidate_ordinal \
                 ) - 1 AS dense_ordinal \
                 FROM wallet_nonce_candidates AS candidate \
             ) AS candidate \
             WHERE candidate.candidate_ordinal <> candidate.dense_ordinal \
                OR candidate.request_json::jsonb \
                       #>> '{candidate_operation_key,digest}' \
                       <> candidate.semantic_candidate_operation_key \
                OR candidate.request_json::jsonb \
                       #>> '{next_candidate,semantic_reservation_key,digest}' \
                       <> candidate.semantic_reservation_key \
                OR candidate.active_candidate_json::jsonb \
                       #>> '{attested_candidate,semantic_reservation_key,digest}' \
                       <> candidate.semantic_reservation_key \
                OR (candidate.active_candidate_json::jsonb \
                       #>> '{attested_candidate,candidate_ordinal}')::integer \
                       <> candidate.candidate_ordinal \
         ), invalid_completions AS ( \
            SELECT 1 FROM wallet_nonce_completions AS completion \
            WHERE completion.completion_json::jsonb \
                       #>> '{semantic_completion_key,digest}' \
                       <> completion.semantic_completion_key \
                OR completion.completion_json::jsonb \
                       #>> '{semantic_reservation_key,digest}' \
                       <> completion.semantic_reservation_key \
                OR completion.request_json::jsonb \
                       #>> '{completion_key,digest}' \
                       <> completion.semantic_completion_key \
                OR completion.request_json::jsonb \
                       #>> '{current_reservation,semantic_reservation_key,digest}' \
                       <> completion.semantic_reservation_key \
                OR completion.canonical_terminal_outcome_json::jsonb \
                       <> completion.request_json::jsonb -> 'canonical_terminal_outcome' \
         ), invalid_incomplete AS ( \
             SELECT 1 FROM ordered_reservations AS reservation \
             LEFT JOIN wallet_nonce_completions AS completion \
               USING (semantic_reservation_key) \
             WHERE completion.semantic_reservation_key IS NULL \
               AND reservation.dense_offset <> reservation.reservation_count \
         ) \
         SELECT (SELECT count(*) FROM invalid_domains) \
              + (SELECT count(*) FROM invalid_reservations) \
              + (SELECT count(*) FROM invalid_candidates) \
              + (SELECT count(*) FROM invalid_completions) \
              + (SELECT count(*) FROM invalid_incomplete)",
    )
    .fetch_one(&mut *connection)
    .await
    .map_err(|_| PostgresEvmWalletError::InvalidAuthority)?;
    if invalid != 0 {
        return Err(PostgresEvmWalletError::InvalidAuthority);
    }
    Ok(())
}

fn text(row: &sqlx::postgres::PgRow, column: &str) -> Result<String> {
    row.try_get(column)
        .map_err(|_| PostgresEvmWalletError::InvalidAuthority)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use sqlx::postgres::PgConnectOptions;
    use sqlx::{AssertSqlSafe, Connection};

    use super::{ACTIVATION_ADMIN_ROLE, ACTIVATION_PUBLIC_ROLE, NONCE_APPLICATION_ROLE};
    use crate::support::open_role_pool;

    const ROLE_DATABASE_URLS: [(&str, &str); 3] = [
        (
            "MFM_EVM_WALLET_ACTIVATION_ADMIN_DATABASE_URL",
            ACTIVATION_ADMIN_ROLE,
        ),
        (
            "MFM_EVM_WALLET_ACTIVATION_PUBLIC_DATABASE_URL",
            ACTIVATION_PUBLIC_ROLE,
        ),
        (
            "MFM_EVM_WALLET_NONCE_APPLICATION_DATABASE_URL",
            NONCE_APPLICATION_ROLE,
        ),
    ];

    #[tokio::test]
    #[ignore = "requires the managed PostgreSQL authoritative-schema task"]
    async fn verification_probe_accepts_the_current_authoritative_schema() {
        let admin_database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL is required");
        let admin_options =
            PgConnectOptions::from_str(&admin_database_url).expect("parse administrator URL");
        let mut admin = sqlx::PgConnection::connect_with(&admin_options)
            .await
            .expect("connect wallet schema administrator");
        let schema = sqlx::query_scalar::<_, String>("SELECT current_schema()::text")
            .fetch_one(&mut admin)
            .await
            .expect("resolve wallet probe schema");
        configure_login_principals(&mut admin).await;

        for (environment, expected_role) in ROLE_DATABASE_URLS {
            let role_database_url = scoped_database_url(
                &std::env::var(environment).unwrap_or_else(|_| panic!("{environment} is required")),
                &schema,
            );
            let pool = open_role_pool(&role_database_url, &schema, expected_role)
                .await
                .unwrap_or_else(|error| panic!("{expected_role} schema probe failed: {error}"));
            pool.close().await;

            let provider_database_url =
                database_url_with_active_role(&role_database_url, expected_role);
            let provider_pool = sqlx::PgPool::connect(&provider_database_url)
                .await
                .expect("connect provider through an explicitly activated role");
            let (session_role, active_role) = sqlx::query_as::<_, (String, String)>(
                "SELECT session_user::text, current_user::text",
            )
            .fetch_one(&provider_pool)
            .await
            .expect("inspect provider session identities");
            assert_ne!(session_role, active_role);
            assert_eq!(active_role, expected_role);
            sqlx::query(AssertSqlSafe(format!(
                "SELECT 1 FROM {}.wallet_store_schema_metadata",
                identifier(&schema)
            )))
            .fetch_optional(&provider_pool)
            .await
            .expect("activated provider role can read its schema metadata");
            provider_pool.close().await;
        }
    }

    async fn configure_login_principals(admin: &mut sqlx::PgConnection) {
        let mut principals = Vec::with_capacity(ROLE_DATABASE_URLS.len());
        for (environment, granted_role) in ROLE_DATABASE_URLS {
            let database_url =
                std::env::var(environment).unwrap_or_else(|_| panic!("{environment} is required"));
            let principal = PgConnectOptions::from_str(&database_url)
                .expect("parse wallet role URL")
                .get_username()
                .to_owned();
            principals.push((principal, granted_role));
        }
        assert!(principals[0].0 != principals[1].0);
        assert!(principals[0].0 != principals[2].0);
        assert!(principals[1].0 != principals[2].0);

        for (principal_name, granted_role) in principals {
            let principal = identifier(&principal_name);
            let exists = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = $1)",
            )
            .bind(&principal_name)
            .fetch_one(&mut *admin)
            .await
            .expect("inspect wallet login principal");
            if !exists {
                sqlx::query(AssertSqlSafe(format!(
                    "CREATE ROLE {principal} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE \
                     NOINHERIT NOREPLICATION NOBYPASSRLS"
                )))
                .execute(&mut *admin)
                .await
                .expect("create wallet login principal");
            }
            sqlx::query(AssertSqlSafe(format!(
                "ALTER ROLE {principal} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE \
                 NOINHERIT NOREPLICATION NOBYPASSRLS"
            )))
            .execute(&mut *admin)
            .await
            .expect("qualify wallet login principal");
            let inherited_roles = sqlx::query_scalar::<_, String>(
                "SELECT granted.rolname FROM pg_catalog.pg_auth_members AS membership \
                 JOIN pg_catalog.pg_roles AS granted ON granted.oid = membership.roleid \
                 JOIN pg_catalog.pg_roles AS member ON member.oid = membership.member \
                 WHERE member.rolname = $1 ORDER BY granted.rolname",
            )
            .bind(&principal_name)
            .fetch_all(&mut *admin)
            .await
            .expect("inventory wallet login memberships");
            for inherited_role in inherited_roles {
                sqlx::query(AssertSqlSafe(format!(
                    "REVOKE {} FROM {principal}",
                    identifier(&inherited_role)
                )))
                .execute(&mut *admin)
                .await
                .expect("remove unrelated wallet login membership");
            }
            sqlx::query(AssertSqlSafe(format!(
                "GRANT {} TO {principal} WITH ADMIN FALSE, INHERIT FALSE, SET TRUE",
                identifier(granted_role)
            )))
            .execute(&mut *admin)
            .await
            .expect("grant exact wallet runtime role");
        }
    }

    fn scoped_database_url(base_url: &str, schema: &str) -> String {
        let separator = if base_url.contains('?') { '&' } else { '?' };
        format!("{base_url}{separator}options=-csearch_path%3D{schema}")
    }

    fn database_url_with_active_role(database_url: &str, role: &str) -> String {
        let separator = if database_url.contains('?') { '&' } else { '?' };
        format!("{database_url}{separator}options%5Brole%5D={role}")
    }

    fn identifier(value: &str) -> String {
        assert!(!value.is_empty());
        assert!(value.chars().enumerate().all(|(index, character)| {
            character == '_'
                || (character.is_ascii_alphanumeric() && (index > 0 || !character.is_ascii_digit()))
        }));
        format!("\"{value}\"")
    }
}
