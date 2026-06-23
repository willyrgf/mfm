use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_events::v1 as events;
use mfm_ids::{
    ArtifactId, ContentDigest, IdentityError, NodeId, RunId, SchemaId, SeedId, SemanticTypeId,
};
use mfm_spec::v1::{MediaType, ResourceNamespace};
use mfm_store::v1::codec::{
    parse_identity, parse_side_effect_ledger_purpose, side_effect_ledger_purpose_json,
};
use mfm_store::v1::{
    payload_from_json_value, prepared_commit_plan_fingerprint, stage_prepared_commit_plan,
    ArtifactAuthorityMap, ArtifactEvidenceRef, AsyncStoreFuture, CodecError, CommitBase,
    CommitFingerprint, CommitKey, CommitOrdinal, CommitOutcome, CommittedBatch,
    EventArtifactRequirement, KernelEventEnvelope, LogicalEventKey, ObservedRunStatus,
    PersistedKernelEventRecord, PreparedArtifactBytes, PreparedCommitBundle, ProjectionSnapshot,
    ProjectionSnapshotParts, ResourceLaneAuthoritySet, ResourceLaneKey, ResourceLaneProjection,
    RetainedArtifactReadFuture, RetainedArtifactReadProvider, RunEventStore, RunObservation,
    RunObservationPage, RunObservationQuery, RunObservationStore, RunState, StagedCommitOutcome,
    StoreError, StoreErrorInspection, StreamSeq, VerifiedRunArtifactBytes,
};
use ring::hmac;
use serde_json::Value;
use sqlx::{postgres::PgRow, PgPool, Postgres, Row, Transaction};

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

const HASH_DOMAIN_VERSION: &str = "mfm.hash-domain.v1";
const CANONICALIZER_IDENTITY: &str = "sha256-jcs-v1";
const PROJECTION_VERSION: &str = "mfm.run_observation.v1";
const RUN_OBSERVATION_SUMMARY_KIND: &str = "run";
const CURSOR_VERSION: &str = "mfm.run_observation.cursor.v1";
const MAX_OBSERVATION_LIMIT: u32 = 100;

fn database_error(context: &'static str, _error: sqlx::Error) -> PostgresStoreError {
    PostgresStoreError::Database(context)
}

/// Maintenance build mode for projection-versioned Postgres read models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadModelBuildMode {
    /// Insert only missing rows for the requested projection version and verify existing rows.
    MissingOnly,
    /// Build rows for a projection version without rewriting any existing version.
    NewVersion,
}

/// Result of a read-model build operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadModelBuildReport {
    /// Projection version that was built or verified.
    pub projection_version: String,
    /// Rows inserted by the build.
    pub inserted_rows: u64,
    /// Existing rows whose hashes matched the rebuilt authority row.
    pub verified_rows: u64,
}

/// Result of validating a projection-versioned read model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadModelValidationReport {
    /// Projection version that was checked.
    pub projection_version: String,
    /// Authority commits checked against the read model.
    pub checked_commits: u64,
    /// Missing read-model rows for the requested version.
    pub missing_rows: u64,
    /// Rows present but not equal to the rebuilt authority-derived row.
    pub drift_rows: u64,
}

/// High watermark for immutable commit-log cursor authority and observation summaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadModelHighWatermark {
    /// Number of immutable commit-log rows.
    pub commit_log_rows: u64,
    /// Number of projection-versioned observation summary rows.
    pub summary_rows: u64,
    /// Highest sealed append transaction id currently present in the commit log.
    pub high_append_xid: Option<String>,
    /// Commit id at the high watermark.
    pub high_commit_id: Option<String>,
    /// RFC v1 bytewise commit sort key at the high watermark.
    pub high_commit_sort_key: Option<Vec<u8>>,
}

/// One read-model drift finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadModelDrift {
    /// Projection version of the affected row.
    pub projection_version: String,
    /// Commit id of the affected summary.
    pub commit_id: String,
    /// Kind of summary row.
    pub summary_kind: String,
    /// Drift class.
    pub kind: ReadModelDriftKind,
}

/// Read-model drift class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadModelDriftKind {
    /// An authority commit has no corresponding summary row.
    Missing,
    /// A summary row's stored hash or canonical bytes do not match the rebuilt row.
    Mismatched,
    /// A summary row references no live commit authority row.
    Orphaned,
}

/// Report of read-model drift across projection versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadModelDriftReport {
    /// Drift findings.
    pub findings: Vec<ReadModelDrift>,
}

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

    /// Connects using the `DATABASE_URL` environment variable.
    pub async fn connect_env() -> Result<Self> {
        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| PostgresStoreError::Database("missing DATABASE_URL"))?;
        Self::connect(&database_url).await
    }

    /// Returns the next store-owned stream sequence for a run.
    pub async fn expected_next_seq(&self, run_id: &RunId) -> Result<StreamSeq> {
        let head = read_head(&self.pool, run_id).await?;
        next_seq_from_head(head)
    }

    /// Atomically admits artifact evidence and appends one typed run commit.
    pub async fn append_prepared_commit_bundle(
        &self,
        bundle: PreparedCommitBundle,
    ) -> Result<CommitOutcome> {
        let plan = bundle.plan();
        let request = plan.request();
        let fingerprint = prepared_commit_plan_fingerprint(plan)?;

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| database_error("failed to start transaction", error))?;

        let prepared_authority = prepared_commit_authority(plan, &fingerprint)?;

        if let Some(stored) =
            read_commit_by_key(&mut tx, request.run_id(), request.commit_key().as_str()).await?
        {
            if stored.commit_idempotency_hash == prepared_authority.commit_idempotency_hash {
                let batch = load_committed_batch_tx(&mut tx, request.run_id(), stored.seq).await?;
                tx.commit()
                    .await
                    .map_err(|_| PostgresStoreError::Database("failed to commit transaction"))?;
                return Ok(CommitOutcome::Idempotent(batch));
            }
            return Err(StoreError::CommitConflict {
                commit_key: request.commit_key().clone(),
            }
            .into());
        }

        lock_run_tx(&mut tx, request.run_id()).await?;
        let head = read_head_tx(&mut tx, request.run_id()).await?;

        // A same-run transaction may have inserted the key while this transaction waited for the
        // run lock. Re-check before sequence validation while preserving the required initial
        // commit-key lookup order.
        if let Some(stored) =
            read_commit_by_key(&mut tx, request.run_id(), request.commit_key().as_str()).await?
        {
            if stored.commit_idempotency_hash == prepared_authority.commit_idempotency_hash {
                let batch = load_committed_batch_tx(&mut tx, request.run_id(), stored.seq).await?;
                tx.commit()
                    .await
                    .map_err(|_| PostgresStoreError::Database("failed to commit transaction"))?;
                return Ok(CommitOutcome::Idempotent(batch));
            }
            return Err(StoreError::CommitConflict {
                commit_key: request.commit_key().clone(),
            }
            .into());
        }

        verify_prepared_artifact_bundle_tx(&mut tx, &bundle).await?;
        let mut artifacts = load_artifacts(&mut tx, request.run_id()).await?;
        admit_artifact_evidence(&mut artifacts, bundle.admitted_artifacts())?;
        lock_resource_lanes_for_request_tx(&mut tx, request).await?;
        let (run_projection, stream_head) =
            rebuild_projection_snapshot_with_head(&mut tx, request.run_id()).await?;
        if stream_head != head {
            return Err(PostgresStoreError::Corruption(
                "run commit head does not match persisted event stream".to_owned(),
            ));
        }
        let resource_lanes = load_active_resource_lanes_tx(&mut tx).await?;
        let resource_lane_authority = load_resource_lane_authority_tx(&mut tx).await?;
        let projections = projection_snapshot_with_resource_lanes(&run_projection, resource_lanes)?;
        let base = CommitBase {
            artifacts,
            logical_keys: load_logical_keys(&mut tx, request.run_id()).await?,
            unique_logical_payloads: load_unique_logical_payloads(&mut tx, request.run_id())
                .await?,
            projections,
            resource_lane_authority,
            actual_next_seq: next_seq_from_head(head)?,
        };
        let staged = match stage_prepared_commit_plan(&base, plan)? {
            StagedCommitOutcome::Staged(staged) => staged,
            StagedCommitOutcome::ResourceLaneClaimBlocked(block) => {
                return Ok(CommitOutcome::ResourceLaneClaimBlocked(block));
            }
        };
        let batch = staged.batch().clone();
        let commit_id = derive_commit_id(
            request.run_id(),
            batch.seq(),
            request.commit_key(),
            plan.purpose_name(),
            &prepared_authority.prepared_authority_hash,
        )?;
        let final_authority = final_commit_authority(plan, &bundle, &batch, &commit_id)?;

        let commit_seq = u64_to_i64(batch.seq().as_u64(), "commits.seq")?;
        let event_count = i32::try_from(batch.events().len())
            .map_err(|_| PostgresStoreError::Corruption("event count overflow".into()))?;
        sqlx::query(
            "INSERT INTO commits \
             (commit_id, run_id, seq, commit_key, commit_purpose, commit_idempotency_hash, \
              idempotency_canonical_json, prepared_authority_hash, prepared_authority_canonical_json, \
              commit_batch_hash, commit_batch_canonical_json, hash_domain_version, \
              canonicalizer_identity, event_count) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
        )
        .bind(&commit_id)
        .bind(request.run_id().as_str())
        .bind(commit_seq)
        .bind(request.commit_key().as_str())
        .bind(plan.purpose_name())
        .bind(&prepared_authority.commit_idempotency_hash)
        .bind(prepared_authority.idempotency_canonical_json.as_slice())
        .bind(&prepared_authority.prepared_authority_hash)
        .bind(prepared_authority.prepared_authority_canonical_json.as_slice())
        .bind(&final_authority.commit_batch_hash)
        .bind(final_authority.commit_batch_canonical_json.as_slice())
        .bind(HASH_DOMAIN_VERSION)
        .bind(CANONICALIZER_IDENTITY)
        .bind(event_count)
        .execute(&mut *tx)
        .await
        .map_err(|error| database_error("failed to insert commit authority", error))?;

        admit_artifact_bundle_tx(
            &mut tx,
            request.run_id(),
            request.commit_key(),
            batch.seq(),
            &commit_id,
            &bundle,
        )
        .await?;

        for event in batch.events() {
            let payload_canonical_json = payload_canonical_bytes(event.payload())?;
            let seq = u64_to_i64(event.seq().as_u64(), "run_events.seq")?;
            let ordinal = i32::try_from(event.ordinal().as_u32()).map_err(|_| {
                PostgresStoreError::Corruption("run_events.ordinal overflow".into())
            })?;
            sqlx::query(
                "INSERT INTO run_events \
                 (run_id, seq, ordinal, commit_id, event_id, event_schema_id, spec_hash, commit_key, \
                  logical_key, payload_hash, payload_canonical_json) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
            )
            .bind(event.run_id().as_str())
            .bind(seq)
            .bind(ordinal)
            .bind(&commit_id)
            .bind(event.event_id().as_str())
            .bind(event.event_schema_id().as_str())
            .bind(event.spec_hash().as_str())
            .bind(event.commit_key().as_str())
            .bind(event.logical_key().as_str())
            .bind(event.payload_hash().as_str())
            .bind(payload_canonical_json.as_bytes())
            .execute(&mut *tx)
            .await
            .map_err(|error| database_error("failed to insert run event", error))?;
        }
        let source_event_count = count_run_events_tx(&mut tx, request.run_id()).await?;
        insert_resource_lane_authority_rows_tx(&mut tx, &commit_id, batch.events()).await?;
        insert_run_commit_log_tx(
            &mut tx,
            request,
            &commit_id,
            &final_authority.commit_batch_hash,
            batch.seq(),
            event_count,
        )
        .await?;
        insert_run_observation_summary_tx(
            &mut tx,
            staged.projections(),
            request.run_id(),
            &commit_id,
            batch.seq(),
            source_event_count,
        )
        .await?;

        tx.commit()
            .await
            .map_err(|error| database_error("failed to commit transaction", error))?;
        Ok(CommitOutcome::Appended(batch))
    }

    /// Loads the current projection snapshot from authoritative event rows.
    pub async fn projection_snapshot(&self, run_id: &RunId) -> Result<ProjectionSnapshot> {
        load_projection_snapshot_client(&self.pool, run_id).await
    }

    /// Loads the authoritative typed run stream from persisted event rows.
    pub async fn load_run_stream(&self, run_id: &RunId) -> Result<Vec<KernelEventEnvelope>> {
        load_run_stream_client(&self.pool, run_id).await
    }

    /// Reads retained artifact bytes from Postgres-owned artifact authority.
    pub async fn read_retained_artifact(
        &self,
        requirement: &EventArtifactRequirement,
    ) -> mfm_store::v1::Result<VerifiedRunArtifactBytes> {
        read_retained_artifact_from_pool(&self.pool, requirement).await
    }

    /// Reads adapter-requested artifact bytes from Postgres-owned artifact authority.
    pub async fn read_artifact(
        &self,
        request: &mfm_artifact_capabilities::ArtifactReadRequest,
    ) -> mfm_artifact_capabilities::Result<mfm_artifact_capabilities::VerifiedArtifactBytes> {
        read_artifact_from_pool(&self.pool, request).await
    }

    /// Reads one observation list/watch page from Postgres-owned read models.
    pub async fn read_run_observations(
        &self,
        query: RunObservationQuery,
    ) -> Result<RunObservationPage> {
        read_run_observations_from_pool(&self.pool, query).await
    }
}

/// Explicit maintenance surface for Postgres read models and cursor epoch rotation.
///
/// Production deployments should construct this with maintenance database credentials, not with the
/// normal app/CLI/REST runtime credentials.
#[derive(Clone)]
pub struct PostgresMaintenance {
    pool: PgPool,
}

impl PostgresMaintenance {
    /// Connects to PostgreSQL, validates the typed schema, and returns the maintenance surface.
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = connect_pool(database_url).await?;
        validate_pool(&pool).await?;
        Ok(Self { pool })
    }

    /// Connects using the `DATABASE_URL` environment variable.
    pub async fn connect_env() -> Result<Self> {
        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| PostgresStoreError::Database("missing DATABASE_URL"))?;
        Self::connect(&database_url).await
    }

    /// Builds or verifies a projection-versioned read model from authority rows.
    pub async fn build_projection_version(
        &self,
        projection_version: &str,
        mode: ReadModelBuildMode,
    ) -> Result<ReadModelBuildReport> {
        build_projection_version_from_pool(&self.pool, projection_version, mode).await
    }

    /// Validates one projection-versioned read model without mutating it.
    pub async fn validate_read_models(
        &self,
        projection_version: &str,
    ) -> Result<ReadModelValidationReport> {
        validate_read_models_from_pool(&self.pool, projection_version).await
    }

    /// Reads the commit-log and summary high watermark for operational maintenance.
    pub async fn read_model_high_watermark(&self) -> Result<ReadModelHighWatermark> {
        read_model_high_watermark_from_pool(&self.pool).await
    }

    /// Returns drift findings across projection-versioned read models.
    pub async fn read_model_drift_report(&self) -> Result<ReadModelDriftReport> {
        read_model_drift_report_from_pool(&self.pool).await
    }

    /// Rotates the store epoch and cursor MAC secret after restore, clone, import, or rollback.
    pub async fn reseed_store_epoch(&self) -> Result<String> {
        reseed_store_epoch_from_pool(&self.pool).await
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
        Box::pin(async move { PostgresRunStore::load_run_stream(self, run_id).await })
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, StreamSeq, Self::Error> {
        Box::pin(async move { PostgresRunStore::expected_next_seq(self, run_id).await })
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, ProjectionSnapshot, Self::Error> {
        Box::pin(async move { PostgresRunStore::projection_snapshot(self, run_id).await })
    }
}

impl RetainedArtifactReadProvider for PostgresRunStore {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a EventArtifactRequirement,
    ) -> RetainedArtifactReadFuture<'a> {
        Box::pin(async move { PostgresRunStore::read_retained_artifact(self, requirement).await })
    }
}

impl mfm_artifact_capabilities::ArtifactReadProvider for PostgresRunStore {
    fn read_artifact<'a>(
        &'a self,
        request: &'a mfm_artifact_capabilities::ArtifactReadRequest,
    ) -> mfm_artifact_capabilities::ArtifactReadFuture<'a> {
        Box::pin(async move { PostgresRunStore::read_artifact(self, request).await })
    }
}

impl RunObservationStore for PostgresRunStore {
    type Error = PostgresStoreError;

    fn read_run_observations<'a>(
        &'a self,
        query: RunObservationQuery,
    ) -> mfm_store::v1::AsyncStoreFuture<'a, RunObservationPage, Self::Error> {
        Box::pin(async move { PostgresRunStore::read_run_observations(self, query).await })
    }
}

#[derive(Clone)]
struct StoreMetadata {
    store_epoch: String,
    cursor_key_id: String,
    cursor_secret: Vec<u8>,
}

#[derive(Clone)]
struct CursorPosition {
    append_xid: String,
    commit_sort_key: Vec<u8>,
    kind: CursorKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CursorKind {
    Frontier,
    Row,
}

impl CursorKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Frontier => "frontier",
            Self::Row => "row",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "frontier" => Ok(Self::Frontier),
            "row" => Ok(Self::Row),
            _ => Err(StoreError::InvalidCursor {
                message: "unknown cursor kind".to_owned(),
            }
            .into()),
        }
    }
}

struct ObservationRow {
    observation: RunObservation,
    append_xid: String,
    commit_sort_key: Vec<u8>,
}

async fn read_run_observations_from_pool(
    pool: &PgPool,
    query: RunObservationQuery,
) -> Result<RunObservationPage> {
    if query.limit == 0 || query.limit > MAX_OBSERVATION_LIMIT {
        return Err(StoreError::LimitOutOfRange {
            limit: query.limit,
            max: MAX_OBSERVATION_LIMIT,
        }
        .into());
    }
    let metadata = load_store_metadata(pool).await?;
    let cursor = query
        .cursor
        .as_deref()
        .map(|cursor| decode_observation_cursor(cursor, &metadata))
        .transpose()?;
    let mut page =
        read_run_observation_page_once(pool, &metadata, cursor.as_ref(), query.limit).await?;
    if page.runs.is_empty() && cursor.is_some() && query.wait_ms > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(query.wait_ms)).await;
        page =
            read_run_observation_page_once(pool, &metadata, cursor.as_ref(), query.limit).await?;
    }
    Ok(page)
}

async fn read_run_observation_page_once(
    pool: &PgPool,
    metadata: &StoreMetadata,
    cursor: Option<&CursorPosition>,
    limit: u32,
) -> Result<RunObservationPage> {
    let frontier_xid = sealed_frontier_xid(pool).await?;
    let rows = match cursor {
        Some(cursor) => read_observation_watch_rows(pool, cursor, &frontier_xid, limit).await?,
        None => read_observation_list_rows(pool, &frontier_xid, limit).await?,
    };
    let next_position = if cursor.is_some() && rows.len() == limit as usize {
        let last = rows
            .last()
            .ok_or_else(|| StoreError::ObservationUnavailable {
                message: "observation page limit was reached without a last row".to_owned(),
            })?;
        CursorPosition {
            append_xid: last.append_xid.clone(),
            commit_sort_key: last.commit_sort_key.clone(),
            kind: CursorKind::Row,
        }
    } else {
        CursorPosition {
            append_xid: frontier_xid,
            commit_sort_key: frontier_sort_key(),
            kind: CursorKind::Frontier,
        }
    };
    Ok(RunObservationPage {
        next_cursor: encode_observation_cursor(metadata, &next_position)?,
        runs: rows.into_iter().map(|row| row.observation).collect(),
    })
}

async fn load_store_metadata(pool: &PgPool) -> Result<StoreMetadata> {
    let row = sqlx::query(
        "SELECT store_epoch, cursor_key_id, cursor_secret FROM store_metadata WHERE singleton",
    )
    .fetch_one(pool)
    .await
    .map_err(|error| database_error("failed to load store metadata", error))?;
    Ok(StoreMetadata {
        store_epoch: row
            .try_get("store_epoch")
            .map_err(|error| database_error("failed to decode store epoch", error))?,
        cursor_key_id: row
            .try_get("cursor_key_id")
            .map_err(|error| database_error("failed to decode cursor key id", error))?,
        cursor_secret: row
            .try_get("cursor_secret")
            .map_err(|error| database_error("failed to decode cursor secret", error))?,
    })
}

async fn sealed_frontier_xid(pool: &PgPool) -> Result<String> {
    let row = sqlx::query("SELECT pg_snapshot_xmin(pg_current_snapshot())::text AS frontier_xid")
        .fetch_one(pool)
        .await
        .map_err(|error| database_error("failed to read observation frontier", error))?;
    row.try_get("frontier_xid")
        .map_err(|error| database_error("failed to decode observation frontier", error))
}

async fn read_observation_list_rows(
    pool: &PgPool,
    frontier_xid: &str,
    limit: u32,
) -> Result<Vec<ObservationRow>> {
    let rows = sqlx::query(
        "WITH bounded AS ( \
           SELECT DISTINCT ON (s.run_id) \
             s.run_id, s.head_seq, s.observed_status, \
             to_char(s.started_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS started_at, \
             to_char(s.updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at, \
             CASE WHEN s.completed_at IS NULL THEN NULL \
               ELSE to_char(s.completed_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
             END AS completed_at, \
             l.commit_id, l.append_xid::text AS append_xid, l.commit_sort_key \
           FROM run_observation_change_summaries s \
           INNER JOIN run_commit_log l ON l.commit_id = s.commit_id \
           WHERE l.append_xid < $1::xid8 \
             AND s.projection_version = $3 \
             AND s.summary_kind = $4 \
           ORDER BY s.run_id, s.head_seq DESC \
         ) \
         SELECT * FROM bounded ORDER BY updated_at DESC, run_id ASC LIMIT $2",
    )
    .bind(frontier_xid)
    .bind(i64::from(limit))
    .bind(PROJECTION_VERSION)
    .bind(RUN_OBSERVATION_SUMMARY_KIND)
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("failed to read run observation list", error))?;
    rows.into_iter().map(observation_row_from_row).collect()
}

async fn read_observation_watch_rows(
    pool: &PgPool,
    cursor: &CursorPosition,
    frontier_xid: &str,
    limit: u32,
) -> Result<Vec<ObservationRow>> {
    let rows = sqlx::query(
        "SELECT s.run_id, s.head_seq, s.observed_status, \
          to_char(s.started_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS started_at, \
          to_char(s.updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at, \
          CASE WHEN s.completed_at IS NULL THEN NULL \
            ELSE to_char(s.completed_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
          END AS completed_at, \
          l.commit_id, l.append_xid::text AS append_xid, l.commit_sort_key \
         FROM run_commit_log l \
         INNER JOIN run_observation_change_summaries s ON s.commit_id = l.commit_id \
         WHERE l.append_xid < $1::xid8 \
           AND s.projection_version = $6 \
           AND s.summary_kind = $7 \
           AND ( \
             ($4 AND l.append_xid >= $2::xid8) \
             OR \
             (NOT $4 AND (l.append_xid, l.commit_sort_key) > ($2::xid8, $3::bytea)) \
           ) \
         ORDER BY l.append_xid ASC, l.commit_sort_key ASC \
         LIMIT $5",
    )
    .bind(frontier_xid)
    .bind(&cursor.append_xid)
    .bind(cursor.commit_sort_key.as_slice())
    .bind(cursor.kind == CursorKind::Frontier)
    .bind(i64::from(limit))
    .bind(PROJECTION_VERSION)
    .bind(RUN_OBSERVATION_SUMMARY_KIND)
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("failed to read run observation changes", error))?;
    rows.into_iter().map(observation_row_from_row).collect()
}

fn observation_row_from_row(row: PgRow) -> Result<ObservationRow> {
    let run_id: String = row
        .try_get("run_id")
        .map_err(|error| database_error("failed to decode observation run id", error))?;
    let head_seq: i64 = row
        .try_get("head_seq")
        .map_err(|error| database_error("failed to decode observation head seq", error))?;
    let observed_status: String = row
        .try_get("observed_status")
        .map_err(|error| database_error("failed to decode observation status", error))?;
    let commit_id: String = row
        .try_get("commit_id")
        .map_err(|error| database_error("failed to decode observation commit id", error))?;
    Ok(ObservationRow {
        observation: RunObservation {
            run_id: parse_identity(&run_id)?,
            head_seq: StreamSeq::new(i64_to_positive_u64(
                head_seq,
                "run_observation_change_summaries.head_seq",
            )?)?,
            observed_status: ObservedRunStatus::parse(&observed_status)?,
            started_at: row.try_get("started_at").map_err(|error| {
                database_error("failed to decode observation started_at", error)
            })?,
            updated_at: row.try_get("updated_at").map_err(|error| {
                database_error("failed to decode observation updated_at", error)
            })?,
            completed_at: row.try_get("completed_at").map_err(|error| {
                database_error("failed to decode observation completed_at", error)
            })?,
            change_id: Some(commit_id),
        },
        append_xid: row
            .try_get("append_xid")
            .map_err(|error| database_error("failed to decode observation append xid", error))?,
        commit_sort_key: row.try_get("commit_sort_key").map_err(|error| {
            database_error("failed to decode observation commit sort key", error)
        })?,
    })
}

fn encode_observation_cursor(
    metadata: &StoreMetadata,
    position: &CursorPosition,
) -> Result<String> {
    let sort_key_hex = bytes_hex(&position.commit_sort_key);
    let mac = observation_cursor_mac(metadata, position.kind, &position.append_xid, &sort_key_hex)?;
    Ok(format!(
        "{CURSOR_VERSION}|{}|{}|{}|{}|{}|{}",
        metadata.cursor_key_id,
        position.kind.as_str(),
        metadata.store_epoch,
        position.append_xid,
        sort_key_hex,
        mac
    ))
}

