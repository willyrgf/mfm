use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::time::Duration;

use crate::schema::{connect_pool, validate_pool};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_events::v1 as events;
use mfm_ids::{
    ArtifactId, ContentDigest, IdentityError, NodeId, RunId, SchemaId, SeedId, SemanticTypeId,
};
use mfm_spec::v1::MediaType;
use mfm_store::v1::codec::parse_identity;
use mfm_store::v1::{
    event_artifact_requirements, payload_from_json_value, prepared_commit_plan_fingerprint,
    stage_prepared_commit_plan, AdmissionLease, AdmissionToken, AdmissionWaiter,
    ArtifactAuthorityMap, ArtifactByteAuthorityMap, ArtifactEvidenceRef, AsyncStoreFuture,
    CodecError, CommitBase, CommitFingerprint, CommitKey, CommitOrdinal, CommitOutcome,
    CommittedBatch, CommittedRunJournal, CurrentProjectionStore, EventArtifactRequirement,
    ExecutionClaimScope, ExecutionClaimStatus, ExecutionClaimStore, ExpiredExecutionClaim,
    JournalLoadVerifier, KernelEventEnvelope, LogicalEventKey, NowaitSkipAdmissionResult,
    ObservedRunStatus, PersistedKernelEventRecord, PreparedArtifactBytes, PreparedCommitBundle,
    ProjectionSnapshot, ProjectionSnapshotParts, ResourceLaneAuthoritySet, ResourceLaneKey,
    ResourceLaneProjection, RetainedArtifactReadFuture, RetainedArtifactReadProvider,
    RunJournalBackend, RunObservation, RunObservationPage, RunObservationQuery,
    RunObservationStore, RunState, StagedCommitOutcome, StoreCommitOrder, StoreError,
    StoreErrorInspection, StoreScopeId, StoreScopeStore, StreamSeq, VerifiedRetainedArtifactBytes,
};
use serde_json::Value;
use sqlx::{
    postgres::{PgListener, PgPoolOptions, PgRow},
    PgPool, Postgres, QueryBuilder, Row, Transaction,
};

/// Error returned by the PostgreSQL run store.
#[derive(Debug, thiserror::Error)]
pub enum PostgresStoreError {
    /// PostgreSQL run store authority validation failed.
    #[error("postgres run store authority validation failed: {0}")]
    Authority(PostgresStoreAuthorityError),
    /// Typed store contract validation failed.
    #[error("{0}")]
    Store(#[from] StoreError),
    /// PostgreSQL operation failed.
    #[error("postgres run store error: {0}")]
    Database(&'static str),
    /// Persisted postgres run store rows are corrupt.
    #[error("postgres run store corruption: {0}")]
    Corruption(String),
}

impl StoreErrorInspection for PostgresStoreError {
    fn as_store_error(&self) -> Option<&StoreError> {
        match self {
            Self::Store(error) => Some(error),
            Self::Authority(_) | Self::Database(_) | Self::Corruption(_) => None,
        }
    }
}

impl From<IdentityError> for PostgresStoreError {
    fn from(error: IdentityError) -> Self {
        Self::Store(StoreError::from(error))
    }
}

impl From<mfm_events::EventError> for PostgresStoreError {
    fn from(error: mfm_events::EventError) -> Self {
        Self::Store(StoreError::from(error))
    }
}

impl From<mfm_spec::SpecError> for PostgresStoreError {
    fn from(error: mfm_spec::SpecError) -> Self {
        Self::Store(StoreError::Identity(error.to_string()))
    }
}

impl From<CodecError> for PostgresStoreError {
    fn from(error: CodecError) -> Self {
        let (CodecError::Field(message) | CodecError::Identity(message)) = error;
        Self::Corruption(message)
    }
}

pub(crate) type Result<T> = std::result::Result<T, PostgresStoreError>;

/// Closed authority-validation failure category for Postgres run-store setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PostgresStoreAuthorityError {
    /// PostgreSQL could not be reached or the connection string could not be used.
    #[error("connection validation failed")]
    Connection,
    /// The connected store does not carry the exact current authority fingerprint and catalog.
    #[error("store authority mismatch")]
    StoreAuthorityMismatch,
    /// The applied SQLx migration checksum differs from the compiled baseline.
    #[error("migration checksum mismatch")]
    MigrationChecksumMismatch,
}

/// Validated Postgres run-store authority loaded during store construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresStoreAuthority {
    store_scope_id: StoreScopeId,
}

