use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use mfm_events::v1 as events;
use mfm_ids::{
    ArtifactId, ContentDigest, IdentityError, NodeId, RunId, SchemaId, SeedId, SemanticTypeId,
};
use mfm_spec::v1::MediaType;
use mfm_store::v1::codec::{
    attempt_projection_json, cell_projection_json, fact_projection_json,
    manual_resolution_projection_json, parse_identity, public_output_projection_json,
    resource_lane_projection_json, run_completion_projection_json, run_state_str,
    saga_engagement_projection_json, side_effect_projection_json,
};
use mfm_store::v1::{
    build_prepared_commit_plan_batch, payload_from_json_value, prepared_commit_plan_fingerprint,
    stage_prepared_commit_plan, ArtifactEvidenceRef, AsyncStoreFuture, AsyncTypedRunEventStore,
    CodecError, CommitKey, CommitOrdinal, CommitOutcome, KernelEventEnvelope, LogicalEventKey,
    PersistedKernelEventRecord, PreparedCommitPlan, ProjectionSnapshot, ProjectionSnapshotParts,
    ResourceLaneKey, ResourceLaneProjection, StoreError, StoreErrorInspection, StreamSeq,
    TypedCommitBase,
};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};

use crate::schema::{connect_pool, validate_pool};

/// Error returned by the PostgreSQL typed run event store.
#[derive(Debug)]
pub enum PostgresTypedStoreError {
    /// Typed store contract validation failed.
    Store(StoreError),
    /// PostgreSQL operation failed.
    Database(&'static str),
    /// Persisted typed store rows are corrupt.
    Corruption(String),
}

impl fmt::Display for PostgresTypedStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(f, "{error}"),
            Self::Database(message) => write!(f, "postgres typed store error: {message}"),
            Self::Corruption(message) => write!(f, "postgres typed store corruption: {message}"),
        }
    }
}

impl std::error::Error for PostgresTypedStoreError {}

impl StoreErrorInspection for PostgresTypedStoreError {
    fn as_store_error(&self) -> Option<&StoreError> {
        match self {
            Self::Store(error) => Some(error),
            Self::Database(_) | Self::Corruption(_) => None,
        }
    }
}

impl From<StoreError> for PostgresTypedStoreError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<IdentityError> for PostgresTypedStoreError {
    fn from(error: IdentityError) -> Self {
        Self::Store(StoreError::from(error))
    }
}

impl From<mfm_events::EventError> for PostgresTypedStoreError {
    fn from(error: mfm_events::EventError) -> Self {
        Self::Store(StoreError::from(error))
    }
}

impl From<mfm_spec::SpecError> for PostgresTypedStoreError {
    fn from(error: mfm_spec::SpecError) -> Self {
        Self::Store(StoreError::Identity(error.to_string()))
    }
}

impl From<CodecError> for PostgresTypedStoreError {
    fn from(error: CodecError) -> Self {
        let (CodecError::Field(message) | CodecError::Identity(message)) = error;
        Self::Corruption(message)
    }
}

pub(crate) type Result<T> = std::result::Result<T, PostgresTypedStoreError>;

fn database_error(context: &'static str, _error: sqlx::Error) -> PostgresTypedStoreError {
    PostgresTypedStoreError::Database(context)
}

/// PostgreSQL-backed typed run event store.
///
/// This is the certified typed storage surface for run events, commit keys, artifact evidence, and
/// derived projections.
#[derive(Clone)]
pub struct PostgresTypedRunEventStore {
    pub(crate) pool: PgPool,
}

