#[cfg(any(test, feature = "parity-tests"))]
use std::sync::{Arc, Mutex};

#[cfg(any(test, feature = "parity-tests"))]
use mfm_ids::{AppendRequestId, RunId};
use mfm_ids::{StoreEpoch, StoreScopeId};
#[cfg(any(test, feature = "parity-tests"))]
use mfm_journal::v1::BatchPurpose;
use mfm_store::v1::{RunAccessAuthorityIssuer, StoreAuthorityContext, StoreIdentity};
use sqlx::{PgConnection, PgPool, Row};

use crate::error::{database_error, PostgresStoreError, Result};
use crate::qualification::{AuthoritativeWriterContext, WriterQualification};
use crate::schema::{APPLICATION_ROLE, SCHEMA_CONTRACT_VERSION};

struct PostgresRunJournalBackendInner {
    writer_pool: PgPool,
    writer_context: AuthoritativeWriterContext,
    authority: StoreAuthorityContext,
    #[cfg(any(test, feature = "parity-tests"))]
    commit_failure: Mutex<Option<TestCommitFailureArm>>,
    #[cfg(any(test, feature = "parity-tests"))]
    admission_run_lock_hook: Mutex<Option<TestAdmissionRunLockHookArm>>,
}

#[cfg(any(test, feature = "parity-tests"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[doc(hidden)]
pub enum TestCommitFailurePoint {
    BeforeCommit,
    AfterCommitBeforeAcknowledgement,
}

#[cfg(any(test, feature = "parity-tests"))]
struct TestCommitFailureArm {
    run_id: RunId,
    batch_purpose: BatchPurpose,
    point: TestCommitFailurePoint,
}

#[cfg(any(test, feature = "parity-tests"))]
struct TestAdmissionRunLockHookArm {
    append_request_id: AppendRequestId,
    reached: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

#[cfg(any(test, feature = "parity-tests"))]
#[cfg_attr(not(test), allow(dead_code))]
#[doc(hidden)]
pub struct TestAdmissionRunLockHook {
    reached: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}

#[cfg(any(test, feature = "parity-tests"))]
#[cfg_attr(not(test), allow(dead_code))]
impl TestAdmissionRunLockHook {
    /// Waits until the exact backend lock point is reached.
    pub async fn wait_until_reached(&self) {
        self.reached.notified().await;
    }

    /// Releases the paused append.
    pub fn release(self) {
        self.release.notify_one();
    }
}

#[cfg(any(test, feature = "parity-tests"))]
impl Drop for TestAdmissionRunLockHook {
    fn drop(&mut self) {
        self.release.notify_one();
    }
}

/// Deployment-qualified PostgreSQL backend hidden behind MFM history wrappers.
#[doc(hidden)]
pub struct PostgresRunJournalBackend {
    inner: PostgresRunJournalBackendInner,
}

impl PostgresRunJournalBackend {
    pub(crate) fn from_qualification(
        writer_pool: PgPool,
        qualification: WriterQualification,
    ) -> (Self, RunAccessAuthorityIssuer) {
        let writer = qualification.into_context();
        let identity = StoreIdentity::new(writer.store_scope_id().clone(), writer.store_epoch());
        let (authority, issuer) = StoreAuthorityContext::bootstrap(identity);
        (
            Self {
                inner: PostgresRunJournalBackendInner {
                    writer_pool,
                    writer_context: writer,
                    authority,
                    #[cfg(any(test, feature = "parity-tests"))]
                    commit_failure: Mutex::new(None),
                    #[cfg(any(test, feature = "parity-tests"))]
                    admission_run_lock_hook: Mutex::new(None),
                },
            },
            issuer,
        )
    }

    /// Immutable store scope validated during writer qualification.
    pub fn store_scope_id(&self) -> &StoreScopeId {
        self.inner.authority.store_identity().store_scope_id()
    }