impl PostgresStoreAuthority {
    pub(crate) fn new(store_scope_id: StoreScopeId) -> Self {
        Self { store_scope_id }
    }

    /// Returns the store-owned deployment scope id validated at construction.
    pub fn store_scope_id(&self) -> &StoreScopeId {
        &self.store_scope_id
    }
}

const CURSOR_VERSION: &str = "mfm.run_observation.cursor.v1";
const OBSERVATION_NOTIFY_CHANNEL: &str = "mfm_run_observation";
const OBSERVATION_NOTIFY_PAYLOAD: &str = "changed";
const OBSERVATION_WAIT_POLL_INTERVAL_MS: u64 = 50;
const WAIT_FIFO_ADMISSION_WAITER_LEASE_SECS: i32 = 60;
const EXECUTION_CLAIM_LEASE_SECS: i32 = mfm_store::v1::EXECUTION_CLAIM_LEASE_TTL_SECS as i32;
const MAX_ARTIFACT_BLOB_BYTES: u64 = 16 * 1024 * 1024;
const MAX_OBSERVATION_LIMIT: u32 = 100;

fn database_error(context: &'static str, _error: sqlx::Error) -> PostgresStoreError {
    PostgresStoreError::Database(context)
}

mod admission_lanes;
mod append;
mod artifact_admission;
mod artifact_writes;
mod artifacts;
mod authority;
mod commits;
mod event_rows;
mod fact_projections;
mod fact_queries;
mod observations;
mod projections;
mod resource_lanes;
#[cfg(all(test, feature = "parity-tests"))]
mod tests;
#[cfg(test)]
mod unit_tests;
mod util;

pub use fact_queries::{PostgresFactQueryResult, PostgresFactQueryRow};

use self::{
    admission_lanes::*, artifact_admission::*, artifact_writes::*, artifacts::*, authority::*,
    commits::*, event_rows::*, fact_projections::*, observations::*, projections::*,
    resource_lanes::*, util::*,
};

/// PostgreSQL-backed typed run event store.
///
/// This is the certified typed storage surface for run events, commit keys, artifact evidence, and
/// derived projections.
#[derive(Clone)]
pub struct PostgresStore {
    pub(crate) pool: PgPool,
    authority: PostgresStoreAuthority,
    #[cfg(all(test, feature = "parity-tests"))]
    committed_journal_load_test_barrier: Option<CommittedJournalLoadTestBarrier>,
}

#[cfg(all(test, feature = "parity-tests"))]
#[derive(Clone)]
struct CommittedJournalLoadTestBarrier {
    run_id: RunId,
    records_loaded: std::sync::Arc<tokio::sync::Barrier>,
    release_load: std::sync::Arc<tokio::sync::Barrier>,
    observed_head: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl PostgresStore {
    /// Connects to PostgreSQL, validates the typed schema, and returns a postgres run store.
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = connect_pool(database_url)
            .await
            .map_err(|_| PostgresStoreError::Authority(PostgresStoreAuthorityError::Connection))?;
        let authority = validate_pool(&pool).await?;
        Ok(Self {
            pool,
            authority,
            #[cfg(all(test, feature = "parity-tests"))]
            committed_journal_load_test_barrier: None,
        })
    }

    /// Returns the store authority validated during construction.
    pub fn store_authority(&self) -> &PostgresStoreAuthority {
        &self.authority
    }
}

impl RunJournalBackend for PostgresStore {
    type Error = PostgresStoreError;

    fn backend_append<'a>(
        &'a self,
        bundle: PreparedCommitBundle,
    ) -> AsyncStoreFuture<'a, CommitOutcome, Self::Error> {
        Box::pin(async move { PostgresStore::append_prepared_commit_bundle(self, bundle).await })
    }

    fn backend_load<'a>(
        &'a self,
        verifier: JournalLoadVerifier,
    ) -> AsyncStoreFuture<'a, CommittedRunJournal, Self::Error> {
        Box::pin(async move {
            let mut tx =
                self.pool.begin().await.map_err(|error| {
                    database_error("failed to begin committed journal load", error)
                })?;
            sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
                .execute(&mut *tx)
                .await
                .map_err(|error| {
                    database_error("failed to set committed journal read mode", error)
                })?;
            let records = load_run_stream_tx(&mut tx, verifier.run_id()).await?;
            #[cfg(all(test, feature = "parity-tests"))]
            if let Some(barrier) = self
                .committed_journal_load_test_barrier
                .as_ref()
                .filter(|barrier| &barrier.run_id == verifier.run_id())
            {
                barrier.records_loaded.wait().await;
                barrier.release_load.wait().await;
                let observed_head: Option<i64> =
                    sqlx::query_scalar("SELECT MAX(seq) FROM commits WHERE run_id = $1")
                        .bind(verifier.run_id().as_str())
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(|error| {
                            database_error("failed to observe committed journal test head", error)
                        })?;
                let observed_head = observed_head
                    .map_or(Ok(0), |head| i64_to_nonnegative_u64(head, "commits.seq"))?;
                barrier
                    .observed_head
                    .store(observed_head, std::sync::atomic::Ordering::SeqCst);
            }
            let artifact_bytes =
                load_run_artifact_bytes_tx(&mut tx, verifier.run_id(), &records).await?;
            let journal = verifier.verify(records, artifact_bytes)?;
            tx.commit().await.map_err(|error| {
                database_error("failed to commit committed journal load", error)
            })?;
            Ok(journal)
        })
    }
}

