//! Opaque deployment-issued exact-target session bundles.
//!
//! Production openers consume only these bundles. Ordinary application code never
//! receives a `PgPool`, URL, connection option, or raw writer fence.

use std::collections::BTreeSet;

use mfm_ids::{StoreEpoch, StoreScopeId};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{PgPool, Row};

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
#[derive(Clone)]
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

impl PartialEq for TargetBinding {
    fn eq(&self, other: &Self) -> bool {
        self.database_name == other.database_name
            && self.schema_name == other.schema_name
            && self.database_oid == other.database_oid
            && self.store_scope_id == other.store_scope_id
            && self.store_epoch == other.store_epoch
            && self.target_key == other.target_key
            && self.fence_generation == other.fence_generation
            && self.release_epoch == other.release_epoch
            && self.roles == other.roles
    }
}

impl Eq for TargetBinding {}

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

    pub(crate) fn target_key(&self) -> &TargetKey {
        &self.target_key
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
    managed_role: String,
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
}

/// Opaque application-facing exact-target sessions.
///
/// Holds physically distinct run-read, run-write, and configuration-read capabilities.
/// No writer-capable pool backs a reader.
pub struct PostgresApplicationSessions {
    target: TargetBinding,
    run_reader: RoleSession,
    run_writer: RoleSession,
    configuration_reader: RoleSession,
}