    /// Immutable store epoch validated during writer qualification.
    pub fn store_epoch(&self) -> StoreEpoch {
        self.inner.authority.store_identity().store_epoch()
    }

    /// Rechecks the bounded local proof that this qualified handle still names its writable
    /// PostgreSQL lineage.
    ///
    /// Readiness proves only connection, writer, retained lineage, and minimum schema authority.
    /// It performs no provider, network, semantic callback, or run-progress work.
    pub(crate) async fn check_ready(&self) -> Result<()> {
        let mut transaction = self
            .writer_pool()
            .begin()
            .await
            .map_err(|_| PostgresStoreError::Connection)?;
        let result = async {
            sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
                .execute(&mut *transaction)
                .await
                .map_err(|error| database_error("set readiness isolation", error))?;
            sqlx::query("SET TRANSACTION READ WRITE")
                .execute(&mut *transaction)
                .await
                .map_err(|_| PostgresStoreError::WriterRequired)?;
            sqlx::query(
                "SELECT pg_catalog.set_config('statement_timeout', '5000', TRUE), \
                        pg_catalog.set_config('lock_timeout', '5000', TRUE)",
            )
            .execute(&mut *transaction)
            .await
            .map_err(|error| database_error("bound readiness transaction", error))?;
            self.pin_transaction_schema(&mut transaction)
                .await
                .map_err(|error| database_error("pin readiness schema", error))?;

            let row = sqlx::query(
                "SELECT pg_catalog.current_database()::text AS database_name, \
                        pg_catalog.current_schema()::text AS schema_name, \
                        database.oid::bigint AS database_oid, \
                        pg_catalog.pg_is_in_recovery() AS in_recovery, \
                        pg_catalog.current_setting('transaction_read_only') \
                            AS transaction_read_only, \
                        coalesce( \
                            pg_catalog.pg_has_role( \
                                current_user, application_role.oid, 'SET' \
                            ), \
                            FALSE \
                        ) AS application_role_settable, \
                        coalesce( \
                            pg_catalog.has_table_privilege( \
                                current_user, journal_relation.oid, 'SELECT' \
                            ), \
                            FALSE \
                        ) AS journal_select, \
                        coalesce( \
                            pg_catalog.has_table_privilege( \
                                current_user, journal_relation.oid, 'INSERT' \
                            ), \
                            FALSE \
                        ) AS journal_insert, \
                        identity.row_count AS identity_row_count, \
                        identity.singleton_count AS identity_singleton_count, \
                        identity.store_scope_id, identity.store_epoch, \
                        metadata.row_count AS metadata_row_count, \
                        metadata.singleton_count AS metadata_singleton_count, \
                        metadata.schema_contract_version \
                   FROM pg_catalog.pg_database AS database \
                   LEFT JOIN pg_catalog.pg_roles AS application_role \
                     ON application_role.rolname = $1 \
                   LEFT JOIN pg_catalog.pg_namespace AS namespace \
                     ON namespace.nspname = pg_catalog.current_schema() \
                   LEFT JOIN pg_catalog.pg_class AS journal_relation \
                     ON journal_relation.relnamespace = namespace.oid \
                    AND journal_relation.relname = 'journal_commits' \
                    AND journal_relation.relkind = 'r' \
                  CROSS JOIN ( \
                        SELECT count(*)::bigint AS row_count, \
                               count(*) FILTER (WHERE singleton) AS singleton_count, \
                               min(store_scope_id) FILTER (WHERE singleton) \
                                   AS store_scope_id, \
                               (min(store_epoch) FILTER (WHERE singleton))::text \
                                   AS store_epoch \
                          FROM store_identity \
                  ) AS identity \
                  CROSS JOIN ( \
                        SELECT count(*)::bigint AS row_count, \
                               count(*) FILTER (WHERE singleton) AS singleton_count, \
                               min(schema_contract_version) FILTER (WHERE singleton) \
                                   AS schema_contract_version \
                          FROM store_schema_metadata \
                  ) AS metadata \
                  WHERE database.datname = pg_catalog.current_database()",
            )
            .bind(APPLICATION_ROLE)
            .fetch_one(&mut *transaction)
            .await
            .map_err(|error| database_error("query readiness proof", error))?;

            let in_recovery = row
                .try_get::<bool, _>("in_recovery")
                .map_err(|_| PostgresStoreError::WriterRequired)?;
            let transaction_read_only = row
                .try_get::<String, _>("transaction_read_only")
                .map_err(|_| PostgresStoreError::WriterRequired)?;
            let application_role_settable = row
                .try_get::<bool, _>("application_role_settable")
                .map_err(|_| PostgresStoreError::WriterRequired)?;
            let journal_select = row
                .try_get::<bool, _>("journal_select")
                .map_err(|_| PostgresStoreError::WriterRequired)?;
            let journal_insert = row
                .try_get::<bool, _>("journal_insert")
                .map_err(|_| PostgresStoreError::WriterRequired)?;
            verify_writer_conditions(
                in_recovery,
                &transaction_read_only,
                application_role_settable,
                journal_select,
                journal_insert,
            )?;

            let database_name = row
                .try_get::<String, _>("database_name")
                .map_err(|_| PostgresStoreError::WriterFenceRejected)?;
            let schema_name = row
                .try_get::<String, _>("schema_name")
                .map_err(|_| PostgresStoreError::WriterFenceRejected)?;
            let database_oid = row
                .try_get::<i64, _>("database_oid")
                .ok()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or(PostgresStoreError::WriterFenceRejected)?;
            if database_name != self.inner.writer_context.database_name()
                || schema_name != self.inner.writer_context.schema_name()
                || database_oid != self.inner.writer_context.database_oid()
            {
                return Err(PostgresStoreError::WriterFenceRejected);
            }

            let identity_row_count = row
                .try_get::<i64, _>("identity_row_count")
                .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
            let identity_singleton_count = row
                .try_get::<i64, _>("identity_singleton_count")
                .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
            let metadata_row_count = row
                .try_get::<i64, _>("metadata_row_count")
                .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
            let metadata_singleton_count = row
                .try_get::<i64, _>("metadata_singleton_count")
                .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
            let schema_contract_version = row
                .try_get::<Option<String>, _>("schema_contract_version")
                .map_err(|_| PostgresStoreError::SchemaAuthorityMismatch)?;
            if identity_row_count != 1
                || identity_singleton_count != 1
                || metadata_row_count != 1
                || metadata_singleton_count != 1
                || schema_contract_version.as_deref() != Some(SCHEMA_CONTRACT_VERSION)
            {
                return Err(PostgresStoreError::SchemaAuthorityMismatch);
            }

            let store_scope_id = row
                .try_get::<Option<String>, _>("store_scope_id")
                .map_err(|_| PostgresStoreError::WriterFenceRejected)?;
            let store_epoch = row
                .try_get::<Option<String>, _>("store_epoch")
                .map_err(|_| PostgresStoreError::WriterFenceRejected)?;
            let expected_store_epoch = self.inner.writer_context.store_epoch().get().to_string();
            if store_scope_id.as_deref()
                != Some(self.inner.writer_context.store_scope_id().as_str())
                || store_epoch.as_deref() != Some(expected_store_epoch.as_str())
            {
                return Err(PostgresStoreError::WriterFenceRejected);
            }
            Ok(())
        }
        .await;
        transaction
            .rollback()
            .await
            .map_err(|error| database_error("rollback readiness transaction", error))?;
        result
    }