fn decode_observation_cursor(cursor: &str, metadata: &StoreMetadata) -> Result<CursorPosition> {
    let parts = cursor.split('|').collect::<Vec<_>>();
    if parts.len() != 7 || parts[0] != CURSOR_VERSION {
        return Err(StoreError::InvalidCursor {
            message: "malformed cursor".to_owned(),
        }
        .into());
    }
    if parts[1] != metadata.cursor_key_id {
        return Err(StoreError::InvalidCursor {
            message: "unknown cursor key".to_owned(),
        }
        .into());
    }
    let kind = CursorKind::parse(parts[2])?;
    if parts[3] != metadata.store_epoch {
        return Err(StoreError::CursorExpired.into());
    }
    verify_observation_cursor_mac(metadata, kind, parts[4], parts[5], parts[6])?;
    Ok(CursorPosition {
        append_xid: parts[4].to_owned(),
        commit_sort_key: bytes_from_hex(parts[5])?,
        kind,
    })
}

fn observation_cursor_mac(
    metadata: &StoreMetadata,
    kind: CursorKind,
    append_xid: &str,
    sort_key_hex: &str,
) -> Result<String> {
    let payload = observation_cursor_mac_payload(metadata, kind, append_xid, sort_key_hex)?;
    let key = hmac::Key::new(hmac::HMAC_SHA256, &metadata.cursor_secret);
    Ok(bytes_hex(hmac::sign(&key, &payload).as_ref()))
}

fn verify_observation_cursor_mac(
    metadata: &StoreMetadata,
    kind: CursorKind,
    append_xid: &str,
    sort_key_hex: &str,
    mac_hex: &str,
) -> Result<()> {
    let payload = observation_cursor_mac_payload(metadata, kind, append_xid, sort_key_hex)?;
    let mac = bytes_from_hex(mac_hex)?;
    let key = hmac::Key::new(hmac::HMAC_SHA256, &metadata.cursor_secret);
    hmac::verify(&key, &payload, &mac).map_err(|_| {
        PostgresStoreError::Store(StoreError::InvalidCursor {
            message: "cursor authentication failed".to_owned(),
        })
    })
}

fn observation_cursor_mac_payload(
    metadata: &StoreMetadata,
    kind: CursorKind,
    append_xid: &str,
    sort_key_hex: &str,
) -> Result<Vec<u8>> {
    Ok(canonical_json(serde_json::json!({
        "append_xid": append_xid,
        "cursor_key_id": metadata.cursor_key_id.as_str(),
        "cursor_version": CURSOR_VERSION,
        "kind": kind.as_str(),
        "sort_key": sort_key_hex,
        "store_epoch": metadata.store_epoch.as_str(),
    }))?
    .as_bytes()
    .to_vec())
}

fn frontier_sort_key() -> Vec<u8> {
    vec![0; 32]
}

fn admit_artifact_evidence(
    artifacts: &mut ArtifactAuthorityMap,
    admitted_artifacts: &[ArtifactEvidenceRef],
) -> Result<()> {
    for evidence in admitted_artifacts {
        let key = (evidence.artifact_id.clone(), evidence.evidence_hash()?);
        if let Some(existing) = artifacts.get(&key) {
            if existing != evidence {
                return Err(StoreError::ArtifactEvidenceMismatch {
                    artifact_id: evidence.artifact_id.clone(),
                    field: "artifact",
                }
                .into());
            }
            continue;
        }
        artifacts.insert(key, evidence.clone());
    }
    Ok(())
}

async fn verify_prepared_artifact_bundle_tx(
    tx: &mut Transaction<'_, Postgres>,
    bundle: &PreparedCommitBundle,
) -> Result<()> {
    for artifact in bundle.artifact_bytes() {
        verify_prepared_artifact_bytes(artifact)?;
    }
    for existing in bundle.existing_artifacts() {
        let evidence =
            admitted_artifact_evidence(bundle, existing.artifact_id(), existing.evidence_hash())?;
        let record =
            load_artifact_record_tx(tx, existing.artifact_id(), existing.evidence_hash()).await?;
        verify_artifact_record(&record, evidence, None)?;
    }
    Ok(())
}

async fn admit_artifact_bundle_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    commit_key: &CommitKey,
    seq: StreamSeq,
    commit_id: &str,
    bundle: &PreparedCommitBundle,
) -> Result<()> {
    for artifact in bundle.artifact_bytes() {
        insert_prepared_artifact_bytes_tx(tx, artifact).await?;
    }
    for existing in bundle.existing_artifacts() {
        let evidence =
            admitted_artifact_evidence(bundle, existing.artifact_id(), existing.evidence_hash())?;
        let record =
            load_artifact_record_tx(tx, existing.artifact_id(), existing.evidence_hash()).await?;
        verify_artifact_record(&record, evidence, None)?;
    }
    for evidence in bundle.request().required_artifacts() {
        insert_commit_artifact_evidence_tx(
            tx, run_id, commit_key, seq, commit_id, evidence, "required",
        )
        .await?;
    }
    for evidence in bundle.admitted_artifacts() {
        link_run_artifact_tx(tx, run_id, commit_key, seq, commit_id, evidence).await?;
    }
    Ok(())
}

fn admitted_artifact_evidence<'a>(
    bundle: &'a PreparedCommitBundle,
    artifact_id: &ArtifactId,
    evidence_hash: &ContentDigest,
) -> Result<&'a ArtifactEvidenceRef> {
    for evidence in bundle.admitted_artifacts() {
        if &evidence.artifact_id == artifact_id && evidence.evidence_hash()? == *evidence_hash {
            return Ok(evidence);
        }
    }
    Err(StoreError::MissingPreparedArtifactBytes {
        artifact_id: artifact_id.clone(),
    }
    .into())
}

fn verify_prepared_artifact_bytes(artifact: &PreparedArtifactBytes) -> Result<()> {
    let verified =
        PreparedArtifactBytes::new(artifact.bytes().to_vec(), artifact.evidence().clone())?;
    if verified.evidence_hash() != artifact.evidence_hash() {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: artifact.evidence().artifact_id.clone(),
            field: "evidence_hash",
        }
        .into());
    }
    Ok(())
}

struct PreparedCommitAuthority {
    commit_idempotency_hash: String,
    idempotency_canonical_json: Vec<u8>,
    prepared_authority_hash: String,
    prepared_authority_canonical_json: Vec<u8>,
}

struct FinalCommitAuthority {
    commit_batch_hash: String,
    commit_batch_canonical_json: Vec<u8>,
}

fn prepared_commit_authority(
    plan: &mfm_store::v1::PreparedCommitPlan,
    fingerprint: &CommitFingerprint,
) -> Result<PreparedCommitAuthority> {
    let request = plan.request();
    let payloads = request
        .payloads()
        .iter()
        .map(payload_json_value)
        .collect::<Result<Vec<_>>>()?;
    let required_artifacts = request
        .required_artifacts()
        .iter()
        .map(artifact_evidence_authority_json)
        .collect::<Result<Vec<_>>>()?;
    let admitted_artifacts = plan
        .admitted_artifacts()
        .iter()
        .map(artifact_evidence_authority_json)
        .collect::<Result<Vec<_>>>()?;
    let stable_request = serde_json::json!({
        "admitted_artifacts": admitted_artifacts,
        "commit_key": request.commit_key().as_str(),
        "commit_purpose": plan.purpose_name(),
        "hash_domain_version": HASH_DOMAIN_VERSION,
        "payloads": payloads,
        "preconditions": preconditions_authority_json(request.preconditions())?,
        "prepared_plan_fingerprint": fingerprint.as_digest().as_str(),
        "required_artifacts": required_artifacts,
        "run_id": request.run_id().as_str(),
    });
    let idempotency = canonical_json(serde_json::json!({
        "domain": "mfm.commit.idempotency.v1",
        "request": stable_request,
    }))?;
    let prepared = canonical_json(serde_json::json!({
        "domain": "mfm.commit.prepared_authority.v1",
        "expected_next_seq": request.expected_next_seq().as_u64(),
        "request": stable_request,
        "store_fill_policy": "mfm.store_fill.resource_lanes.v1",
    }))?;
    Ok(PreparedCommitAuthority {
        commit_idempotency_hash: idempotency.content_digest().as_str().to_owned(),
        idempotency_canonical_json: idempotency.as_bytes().to_vec(),
        prepared_authority_hash: prepared.content_digest().as_str().to_owned(),
        prepared_authority_canonical_json: prepared.as_bytes().to_vec(),
    })
}

fn final_commit_authority(
    plan: &mfm_store::v1::PreparedCommitPlan,
    bundle: &PreparedCommitBundle,
    batch: &CommittedBatch,
    commit_id: &str,
) -> Result<FinalCommitAuthority> {
    let events = batch
        .events()
        .iter()
        .map(event_authority_json)
        .collect::<Result<Vec<_>>>()?;
    let required = plan
        .request()
        .required_artifacts()
        .iter()
        .map(|evidence| artifact_binding_authority_json("required", evidence))
        .collect::<Result<Vec<_>>>()?;
    let admitted = bundle
        .admitted_artifacts()
        .iter()
        .map(|evidence| artifact_binding_authority_json("admitted", evidence))
        .collect::<Result<Vec<_>>>()?;
    let canonical = canonical_json(serde_json::json!({
        "admitted_artifacts": admitted,
        "commit_id": commit_id,
        "commit_key": batch.commit_key().as_str(),
        "domain": "mfm.commit.batch.v1",
        "events": events,
        "hash_domain_version": HASH_DOMAIN_VERSION,
        "required_artifacts": required,
        "run_id": batch.run_id().as_str(),
        "seq": batch.seq().as_u64(),
    }))?;
    Ok(FinalCommitAuthority {
        commit_batch_hash: canonical.content_digest().as_str().to_owned(),
        commit_batch_canonical_json: canonical.as_bytes().to_vec(),
    })
}

fn event_authority_json(event: &KernelEventEnvelope) -> Result<Value> {
    Ok(serde_json::json!({
        "commit_key": event.commit_key().as_str(),
        "event_id": event.event_id().as_str(),
        "event_schema_id": event.event_schema_id().as_str(),
        "logical_key": event.logical_key().as_str(),
        "ordinal": event.ordinal().as_u32(),
        "payload": payload_json_value(event.payload())?,
        "payload_hash": event.payload_hash().as_str(),
        "run_id": event.run_id().as_str(),
        "seq": event.seq().as_u64(),
        "spec_hash": event.spec_hash().as_str(),
    }))
}

fn artifact_binding_authority_json(
    binding_kind: &str,
    evidence: &ArtifactEvidenceRef,
) -> Result<Value> {
    let mut value = artifact_evidence_authority_json(evidence)?;
    if let Value::Object(ref mut object) = value {
        object.insert(
            "binding_kind".to_owned(),
            Value::String(binding_kind.to_owned()),
        );
    }
    Ok(value)
}

fn artifact_evidence_authority_json(evidence: &ArtifactEvidenceRef) -> Result<Value> {
    Ok(serde_json::json!({
        "artifact_id": evidence.artifact_id.as_str(),
        "artifact_role": evidence.artifact_role.as_str(),
        "byte_len": evidence.byte_len,
        "digest": evidence.digest.as_str(),
        "evidence_hash": evidence.evidence_hash()?.as_str(),
        "media_type": evidence.media_type.as_str(),
        "producer_node_id": evidence.producer_node_id.as_ref().map(NodeId::as_str),
        "producer_seed_id": evidence.producer_seed_id.as_ref().map(SeedId::as_str),
        "schema_id": evidence.schema_id.as_ref().map(SchemaId::as_str),
        "semantic_type_id": evidence.semantic_type_id.as_ref().map(SemanticTypeId::as_str),
    }))
}

fn artifact_evidence_canonical_json(evidence: &ArtifactEvidenceRef) -> Result<Vec<u8>> {
    Ok(canonical_json(serde_json::json!({
        "domain": "mfm.artifact.evidence.v1",
        "evidence": artifact_evidence_authority_json(evidence)?,
    }))?
    .as_bytes()
    .to_vec())
}

fn preconditions_authority_json(
    preconditions: &mfm_store::v1::CommitPreconditions,
) -> Result<Value> {
    Ok(serde_json::json!({
        "required_absent_logical_keys": preconditions.required_absent_logical_keys.iter().map(LogicalEventKey::as_str).collect::<Vec<_>>(),
        "required_cell_states": preconditions.required_cell_states.iter().map(cell_precondition_json).collect::<Vec<_>>(),
        "required_present_logical_keys": preconditions.required_present_logical_keys.iter().map(LogicalEventKey::as_str).collect::<Vec<_>>(),
        "required_public_output_absent": preconditions.required_public_output_absent,
        "required_run_state": required_run_state_tag(preconditions.required_run_state),
        "required_side_effect_states": preconditions.required_side_effect_states.iter().map(side_effect_precondition_json).collect::<Vec<_>>(),
        "saga_admit_token": preconditions.saga_admit_token.as_ref().map(saga_admit_token_json),
    }))
}

fn cell_precondition_json(precondition: &mfm_store::v1::CellStatePrecondition) -> Value {
    serde_json::json!({
        "cell_id": precondition.cell_id.as_str(),
        "required": required_cell_state_tag(precondition.required),
    })
}

fn side_effect_precondition_json(
    precondition: &mfm_store::v1::SideEffectStatePrecondition,
) -> Value {
    serde_json::json!({
        "ledger_key": precondition.ledger_key.as_str(),
        "required": required_side_effect_state_tag(precondition.required),
    })
}

fn saga_admit_token_json(token: &mfm_store::v1::SagaAdmitToken) -> Value {
    serde_json::json!({
        "run_id": token.run_id().as_str(),
        "saga_policy_digest": token.saga_policy_digest().as_str(),
        "spec_hash": token.spec_hash().as_str(),
    })
}

fn required_run_state_tag(value: mfm_store::v1::RequiredRunState) -> &'static str {
    match value {
        mfm_store::v1::RequiredRunState::Any => "any",
        mfm_store::v1::RequiredRunState::Absent => "absent",
        mfm_store::v1::RequiredRunState::Started => "started",
        mfm_store::v1::RequiredRunState::NotCompleted => "not_completed",
        mfm_store::v1::RequiredRunState::Completed => "completed",
    }
}

fn required_cell_state_tag(value: mfm_store::v1::RequiredCellState) -> &'static str {
    match value {
        mfm_store::v1::RequiredCellState::Absent => "absent",
        mfm_store::v1::RequiredCellState::Produced => "produced",
        mfm_store::v1::RequiredCellState::Skipped => "skipped",
        mfm_store::v1::RequiredCellState::Terminal => "terminal",
    }
}

fn required_side_effect_state_tag(value: mfm_store::v1::RequiredSideEffectState) -> &'static str {
    match value {
        mfm_store::v1::RequiredSideEffectState::Absent => "absent",
        mfm_store::v1::RequiredSideEffectState::IntentPersisted => "intent_persisted",
        mfm_store::v1::RequiredSideEffectState::Claimed => "claimed",
        mfm_store::v1::RequiredSideEffectState::InvocationPrepared => "invocation_prepared",
        mfm_store::v1::RequiredSideEffectState::InvocationStarted => "invocation_started",
        mfm_store::v1::RequiredSideEffectState::SubmissionResult => "submission_result",
        mfm_store::v1::RequiredSideEffectState::ReceiptObserved => "receipt_observed",
        mfm_store::v1::RequiredSideEffectState::ConfirmationObserved => "confirmation_observed",
        mfm_store::v1::RequiredSideEffectState::Ambiguous => "ambiguous",
        mfm_store::v1::RequiredSideEffectState::Failed => "failed",
    }
}

async fn insert_prepared_artifact_bytes_tx(
    tx: &mut Transaction<'_, Postgres>,
    artifact: &PreparedArtifactBytes,
) -> Result<()> {
    verify_prepared_artifact_bytes(artifact)?;
    let evidence = artifact.evidence();
    let evidence_hash = artifact.evidence_hash();
    let evidence_canonical_json = artifact_evidence_canonical_json(evidence)?;
    let byte_len = u64_to_i64(evidence.byte_len, "artifact_blobs.byte_len")?;
    sqlx::query(
        "INSERT INTO artifact_blobs (artifact_id, digest, byte_len, bytes) \
         VALUES ($1,$2,$3,$4) \
         ON CONFLICT (artifact_id) DO NOTHING",
    )
    .bind(evidence.artifact_id.as_str())
    .bind(evidence.digest.as_str())
    .bind(byte_len)
    .bind(artifact.bytes())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert artifact blob", error))?;
    sqlx::query(
        "INSERT INTO artifact_admissions \
         (evidence_hash, artifact_id, digest, byte_len, evidence_schema_version, media_type, \
          schema_id, semantic_type_id, producer_node_id, producer_seed_id, artifact_role, \
          evidence_canonical_json) \
         VALUES ($1,$2,$3,$4,'mfm.artifact.evidence.v1',$5,$6,$7,$8,$9,$10,$11) \
         ON CONFLICT (evidence_hash) DO NOTHING",
    )
    .bind(evidence_hash.as_str())
    .bind(evidence.artifact_id.as_str())
    .bind(evidence.digest.as_str())
    .bind(byte_len)
    .bind(evidence.media_type.as_str())
    .bind(evidence.schema_id.as_ref().map(SchemaId::as_str))
    .bind(
        evidence
            .semantic_type_id
            .as_ref()
            .map(SemanticTypeId::as_str),
    )
    .bind(evidence.producer_node_id.as_ref().map(NodeId::as_str))
    .bind(evidence.producer_seed_id.as_ref().map(SeedId::as_str))
    .bind(evidence.artifact_role.as_str())
    .bind(evidence_canonical_json.as_slice())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert artifact admission", error))?
    .rows_affected();
    let record = load_artifact_record_tx(tx, &evidence.artifact_id, evidence_hash).await?;
    verify_artifact_record(&record, evidence, Some(artifact.bytes()))?;
    Ok(())
}

async fn link_run_artifact_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    commit_key: &CommitKey,
    seq: StreamSeq,
    commit_id: &str,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    insert_commit_artifact_evidence_tx(
        tx, run_id, commit_key, seq, commit_id, evidence, "admitted",
    )
    .await?;
    let evidence_hash = evidence.evidence_hash()?;
    sqlx::query(
        "INSERT INTO run_artifact_admissions \
         (run_id, artifact_id, evidence_hash, first_commit_id, first_commit_key, first_seq, \
          first_binding_kind) \
         VALUES ($1,$2,$3,$4,$5,$6,'admitted') \
         ON CONFLICT (run_id, artifact_id, evidence_hash) DO NOTHING",
    )
    .bind(run_id.as_str())
    .bind(evidence.artifact_id.as_str())
    .bind(evidence_hash.as_str())
    .bind(commit_id)
    .bind(commit_key.as_str())
    .bind(u64_to_i64(
        seq.as_u64(),
        "run_artifact_admissions.first_seq",
    )?)
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert run artifact evidence", error))?;
    Ok(())
}

async fn insert_commit_artifact_evidence_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    commit_key: &CommitKey,
    seq: StreamSeq,
    commit_id: &str,
    evidence: &ArtifactEvidenceRef,
    binding_kind: &str,
) -> Result<()> {
    let evidence_hash = evidence.evidence_hash()?;
    sqlx::query(
        "INSERT INTO commit_artifact_evidence \
         (run_id, seq, commit_key, commit_id, artifact_id, evidence_hash, binding_kind) \
         VALUES ($1,$2,$3,$4,$5,$6,$7) \
         ON CONFLICT (run_id, seq, binding_kind, artifact_id, evidence_hash) DO NOTHING",
    )
    .bind(run_id.as_str())
    .bind(u64_to_i64(seq.as_u64(), "commit_artifact_evidence.seq")?)
    .bind(commit_key.as_str())
    .bind(commit_id)
    .bind(evidence.artifact_id.as_str())
    .bind(evidence_hash.as_str())
    .bind(binding_kind)
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert commit artifact evidence", error))?;
    Ok(())
}

async fn insert_run_commit_log_tx(
    tx: &mut Transaction<'_, Postgres>,
    request: &mfm_store::v1::CommitRequest,
    commit_id: &str,
    commit_batch_hash: &str,
    seq: StreamSeq,
    event_count: i32,
) -> Result<()> {
    let last_ordinal = event_count.checked_sub(1).ok_or_else(|| {
        PostgresStoreError::Corruption("run commit log event count underflow".to_owned())
    })?;
    let commit_sort_key = run_commit_sort_key(
        request.run_id(),
        seq,
        request.commit_key(),
        commit_id,
        commit_batch_hash,
    )?;
    sqlx::query(
        "INSERT INTO run_commit_log \
         (commit_id, run_id, seq, commit_key, first_ordinal, last_ordinal, event_count, \
          commit_sort_key) \
         VALUES ($1,$2,$3,$4,0,$5,$6,$7)",
    )
    .bind(commit_id)
    .bind(request.run_id().as_str())
    .bind(u64_to_i64(seq.as_u64(), "run_commit_log.seq")?)
    .bind(request.commit_key().as_str())
    .bind(last_ordinal)
    .bind(event_count)
    .bind(commit_sort_key)
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert run commit log", error))?;
    Ok(())
}

async fn insert_run_observation_summary_tx(
    tx: &mut Transaction<'_, Postgres>,
    projections: &ProjectionSnapshot,
    run_id: &RunId,
    commit_id: &str,
    head_seq: StreamSeq,
    source_event_count: i64,
) -> Result<()> {
    let observed_status = observed_status_from_run_state(projections.run_state(run_id))?;
    let summary = materialize_run_observation_summary_tx(
        tx,
        PROJECTION_VERSION,
        run_id,
        commit_id,
        head_seq,
        observed_status,
        source_event_count,
    )
    .await?;
    sqlx::query(
        "INSERT INTO run_observation_change_summaries \
         (projection_version, commit_id, summary_kind, run_id, head_seq, observed_status, \
          started_at, updated_at, completed_at, source_authority_hash, source_event_count, \
          summary_row_hash, summary_row_canonical_json) \
         SELECT $1, $2, $3, $4, $5, $6, \
           (SELECT MIN(committed_at) FROM commits WHERE run_id = $4), \
           c.committed_at, \
           CASE WHEN $6 = 'completed' THEN c.committed_at ELSE NULL END, \
           $7, $8, $9, $10 \
         FROM commits c WHERE c.commit_id = $2",
    )
    .bind(summary.projection_version.as_str())
    .bind(summary.commit_id.as_str())
    .bind(summary.summary_kind)
    .bind(summary.run_id.as_str())
    .bind(u64_to_i64(
        summary.head_seq.as_u64(),
        "run_observation_change_summaries.head_seq",
    )?)
    .bind(summary.observed_status.as_str())
    .bind(summary.source_authority_hash.as_str())
    .bind(summary.source_event_count)
    .bind(summary.summary_row_hash.as_str())
    .bind(summary.summary_row_canonical_json.as_bytes())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert run observation summary", error))?;
    Ok(())
}

struct RunObservationSummaryMaterialization {
    projection_version: String,
    commit_id: String,
    summary_kind: &'static str,
    run_id: RunId,
    head_seq: StreamSeq,
    observed_status: ObservedRunStatus,
    started_at: String,
    updated_at: String,
    completed_at: Option<String>,
    source_authority_hash: String,
    source_event_count: i64,
    summary_row_hash: String,
    summary_row_canonical_json: PlainCanonicalJsonBytes,
}

async fn materialize_run_observation_summary_tx(
    tx: &mut Transaction<'_, Postgres>,
    projection_version: &str,
    run_id: &RunId,
    commit_id: &str,
    head_seq: StreamSeq,
    observed_status: ObservedRunStatus,
    source_event_count: i64,
) -> Result<RunObservationSummaryMaterialization> {
    if source_event_count < 1 {
        return Err(PostgresStoreError::Corruption(
            "observation source event count must be positive".to_owned(),
        ));
    }
    let row = sqlx::query(
        "SELECT \
           to_char((SELECT MIN(committed_at) FROM commits WHERE run_id = $1) AT TIME ZONE 'UTC', \
             'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS started_at, \
           to_char(c.committed_at AT TIME ZONE 'UTC', \
             'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at, \
           CASE WHEN $2 = 'completed' THEN \
             to_char(c.committed_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
           ELSE NULL END AS completed_at \
         FROM commits c WHERE c.commit_id = $3",
    )
    .bind(run_id.as_str())
    .bind(observed_status.as_str())
    .bind(commit_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| database_error("failed to materialize observation timestamps", error))?;
    let started_at: String = row
        .try_get("started_at")
        .map_err(|error| database_error("failed to decode observation started_at", error))?;
    let updated_at: String = row
        .try_get("updated_at")
        .map_err(|error| database_error("failed to decode observation updated_at", error))?;
    let completed_at: Option<String> = row
        .try_get("completed_at")
        .map_err(|error| database_error("failed to decode observation completed_at", error))?;
    let source_authority_hash = observation_source_authority_hash(
        run_id,
        commit_id,
        head_seq,
        observed_status,
        source_event_count,
    )?;
    let summary_row_canonical_json = run_observation_summary_canonical_json(
        projection_version,
        run_id,
        commit_id,
        head_seq,
        observed_status,
        &started_at,
        &updated_at,
        completed_at.as_deref(),
        &source_authority_hash,
        source_event_count,
    )?;
    let summary_row_hash = summary_row_canonical_json
        .content_digest()
        .as_str()
        .to_owned();
    Ok(RunObservationSummaryMaterialization {
        projection_version: projection_version.to_owned(),
        commit_id: commit_id.to_owned(),
        summary_kind: RUN_OBSERVATION_SUMMARY_KIND,
        run_id: run_id.clone(),
        head_seq,
        observed_status,
        started_at,
        updated_at,
        completed_at,
        source_authority_hash,
        source_event_count,
        summary_row_hash,
        summary_row_canonical_json,
    })
}

fn observed_status_from_run_state(run_state: RunState) -> Result<ObservedRunStatus> {
    match run_state {
        RunState::Started => Ok(ObservedRunStatus::Started),
        RunState::Completed => Ok(ObservedRunStatus::Completed),
        RunState::Absent => Err(StoreError::ObservationUnavailable {
            message: "committed run observation had absent run state".to_owned(),
        }
        .into()),
    }
}