impl PostgresApplicationSessions {
    pub(crate) fn into_run_parts(self) -> (RoleSession, RoleSession, TargetBinding) {
        (self.run_reader, self.run_writer, self.target)
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

impl std::fmt::Debug for PostgresApplicationSessions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PostgresApplicationSessions")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

/// Opaque deployment-maintenance sessions for append-only configuration.
pub struct PostgresConfigurationSessions {
    target: TargetBinding,
    configuration_reader: RoleSession,
    configuration_writer: RoleSession,
}

impl PostgresConfigurationSessions {
    pub(crate) fn into_parts(self) -> (RoleSession, RoleSession, TargetBinding) {
        (
            self.configuration_reader,
            self.configuration_writer,
            self.target,
        )
    }
}

impl std::fmt::Debug for PostgresConfigurationSessions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PostgresConfigurationSessions")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

/// Combined run and configuration sessions used by joint assembly.
pub struct PostgresCombinedSessions {
    target: TargetBinding,
    run_reader: RoleSession,
    run_writer: RoleSession,
    configuration_reader: RoleSession,
    configuration_writer: RoleSession,
}

impl PostgresCombinedSessions {
    /// Drops the configuration writer, leaving application sessions only.
    pub fn into_application(self) -> PostgresApplicationSessions {
        PostgresApplicationSessions {
            target: self.target,
            run_reader: self.run_reader,
            run_writer: self.run_writer,
            configuration_reader: self.configuration_reader,
        }
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

impl std::fmt::Debug for PostgresCombinedSessions {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PostgresCombinedSessions")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

/// Deployment-issued target admission consumed exactly once by the PostgreSQL opener.
///
/// The deployment authority creates this value and retains all credential material inside its
/// own boundary. Application code can only move the admission into an opener; it cannot inspect,
/// clone, or use the underlying logins.
pub struct PostgresTargetAdmission {
    schema_name: String,
    run_reader: String,
    run_writer: String,
    configuration_reader: String,
    configuration_writer: Option<String>,
}

impl std::fmt::Debug for PostgresTargetAdmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PostgresTargetAdmission")
            .field("schema_name", &self.schema_name)
            .finish_non_exhaustive()
    }
}

/// Deployment-side sink used to issue one target admission without exposing
/// the admission's private credential fields to the broker implementation.
///
/// A deployment broker owns the raw login material and calls issue_target
/// exactly once. The resulting admission remains move-only and is consumed by
/// a session opener.
pub trait DeploymentCredentialSink: mfm_authority_seal::DeploymentCredentialSinkSeal {
    /// Issues one target admission from deployment-owned login material.
    fn issue_target(
        &mut self,
        schema_name: String,
        run_reader: String,
        run_writer: String,
        configuration_reader: String,
        configuration_writer: Option<String>,
    ) -> Result<PostgresTargetAdmission>;
}

/// Deployment-side credential broker for one-shot target admission.
pub trait DeploymentCredentialBroker:
    mfm_authority_seal::DeploymentCredentialBrokerSeal + Send + 'static
{
    /// Consumes the broker and asks the PostgreSQL boundary to issue one
    /// opaque admission. Raw credentials never cross this API as a returned
    /// application value.
    fn issue(
        self: Box<Self>,
        sink: &mut dyn DeploymentCredentialSink,
    ) -> Result<PostgresTargetAdmission>;
}

struct AdmissionSink;

impl mfm_authority_seal::DeploymentCredentialSinkSeal for AdmissionSink {}

impl DeploymentCredentialSink for AdmissionSink {
    fn issue_target(
        &mut self,
        schema_name: String,
        run_reader: String,
        run_writer: String,
        configuration_reader: String,
        configuration_writer: Option<String>,
    ) -> Result<PostgresTargetAdmission> {
        if schema_name.is_empty()
            || run_reader.is_empty()
            || run_writer.is_empty()
            || configuration_reader.is_empty()
            || configuration_writer.as_deref().is_some_and(str::is_empty)
        {
            return Err(PostgresStoreError::TargetSessionRejected);
        }
        Ok(PostgresTargetAdmission {
            schema_name,
            run_reader,
            run_writer,
            configuration_reader,
            configuration_writer,
        })
    }
}

impl PostgresTargetAdmission {
    /// Creates an admission from a deployment broker.
    pub fn from_broker<B: DeploymentCredentialBroker>(broker: B) -> Result<Self> {
        Box::new(broker).issue(&mut AdmissionSink)
    }

    #[cfg(feature = "test-support")]
    /// Builds a test-only admission backed by fixture credentials.
    pub fn for_test(
        schema_name: String,
        run_reader: String,
        run_writer: String,
        configuration_reader: String,
        configuration_writer: Option<String>,
    ) -> Self {
        Self {
            schema_name,
            run_reader,
            run_writer,
            configuration_reader,
            configuration_writer,
        }
    }
}

/// Test-only fixture credential. It is intentionally unavailable in production builds.
#[cfg(feature = "test-support")]
#[derive(Clone)]
pub struct TestLoginCredential {
    /// Fixture connection URL.
    pub database_url: String,
}

/// Test-only fixture credential bundle used by the managed PostgreSQL fixture.
#[cfg(feature = "test-support")]
pub struct TestTargetCredentials {
    /// Test target schema.
    pub schema_name: String,
    /// Test run reader login.
    pub run_reader: TestLoginCredential,
    /// Test run writer login.
    pub run_writer: TestLoginCredential,
    /// Test configuration reader login.
    pub configuration_reader: TestLoginCredential,
    /// Test configuration writer login.
    pub configuration_writer: Option<TestLoginCredential>,
}

#[cfg(feature = "test-support")]
impl From<TestTargetCredentials> for PostgresTargetAdmission {
    fn from(materials: TestTargetCredentials) -> Self {
        Self::for_test(
            materials.schema_name,
            materials.run_reader.database_url,
            materials.run_writer.database_url,
            materials.configuration_reader.database_url,
            materials
                .configuration_writer
                .map(|material| material.database_url),
        )
    }
}

#[cfg(feature = "test-support")]
/// Opens fixture application sessions for PostgreSQL integration tests.
pub async fn open_test_application_sessions(
    materials: TestTargetCredentials,
) -> Result<PostgresApplicationSessions> {
    open_application_sessions(materials.into()).await
}

#[cfg(feature = "test-support")]
/// Opens fixture configuration sessions for PostgreSQL integration tests.
pub async fn open_test_configuration_sessions(
    schema_name: String,
    configuration_reader_material: TestLoginCredential,
    configuration_writer_material: TestLoginCredential,
) -> Result<PostgresConfigurationSessions> {
    open_configuration_sessions(PostgresTargetAdmission::for_test(
        schema_name,
        String::new(),
        String::new(),
        configuration_reader_material.database_url,
        Some(configuration_writer_material.database_url),
    ))
    .await
}

#[cfg(feature = "test-support")]
/// Opens fixture combined sessions for PostgreSQL integration tests.
pub async fn open_test_combined_sessions(
    materials: TestTargetCredentials,
) -> Result<PostgresCombinedSessions> {
    open_combined_sessions(materials.into()).await
}

/// Opens an opaque application session bundle for the test-only fixture admission.
pub async fn open_application_sessions(
    admission: PostgresTargetAdmission,
) -> Result<PostgresApplicationSessions> {
    if admission.configuration_writer.is_some() {
        return Err(PostgresStoreError::TargetSessionRejected);
    }
    let PostgresTargetAdmission {
        schema_name,
        run_reader: run_reader_url,
        run_writer: run_writer_url,
        configuration_reader: configuration_reader_url,
        ..
    } = admission;
    let target = load_target_from_login(&run_reader_url, &schema_name).await?;
    let run_reader = open_role_session(
        &run_reader_url,
        &schema_name,
        &target,
        SessionKind::RunReader,
    )
    .await?;
    let run_writer = open_role_session(
        &run_writer_url,
        &schema_name,
        &target,
        SessionKind::RunWriter,
    )
    .await?;
    let configuration_reader = open_role_session(
        &configuration_reader_url,
        &schema_name,
        &target,
        SessionKind::ConfigurationReader,
    )
    .await?;
    if load_target_from_login(&configuration_reader_url, &schema_name).await? != target {
        return Err(PostgresStoreError::TargetSessionRejected);
    }
    let after = load_target_from_login(&run_writer_url, &schema_name).await?;
    if after != target {
        return Err(PostgresStoreError::TargetSessionRejected);
    }
    Ok(PostgresApplicationSessions {
        target,
        run_reader,
        run_writer,
        configuration_reader,
    })
}

/// Opens opaque configuration-maintenance sessions for the test-only fixture admission.
pub async fn open_configuration_sessions(
    admission: PostgresTargetAdmission,
) -> Result<PostgresConfigurationSessions> {
    let PostgresTargetAdmission {
        schema_name,
        configuration_reader: configuration_reader_url,
        configuration_writer: Some(configuration_writer_url),
        ..
    } = admission
    else {
        return Err(PostgresStoreError::TargetSessionRejected);
    };
    let target = load_target_from_login(&configuration_reader_url, &schema_name).await?;
    let configuration_reader = open_role_session(
        &configuration_reader_url,
        &schema_name,
        &target,
        SessionKind::ConfigurationReader,
    )
    .await?;
    let configuration_writer = open_role_session(
        &configuration_writer_url,
        &schema_name,
        &target,
        SessionKind::ConfigurationWriter,
    )
    .await?;
    let after = load_target_from_login(&configuration_writer_url, &schema_name).await?;
    if after != target {
        return Err(PostgresStoreError::TargetSessionRejected);
    }
    Ok(PostgresConfigurationSessions {
        target,
        configuration_reader,
        configuration_writer,
    })
}

/// Opens combined run and configuration sessions for the test-only fixture admission.
pub async fn open_combined_sessions(
    admission: PostgresTargetAdmission,
) -> Result<PostgresCombinedSessions> {
    let PostgresTargetAdmission {
        schema_name,
        run_reader: run_reader_material,
        run_writer: run_writer_material,
        configuration_reader: configuration_reader_material,
        configuration_writer: Some(configuration_writer_material),
        ..
    } = admission
    else {
        return Err(PostgresStoreError::TargetSessionRejected);
    };
    let target = load_target_from_login(&run_reader_material, &schema_name).await?;
    let run_reader = open_role_session(
        &run_reader_material,
        &schema_name,
        &target,
        SessionKind::RunReader,
    )
    .await?;
    let run_writer = open_role_session(
        &run_writer_material,
        &schema_name,
        &target,
        SessionKind::RunWriter,
    )
    .await?;
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
    if load_target_from_login(&configuration_reader_material, &schema_name).await? != target
        || load_target_from_login(&configuration_writer_material, &schema_name).await? != target
    {
        return Err(PostgresStoreError::TargetSessionRejected);
    }
    let after = load_target_from_login(&run_writer_material, &schema_name).await?;
    if after != target {
        return Err(PostgresStoreError::TargetSessionRejected);
    }
    Ok(PostgresCombinedSessions {
        target,
        run_reader,
        run_writer,
        configuration_reader,
        configuration_writer,
    })
}

async fn load_target_from_login(
    database_url: &str,
    expected_schema: &str,
) -> Result<TargetBinding> {
    let pool = connect_login(database_url, expected_schema).await?;
    let identity = validate_authoritative_schema_at(&pool, expected_schema).await?;
    let binding = load_target_binding(&pool, expected_schema, identity).await?;
    pool.close().await;
    Ok(binding)
}

async fn open_role_session(
    database_url: &str,
    expected_schema: &str,
    target: &TargetBinding,
    kind: SessionKind,
) -> Result<RoleSession> {
    let pool = connect_login(database_url, expected_schema).await?;
    let managed_role = probe_login_shape(&pool, target, kind).await?;
    Ok(RoleSession {
        pool,
        kind,
        managed_role,
    })
}

async fn connect_login(database_url: &str, schema: &str) -> Result<PgPool> {
    let options = database_url
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
    sqlx::query(crate::sql_catalog::session_set_role(&qualification))
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
) -> Result<String> {
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

    // Managed role names are issued from the target key and validated by the
    // private catalog bridge before they become SQL.
    sqlx::query(crate::sql_catalog::session_probe_role(&managed))
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
    Ok(managed)
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