impl PostgresTypedRunEventStore {
    /// Connects to PostgreSQL, validates the typed schema, and returns a typed store.
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = connect_pool(database_url).await?;
        validate_pool(&pool).await?;
        Ok(Self { pool })
    }

    /// Connects using the `DATABASE_URL` environment variable.
    pub async fn connect_env() -> Result<Self> {
        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| PostgresTypedStoreError::Database("missing DATABASE_URL"))?;
        Self::connect(&database_url).await
    }

    /// Returns the next store-owned stream sequence for a run.
    pub async fn expected_next_seq(&self, run_id: &RunId) -> Result<StreamSeq> {
        let head = read_head(&self.pool, run_id).await?;
        next_seq_from_head(head)
    }

    /// Atomically admits artifact evidence and appends one typed run commit.
    pub async fn append_prepared_commit_plan(
        &self,
        plan: PreparedCommitPlan,
    ) -> Result<CommitOutcome> {
        let request = plan.request();
        let fingerprint = prepared_commit_plan_fingerprint(&plan)?;
        let fingerprint_text = fingerprint.as_digest().as_str().to_owned();

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| database_error("failed to start transaction", error))?;

        if let Some((stored_fingerprint, stored_seq)) =
            read_commit_key(&mut tx, request.run_id(), request.commit_key().as_str()).await?
        {
            if stored_fingerprint == fingerprint_text {
                let batch = build_prepared_commit_plan_batch(&plan, stored_seq)?;
                tx.commit().await.map_err(|_| {
                    PostgresTypedStoreError::Database("failed to commit transaction")
                })?;
                return Ok(CommitOutcome::Idempotent(batch));
            }
            return Err(StoreError::CommitConflict {
                commit_key: request.commit_key().clone(),
            }
            .into());
        }

        ensure_run_head(&mut tx, request.run_id()).await?;
        let head = read_head_for_update(&mut tx, request.run_id()).await?;

        // A same-run transaction may have inserted the key while this transaction waited for the
        // run-head lock. Re-check before sequence validation while preserving the required initial
        // commit-key lookup order.
        if let Some((stored_fingerprint, stored_seq)) =
            read_commit_key(&mut tx, request.run_id(), request.commit_key().as_str()).await?
        {
            if stored_fingerprint == fingerprint_text {
                let batch = build_prepared_commit_plan_batch(&plan, stored_seq)?;
                tx.commit().await.map_err(|_| {
                    PostgresTypedStoreError::Database("failed to commit transaction")
                })?;
                return Ok(CommitOutcome::Idempotent(batch));
            }
            return Err(StoreError::CommitConflict {
                commit_key: request.commit_key().clone(),
            }
            .into());
        }

        let mut artifacts = load_artifacts(&mut tx, request.run_id()).await?;
        admit_artifact_evidence(&mut artifacts, plan.admitted_artifacts())?;
        lock_resource_lanes_tx(&mut tx).await?;
        let (run_projection, stream_head) =
            rebuild_projection_snapshot_with_head(&mut tx, request.run_id()).await?;
        if stream_head != head {
            return Err(PostgresTypedStoreError::Corruption(
                "typed run head does not match persisted event stream".to_owned(),
            ));
        }
        let resource_lanes = rebuild_global_resource_lanes_from_events_tx(&mut tx).await?;
        let projections = projection_snapshot_with_resource_lanes(&run_projection, resource_lanes)?;
        let base = TypedCommitBase {
            artifacts,
            logical_keys: load_logical_keys(&mut tx, request.run_id()).await?,
            unique_logical_payloads: load_unique_logical_payloads(&mut tx, request.run_id())
                .await?,
            projections,
            actual_next_seq: next_seq_from_head(head)?,
        };
        let staged = stage_prepared_commit_plan(&base, &plan)?;
        let batch = staged.batch().clone();
        let staged_projections = staged.projections().clone();

        for evidence in plan.admitted_artifacts() {
            admit_artifact_evidence_tx(
                &mut tx,
                request.run_id(),
                request.commit_key(),
                batch.seq(),
                evidence,
            )
            .await?;
        }

        for event in batch.events() {
            let payload_json = canonical_payload_value(event.payload())?;
            let seq = u64_to_i64(event.seq().as_u64(), "typed_run_events.seq")?;
            let ordinal = i32::try_from(event.ordinal().as_u32()).map_err(|_| {
                PostgresTypedStoreError::Corruption("typed_run_events.ordinal overflow".into())
            })?;
            let payload_canonical_byte_len = u64_to_i64(
                event.audit().payload_canonical_byte_len(),
                "typed_run_events.payload_canonical_byte_len",
            )?;
            sqlx::query!(
                "INSERT INTO typed_run_events \
                 (run_id, seq, ordinal, event_id, event_schema_id, spec_hash, commit_key, \
                  logical_key, payload_hash, payload_canonical_byte_len, payload_json) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
                event.run_id().as_str(),
                seq,
                ordinal,
                event.event_id().as_str(),
                event.event_schema_id().as_str(),
                event.spec_hash().as_str(),
                event.commit_key().as_str(),
                event.logical_key().as_str(),
                event.payload_hash().as_str(),
                payload_canonical_byte_len,
                payload_json,
            )
            .execute(&mut *tx)
            .await
            .map_err(|error| database_error("failed to insert typed event", error))?;
        }

        let commit_seq = u64_to_i64(batch.seq().as_u64(), "typed_commit_keys.seq")?;
        let event_count = i32::try_from(batch.events().len())
            .map_err(|_| PostgresTypedStoreError::Corruption("event count overflow".into()))?;
        sqlx::query!(
            "INSERT INTO typed_commit_keys \
             (run_id, commit_key, commit_fingerprint, seq, event_count) \
            VALUES ($1,$2,$3,$4,$5)",
            request.run_id().as_str(),
            request.commit_key().as_str(),
            fingerprint_text,
            commit_seq,
            event_count,
        )
        .execute(&mut *tx)
        .await
        .map_err(|error| database_error("failed to insert commit key", error))?;

        for event in batch.events() {
            sqlx::query!(
                "INSERT INTO typed_logical_keys (run_id, logical_key) VALUES ($1,$2) \
                 ON CONFLICT (run_id, logical_key) DO NOTHING",
                request.run_id().as_str(),
                event.logical_key().as_str(),
            )
            .execute(&mut *tx)
            .await
            .map_err(|error| database_error("failed to insert logical key", error))?;
            if is_unique_logical_key(event.logical_key()) {
                sqlx::query!(
                    "INSERT INTO typed_unique_logical_payloads \
                     (run_id, logical_key, payload_hash) VALUES ($1,$2,$3) \
                     ON CONFLICT (run_id, logical_key) DO UPDATE \
                    SET payload_hash = EXCLUDED.payload_hash",
                    request.run_id().as_str(),
                    event.logical_key().as_str(),
                    event.payload_hash().as_str(),
                )
                .execute(&mut *tx)
                .await
                .map_err(|error| database_error("failed to insert unique logical key", error))?;
            }
        }

        write_projection_tables(&mut tx, request.run_id(), &staged_projections).await?;
        sqlx::query!(
            "UPDATE typed_run_heads SET head_seq = $2 WHERE run_id = $1",
            request.run_id().as_str(),
            u64_to_i64(batch.seq().as_u64(), "typed_run_heads.head_seq")?,
        )
        .execute(&mut *tx)
        .await
        .map_err(|error| database_error("failed to update run head", error))?;

        tx.commit()
            .await
            .map_err(|error| database_error("failed to commit transaction", error))?;
        Ok(CommitOutcome::Appended(batch))
    }

    /// Loads the current projection snapshot from projection tables.
    pub async fn projection_snapshot(&self, run_id: &RunId) -> Result<ProjectionSnapshot> {
        load_projection_snapshot_client(&self.pool, run_id).await
    }

    /// Loads the authoritative typed run stream from persisted event rows.
    pub async fn load_run_stream(&self, run_id: &RunId) -> Result<Vec<KernelEventEnvelope>> {
        load_run_stream_client(&self.pool, run_id).await
    }

    /// Rebuilds projection tables from the authoritative `typed_run_events` stream.
    pub async fn rebuild_projections_from_events(
        &self,
        run_id: &RunId,
    ) -> Result<ProjectionSnapshot> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| database_error("failed to start transaction", error))?;
        ensure_run_head(&mut tx, run_id).await?;
        let head = read_head_for_update(&mut tx, run_id).await?;
        lock_resource_lanes_tx(&mut tx).await?;
        let snapshot = rebuild_projection_snapshot_from_events(&mut tx, run_id).await?;
        let stream_head = projection_stream_head(&mut tx, run_id).await?;
        if stream_head != head {
            return Err(PostgresTypedStoreError::Corruption(
                "typed run head does not match persisted event stream".to_owned(),
            ));
        }
        write_projection_tables(&mut tx, run_id, &snapshot).await?;
        tx.commit()
            .await
            .map_err(|error| database_error("failed to commit transaction", error))?;
        Ok(snapshot)
    }
}

impl AsyncTypedRunEventStore for PostgresTypedRunEventStore {
    type Error = PostgresTypedStoreError;

    fn append_prepared_commit_plan<'a>(
        &'a self,
        plan: PreparedCommitPlan,
    ) -> AsyncStoreFuture<'a, CommitOutcome, Self::Error> {
        Box::pin(async move {
            PostgresTypedRunEventStore::append_prepared_commit_plan(self, plan).await
        })
    }

    fn load_run_stream<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, Vec<KernelEventEnvelope>, Self::Error> {
        Box::pin(async move { PostgresTypedRunEventStore::load_run_stream(self, run_id).await })
    }

    fn expected_next_seq<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, StreamSeq, Self::Error> {
        Box::pin(async move { PostgresTypedRunEventStore::expected_next_seq(self, run_id).await })
    }

    fn status_projection_snapshot<'a>(
        &'a self,
        run_id: &'a RunId,
    ) -> AsyncStoreFuture<'a, ProjectionSnapshot, Self::Error> {
        Box::pin(async move { PostgresTypedRunEventStore::projection_snapshot(self, run_id).await })
    }
}

fn admit_artifact_evidence(
    artifacts: &mut BTreeMap<ArtifactId, ArtifactEvidenceRef>,
    admitted_artifacts: &[ArtifactEvidenceRef],
) -> Result<()> {
    for evidence in admitted_artifacts {
        if let Some(existing) = artifacts.get(&evidence.artifact_id) {
            if existing != evidence {
                return Err(StoreError::ArtifactEvidenceMismatch {
                    artifact_id: evidence.artifact_id.clone(),
                    field: "artifact",
                }
                .into());
            }
            continue;
        }
        artifacts.insert(evidence.artifact_id.clone(), evidence.clone());
    }
    Ok(())
}