fn observation_source_authority_hash(
    run_id: &RunId,
    commit_id: &str,
    head_seq: StreamSeq,
    observed_status: ObservedRunStatus,
    source_event_count: i64,
) -> Result<String> {
    Ok(canonical_json(serde_json::json!({
        "commit_id": commit_id,
        "domain": "mfm.run_observation.summary.source.v1",
        "head_seq": head_seq.as_u64(),
        "observed_status": observed_status.as_str(),
        "run_id": run_id.as_str(),
        "source_event_count": source_event_count,
    }))?
    .content_digest()
    .as_str()
    .to_owned())
}

fn run_observation_summary_canonical_json(
    projection_version: &str,
    run_id: &RunId,
    commit_id: &str,
    head_seq: StreamSeq,
    observed_status: ObservedRunStatus,
    started_at: &str,
    updated_at: &str,
    completed_at: Option<&str>,
    source_authority_hash: &str,
    source_event_count: i64,
) -> Result<PlainCanonicalJsonBytes> {
    canonical_json(serde_json::json!({
        "commit_id": commit_id,
        "completed_at": completed_at,
        "domain": "mfm.run_observation.summary.row.v1",
        "head_seq": head_seq.as_u64(),
        "observed_status": observed_status.as_str(),
        "projection_version": projection_version,
        "run_id": run_id.as_str(),
        "source_authority_hash": source_authority_hash,
        "source_event_count": source_event_count,
        "started_at": started_at,
        "summary_kind": RUN_OBSERVATION_SUMMARY_KIND,
        "updated_at": updated_at,
    }))
    .map_err(PostgresStoreError::from)
}

async fn build_projection_version_from_pool(
    pool: &PgPool,
    projection_version: &str,
    mode: ReadModelBuildMode,
) -> Result<ReadModelBuildReport> {
    validate_projection_version(projection_version)?;
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| database_error("failed to start read-model build transaction", error))?;
    let summaries =
        materialize_all_run_observation_summaries_tx(&mut tx, projection_version).await?;
    let mut report = ReadModelBuildReport {
        projection_version: projection_version.to_owned(),
        inserted_rows: 0,
        verified_rows: 0,
    };
    for summary in summaries {
        match compare_existing_run_observation_summary_tx(&mut tx, &summary).await? {
            SummaryComparison::Match => {
                report.verified_rows = report.verified_rows.checked_add(1).ok_or_else(|| {
                    PostgresStoreError::Corruption(
                        "read-model verified row count overflow".to_owned(),
                    )
                })?;
            }
            SummaryComparison::Missing => {
                if matches!(
                    mode,
                    ReadModelBuildMode::MissingOnly | ReadModelBuildMode::NewVersion
                ) {
                    if insert_materialized_run_observation_summary_tx(&mut tx, &summary).await? {
                        report.inserted_rows =
                            report.inserted_rows.checked_add(1).ok_or_else(|| {
                                PostgresStoreError::Corruption(
                                    "read-model inserted row count overflow".to_owned(),
                                )
                            })?;
                    }
                }
            }
            SummaryComparison::Mismatch => {
                return Err(PostgresStoreError::Corruption(format!(
                    "read-model drift for projection {} commit {}",
                    summary.projection_version, summary.commit_id
                )));
            }
        }
    }
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit read-model build", error))?;
    Ok(report)
}

async fn validate_read_models_from_pool(
    pool: &PgPool,
    projection_version: &str,
) -> Result<ReadModelValidationReport> {
    validate_projection_version(projection_version)?;
    let mut tx = pool.begin().await.map_err(|error| {
        database_error("failed to start read-model validation transaction", error)
    })?;
    let summaries =
        materialize_all_run_observation_summaries_tx(&mut tx, projection_version).await?;
    let mut report = ReadModelValidationReport {
        projection_version: projection_version.to_owned(),
        checked_commits: 0,
        missing_rows: 0,
        drift_rows: 0,
    };
    for summary in summaries {
        report.checked_commits = report.checked_commits.checked_add(1).ok_or_else(|| {
            PostgresStoreError::Corruption("read-model checked row count overflow".to_owned())
        })?;
        match compare_existing_run_observation_summary_tx(&mut tx, &summary).await? {
            SummaryComparison::Match => {}
            SummaryComparison::Missing => {
                report.missing_rows = report.missing_rows.checked_add(1).ok_or_else(|| {
                    PostgresStoreError::Corruption(
                        "read-model missing row count overflow".to_owned(),
                    )
                })?;
            }
            SummaryComparison::Mismatch => {
                report.drift_rows = report.drift_rows.checked_add(1).ok_or_else(|| {
                    PostgresStoreError::Corruption("read-model drift row count overflow".to_owned())
                })?;
            }
        }
    }
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit read-model validation", error))?;
    Ok(report)
}

async fn read_model_high_watermark_from_pool(pool: &PgPool) -> Result<ReadModelHighWatermark> {
    let commit_log_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM run_commit_log")
        .fetch_one(pool)
        .await
        .map_err(|error| database_error("failed to count commit-log rows", error))?;
    let summary_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM run_observation_change_summaries")
            .fetch_one(pool)
            .await
            .map_err(|error| database_error("failed to count observation summary rows", error))?;
    let high = sqlx::query(
        "SELECT append_xid::text AS append_xid, commit_id, commit_sort_key \
         FROM run_commit_log ORDER BY append_xid DESC, commit_sort_key DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|error| database_error("failed to read read-model high watermark", error))?;
    let (high_append_xid, high_commit_id, high_commit_sort_key) = if let Some(row) = high {
        (
            Some(row.try_get("append_xid").map_err(|error| {
                database_error("failed to decode high-watermark append xid", error)
            })?),
            Some(row.try_get("commit_id").map_err(|error| {
                database_error("failed to decode high-watermark commit id", error)
            })?),
            Some(row.try_get("commit_sort_key").map_err(|error| {
                database_error("failed to decode high-watermark sort key", error)
            })?),
        )
    } else {
        (None, None, None)
    };
    Ok(ReadModelHighWatermark {
        commit_log_rows: i64_to_nonnegative_u64(commit_log_rows, "run_commit_log.count")?,
        summary_rows: i64_to_nonnegative_u64(
            summary_rows,
            "run_observation_change_summaries.count",
        )?,
        high_append_xid,
        high_commit_id,
        high_commit_sort_key,
    })
}

async fn read_model_drift_report_from_pool(pool: &PgPool) -> Result<ReadModelDriftReport> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| database_error("failed to start read-model drift transaction", error))?;
    let version_rows =
        sqlx::query("SELECT DISTINCT projection_version FROM run_observation_change_summaries")
            .fetch_all(&mut *tx)
            .await
            .map_err(|error| database_error("failed to load read-model versions", error))?;
    let mut versions = BTreeSet::from([PROJECTION_VERSION.to_owned()]);
    for row in version_rows {
        versions.insert(row.try_get("projection_version").map_err(|error| {
            database_error("failed to decode read-model projection version", error)
        })?);
    }

    let mut findings = Vec::new();
    for projection_version in versions {
        let summaries =
            materialize_all_run_observation_summaries_tx(&mut tx, &projection_version).await?;
        for summary in summaries {
            match compare_existing_run_observation_summary_tx(&mut tx, &summary).await? {
                SummaryComparison::Match => {}
                SummaryComparison::Missing => findings.push(ReadModelDrift {
                    projection_version: summary.projection_version,
                    commit_id: summary.commit_id,
                    summary_kind: summary.summary_kind.to_owned(),
                    kind: ReadModelDriftKind::Missing,
                }),
                SummaryComparison::Mismatch => findings.push(ReadModelDrift {
                    projection_version: summary.projection_version,
                    commit_id: summary.commit_id,
                    summary_kind: summary.summary_kind.to_owned(),
                    kind: ReadModelDriftKind::Mismatched,
                }),
            }
        }
    }
    let orphan_rows = sqlx::query(
        "SELECT s.projection_version, s.commit_id, s.summary_kind \
         FROM run_observation_change_summaries s \
         LEFT JOIN run_commit_log l ON l.commit_id = s.commit_id \
         WHERE l.commit_id IS NULL",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| database_error("failed to load orphaned read-model rows", error))?;
    for row in orphan_rows {
        findings.push(ReadModelDrift {
            projection_version: row.try_get("projection_version").map_err(|error| {
                database_error("failed to decode orphan projection version", error)
            })?,
            commit_id: row
                .try_get("commit_id")
                .map_err(|error| database_error("failed to decode orphan commit id", error))?,
            summary_kind: row
                .try_get("summary_kind")
                .map_err(|error| database_error("failed to decode orphan summary kind", error))?,
            kind: ReadModelDriftKind::Orphaned,
        });
    }
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit read-model drift transaction", error))?;
    Ok(ReadModelDriftReport { findings })
}

async fn reseed_store_epoch_from_pool(pool: &PgPool) -> Result<String> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| database_error("failed to start store epoch reseed", error))?;
    sqlx::query("ALTER TABLE store_metadata DISABLE TRIGGER store_metadata_no_update")
        .execute(&mut *tx)
        .await
        .map_err(|error| database_error("failed to enter store epoch maintenance", error))?;
    let row = sqlx::query(
        "UPDATE store_metadata \
         SET store_epoch = 'mfm.store.epoch.v1:' || encode(public.gen_random_bytes(16), 'hex'), \
             cursor_secret = public.gen_random_bytes(32) \
         WHERE singleton \
         RETURNING store_epoch",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| database_error("failed to reseed store epoch", error))?;
    sqlx::query("ALTER TABLE store_metadata ENABLE TRIGGER store_metadata_no_update")
        .execute(&mut *tx)
        .await
        .map_err(|error| database_error("failed to leave store epoch maintenance", error))?;
    let store_epoch = row
        .try_get("store_epoch")
        .map_err(|error| database_error("failed to decode reseeded store epoch", error))?;
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit store epoch reseed", error))?;
    Ok(store_epoch)
}

fn validate_projection_version(projection_version: &str) -> Result<()> {
    if projection_version.trim().is_empty() {
        return Err(StoreError::ObservationUnavailable {
            message: "projection version must not be empty".to_owned(),
        }
        .into());
    }
    Ok(())
}

enum SummaryComparison {
    Missing,
    Match,
    Mismatch,
}

async fn materialize_all_run_observation_summaries_tx(
    tx: &mut Transaction<'_, Postgres>,
    projection_version: &str,
) -> Result<Vec<RunObservationSummaryMaterialization>> {
    let rows = sqlx::query("SELECT DISTINCT run_id FROM commits ORDER BY run_id ASC")
        .fetch_all(&mut **tx)
        .await
        .map_err(|error| database_error("failed to load read-model run ids", error))?;
    let mut summaries = Vec::new();
    for row in rows {
        let run_id = parse_identity::<RunId>(
            &row.try_get::<String, _>("run_id")
                .map_err(|error| database_error("failed to decode read-model run id", error))?,
        )?;
        let commits = load_commit_authority_rows_tx(tx, &run_id).await?;
        let stream = load_run_stream_tx(tx, &run_id).await?;
        let mut prefix = Vec::new();
        let mut cursor = 0_usize;
        for commit in commits {
            while cursor < stream.len() && stream[cursor].seq().as_u64() <= commit.seq.as_u64() {
                prefix.push(stream[cursor].clone());
                cursor += 1;
            }
            if prefix.last().map(KernelEventEnvelope::seq) != Some(commit.seq) {
                return Err(PostgresStoreError::Corruption(
                    "commit has no authority event in rebuilt observation prefix".to_owned(),
                ));
            }
            let snapshot = ProjectionSnapshot::rebuild_from_run_stream(&prefix)?;
            let observed_status = observed_status_from_run_state(snapshot.run_state(&run_id))?;
            let source_event_count = i64::try_from(prefix.len()).map_err(|_| {
                PostgresStoreError::Corruption(
                    "read-model source event count exceeded PostgreSQL bigint range".to_owned(),
                )
            })?;
            summaries.push(
                materialize_run_observation_summary_tx(
                    tx,
                    projection_version,
                    &run_id,
                    &commit.commit_id,
                    commit.seq,
                    observed_status,
                    source_event_count,
                )
                .await?,
            );
        }
    }
    Ok(summaries)
}

async fn compare_existing_run_observation_summary_tx(
    tx: &mut Transaction<'_, Postgres>,
    summary: &RunObservationSummaryMaterialization,
) -> Result<SummaryComparison> {
    let row = sqlx::query(
        "SELECT run_id, head_seq, observed_status, \
          to_char(started_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS started_at, \
          to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS updated_at, \
          CASE WHEN completed_at IS NULL THEN NULL \
            ELSE to_char(completed_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
          END AS completed_at, \
          source_authority_hash, source_event_count, summary_row_hash, summary_row_canonical_json \
         FROM run_observation_change_summaries \
         WHERE projection_version = $1 AND commit_id = $2 AND summary_kind = $3",
    )
    .bind(summary.projection_version.as_str())
    .bind(summary.commit_id.as_str())
    .bind(summary.summary_kind)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load existing observation summary", error))?;
    let Some(row) = row else {
        return Ok(SummaryComparison::Missing);
    };
    let run_id_text: String = row
        .try_get("run_id")
        .map_err(|error| database_error("failed to decode summary run id", error))?;
    let run_id = parse_identity::<RunId>(&run_id_text)?;
    let head_seq = StreamSeq::new(i64_to_positive_u64(
        row.try_get("head_seq")
            .map_err(|error| database_error("failed to decode summary head seq", error))?,
        "run_observation_change_summaries.head_seq",
    )?)?;
    let observed_status_text: String = row
        .try_get("observed_status")
        .map_err(|error| database_error("failed to decode summary observed status", error))?;
    let source_event_count: i64 = row
        .try_get("source_event_count")
        .map_err(|error| database_error("failed to decode summary source count", error))?;
    let summary_row_hash: String = row
        .try_get("summary_row_hash")
        .map_err(|error| database_error("failed to decode summary row hash", error))?;
    let summary_row_canonical_json: Vec<u8> = row
        .try_get("summary_row_canonical_json")
        .map_err(|error| database_error("failed to decode summary row canonical bytes", error))?;
    let stored_canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(
        &summary_row_canonical_json,
    )
    .map_err(|error| {
        PostgresStoreError::Corruption(format!(
            "read-model summary canonical bytes are invalid: {error}"
        ))
    })?;
    if stored_canonical.content_digest().as_str() != summary_row_hash {
        return Ok(SummaryComparison::Mismatch);
    }
    let matches = run_id == summary.run_id
        && head_seq == summary.head_seq
        && observed_status_text == summary.observed_status.as_str()
        && row
            .try_get::<String, _>("started_at")
            .map_err(|error| database_error("failed to decode summary started_at", error))?
            == summary.started_at
        && row
            .try_get::<String, _>("updated_at")
            .map_err(|error| database_error("failed to decode summary updated_at", error))?
            == summary.updated_at
        && row
            .try_get::<Option<String>, _>("completed_at")
            .map_err(|error| database_error("failed to decode summary completed_at", error))?
            == summary.completed_at
        && row
            .try_get::<String, _>("source_authority_hash")
            .map_err(|error| database_error("failed to decode summary source hash", error))?
            == summary.source_authority_hash
        && source_event_count == summary.source_event_count
        && summary_row_hash == summary.summary_row_hash
        && summary_row_canonical_json == summary.summary_row_canonical_json.as_bytes();
    Ok(if matches {
        SummaryComparison::Match
    } else {
        SummaryComparison::Mismatch
    })
}

async fn insert_materialized_run_observation_summary_tx(
    tx: &mut Transaction<'_, Postgres>,
    summary: &RunObservationSummaryMaterialization,
) -> Result<bool> {
    let rows = sqlx::query(
        "INSERT INTO run_observation_change_summaries \
         (projection_version, commit_id, summary_kind, run_id, head_seq, observed_status, \
          started_at, updated_at, completed_at, source_authority_hash, source_event_count, \
          summary_row_hash, summary_row_canonical_json) \
         SELECT $1, $2, $3, $4, $5, $6, \
           (SELECT MIN(committed_at) FROM commits WHERE run_id = $4), \
           c.committed_at, \
           CASE WHEN $6 = 'completed' THEN c.committed_at ELSE NULL END, \
           $7, $8, $9, $10 \
         FROM commits c WHERE c.commit_id = $2 \
         ON CONFLICT (projection_version, commit_id, summary_kind) DO NOTHING",
    )
    .bind(summary.projection_version.as_str())
    .bind(summary.commit_id.as_str())
    .bind(summary.summary_kind)
    .bind(summary.run_id.as_str())
    .bind(u64_to_i64(
        summary.head_seq.as_u64(),
        "run_observation_change_summaries.head_seq",
    )?)
    .bind(summary.observed_status.as_str())
    .bind(summary.source_authority_hash.as_str())
    .bind(summary.source_event_count)
    .bind(summary.summary_row_hash.as_str())
    .bind(summary.summary_row_canonical_json.as_bytes())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert rebuilt observation summary", error))?
    .rows_affected();
    Ok(rows == 1)
}

fn run_commit_sort_key(
    run_id: &RunId,
    seq: StreamSeq,
    commit_key: &CommitKey,
    commit_id: &str,
    commit_batch_hash: &str,
) -> Result<Vec<u8>> {
    let digest = canonical_json(serde_json::json!({
        "commit_batch_hash": commit_batch_hash,
        "commit_key": commit_key.as_str(),
        "commit_id": commit_id,
        "domain": "mfm.run_commit_log.sort_key.v1",
        "run_id": run_id.as_str(),
        "seq": seq.as_u64(),
    }))?
    .digest_bytes();
    let mut key = Vec::with_capacity(32);
    key.push(1);
    key.extend_from_slice(&digest.as_bytes()[..31]);
    Ok(key)
}

async fn read_head(pool: &PgPool, run_id: &RunId) -> Result<u64> {
    let row =
        sqlx::query("SELECT COALESCE(MAX(seq), 0) AS head_seq FROM commits WHERE run_id = $1")
            .bind(run_id.as_str())
            .fetch_one(pool)
            .await
            .map_err(|error| database_error("failed to query run head", error))?;
    let head_seq: i64 = row
        .try_get("head_seq")
        .map_err(|error| database_error("failed to decode run head", error))?;
    i64_to_nonnegative_u64(head_seq, "commits.seq")
}

async fn read_head_tx(tx: &mut Transaction<'_, Postgres>, run_id: &RunId) -> Result<u64> {
    let row =
        sqlx::query("SELECT COALESCE(MAX(seq), 0) AS head_seq FROM commits WHERE run_id = $1")
            .bind(run_id.as_str())
            .fetch_one(&mut **tx)
            .await
            .map_err(|error| database_error("failed to query run head", error))?;
    let head_seq: i64 = row
        .try_get("head_seq")
        .map_err(|error| database_error("failed to decode run head", error))?;
    i64_to_nonnegative_u64(head_seq, "commits.seq")
}

async fn count_run_events_tx(tx: &mut Transaction<'_, Postgres>, run_id: &RunId) -> Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM run_events WHERE run_id = $1")
        .bind(run_id.as_str())
        .fetch_one(&mut **tx)
        .await
        .map_err(|error| database_error("failed to count run events", error))
}

async fn lock_run_tx(tx: &mut Transaction<'_, Postgres>, run_id: &RunId) -> Result<()> {
    const RUN_LOCK_CLASS_ID: i32 = 0x4d46_5201;
    let object_id = advisory_object_id(run_id.as_str().as_bytes());
    sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
        .bind(RUN_LOCK_CLASS_ID)
        .bind(object_id)
        .execute(&mut **tx)
        .await
        .map_err(|error| database_error("failed to lock run append authority", error))?;
    Ok(())
}

fn advisory_object_id(bytes: &[u8]) -> i32 {
    let digest = sha256_digest_bytes(bytes);
    i32::from_be_bytes([
        digest.as_bytes()[0],
        digest.as_bytes()[1],
        digest.as_bytes()[2],
        digest.as_bytes()[3],
    ])
}

async fn lock_resource_lanes_for_request_tx(
    tx: &mut Transaction<'_, Postgres>,
    request: &mfm_store::v1::CommitRequest,
) -> Result<()> {
    let mut lane_ids = BTreeSet::<Vec<u8>>::new();
    for payload in request.payloads() {
        match payload {
            events::KernelEventPayload::ResourceLaneClaimIntent(intent) => {
                lane_ids.insert(resource_lane_id(&intent.resource_key)?.to_vec());
            }
            events::KernelEventPayload::ResourceLaneReleaseIntent(intent) => {
                lane_ids.insert(load_claim_lane_id_tx(tx, &intent.claim_id).await?);
            }
            _ => {}
        }
    }
    for lane_id in lane_ids {
        lock_resource_lane_tx(tx, &lane_id).await?;
    }
    Ok(())
}

async fn lock_resource_lane_tx(tx: &mut Transaction<'_, Postgres>, lane_id: &[u8]) -> Result<()> {
    const RESOURCE_LANE_LOCK_CLASS_ID: i32 = 0x4d46_5202;
    let object_id = advisory_object_id(lane_id);
    sqlx::query("SELECT pg_advisory_xact_lock($1, $2)")
        .bind(RESOURCE_LANE_LOCK_CLASS_ID)
        .bind(object_id)
        .execute(&mut **tx)
        .await
        .map_err(|error| database_error("failed to lock resource lane", error))?;
    Ok(())
}

fn resource_key_canonical_json(evidence: &events::ResourceKeyEvidence) -> Result<Vec<u8>> {
    Ok(canonical_json(serde_json::json!({
        "key": evidence.key.as_str(),
    }))?
    .to_vec())
}

fn resource_lane_id(evidence: &events::ResourceKeyEvidence) -> Result<[u8; 32]> {
    let key_canonical_json = resource_key_canonical_json(evidence)?;
    let canonical = canonical_json(serde_json::json!({
        "key_canonical_json": String::from_utf8(key_canonical_json).map_err(|error| {
            PostgresStoreError::Corruption(format!("resource key canonical JSON was not UTF-8: {error}"))
        })?,
        "key_schema_id": evidence.key_schema_id.as_str(),
        "mode": "exclusive",
        "namespace": evidence.namespace.as_str(),
    }))?;
    Ok(*canonical.digest_bytes().as_bytes())
}

async fn load_claim_lane_id_tx(
    tx: &mut Transaction<'_, Postgres>,
    claim_id: &events::ResourceLaneClaimId,
) -> Result<Vec<u8>> {
    let row = sqlx::query("SELECT lane_id FROM resource_lane_claim_events WHERE claim_id = $1")
        .bind(claim_id.as_str())
        .fetch_optional(&mut **tx)
        .await
        .map_err(|error| database_error("failed to load resource lane claim", error))?;
    let Some(row) = row else {
        return Err(StoreError::ProjectionConflict {
            key: format!("resource_lane_claim:{}", claim_id),
            message: "resource lane release references unknown claim".to_owned(),
        }
        .into());
    };
    row.try_get("lane_id")
        .map_err(|error| database_error("failed to decode resource lane id", error))
}

async fn insert_resource_lane_authority_rows_tx(
    tx: &mut Transaction<'_, Postgres>,
    commit_id: &str,
    events: &[KernelEventEnvelope],
) -> Result<()> {
    for event in events {
        match event.payload() {
            events::KernelEventPayload::ResourceLaneClaimed(payload) => {
                insert_resource_lane_claim_tx(tx, commit_id, event, payload).await?;
            }
            events::KernelEventPayload::ResourceLaneReleased(payload) => {
                insert_resource_lane_release_tx(tx, commit_id, event, payload).await?;
            }
            _ => {}
        }
    }
    Ok(())
}

async fn insert_resource_lane_claim_tx(
    tx: &mut Transaction<'_, Postgres>,
    commit_id: &str,
    event: &KernelEventEnvelope,
    payload: &events::ResourceLaneClaimed,
) -> Result<()> {
    let lane_id = resource_lane_id(&payload.resource_key)?;
    let key_canonical_json = resource_key_canonical_json(&payload.resource_key)?;
    sqlx::query(
        "INSERT INTO resource_lane_claim_events \
         (claim_id, lane_id, run_id, node_id, attempt_id, ledger_key, ledger_purpose, \
          invocation_epoch, namespace, key_schema_id, key_canonical_json, key_value, mode, \
          requirement_digest, resolved_by_capability_impl, fencing_token, commit_id, source_seq, \
          source_ordinal, source_event_id, source_event_type, source_event_payload_hash) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'exclusive',$13,$14,$15,$16,$17,$18,$19,'ResourceLaneClaimed',$20)",
    )
    .bind(payload.claim_id.as_str())
    .bind(lane_id.as_slice())
    .bind(event.run_id().as_str())
    .bind(payload.node_id.as_str())
    .bind(payload.attempt_id.as_str())
    .bind(payload.ledger_key.as_str())
    .bind(side_effect_ledger_purpose_json(&payload.ledger_purpose))
    .bind(i32::try_from(payload.invocation_epoch).map_err(|_| {
        PostgresStoreError::Corruption("resource lane invocation epoch overflow".to_owned())
    })?)
    .bind(payload.resource_key.namespace.as_str())
    .bind(payload.resource_key.key_schema_id.as_str())
    .bind(key_canonical_json)
    .bind(payload.resource_key.key.as_str())
    .bind(payload.requirement_digest.as_str())
    .bind(payload.resolved_by_capability_impl.as_str())
    .bind(u64_to_i64(payload.claim_fencing_token, "resource_lane_claim_events.fencing_token")?)
    .bind(commit_id)
    .bind(u64_to_i64(event.seq().as_u64(), "resource_lane_claim_events.source_seq")?)
    .bind(i32::try_from(event.ordinal().as_u32()).map_err(|_| {
        PostgresStoreError::Corruption("resource lane source ordinal overflow".to_owned())
    })?)
    .bind(event.event_id().as_str())
    .bind(event.payload_hash().as_str())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert resource lane claim", error))?;
    insert_resource_lane_transition_tx(
        tx,
        ResourceLaneTransitionInsert {
            lane_id: &lane_id,
            lane_transition_seq: payload.lane_transition_seq,
            transition_kind: "claim",
            claim_id: &payload.claim_id,
            release_id: None,
            claim_fencing_token: payload.claim_fencing_token,
            event,
            commit_id,
        },
    )
    .await
}