impl CurrentProjectionStore for PostgresStore {
    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, ProjectionSnapshot, Self::Error> {
        Box::pin(async move { load_projection_snapshot_client(&self.pool, run_id).await })
    }

    fn fact_projection_snapshot<'a>(
        &'a self,
    ) -> AsyncStoreFuture<'a, ProjectionSnapshot, Self::Error> {
        Box::pin(async move { load_fact_projection_snapshot_client(&self.pool).await })
    }
}

impl ExecutionClaimStore for PostgresStore {
    type Error = PostgresStoreError;

    fn acquire_execution_claim<'a>(
        &'a self,
        scope: &'a ExecutionClaimScope,
        holder_run_id: &'a RunId,
        token: AdmissionToken,
    ) -> AsyncStoreFuture<'a, NowaitSkipAdmissionResult, Self::Error> {
        Box::pin(async move {
            acquire_execution_claim_client(&self.pool, scope, holder_run_id, token).await
        })
    }

    fn execution_claim_status<'a>(
        &'a self,
        scope: &'a ExecutionClaimScope,
    ) -> AsyncStoreFuture<'a, ExecutionClaimStatus, Self::Error> {
        Box::pin(async move { execution_claim_status_client(&self.pool, scope).await })
    }

    fn renew_execution_claim<'a>(
        &'a self,
        scope: &'a ExecutionClaimScope,
        holder_run_id: &'a RunId,
        token: &'a AdmissionToken,
    ) -> AsyncStoreFuture<'a, Option<AdmissionLease>, Self::Error> {
        Box::pin(async move {
            renew_execution_claim_client(&self.pool, scope, holder_run_id, token).await
        })
    }

    fn release_execution_claim<'a>(
        &'a self,
        scope: &'a ExecutionClaimScope,
        holder_run_id: &'a RunId,
        token: &'a AdmissionToken,
    ) -> AsyncStoreFuture<'a, bool, Self::Error> {
        Box::pin(async move {
            release_execution_claim_client(&self.pool, scope, holder_run_id, token).await
        })
    }

    fn expired_execution_claims<'a>(
        &'a self,
    ) -> AsyncStoreFuture<'a, Vec<ExpiredExecutionClaim>, Self::Error> {
        Box::pin(async move { expired_execution_claims_client(&self.pool).await })
    }

    fn reap_expired_execution_claim<'a>(
        &'a self,
        scope: &'a ExecutionClaimScope,
        holder_run_id: &'a RunId,
        token: &'a AdmissionToken,
    ) -> AsyncStoreFuture<'a, bool, Self::Error> {
        Box::pin(async move {
            reap_expired_execution_claim_client(&self.pool, scope, holder_run_id, token).await
        })
    }
}

impl StoreScopeStore for PostgresStore {
    type Error = PostgresStoreError;

    fn load_store_scope_id<'a>(&'a self) -> AsyncStoreFuture<'a, StoreScopeId, Self::Error> {
        Box::pin(async move { load_store_scope_id_client(&self.pool).await })
    }
}

impl RetainedArtifactReadProvider for PostgresStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a EventArtifactRequirement,
    ) -> RetainedArtifactReadFuture<'a> {
        Box::pin(async move { read_retained_artifact_from_pool(&self.pool, requirement).await })
    }
}

impl RunObservationStore for PostgresStore {
    type Error = PostgresStoreError;

    fn read_run_observations<'a>(
        &'a self,
        query: RunObservationQuery,
    ) -> mfm_store::v1::AsyncStoreFuture<'a, RunObservationPage, Self::Error> {
        Box::pin(async move { read_run_observations_from_pool(&self.pool, query).await })
    }
}
