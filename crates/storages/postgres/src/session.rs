//! Opaque deployment-issued exact-target session bundles.
//!
//! Production openers consume only these bundles. Ordinary application code never
//! receives a `PgPool`, URL, connection option, or raw writer fence.

use std::collections::BTreeSet;

use mfm_ids::{StoreEpoch, StoreScopeId};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool, Row};

use crate::error::{PostgresStoreError, Result};
use crate::roles::{TargetKey, TargetRoleKind, TargetRoleNames};
use crate::schema::{
    validate_authoritative_schema_at, ValidatedStoreIdentity, SCHEMA_CONTRACT_VERSION,
};

/// Capability kind bound to one physical login/session pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionKind {
    RunReader,
    RunWriter,
    ConfigurationReader,
    ConfigurationWriter,
}

impl SessionKind {
    const fn role_kind(self) -> TargetRoleKind {
        match self {
            Self::RunReader => TargetRoleKind::RunReader,
            Self::RunWriter => TargetRoleKind::RunWriter,
            Self::ConfigurationReader => TargetRoleKind::ConfigurationReader,
            Self::ConfigurationWriter => TargetRoleKind::ConfigurationWriter,
        }
    }

    const fn expects_write(self) -> bool {
        matches!(self, Self::RunWriter | Self::ConfigurationWriter)
    }
}

/// Non-secret target binding retained by every opaque session bundle.
#[derive(Clone, PartialEq, Eq)]
pub struct TargetBinding {
    database_name: String,
    schema_name: String,
    database_oid: u32,
    store_scope_id: StoreScopeId,
    store_epoch: StoreEpoch,
    target_key: TargetKey,
    fence_generation: u64,
    release_epoch: u64,
    roles: TargetRoleNames,
}

impl TargetBinding {
    /// Connected PostgreSQL database name.
    pub fn database_name(&self) -> &str {
        &self.database_name
    }

    /// Schema containing the closed MFM authority catalog.
    pub fn schema_name(&self) -> &str {
        &self.schema_name
    }

    /// PostgreSQL database OID observed during issuance.
    pub const fn database_oid(&self) -> u32 {
        self.database_oid
    }

    /// Immutable MFM store scope.
    pub const fn store_scope_id(&self) -> &StoreScopeId {
        &self.store_scope_id
    }

    /// Immutable MFM store epoch.
    pub const fn store_epoch(&self) -> StoreEpoch {
        self.store_epoch
    }

    pub(crate) const fn fence_generation(&self) -> u64 {
        self.fence_generation
    }

    pub(crate) const fn release_epoch(&self) -> u64 {
        self.release_epoch
    }

    pub(crate) fn roles(&self) -> &TargetRoleNames {
        &self.roles
    }
}

impl std::fmt::Debug for TargetBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TargetBinding")
            .field("database_name", &self.database_name)
            .field("schema_name", &self.schema_name)
            .field("store_scope_id", &self.store_scope_id)
            .field("store_epoch", &self.store_epoch)
            .field("fence_generation", &self.fence_generation)
            .finish_non_exhaustive()
    }
}

/// One physical role-bound session pool. Not publicly constructible.
pub(crate) struct RoleSession {
    pool: PgPool,
    kind: SessionKind,
    login_role: String,
    managed_role: String,
    managed_role_oid: u32,
}

impl RoleSession {
    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub(crate) const fn kind(&self) -> SessionKind {
        self.kind
    }

    pub(crate) fn managed_role(&self) -> &str {
        &self.managed_role
    }

    pub(crate) const fn managed_role_oid(&self) -> u32 {
        self.managed_role_oid
    }

    pub(crate) fn login_role(&self) -> &str {
        &self.login_role
    }
}

/// Opaque application-facing exact-target sessions.
///
/// Holds physically distinct run-read, run-write, and configuration-read capabilities.
/// No writer-capable pool backs a reader.
pub struct ApplicationTargetSessions {
    target: TargetBinding,
    run_reader: RoleSession,
    run_writer: RoleSession,
    configuration_reader: RoleSession,
}

impl ApplicationTargetSessions {
    pub(crate) fn target(&self) -> &TargetBinding {
        &self.target
    }

    pub(crate) fn run_reader(&self) -> &RoleSession {
        &self.run_reader
    }

