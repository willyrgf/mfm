use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use mfm_ids::{StoreEpoch, StoreScopeId};
use mfm_store::structured::{
    ConfigurationHistoryStore, PublicPhysicalBindingVerifier, StructuredProgramVerifier,
    StructuredRunStore,
};
use sqlx::{PgPool, Row};

use crate::configuration::PostgresConfigurationHistoryBackend;
use crate::error::{PostgresStoreError, Result};
use crate::schema::{
    validate_authoritative_schema_at, ValidatedStoreIdentity, APPLICATION_ROLE,
    CONFIGURATION_MAINTENANCE_ROLE, OWNER_ROLE, QUALIFICATION_ROLE,
};
use crate::structured::PostgresStructuredHistoryBackend;

/// Future returned by a deployment-owned authoritative-writer fence.
pub type AuthoritativeWriterFenceFuture<'a, E> =
    Pin<Box<dyn Future<Output = std::result::Result<(), E>> + Send + 'a>>;

/// Deployment authority that proves one non-rollback writable PostgreSQL lineage.
///
/// Implementations are supplied by the deployment that owns database promotion, WAL/backup
/// lineage, and stale-primary fencing. Returning success asserts that the exact database,
/// schema, store scope, and epoch in [`AuthoritativeWriterContext`] have one writable lineage;
/// that every published run suffix and tenant fact head is present; and that stale and sibling
/// writers are permanently fenced.
///
/// This crate deliberately provides no production implementation, boolean bypass, connection
/// option, or "caught-up replica" mode. PostgreSQL recovery state and local schema checks are
/// necessary but cannot replace this external proof. The supplied pool retains its restricted
/// ordinary login; implementations that inspect store rows must pin the context schema and use
/// transaction-scoped `mfm_store_qualification` role selection.
pub trait AuthoritativeWriterFence: Send + Sync {
    /// Deployment-private verification error.
    type Error: Send + Sync + 'static;

    /// Verifies the supplied writer pool and its exact retained identity.
    fn verify<'a>(
        &'a self,
        writer_pool: &'a PgPool,
        context: &'a AuthoritativeWriterContext,
    ) -> AuthoritativeWriterFenceFuture<'a, Self::Error>;
}

/// Non-secret identity presented to the deployment-owned writer fence.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthoritativeWriterContext {
    database_name: String,
    schema_name: String,
    database_oid: u32,
    store_scope_id: StoreScopeId,
    store_epoch: StoreEpoch,
}

impl AuthoritativeWriterContext {
    /// Connected PostgreSQL database name.
    pub fn database_name(&self) -> &str {
        &self.database_name
    }

    /// Schema containing the closed MFM authority catalog.
    pub fn schema_name(&self) -> &str {
        &self.schema_name
    }

    /// PostgreSQL database OID observed on the writer connection.
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
}

/// Opens the sole structured RunHistory store after deployment-owned writer fencing.
///
/// The pool must authenticate an exact non-inheriting application session login; migration,
/// owner, role-substituted, and multiply privileged sessions are rejected. It is consumed and
/// retained only by the real SQL backend. Qualification is repeated on every fresh process or
/// pool; database unavailability or a missing/rejected fence fails closed.
pub async fn open_structured_authoritative<F>(
    writer_pool: PgPool,
    deployment_writer_fence: F,
    program_verifier: Arc<dyn StructuredProgramVerifier>,
    physical_binding_verifier: Arc<dyn PublicPhysicalBindingVerifier>,
) -> Result<StructuredRunStore<PostgresStructuredHistoryBackend>>
where
    F: AuthoritativeWriterFence,
{
    let context = qualify_writer(
        &writer_pool,
        &deployment_writer_fence,
        SessionAuthority::Application,
    )
    .await?;
    Ok(StructuredRunStore::new(
        PostgresStructuredHistoryBackend::new(writer_pool, context),
        program_verifier,
        physical_binding_verifier,
    ))
}