    pub(crate) fn writer_pool(&self) -> &PgPool {
        &self.inner.writer_pool
    }

    pub(crate) async fn pin_transaction_schema(
        &self,
        connection: &mut PgConnection,
    ) -> std::result::Result<(), sqlx::Error> {
        sqlx::query(
            "SELECT pg_catalog.set_config( \
                 'search_path', pg_catalog.format('%I, pg_catalog', $1), TRUE \
             )",
        )
        .bind(self.inner.writer_context.schema_name())
        .execute(connection)
        .await
        .map(drop)
    }

    pub(crate) fn store_authority_context(&self) -> &StoreAuthorityContext {
        &self.inner.authority
    }

    #[cfg(any(test, feature = "parity-tests"))]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn inject_commit_failure(
        &self,
        run_id: RunId,
        batch_purpose: BatchPurpose,
        point: TestCommitFailurePoint,
    ) -> Result<()> {
        let mut armed = self
            .inner
            .commit_failure
            .lock()
            .map_err(|_| PostgresStoreError::Database("lock test commit failure selector"))?;
        if armed.is_some() {
            return Err(PostgresStoreError::Database(
                "test commit failure selector is already armed",
            ));
        }
        *armed = Some(TestCommitFailureArm {
            run_id,
            batch_purpose,
            point,
        });
        Ok(())
    }