    pub(crate) fn run_writer(&self) -> &RoleSession {
        &self.run_writer
    }

    pub(crate) fn configuration_reader(&self) -> &RoleSession {
        &self.configuration_reader
    }

    pub(crate) fn into_run_parts(self) -> (RoleSession, RoleSession, TargetBinding) {
        (self.run_reader, self.run_writer, self.target)
    }

    pub(crate) fn into_configuration_reader(self) -> (RoleSession, TargetBinding) {
        (self.configuration_reader, self.target)
    }

    pub(crate) fn into_application_parts(
        self,
    ) -> (RoleSession, RoleSession, RoleSession, TargetBinding) {
        (
            self.run_reader,
            self.run_writer,
            self.configuration_reader,
            self.target,
        )
    }
}

impl std::fmt::Debug for ApplicationTargetSessions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApplicationTargetSessions")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

/// Opaque deployment-maintenance sessions for append-only configuration.
pub struct ConfigurationMaintenanceSessions {
    target: TargetBinding,
    configuration_reader: RoleSession,
    configuration_writer: RoleSession,
}

impl ConfigurationMaintenanceSessions {
    pub(crate) fn target(&self) -> &TargetBinding {
        &self.target
    }

    pub(crate) fn configuration_reader(&self) -> &RoleSession {
        &self.configuration_reader
    }

    pub(crate) fn configuration_writer(&self) -> &RoleSession {
        &self.configuration_writer
    }

    pub(crate) fn into_parts(self) -> (RoleSession, RoleSession, TargetBinding) {
        (
            self.configuration_reader,
            self.configuration_writer,
            self.target,
        )
    }
}

impl std::fmt::Debug for ConfigurationMaintenanceSessions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConfigurationMaintenanceSessions")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

/// Combined run and configuration sessions used by joint assembly.
pub struct CombinedTargetSessions {
    target: TargetBinding,
    run_reader: RoleSession,
    run_writer: RoleSession,
    configuration_reader: RoleSession,
    configuration_writer: RoleSession,
}

impl CombinedTargetSessions {
    pub(crate) fn target(&self) -> &TargetBinding {
        &self.target
    }

    pub(crate) fn run_reader(&self) -> &RoleSession {
        &self.run_reader
    }

    pub(crate) fn run_writer(&self) -> &RoleSession {
        &self.run_writer
    }

    pub(crate) fn configuration_reader(&self) -> &RoleSession {
        &self.configuration_reader
    }

    pub(crate) fn configuration_writer(&self) -> &RoleSession {
        &self.configuration_writer
    }

    /// Drops the configuration writer, leaving application sessions only.
    pub fn into_application(self) -> ApplicationTargetSessions {
        ApplicationTargetSessions {
            target: self.target,
            run_reader: self.run_reader,
            run_writer: self.run_writer,
            configuration_reader: self.configuration_reader,
        }
    }

    pub(crate) fn into_run_parts(self) -> (RoleSession, RoleSession, TargetBinding) {
        (self.run_reader, self.run_writer, self.target)
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        RoleSession,
        RoleSession,
        RoleSession,
        RoleSession,
        TargetBinding,
    ) {
        (
            self.run_reader,
            self.run_writer,
            self.configuration_reader,
            self.configuration_writer,
            self.target,
        )
    }
}

impl std::fmt::Debug for CombinedTargetSessions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CombinedTargetSessions")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

/// Deployment-private login material for one physical session.
///
/// Holders of these credentials are inside the deployment TCB. Ordinary MFM
/// assembly never receives this type.
#[derive(Clone)]
pub struct SessionLoginMaterial {
    /// PostgreSQL connection URL authenticating the exact restricted login.
    pub database_url: String,
}

impl std::fmt::Debug for SessionLoginMaterial {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionLoginMaterial")
            .finish_non_exhaustive()
    }
}

/// Deployment-private materials used only while issuing an opaque session bundle.
pub struct TargetSessionMaterials {
    /// Target schema name that owns the closed authority catalog.
    pub schema_name: String,
    /// Run-reader login.
    pub run_reader: SessionLoginMaterial,
    /// Run-writer login.
    pub run_writer: SessionLoginMaterial,
    /// Configuration-reader login.
    pub configuration_reader: SessionLoginMaterial,
    /// Configuration-writer login when combined issuance is required.
    pub configuration_writer: Option<SessionLoginMaterial>,
}