async fn insert_resource_lane_release_tx(
    tx: &mut Transaction<'_, Postgres>,
    commit_id: &str,
    event: &KernelEventEnvelope,
    payload: &events::ResourceLaneReleased,
) -> Result<()> {
    let claim = load_resource_lane_claim_row_tx(tx, &payload.claim_id).await?;
    if claim.run_id != event.run_id().as_str()
        || claim.claim_fencing_token != payload.claim_fencing_token
    {
        return Err(PostgresStoreError::Corruption(
            "resource lane release claim binding mismatch".to_owned(),
        ));
    }
    sqlx::query(
        "INSERT INTO resource_lane_release_events \
         (release_id, claim_id, lane_id, run_id, node_id, attempt_id, ledger_key, ledger_purpose, \
          invocation_epoch, claim_fencing_token, release_reason, commit_id, source_seq, \
          source_ordinal, source_event_id, source_event_type, source_event_payload_hash) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,'ResourceLaneReleased',$16)",
    )
    .bind(payload.release_id.as_str())
    .bind(payload.claim_id.as_str())
    .bind(claim.lane_id.as_slice())
    .bind(event.run_id().as_str())
    .bind(payload.node_id.as_str())
    .bind(payload.attempt_id.as_str())
    .bind(payload.ledger_key.as_str())
    .bind(side_effect_ledger_purpose_json(&payload.ledger_purpose))
    .bind(i32::try_from(payload.invocation_epoch).map_err(|_| {
        PostgresStoreError::Corruption("resource lane invocation epoch overflow".to_owned())
    })?)
    .bind(u64_to_i64(
        payload.claim_fencing_token,
        "resource_lane_release_events.claim_fencing_token",
    )?)
    .bind(payload.release_reason.as_str())
    .bind(commit_id)
    .bind(u64_to_i64(
        event.seq().as_u64(),
        "resource_lane_release_events.source_seq",
    )?)
    .bind(i32::try_from(event.ordinal().as_u32()).map_err(|_| {
        PostgresStoreError::Corruption("resource lane source ordinal overflow".to_owned())
    })?)
    .bind(event.event_id().as_str())
    .bind(event.payload_hash().as_str())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert resource lane release", error))?;
    insert_resource_lane_transition_tx(
        tx,
        ResourceLaneTransitionInsert {
            lane_id: &claim.lane_id,
            lane_transition_seq: payload.lane_transition_seq,
            transition_kind: "release",
            claim_id: &payload.claim_id,
            release_id: Some(&payload.release_id),
            claim_fencing_token: payload.claim_fencing_token,
            event,
            commit_id,
        },
    )
    .await
}

struct ResourceLaneTransitionInsert<'a> {
    lane_id: &'a [u8],
    lane_transition_seq: u64,
    transition_kind: &'a str,
    claim_id: &'a events::ResourceLaneClaimId,
    release_id: Option<&'a events::ResourceLaneReleaseId>,
    claim_fencing_token: u64,
    event: &'a KernelEventEnvelope,
    commit_id: &'a str,
}

async fn insert_resource_lane_transition_tx(
    tx: &mut Transaction<'_, Postgres>,
    transition: ResourceLaneTransitionInsert<'_>,
) -> Result<()> {
    let previous_transition_hash =
        previous_resource_lane_transition_hash_tx(tx, transition.lane_id).await?;
    let transition_hash =
        resource_lane_transition_hash(&transition, previous_transition_hash.as_deref())?;
    sqlx::query(
        "INSERT INTO resource_lane_transitions \
         (lane_id, lane_transition_seq, transition_kind, claim_id, release_id, claim_fencing_token, \
          previous_transition_hash, transition_hash, run_id, commit_id, source_seq, source_ordinal, \
         source_event_id, source_event_type, source_event_payload_hash) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)",
    )
    .bind(transition.lane_id)
    .bind(u64_to_i64(
        transition.lane_transition_seq,
        "resource_lane_transitions.lane_transition_seq",
    )?)
    .bind(transition.transition_kind)
    .bind(transition.claim_id.as_str())
    .bind(
        transition
            .release_id
            .map(events::ResourceLaneReleaseId::as_str),
    )
    .bind(u64_to_i64(
        transition.claim_fencing_token,
        "resource_lane_transitions.claim_fencing_token",
    )?)
    .bind(previous_transition_hash)
    .bind(transition_hash)
    .bind(transition.event.run_id().as_str())
    .bind(transition.commit_id)
    .bind(u64_to_i64(
        transition.event.seq().as_u64(),
        "resource_lane_transitions.source_seq",
    )?)
    .bind(i32::try_from(transition.event.ordinal().as_u32()).map_err(|_| {
        PostgresStoreError::Corruption("resource lane source ordinal overflow".to_owned())
    })?)
    .bind(transition.event.event_id().as_str())
    .bind(match transition.event.payload() {
        events::KernelEventPayload::ResourceLaneClaimed(_) => "ResourceLaneClaimed",
        events::KernelEventPayload::ResourceLaneReleased(_) => "ResourceLaneReleased",
        _ => {
            return Err(PostgresStoreError::Corruption(
                "resource lane transition source event is not a lane event".to_owned(),
            ))
        }
    })
    .bind(transition.event.payload_hash().as_str())
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert resource lane transition", error))?;
    Ok(())
}

async fn previous_resource_lane_transition_hash_tx(
    tx: &mut Transaction<'_, Postgres>,
    lane_id: &[u8],
) -> Result<Option<String>> {
    sqlx::query_scalar::<_, String>(
        "SELECT transition_hash FROM resource_lane_transitions \
         WHERE lane_id = $1 ORDER BY lane_transition_seq DESC LIMIT 1",
    )
    .bind(lane_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load previous resource lane transition", error))
}

struct ResourceLaneTransitionAuthorityRow {
    lane_id: Vec<u8>,
    lane_transition_seq: u64,
    transition_kind: String,
    claim_id: String,
    release_id: Option<String>,
    claim_fencing_token: u64,
    previous_transition_hash: Option<String>,
    transition_hash: String,
    source_seq: u64,
    source_ordinal: u32,
    source_event_id: String,
    source_event_type: String,
    source_event_payload_hash: String,
}

async fn validate_resource_lane_transition_hash_chains_tx(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<()> {
    let rows = sqlx::query(
        "SELECT lane_id, lane_transition_seq, transition_kind, claim_id, release_id, \
         claim_fencing_token, previous_transition_hash, transition_hash, source_seq, \
         source_ordinal, source_event_id, source_event_type, source_event_payload_hash \
         FROM resource_lane_transitions ORDER BY lane_id ASC, lane_transition_seq ASC",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load resource lane transition rows", error))?;
    let mut current_lane: Option<Vec<u8>> = None;
    let mut expected_seq = 1_u64;
    let mut expected_previous_hash: Option<String> = None;
    for row in rows {
        let row = resource_lane_transition_authority_row(row)?;
        if current_lane.as_ref() != Some(&row.lane_id) {
            current_lane = Some(row.lane_id.clone());
            expected_seq = 1;
            expected_previous_hash = None;
        }
        if row.lane_transition_seq != expected_seq {
            return Err(PostgresStoreError::Corruption(
                "resource lane transition sequence gap".to_owned(),
            ));
        }
        if row.previous_transition_hash != expected_previous_hash {
            return Err(PostgresStoreError::Corruption(
                "resource lane transition previous hash mismatch".to_owned(),
            ));
        }
        if !resource_lane_transition_kind_matches_source(&row) {
            return Err(PostgresStoreError::Corruption(
                "resource lane transition kind/source mismatch".to_owned(),
            ));
        }
        let expected_hash = resource_lane_transition_hash_from_row(&row)?;
        if row.transition_hash != expected_hash {
            return Err(PostgresStoreError::Corruption(
                "resource lane transition hash mismatch".to_owned(),
            ));
        }
        expected_previous_hash = Some(row.transition_hash);
        expected_seq = expected_seq.checked_add(1).ok_or_else(|| {
            PostgresStoreError::Corruption("resource lane transition sequence overflow".to_owned())
        })?;
    }
    Ok(())
}

fn resource_lane_transition_authority_row(
    row: PgRow,
) -> Result<ResourceLaneTransitionAuthorityRow> {
    Ok(ResourceLaneTransitionAuthorityRow {
        lane_id: row
            .try_get("lane_id")
            .map_err(|error| database_error("failed to decode transition lane id", error))?,
        lane_transition_seq: i64_to_positive_u64(
            row.try_get("lane_transition_seq")
                .map_err(|error| database_error("failed to decode transition sequence", error))?,
            "resource_lane_transitions.lane_transition_seq",
        )?,
        transition_kind: row
            .try_get("transition_kind")
            .map_err(|error| database_error("failed to decode transition kind", error))?,
        claim_id: row
            .try_get("claim_id")
            .map_err(|error| database_error("failed to decode transition claim id", error))?,
        release_id: row
            .try_get("release_id")
            .map_err(|error| database_error("failed to decode transition release id", error))?,
        claim_fencing_token: i64_to_positive_u64(
            row.try_get("claim_fencing_token").map_err(|error| {
                database_error("failed to decode transition fencing token", error)
            })?,
            "resource_lane_transitions.claim_fencing_token",
        )?,
        previous_transition_hash: row
            .try_get("previous_transition_hash")
            .map_err(|error| database_error("failed to decode transition previous hash", error))?,
        transition_hash: row
            .try_get("transition_hash")
            .map_err(|error| database_error("failed to decode transition hash", error))?,
        source_seq: i64_to_positive_u64(
            row.try_get("source_seq")
                .map_err(|error| database_error("failed to decode transition source seq", error))?,
            "resource_lane_transitions.source_seq",
        )?,
        source_ordinal: i32_to_u32(
            row.try_get("source_ordinal").map_err(|error| {
                database_error("failed to decode transition source ordinal", error)
            })?,
            "resource_lane_transitions.source_ordinal",
        )?,
        source_event_id: row
            .try_get("source_event_id")
            .map_err(|error| database_error("failed to decode transition event id", error))?,
        source_event_type: row
            .try_get("source_event_type")
            .map_err(|error| database_error("failed to decode transition event type", error))?,
        source_event_payload_hash: row
            .try_get("source_event_payload_hash")
            .map_err(|error| database_error("failed to decode transition payload hash", error))?,
    })
}

fn resource_lane_transition_kind_matches_source(row: &ResourceLaneTransitionAuthorityRow) -> bool {
    matches!(
        (
            row.transition_kind.as_str(),
            row.release_id.is_some(),
            row.source_event_type.as_str()
        ),
        ("claim", false, "ResourceLaneClaimed") | ("release", true, "ResourceLaneReleased")
    )
}

fn resource_lane_transition_hash_from_row(
    row: &ResourceLaneTransitionAuthorityRow,
) -> Result<String> {
    let digest = canonical_json(serde_json::json!({
        "claim_fencing_token": row.claim_fencing_token,
        "claim_id": row.claim_id.as_str(),
        "lane_id": bytes_hex(&row.lane_id),
        "lane_transition_seq": row.lane_transition_seq,
        "previous_transition_hash": row.previous_transition_hash.as_deref(),
        "release_id": row.release_id.as_deref(),
        "source_event_id": row.source_event_id.as_str(),
        "source_event_payload_hash": row.source_event_payload_hash.as_str(),
        "source_ordinal": row.source_ordinal,
        "source_seq": row.source_seq,
        "transition_kind": row.transition_kind.as_str(),
    }))?
    .content_digest();
    Ok(digest.as_str().to_owned())
}

fn resource_lane_transition_hash(
    transition: &ResourceLaneTransitionInsert<'_>,
    previous_transition_hash: Option<&str>,
) -> Result<String> {
    let digest = canonical_json(serde_json::json!({
        "claim_fencing_token": transition.claim_fencing_token,
        "claim_id": transition.claim_id.as_str(),
        "lane_id": bytes_hex(transition.lane_id),
        "lane_transition_seq": transition.lane_transition_seq,
        "previous_transition_hash": previous_transition_hash,
        "release_id": transition.release_id.map(events::ResourceLaneReleaseId::as_str),
        "source_event_id": transition.event.event_id().as_str(),
        "source_event_payload_hash": transition.event.payload_hash().as_str(),
        "source_ordinal": transition.event.ordinal().as_u32(),
        "source_seq": transition.event.seq().as_u64(),
        "transition_kind": transition.transition_kind,
    }))?
    .content_digest();
    Ok(digest.as_str().to_owned())
}

fn bytes_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn bytes_from_hex(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(StoreError::InvalidCursor {
            message: "hex field has odd length".to_owned(),
        }
        .into());
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    for chunk in value.as_bytes().chunks_exact(2) {
        let high = hex_nibble(chunk[0])?;
        let low = hex_nibble(chunk[1])?;
        out.push((high << 4) | low);
    }
    Ok(out)
}

fn hex_nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(StoreError::InvalidCursor {
            message: "hex field contains a non-hex byte".to_owned(),
        }
        .into()),
    }
}

struct ResourceLaneClaimAuthorityRow {
    lane_id: Vec<u8>,
    run_id: String,
    claim_fencing_token: u64,
}

async fn load_resource_lane_claim_row_tx(
    tx: &mut Transaction<'_, Postgres>,
    claim_id: &events::ResourceLaneClaimId,
) -> Result<ResourceLaneClaimAuthorityRow> {
    let row = sqlx::query(
        "SELECT lane_id, run_id, fencing_token \
         FROM resource_lane_claim_events WHERE claim_id = $1",
    )
    .bind(claim_id.as_str())
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load resource lane claim", error))?;
    let fencing_token: i64 = row
        .try_get("fencing_token")
        .map_err(|error| database_error("failed to decode resource lane claim token", error))?;
    Ok(ResourceLaneClaimAuthorityRow {
        lane_id: row
            .try_get("lane_id")
            .map_err(|error| database_error("failed to decode resource lane id", error))?,
        run_id: row.try_get("run_id").map_err(|error| {
            database_error("failed to decode resource lane claim run id", error)
        })?,
        claim_fencing_token: i64_to_positive_u64(
            fencing_token,
            "resource_lane_claim_events.fencing_token",
        )?,
    })
}

async fn read_commit_by_key(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    commit_key: &str,
) -> Result<Option<CommitAuthorityRow>> {
    let row = sqlx::query(
        "SELECT commit_id, run_id, seq, commit_key, commit_purpose, commit_idempotency_hash, \
         idempotency_canonical_json, prepared_authority_hash, prepared_authority_canonical_json, \
         commit_batch_hash, commit_batch_canonical_json, hash_domain_version, \
         canonicalizer_identity, event_count \
         FROM commits WHERE run_id = $1 AND commit_key = $2",
    )
    .bind(run_id.as_str())
    .bind(commit_key)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to query commit authority", error))?;
    row.map(commit_authority_row_from_row).transpose()
}

async fn read_commit_by_seq(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    seq: StreamSeq,
) -> Result<CommitAuthorityRow> {
    let row = sqlx::query(
        "SELECT commit_id, run_id, seq, commit_key, commit_purpose, commit_idempotency_hash, \
         idempotency_canonical_json, prepared_authority_hash, prepared_authority_canonical_json, \
         commit_batch_hash, commit_batch_canonical_json, hash_domain_version, \
         canonicalizer_identity, event_count \
         FROM commits WHERE run_id = $1 AND seq = $2",
    )
    .bind(run_id.as_str())
    .bind(u64_to_i64(seq.as_u64(), "commits.seq")?)
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load commit authority", error))?;
    commit_authority_row_from_row(row)
}

async fn load_committed_batch_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    seq: StreamSeq,
) -> Result<CommittedBatch> {
    let commit = read_commit_by_seq(tx, run_id, seq).await?;
    let events = load_run_commit_events_tx(tx, run_id, seq).await?;
    if events.len() != commit.event_count {
        return Err(PostgresStoreError::Corruption(
            "commit event count does not match run_events rows".to_owned(),
        ));
    }
    validate_run_commit_log_row_tx(tx, &commit).await?;
    validate_resource_lane_transition_hash_chains_tx(tx).await?;
    CommittedBatch::from_persisted_events(
        run_id.clone(),
        CommitKey::new(commit.commit_key)?,
        CommitFingerprint::from_digest(parse_identity::<ContentDigest>(
            &commit.commit_idempotency_hash,
        )?),
        seq,
        events,
    )
    .map_err(PostgresStoreError::from)
}

async fn load_commit_authority_rows_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<Vec<CommitAuthorityRow>> {
    let rows = sqlx::query(
        "SELECT commit_id, run_id, seq, commit_key, commit_purpose, commit_idempotency_hash, \
         idempotency_canonical_json, prepared_authority_hash, prepared_authority_canonical_json, \
         commit_batch_hash, commit_batch_canonical_json, hash_domain_version, \
         canonicalizer_identity, event_count \
         FROM commits WHERE run_id = $1 ORDER BY seq ASC",
    )
    .bind(run_id.as_str())
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load commit authority rows", error))?;
    rows.into_iter()
        .map(commit_authority_row_from_row)
        .collect()
}

async fn validate_persisted_run_authority_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<()> {
    let commits = load_commit_authority_rows_tx(tx, run_id).await?;
    for commit in &commits {
        validate_run_commit_log_row_tx(tx, commit).await?;
        validate_run_commit_event_count_tx(tx, commit).await?;
    }
    validate_resource_lane_transition_hash_chains_tx(tx).await
}

async fn validate_run_commit_event_count_tx(
    tx: &mut Transaction<'_, Postgres>,
    commit: &CommitAuthorityRow,
) -> Result<()> {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM run_events WHERE run_id = $1 AND seq = $2")
            .bind(commit.run_id.as_str())
            .bind(u64_to_i64(commit.seq.as_u64(), "run_events.seq")?)
            .fetch_one(&mut **tx)
            .await
            .map_err(|error| database_error("failed to count commit events", error))?;
    let count = usize::try_from(count).map_err(|_| {
        PostgresStoreError::Corruption("commit event count was negative".to_owned())
    })?;
    if count != commit.event_count {
        return Err(PostgresStoreError::Corruption(
            "commit event count does not match run_events rows".to_owned(),
        ));
    }
    Ok(())
}

async fn validate_run_commit_log_row_tx(
    tx: &mut Transaction<'_, Postgres>,
    commit: &CommitAuthorityRow,
) -> Result<()> {
    let row = sqlx::query(
        "SELECT commit_id, run_id, seq, commit_key, first_ordinal, last_ordinal, event_count, \
         commit_sort_key FROM run_commit_log WHERE commit_id = $1",
    )
    .bind(&commit.commit_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load run commit log row", error))?
    .ok_or_else(|| {
        PostgresStoreError::Corruption(format!(
            "commit {} is missing run_commit_log row",
            commit.commit_id
        ))
    })?;
    let log_run_id = parse_identity::<RunId>(
        &row.try_get::<String, _>("run_id")
            .map_err(|error| database_error("failed to decode run commit log run id", error))?,
    )?;
    let log_seq = StreamSeq::new(i64_to_positive_u64(
        row.try_get("seq")
            .map_err(|error| database_error("failed to decode run commit log seq", error))?,
        "run_commit_log.seq",
    )?)?;
    let log_commit_key: String = row
        .try_get("commit_key")
        .map_err(|error| database_error("failed to decode run commit log key", error))?;
    let first_ordinal: i32 = row
        .try_get("first_ordinal")
        .map_err(|error| database_error("failed to decode run commit log first ordinal", error))?;
    let last_ordinal: i32 = row
        .try_get("last_ordinal")
        .map_err(|error| database_error("failed to decode run commit log last ordinal", error))?;
    let event_count: i32 = row
        .try_get("event_count")
        .map_err(|error| database_error("failed to decode run commit log event count", error))?;
    let commit_sort_key: Vec<u8> = row
        .try_get("commit_sort_key")
        .map_err(|error| database_error("failed to decode run commit sort key", error))?;
    if log_run_id != commit.run_id
        || log_seq != commit.seq
        || log_commit_key != commit.commit_key
        || first_ordinal != 0
        || usize::try_from(event_count).ok() != Some(commit.event_count)
        || last_ordinal
            != i32::try_from(commit.event_count.saturating_sub(1)).map_err(|_| {
                PostgresStoreError::Corruption(
                    "run_commit_log event count exceeded ordinal range".to_owned(),
                )
            })?
    {
        return Err(PostgresStoreError::Corruption(
            "run_commit_log row does not match commit authority".to_owned(),
        ));
    }
    let expected_sort_key = run_commit_sort_key(
        &commit.run_id,
        commit.seq,
        &CommitKey::new(commit.commit_key.clone())?,
        &commit.commit_id,
        &commit.commit_batch_hash,
    )?;
    if commit_sort_key != expected_sort_key {
        return Err(PostgresStoreError::Corruption(
            "run_commit_log sort key does not match commit authority".to_owned(),
        ));
    }
    Ok(())
}

async fn load_run_commit_events_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    seq: StreamSeq,
) -> Result<Vec<KernelEventEnvelope>> {
    let rows = sqlx::query(
        "SELECT run_id, seq, ordinal, event_id, event_schema_id, spec_hash, commit_key, \
         logical_key, payload_hash, payload_canonical_json \
         FROM run_events WHERE run_id = $1 AND seq = $2 ORDER BY ordinal ASC",
    )
    .bind(run_id.as_str())
    .bind(u64_to_i64(seq.as_u64(), "run_events.seq")?)
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load committed run events", error))?;
    rows.into_iter().map(event_envelope_from_row).collect()
}

struct CommitAuthorityRow {
    commit_id: String,
    run_id: RunId,
    commit_key: String,
    commit_idempotency_hash: String,
    commit_batch_hash: String,
    seq: StreamSeq,
    event_count: usize,
}

fn commit_authority_row_from_row(row: PgRow) -> Result<CommitAuthorityRow> {
    let commit_id: String = row
        .try_get("commit_id")
        .map_err(|error| database_error("failed to decode commit id", error))?;
    let run_id = parse_identity::<RunId>(
        &row.try_get::<String, _>("run_id")
            .map_err(|error| database_error("failed to decode commit run id", error))?,
    )?;
    let commit_key = row
        .try_get::<String, _>("commit_key")
        .map_err(|error| database_error("failed to decode commit key", error))?;
    let commit_purpose: String = row
        .try_get("commit_purpose")
        .map_err(|error| database_error("failed to decode commit purpose", error))?;
    let commit_idempotency_hash: String = row
        .try_get("commit_idempotency_hash")
        .map_err(|error| database_error("failed to decode commit idempotency hash", error))?;
    let idempotency_canonical_json: Vec<u8> = row
        .try_get("idempotency_canonical_json")
        .map_err(|error| database_error("failed to decode commit idempotency bytes", error))?;
    let prepared_authority_hash: String = row
        .try_get("prepared_authority_hash")
        .map_err(|error| database_error("failed to decode prepared authority hash", error))?;
    let prepared_authority_canonical_json: Vec<u8> = row
        .try_get("prepared_authority_canonical_json")
        .map_err(|error| database_error("failed to decode prepared authority bytes", error))?;
    let commit_batch_hash: String = row
        .try_get("commit_batch_hash")
        .map_err(|error| database_error("failed to decode commit batch hash", error))?;
    let commit_batch_canonical_json: Vec<u8> = row
        .try_get("commit_batch_canonical_json")
        .map_err(|error| database_error("failed to decode commit batch bytes", error))?;
    let seq: i64 = row
        .try_get("seq")
        .map_err(|error| database_error("failed to decode commit seq", error))?;
    let event_count: i32 = row
        .try_get("event_count")
        .map_err(|error| database_error("failed to decode commit event count", error))?;
    let hash_domain_version: String = row
        .try_get("hash_domain_version")
        .map_err(|error| database_error("failed to decode commit hash domain", error))?;
    if hash_domain_version != HASH_DOMAIN_VERSION {
        return Err(PostgresStoreError::Corruption(
            "commit hash domain version mismatch".to_owned(),
        ));
    }
    let canonicalizer_identity: String = row
        .try_get("canonicalizer_identity")
        .map_err(|error| database_error("failed to decode commit canonicalizer", error))?;
    if canonicalizer_identity != CANONICALIZER_IDENTITY {
        return Err(PostgresStoreError::Corruption(
            "commit canonicalizer identity mismatch".to_owned(),
        ));
    }
    verify_persisted_canonical_hash(
        "commit idempotency",
        &idempotency_canonical_json,
        &commit_idempotency_hash,
    )?;
    verify_persisted_canonical_hash(
        "prepared authority",
        &prepared_authority_canonical_json,
        &prepared_authority_hash,
    )?;
    verify_persisted_canonical_hash(
        "commit batch",
        &commit_batch_canonical_json,
        &commit_batch_hash,
    )?;
    let seq = StreamSeq::new(i64_to_positive_u64(seq, "commits.seq")?)?;
    let commit_key_identity = CommitKey::new(commit_key.clone())?;
    let expected_commit_id = derive_commit_id(
        &run_id,
        seq,
        &commit_key_identity,
        &commit_purpose,
        &prepared_authority_hash,
    )?;
    if commit_id != expected_commit_id {
        return Err(PostgresStoreError::Corruption(
            "commit id does not match persisted prepared authority".to_owned(),
        ));
    }
    Ok(CommitAuthorityRow {
        commit_id,
        run_id,
        commit_key,
        commit_idempotency_hash,
        commit_batch_hash,
        seq,
        event_count: usize::try_from(event_count).map_err(|_| {
            PostgresStoreError::Corruption("commits.event_count was negative".to_owned())
        })?,
    })
}