async fn admit_artifact_evidence_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    commit_key: &CommitKey,
    seq: StreamSeq,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    let inserted = sqlx::query!(
        "INSERT INTO typed_artifacts \
         (artifact_id, digest, byte_len, media_type, schema_id, semantic_type_id, \
          producer_node_id, producer_seed_id, artifact_role) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) \
         ON CONFLICT (artifact_id) DO NOTHING",
        evidence.artifact_id.as_str(),
        evidence.digest.as_str(),
        u64_to_i64(evidence.byte_len, "typed_artifacts.byte_len")?,
        evidence.media_type.as_str(),
        evidence.schema_id.as_ref().map(SchemaId::as_str),
        evidence
            .semantic_type_id
            .as_ref()
            .map(SemanticTypeId::as_str),
        evidence.producer_node_id.as_ref().map(NodeId::as_str),
        evidence.producer_seed_id.as_ref().map(SeedId::as_str),
        evidence.artifact_role.as_str(),
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert artifact evidence", error))?
    .rows_affected();
    if inserted == 0 {
        let row = sqlx::query!(
            "SELECT digest, byte_len, media_type, schema_id, semantic_type_id, producer_node_id, \
             producer_seed_id, artifact_role FROM typed_artifacts WHERE artifact_id = $1",
            evidence.artifact_id.as_str(),
        )
        .fetch_one(&mut **tx)
        .await
        .map_err(|error| database_error("failed to query artifact evidence", error))?;
        let stored = ArtifactEvidenceParts {
            artifact_id: evidence.artifact_id.clone(),
            digest: row.digest,
            byte_len: row.byte_len,
            media_type: row.media_type,
            schema_id: row.schema_id,
            semantic_type_id: row.semantic_type_id,
            producer_node_id: row.producer_node_id,
            producer_seed_id: row.producer_seed_id,
            artifact_role: row.artifact_role,
        }
        .into_evidence_ref()?;
        if &stored != evidence {
            return Err(StoreError::ArtifactEvidenceMismatch {
                artifact_id: evidence.artifact_id.clone(),
                field: "artifact",
            }
            .into());
        }
    }

    sqlx::query!(
        "INSERT INTO typed_run_artifacts (run_id, artifact_id, commit_key, seq) \
         VALUES ($1,$2,$3,$4) \
         ON CONFLICT (run_id, artifact_id) DO NOTHING",
        run_id.as_str(),
        evidence.artifact_id.as_str(),
        commit_key.as_str(),
        u64_to_i64(seq.as_u64(), "typed_run_artifacts.seq")?,
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to insert run artifact evidence", error))?;

    Ok(())
}

async fn ensure_run_head(tx: &mut Transaction<'_, Postgres>, run_id: &RunId) -> Result<()> {
    sqlx::query!(
        "INSERT INTO typed_run_heads (run_id, head_seq) VALUES ($1, 0) \
         ON CONFLICT (run_id) DO NOTHING",
        run_id.as_str(),
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to ensure typed run head", error))?;
    Ok(())
}

async fn read_head(pool: &PgPool, run_id: &RunId) -> Result<u64> {
    let row = sqlx::query!(
        "SELECT head_seq FROM typed_run_heads WHERE run_id = $1",
        run_id.as_str(),
    )
    .fetch_optional(pool)
    .await
    .map_err(|error| database_error("failed to query typed run head", error))?;
    let Some(row) = row else {
        return Ok(0);
    };
    i64_to_nonnegative_u64(row.head_seq, "typed_run_heads.head_seq")
}

async fn read_head_for_update(tx: &mut Transaction<'_, Postgres>, run_id: &RunId) -> Result<u64> {
    let row = sqlx::query!(
        "SELECT head_seq FROM typed_run_heads WHERE run_id = $1 FOR UPDATE",
        run_id.as_str(),
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| database_error("failed to lock typed run head", error))?;
    i64_to_nonnegative_u64(row.head_seq, "typed_run_heads.head_seq")
}

async fn lock_resource_lanes_tx(tx: &mut Transaction<'_, Postgres>) -> Result<()> {
    // This table lock is the cross-run resource-lane admission mutex. Lane authority is rebuilt
    // from typed_run_events, so the contents of projection rows are not trusted during admission.
    sqlx::query!("LOCK TABLE typed_resource_lane_projection IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut **tx)
        .await
        .map_err(|error| database_error("failed to lock resource lanes", error))?;
    Ok(())
}

async fn read_commit_key(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    commit_key: &str,
) -> Result<Option<(String, StreamSeq)>> {
    let row = sqlx::query!(
        "SELECT commit_fingerprint, seq FROM typed_commit_keys \
         WHERE run_id = $1 AND commit_key = $2",
        run_id.as_str(),
        commit_key,
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| database_error("failed to query commit key", error))?;
    row.map(|row| {
        Ok((
            row.commit_fingerprint,
            StreamSeq::new(i64_to_positive_u64(row.seq, "typed_commit_keys.seq")?)?,
        ))
    })
    .transpose()
}

async fn load_artifacts(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<BTreeMap<ArtifactId, ArtifactEvidenceRef>> {
    let rows = sqlx::query!(
        "SELECT a.artifact_id, a.digest, a.byte_len, a.media_type, a.schema_id, \
         a.semantic_type_id, a.producer_node_id, a.producer_seed_id, a.artifact_role \
         FROM typed_artifacts a \
         INNER JOIN typed_run_artifacts ra ON ra.artifact_id = a.artifact_id \
         WHERE ra.run_id = $1",
        run_id.as_str(),
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load artifact evidence", error))?;
    let mut artifacts = BTreeMap::new();
    for row in rows {
        let artifact_id = parse_identity::<ArtifactId>(&row.artifact_id)?;
        let evidence = ArtifactEvidenceParts {
            artifact_id: artifact_id.clone(),
            digest: row.digest,
            byte_len: row.byte_len,
            media_type: row.media_type,
            schema_id: row.schema_id,
            semantic_type_id: row.semantic_type_id,
            producer_node_id: row.producer_node_id,
            producer_seed_id: row.producer_seed_id,
            artifact_role: row.artifact_role,
        }
        .into_evidence_ref()?;
        artifacts.insert(artifact_id, evidence);
    }
    Ok(artifacts)
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
            byte_len: i64_to_nonnegative_u64(self.byte_len, "typed_artifacts.byte_len")?,
            media_type: MediaType::new(self.media_type)?,
            schema_id: parse_optional_identity(self.schema_id)?,
            semantic_type_id: parse_optional_identity(self.semantic_type_id)?,
            producer_node_id: parse_optional_identity(self.producer_node_id)?,
            producer_seed_id: parse_optional_identity(self.producer_seed_id)?,
            artifact_role: parse_artifact_role(&self.artifact_role)?,
        })
    }
}

fn parse_artifact_role(value: &str) -> Result<events::ArtifactRole> {
    events::ArtifactRole::parse(value)
        .ok_or_else(|| StoreError::Identity(format!("unknown artifact role {value}")).into())
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn artifact_role_contract_postgres_tag_roundtrip_uses_events_contract() {
        for role in events::ArtifactRole::ALL {
            assert_eq!(
                parse_artifact_role(role.as_str()).expect("role tag parses"),
                *role
            );
        }

        assert!(matches!(
            parse_artifact_role("unknown_artifact_role"),
            Err(PostgresTypedStoreError::Store(StoreError::Identity(message)))
                if message.contains("unknown artifact role unknown_artifact_role")
        ));
    }
}

async fn load_logical_keys(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<BTreeSet<(RunId, LogicalEventKey)>> {
    let rows = sqlx::query!(
        "SELECT logical_key FROM typed_logical_keys WHERE run_id = $1",
        run_id.as_str(),
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load logical keys", error))?;
    let mut keys = BTreeSet::new();
    for row in rows {
        keys.insert((run_id.clone(), LogicalEventKey::new(row.logical_key)?));
    }
    Ok(keys)
}

async fn load_unique_logical_payloads(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<BTreeMap<(RunId, LogicalEventKey), ContentDigest>> {
    let rows = sqlx::query!(
        "SELECT logical_key, payload_hash FROM typed_unique_logical_payloads WHERE run_id = $1",
        run_id.as_str(),
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load unique logical payloads", error))?;
    let mut payloads = BTreeMap::new();
    for row in rows {
        payloads.insert(
            (run_id.clone(), LogicalEventKey::new(row.logical_key)?),
            parse_identity::<ContentDigest>(&row.payload_hash)?,
        );
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
    let resource_lanes = rebuild_global_resource_lanes_from_events_tx(tx).await?;
    projection_snapshot_with_resource_lanes(&snapshot, resource_lanes)
}

async fn rebuild_global_resource_lanes_from_events_tx(
    tx: &mut Transaction<'_, Postgres>,
) -> Result<BTreeMap<ResourceLaneKey, ResourceLaneProjection>> {
    let mut resource_lanes = BTreeMap::new();
    for run_id in load_all_stream_run_ids_tx(tx).await? {
        let stream = load_run_stream_tx(tx, &run_id).await?;
        let snapshot = ProjectionSnapshot::rebuild_from_run_stream(&stream)?;
        for (lane_key, projection) in snapshot.resource_lanes() {
            if resource_lanes
                .insert(lane_key.clone(), projection.clone())
                .is_some()
            {
                return Err(PostgresTypedStoreError::Corruption(
                    "authoritative streams contain duplicate active resource lane".to_owned(),
                ));
            }
        }
    }
    Ok(resource_lanes)
}

async fn load_all_stream_run_ids_tx(tx: &mut Transaction<'_, Postgres>) -> Result<Vec<RunId>> {
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT run_id FROM typed_run_heads \
         UNION SELECT DISTINCT run_id FROM typed_run_events \
         ORDER BY run_id",
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load typed stream run ids", error))?;
    rows.into_iter()
        .map(|run_id| parse_identity::<RunId>(&run_id).map_err(Into::into))
        .collect()
}

fn projection_snapshot_with_resource_lanes(
    snapshot: &ProjectionSnapshot,
    resource_lanes: BTreeMap<ResourceLaneKey, ResourceLaneProjection>,
) -> Result<ProjectionSnapshot> {
    ProjectionSnapshot::from_parts(ProjectionSnapshotParts {
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
    })
    .map_err(PostgresTypedStoreError::Store)
}

async fn load_run_stream_client(pool: &PgPool, run_id: &RunId) -> Result<Vec<KernelEventEnvelope>> {
    let rows = sqlx::query_as!(
        TypedRunEventRow,
        "SELECT run_id, seq, ordinal, event_id, event_schema_id, spec_hash, commit_key, \
         logical_key, payload_hash, payload_canonical_byte_len, payload_json \
         FROM typed_run_events WHERE run_id = $1 ORDER BY seq ASC, ordinal ASC",
        run_id.as_str(),
    )
    .fetch_all(pool)
    .await
    .map_err(|error| database_error("failed to load typed run stream", error))?;
    let events = rows
        .into_iter()
        .map(event_envelope_from_row)
        .collect::<Result<Vec<_>>>()?;
    ProjectionSnapshot::validate_run_stream(&events)?;
    Ok(events)
}

async fn load_run_stream_tx(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<Vec<KernelEventEnvelope>> {
    let rows = sqlx::query_as!(
        TypedRunEventRow,
        "SELECT run_id, seq, ordinal, event_id, event_schema_id, spec_hash, commit_key, \
         logical_key, payload_hash, payload_canonical_byte_len, payload_json \
         FROM typed_run_events WHERE run_id = $1 ORDER BY seq ASC, ordinal ASC",
        run_id.as_str(),
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| database_error("failed to load typed run stream", error))?;
    let events = rows
        .into_iter()
        .map(event_envelope_from_row)
        .collect::<Result<Vec<_>>>()?;
    ProjectionSnapshot::validate_run_stream(&events)?;
    Ok(events)
}

fn event_envelope_from_row(row: TypedRunEventRow) -> Result<KernelEventEnvelope> {
    let payload = payload_from_json_value(&row.payload_json)?;
    let ordinal = u32::try_from(row.ordinal).map_err(|_| {
        PostgresTypedStoreError::Corruption(
            "typed_run_events.ordinal contained a negative integer".into(),
        )
    })?;

    Ok(KernelEventEnvelope::from_persisted_record(
        PersistedKernelEventRecord {
            event_id: parse_identity::<mfm_ids::EventId>(&row.event_id)?,
            event_schema_id: parse_identity::<SchemaId>(&row.event_schema_id)?,
            run_id: parse_identity::<RunId>(&row.run_id)?,
            seq: StreamSeq::new(i64_to_positive_u64(row.seq, "typed_run_events.seq")?)?,
            ordinal: CommitOrdinal::new(ordinal),
            spec_hash: parse_identity::<mfm_ids::SpecHash>(&row.spec_hash)?,
            commit_key: CommitKey::new(row.commit_key)?,
            logical_key: LogicalEventKey::new(row.logical_key)?,
            payload_hash: parse_identity::<ContentDigest>(&row.payload_hash)?,
            payload,
            payload_canonical_byte_len: i64_to_nonnegative_u64(
                row.payload_canonical_byte_len,
                "typed_run_events.payload_canonical_byte_len",
            )?,
        },
    )?)
}

struct TypedRunEventRow {
    run_id: String,
    seq: i64,
    ordinal: i32,
    event_id: String,
    event_schema_id: String,
    spec_hash: String,
    commit_key: String,
    logical_key: String,
    payload_hash: String,
    payload_canonical_byte_len: i64,
    payload_json: Value,
}

async fn write_projection_tables(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
    snapshot: &ProjectionSnapshot,
) -> Result<()> {
    clear_projection_tables(tx, run_id).await?;

    for (projected_run_id, state) in snapshot.run_states() {
        if projected_run_id == run_id {
            let json = serde_json::json!({
                "run_id": projected_run_id.as_str(),
                "run_state": run_state_str(*state),
            });
            sqlx::query!(
                "INSERT INTO typed_run_projection (run_id, run_state, projection_json) \
                 VALUES ($1,$2,$3)",
                projected_run_id.as_str(),
                run_state_str(*state),
                json,
            )
            .execute(&mut **tx)
            .await
            .map_err(|error| database_error("failed to write run projection", error))?;
        }
    }

    for (projected_run_id, projection) in snapshot.run_completions() {
        if projected_run_id == run_id {
            let json = run_completion_projection_json(projected_run_id, projection);
            sqlx::query!(
                "INSERT INTO typed_run_completion_projection (run_id, projection_json) \
                 VALUES ($1,$2)",
                projected_run_id.as_str(),
                json,
            )
            .execute(&mut **tx)
            .await
            .map_err(|error| database_error("failed to write run completion projection", error))?;
        }
    }

    for (projected_run_id, projection) in snapshot.saga_engagements() {
        if projected_run_id == run_id {
            let json = saga_engagement_projection_json(projected_run_id, projection);
            sqlx::query!(
                "INSERT INTO typed_saga_engagement_projection (run_id, projection_json) \
                 VALUES ($1,$2)",
                projected_run_id.as_str(),
                json,
            )
            .execute(&mut **tx)
            .await
            .map_err(|error| database_error("failed to write saga engagement projection", error))?;
        }
    }

    for (projected_run_id, projection) in snapshot.manual_resolutions() {
        if projected_run_id == run_id {
            let json = manual_resolution_projection_json(projected_run_id, projection);
            sqlx::query!(
                "INSERT INTO typed_manual_resolution_projection (run_id, projection_json) \
                 VALUES ($1,$2)",
                projected_run_id.as_str(),
                json,
            )
            .execute(&mut **tx)
            .await
            .map_err(|error| {
                database_error("failed to write manual resolution projection", error)
            })?;
        }
    }

    for (_, projection) in snapshot.attempts() {
        let json = attempt_projection_json(projection);
        sqlx::query!(
            "INSERT INTO typed_attempt_projection (run_id, node_id, attempt_id, projection_json) \
             VALUES ($1,$2,$3,$4)",
            run_id.as_str(),
            projection.node_id.as_str(),
            projection.attempt_id.as_str(),
            json,
        )
        .execute(&mut **tx)
        .await
        .map_err(|error| database_error("failed to write attempt projection", error))?;
    }

    for (cell_id, projection) in snapshot.cells() {
        let json = cell_projection_json(cell_id, projection);
        sqlx::query!(
            "INSERT INTO typed_cell_projection (run_id, cell_id, projection_json) VALUES ($1,$2,$3)",
            run_id.as_str(),
            cell_id.as_str(),
            json,
        )
        .execute(&mut **tx)
        .await
        .map_err(|error| database_error("failed to write cell projection", error))?;
    }

    for (_, projection) in snapshot.facts() {
        let json = fact_projection_json(projection);
        sqlx::query!(
            "INSERT INTO typed_fact_projection \
             (run_id, node_id, attempt_id, fact_key, projection_json) VALUES ($1,$2,$3,$4,$5)",
            run_id.as_str(),
            projection.node_id.as_str(),
            projection.attempt_id.as_str(),
            projection.fact_key.as_str(),
            json,
        )
        .execute(&mut **tx)
        .await
        .map_err(|error| database_error("failed to write fact projection", error))?;
    }

    for (ledger_ref, projection) in snapshot.side_effects() {
        if &ledger_ref.run_id == run_id {
            let json = side_effect_projection_json(projection);
            sqlx::query!(
                "INSERT INTO typed_side_effect_projection (run_id, ledger_key, projection_json) \
                 VALUES ($1,$2,$3)",
                ledger_ref.run_id.as_str(),
                ledger_ref.ledger_key.as_str(),
                json,
            )
            .execute(&mut **tx)
            .await
            .map_err(|error| database_error("failed to write side-effect projection", error))?;
        }
    }

    for (lane_key, projection) in snapshot.resource_lanes() {
        if &projection.holder.run_id == run_id {
            let json = resource_lane_projection_json(lane_key, projection);
            sqlx::query(
                "DELETE FROM typed_resource_lane_projection \
                 WHERE namespace = $1 AND resource_key = $2",
            )
            .bind(lane_key.namespace.as_str())
            .bind(lane_key.key.as_str())
            .execute(&mut **tx)
            .await
            .map_err(|error| database_error("failed to clear resource lane key", error))?;
            sqlx::query!(
                "INSERT INTO typed_resource_lane_projection \
                 (namespace, resource_key, run_id, ledger_key, projection_json) \
                 VALUES ($1,$2,$3,$4,$5)",
                lane_key.namespace.as_str(),
                lane_key.key.as_str(),
                projection.holder.run_id.as_str(),
                projection.holder.ledger_key.as_str(),
                json,
            )
            .execute(&mut **tx)
            .await
            .map_err(|error| database_error("failed to write resource lane projection", error))?;
        }
    }

    for (schema_id, projection) in snapshot.public_outputs() {
        let json = public_output_projection_json(schema_id, projection);
        sqlx::query!(
            "INSERT INTO typed_public_output_projection \
             (run_id, public_schema_id, projection_json) VALUES ($1,$2,$3)",
            run_id.as_str(),
            schema_id.as_str(),
            json,
        )
        .execute(&mut **tx)
        .await
        .map_err(|error| database_error("failed to write public-output projection", error))?;
    }

    Ok(())
}

async fn clear_projection_tables(tx: &mut Transaction<'_, Postgres>, run_id: &RunId) -> Result<()> {
    sqlx::query!(
        "DELETE FROM typed_run_projection WHERE run_id = $1",
        run_id.as_str()
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to clear run projection", error))?;
    sqlx::query!(
        "DELETE FROM typed_run_completion_projection WHERE run_id = $1",
        run_id.as_str()
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to clear run completion projection", error))?;
    sqlx::query!(
        "DELETE FROM typed_saga_engagement_projection WHERE run_id = $1",
        run_id.as_str()
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to clear saga engagement projection", error))?;
    sqlx::query!(
        "DELETE FROM typed_manual_resolution_projection WHERE run_id = $1",
        run_id.as_str()
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to clear manual resolution projection", error))?;
    sqlx::query!(
        "DELETE FROM typed_attempt_projection WHERE run_id = $1",
        run_id.as_str()
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to clear attempt projection", error))?;
    sqlx::query!(
        "DELETE FROM typed_cell_projection WHERE run_id = $1",
        run_id.as_str()
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to clear cell projection", error))?;
    sqlx::query!(
        "DELETE FROM typed_fact_projection WHERE run_id = $1",
        run_id.as_str()
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to clear fact projection", error))?;
    sqlx::query!(
        "DELETE FROM typed_side_effect_projection WHERE run_id = $1",
        run_id.as_str()
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to clear side-effect projection", error))?;
    sqlx::query!(
        "DELETE FROM typed_resource_lane_projection WHERE run_id = $1",
        run_id.as_str()
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to clear resource lane projection", error))?;
    sqlx::query!(
        "DELETE FROM typed_public_output_projection WHERE run_id = $1",
        run_id.as_str()
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| database_error("failed to clear public-output projection", error))?;
    Ok(())
}

async fn rebuild_projection_snapshot_from_events(
    tx: &mut Transaction<'_, Postgres>,
    run_id: &RunId,
) -> Result<ProjectionSnapshot> {
    let stream = load_run_stream_tx(tx, run_id).await?;
    Ok(ProjectionSnapshot::rebuild_from_run_stream(&stream)?)
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

async fn projection_stream_head(tx: &mut Transaction<'_, Postgres>, run_id: &RunId) -> Result<u64> {
    let row = sqlx::query!(
        "SELECT COALESCE(MAX(seq), 0) as \"head!\" FROM typed_run_events WHERE run_id = $1",
        run_id.as_str(),
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| database_error("failed to query typed stream head", error))?;
    i64_to_nonnegative_u64(row.head, "typed_run_events.seq")
}

fn canonical_payload_value(payload: &events::KernelEventPayload) -> Result<Value> {
    let canonical = mfm_store::v1::payload_canonical_json(payload)?;
    serde_json::from_slice(canonical.as_bytes()).map_err(|error| {
        PostgresTypedStoreError::Corruption(format!("canonical payload was not JSON: {error}"))
    })
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
        PostgresTypedStoreError::Corruption(format!("{field} exceeded PostgreSQL bigint range"))
    })
}

fn i64_to_nonnegative_u64(value: i64, field: &'static str) -> Result<u64> {
    u64::try_from(value).map_err(|_| {
        PostgresTypedStoreError::Corruption(format!("{field} contained a negative bigint"))
    })
}

fn i64_to_positive_u64(value: i64, field: &'static str) -> Result<u64> {
    let value = i64_to_nonnegative_u64(value, field)?;
    if value == 0 {
        return Err(PostgresTypedStoreError::Corruption(format!(
            "{field} contained a non-positive sequence"
        )));
    }
    Ok(value)
}

fn parse_optional_identity<T>(value: Option<String>) -> Result<Option<T>>
where
    T: FromStr<Err = IdentityError>,
{
    value
        .as_deref()
        .map(parse_identity)
        .transpose()
        .map_err(Into::into)
}

#[cfg(all(test, feature = "parity-tests"))]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use mfm_events::v1::{self as events, ArtifactRole, KernelEventPayload};
    use mfm_ids::{
        AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion,
        CellId, ContentDigest, DigestAlgorithm, DigestBytes, EventId, LoweringVersion, NodeId,
        RunId, SchemaId, ScopeId, SemanticTypeId, SpecHash, SpecVersion, StateKind, StateVersion,
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
        build_committed_batch, ArtifactEvidenceRef, AttemptStatus, AttemptTerminal,
        CellTerminalProjection, CommitArtifactEvidenceSet, CommitKey, CommitOutcome,
        CommitPreconditions, ManualResolution, PreparedCommit, PreparedCommitPlan,
        RequiredRunState, ResourceLaneKey, Retention, RunAdmission, RunState, SagaEngagementReason,
        SagaTerminal, SagaTerminalProof, SideEffectPhase, SideEffectProgress, SideEffectTerminal,
        StateAttemptStarted, StoreError, StreamSeq,
    };
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use sqlx::{AssertSqlSafe, Row};

    use super::*;

    static SCHEMA_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_schema() -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        let counter = SCHEMA_COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("typed_{}_{}_{}", std::process::id(), nanos, counter)
    }

    async fn test_store() -> (PostgresTypedRunEventStore, String) {
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
        let store = PostgresTypedRunEventStore { pool };
        (store, schema)
    }

    async fn drop_schema(store: &PostgresTypedRunEventStore, schema: &str) {
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

    async fn clear_projection_rows(store: &PostgresTypedRunEventStore, run_id: &RunId) {
        let mut tx = store.pool.begin().await.expect("start projection clear tx");
        clear_projection_tables(&mut tx, run_id)
            .await
            .expect("clear projections");
        tx.commit().await.expect("commit projection clear tx");
    }

    async fn poison_run_projection_state(store: &PostgresTypedRunEventStore, run_id: &RunId) {
        sqlx::query("UPDATE typed_run_projection SET run_state = $2 WHERE run_id = $1")
            .bind(run_id.as_str())
            .bind(run_state_str(RunState::Completed))
            .execute(&store.pool)
            .await
            .expect("poison run projection");
    }

    async fn poison_cell_projection_json(store: &PostgresTypedRunEventStore, run_id: &RunId) {
        sqlx::query("UPDATE typed_cell_projection SET projection_json = $2 WHERE run_id = $1")
            .bind(run_id.as_str())
            .bind(serde_json::json!({
                "cell_id": cell_id(199).as_str(),
                "poisoned": true,
            }))
            .execute(&store.pool)
            .await
            .expect("poison cell projection");
    }

    async fn poison_resource_lane_projection_row(
        store: &PostgresTypedRunEventStore,
        lane_key: &ResourceLaneKey,
        poisoned_run: &RunId,
    ) {
        let poisoned_ledger = side_effect_ledger_key();
        let updated = sqlx::query(
            "UPDATE typed_resource_lane_projection \
             SET run_id = $3, ledger_key = $4, projection_json = $5 \
             WHERE namespace = $1 AND resource_key = $2",
        )
        .bind(lane_key.namespace.as_str())
        .bind(lane_key.key.as_str())
        .bind(poisoned_run.as_str())
        .bind(poisoned_ledger.as_str())
        .bind(serde_json::json!({
            "poisoned": true,
            "run_id": poisoned_run.as_str(),
        }))
        .execute(&store.pool)
        .await
        .expect("poison resource lane projection");
        assert_eq!(updated.rows_affected(), 1);
    }

    async fn insert_stale_resource_lane_projection_row(
        store: &PostgresTypedRunEventStore,
        lane_key: &ResourceLaneKey,
        stale_run: &RunId,
    ) {
        let stale_ledger = side_effect_ledger_key();
        let projection = ResourceLaneProjection {
            event_id: EventId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(40)),
            holder: mfm_store::v1::SideEffectLedgerRef::new(
                stale_run.clone(),
                stale_ledger.clone(),
            ),
            ledger_purpose: side_effect_ledger_purpose(),
            node_id: node_id(70),
            attempt_id: attempt_id(72),
            invocation_epoch: 1,
        };
        sqlx::query(
            "INSERT INTO typed_resource_lane_projection \
             (namespace, resource_key, run_id, ledger_key, projection_json) \
             VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(lane_key.namespace.as_str())
        .bind(lane_key.key.as_str())
        .bind(stale_run.as_str())
        .bind(stale_ledger.as_str())
        .bind(resource_lane_projection_json(lane_key, &projection))
        .execute(&store.pool)
        .await
        .expect("insert stale resource lane projection");
    }

    async fn assert_resource_lane_projection_row_holder(
        store: &PostgresTypedRunEventStore,
        lane_key: &ResourceLaneKey,
        expected_run: &RunId,
    ) {
        let row = sqlx::query(
            "SELECT run_id FROM typed_resource_lane_projection \
             WHERE namespace = $1 AND resource_key = $2",
        )
        .bind(lane_key.namespace.as_str())
        .bind(lane_key.key.as_str())
        .fetch_one(&store.pool)
        .await
        .expect("resource lane projection row");
        let run_id: String = row.try_get("run_id").expect("run_id column");
        assert_eq!(run_id, expected_run.as_str());
    }

    async fn insert_persisted_events_direct(
        store: &PostgresTypedRunEventStore,
        run_id: &RunId,
        events: &[KernelEventEnvelope],
    ) -> Result<()> {
        let mut tx = store.pool.begin().await.expect("start direct insert tx");
        let head_seq = events
            .last()
            .map(|event| event.seq().as_u64())
            .unwrap_or_default();
        sqlx::query("INSERT INTO typed_run_heads (run_id, head_seq) VALUES ($1, $2)")
            .bind(run_id.as_str())
            .bind(u64_to_i64(head_seq, "typed_run_heads.head_seq")?)
            .execute(&mut *tx)
            .await
            .map_err(|error| database_error("failed to insert typed run head", error))?;
        for event in events {
            let payload_json = canonical_payload_value(event.payload())?;
            let ordinal = i32::try_from(event.ordinal().as_u32()).map_err(|_| {
                PostgresTypedStoreError::Corruption("typed_run_events.ordinal overflow".to_owned())
            })?;
            sqlx::query(
                "INSERT INTO typed_run_events \
                 (run_id, seq, ordinal, event_id, event_schema_id, spec_hash, commit_key, \
                  logical_key, payload_hash, payload_canonical_byte_len, payload_json) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
            )
            .bind(event.run_id().as_str())
            .bind(u64_to_i64(event.seq().as_u64(), "typed_run_events.seq")?)
            .bind(ordinal)
            .bind(event.event_id().as_str())
            .bind(event.event_schema_id().as_str())
            .bind(event.spec_hash().as_str())
            .bind(event.commit_key().as_str())
            .bind(event.logical_key().as_str())
            .bind(event.payload_hash().as_str())
            .bind(u64_to_i64(
                event.audit().payload_canonical_byte_len(),
                "typed_run_events.payload_canonical_byte_len",
            )?)
            .bind(payload_json)
            .execute(&mut *tx)
            .await
            .map_err(|error| database_error("failed to insert typed event", error))?;
        }
        tx.commit()
            .await
            .map_err(|error| database_error("failed to commit direct insert", error))?;
        Ok(())
    }

    fn assert_old_model_rejection(error: PostgresTypedStoreError) {
        assert!(matches!(
            error,
            PostgresTypedStoreError::Store(StoreError::ProjectionConflict { message, .. })
                if message.contains("unsupported old stream model")
                    && message.contains("StateAttemptStarted")
        ));
    }

    fn digest_bytes(byte: u8) -> DigestBytes {
        DigestBytes::from_array([byte; 32])
    }

    fn content_digest(byte: u8) -> ContentDigest {
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
    }

    fn spec_hash(byte: u8) -> SpecHash {
        SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
    }

    fn run_id(byte: u8) -> RunId {
        RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
    }

    fn artifact_id(byte: u8) -> ArtifactId {
        ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
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
            framework_version: events::FrameworkVersion::new("mfm.test.1")
                .expect("framework version"),
            source_revision: events::SourceRevision::new("test-revision").expect("source revision"),
            launched_at_unix_ms: 1_700_000_000_000,
            seed_cells: Vec::new(),
        }))
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
            prepared_artifact_id: None,
            prepared_hash: None,
            resource_key: None,
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
        ResourceLaneKey::from_evidence(&resource_key(value, 200))
    }

    fn side_effect_prepared_with_resource_key(
        resource_key: events::ResourceKeyEvidence,
    ) -> KernelEventPayload {
        let mut prepared = side_effect_prepared();
        let KernelEventPayload::SideEffectInvocationPrepared(payload) = &mut prepared else {
            unreachable!("helper returns invocation-prepared payload");
        };
        payload.resource_key = Some(resource_key);
        prepared
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
        KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
            run_id,
            spec_hash: spec_hash(1),
            refs: vec![events::RetentionRef {
                artifact_id,
                role,
                content_digest: digest,
            }],
            reason: events::RetentionReason::RuntimeEvidence,
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
        let hash = spec_hash(1);
        ArtifactEvidenceRef {
            artifact_id: artifact_id(2),
            digest: ContentDigest::from_digest(hash.algorithm(), *hash.digest()),
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
            digest: content_digest(5),
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
    ) -> mfm_store::v1::TypedCommitRequest {
        mfm_store::v1::TypedCommitRequest::from_payloads(
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
        store: &PostgresTypedRunEventStore,
        mut request: mfm_store::v1::TypedCommitRequest,
        artifacts: Vec<ArtifactEvidenceRef>,
    ) -> Result<CommitOutcome> {
        if request.required_artifacts().is_empty() && !artifacts.is_empty() {
            request = request.with_required_artifacts(artifacts.clone());
        }
        let plan = test_prepared_commit_plan(request, artifacts)?;
        store.append_prepared_commit_plan(plan).await
    }

    fn test_prepared_commit_plan(
        request: mfm_store::v1::TypedCommitRequest,
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
        )
    }

    async fn append_run_start(
        store: &PostgresTypedRunEventStore,
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
        store: &PostgresTypedRunEventStore,
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
        store: &PostgresTypedRunEventStore,
        run_id: &RunId,
        commit_key: &str,
        lane_value: &str,
        artifact_byte: u8,
    ) -> Result<CommitOutcome> {
        let intent_artifact = artifact_id(artifact_byte);
        let intent_digest = content_digest(artifact_byte + 1);
        append_prepared(
            store,
            request(
                run_id.clone(),
                3,
                commit_key,
                vec![
                    side_effect_intent(intent_artifact.clone(), intent_digest.clone()),
                    side_effect_claim(),
                    side_effect_prepared_with_resource_key(resource_key(lane_value, 201)),
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
        error: PostgresTypedStoreError,
        expected_lane_key: &ResourceLaneKey,
        expected_holder_run: &RunId,
    ) {
        let PostgresTypedStoreError::Store(StoreError::ResourceLaneBlocked { lane_key, holder }) =
            error
        else {
            panic!("expected typed resource lane block, got {error:?}");
        };
        assert_eq!(&*lane_key, expected_lane_key);
        assert_eq!(&holder.run_id, expected_holder_run);
        assert_eq!(holder.ledger_key, side_effect_ledger_key());
    }

    #[tokio::test]
    async fn typed_prepared_commit_idempotency_fingerprint_includes_admitted_artifacts() {
        let (store, schema) = test_store().await;
        let run = run_id(120);
        let artifact = artifact_id(121);
        let digest = content_digest(122);
        let evidence =
            store_artifact_ref(artifact.clone(), digest.clone(), ArtifactRole::StateOutput);
        let mut conflicting_evidence = evidence.clone();
        conflicting_evidence.byte_len += 1;
        append_prepared(
            &store,
            request(run.clone(), 1, "run-start", vec![run_admitted(run.clone())]),
            vec![spec_artifact_ref(), certificate_artifact_ref()],
        )
        .await
        .expect("run start");
        let request = mfm_store::v1::TypedCommitRequest::from_payloads(
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
            PostgresTypedStoreError::Store(StoreError::CommitConflict { .. })
        ));

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn typed_commit_key_sequence_and_projection_rebuild_contract() {
        let (store, schema) = test_store().await;
        let run = run_id(7);
        let artifact = artifact_id(8);
        let digest = content_digest(9);

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

        poison_run_projection_state(&store, &run).await;
        poison_cell_projection_json(&store, &run).await;
        assert_eq!(
            store
                .projection_snapshot(&run)
                .await
                .expect("stream-authoritative projection ignores poisoned rows"),
            before
        );

        clear_projection_rows(&store, &run).await;
        assert_eq!(
            store
                .projection_snapshot(&run)
                .await
                .expect("stream-authoritative projection ignores deleted rows"),
            before
        );
        let rebuilt = store
            .rebuild_projections_from_events(&run)
            .await
            .expect("rebuild projections");
        assert_eq!(rebuilt, before);

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn typed_interrupted_attempt_projection_persists_and_rebuilds_from_events() {
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

        clear_projection_rows(&store, &run).await;
        let rebuilt = store
            .rebuild_projections_from_events(&run)
            .await
            .expect("rebuild projections");
        assert_eq!(rebuilt, before);
        assert!(matches!(
            &rebuilt
                .attempt(&node_id(20), &attempt_id(23))
                .expect("rebuilt attempt projection")
                .status,
            AttemptStatus::Interrupted
        ));

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn typed_old_model_persisted_rows_reject_on_load_and_rebuild() {
        let (store, schema) = test_store().await;
        let run = run_id(18);
        let run_start = build_committed_batch(
            &request(run.clone(), 1, "run-start", vec![run_admitted(run.clone())]),
            StreamSeq::FIRST,
        )
        .expect("run start batch");
        let old_terminal = build_committed_batch(
            &request(
                run.clone(),
                2,
                "old-terminal-without-start",
                vec![
                    cell_produced(artifact_id(19), content_digest(20)),
                    state_attempt_completed(),
                ],
            ),
            StreamSeq::new(2).expect("terminal seq"),
        )
        .expect("old terminal batch");
        let mut events = run_start.events().to_vec();
        events.extend(old_terminal.events().iter().cloned());
        insert_persisted_events_direct(&store, &run, &events)
            .await
            .expect("insert old rows");

        let load_error = store
            .load_run_stream(&run)
            .await
            .expect_err("old model rows reject on load");
        assert_old_model_rejection(load_error);
        let rebuild_error = store
            .rebuild_projections_from_events(&run)
            .await
            .expect_err("old model rows reject on rebuild");
        assert_old_model_rejection(rebuild_error);

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn typed_resource_lane_projection_persists_and_rebuilds_from_events() {
        let (store, schema) = test_store().await;
        let run = run_id(21);
        let intent_artifact = artifact_id(22);
        let intent_digest = content_digest(23);
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
                    side_effect_prepared_with_resource_key(resource_key("wallet-1", 201)),
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
        let lane = before
            .resource_lane(&lane_key)
            .expect("persisted resource lane");
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

        clear_projection_rows(&store, &run).await;
        clear_projection_rows(&store, &peer_run).await;
        let peer_after_deleted = store
            .projection_snapshot(&peer_run)
            .await
            .expect("peer stream-authoritative projection");
        let peer_lane = peer_after_deleted
            .resource_lane(&lane_key)
            .expect("peer snapshot rebuilds cross-run lane from stream");
        assert_eq!(&peer_lane.holder.run_id, &run);
        assert_eq!(&peer_lane.holder.ledger_key, &side_effect_ledger_key());
        assert_eq!(peer_after_deleted.run_state(&peer_run), RunState::Started);
        assert_eq!(
            store
                .projection_snapshot(&run)
                .await
                .expect("holder stream-authoritative projection ignores deleted lane row"),
            before
        );
        let rebuilt = store
            .rebuild_projections_from_events(&run)
            .await
            .expect("rebuild projections");
        assert_eq!(rebuilt, before);
        assert!(
            rebuilt.resource_lane(&lane_key).is_some(),
            "rebuilt projection must retain non-terminal lane"
        );

        clear_projection_rows(&store, &peer_run).await;
        store
            .rebuild_projections_from_events(&peer_run)
            .await
            .expect("rebuild peer projections");
        let peer_after_rebuild = store
            .projection_snapshot(&peer_run)
            .await
            .expect("peer status projection after rebuild");
        let peer_lane = peer_after_rebuild
            .resource_lane(&lane_key)
            .expect("peer rebuilt status projection includes cross-run lane");
        assert_eq!(&peer_lane.holder.run_id, &run);
        assert_eq!(&peer_lane.holder.ledger_key, &side_effect_ledger_key());

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn typed_resource_lane_append_admission_uses_stream_authority_after_projection_damage() {
        let (store, schema) = test_store().await;
        let holder_run = run_id(25);
        let deleted_projection_contender = run_id(26);
        let poisoned_projection_contender = run_id(27);
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

        append_run_start(
            &store,
            &deleted_projection_contender,
            "deleted-contender-run-start",
        )
        .await
        .expect("deleted contender run start");
        append_resource_lane_attempt_start(
            &store,
            &deleted_projection_contender,
            "deleted-contender-attempt-start",
        )
        .await
        .expect("deleted contender attempt start");
        let deleted_stream_before = store
            .load_run_stream(&deleted_projection_contender)
            .await
            .expect("deleted contender stream before conflict");
        clear_projection_rows(&store, &holder_run).await;
        let deleted_projection_error = append_resource_lane_prepare(
            &store,
            &deleted_projection_contender,
            "deleted-contender-prepare",
            lane_value,
            30,
        )
        .await
        .expect_err("deleted resource lane projection row still blocks from stream authority");
        assert_resource_lane_blocked(deleted_projection_error, &lane_key, &holder_run);
        assert_eq!(
            store
                .load_run_stream(&deleted_projection_contender)
                .await
                .expect("deleted contender stream after conflict"),
            deleted_stream_before
        );
        assert_eq!(
            store
                .expected_next_seq(&deleted_projection_contender)
                .await
                .expect("deleted contender next seq"),
            StreamSeq::new(3).expect("deleted contender prepare seq")
        );

        store
            .rebuild_projections_from_events(&holder_run)
            .await
            .expect("restore holder resource lane projection row");
        append_run_start(
            &store,
            &poisoned_projection_contender,
            "poisoned-contender-run-start",
        )
        .await
        .expect("poisoned contender run start");
        append_resource_lane_attempt_start(
            &store,
            &poisoned_projection_contender,
            "poisoned-contender-attempt-start",
        )
        .await
        .expect("poisoned contender attempt start");
        let poisoned_stream_before = store
            .load_run_stream(&poisoned_projection_contender)
            .await
            .expect("poisoned contender stream before conflict");
        poison_resource_lane_projection_row(&store, &lane_key, &poisoned_projection_contender)
            .await;
        let poisoned_projection_error = append_resource_lane_prepare(
            &store,
            &poisoned_projection_contender,
            "poisoned-contender-prepare",
            lane_value,
            32,
        )
        .await
        .expect_err("poisoned resource lane projection row still blocks from stream authority");
        assert_resource_lane_blocked(poisoned_projection_error, &lane_key, &holder_run);
        assert_eq!(
            store
                .load_run_stream(&poisoned_projection_contender)
                .await
                .expect("poisoned contender stream after conflict"),
            poisoned_stream_before
        );
        assert_eq!(
            store
                .expected_next_seq(&poisoned_projection_contender)
                .await
                .expect("poisoned contender next seq"),
            StreamSeq::new(3).expect("poisoned contender prepare seq")
        );

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn typed_resource_lane_projection_writer_repairs_stale_free_lane_rows() {
        let (store, schema) = test_store().await;
        let stale_projection_run = run_id(33);
        let contender_run = run_id(34);
        let lane_value = "wallet-stale-free-lane";
        let lane_key = resource_lane_key(lane_value);

        append_run_start(&store, &stale_projection_run, "stale-free-row-run-start")
            .await
            .expect("stale projection run start");
        insert_stale_resource_lane_projection_row(&store, &lane_key, &stale_projection_run).await;
        assert_resource_lane_projection_row_holder(&store, &lane_key, &stale_projection_run).await;

        append_run_start(&store, &contender_run, "stale-free-contender-run-start")
            .await
            .expect("contender run start");
        append_resource_lane_attempt_start(
            &store,
            &contender_run,
            "stale-free-contender-attempt-start",
        )
        .await
        .expect("contender attempt start");
        append_resource_lane_prepare(
            &store,
            &contender_run,
            "stale-free-contender-prepare",
            lane_value,
            35,
        )
        .await
        .expect("stale projection row must not veto stream-free lane");
        let projection = store
            .projection_snapshot(&contender_run)
            .await
            .expect("contender projection");
        let lane = projection
            .resource_lane(&lane_key)
            .expect("contender acquired lane");
        assert_eq!(&lane.holder.run_id, &contender_run);
        assert_resource_lane_projection_row_holder(&store, &lane_key, &contender_run).await;

        poison_resource_lane_projection_row(&store, &lane_key, &stale_projection_run).await;
        assert_resource_lane_projection_row_holder(&store, &lane_key, &stale_projection_run).await;
        let rebuilt = store
            .rebuild_projections_from_events(&contender_run)
            .await
            .expect("rebuild repairs stale free-lane row");
        let rebuilt_lane = rebuilt
            .resource_lane(&lane_key)
            .expect("rebuilt contender lane");
        assert_eq!(&rebuilt_lane.holder.run_id, &contender_run);
        assert_resource_lane_projection_row_holder(&store, &lane_key, &contender_run).await;

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn typed_saga_projection_tables_persist_and_rebuild_from_events() {
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
        let intent_digest = content_digest(41);
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
        let ambiguity_digest = content_digest(141);
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
        let manual_request = mfm_store::v1::TypedCommitRequest::from_payloads(
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
            .append_prepared_commit_plan(manual_commit.into())
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

        clear_projection_rows(&store, &run).await;
        let rebuilt = store
            .rebuild_projections_from_events(&run)
            .await
            .expect("rebuild projections");
        assert_eq!(rebuilt, before);

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
        let terminal_intent_digest = content_digest(151);
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
        let terminal_ambiguity_digest = content_digest(153);
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
            .append_prepared_commit_plan(terminal_commit.into())
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

        clear_projection_rows(&store, &terminal_run).await;
        let terminal_rebuilt = store
            .rebuild_projections_from_events(&terminal_run)
            .await
            .expect("rebuild terminal projections");
        assert_eq!(terminal_rebuilt, terminal_before);

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn typed_required_artifacts_and_fact_projection_are_atomic() {
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
        let missing_digest = content_digest(12);
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
            PostgresTypedStoreError::Store(StoreError::MissingArtifact { .. })
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
    async fn typed_side_effect_unknown_recovery_updates_submission_result_slot() {
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
        let intent_digest = content_digest(15);
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
        let unknown_digest = content_digest(17);
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
        .batch()
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
        let submission_digest = content_digest(19);
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
        .batch()
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

        let stored_payload_hash = sqlx::query_scalar!(
            "SELECT payload_hash FROM typed_unique_logical_payloads \
             WHERE run_id = $1 AND logical_key = $2",
            run.as_str(),
            submission_result_key.as_str(),
        )
        .fetch_one(&store.pool)
        .await
        .expect("unique logical payload row");
        assert_eq!(
            stored_payload_hash,
            observed.events()[0].payload_hash().as_str()
        );

        drop_schema(&store, &schema).await;
    }
}