impl std::fmt::Debug for TargetSessionMaterials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TargetSessionMaterials")
            .field("schema_name", &self.schema_name)
            .finish_non_exhaustive()
    }
}

/// Issues an opaque application session bundle after exact-target verification.
///
/// Deployment infrastructure is the trusted credential boundary. A holder of the
/// login materials is inside the TCB.
pub async fn issue_application_sessions(
    materials: TargetSessionMaterials,
) -> Result<ApplicationTargetSessions> {
    if materials.configuration_writer.is_some() {
        return Err(PostgresStoreError::TargetSessionRejected);
    }
    let target = load_target_from_login(&materials.run_reader, &materials.schema_name).await?;
    let run_reader = open_role_session(
        &materials.run_reader,
        &materials.schema_name,
        &target,
        SessionKind::RunReader,
    )
    .await?;
    let run_writer = open_role_session(
        &materials.run_writer,
        &materials.schema_name,
        &target,
        SessionKind::RunWriter,
    )
    .await?;
    let configuration_reader = open_role_session(
        &materials.configuration_reader,
        &materials.schema_name,
        &target,
        SessionKind::ConfigurationReader,
    )
    .await?;
    let after = load_target_from_login(&materials.run_writer, &materials.schema_name).await?;
    if after != target {
        return Err(PostgresStoreError::TargetSessionRejected);
    }
    Ok(ApplicationTargetSessions {
        target,
        run_reader,
        run_writer,
        configuration_reader,
    })
}

/// Issues opaque configuration-maintenance sessions.
pub async fn issue_configuration_maintenance_sessions(
    schema_name: String,
    configuration_reader_material: SessionLoginMaterial,
    configuration_writer_material: SessionLoginMaterial,
) -> Result<ConfigurationMaintenanceSessions> {
    let target = load_target_from_login(&configuration_reader_material, &schema_name).await?;
    let configuration_reader = open_role_session(
        &configuration_reader_material,
        &schema_name,
        &target,
        SessionKind::ConfigurationReader,
    )
    .await?;
    let configuration_writer = open_role_session(
        &configuration_writer_material,
        &schema_name,
        &target,
        SessionKind::ConfigurationWriter,
    )
    .await?;
    let after = load_target_from_login(&configuration_writer_material, &schema_name).await?;
    if after != target {
        return Err(PostgresStoreError::TargetSessionRejected);
    }
    Ok(ConfigurationMaintenanceSessions {
        target,
        configuration_reader,
        configuration_writer,
    })
}

/// Issues combined run and configuration sessions for joint assembly.
pub async fn issue_combined_sessions(
    materials: TargetSessionMaterials,
) -> Result<CombinedTargetSessions> {
    let Some(configuration_writer_material) = materials.configuration_writer else {
        return Err(PostgresStoreError::TargetSessionRejected);
    };
    let target = load_target_from_login(&materials.run_reader, &materials.schema_name).await?;
    let run_reader = open_role_session(
        &materials.run_reader,
        &materials.schema_name,
        &target,
        SessionKind::RunReader,
    )
    .await?;
    let run_writer = open_role_session(
        &materials.run_writer,
        &materials.schema_name,
        &target,
        SessionKind::RunWriter,
    )
    .await?;
    let configuration_reader = open_role_session(
        &materials.configuration_reader,
        &materials.schema_name,
        &target,
        SessionKind::ConfigurationReader,
    )
    .await?;
    let configuration_writer = open_role_session(
        &configuration_writer_material,
        &materials.schema_name,
        &target,
        SessionKind::ConfigurationWriter,
    )
    .await?;
    let after = load_target_from_login(&materials.run_writer, &materials.schema_name).await?;
    if after != target {
        return Err(PostgresStoreError::TargetSessionRejected);
    }
    Ok(CombinedTargetSessions {
        target,
        run_reader,
        run_writer,
        configuration_reader,
        configuration_writer,
    })
}

async fn load_target_from_login(
    login: &SessionLoginMaterial,
    expected_schema: &str,
) -> Result<TargetBinding> {
    let pool = connect_login(login, expected_schema).await?;
    let identity = validate_authoritative_schema_at(&pool, expected_schema).await?;
    let binding = load_target_binding(&pool, expected_schema, identity).await?;
    pool.close().await;
    Ok(binding)
}

