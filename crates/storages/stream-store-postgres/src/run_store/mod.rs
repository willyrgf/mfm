use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::time::Duration;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_events::v1 as events;
use mfm_ids::{
    ArtifactId, ContentDigest, IdentityError, NodeId, RunId, SchemaId, SeedId, SemanticTypeId,
};
use mfm_spec::v1::MediaType;
use mfm_store::v1::codec::parse_identity;
use mfm_store::v1::{
    payload_from_json_value, prepared_commit_plan_fingerprint, stage_prepared_commit_plan,
    ArtifactAuthorityMap, ArtifactEvidenceRef, AsyncStoreFuture, CodecError, CommitBase,
    CommitFingerprint, CommitKey, CommitOrdinal, CommitOutcome, CommittedBatch,
    EventArtifactRequirement, KernelEventEnvelope, LogicalEventKey, ObservedRunStatus,
    PersistedKernelEventRecord, PreparedArtifactBytes, PreparedCommitBundle, ProjectionSnapshot,
    ProjectionSnapshotParts, ResourceLaneAuthoritySet, ResourceLaneKey, ResourceLaneProjection,
    ResourceLaneWaiterBlock, RetainedArtifactReadFuture, RetainedArtifactReadProvider,
    RunEventStore, RunObservation, RunObservationPage, RunObservationQuery, RunObservationStore,
    RunState, StagedCommitOutcome, StoreError, StoreErrorInspection, StreamSeq,
    VerifiedRunArtifactBytes,
};
use serde_json::Value;
use sqlx::{
    postgres::{PgListener, PgPoolOptions, PgRow},
    PgPool, Postgres, Row, Transaction,
};

use crate::schema::{connect_pool, validate_pool};

/// Error returned by the PostgreSQL run store.
#[derive(Debug, thiserror::Error)]
pub enum PostgresStoreError {
    /// Typed store contract validation failed.
    #[error("{0}")]
    Store(StoreError),
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
            Self::Database(_) | Self::Corruption(_) => None,
        }
    }
}

impl From<StoreError> for PostgresStoreError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
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

const CURSOR_VERSION: &str = "mfm.run_observation.cursor.v1";
const OBSERVATION_NOTIFY_CHANNEL: &str = "mfm_run_observation";
const OBSERVATION_NOTIFY_PAYLOAD: &str = "changed";
const OBSERVATION_WAIT_POLL_INTERVAL_MS: u64 = 50;
const RESOURCE_LANE_WAITER_LEASE_SECS: i32 = 60;
const MAX_ARTIFACT_BLOB_BYTES: u64 = 16 * 1024 * 1024;
const MAX_OBSERVATION_LIMIT: u32 = 100;

fn database_error(context: &'static str, _error: sqlx::Error) -> PostgresStoreError {
    PostgresStoreError::Database(context)
}

mod append;
mod artifact_admission;
mod artifact_writes;
mod artifacts;
mod authority;
mod commits;
mod event_rows;
mod observations;
mod projections;
mod resource_lanes;
mod stream;
#[cfg(all(test, feature = "parity-tests"))]
mod tests;
#[cfg(test)]
mod unit_tests;
mod util;

use self::{
    artifact_admission::*, artifact_writes::*, artifacts::*, authority::*, commits::*,
    event_rows::*, observations::*, projections::*, resource_lanes::*, stream::*, util::*,
};

/// PostgreSQL-backed typed run event store.
///
/// This is the certified typed storage surface for run events, commit keys, artifact evidence, and
/// derived projections.
#[derive(Clone)]
pub struct PostgresRunStore {
    pub(crate) pool: PgPool,
}

impl PostgresRunStore {
    /// Connects to PostgreSQL, validates the typed schema, and returns a postgres run store.
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = connect_pool(database_url).await?;
        validate_pool(&pool).await?;
        Ok(Self { pool })
    }
}

impl RunEventStore for PostgresRunStore {
    type Error = PostgresStoreError;

    fn append_prepared_commit_bundle<'a>(
        &'a self,
        bundle: PreparedCommitBundle,
    ) -> AsyncStoreFuture<'a, CommitOutcome, Self::Error> {
        Box::pin(async move { PostgresRunStore::append_prepared_commit_bundle(self, bundle).await })
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, Vec<KernelEventEnvelope>, Self::Error> {
        Box::pin(async move { load_run_stream_client(&self.pool, run_id).await })
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, StreamSeq, Self::Error> {
        Box::pin(async move {
            let head = read_head(&self.pool, run_id).await?;
            next_seq_from_head(head)
        })
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, ProjectionSnapshot, Self::Error> {
        Box::pin(async move { load_projection_snapshot_client(&self.pool, run_id).await })
    }
}

impl RetainedArtifactReadProvider for PostgresRunStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a EventArtifactRequirement,
    ) -> RetainedArtifactReadFuture<'a> {
        Box::pin(async move { read_retained_artifact_from_pool(&self.pool, requirement).await })
    }
}

impl RunObservationStore for PostgresRunStore {
    type Error = PostgresStoreError;

    fn read_run_observations<'a>(
        &'a self,
        query: RunObservationQuery,
    ) -> mfm_store::v1::AsyncStoreFuture<'a, RunObservationPage, Self::Error> {
        Box::pin(async move { read_run_observations_from_pool(&self.pool, query).await })
    }
}