/// Opens structured RunHistory together with append-only configured-value history.
///
/// The pool must authenticate an exact non-inheriting combined assembly login with only the
/// qualification, application, and configuration-maintenance memberships.
///
/// The returned configuration store is split once by assembly: deployment maintenance retains
/// its non-cloneable writer, while normal application admission receives only the cloneable
/// resolve capability. The underlying pool and role-switching backend remain private.
pub async fn open_structured_authoritative_with_configuration<F>(
    writer_pool: PgPool,
    deployment_writer_fence: F,
    program_verifier: Arc<dyn StructuredProgramVerifier>,
    physical_binding_verifier: Arc<dyn PublicPhysicalBindingVerifier>,
) -> Result<(
    StructuredRunStore<PostgresStructuredHistoryBackend>,
    ConfigurationHistoryStore<PostgresConfigurationHistoryBackend>,
)>
where
    F: AuthoritativeWriterFence,
{
    let context = qualify_writer(
        &writer_pool,
        &deployment_writer_fence,
        SessionAuthority::ApplicationAndConfiguration,
    )
    .await?;
    let configuration_backend =
        PostgresConfigurationHistoryBackend::new_application(writer_pool.clone(), context.clone());
    let run_history = StructuredRunStore::new(
        PostgresStructuredHistoryBackend::new(writer_pool, context),
        program_verifier,
        physical_binding_verifier,
    );
    Ok((
        run_history,
        ConfigurationHistoryStore::new(configuration_backend),
    ))
}

/// Opens the application boundary with structured RunHistory authority and resolve-only
/// configured-value history.
///
/// The pool uses the same exact application-only session profile as
/// [`open_structured_authoritative`]; resolve access does not require maintenance membership.
///
/// The configuration append half is discarded inside assembly and cannot reach the returned
/// application-facing capability set.
pub async fn open_structured_authoritative_application<F>(
    writer_pool: PgPool,
    deployment_writer_fence: F,
    program_verifier: Arc<dyn StructuredProgramVerifier>,
    physical_binding_verifier: Arc<dyn PublicPhysicalBindingVerifier>,
) -> Result<(
    StructuredRunStore<PostgresStructuredHistoryBackend>,
    mfm_store::structured::ConfigurationHistoryReader<PostgresConfigurationHistoryBackend>,
)>
where
    F: AuthoritativeWriterFence,
{
    let context = qualify_writer(
        &writer_pool,
        &deployment_writer_fence,
        SessionAuthority::Application,
    )
    .await?;
    let configuration = ConfigurationHistoryStore::new(
        PostgresConfigurationHistoryBackend::new_application(writer_pool.clone(), context.clone()),
    );
    let run_history = StructuredRunStore::new(
        PostgresStructuredHistoryBackend::new(writer_pool, context),
        program_verifier,
        physical_binding_verifier,
    );
    let (_writer, reader) = configuration.split();
    Ok((run_history, reader))
}

/// Opens the separately held deployment-maintenance authority for append-only configuration.
///
/// The pool must authenticate an exact non-inheriting maintenance session login with only the
/// qualification and configuration-maintenance memberships. This boundary returns no RunHistory
/// writer, runtime, application facade, or resolve handle.
pub async fn open_configuration_maintenance<F>(
    writer_pool: PgPool,
    deployment_writer_fence: F,
) -> Result<mfm_store::structured::ConfigurationHistoryWriter<PostgresConfigurationHistoryBackend>>
where
    F: AuthoritativeWriterFence,
{
    let context = qualify_writer(
        &writer_pool,
        &deployment_writer_fence,
        SessionAuthority::ConfigurationMaintenance,
    )
    .await?;
    let configuration = ConfigurationHistoryStore::new(
        PostgresConfigurationHistoryBackend::new_maintenance(writer_pool, context),
    );
    let (writer, _reader) = configuration.split();
    Ok(writer)
}

async fn qualify_writer<F>(
    writer_pool: &PgPool,
    deployment_writer_fence: &F,
    session_authority: SessionAuthority,
) -> Result<AuthoritativeWriterContext>
where
    F: AuthoritativeWriterFence,
{
    let before = probe_writer(writer_pool, session_authority).await?;
    let session_role_name = before.session_role_name.clone();
    let identity = validate_authoritative_schema_at(writer_pool, &before.schema_name).await?;
    let context = before.with_identity(identity);

    deployment_writer_fence
        .verify(writer_pool, &context)
        .await
        .map_err(|_| PostgresStoreError::WriterFenceRejected)?;

    let after = probe_writer(writer_pool, session_authority).await?;
    if after.session_role_name != session_role_name {
        return Err(PostgresStoreError::WriterFenceRejected);
    }
    let retained_identity =
        validate_authoritative_schema_at(writer_pool, &after.schema_name).await?;
    let after_context = after.with_identity(retained_identity);
    if after_context != context {
        return Err(PostgresStoreError::WriterFenceRejected);
    }
    Ok(context)
}