async fn open_role_session(
    login: &SessionLoginMaterial,
    expected_schema: &str,
    target: &TargetBinding,
    kind: SessionKind,
) -> Result<RoleSession> {
    let pool = connect_login(login, expected_schema).await?;
    let (login_role, managed_role, managed_role_oid) =
        probe_login_shape(&pool, target, kind).await?;
    Ok(RoleSession {
        pool,
        kind,
        login_role,
        managed_role,
        managed_role_oid,
    })
}

async fn connect_login(login: &SessionLoginMaterial, schema: &str) -> Result<PgPool> {
    let options = login
        .database_url
        .parse::<PgConnectOptions>()
        .map_err(|_| PostgresStoreError::Connection)?
        .options([("search_path", schema)]);
    PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .map_err(|_| PostgresStoreError::Connection)
}

pub(crate) async fn load_target_binding(
    pool: &PgPool,
    expected_schema: &str,
    identity: ValidatedStoreIdentity,
) -> Result<TargetBinding> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|_| PostgresStoreError::Connection)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    sqlx::query("SET TRANSACTION READ ONLY")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    sqlx::query(
        "SELECT pg_catalog.set_config( \
             'search_path', pg_catalog.format('%I, pg_catalog', $1), TRUE \
         )",
    )
    .bind(expected_schema)
    .execute(&mut *transaction)
    .await
    .map_err(|_| PostgresStoreError::WriterRequired)?;

    // NOINHERIT logins only observe authority rows after selecting qualification.
    let qualification =
        TargetKey::from_schema(expected_schema).role_name(TargetRoleKind::Qualification);
    let set_role = format!("SET LOCAL ROLE {}", quote_ident(&qualification));
    sqlx::query(AssertSqlSafe(set_role))
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresStoreError::TargetSessionRejected)?;

    let row = sqlx::query(
        "SELECT current_database()::text AS database_name, \
                current_schema()::text AS schema_name, \
                (SELECT oid::bigint FROM pg_catalog.pg_database \
                  WHERE datname = current_database()) AS database_oid, \
                authority.target_key, \
                authority.fence_generation::text AS fence_generation, \
                authority.release_epoch::text AS release_epoch, \
                authority.owner_role, authority.qualification_role, \
                authority.run_reader_role, authority.run_writer_role, \
                authority.configuration_reader_role, authority.configuration_writer_role, \
                identity.store_scope_id, identity.store_epoch::text AS store_epoch, \
                metadata.schema_contract_version \
           FROM target_authority AS authority \
           CROSS JOIN store_identity AS identity \
           CROSS JOIN store_schema_metadata AS metadata \
          WHERE authority.singleton AND identity.singleton AND metadata.singleton",
    )
    .fetch_one(&mut *transaction)
    .await
    .map_err(|_| PostgresStoreError::TargetSessionRejected)?;

    let schema_name = row
        .try_get::<String, _>("schema_name")
        .map_err(|_| PostgresStoreError::TargetSessionRejected)?;
    if schema_name != expected_schema {
        return Err(PostgresStoreError::TargetSessionRejected);
    }
    let contract = row
        .try_get::<String, _>("schema_contract_version")
        .map_err(|_| PostgresStoreError::TargetSessionRejected)?;
    if contract != SCHEMA_CONTRACT_VERSION {
        return Err(PostgresStoreError::SchemaAuthorityMismatch);
    }
    let target_key = TargetKey::parse(
        &row.try_get::<String, _>("target_key")
            .map_err(|_| PostgresStoreError::TargetSessionRejected)?,
    )
    .ok_or(PostgresStoreError::TargetSessionRejected)?;
    let roles = TargetRoleNames::from_target_key(target_key.clone());
    if row.try_get::<String, _>("owner_role").ok().as_deref() != Some(roles.owner.as_str())
        || row
            .try_get::<String, _>("qualification_role")
            .ok()
            .as_deref()
            != Some(roles.qualification.as_str())
        || row.try_get::<String, _>("run_reader_role").ok().as_deref()
            != Some(roles.run_reader.as_str())
        || row.try_get::<String, _>("run_writer_role").ok().as_deref()
            != Some(roles.run_writer.as_str())
        || row
            .try_get::<String, _>("configuration_reader_role")
            .ok()
            .as_deref()
            != Some(roles.configuration_reader.as_str())
        || row
            .try_get::<String, _>("configuration_writer_role")
            .ok()
            .as_deref()
            != Some(roles.configuration_writer.as_str())
    {
        return Err(PostgresStoreError::TargetSessionRejected);
    }
    let store_scope_id = StoreScopeId::new(
        row.try_get::<String, _>("store_scope_id")
            .map_err(|_| PostgresStoreError::TargetSessionRejected)?,
    )
    .map_err(|_| PostgresStoreError::TargetSessionRejected)?;
    let store_epoch = StoreEpoch::parse(
        row.try_get::<String, _>("store_epoch")
            .map_err(|_| PostgresStoreError::TargetSessionRejected)?,
    )
    .map_err(|_| PostgresStoreError::TargetSessionRejected)?;
    if store_scope_id != identity.store_scope_id || store_epoch != identity.store_epoch {
        return Err(PostgresStoreError::TargetSessionRejected);
    }
    let database_oid = row
        .try_get::<i64, _>("database_oid")
        .ok()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(PostgresStoreError::TargetSessionRejected)?;
    let fence_generation = parse_u64(
        &row.try_get::<String, _>("fence_generation")
            .map_err(|_| PostgresStoreError::TargetSessionRejected)?,
    )?;
    let release_epoch = parse_u64(
        &row.try_get::<String, _>("release_epoch")
            .map_err(|_| PostgresStoreError::TargetSessionRejected)?,
    )?;
    transaction
        .commit()
        .await
        .map_err(|_| PostgresStoreError::TargetSessionRejected)?;
    Ok(TargetBinding {
        database_name: row
            .try_get::<String, _>("database_name")
            .map_err(|_| PostgresStoreError::TargetSessionRejected)?,
        schema_name,
        database_oid,
        store_scope_id,
        store_epoch,
        target_key,
        fence_generation,
        release_epoch,
        roles,
    })
}