fn verify_persisted_canonical_hash(context: &str, bytes: &[u8], expected: &str) -> Result<()> {
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes).map_err(|error| {
        PostgresStoreError::Corruption(format!("{context} canonical bytes are invalid: {error}"))
    })?;
    if canonical.content_digest().as_str() != expected {
        return Err(PostgresStoreError::Corruption(format!(
            "{context} hash does not match persisted canonical bytes"
        )));
    }
    Ok(())
}

async fn load_artifacts(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<ArtifactAuthorityMap> {
    let rows = sqlx::query(
        "SELECT a.artifact_id, a.evidence_hash, a.digest, a.byte_len, a.media_type, a.schema_id, \
         a.semantic_type_id, a.producer_node_id, a.producer_seed_id, a.artifact_role \
         FROM artifact_admissions a \
         INNER JOIN run_artifact_admissions ra \
           ON ra.artifact_id = a.artifact_id AND ra.evidence_hash = a.evidence_hash \
         WHERE ra.run_id = $1",
    )
    .bind(run_id.as_str())
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load artifact evidence", error))?;
    let mut artifacts = BTreeMap::new();
    for row in rows {
        let artifact_id: String = row
            .try_get("artifact_id")
            .map_err(|error| database_error("failed to decode artifact evidence row", error))?;
        let artifact_id = parse_identity::<ArtifactId>(&artifact_id)?;
        let evidence_hash: String = row
            .try_get("evidence_hash")
            .map_err(|error| database_error("failed to decode artifact evidence row", error))?;
        let evidence_hash = parse_identity::<ContentDigest>(&evidence_hash)?;
        let evidence = ArtifactEvidenceParts {
            artifact_id: artifact_id.clone(),
            digest: row
                .try_get("digest")
                .map_err(|error| database_error("failed to decode artifact evidence row", error))?,
            byte_len: row
                .try_get("byte_len")
                .map_err(|error| database_error("failed to decode artifact evidence row", error))?,
            media_type: row
                .try_get("media_type")
                .map_err(|error| database_error("failed to decode artifact evidence row", error))?,
            schema_id: row
                .try_get("schema_id")
                .map_err(|error| database_error("failed to decode artifact evidence row", error))?,
            semantic_type_id: row
                .try_get("semantic_type_id")
                .map_err(|error| database_error("failed to decode artifact evidence row", error))?,
            producer_node_id: row
                .try_get("producer_node_id")
                .map_err(|error| database_error("failed to decode artifact evidence row", error))?,
            producer_seed_id: row
                .try_get("producer_seed_id")
                .map_err(|error| database_error("failed to decode artifact evidence row", error))?,
            artifact_role: row
                .try_get("artifact_role")
                .map_err(|error| database_error("failed to decode artifact evidence row", error))?,
        }
        .into_evidence_ref()?;
        if evidence.evidence_hash()? != evidence_hash {
            return Err(StoreError::ArtifactEvidenceMismatch {
                artifact_id,
                field: "evidence_hash",
            }
            .into());
        }
        let key = (artifact_id.clone(), evidence_hash);
        if let Some(existing) = artifacts.get(&key) {
            if existing != &evidence {
                return Err(StoreError::ArtifactEvidenceMismatch {
                    artifact_id,
                    field: "artifact",
                }
                .into());
            }
        } else {
            artifacts.insert(key, evidence);
        }
    }
    Ok(artifacts)
}

async fn load_artifact_record_tx(
    tx: &mut Transaction<'_, Postgres>,
    artifact_id: &ArtifactId,
    evidence_hash: &ContentDigest,
) -> Result<ArtifactRecord> {
    let row = sqlx::query(
        "SELECT a.artifact_id, a.evidence_hash, a.digest, a.byte_len, a.media_type, a.schema_id, \
         a.semantic_type_id, a.producer_node_id, a.producer_seed_id, a.artifact_role, b.bytes \
         FROM artifact_admissions a \
         INNER JOIN artifact_blobs b \
           ON b.artifact_id = a.artifact_id AND b.digest = a.digest AND b.byte_len = a.byte_len \
         WHERE a.artifact_id = $1 AND a.evidence_hash = $2",
    )
    .bind(artifact_id.as_str())
    .bind(evidence_hash.as_str())
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to query artifact bytes", error))?;
    let Some(row) = row else {
        return Err(StoreError::MissingArtifact {
            artifact_id: artifact_id.clone(),
        }
        .into());
    };
    artifact_record_from_row(row)
}

fn verify_artifact_record(
    record: &ArtifactRecord,
    evidence: &ArtifactEvidenceRef,
    bytes: Option<&[u8]>,
) -> Result<()> {
    PreparedArtifactBytes::new(record.artifact_bytes.clone(), record.evidence.clone())?;
    if record.evidence_hash != evidence.evidence_hash()? || record.evidence != *evidence {
        return Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "artifact",
        }
        .into());
    }
    if let Some(bytes) = bytes {
        if record.artifact_bytes.as_slice() != bytes {
            return Err(StoreError::ArtifactEvidenceMismatch {
                artifact_id: evidence.artifact_id.clone(),
                field: "artifact_bytes",
            }
            .into());
        }
    }
    Ok(())
}

struct ArtifactRecord {
    evidence_hash: ContentDigest,
    evidence: ArtifactEvidenceRef,
    artifact_bytes: Vec<u8>,
}

fn artifact_record_from_row(row: PgRow) -> Result<ArtifactRecord> {
    let artifact_id_text: String = row
        .try_get("artifact_id")
        .map_err(|error| database_error("failed to decode artifact row", error))?;
    let evidence_hash_text: String = row
        .try_get("evidence_hash")
        .map_err(|error| database_error("failed to decode artifact row", error))?;
    let evidence = ArtifactEvidenceParts {
        artifact_id: parse_identity::<ArtifactId>(&artifact_id_text)?,
        digest: row
            .try_get("digest")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
        byte_len: row
            .try_get("byte_len")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
        media_type: row
            .try_get("media_type")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
        schema_id: row
            .try_get("schema_id")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
        semantic_type_id: row
            .try_get("semantic_type_id")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
        producer_node_id: row
            .try_get("producer_node_id")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
        producer_seed_id: row
            .try_get("producer_seed_id")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
        artifact_role: row
            .try_get("artifact_role")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
    }
    .into_evidence_ref()?;
    Ok(ArtifactRecord {
        evidence_hash: parse_identity::<ContentDigest>(&evidence_hash_text)?,
        evidence,
        artifact_bytes: row
            .try_get("bytes")
            .map_err(|error| database_error("failed to decode artifact row", error))?,
    })
}

async fn read_retained_artifact_from_pool(
    pool: &PgPool,
    requirement: &EventArtifactRequirement,
) -> mfm_store::v1::Result<VerifiedRunArtifactBytes> {
    let rows = sqlx::query(
        "SELECT a.artifact_id, a.evidence_hash, a.digest, a.byte_len, a.media_type, a.schema_id, \
         a.semantic_type_id, a.producer_node_id, a.producer_seed_id, a.artifact_role, b.bytes \
         FROM artifact_admissions a \
         INNER JOIN artifact_blobs b \
           ON b.artifact_id = a.artifact_id AND b.digest = a.digest AND b.byte_len = a.byte_len \
         WHERE a.artifact_id = $1 ORDER BY a.evidence_hash",
    )
    .bind(requirement.artifact_id.as_str())
    .fetch_all(pool)
    .await
    .map_err(|_| StoreError::ArtifactReadFailed {
        artifact_id: requirement.artifact_id.clone(),
    })?;
    if rows.is_empty() {
        return Err(StoreError::MissingArtifact {
            artifact_id: requirement.artifact_id.clone(),
        });
    }

    let mut saw_mismatch = false;
    for row in rows {
        let record = artifact_record_from_row(row).map_err(|_| StoreError::ArtifactReadFailed {
            artifact_id: requirement.artifact_id.clone(),
        })?;
        match VerifiedRunArtifactBytes::new(record.artifact_bytes, record.evidence, requirement) {
            Ok(verified) => return Ok(verified),
            Err(StoreError::ArtifactEvidenceMismatch { .. }) => {
                saw_mismatch = true;
            }
            Err(error) => return Err(error),
        }
    }

    if saw_mismatch {
        Err(StoreError::ArtifactEvidenceMismatch {
            artifact_id: requirement.artifact_id.clone(),
            field: "artifact",
        })
    } else {
        Err(StoreError::MissingArtifact {
            artifact_id: requirement.artifact_id.clone(),
        })
    }
}

async fn read_artifact_from_pool(
    pool: &PgPool,
    request: &mfm_artifact_capabilities::ArtifactReadRequest,
) -> mfm_artifact_capabilities::Result<mfm_artifact_capabilities::VerifiedArtifactBytes> {
    let rows = sqlx::query(
        "SELECT a.artifact_id, a.evidence_hash, a.digest, a.byte_len, a.media_type, a.schema_id, \
         a.semantic_type_id, a.producer_node_id, a.producer_seed_id, a.artifact_role, b.bytes \
         FROM artifact_admissions a \
         INNER JOIN artifact_blobs b \
           ON b.artifact_id = a.artifact_id AND b.digest = a.digest AND b.byte_len = a.byte_len \
         WHERE a.artifact_id = $1 ORDER BY a.evidence_hash",
    )
    .bind(request.artifact_id().as_str())
    .fetch_all(pool)
    .await
    .map_err(mfm_artifact_capabilities::ArtifactReadError::redacted_backend_failure)?;
    if rows.is_empty() {
        return Err(mfm_artifact_capabilities::ArtifactReadError::NotFound {
            artifact_id: Box::new(request.artifact_id().clone()),
        });
    }

    let mut saw_mismatch = false;
    for row in rows {
        let record = artifact_record_from_row(row)
            .map_err(mfm_artifact_capabilities::ArtifactReadError::redacted_backend_failure)?;
        let evidence = mfm_artifact_capabilities::ArtifactEvidenceRef::from(record.evidence);
        match mfm_artifact_capabilities::VerifiedArtifactBytes::new(
            record.artifact_bytes,
            evidence,
            request,
        ) {
            Ok(verified) => return Ok(verified),
            Err(mfm_artifact_capabilities::ArtifactReadError::EvidenceMismatch { .. }) => {
                saw_mismatch = true;
            }
            Err(error) => return Err(error),
        }
    }

    if saw_mismatch {
        Err(
            mfm_artifact_capabilities::ArtifactReadError::EvidenceMismatch {
                artifact_id: Box::new(request.artifact_id().clone()),
                field: "artifact",
            },
        )
    } else {
        Err(mfm_artifact_capabilities::ArtifactReadError::NotFound {
            artifact_id: Box::new(request.artifact_id().clone()),
        })
    }
}

struct ArtifactEvidenceParts {
    artifact_id: ArtifactId,
    digest: String,
    byte_len: i64,
    media_type: String,
    schema_id: Option<String>,
    semantic_type_id: Option<String>,
    producer_node_id: Option<String>,
    producer_seed_id: Option<String>,
    artifact_role: String,
}

impl ArtifactEvidenceParts {
    fn into_evidence_ref(self) -> Result<ArtifactEvidenceRef> {
        Ok(ArtifactEvidenceRef {
            artifact_id: self.artifact_id,
            digest: parse_identity::<ContentDigest>(&self.digest)?,
            byte_len: i64_to_nonnegative_u64(self.byte_len, "artifact_admissions.byte_len")?,
            media_type: MediaType::new(self.media_type)?,
            schema_id: parse_optional_identity(self.schema_id)?,
            semantic_type_id: parse_optional_identity(self.semantic_type_id)?,
            producer_node_id: parse_optional_identity(self.producer_node_id)?,
            producer_seed_id: parse_optional_identity(self.producer_seed_id)?,
            artifact_role: decode_artifact_role_tag(&self.artifact_role)?,
        })
    }
}

fn decode_artifact_role_tag(value: &str) -> Result<events::ArtifactRole> {
    events::ArtifactRole::parse(value)
        .ok_or_else(|| StoreError::Identity(format!("unknown artifact role {value}")).into())
}

async fn load_logical_keys(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<BTreeSet<(RunId, LogicalEventKey)>> {
    let rows = sqlx::query("SELECT logical_key FROM run_events WHERE run_id = $1")
        .bind(run_id.as_str())
        .fetch_all(&mut **tx)
        .await
        .map_err(|error| database_error("failed to load logical keys", error))?;
    let mut keys = BTreeSet::new();
    for row in rows {
        let logical_key: String = row
            .try_get("logical_key")
            .map_err(|error| database_error("failed to decode logical key", error))?;
        keys.insert((run_id.clone(), LogicalEventKey::new(logical_key)?));
    }
    Ok(keys)
}

async fn load_unique_logical_payloads(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<BTreeMap<(RunId, LogicalEventKey), ContentDigest>> {
    let rows = sqlx::query("SELECT logical_key, payload_hash FROM run_events WHERE run_id = $1")
        .bind(run_id.as_str())
        .fetch_all(&mut **tx)
        .await
        .map_err(|error| database_error("failed to load unique logical payloads", error))?;
    let mut payloads = BTreeMap::new();
    for row in rows {
        let logical_key = LogicalEventKey::new(
            row.try_get::<String, _>("logical_key")
                .map_err(|error| database_error("failed to decode logical key", error))?,
        )?;
        if !is_unique_logical_key(&logical_key) {
            continue;
        }
        let payload_hash = parse_identity::<ContentDigest>(
            &row.try_get::<String, _>("payload_hash")
                .map_err(|error| database_error("failed to decode payload hash", error))?,
        )?;
        let key = (run_id.clone(), logical_key);
        if let Some(existing) = payloads.insert(key.clone(), payload_hash.clone()) {
            if existing != payload_hash {
                return Err(PostgresStoreError::Corruption(format!(
                    "run_events contain conflicting payload hashes for logical key {}",
                    key.1
                )));
            }
        }
    }
    Ok(payloads)
}

async fn load_projection_snapshot_client(
    pool: &PgPool,
    run_id: &RunId,
) -> Result<ProjectionSnapshot> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| database_error("failed to start read transaction", error))?;
    let snapshot = load_stream_authoritative_projection_snapshot_tx(&mut tx, run_id).await?;
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit read transaction", error))?;
    Ok(snapshot)
}

async fn load_stream_authoritative_projection_snapshot_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<ProjectionSnapshot> {
    let stream = load_run_stream_tx(tx, run_id).await?;
    let snapshot = ProjectionSnapshot::rebuild_from_run_stream(&stream)?;
    let resource_lanes = load_active_resource_lanes_tx(tx).await?;
    projection_snapshot_with_resource_lanes(&snapshot, resource_lanes)
}

async fn load_active_resource_lanes_tx(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<BTreeMap<ResourceLaneKey, ResourceLaneProjection>> {
    let rows = sqlx::query(
        "SELECT c.lane_id, c.run_id, c.node_id, c.attempt_id, c.ledger_key, c.ledger_purpose, \
         c.invocation_epoch, c.namespace, c.key_schema_id, c.key_value, c.claim_id, \
         c.fencing_token, c.source_event_id, t.lane_transition_seq \
         FROM resource_lane_claim_events c \
         INNER JOIN resource_lane_transitions t \
           ON t.claim_id = c.claim_id AND t.transition_kind = 'claim' \
         WHERE NOT EXISTS (\
           SELECT 1 FROM resource_lane_release_events r WHERE r.claim_id = c.claim_id\
         ) \
         ORDER BY c.lane_id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load active resource lanes", error))?;
    let mut resource_lanes = BTreeMap::new();
    for row in rows {
        let evidence = resource_key_evidence_from_lane_row(&row)?;
        verify_lane_id_row(&row, &evidence)?;
        let lane_key = ResourceLaneKey::from_evidence(&evidence);
        let projection = ResourceLaneProjection {
            event_id: parse_identity(&row.try_get::<String, _>("source_event_id").map_err(
                |error| database_error("failed to decode resource lane event id", error),
            )?)?,
            holder: mfm_store::v1::SideEffectLedgerRef::new(
                parse_identity(&row.try_get::<String, _>("run_id").map_err(|error| {
                    database_error("failed to decode resource lane run id", error)
                })?)?,
                events::SideEffectLedgerKey::new(row.try_get::<String, _>("ledger_key").map_err(
                    |error| database_error("failed to decode resource lane ledger key", error),
                )?)?,
            ),
            ledger_purpose: parse_side_effect_ledger_purpose(
                &row.try_get::<Value, _>("ledger_purpose").map_err(|error| {
                    database_error("failed to decode resource lane purpose", error)
                })?,
            )?,
            node_id: parse_identity(&row.try_get::<String, _>("node_id").map_err(|error| {
                database_error("failed to decode resource lane node id", error)
            })?)?,
            attempt_id: parse_identity(&row.try_get::<String, _>("attempt_id").map_err(
                |error| database_error("failed to decode resource lane attempt id", error),
            )?)?,
            invocation_epoch: i32_to_u32(
                row.try_get("invocation_epoch").map_err(|error| {
                    database_error("failed to decode resource lane epoch", error)
                })?,
                "resource_lane_claim_events.invocation_epoch",
            )?,
            claim_id: events::ResourceLaneClaimId::new(
                row.try_get::<String, _>("claim_id").map_err(|error| {
                    database_error("failed to decode resource lane claim id", error)
                })?,
            )?,
            claim_fencing_token: i64_to_positive_u64(
                row.try_get("fencing_token").map_err(|error| {
                    database_error("failed to decode resource lane fencing token", error)
                })?,
                "resource_lane_claim_events.fencing_token",
            )?,
            lane_transition_seq: i64_to_positive_u64(
                row.try_get("lane_transition_seq").map_err(|error| {
                    database_error("failed to decode resource lane transition seq", error)
                })?,
                "resource_lane_transitions.lane_transition_seq",
            )?,
        };
        if resource_lanes.insert(lane_key, projection).is_some() {
            return Err(PostgresStoreError::Corruption(
                "resource lane tables contain duplicate active resource lane".to_owned(),
            ));
        }
    }
    Ok(resource_lanes)
}

async fn load_resource_lane_authority_tx(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<ResourceLaneAuthoritySet> {
    let rows = sqlx::query(
        "SELECT c.lane_id, c.namespace, c.key_schema_id, c.key_value, \
         MAX(t.lane_transition_seq) AS last_transition_seq, \
         MAX(t.claim_fencing_token) AS last_claim_fencing_token \
         FROM resource_lane_transitions t \
         INNER JOIN resource_lane_claim_events c ON c.claim_id = t.claim_id \
         GROUP BY c.lane_id, c.namespace, c.key_schema_id, c.key_value",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load resource lane authority", error))?;
    let mut authority = ResourceLaneAuthoritySet::new();
    for row in rows {
        let evidence = resource_key_evidence_from_lane_row(&row)?;
        verify_lane_id_row(&row, &evidence)?;
        authority.insert(
            ResourceLaneKey::from_evidence(&evidence),
            mfm_store::v1::ResourceLaneAuthority {
                last_transition_seq: i64_to_positive_u64(
                    row.try_get("last_transition_seq").map_err(|error| {
                        database_error("failed to decode resource lane transition seq", error)
                    })?,
                    "resource_lane_transitions.lane_transition_seq",
                )?,
                last_claim_fencing_token: i64_to_positive_u64(
                    row.try_get("last_claim_fencing_token").map_err(|error| {
                        database_error("failed to decode resource lane fencing token", error)
                    })?,
                    "resource_lane_transitions.claim_fencing_token",
                )?,
            },
        );
    }
    Ok(authority)
}

fn resource_key_evidence_from_lane_row(row: &PgRow) -> Result<events::ResourceKeyEvidence> {
    Ok(events::ResourceKeyEvidence {
        namespace: ResourceNamespace::new(
            row.try_get::<String, _>("namespace").map_err(|error| {
                database_error("failed to decode resource lane namespace", error)
            })?,
        )
        .map_err(|error| PostgresStoreError::Store(StoreError::Identity(error.to_string())))?,
        key_schema_id: parse_identity(&row.try_get::<String, _>("key_schema_id").map_err(
            |error| database_error("failed to decode resource lane key schema", error),
        )?)?,
        key: events::ResourceKey::new(
            row.try_get::<String, _>("key_value").map_err(|error| {
                database_error("failed to decode resource lane key value", error)
            })?,
        )?,
    })
}

fn verify_lane_id_row(row: &PgRow, evidence: &events::ResourceKeyEvidence) -> Result<()> {
    let stored: Vec<u8> = row
        .try_get("lane_id")
        .map_err(|error| database_error("failed to decode resource lane id", error))?;
    let derived = resource_lane_id(evidence)?;
    if stored != derived {
        return Err(PostgresStoreError::Corruption(
            "resource lane id does not match lane descriptor".to_owned(),
        ));
    }
    Ok(())
}

fn projection_snapshot_with_resource_lanes(
    snapshot: &ProjectionSnapshot,
    resource_lanes: BTreeMap<ResourceLaneKey, ResourceLaneProjection>,
) -> Result<ProjectionSnapshot> {
    Ok(ProjectionSnapshot::from_parts(ProjectionSnapshotParts {
        run_states: snapshot
            .run_states()
            .map(|(run_id, state)| (run_id.clone(), *state))
            .collect(),
        saga_policy_digests: snapshot
            .saga_policy_digests()
            .map(|(run_id, digest)| (run_id.clone(), digest.clone()))
            .collect(),
        run_completions: snapshot
            .run_completions()
            .map(|(run_id, projection)| (run_id.clone(), projection.clone()))
            .collect(),
        saga_engagements: snapshot
            .saga_engagements()
            .map(|(run_id, projection)| (run_id.clone(), projection.clone()))
            .collect(),
        manual_resolutions: snapshot
            .manual_resolutions()
            .map(|(run_id, projection)| (run_id.clone(), projection.clone()))
            .collect(),
        attempts: snapshot
            .attempts()
            .map(|(key, projection)| (key.clone(), projection.clone()))
            .collect(),
        cells: snapshot
            .cells()
            .map(|(cell_id, projection)| (cell_id.clone(), projection.clone()))
            .collect(),
        facts: snapshot
            .facts()
            .map(|(key, projection)| (key.clone(), projection.clone()))
            .collect(),
        side_effects: snapshot
            .side_effects()
            .map(|(ledger_ref, projection)| (ledger_ref.clone(), projection.clone()))
            .collect(),
        resource_lanes,
        public_outputs: snapshot
            .public_outputs()
            .map(|(schema_id, projection)| (schema_id.clone(), projection.clone()))
            .collect(),
        retentions: snapshot
            .retentions()
            .map(|(run_id, projection)| (run_id.clone(), projection.clone()))
            .collect(),
    })?)
}

async fn load_run_stream_client(pool: &PgPool, run_id: &RunId) -> Result<Vec<KernelEventEnvelope>> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|error| database_error("failed to start read transaction", error))?;
    let events = load_run_stream_tx(&mut tx, run_id).await?;
    tx.commit()
        .await
        .map_err(|error| database_error("failed to commit read transaction", error))?;
    Ok(events)
}

async fn load_run_stream_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<Vec<KernelEventEnvelope>> {
    validate_persisted_run_authority_tx(tx, run_id).await?;
    let rows = sqlx::query(
        "SELECT run_id, seq, ordinal, event_id, event_schema_id, spec_hash, commit_key, \
         logical_key, payload_hash, payload_canonical_json \
         FROM run_events WHERE run_id = $1 ORDER BY seq ASC, ordinal ASC",
    )
    .bind(run_id.as_str())
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load run stream", error))?;
    let events = rows
        .into_iter()
        .map(event_envelope_from_row)
        .collect::<Result<Vec<_>>>()?;
    ProjectionSnapshot::validate_run_stream(&events)?;
    Ok(events)
}

fn event_envelope_from_row(row: PgRow) -> Result<KernelEventEnvelope> {
    let row = RunEventRow {
        run_id: row
            .try_get("run_id")
            .map_err(|error| database_error("failed to decode run event run_id", error))?,
        seq: row
            .try_get("seq")
            .map_err(|error| database_error("failed to decode run event seq", error))?,
        ordinal: row
            .try_get("ordinal")
            .map_err(|error| database_error("failed to decode run event ordinal", error))?,
        event_id: row
            .try_get("event_id")
            .map_err(|error| database_error("failed to decode run event event_id", error))?,
        event_schema_id: row
            .try_get("event_schema_id")
            .map_err(|error| database_error("failed to decode run event event_schema_id", error))?,
        spec_hash: row
            .try_get("spec_hash")
            .map_err(|error| database_error("failed to decode run event spec_hash", error))?,
        commit_key: row
            .try_get("commit_key")
            .map_err(|error| database_error("failed to decode run event commit_key", error))?,
        logical_key: row
            .try_get("logical_key")
            .map_err(|error| database_error("failed to decode run event logical_key", error))?,
        payload_hash: row
            .try_get("payload_hash")
            .map_err(|error| database_error("failed to decode run event payload_hash", error))?,
        payload_canonical_json: row.try_get("payload_canonical_json").map_err(|error| {
            database_error("failed to decode run event payload_canonical_json", error)
        })?,
    };
    let payload_value: Value =
        serde_json::from_slice(&row.payload_canonical_json).map_err(|error| {
            PostgresStoreError::Corruption(format!(
                "persisted payload canonical bytes were not JSON: {error}"
            ))
        })?;
    let payload = payload_from_json_value(&payload_value)?;
    let canonical = payload_canonical_bytes(&payload)?;
    if canonical.as_bytes() != row.payload_canonical_json.as_slice() {
        return Err(PostgresStoreError::Corruption(
            "run event payload bytes are not canonical for decoded payload".to_owned(),
        ));
    }
    let ordinal = u32::try_from(row.ordinal).map_err(|_| {
        PostgresStoreError::Corruption("run_events.ordinal contained a negative integer".into())
    })?;

    Ok(KernelEventEnvelope::from_persisted_record(
        PersistedKernelEventRecord {
            event_id: parse_identity::<mfm_ids::EventId>(&row.event_id)?,
            event_schema_id: parse_identity::<SchemaId>(&row.event_schema_id)?,
            run_id: parse_identity::<RunId>(&row.run_id)?,
            seq: StreamSeq::new(i64_to_positive_u64(row.seq, "run_events.seq")?)?,
            ordinal: CommitOrdinal::new(ordinal),
            spec_hash: parse_identity::<mfm_ids::SpecHash>(&row.spec_hash)?,
            commit_key: CommitKey::new(row.commit_key)?,
            logical_key: LogicalEventKey::new(row.logical_key)?,
            payload_hash: parse_identity::<ContentDigest>(&row.payload_hash)?,
            payload,
        },
    )?)
}