struct WriterProbe {
    database_name: String,
    schema_name: String,
    database_oid: u32,
    session_role_name: String,
}

impl WriterProbe {
    fn with_identity(self, identity: ValidatedStoreIdentity) -> AuthoritativeWriterContext {
        AuthoritativeWriterContext {
            database_name: self.database_name,
            schema_name: self.schema_name,
            database_oid: self.database_oid,
            store_scope_id: identity.store_scope_id,
            store_epoch: identity.store_epoch,
        }
    }
}

#[derive(Clone, Copy)]
enum SessionAuthority {
    Application,
    ApplicationAndConfiguration,
    ConfigurationMaintenance,
}

impl SessionAuthority {
    fn expected_memberships(self) -> &'static [&'static str] {
        match self {
            Self::Application => &[QUALIFICATION_ROLE, APPLICATION_ROLE],
            Self::ApplicationAndConfiguration => &[
                QUALIFICATION_ROLE,
                APPLICATION_ROLE,
                CONFIGURATION_MAINTENANCE_ROLE,
            ],
            Self::ConfigurationMaintenance => &[QUALIFICATION_ROLE, CONFIGURATION_MAINTENANCE_ROLE],
        }
    }
}

async fn probe_writer(pool: &PgPool, session_authority: SessionAuthority) -> Result<WriterProbe> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|_| PostgresStoreError::Connection)?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    sqlx::query("SET TRANSACTION READ WRITE")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresStoreError::WriterRequired)?;

    let row = sqlx::query(
        "SELECT current_database()::text AS database_name, \
                current_user::text AS session_role_name, \
                session_user::text AS authenticated_role_name, \
                (SELECT oid::bigint FROM pg_catalog.pg_database \
                  WHERE datname = current_database()) AS database_oid, \
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
    let expected_memberships = session_authority
        .expected_memberships()
        .iter()
        .copied()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if actual_memberships != expected_memberships {
        return Err(PostgresStoreError::WriterRequired);
    }

    for managed_role in [
        OWNER_ROLE,
        QUALIFICATION_ROLE,
        APPLICATION_ROLE,
        CONFIGURATION_MAINTENANCE_ROLE,
    ] {
        let settable =
            sqlx::query_scalar::<_, bool>("SELECT pg_catalog.pg_has_role(current_user, $1, 'SET')")
                .bind(managed_role)
                .fetch_one(&mut *transaction)
                .await
                .map_err(|_| PostgresStoreError::WriterRequired)?;
        if settable != expected_memberships.contains(managed_role) {
            return Err(PostgresStoreError::WriterRequired);
        }
    }

    let in_recovery = row
        .try_get::<bool, _>("in_recovery")
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    let transaction_read_only = row
        .try_get::<String, _>("transaction_read_only")
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    if in_recovery || transaction_read_only != "off" {
        return Err(PostgresStoreError::WriterRequired);
    }

    sqlx::query("SET LOCAL ROLE mfm_store_qualification")
        .execute(&mut *transaction)
        .await
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    let schema_name = sqlx::query_scalar::<_, String>("SELECT current_schema()::text")
        .fetch_one(&mut *transaction)
        .await
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    if schema_name == "pg_catalog" {
        return Err(PostgresStoreError::WriterRequired);
    }

    let database_name = row
        .try_get::<String, _>("database_name")
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    let database_oid = row
        .try_get::<i64, _>("database_oid")
        .ok()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(PostgresStoreError::WriterRequired)?;

    transaction
        .rollback()
        .await
        .map_err(|_| PostgresStoreError::WriterRequired)?;
    Ok(WriterProbe {
        database_name,
        schema_name,
        database_oid,
        session_role_name,
    })
}

#[cfg(any(test, feature = "parity-tests"))]
/// Test-only fence for database conformance tests.
pub struct TestAuthoritativeWriterFence;

#[cfg(any(test, feature = "parity-tests"))]
impl AuthoritativeWriterFence for TestAuthoritativeWriterFence {
    type Error = std::convert::Infallible;

    fn verify<'a>(
        &'a self,
        _writer_pool: &'a PgPool,
        _context: &'a AuthoritativeWriterContext,
    ) -> AuthoritativeWriterFenceFuture<'a, Self::Error> {
        Box::pin(async { Ok(()) })
    }
}