    #[cfg(any(test, feature = "parity-tests"))]
    pub(crate) fn take_commit_failure(
        &self,
        run_id: &RunId,
        batch_purpose: BatchPurpose,
    ) -> Result<Option<TestCommitFailurePoint>> {
        let mut armed = self
            .inner
            .commit_failure
            .lock()
            .map_err(|_| PostgresStoreError::Database("lock test commit failure selector"))?;
        if armed.as_ref().is_some_and(|selector| {
            selector.run_id == *run_id && selector.batch_purpose == batch_purpose
        }) {
            return Ok(armed.take().map(|selector| selector.point));
        }
        Ok(None)
    }

    #[cfg(any(test, feature = "parity-tests"))]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn commit_failure_is_armed(&self) -> Result<bool> {
        self.inner
            .commit_failure
            .lock()
            .map(|armed| armed.is_some())
            .map_err(|_| PostgresStoreError::Database("lock test commit failure selector"))
    }

    #[cfg(any(test, feature = "parity-tests"))]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn inject_before_admission_run_lock(
        &self,
        append_request_id: AppendRequestId,
    ) -> Result<TestAdmissionRunLockHook> {
        let reached = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let mut armed = self
            .inner
            .admission_run_lock_hook
            .lock()
            .map_err(|_| PostgresStoreError::Database("lock admission run-lock test hook"))?;
        if armed.is_some() {
            return Err(PostgresStoreError::Database(
                "admission run-lock test hook is already armed",
            ));
        }
        *armed = Some(TestAdmissionRunLockHookArm {
            append_request_id,
            reached: Arc::clone(&reached),
            release: Arc::clone(&release),
        });
        Ok(TestAdmissionRunLockHook { reached, release })
    }

    #[cfg(any(test, feature = "parity-tests"))]
    pub(crate) async fn pause_before_admission_run_lock(
        &self,
        append_request_id: &AppendRequestId,
    ) -> Result<()> {
        let hook = {
            let mut armed =
                self.inner.admission_run_lock_hook.lock().map_err(|_| {
                    PostgresStoreError::Database("lock admission run-lock test hook")
                })?;
            if armed
                .as_ref()
                .is_some_and(|hook| hook.append_request_id == *append_request_id)
            {
                armed.take()
            } else {
                None
            }
        };
        if let Some(hook) = hook {
            hook.reached.notify_one();
            hook.release.notified().await;
        }
        Ok(())
    }
}

pub(crate) fn verify_writer_conditions(
    in_recovery: bool,
    transaction_read_only: &str,
    application_role_settable: bool,
    journal_select: bool,
    journal_insert: bool,
) -> Result<()> {
    if in_recovery
        || transaction_read_only != "off"
        || !application_role_settable
        || !journal_select
        || !journal_insert
    {
        return Err(PostgresStoreError::WriterRequired);
    }
    Ok(())
}