async fn probe_login_shape(
    pool: &PgPool,
    target: &TargetBinding,
    kind: SessionKind,
) -> Result<(String, String, u32)> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|_| PostgresStoreError::Connection)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    if kind.expects_write() {
        sqlx::query("SET TRANSACTION READ WRITE")
            .execute(&mut *transaction)
            .await
            .map_err(|_| PostgresStoreError::WriterRequired)?;
    } else {
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *transaction)
            .await
            .map_err(|_| PostgresStoreError::WriterRequired)?;
    }

    let row = sqlx::query(
        "SELECT current_user::text AS session_role_name, \
                session_user::text AS authenticated_role_name, \
                pg_catalog.pg_is_in_recovery() AS in_recovery, \
                current_setting('transaction_read_only') AS transaction_read_only, \
                login_role.rolsuper, login_role.rolinherit, login_role.rolcreaterole, \
                login_role.rolcreatedb, login_role.rolcanlogin, login_role.rolreplication, \
                login_role.rolbypassrls, login_role.rolconfig IS NULL AS no_role_config, \
                database_row.datdba = login_role.oid \
                    OR pg_catalog.pg_has_role(login_role.oid, database_row.datdba, 'MEMBER') \
                    AS controls_database \
           FROM pg_catalog.pg_roles AS login_role \
           JOIN pg_catalog.pg_database AS database_row \
             ON database_row.datname = pg_catalog.current_database() \
          WHERE login_role.rolname = current_user",
    )
    .fetch_one(&mut *transaction)
    .await
    .map_err(|_| PostgresStoreError::WriterRequired)?;

    let session_role_name = row
        .try_get::<String, _>("session_role_name")
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    let authenticated_role_name = row
        .try_get::<String, _>("authenticated_role_name")
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    let rejected_role_shape = row.try_get::<bool, _>("rolsuper").unwrap_or(true)
        || row.try_get::<bool, _>("rolinherit").unwrap_or(true)
        || row.try_get::<bool, _>("rolcreaterole").unwrap_or(true)
        || row.try_get::<bool, _>("rolcreatedb").unwrap_or(true)
        || !row.try_get::<bool, _>("rolcanlogin").unwrap_or(false)
        || row.try_get::<bool, _>("rolreplication").unwrap_or(true)
        || row.try_get::<bool, _>("rolbypassrls").unwrap_or(true)
        || !row.try_get::<bool, _>("no_role_config").unwrap_or(false)
        || row.try_get::<bool, _>("controls_database").unwrap_or(true);
    if session_role_name != authenticated_role_name || rejected_role_shape {
        return Err(PostgresStoreError::WriterRequired);
    }

    let memberships = sqlx::query(
        "SELECT parent_role.rolname, membership.admin_option, \
                membership.inherit_option, membership.set_option \
           FROM pg_catalog.pg_auth_members AS membership \
           JOIN pg_catalog.pg_roles AS member_role ON member_role.oid = membership.member \
           JOIN pg_catalog.pg_roles AS parent_role ON parent_role.oid = membership.roleid \
          WHERE member_role.rolname = current_user \
          ORDER BY parent_role.rolname",
    )
    .fetch_all(&mut *transaction)
    .await
    .map_err(|_| PostgresStoreError::WriterRequired)?;
    let actual_memberships = memberships
        .iter()
        .map(|membership| {
            if membership
                .try_get::<bool, _>("admin_option")
                .unwrap_or(true)
                || membership
                    .try_get::<bool, _>("inherit_option")
                    .unwrap_or(true)
                || !membership.try_get::<bool, _>("set_option").unwrap_or(false)
            {
                return Err(PostgresStoreError::WriterRequired);
            }
            membership
                .try_get::<String, _>("rolname")
                .map_err(|_| PostgresStoreError::WriterRequired)
        })
        .collect::<Result<BTreeSet<_>>>()?;

    let managed = target.roles().name(kind.role_kind()).to_owned();
    let qualification = target.roles().qualification.clone();
    let expected = BTreeSet::from([qualification.clone(), managed.clone()]);
    if actual_memberships != expected {
        return Err(PostgresStoreError::WriterRequired);
    }

    for role_name in target.roles().managed_names() {
        let settable =
            sqlx::query_scalar::<_, bool>("SELECT pg_catalog.pg_has_role(current_user, $1, 'SET')")
                .bind(role_name)
                .fetch_one(&mut *transaction)
                .await
                .map_err(|_| PostgresStoreError::WriterRequired)?;
        let allowed = role_name == managed.as_str() || role_name == qualification.as_str();
        if settable != allowed {
            return Err(PostgresStoreError::WriterRequired);
        }
    }

    let in_recovery = row
        .try_get::<bool, _>("in_recovery")
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    let transaction_read_only = row
        .try_get::<String, _>("transaction_read_only")
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    let expected_read_only = if kind.expects_write() { "off" } else { "on" };
    if in_recovery || transaction_read_only != expected_read_only {
        return Err(PostgresStoreError::WriterRequired);
    }

    // Managed role names are issued from the target key and double-quoted.
    let set_role = format!("SET LOCAL ROLE {}", quote_ident(&managed));
    sqlx::query(AssertSqlSafe(set_role))
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    let role_oid = sqlx::query_scalar::<_, i64>(
        "SELECT oid::bigint FROM pg_catalog.pg_roles WHERE rolname = $1",
    )
    .bind(&managed)
    .fetch_one(&mut *transaction)
    .await
    .map_err(|_| PostgresStoreError::WriterRequired)?;
    let managed_role_oid =
        u32::try_from(role_oid).map_err(|_| PostgresStoreError::WriterRequired)?;

    sqlx::query(
        "SELECT pg_catalog.set_config( \
             'search_path', pg_catalog.format('%I, pg_catalog', $1), TRUE \
         )",
    )
    .bind(target.schema_name())
    .execute(&mut *transaction)
    .await
    .map_err(|_| PostgresStoreError::WriterRequired)?;
    let schema_name = sqlx::query_scalar::<_, String>("SELECT current_schema()::text")
        .fetch_one(&mut *transaction)
        .await
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    if schema_name != target.schema_name() {
        return Err(PostgresStoreError::WriterRequired);
    }

    transaction
        .rollback()
        .await
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    Ok((session_role_name, managed, managed_role_oid))
}

fn parse_u64(value: &str) -> Result<u64> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| PostgresStoreError::TargetSessionRejected)?;
    if parsed == 0 || parsed.to_string() != value {
        return Err(PostgresStoreError::TargetSessionRejected);
    }
    Ok(parsed)
}

fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