struct RunEventRow {
    run_id: String,
    seq: i64,
    ordinal: i32,
    event_id: String,
    event_schema_id: String,
    spec_hash: String,
    commit_key: String,
    logical_key: String,
    payload_hash: String,
    payload_canonical_json: Vec<u8>,
}

async fn rebuild_projection_snapshot_with_head(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<(ProjectionSnapshot, u64)> {
    let stream = load_run_stream_tx(tx, run_id).await?;
    let head = stream.last().map(|event| event.seq().as_u64()).unwrap_or(0);
    let snapshot = ProjectionSnapshot::rebuild_from_run_stream(&stream)?;
    Ok((snapshot, head))
}

fn payload_canonical_bytes(
    payload: &events::KernelEventPayload,
) -> Result<PlainCanonicalJsonBytes> {
    Ok(mfm_store::v1::payload_canonical_json(payload)?)
}

fn payload_json_value(payload: &events::KernelEventPayload) -> Result<Value> {
    let canonical = payload_canonical_bytes(payload)?;
    serde_json::from_slice(canonical.as_bytes()).map_err(|error| {
        PostgresStoreError::Corruption(format!("canonical payload was not JSON: {error}"))
    })
}

fn canonical_json(value: serde_json::Value) -> Result<PlainCanonicalJsonBytes> {
    PlainCanonicalJsonBytes::from_json_str(&value.to_string()).map_err(|error| {
        PostgresStoreError::Corruption(format!(
            "failed to canonicalize Postgres authority JSON: {error}"
        ))
    })
}

fn derive_commit_id(
    run_id: &RunId,
    seq: StreamSeq,
    commit_key: &CommitKey,
    commit_purpose: &str,
    prepared_authority_hash: &str,
) -> Result<String> {
    let digest = canonical_json(serde_json::json!({
        "commit_key": commit_key.as_str(),
        "commit_purpose": commit_purpose,
        "domain": "mfm.commit.id.v1",
        "prepared_authority_hash": prepared_authority_hash,
        "run_id": run_id.as_str(),
        "seq": seq.as_u64(),
    }))?
    .content_digest();
    Ok(format!("mfm.commit.id.v1:{}", digest.as_str()))
}

fn is_unique_logical_key(key: &LogicalEventKey) -> bool {
    !key.as_str().starts_with("attempt:")
}

fn next_seq_from_head(head: u64) -> Result<StreamSeq> {
    let next = head.checked_add(1).ok_or(StoreError::SequenceOverflow)?;
    Ok(StreamSeq::new(next)?)
}

fn u64_to_i64(value: u64, field: &'static str) -> Result<i64> {
    i64::try_from(value).map_err(|_| {
        PostgresStoreError::Corruption(format!("{field} exceeded PostgreSQL bigint range"))
    })
}

fn i64_to_nonnegative_u64(value: i64, field: &'static str) -> Result<u64> {
    u64::try_from(value)
        .map_err(|_| PostgresStoreError::Corruption(format!("{field} contained a negative bigint")))
}

fn i64_to_positive_u64(value: i64, field: &'static str) -> Result<u64> {
    let value = i64_to_nonnegative_u64(value, field)?;
    if value == 0 {
        return Err(PostgresStoreError::Corruption(format!(
            "{field} contained a non-positive sequence"
        )));
    }
    Ok(value)
}

fn i32_to_u32(value: i32, field: &'static str) -> Result<u32> {
    u32::try_from(value).map_err(|_| {
        PostgresStoreError::Corruption(format!("{field} contained a negative integer"))
    })
}

fn parse_optional_identity<T>(value: Option<String>) -> Result<Option<T>>
where
    T: FromStr<Err = IdentityError>,
{
    Ok(value.as_deref().map(parse_identity).transpose()?)
}

#[cfg(all(test, feature = "parity-tests"))]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use mfm_canonical::sha256_digest_bytes;
    use mfm_events::v1::{self as events, ArtifactRole, KernelEventPayload};
    use mfm_ids::{
        AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion,
        CellId, ContentDigest, DigestAlgorithm, DigestBytes, LoweringVersion, NodeId, RunId,
        SchemaId, ScopeId, SemanticTypeId, SpecHash, SpecVersion, StateKind, StateVersion,
    };
    use mfm_manual_auth::{
        manual_authorization_proof_schema_id, ManualAuthorizationSignatureBytes,
        ManualResolutionAuthorizationProof, ManualResolutionAuthorizationSignature,
        ManualResolutionBlockReason, ManualResolutionEvidenceRef, ManualResolutionPrefixAuthority,
        ManualResolutionProofAuthority, VerifiedManualResolutionForPrefix,
    };
    use mfm_spec::v1::{
        self as spec, CanonicalizerIdentity, ManualResolutionEvidenceSpec, MediaType,
        ResourceNamespace, SagaPolicySpec, ValueLineageRef,
    };
    use mfm_store::v1::{
        ArtifactEvidenceRef, AttemptStatus, AttemptTerminal, CellTerminalProjection,
        CommitArtifactEvidenceSet, CommitKey, CommitOutcome, CommitPreconditions, ManualResolution,
        PreparedCommit, PreparedCommitPlan, RequiredRunState, ResourceLaneKey, Retention,
        RunAdmission, RunState, SagaEngagementReason, SagaTerminal, SagaTerminalProof,
        SideEffectPhase, SideEffectProgress, SideEffectTerminal, StateAttemptStarted, StoreError,
        StreamSeq,
    };
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use sqlx::AssertSqlSafe;

    use super::*;

    static SCHEMA_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_schema() -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        let counter = SCHEMA_COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("run_store_{}_{}_{}", std::process::id(), nanos, counter)
    }

    async fn test_store() -> (PostgresRunStore, String) {
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
        let admin_pool = PgPool::connect(&database_url)
            .await
            .expect("connect postgres");

        let schema = unique_schema();
        // The schema name is generated from process/time/counter digits and never comes from user
        // input; dynamic DDL is required because PostgreSQL does not parameterize identifiers.
        sqlx::raw_sql(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
            .execute(&admin_pool)
            .await
            .expect("create schema");
        let options = PgConnectOptions::from_str(&database_url)
            .expect("postgres URL")
            .options([("search_path", schema.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("connect schema-scoped postgres");
        crate::schema::migrate_pool(&pool)
            .await
            .expect("migrate schema");
        crate::schema::validate_pool(&pool)
            .await
            .expect("validate schema");
        let store = PostgresRunStore { pool };
        (store, schema)
    }

    async fn drop_schema(store: &PostgresRunStore, schema: &str) {
        store.pool.close().await;
        let database_url =
            std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for parity tests");
        let admin_pool = PgPool::connect(&database_url)
            .await
            .expect("connect postgres");
        sqlx::raw_sql(AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS {schema} CASCADE"
        )))
        .execute(&admin_pool)
        .await
        .expect("drop schema");
    }

    fn digest_bytes(byte: u8) -> DigestBytes {
        DigestBytes::from_array([byte; 32])
    }

    fn content_digest(byte: u8) -> ContentDigest {
        ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(&test_artifact_bytes(byte)),
        )
    }

    fn spec_hash(byte: u8) -> SpecHash {
        SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
    }

    fn run_id(byte: u8) -> RunId {
        RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
    }

    fn artifact_id(byte: u8) -> ArtifactId {
        let digest = content_digest(byte);
        ArtifactId::from_digest(digest.algorithm(), *digest.digest())
    }

    fn test_artifact_bytes(byte: u8) -> Vec<u8> {
        vec![byte; 128]
    }

    fn test_artifact_bytes_for_digest(digest: &ContentDigest) -> Option<Vec<u8>> {
        (u8::MIN..=u8::MAX).map(test_artifact_bytes).find(|bytes| {
            ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
                == *digest
        })
    }

    fn test_prepared_artifact_bytes(
        evidence: &ArtifactEvidenceRef,
    ) -> mfm_store::v1::Result<PreparedArtifactBytes> {
        let bytes = test_artifact_bytes_for_digest(&evidence.digest).ok_or_else(|| {
            StoreError::ArtifactEvidenceMismatch {
                artifact_id: evidence.artifact_id.clone(),
                field: "bytes",
            }
        })?;
        PreparedArtifactBytes::new(bytes, evidence.clone())
    }

    fn node_id(byte: u8) -> NodeId {
        NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
    }

    fn attempt_id(byte: u8) -> AttemptId {
        AttemptId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
    }

    fn cell_id(byte: u8) -> CellId {
        CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
    }

    fn scope_id(byte: u8) -> ScopeId {
        ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
    }

    fn schema_id(name: &str, byte: u8) -> SchemaId {
        SchemaId::new(name, "1", DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
            .expect("schema id")
    }

    fn semantic_id(name: &str, byte: u8) -> SemanticTypeId {
        SemanticTypeId::new(
            "mfm.test",
            name,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(byte),
        )
        .expect("semantic id")
    }

    fn state_kind(byte: u8) -> StateKind {
        StateKind::new(
            "mfm.test",
            "state",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(byte),
        )
        .expect("state kind")
    }

    fn media_type(value: &str) -> MediaType {
        MediaType::new(value).expect("media type")
    }

    fn run_admitted(run_id: RunId) -> KernelEventPayload {
        run_admitted_with_saga_policy(run_id, &SagaPolicySpec::NoSideEffects)
    }

    fn run_admitted_with_saga_policy(
        run_id: RunId,
        saga_policy: &SagaPolicySpec,
    ) -> KernelEventPayload {
        let spec_artifact = spec_artifact_ref();
        let certificate_artifact = certificate_artifact_ref();
        KernelEventPayload::RunAdmitted(Box::new(events::RunAdmitted {
            run_id,
            entry_point: entry_point_launch_evidence(),
            spec_hash: spec_hash(1),
            spec_artifact: run_artifact_ref(&spec_artifact),
            certificate_artifact: run_artifact_ref(&certificate_artifact),
            config_artifacts: Vec::new(),
            spec_version: SpecVersion::new("mfm.typed.execution_spec.v1").expect("spec version"),
            lowering_version: LoweringVersion::new("mfm.typed.lowering.v1")
                .expect("lowering version"),
            public_output_schema_id: schema_id("mfm.test.public_output", 3),
            saga_policy_digest: saga_policy
                .saga_policy_digest()
                .expect("saga policy digest"),
            descriptor_identities: Vec::new(),
            runner_executables: Vec::new(),
            adapter_executables: Vec::new(),
            admitted_binding_digest: content_digest(9),
            canonicalizer_identity: CanonicalizerIdentity::new("mfm.jcs.v1")
                .expect("canonicalizer"),
            seed_cells: Vec::new(),
        }))
    }

    fn entry_point_launch_evidence() -> events::EntryPointLaunchEvidence {
        events::EntryPointLaunchEvidence {
            resolved_op_id: events::EntryPointOpId::new("mfm.test:portfolio_snapshot:1")
                .expect("entry-point op id"),
            entry_point_registry_digest: content_digest(30),
        }
    }

    fn state_attempt_started() -> KernelEventPayload {
        KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
            spec_hash: spec_hash(1),
            node_id: node_id(20),
            attempt_id: attempt_id(23),
            attempt_no: 1,
            state_kind: state_kind(12),
            state_version: StateVersion::new("mfm.test.state.v1").expect("state version"),
        })
    }

    fn state_attempt_completed() -> KernelEventPayload {
        KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
            spec_hash: spec_hash(1),
            node_id: node_id(20),
            attempt_id: attempt_id(23),
            output_cell_id: cell_id(21),
        })
    }

    fn state_attempt_interrupted() -> KernelEventPayload {
        KernelEventPayload::StateAttemptInterrupted(events::StateAttemptInterrupted {
            spec_hash: spec_hash(1),
            node_id: node_id(20),
            attempt_id: attempt_id(23),
        })
    }

    fn cell_produced(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
        KernelEventPayload::CellProduced(events::CellProduced {
            spec_hash: spec_hash(1),
            node_id: node_id(20),
            cell_id: cell_id(21),
            scope_id: scope_id(22),
            attempt_id: attempt_id(23),
            semantic_type_id: semantic_id("position", 24),
            schema_id: schema_id("mfm.test.position", 25),
            value_lineage: ValueLineageRef {
                lineage_digest: content_digest(26),
            },
            artifact_id,
            content_digest: digest,
            producer_state_kind: None,
            producer_state_version: None,
        })
    }

    fn fact_recorded(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
        KernelEventPayload::FactRecorded(events::FactRecorded {
            spec_hash: spec_hash(1),
            node_id: node_id(30),
            attempt_id: attempt_id(31),
            capability_kind: CapabilityKind::new(
                "mfm.test",
                "fact",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(32),
            )
            .expect("capability kind"),
            capability_version: CapabilityVersion::new("mfm.test.fact.v1")
                .expect("capability version"),
            adapter_kind: AdapterKind::new(
                "mfm.test",
                "adapter",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(33),
            )
            .expect("adapter kind"),
            adapter_version: AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version"),
            request_schema_id: schema_id("mfm.test.fact_request", 34),
            request_hash: content_digest(35),
            response_schema_id: schema_id("mfm.test.fact_response", 36),
            response_hash: digest,
            fact_key: events::FactKey::new("fact-key-1").expect("fact key"),
            artifact_id,
        })
    }

    fn fact_attempt_started() -> KernelEventPayload {
        KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
            spec_hash: spec_hash(1),
            node_id: node_id(30),
            attempt_id: attempt_id(31),
            attempt_no: 1,
            state_kind: state_kind(30),
            state_version: StateVersion::new("mfm.test.fact_state.v1").expect("state version"),
        })
    }

    fn run_completed(run_id: RunId, outcome: events::RunCompletionOutcome) -> KernelEventPayload {
        KernelEventPayload::RunCompleted(events::RunCompleted {
            run_id,
            spec_hash: spec_hash(1),
            outcome,
        })
    }

    fn manual_resolution_recorded(
        verified: &VerifiedManualResolutionForPrefix,
    ) -> KernelEventPayload {
        let claim = verified.claim();
        let authorization = verified.authorization();
        KernelEventPayload::ManualResolutionRecorded(events::ManualResolutionRecorded {
            run_id: claim.run_id.clone(),
            spec_hash: claim.spec_hash.clone(),
            outcome: claim.outcome,
            evidence_schema_id: claim.evidence.schema_id.clone(),
            evidence_hash: claim.evidence.content_hash.clone(),
            evidence_artifact_id: claim.evidence.artifact_id.clone(),
            authorization_schema_id: authorization.schema_id.clone(),
            authorization_hash: authorization.content_hash.clone(),
            authorization_artifact_id: authorization.artifact_id.clone(),
            note: Some(events::ManualResolutionNote::new("reviewed evidence").expect("note")),
        })
    }

    fn manual_resolution_artifacts(
        verified: &VerifiedManualResolutionForPrefix,
    ) -> Vec<ArtifactEvidenceRef> {
        let claim = verified.claim();
        let authorization = verified.authorization();
        vec![
            ArtifactEvidenceRef {
                artifact_id: claim.evidence.artifact_id.clone(),
                digest: claim.evidence.content_hash.clone(),
                byte_len: 128,
                media_type: media_type("application/json"),
                schema_id: Some(claim.evidence.schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: None,
                producer_seed_id: None,
                artifact_role: ArtifactRole::ManualResolutionEvidence,
            },
            ArtifactEvidenceRef {
                artifact_id: authorization.artifact_id.clone(),
                digest: authorization.content_hash.clone(),
                byte_len: verified.proof_bytes().len() as u64,
                media_type: media_type("application/json"),
                schema_id: Some(authorization.schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: None,
                producer_seed_id: None,
                artifact_role: ArtifactRole::ManualResolutionAuthorization,
            },
        ]
    }

    fn manual_resolution_prepared_artifact_bytes(
        verified: &VerifiedManualResolutionForPrefix,
    ) -> mfm_store::v1::Result<Vec<PreparedArtifactBytes>> {
        let artifacts = manual_resolution_artifacts(verified);
        Ok(vec![
            test_prepared_artifact_bytes(&artifacts[0])?,
            PreparedArtifactBytes::new(verified.proof_bytes().to_vec(), artifacts[1].clone())?,
        ])
    }

    fn manual_saga_policy(byte: u8) -> SagaPolicySpec {
        SagaPolicySpec::ManualResolution {
            manual: ManualResolutionEvidenceSpec {
                evidence_schema: schema_id("mfm.test.manual_evidence", byte + 1),
                authorization: manual_authorization(byte),
            },
        }
    }

    fn manual_authorization(byte: u8) -> spec::ManualResolutionAuthorizationSpec {
        spec::ManualResolutionAuthorizationSpec {
            verifier_id: spec::ManualAuthorizationVerifierId::new(format!(
                "mfm.test.manual.verifier.{byte}"
            ))
            .expect("verifier id"),
            signing_scheme: spec::ManualSigningSchemeSpec::new(
                "mfm.manual_resolution.digest_signature.v1",
            )
            .expect("signing scheme"),
            authority: spec::OperatorAuthoritySnapshotSpec {
                authority_id: spec::OperatorAuthorityId::new(format!(
                    "mfm.test.manual.authority.{byte}"
                ))
                .expect("authority id"),
                operators: vec![spec::OperatorAuthorityMemberSpec {
                    operator_id: spec::OperatorId::new(format!("operator.{byte}"))
                        .expect("operator id"),
                    public_identity: spec::OperatorPublicIdentity::new(
                        "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
                    )
                    .expect("operator public identity"),
                }],
            },
            quorum: spec::ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
        }
    }

    fn verified_manual_resolution_for_seq(
        run_id: &RunId,
        expected_next_seq: u64,
        byte: u8,
    ) -> VerifiedManualResolutionForPrefix {
        let SagaPolicySpec::ManualResolution { manual } = manual_saga_policy(byte) else {
            unreachable!("manual_saga_policy builds manual policy")
        };
        let prefix = ManualResolutionPrefixAuthority::new(
            run_id.clone(),
            spec_hash(1),
            expected_next_seq,
            content_digest(byte + 3),
            ManualResolutionBlockReason::PolicyManualResolution,
            content_digest(byte + 4),
            manual.clone(),
        )
        .expect("manual prefix authority");
        let evidence = ManualResolutionEvidenceRef {
            schema_id: manual.evidence_schema.clone(),
            content_hash: content_digest(byte + 1),
            artifact_id: artifact_id(byte + 1),
        };
        let claim = prefix
            .authorization_claim(
                events::ManualResolutionOutcome::ConfirmRemediated,
                evidence.clone(),
            )
            .expect("manual authorization claim");
        let proof = signed_manual_resolution_proof(&manual.authorization, claim.clone());
        let proof_bytes = proof
            .canonical_json()
            .expect("manual proof canonical json")
            .to_vec();
        let authorization = manual_authorization_ref(&proof_bytes);
        ManualResolutionProofAuthority::new(
            prefix,
            claim.outcome,
            evidence,
            authorization,
            proof_bytes,
        )
        .and_then(ManualResolutionProofAuthority::verify)
        .expect("verified manual resolution")
    }

    fn signed_manual_resolution_proof(
        policy: &spec::ManualResolutionAuthorizationSpec,
        claim: mfm_manual_auth::ManualResolutionAuthorizationClaim,
    ) -> ManualResolutionAuthorizationProof {
        let operator = policy.authority.operators[0].clone();
        let claim_digest = claim.digest().expect("manual claim digest");
        ManualResolutionAuthorizationProof {
            verifier_id: policy.verifier_id.clone(),
            signing_scheme: policy.signing_scheme.clone(),
            claim,
            signatures: vec![ManualResolutionAuthorizationSignature {
                operator_id: operator.operator_id,
                public_identity: operator.public_identity,
                signature: ManualAuthorizationSignatureBytes::new(sign_manual_claim_digest(
                    &test_manual_signing_key(),
                    claim_digest.digest().as_bytes(),
                ))
                .expect("manual signature bytes"),
            }],
        }
    }

    fn manual_authorization_ref(proof_bytes: &[u8]) -> ManualResolutionEvidenceRef {
        let proof = ManualResolutionAuthorizationProof::from_json_slice(proof_bytes)
            .expect("manual authorization proof");
        let content_hash = proof.content_digest().expect("manual proof content digest");
        ManualResolutionEvidenceRef {
            schema_id: manual_authorization_proof_schema_id().expect("manual authorization schema"),
            artifact_id: ArtifactId::from_digest(content_hash.algorithm(), *content_hash.digest()),
            content_hash,
        }
    }

    fn test_manual_signing_key() -> k256::ecdsa::SigningKey {
        let mut key_bytes = [0u8; 32];
        key_bytes[31] = 1;
        let secret_key = k256::SecretKey::from_slice(&key_bytes).expect("test key");
        k256::ecdsa::SigningKey::from(&secret_key)
    }

    fn sign_manual_claim_digest(
        signing_key: &k256::ecdsa::SigningKey,
        digest: &[u8; 32],
    ) -> Vec<u8> {
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(digest)
            .expect("manual signature");
        let mut signature_bytes = signature.to_bytes().to_vec();
        signature_bytes.push(u8::from(recovery_id.is_y_odd()));
        signature_bytes
    }

    fn saga_preconditions(run_id: &RunId, policy: SagaPolicySpec) -> CommitPreconditions {
        CommitPreconditions {
            saga_admit_token: Some(
                mfm_store::v1::SagaAdmitToken::new(run_id.clone(), spec_hash(1), policy)
                    .expect("saga admit token"),
            ),
            ..CommitPreconditions::default()
        }
    }

    fn side_effect_ledger_key() -> events::SideEffectLedgerKey {
        events::SideEffectLedgerKey::new("ledger-key-1").expect("ledger key")
    }

    fn side_effect_ledger_purpose() -> events::SideEffectLedgerPurpose {
        events::SideEffectLedgerPurpose::Forward
    }

    fn side_effect_attempt_started() -> KernelEventPayload {
        KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
            spec_hash: spec_hash(1),
            node_id: node_id(70),
            attempt_id: attempt_id(72),
            attempt_no: 1,
            state_kind: state_kind(70),
            state_version: StateVersion::new("mfm.test.side_effect_state.v1")
                .expect("state version"),
        })
    }

    fn side_effect_intent(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
        KernelEventPayload::SideEffectIntentPersisted(events::side_effect::IntentPersisted {
            spec_hash: spec_hash(1),
            node_id: node_id(70),
            scope_id: scope_id(71),
            attempt_id: attempt_id(72),
            ledger_key: side_effect_ledger_key(),
            ledger_purpose: side_effect_ledger_purpose(),
            invocation_epoch: 1,
            intent_schema_id: schema_id("mfm.test.side_effect_intent", 70),
            intent_hash: digest,
            intent_artifact_id: artifact_id,
            idempotency_input_schema_id: schema_id("mfm.test.idempotency_input", 73),
            idempotency_input_hash: content_digest(74),
            idempotency_key: events::IdempotencyKeyRef::new("idem-key-1").expect("idempotency key"),
            capability_kind: CapabilityKind::new(
                "mfm.test",
                "side_effect",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(75),
            )
            .expect("capability kind"),
            capability_version: CapabilityVersion::new("mfm.test.side_effect.v1")
                .expect("capability version"),
            adapter_kind: AdapterKind::new(
                "mfm.test",
                "adapter",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(76),
            )
            .expect("adapter kind"),
            adapter_version: AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version"),
        })
    }

    fn side_effect_claim() -> KernelEventPayload {
        KernelEventPayload::SideEffectClaimed(events::side_effect::Claimed {
            spec_hash: spec_hash(1),
            node_id: node_id(70),
            attempt_id: attempt_id(72),
            ledger_key: side_effect_ledger_key(),
            ledger_purpose: side_effect_ledger_purpose(),
            claim_owner: events::RunnerInvocationId::new("owner-1").expect("claim owner"),
            invocation_epoch: 1,
            claim_generation: 1,
            claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1")
                .expect("token"),
        })
    }

    fn side_effect_prepared() -> KernelEventPayload {
        KernelEventPayload::SideEffectInvocationPrepared(events::side_effect::InvocationPrepared {
            spec_hash: spec_hash(1),
            node_id: node_id(70),
            attempt_id: attempt_id(72),
            ledger_key: side_effect_ledger_key(),
            ledger_purpose: side_effect_ledger_purpose(),
            invocation_epoch: 1,
            claim_generation: 1,
            claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1")
                .expect("token"),
            resource_key: None,
            prepared_artifact_id: None,
            prepared_hash: None,
        })
    }

    fn resource_namespace() -> ResourceNamespace {
        ResourceNamespace::new("mfm.test.account_nonce").expect("resource namespace")
    }

    fn resource_key(value: &str, schema_byte: u8) -> events::ResourceKeyEvidence {
        events::ResourceKeyEvidence {
            namespace: resource_namespace(),
            key_schema_id: schema_id("mfm.test.resource_key", schema_byte),
            key: events::ResourceKey::new(value).expect("resource key"),
        }
    }

    fn resource_lane_key(value: &str) -> ResourceLaneKey {
        ResourceLaneKey::from_evidence(&resource_key(value, 201))
    }

    fn resource_lane_claim_intent(resource_key: events::ResourceKeyEvidence) -> KernelEventPayload {
        KernelEventPayload::ResourceLaneClaimIntent(events::ResourceLaneClaimIntent {
            spec_hash: spec_hash(1),
            node_id: node_id(70),
            attempt_id: attempt_id(72),
            ledger_key: side_effect_ledger_key(),
            ledger_purpose: side_effect_ledger_purpose(),
            invocation_epoch: 1,
            resource_key,
            requirement_digest: content_digest(210),
            resolved_by_capability_impl: events::RunnerFactoryId::new("mfm.test.postgres.runner")
                .expect("runner factory"),
        })
    }

    fn side_effect_started() -> KernelEventPayload {
        KernelEventPayload::SideEffectInvocationStarted(events::side_effect::InvocationStarted {
            spec_hash: spec_hash(1),
            node_id: node_id(70),
            attempt_id: attempt_id(72),
            ledger_key: side_effect_ledger_key(),
            ledger_purpose: side_effect_ledger_purpose(),
            invocation_epoch: 1,
            claim_owner: events::RunnerInvocationId::new("owner-1").expect("claim owner"),
            claim_generation: 1,
            claim_fencing_token: events::side_effect::ClaimFencingToken::new("token-1")
                .expect("token"),
        })
    }

    fn unknown_schema() -> SchemaId {
        schema_id("mfm.test.submission_unknown", 83)
    }

    fn submission_schema() -> SchemaId {
        schema_id("mfm.test.submission", 77)
    }

    fn side_effect_submission_unknown(
        artifact_id: ArtifactId,
        digest: ContentDigest,
    ) -> KernelEventPayload {
        KernelEventPayload::SideEffectSubmissionUnknown(events::side_effect::SubmissionUnknown {
            spec_hash: spec_hash(1),
            node_id: node_id(70),
            attempt_id: attempt_id(72),
            ledger_key: side_effect_ledger_key(),
            ledger_purpose: side_effect_ledger_purpose(),
            invocation_epoch: 1,
            evidence_schema_id: unknown_schema(),
            evidence_hash: digest,
            evidence_artifact_id: artifact_id,
        })
    }

    fn side_effect_submission_observed(
        artifact_id: ArtifactId,
        digest: ContentDigest,
    ) -> KernelEventPayload {
        KernelEventPayload::SideEffectSubmissionObserved(events::side_effect::SubmissionObserved {
            spec_hash: spec_hash(1),
            node_id: node_id(70),
            attempt_id: attempt_id(72),
            ledger_key: side_effect_ledger_key(),
            ledger_purpose: side_effect_ledger_purpose(),
            invocation_epoch: 1,
            submission_schema_id: submission_schema(),
            submission_hash: digest,
            submission_artifact_id: artifact_id,
        })
    }

    fn side_effect_ambiguous(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
        KernelEventPayload::SideEffectAmbiguous(events::side_effect::Ambiguous {
            spec_hash: spec_hash(1),
            node_id: node_id(70),
            attempt_id: attempt_id(72),
            ledger_key: side_effect_ledger_key(),
            ledger_purpose: side_effect_ledger_purpose(),
            invocation_epoch: 1,
            ambiguity_code: events::AmbiguityCode::new("ambiguous").expect("ambiguity code"),
            evidence_schema_id: schema_id("mfm.test.ambiguity", 84),
            evidence_hash: digest,
            evidence_artifact_id: artifact_id,
        })
    }

    fn side_effect_attempt_failed() -> KernelEventPayload {
        KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
            spec_hash: spec_hash(1),
            node_id: node_id(70),
            attempt_id: attempt_id(72),
            retryable: false,
            error: events::MfmErrorInfo {
                code: events::ErrorCode::new("side_effect_ambiguous").expect("error code"),
                category: events::ErrorCategory::SideEffect,
                retryable: false,
                safe_message: "side-effect outcome is ambiguous".to_owned(),
                public_details: None,
                diagnostic_ref: None,
            },
        })
    }

    fn store_artifact_ref(
        artifact_id: ArtifactId,
        digest: ContentDigest,
        role: ArtifactRole,
    ) -> ArtifactEvidenceRef {
        let (schema_id, semantic_type_id, producer_node_id) = match role {
            ArtifactRole::StateOutput => (
                Some(schema_id("mfm.test.position", 25)),
                Some(semantic_id("position", 24)),
                Some(node_id(20)),
            ),
            ArtifactRole::FactResponse => (
                Some(schema_id("mfm.test.fact_response", 36)),
                None,
                Some(node_id(30)),
            ),
            _ => (None, None, None),
        };
        ArtifactEvidenceRef {
            artifact_id,
            digest,
            byte_len: 128,
            media_type: media_type("application/json"),
            schema_id,
            semantic_type_id,
            producer_node_id,
            producer_seed_id: None,
            artifact_role: role,
        }
    }

    fn retention_refs_appended(
        run_id: RunId,
        artifact_id: ArtifactId,
        digest: ContentDigest,
        role: ArtifactRole,
    ) -> KernelEventPayload {
        retention_refs_appended_with_reason(
            run_id,
            artifact_id,
            digest,
            role,
            events::RetentionReason::RuntimeEvidence,
        )
    }

    fn retention_refs_appended_with_reason(
        run_id: RunId,
        artifact_id: ArtifactId,
        digest: ContentDigest,
        role: ArtifactRole,
        reason: events::RetentionReason,
    ) -> KernelEventPayload {
        KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
            run_id,
            spec_hash: spec_hash(1),
            refs: vec![events::RetentionRef {
                artifact_id,
                role,
                content_digest: digest,
            }],
            reason,
        })
    }

    fn side_effect_artifact_ref(
        artifact_id: ArtifactId,
        digest: ContentDigest,
        schema_id: SchemaId,
        role: ArtifactRole,
    ) -> ArtifactEvidenceRef {
        ArtifactEvidenceRef {
            artifact_id,
            digest,
            byte_len: 128,
            media_type: media_type("application/json"),
            schema_id: Some(schema_id),
            semantic_type_id: None,
            producer_node_id: Some(node_id(70)),
            producer_seed_id: None,
            artifact_role: role,
        }
    }

    fn spec_artifact_ref() -> ArtifactEvidenceRef {
        ArtifactEvidenceRef {
            artifact_id: artifact_id(2),
            digest: content_digest(2),
            byte_len: 128,
            media_type: media_type("application/vnd.mfm.typed-execution-spec+json;version=1"),
            schema_id: None,
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: ArtifactRole::TypedExecutionSpec,
        }
    }

    fn certificate_artifact_ref() -> ArtifactEvidenceRef {
        ArtifactEvidenceRef {
            artifact_id: artifact_id(4),
            digest: content_digest(4),
            byte_len: 128,
            media_type: media_type("application/vnd.mfm.typed-spec-certificate+json;version=1"),
            schema_id: None,
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: ArtifactRole::TypedSpecCertificate,
        }
    }

    fn run_artifact_ref(artifact: &ArtifactEvidenceRef) -> events::RunArtifactEvidenceRef {
        events::RunArtifactEvidenceRef {
            artifact_id: artifact.artifact_id.clone(),
            role: artifact.artifact_role,
            schema_id: artifact.schema_id.clone(),
            semantic_type_id: artifact.semantic_type_id.clone(),
            content_digest: artifact.digest.clone(),
            byte_len: artifact.byte_len,
            media_type: artifact.media_type.clone(),
        }
    }

    fn request(
        run_id: RunId,
        seq: u64,
        key: &str,
        payloads: Vec<KernelEventPayload>,
    ) -> mfm_store::v1::CommitRequest {
        mfm_store::v1::CommitRequest::from_payloads(
            run_id,
            StreamSeq::new(seq).expect("seq"),
            CommitKey::new(key).expect("commit key"),
            payloads,
            Vec::new(),
            CommitPreconditions::default(),
        )
        .expect("typed commit request")
    }

    async fn append_prepared(
        store: &PostgresRunStore,
        mut request: mfm_store::v1::CommitRequest,
        artifacts: Vec<ArtifactEvidenceRef>,
    ) -> Result<CommitOutcome> {
        if request.required_artifacts().is_empty() && !artifacts.is_empty() {
            request = request.with_required_artifacts(artifacts.clone());
        }
        let plan = test_prepared_commit_plan(request, artifacts)?;
        store
            .append_prepared_commit_bundle(test_prepared_commit_bundle(plan)?)
            .await
    }

    fn test_prepared_commit_bundle(
        plan: PreparedCommitPlan,
    ) -> mfm_store::v1::Result<PreparedCommitBundle> {
        let artifact_bytes = plan
            .admitted_artifacts()
            .iter()
            .map(test_prepared_artifact_bytes)
            .collect::<mfm_store::v1::Result<Vec<_>>>()?;
        PreparedCommitBundle::new(plan, artifact_bytes, Vec::new())
    }

    fn test_prepared_commit_plan(
        request: mfm_store::v1::CommitRequest,
        artifacts: Vec<ArtifactEvidenceRef>,
    ) -> mfm_store::v1::Result<PreparedCommitPlan> {
        let artifact_set =
            CommitArtifactEvidenceSet::new(request.required_artifacts().to_vec(), artifacts)?;
        let payloads = request.payloads();
        if payloads
            .iter()
            .all(|payload| matches!(payload, KernelEventPayload::RunAdmitted(_)))
        {
            let mut preconditions = request.preconditions().clone();
            preconditions.required_run_state = RequiredRunState::Absent;
            let request = request.with_preconditions(preconditions);
            return Ok(PreparedCommit::<RunAdmission>::new(request, artifact_set)?.into());
        }
        if payloads
            .iter()
            .all(|payload| matches!(payload, KernelEventPayload::StateAttemptStarted(_)))
        {
            let mut preconditions = request.preconditions().clone();
            preconditions.required_run_state = RequiredRunState::NotCompleted;
            let request = request.with_preconditions(preconditions);
            return Ok(PreparedCommit::<StateAttemptStarted>::new(request, artifact_set)?.into());
        }
        if payloads.iter().any(test_is_run_completed_payload) {
            return Ok(PreparedCommit::<AttemptTerminal>::new(request, artifact_set)?.into());
        }
        if payloads.iter().any(test_is_side_effect_terminal_payload) {
            return Ok(PreparedCommit::<SideEffectTerminal>::new(request, artifact_set)?.into());
        }
        if payloads
            .iter()
            .any(|payload| payload.side_effect_ref().is_some())
        {
            return Ok(PreparedCommit::<SideEffectProgress>::new(request, artifact_set)?.into());
        }
        if payloads.iter().any(test_is_retention_payload) {
            return Ok(PreparedCommit::<Retention>::new(request, artifact_set)?.into());
        }
        Ok(PreparedCommit::<AttemptTerminal>::new(request, artifact_set)?.into())
    }

    fn test_is_retention_payload(payload: &KernelEventPayload) -> bool {
        matches!(
            payload,
            KernelEventPayload::RetentionRefsAppended(_)
                | KernelEventPayload::RetentionManifestProjected(_)
        )
    }

    fn test_is_run_completed_payload(payload: &KernelEventPayload) -> bool {
        matches!(payload, KernelEventPayload::RunCompleted(_))
    }

    fn test_is_side_effect_terminal_payload(payload: &KernelEventPayload) -> bool {
        matches!(
            payload,
            KernelEventPayload::SideEffectNotSubmittedProven(_)
                | KernelEventPayload::SideEffectSubmissionObserved(_)
                | KernelEventPayload::SideEffectSubmissionUnknown(_)
                | KernelEventPayload::SideEffectReceiptObserved(_)
                | KernelEventPayload::SideEffectConfirmationObserved(_)
                | KernelEventPayload::SideEffectAmbiguous(_)
                | KernelEventPayload::SideEffectFailed(_)
                | KernelEventPayload::ResourceLaneReleaseIntent(_)
                | KernelEventPayload::ResourceLaneReleased(_)
        )
    }

    async fn append_run_start(
        store: &PostgresRunStore,
        run_id: &RunId,
        commit_key: &str,
    ) -> Result<CommitOutcome> {
        append_prepared(
            store,
            request(
                run_id.clone(),
                1,
                commit_key,
                vec![run_admitted(run_id.clone())],
            ),
            vec![spec_artifact_ref(), certificate_artifact_ref()],
        )
        .await
    }

    async fn append_resource_lane_attempt_start(
        store: &PostgresRunStore,
        run_id: &RunId,
        commit_key: &str,
    ) -> Result<CommitOutcome> {
        append_prepared(
            store,
            request(
                run_id.clone(),
                2,
                commit_key,
                vec![side_effect_attempt_started()],
            ),
            Vec::new(),
        )
        .await
    }

    async fn append_resource_lane_prepare(
        store: &PostgresRunStore,
        run_id: &RunId,
        commit_key: &str,
        lane_value: &str,
        artifact_byte: u8,
    ) -> Result<CommitOutcome> {
        let intent_artifact = artifact_id(artifact_byte);
        let intent_digest = content_digest(artifact_byte);
        append_prepared(
            store,
            request(
                run_id.clone(),
                3,
                commit_key,
                vec![
                    side_effect_intent(intent_artifact.clone(), intent_digest.clone()),
                    side_effect_claim(),
                    resource_lane_claim_intent(resource_key(lane_value, 201)),
                ],
            ),
            vec![side_effect_artifact_ref(
                intent_artifact,
                intent_digest,
                schema_id("mfm.test.side_effect_intent", 70),
                ArtifactRole::SideEffectIntent,
            )],
        )
        .await
    }

    fn assert_resource_lane_blocked(
        outcome: CommitOutcome,
        expected_lane_key: &ResourceLaneKey,
        expected_holder_run: &RunId,
    ) {
        let CommitOutcome::ResourceLaneClaimBlocked(block) = outcome else {
            panic!("expected typed resource lane block, got {outcome:?}");
        };
        assert_eq!(&block.lane_key, expected_lane_key);
        assert_eq!(&block.holder.run_id, expected_holder_run);
        assert_eq!(block.holder.ledger_key, side_effect_ledger_key());
    }

    fn assert_corruption(error: PostgresStoreError, expected: &str) {
        let PostgresStoreError::Corruption(message) = error else {
            panic!("expected corruption error, got {error:?}");
        };
        assert!(
            message.contains(expected),
            "expected corruption containing {expected:?}, got {message:?}"
        );
    }

    #[tokio::test]
    async fn prepared_commit_idempotency_fingerprint_includes_admitted_artifacts() {
        let (store, schema) = test_store().await;
        let run = run_id(120);
        let artifact = artifact_id(121);
        let digest = content_digest(121);
        let evidence =
            store_artifact_ref(artifact.clone(), digest.clone(), ArtifactRole::StateOutput);
        let mut conflicting_evidence = evidence.clone();
        conflicting_evidence.media_type = media_type("application/octet-stream");
        append_prepared(
            &store,
            request(run.clone(), 1, "run-start", vec![run_admitted(run.clone())]),
            vec![spec_artifact_ref(), certificate_artifact_ref()],
        )
        .await
        .expect("run start");
        let request = mfm_store::v1::CommitRequest::from_payloads(
            run.clone(),
            store.expected_next_seq(&run).await.expect("next seq"),
            CommitKey::new("prepared-fingerprint").expect("commit key"),
            vec![retention_refs_appended(
                run.clone(),
                artifact,
                digest,
                ArtifactRole::StateOutput,
            )],
            vec![evidence.clone()],
            CommitPreconditions {
                required_run_state: RequiredRunState::Started,
                ..CommitPreconditions::default()
            },
        )
        .expect("typed commit request");

        append_prepared(&store, request.clone(), vec![evidence])
            .await
            .expect("append initial prepared commit");
        let retry = request.with_expected_next_seq(StreamSeq::new(99).expect("stale seq"));
        let error = append_prepared(&store, retry, vec![conflicting_evidence])
            .await
            .expect_err("same request with different admitted evidence is not idempotent");
        assert!(matches!(
            error,
            PostgresStoreError::Store(StoreError::CommitConflict { .. })
        ));

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn run_commit_log_sort_keys_use_v1_rfc_tuple() {
        let (store, schema) = test_store().await;
        let run = run_id(125);
        append_run_start(&store, &run, "sort-key-run-start")
            .await
            .expect("run start");
        append_resource_lane_attempt_start(&store, &run, "sort-key-attempt-start")
            .await
            .expect("attempt start");

        let rows = sqlx::query(
            "SELECT l.seq, l.commit_sort_key, c.commit_key, c.commit_id, c.commit_batch_hash \
             FROM run_commit_log l \
             INNER JOIN commits c ON c.commit_id = l.commit_id \
             WHERE l.run_id = $1 \
             ORDER BY l.seq",
        )
        .bind(run.as_str())
        .fetch_all(&store.pool)
        .await
        .expect("query commit sort keys");
        assert_eq!(rows.len(), 2);
        let mut previous: Option<Vec<u8>> = None;
        for row in rows {
            let seq: i64 = row.try_get("seq").expect("seq");
            let commit_sort_key: Vec<u8> = row.try_get("commit_sort_key").expect("commit_sort_key");
            let commit_key = CommitKey::new(row.try_get::<String, _>("commit_key").expect("key"))
                .expect("commit key");
            let commit_id: String = row.try_get("commit_id").expect("commit id");
            let commit_batch_hash: String =
                row.try_get("commit_batch_hash").expect("commit batch hash");
            assert_eq!(commit_sort_key.len(), 32);
            assert_eq!(commit_sort_key[0], 1);
            assert_ne!(commit_sort_key, vec![0; 32]);
            assert_eq!(
                commit_sort_key,
                run_commit_sort_key(
                    &run,
                    StreamSeq::new(
                        i64_to_positive_u64(seq, "run_commit_log.seq").expect("positive seq"),
                    )
                    .expect("stream seq"),
                    &commit_key,
                    &commit_id,
                    &commit_batch_hash,
                )
                .expect("expected sort key")
            );
            if let Some(previous) = previous.replace(commit_sort_key.clone()) {
                assert_ne!(previous, commit_sort_key);
            }
        }

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn strict_load_rejects_corrupt_commit_canonical_bytes() {
        let (store, schema) = test_store().await;
        let run = run_id(126);
        append_run_start(&store, &run, "strict-canonical-run-start")
            .await
            .expect("run start");

        sqlx::query("ALTER TABLE commits DISABLE TRIGGER commits_no_update")
            .execute(&store.pool)
            .await
            .expect("disable commit mutation guard");
        sqlx::query(
            "UPDATE commits SET commit_batch_canonical_json = $1 WHERE run_id = $2 AND seq = 1",
        )
        .bind(b"{}".as_slice())
        .bind(run.as_str())
        .execute(&store.pool)
        .await
        .expect("corrupt commit batch canonical bytes");

        let error = store
            .load_run_stream(&run)
            .await
            .expect_err("strict load rejects corrupt commit batch bytes");
        assert_corruption(error, "commit batch hash does not match");

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn strict_load_rejects_corrupt_run_commit_log_sort_key() {
        let (store, schema) = test_store().await;
        let run = run_id(127);
        append_run_start(&store, &run, "strict-log-run-start")
            .await
            .expect("run start");

        sqlx::query("ALTER TABLE run_commit_log DISABLE TRIGGER run_commit_log_no_update")
            .execute(&store.pool)
            .await
            .expect("disable commit log mutation guard");
        sqlx::query("UPDATE run_commit_log SET commit_sort_key = $1 WHERE run_id = $2 AND seq = 1")
            .bind(vec![1_u8; 32])
            .bind(run.as_str())
            .execute(&store.pool)
            .await
            .expect("corrupt commit sort key");

        let error = store
            .load_run_stream(&run)
            .await
            .expect_err("strict load rejects corrupt run_commit_log sort key");
        assert_corruption(error, "run_commit_log sort key does not match");

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn strict_load_rejects_corrupt_resource_lane_transition_hash() {
        let (store, schema) = test_store().await;
        let run = run_id(128);
        append_run_start(&store, &run, "strict-lane-run-start")
            .await
            .expect("run start");
        append_resource_lane_attempt_start(&store, &run, "strict-lane-attempt-start")
            .await
            .expect("attempt start");
        append_resource_lane_prepare(&store, &run, "strict-lane-prepare", "wallet-strict", 129)
            .await
            .expect("resource lane prepare");

        sqlx::query(
            "ALTER TABLE resource_lane_transitions DISABLE TRIGGER \
             resource_lane_transitions_no_update",
        )
        .execute(&store.pool)
        .await
        .expect("disable resource lane transition mutation guard");
        sqlx::query("UPDATE resource_lane_transitions SET transition_hash = $1")
            .bind("tampered-transition-hash")
            .execute(&store.pool)
            .await
            .expect("corrupt resource lane transition hash");

        let error = store
            .load_run_stream(&run)
            .await
            .expect_err("strict load rejects corrupt lane transition hash");
        assert_corruption(error, "resource lane transition hash mismatch");

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn read_model_maintenance_builds_validates_and_reports_watermark() {
        let (store, schema) = test_store().await;
        let run = run_id(130);
        append_run_start(&store, &run, "maintenance-run-start")
            .await
            .expect("run start");
        append_resource_lane_attempt_start(&store, &run, "maintenance-attempt-start")
            .await
            .expect("attempt start");
        let maintenance = PostgresMaintenance {
            pool: store.pool.clone(),
        };

        let current = maintenance
            .validate_read_models(PROJECTION_VERSION)
            .await
            .expect("validate current projection");
        assert_eq!(current.checked_commits, 2);
        assert_eq!(current.missing_rows, 0);
        assert_eq!(current.drift_rows, 0);

        let built = maintenance
            .build_projection_version(
                "mfm.run_observation.test.v2",
                ReadModelBuildMode::NewVersion,
            )
            .await
            .expect("build new projection version");
        assert_eq!(built.inserted_rows, 2);
        assert_eq!(built.verified_rows, 0);
        let rebuilt = maintenance
            .build_projection_version(
                "mfm.run_observation.test.v2",
                ReadModelBuildMode::MissingOnly,
            )
            .await
            .expect("verify rebuilt projection version");
        assert_eq!(rebuilt.inserted_rows, 0);
        assert_eq!(rebuilt.verified_rows, 2);
        let validated = maintenance
            .validate_read_models("mfm.run_observation.test.v2")
            .await
            .expect("validate rebuilt projection");
        assert_eq!(validated.checked_commits, 2);
        assert_eq!(validated.missing_rows, 0);
        assert_eq!(validated.drift_rows, 0);

        let watermark = maintenance
            .read_model_high_watermark()
            .await
            .expect("read watermark");
        assert_eq!(watermark.commit_log_rows, 2);
        assert_eq!(watermark.summary_rows, 4);
        assert!(watermark.high_append_xid.is_some());
        assert!(watermark.high_commit_id.is_some());
        assert_eq!(
            watermark.high_commit_sort_key.as_ref().map(Vec::len),
            Some(32)
        );
        let drift = maintenance
            .read_model_drift_report()
            .await
            .expect("drift report");
        assert!(drift.findings.is_empty());

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn read_model_missing_only_rebuild_repairs_missing_rows() {
        let (store, schema) = test_store().await;
        let run = run_id(131);
        append_run_start(&store, &run, "missing-read-model-run-start")
            .await
            .expect("run start");
        append_resource_lane_attempt_start(&store, &run, "missing-read-model-attempt-start")
            .await
            .expect("attempt start");
        let maintenance = PostgresMaintenance {
            pool: store.pool.clone(),
        };

        sqlx::query(
            "ALTER TABLE run_observation_change_summaries DISABLE TRIGGER \
             run_observation_change_summaries_no_update",
        )
        .execute(&store.pool)
        .await
        .expect("disable read-model mutation guard");
        sqlx::query(
            "DELETE FROM run_observation_change_summaries \
             WHERE projection_version = $1 AND summary_kind = $2 AND run_id = $3 AND head_seq = 2",
        )
        .bind(PROJECTION_VERSION)
        .bind(RUN_OBSERVATION_SUMMARY_KIND)
        .bind(run.as_str())
        .execute(&store.pool)
        .await
        .expect("delete read-model row");
        sqlx::query(
            "ALTER TABLE run_observation_change_summaries ENABLE TRIGGER \
             run_observation_change_summaries_no_update",
        )
        .execute(&store.pool)
        .await
        .expect("reenable read-model mutation guard");

        let missing = maintenance
            .validate_read_models(PROJECTION_VERSION)
            .await
            .expect("validate missing row");
        assert_eq!(missing.missing_rows, 1);
        assert_eq!(missing.drift_rows, 0);
        let repaired = maintenance
            .build_projection_version(PROJECTION_VERSION, ReadModelBuildMode::MissingOnly)
            .await
            .expect("repair missing row");
        assert_eq!(repaired.inserted_rows, 1);
        let valid = maintenance
            .validate_read_models(PROJECTION_VERSION)
            .await
            .expect("validate repaired projection");
        assert_eq!(valid.missing_rows, 0);
        assert_eq!(valid.drift_rows, 0);

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn read_model_validation_detects_drift() {
        let (store, schema) = test_store().await;
        let run = run_id(132);
        append_run_start(&store, &run, "drift-read-model-run-start")
            .await
            .expect("run start");
        let maintenance = PostgresMaintenance {
            pool: store.pool.clone(),
        };

        sqlx::query(
            "ALTER TABLE run_observation_change_summaries DISABLE TRIGGER \
             run_observation_change_summaries_no_update",
        )
        .execute(&store.pool)
        .await
        .expect("disable read-model mutation guard");
        sqlx::query(
            "UPDATE run_observation_change_summaries \
             SET summary_row_hash = 'tampered-summary-hash' \
             WHERE projection_version = $1 AND summary_kind = $2 AND run_id = $3",
        )
        .bind(PROJECTION_VERSION)
        .bind(RUN_OBSERVATION_SUMMARY_KIND)
        .bind(run.as_str())
        .execute(&store.pool)
        .await
        .expect("tamper read-model hash");

        let validation = maintenance
            .validate_read_models(PROJECTION_VERSION)
            .await
            .expect("validate drifted projection");
        assert_eq!(validation.drift_rows, 1);
        let drift = maintenance
            .read_model_drift_report()
            .await
            .expect("drift report");
        assert_eq!(drift.findings.len(), 1);
        assert_eq!(drift.findings[0].kind, ReadModelDriftKind::Mismatched);
        let error = maintenance
            .build_projection_version(PROJECTION_VERSION, ReadModelBuildMode::MissingOnly)
            .await
            .expect_err("rebuild fails closed on drift");
        assert_corruption(error, "read-model drift");

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn reseed_store_epoch_expires_existing_observation_cursors() {
        let (store, schema) = test_store().await;
        let run = run_id(133);
        append_run_start(&store, &run, "epoch-run-start")
            .await
            .expect("run start");
        let page = store
            .read_run_observations(RunObservationQuery::new(None, 10, 0))
            .await
            .expect("read observations before reseed");
        let old_cursor = page.next_cursor;
        let maintenance = PostgresMaintenance {
            pool: store.pool.clone(),
        };
        let new_epoch = maintenance
            .reseed_store_epoch()
            .await
            .expect("reseed store epoch");
        assert!(new_epoch.starts_with("mfm.store.epoch.v1:"));

        let error = store
            .read_run_observations(RunObservationQuery::new(Some(old_cursor), 10, 0))
            .await
            .expect_err("old cursor expires after epoch reseed");
        assert!(matches!(
            error,
            PostgresStoreError::Store(StoreError::CursorExpired)
        ));

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn artifact_authority_accepts_distinct_evidence_for_same_artifact_id() {
        let (store, schema) = test_store().await;
        let run = run_id(122);
        let artifact = artifact_id(123);
        let digest = content_digest(123);
        let first_evidence =
            store_artifact_ref(artifact.clone(), digest.clone(), ArtifactRole::StateOutput);
        let mut second_evidence = first_evidence.clone();
        second_evidence.schema_id = Some(schema_id("mfm.test.alternate_position", 124));
        assert_ne!(
            first_evidence.evidence_hash().expect("first evidence hash"),
            second_evidence
                .evidence_hash()
                .expect("second evidence hash")
        );

        append_prepared(
            &store,
            request(
                run.clone(),
                1,
                "same-id-run-start",
                vec![run_admitted(run.clone())],
            ),
            vec![spec_artifact_ref(), certificate_artifact_ref()],
        )
        .await
        .expect("run start");

        let first_request = mfm_store::v1::CommitRequest::from_payloads(
            run.clone(),
            store.expected_next_seq(&run).await.expect("next seq"),
            CommitKey::new("same-id-first-evidence").expect("commit key"),
            vec![retention_refs_appended(
                run.clone(),
                artifact.clone(),
                digest.clone(),
                ArtifactRole::StateOutput,
            )],
            vec![first_evidence.clone()],
            CommitPreconditions {
                required_run_state: RequiredRunState::Started,
                ..CommitPreconditions::default()
            },
        )
        .expect("typed first request");
        append_prepared(&store, first_request, vec![first_evidence])
            .await
            .expect("append first evidence");

        let second_request = mfm_store::v1::CommitRequest::from_payloads(
            run.clone(),
            store.expected_next_seq(&run).await.expect("next seq"),
            CommitKey::new("same-id-second-evidence").expect("commit key"),
            vec![retention_refs_appended_with_reason(
                run.clone(),
                artifact,
                digest,
                ArtifactRole::StateOutput,
                events::RetentionReason::PublicOutput,
            )],
            vec![second_evidence.clone()],
            CommitPreconditions {
                required_run_state: RequiredRunState::Started,
                ..CommitPreconditions::default()
            },
        )
        .expect("typed second request");
        append_prepared(&store, second_request, vec![second_evidence])
            .await
            .expect("append second evidence for same artifact id");

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn commit_key_sequence_and_projection_rebuild_contract() {
        let (store, schema) = test_store().await;
        let run = run_id(7);
        let artifact = artifact_id(8);
        let digest = content_digest(8);

        append_prepared(
            &store,
            request(run.clone(), 1, "run-start", vec![run_admitted(run.clone())]),
            vec![spec_artifact_ref(), certificate_artifact_ref()],
        )
        .await
        .expect("run start");
        append_prepared(
            &store,
            request(
                run.clone(),
                2,
                "attempt-start",
                vec![state_attempt_started()],
            ),
            Vec::new(),
        )
        .await
        .expect("attempt start");
        let terminal_request = request(
            run.clone(),
            3,
            "terminal",
            vec![
                cell_produced(artifact.clone(), digest.clone()),
                state_attempt_completed(),
            ],
        );
        let appended = append_prepared(
            &store,
            terminal_request.clone(),
            vec![store_artifact_ref(
                artifact.clone(),
                digest.clone(),
                ArtifactRole::StateOutput,
            )],
        )
        .await
        .expect("terminal commit");
        assert!(matches!(appended, CommitOutcome::Appended(_)));

        let stale_retry =
            terminal_request.with_expected_next_seq(StreamSeq::new(1).expect("stale seq"));
        let idempotent = append_prepared(
            &store,
            stale_retry,
            vec![store_artifact_ref(
                artifact,
                digest,
                ArtifactRole::StateOutput,
            )],
        )
        .await
        .expect("idempotent retry before stale seq");
        assert!(matches!(idempotent, CommitOutcome::Idempotent(_)));
        assert_eq!(
            store.expected_next_seq(&run).await.expect("next seq"),
            StreamSeq::new(4).expect("seq")
        );

        let before = store.projection_snapshot(&run).await.expect("projection");
        assert!(matches!(
            before.cell_terminal(&cell_id(21)),
            Some(CellTerminalProjection::Produced { .. })
        ));
        let stream = store.load_run_stream(&run).await.expect("typed run stream");
        assert_eq!(stream.len(), 4);
        assert_eq!(
            ProjectionSnapshot::rebuild_from_run_stream(&stream).expect("payload rebuild"),
            before
        );

        assert_eq!(
            store
                .projection_snapshot(&run)
                .await
                .expect("stream-authoritative projection rebuilds from events"),
            before
        );

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn interrupted_attempt_projection_rebuilds_from_events() {
        let (store, schema) = test_store().await;
        let run = run_id(17);

        append_prepared(
            &store,
            request(run.clone(), 1, "run-start", vec![run_admitted(run.clone())]),
            vec![spec_artifact_ref(), certificate_artifact_ref()],
        )
        .await
        .expect("run start");
        append_prepared(
            &store,
            request(
                run.clone(),
                2,
                "attempt-start",
                vec![state_attempt_started()],
            ),
            Vec::new(),
        )
        .await
        .expect("attempt start");
        append_prepared(
            &store,
            request(
                run.clone(),
                3,
                "attempt-interrupted",
                vec![state_attempt_interrupted()],
            ),
            Vec::new(),
        )
        .await
        .expect("attempt interrupted");

        let before = store.projection_snapshot(&run).await.expect("projection");
        let attempt = before
            .attempt(&node_id(20), &attempt_id(23))
            .expect("attempt projection");
        assert!(matches!(&attempt.status, AttemptStatus::Interrupted));
        assert!(before.saga_engagement(&run).is_none());

        let rebuilt = store
            .projection_snapshot(&run)
            .await
            .expect("projection rebuilds from events");
        assert_eq!(rebuilt, before);
        assert!(matches!(
            &rebuilt
                .attempt(&node_id(20), &attempt_id(23))
                .expect("stream-derived attempt projection")
                .status,
            AttemptStatus::Interrupted
        ));

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn resource_lane_projection_rebuilds_from_events() {
        let (store, schema) = test_store().await;
        let run = run_id(21);
        let intent_artifact = artifact_id(22);
        let intent_digest = content_digest(22);
        let lane_key = resource_lane_key("wallet-1");

        append_prepared(
            &store,
            request(
                run.clone(),
                1,
                "resource-run-start",
                vec![run_admitted(run.clone())],
            ),
            vec![spec_artifact_ref(), certificate_artifact_ref()],
        )
        .await
        .expect("run start");
        append_prepared(
            &store,
            request(
                run.clone(),
                2,
                "resource-attempt-start",
                vec![side_effect_attempt_started()],
            ),
            Vec::new(),
        )
        .await
        .expect("attempt start");
        append_prepared(
            &store,
            request(
                run.clone(),
                3,
                "resource-prepare",
                vec![
                    side_effect_intent(intent_artifact.clone(), intent_digest.clone()),
                    side_effect_claim(),
                    resource_lane_claim_intent(resource_key("wallet-1", 201)),
                ],
            ),
            vec![side_effect_artifact_ref(
                intent_artifact,
                intent_digest,
                schema_id("mfm.test.side_effect_intent", 70),
                ArtifactRole::SideEffectIntent,
            )],
        )
        .await
        .expect("prepare with resource key");

        let before = store.projection_snapshot(&run).await.expect("projection");
        let lane = before.resource_lane(&lane_key).expect("resource lane");
        assert_eq!(lane.holder.run_id, run);
        assert_eq!(lane.holder.ledger_key, side_effect_ledger_key());

        let stream = store.load_run_stream(&run).await.expect("typed run stream");
        assert_eq!(
            ProjectionSnapshot::rebuild_from_run_stream(&stream).expect("payload rebuild"),
            before
        );

        let peer_run = run_id(24);
        append_prepared(
            &store,
            request(
                peer_run.clone(),
                1,
                "resource-peer-run-start",
                vec![run_admitted(peer_run.clone())],
            ),
            vec![spec_artifact_ref(), certificate_artifact_ref()],
        )
        .await
        .expect("peer run start");
        let peer_before = store
            .projection_snapshot(&peer_run)
            .await
            .expect("peer projection");
        let peer_lane = peer_before
            .resource_lane(&lane_key)
            .expect("peer snapshot includes cross-run lane");
        assert_eq!(&peer_lane.holder.run_id, &run);
        assert_eq!(peer_before.resource_lanes().count(), 1);

        let peer_after_reload = store
            .projection_snapshot(&peer_run)
            .await
            .expect("peer stream-authoritative projection");
        let peer_lane = peer_after_reload
            .resource_lane(&lane_key)
            .expect("peer snapshot includes cross-run lane from stream");
        assert_eq!(&peer_lane.holder.run_id, &run);
        assert_eq!(&peer_lane.holder.ledger_key, &side_effect_ledger_key());
        assert_eq!(peer_after_reload.run_state(&peer_run), RunState::Started);
        assert_eq!(
            store
                .projection_snapshot(&run)
                .await
                .expect("holder stream-authoritative projection"),
            before
        );

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn resource_lane_append_admission_uses_stream_authority() {
        let (store, schema) = test_store().await;
        let holder_run = run_id(25);
        let contender = run_id(26);
        let lane_value = "wallet-admission";
        let lane_key = resource_lane_key(lane_value);

        append_run_start(&store, &holder_run, "admission-holder-run-start")
            .await
            .expect("holder run start");
        append_resource_lane_attempt_start(&store, &holder_run, "admission-holder-attempt-start")
            .await
            .expect("holder attempt start");
        append_resource_lane_prepare(
            &store,
            &holder_run,
            "admission-holder-prepare",
            lane_value,
            28,
        )
        .await
        .expect("holder resource lane prepare");
        let holder_projection = store
            .projection_snapshot(&holder_run)
            .await
            .expect("holder projection");
        assert_eq!(
            holder_projection
                .resource_lane(&lane_key)
                .expect("holder resource lane")
                .holder
                .run_id,
            holder_run
        );

        append_run_start(&store, &contender, "contender-run-start")
            .await
            .expect("contender run start");
        append_resource_lane_attempt_start(&store, &contender, "contender-attempt-start")
            .await
            .expect("contender attempt start");
        let stream_before = store
            .load_run_stream(&contender)
            .await
            .expect("contender stream before conflict");
        let outcome =
            append_resource_lane_prepare(&store, &contender, "contender-prepare", lane_value, 30)
                .await
                .expect("resource lane stream authority blocks contender");
        assert_resource_lane_blocked(outcome, &lane_key, &holder_run);
        assert_eq!(
            store
                .load_run_stream(&contender)
                .await
                .expect("contender stream after conflict"),
            stream_before
        );
        assert_eq!(
            store
                .expected_next_seq(&contender)
                .await
                .expect("contender next seq"),
            StreamSeq::new(3).expect("contender prepare seq")
        );

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn saga_projection_rebuilds_from_events() {
        let (store, schema) = test_store().await;
        let run = run_id(41);
        let saga_policy = manual_saga_policy(42);

        append_prepared(
            &store,
            request(
                run.clone(),
                1,
                "saga-run-start",
                vec![run_admitted_with_saga_policy(run.clone(), &saga_policy)],
            ),
            vec![spec_artifact_ref(), certificate_artifact_ref()],
        )
        .await
        .expect("run start");
        append_prepared(
            &store,
            request(
                run.clone(),
                2,
                "saga-side-effect-attempt-start",
                vec![side_effect_attempt_started()],
            ),
            Vec::new(),
        )
        .await
        .expect("side-effect attempt start");
        let intent_artifact = artifact_id(40);
        let intent_digest = content_digest(40);
        append_prepared(
            &store,
            request(
                run.clone(),
                3,
                "saga-side-effect-prepare",
                vec![
                    side_effect_intent(intent_artifact.clone(), intent_digest.clone()),
                    side_effect_claim(),
                    side_effect_prepared(),
                ],
            ),
            vec![side_effect_artifact_ref(
                intent_artifact,
                intent_digest,
                schema_id("mfm.test.side_effect_intent", 70),
                ArtifactRole::SideEffectIntent,
            )],
        )
        .await
        .expect("side-effect prepare");
        let ambiguity_artifact = artifact_id(140);
        let ambiguity_digest = content_digest(140);
        append_prepared(
            &store,
            request(
                run.clone(),
                4,
                "saga-side-effect-ambiguous",
                vec![
                    side_effect_started(),
                    side_effect_ambiguous(ambiguity_artifact.clone(), ambiguity_digest.clone()),
                    side_effect_attempt_failed(),
                ],
            ),
            vec![side_effect_artifact_ref(
                ambiguity_artifact,
                ambiguity_digest,
                schema_id("mfm.test.ambiguity", 84),
                ArtifactRole::AmbiguityEvidence,
            )],
        )
        .await
        .expect("side-effect ambiguous");
        let verified = verified_manual_resolution_for_seq(&run, 5, 42);
        let manual_artifacts = manual_resolution_artifacts(&verified);
        let manual_request = mfm_store::v1::CommitRequest::from_payloads(
            run.clone(),
            StreamSeq::new(5).expect("manual resolution seq"),
            CommitKey::new("saga-manual-resolution").expect("commit key"),
            vec![manual_resolution_recorded(&verified)],
            manual_artifacts.clone(),
            CommitPreconditions {
                required_run_state: RequiredRunState::NotCompleted,
                ..saga_preconditions(&run, saga_policy)
            },
        )
        .expect("manual resolution request");
        let manual_commit = PreparedCommit::<ManualResolution>::new(
            manual_request,
            CommitArtifactEvidenceSet::new(manual_artifacts.clone(), manual_artifacts)
                .expect("manual artifact evidence set"),
            &verified,
        )
        .expect("proof-backed manual resolution prepared commit");
        store
            .append_prepared_commit_bundle(
                PreparedCommitBundle::new(
                    manual_commit.into(),
                    manual_resolution_prepared_artifact_bytes(&verified)
                        .expect("manual artifact bytes"),
                    Vec::new(),
                )
                .expect("manual bundle"),
            )
            .await
            .expect("manual resolution");
        let before = store.projection_snapshot(&run).await.expect("projection");
        let engagement = before.saga_engagement(&run).expect("saga engagement");
        assert!(matches!(
            engagement.reason,
            SagaEngagementReason::ForwardAmbiguous { .. }
        ));
        let manual = before
            .manual_resolution(&run)
            .expect("manual resolution projection");
        assert_eq!(
            manual.outcome,
            events::ManualResolutionOutcome::ConfirmRemediated
        );
        assert_eq!(
            manual
                .note
                .as_ref()
                .map(events::ManualResolutionNote::as_str),
            Some("reviewed evidence")
        );
        assert!(before.run_completion(&run).is_none());

        let stream = store.load_run_stream(&run).await.expect("typed run stream");
        assert_eq!(
            ProjectionSnapshot::rebuild_from_run_stream(&stream).expect("payload rebuild"),
            before
        );

        let terminal_run = run_id(43);
        let terminal_policy = SagaPolicySpec::FailWithoutAcdcClaim;
        append_prepared(
            &store,
            request(
                terminal_run.clone(),
                1,
                "saga-terminal-run-start",
                vec![run_admitted_with_saga_policy(
                    terminal_run.clone(),
                    &terminal_policy,
                )],
            ),
            vec![spec_artifact_ref(), certificate_artifact_ref()],
        )
        .await
        .expect("terminal run start");
        append_prepared(
            &store,
            request(
                terminal_run.clone(),
                2,
                "saga-terminal-side-effect-attempt-start",
                vec![side_effect_attempt_started()],
            ),
            Vec::new(),
        )
        .await
        .expect("terminal side-effect attempt start");
        let terminal_intent_artifact = artifact_id(150);
        let terminal_intent_digest = content_digest(150);
        append_prepared(
            &store,
            request(
                terminal_run.clone(),
                3,
                "saga-terminal-side-effect-prepare",
                vec![
                    side_effect_intent(
                        terminal_intent_artifact.clone(),
                        terminal_intent_digest.clone(),
                    ),
                    side_effect_claim(),
                    side_effect_prepared(),
                ],
            ),
            vec![side_effect_artifact_ref(
                terminal_intent_artifact,
                terminal_intent_digest,
                schema_id("mfm.test.side_effect_intent", 70),
                ArtifactRole::SideEffectIntent,
            )],
        )
        .await
        .expect("terminal side-effect prepare");
        let terminal_ambiguity_artifact = artifact_id(152);
        let terminal_ambiguity_digest = content_digest(152);
        append_prepared(
            &store,
            request(
                terminal_run.clone(),
                4,
                "saga-terminal-side-effect-ambiguous",
                vec![
                    side_effect_started(),
                    side_effect_ambiguous(
                        terminal_ambiguity_artifact.clone(),
                        terminal_ambiguity_digest.clone(),
                    ),
                    side_effect_attempt_failed(),
                ],
            ),
            vec![side_effect_artifact_ref(
                terminal_ambiguity_artifact,
                terminal_ambiguity_digest,
                schema_id("mfm.test.ambiguity", 84),
                ArtifactRole::AmbiguityEvidence,
            )],
        )
        .await
        .expect("terminal side-effect ambiguous");
        let terminal_before_completion = store
            .projection_snapshot(&terminal_run)
            .await
            .expect("terminal projection before completion");
        let terminal_saga =
            terminal_before_completion.derive_saga_projection(&terminal_run, &terminal_policy);
        let terminal_proof = SagaTerminalProof::new(
            &terminal_policy,
            &terminal_saga,
            store
                .expected_next_seq(&terminal_run)
                .await
                .expect("terminal next seq"),
            None,
        )
        .expect("terminal proof");
        let terminal_request = request(
            terminal_run.clone(),
            5,
            "saga-terminal-run-completed",
            vec![run_completed(
                terminal_run.clone(),
                events::RunCompletionOutcome::FailedWithoutAcdcClaim,
            )],
        );
        let terminal_request =
            terminal_request.with_preconditions(saga_preconditions(&terminal_run, terminal_policy));
        let terminal_commit = PreparedCommit::<SagaTerminal>::new(
            terminal_request,
            CommitArtifactEvidenceSet::empty(),
            &terminal_proof,
        )
        .expect("proof-backed terminal commit");
        store
            .append_prepared_commit_bundle(
                test_prepared_commit_bundle(terminal_commit.into()).expect("terminal bundle"),
            )
            .await
            .expect("terminal run completed");

        let terminal_before = store
            .projection_snapshot(&terminal_run)
            .await
            .expect("terminal projection");
        assert!(matches!(
            terminal_before
                .run_completion(&terminal_run)
                .map(|projection| &projection.outcome),
            Some(events::RunCompletionOutcome::FailedWithoutAcdcClaim)
        ));
        let terminal_stream = store
            .load_run_stream(&terminal_run)
            .await
            .expect("terminal typed run stream");
        assert_eq!(
            ProjectionSnapshot::rebuild_from_run_stream(&terminal_stream)
                .expect("terminal payload rebuild"),
            terminal_before
        );

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn required_artifacts_and_fact_projection_are_atomic() {
        let (store, schema) = test_store().await;
        let run = run_id(10);
        append_prepared(
            &store,
            request(run.clone(), 1, "run-start", vec![run_admitted(run.clone())]),
            vec![spec_artifact_ref(), certificate_artifact_ref()],
        )
        .await
        .expect("run start");
        append_prepared(
            &store,
            request(
                run.clone(),
                2,
                "fact-attempt-start",
                vec![fact_attempt_started()],
            ),
            Vec::new(),
        )
        .await
        .expect("fact attempt start");

        let missing_artifact = artifact_id(11);
        let missing_digest = content_digest(11);
        let missing_ref = store_artifact_ref(
            missing_artifact.clone(),
            missing_digest.clone(),
            ArtifactRole::FactResponse,
        );
        let fact_request = request(
            run.clone(),
            3,
            "fact",
            vec![fact_recorded(
                missing_artifact.clone(),
                missing_digest.clone(),
            )],
        );
        let fact_request = fact_request.with_preconditions(CommitPreconditions {
            required_run_state: RequiredRunState::Started,
            ..CommitPreconditions::default()
        });
        let fact_request = fact_request.with_required_artifacts(vec![missing_ref.clone()]);
        let err = append_prepared(&store, fact_request.clone(), Vec::new())
            .await
            .expect_err("missing fact artifact");
        assert!(matches!(
            err,
            PostgresStoreError::Store(StoreError::MissingArtifact { .. })
        ));
        assert_eq!(
            store.expected_next_seq(&run).await.expect("next seq"),
            StreamSeq::new(3).expect("seq")
        );

        append_prepared(&store, fact_request, vec![missing_ref])
            .await
            .expect("fact commit after artifact");
        let projection = store.projection_snapshot(&run).await.expect("projection");
        assert!(projection
            .fact(
                &node_id(30),
                &attempt_id(31),
                &events::FactKey::new("fact-key-1").unwrap()
            )
            .is_some());

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn side_effect_unknown_recovery_updates_submission_result_slot() {
        let (store, schema) = test_store().await;
        let run = run_id(13);

        append_prepared(
            &store,
            request(run.clone(), 1, "run-start", vec![run_admitted(run.clone())]),
            vec![spec_artifact_ref(), certificate_artifact_ref()],
        )
        .await
        .expect("run start");

        let intent_artifact = artifact_id(14);
        let intent_digest = content_digest(14);
        append_prepared(
            &store,
            request(
                run.clone(),
                2,
                "sidefx-attempt-start",
                vec![side_effect_attempt_started()],
            ),
            Vec::new(),
        )
        .await
        .expect("attempt start");
        append_prepared(
            &store,
            request(
                run.clone(),
                3,
                "sidefx-prepare",
                vec![
                    side_effect_intent(intent_artifact.clone(), intent_digest.clone()),
                    side_effect_claim(),
                    side_effect_prepared(),
                ],
            ),
            vec![side_effect_artifact_ref(
                intent_artifact,
                intent_digest,
                schema_id("mfm.test.side_effect_intent", 70),
                ArtifactRole::SideEffectIntent,
            )],
        )
        .await
        .expect("prepare");
        append_prepared(
            &store,
            request(
                run.clone(),
                4,
                "sidefx-started",
                vec![side_effect_started()],
            ),
            Vec::new(),
        )
        .await
        .expect("started");

        let unknown_artifact = artifact_id(16);
        let unknown_digest = content_digest(16);
        let unknown = append_prepared(
            &store,
            request(
                run.clone(),
                5,
                "sidefx-submission-unknown",
                vec![side_effect_submission_unknown(
                    unknown_artifact.clone(),
                    unknown_digest.clone(),
                )],
            ),
            vec![side_effect_artifact_ref(
                unknown_artifact,
                unknown_digest,
                unknown_schema(),
                ArtifactRole::SubmissionUnknownEvidence,
            )],
        )
        .await
        .expect("submission unknown")
        .committed_batch()
        .expect("submission unknown committed batch")
        .clone();
        let submission_result_key = format!(
            "sidefx:forward:{}:invocation:1:submission_result",
            side_effect_ledger_key()
        );
        assert_eq!(
            unknown.events()[0].logical_key().as_str(),
            submission_result_key
        );

        let submission_artifact = artifact_id(18);
        let submission_digest = content_digest(18);
        let observed = append_prepared(
            &store,
            request(
                run.clone(),
                6,
                "sidefx-submission-observed-after-unknown",
                vec![side_effect_submission_observed(
                    submission_artifact.clone(),
                    submission_digest.clone(),
                )],
            ),
            vec![side_effect_artifact_ref(
                submission_artifact,
                submission_digest,
                submission_schema(),
                ArtifactRole::Submission,
            )],
        )
        .await
        .expect("submission observed recovery")
        .committed_batch()
        .expect("submission observed committed batch")
        .clone();
        assert_eq!(
            observed.events()[0].logical_key().as_str(),
            submission_result_key
        );

        let projection = store.projection_snapshot(&run).await.expect("projection");
        let side_effect = projection
            .side_effect(&side_effect_ledger_key())
            .expect("side-effect projection");
        assert!(matches!(
            side_effect.phase,
            SideEffectPhase::SubmissionObserved {
                invocation_epoch: 1
            }
        ));

        let stored_payload_hash = sqlx::query_scalar::<_, String>(
            "SELECT payload_hash FROM run_events \
             WHERE run_id = $1 AND logical_key = $2",
        )
        .bind(run.as_str())
        .bind(submission_result_key.as_str())
        .fetch_one(&store.pool)
        .await
        .expect("run event payload row");
        assert_eq!(
            stored_payload_hash,
            observed.events()[0].payload_hash().as_str()
        );

        drop_schema(&store, &schema).await;
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn artifact_role_contract_postgres_tag_roundtrip_uses_events_contract() {
        for role in events::ArtifactRole::ALL {
            assert_eq!(
                decode_artifact_role_tag(role.as_str()).expect("role tag parses"),
                *role
            );
        }

        assert!(matches!(
            decode_artifact_role_tag("unknown_artifact_role"),
            Err(PostgresStoreError::Store(StoreError::Identity(message)))
                if message.contains("unknown artifact role unknown_artifact_role")
        ));
    }
}
