use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use mfm_events::v1::{self as events, side_effect, ArtifactRole};
use mfm_ids::{
    ArtifactId, CellId, ContentDigest, IdentityError, NodeId, RunId, SchemaId, SeedId,
    SemanticTypeId,
};
use mfm_spec::v1::MediaType;
use mfm_store::v1::{
    build_committed_batch, commit_fingerprint, payload_from_json_value, ArtifactEvidenceRef,
    AttemptProjection, AttemptStatus, CellTerminalProjection, CommitKey, CommitOrdinal,
    CommitOutcome, FactProjection, KernelEventEnvelope, LogicalEventKey,
    PersistedKernelEventRecord, ProjectionSnapshot, PublicOutputProjection,
    RetentionManifestProjection, RetentionProjection, RunState, SideEffectClaimProjection,
    SideEffectIntentProjection, SideEffectPhase, SideEffectProjection, StoreError, StreamSeq,
    TypedCommitBase, TypedCommitRequest,
};
use serde_json::Value;
use tokio::sync::Mutex;
use tokio_postgres::{Client, Transaction};

use crate::PostgresStreamStore;

/// Error returned by the PostgreSQL typed run event store.
#[derive(Debug)]
pub enum PostgresTypedStoreError {
    /// Typed store contract validation failed.
    Store(StoreError),
    /// PostgreSQL operation failed.
    Database(&'static str),
    /// PostgreSQL operation failed with a source diagnostic.
    DatabaseSource {
        /// Static operation context.
        context: &'static str,
        /// Sanitized PostgreSQL error string.
        source: String,
    },
    /// Persisted typed store rows are corrupt.
    Corruption(String),
}

impl fmt::Display for PostgresTypedStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(f, "{error}"),
            Self::Database(message) => write!(f, "postgres typed store error: {message}"),
            Self::DatabaseSource { context, source } => {
                write!(f, "postgres typed store error: {context}: {source}")
            }
            Self::Corruption(message) => write!(f, "postgres typed store corruption: {message}"),
        }
    }
}

impl std::error::Error for PostgresTypedStoreError {}

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

type Result<T> = std::result::Result<T, PostgresTypedStoreError>;

fn database_source(context: &'static str, error: tokio_postgres::Error) -> PostgresTypedStoreError {
    PostgresTypedStoreError::DatabaseSource {
        context,
        source: format!("{error:?}"),
    }
}

/// PostgreSQL-backed typed run event store.
///
/// This is the certified typed storage surface for run events, commit keys, artifact evidence, and
/// derived projections. The legacy [`PostgresStreamStore`] remains available only for pre-typed
/// dynamic callers.
#[derive(Clone)]
pub struct PostgresTypedRunEventStore {
    pub(crate) client: Arc<Mutex<Client>>,
}

impl PostgresTypedRunEventStore {
    /// Connects to PostgreSQL, initializes the legacy and typed schemas, and returns a typed store.
    pub async fn connect(database_url: &str) -> Result<Self> {
        let legacy = PostgresStreamStore::connect(database_url)
            .await
            .map_err(|_| PostgresTypedStoreError::Database("connect failed"))?;
        Ok(legacy.typed_run_event_store())
    }

    /// Connects using the `DATABASE_URL` environment variable.
    pub async fn connect_env() -> Result<Self> {
        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| PostgresTypedStoreError::Database("missing DATABASE_URL"))?;
        Self::connect(&database_url).await
    }

    pub(crate) fn from_client(client: Arc<Mutex<Client>>) -> Self {
        Self { client }
    }

    /// Records artifact evidence before typed run events reference that artifact.
    pub async fn record_artifact_evidence(&self, evidence: ArtifactEvidenceRef) -> Result<()> {
        let mut client = self.client.lock().await;
        let tx = client
            .transaction()
            .await
            .map_err(|_| PostgresTypedStoreError::Database("failed to start transaction"))?;
        record_artifact_evidence_tx(&tx, &evidence).await?;
        tx.commit()
            .await
            .map_err(|_| PostgresTypedStoreError::Database("failed to commit transaction"))?;
        Ok(())
    }

    /// Returns the next store-owned stream sequence for a run.
    pub async fn expected_next_seq(&self, run_id: &RunId) -> Result<StreamSeq> {
        let client = self.client.lock().await;
        let head = read_head(&client, run_id).await?;
        next_seq_from_head(head)
    }

    /// Appends one typed run commit atomically.
    pub async fn append_typed_run_commit(
        &self,
        request: TypedCommitRequest,
    ) -> Result<CommitOutcome> {
        let fingerprint = commit_fingerprint(&request)?;
        let fingerprint_text = fingerprint.as_digest().as_str().to_owned();

        let mut client = self.client.lock().await;
        let tx = client
            .transaction()
            .await
            .map_err(|_| PostgresTypedStoreError::Database("failed to start transaction"))?;

        if let Some((stored_fingerprint, stored_seq)) =
            read_commit_key(&tx, &request.run_id, request.commit_key.as_str()).await?
        {
            if stored_fingerprint == fingerprint_text {
                let batch = build_committed_batch(&request, stored_seq)?;
                tx.commit().await.map_err(|_| {
                    PostgresTypedStoreError::Database("failed to commit transaction")
                })?;
                return Ok(CommitOutcome::Idempotent(batch));
            }
            return Err(StoreError::CommitConflict {
                commit_key: request.commit_key,
            }
            .into());
        }

        ensure_run_head(&tx, &request.run_id).await?;
        let head = read_head_for_update(&tx, &request.run_id).await?;

        // A same-run transaction may have inserted the key while this transaction waited for the
        // run-head lock. Re-check before sequence validation while preserving the required initial
        // commit-key lookup order.
        if let Some((stored_fingerprint, stored_seq)) =
            read_commit_key(&tx, &request.run_id, request.commit_key.as_str()).await?
        {
            if stored_fingerprint == fingerprint_text {
                let batch = build_committed_batch(&request, stored_seq)?;
                tx.commit().await.map_err(|_| {
                    PostgresTypedStoreError::Database("failed to commit transaction")
                })?;
                return Ok(CommitOutcome::Idempotent(batch));
            }
            return Err(StoreError::CommitConflict {
                commit_key: request.commit_key,
            }
            .into());
        }

        let base = TypedCommitBase {
            artifacts: load_artifacts(&tx).await?,
            logical_keys: load_logical_keys(&tx, &request.run_id).await?,
            unique_logical_payloads: load_unique_logical_payloads(&tx, &request.run_id).await?,
            projections: load_projection_snapshot_tx(&tx, &request.run_id).await?,
            actual_next_seq: next_seq_from_head(head)?,
        };
        let staged = mfm_store::v1::stage_typed_run_commit(&base, &request)?;
        let batch = staged.batch().clone();
        let staged_projections = staged.projections().clone();

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
            tx.execute(
                "INSERT INTO typed_run_events \
                 (run_id, seq, ordinal, event_id, event_schema_id, spec_hash, commit_key, \
                  logical_key, payload_hash, payload_canonical_byte_len, payload_json) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)",
                &[
                    &event.run_id().as_str(),
                    &seq,
                    &ordinal,
                    &event.event_id().as_str(),
                    &event.event_schema_id().as_str(),
                    &event.spec_hash().as_str(),
                    &event.commit_key().as_str(),
                    &event.logical_key().as_str(),
                    &event.payload_hash().as_str(),
                    &payload_canonical_byte_len,
                    &payload_json,
                ],
            )
            .await
            .map_err(|error| database_source("failed to insert typed event", error))?;
        }

        let commit_seq = u64_to_i64(batch.seq().as_u64(), "typed_commit_keys.seq")?;
        let event_count = i32::try_from(batch.events().len())
            .map_err(|_| PostgresTypedStoreError::Corruption("event count overflow".into()))?;
        tx.execute(
            "INSERT INTO typed_commit_keys \
             (run_id, commit_key, commit_fingerprint, seq, event_count) \
             VALUES ($1,$2,$3,$4,$5)",
            &[
                &request.run_id.as_str(),
                &request.commit_key.as_str(),
                &fingerprint_text,
                &commit_seq,
                &event_count,
            ],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to insert commit key"))?;

        for event in batch.events() {
            tx.execute(
                "INSERT INTO typed_logical_keys (run_id, logical_key) VALUES ($1,$2) \
                 ON CONFLICT (run_id, logical_key) DO NOTHING",
                &[&request.run_id.as_str(), &event.logical_key().as_str()],
            )
            .await
            .map_err(|_| PostgresTypedStoreError::Database("failed to insert logical key"))?;
            if is_unique_logical_key(event.logical_key()) {
                tx.execute(
                    "INSERT INTO typed_unique_logical_payloads \
                     (run_id, logical_key, payload_hash) VALUES ($1,$2,$3)",
                    &[
                        &request.run_id.as_str(),
                        &event.logical_key().as_str(),
                        &event.payload_hash().as_str(),
                    ],
                )
                .await
                .map_err(|_| {
                    PostgresTypedStoreError::Database("failed to insert unique logical key")
                })?;
            }
        }

        write_projection_tables(&tx, &request.run_id, &staged_projections).await?;
        tx.execute(
            "UPDATE typed_run_heads SET head_seq = $2 WHERE run_id = $1",
            &[
                &request.run_id.as_str(),
                &u64_to_i64(batch.seq().as_u64(), "typed_run_heads.head_seq")?,
            ],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to update run head"))?;

        tx.commit()
            .await
            .map_err(|_| PostgresTypedStoreError::Database("failed to commit transaction"))?;
        Ok(CommitOutcome::Appended(batch))
    }

    /// Loads the current projection snapshot from projection tables.
    pub async fn projection_snapshot(&self, run_id: &RunId) -> Result<ProjectionSnapshot> {
        let mut client = self.client.lock().await;
        load_projection_snapshot_client(&mut client, run_id).await
    }

    /// Loads the authoritative typed run stream from persisted event rows.
    pub async fn load_run_stream(&self, run_id: &RunId) -> Result<Vec<KernelEventEnvelope>> {
        let client = self.client.lock().await;
        load_run_stream_client(&client, run_id).await
    }

    /// Rebuilds projection tables from the authoritative `typed_run_events` stream.
    pub async fn rebuild_projections_from_events(
        &self,
        run_id: &RunId,
    ) -> Result<ProjectionSnapshot> {
        let mut client = self.client.lock().await;
        let tx = client
            .transaction()
            .await
            .map_err(|_| PostgresTypedStoreError::Database("failed to start transaction"))?;
        let snapshot = rebuild_projection_snapshot_from_events(&tx, run_id).await?;
        write_projection_tables(&tx, run_id, &snapshot).await?;
        tx.commit()
            .await
            .map_err(|_| PostgresTypedStoreError::Database("failed to commit transaction"))?;
        Ok(snapshot)
    }
}

async fn record_artifact_evidence_tx(
    tx: &Transaction<'_>,
    evidence: &ArtifactEvidenceRef,
) -> Result<()> {
    let inserted = tx
        .execute(
            "INSERT INTO typed_artifacts \
         (artifact_id, digest, byte_len, media_type, schema_id, semantic_type_id, \
          producer_node_id, producer_seed_id, artifact_role) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) \
         ON CONFLICT (artifact_id) DO NOTHING",
            &[
                &evidence.artifact_id.as_str(),
                &evidence.digest.as_str(),
                &u64_to_i64(evidence.byte_len, "typed_artifacts.byte_len")?,
                &evidence.media_type.as_str(),
                &evidence.schema_id.as_ref().map(SchemaId::as_str),
                &evidence
                    .semantic_type_id
                    .as_ref()
                    .map(SemanticTypeId::as_str),
                &evidence.producer_node_id.as_ref().map(NodeId::as_str),
                &evidence.producer_seed_id.as_ref().map(SeedId::as_str),
                &artifact_role_str(evidence.artifact_role),
            ],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to insert artifact evidence"))?;
    if inserted == 1 {
        return Ok(());
    }

    let row = tx
        .query_one(
            "SELECT digest, byte_len, media_type, schema_id, semantic_type_id, producer_node_id, \
             producer_seed_id, artifact_role FROM typed_artifacts WHERE artifact_id = $1",
            &[&evidence.artifact_id.as_str()],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to query artifact evidence"))?;
    let stored = artifact_from_row(evidence.artifact_id.clone(), &row)?;
    if &stored == evidence {
        return Ok(());
    }
    Err(StoreError::ArtifactEvidenceMismatch {
        artifact_id: evidence.artifact_id.clone(),
        field: "artifact",
    }
    .into())
}

async fn ensure_run_head(tx: &Transaction<'_>, run_id: &RunId) -> Result<()> {
    tx.execute(
        "INSERT INTO typed_run_heads (run_id, head_seq) VALUES ($1, 0) \
         ON CONFLICT (run_id) DO NOTHING",
        &[&run_id.as_str()],
    )
    .await
    .map_err(|_| PostgresTypedStoreError::Database("failed to ensure typed run head"))?;
    Ok(())
}

async fn read_head(client: &Client, run_id: &RunId) -> Result<u64> {
    let row = client
        .query_opt(
            "SELECT head_seq FROM typed_run_heads WHERE run_id = $1",
            &[&run_id.as_str()],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to query typed run head"))?;
    let Some(row) = row else {
        return Ok(0);
    };
    let head: i64 = row.get(0);
    i64_to_nonnegative_u64(head, "typed_run_heads.head_seq")
}

async fn read_head_for_update(tx: &Transaction<'_>, run_id: &RunId) -> Result<u64> {
    let row = tx
        .query_one(
            "SELECT head_seq FROM typed_run_heads WHERE run_id = $1 FOR UPDATE",
            &[&run_id.as_str()],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to lock typed run head"))?;
    let head: i64 = row.get(0);
    i64_to_nonnegative_u64(head, "typed_run_heads.head_seq")
}

async fn read_commit_key(
    tx: &Transaction<'_>,
    run_id: &RunId,
    commit_key: &str,
) -> Result<Option<(String, StreamSeq)>> {
    let row = tx
        .query_opt(
            "SELECT commit_fingerprint, seq FROM typed_commit_keys \
             WHERE run_id = $1 AND commit_key = $2",
            &[&run_id.as_str(), &commit_key],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to query commit key"))?;
    row.map(|row| {
        let fingerprint: String = row.get(0);
        let seq: i64 = row.get(1);
        Ok((
            fingerprint,
            StreamSeq::new(i64_to_positive_u64(seq, "typed_commit_keys.seq")?)?,
        ))
    })
    .transpose()
}

async fn load_artifacts(tx: &Transaction<'_>) -> Result<BTreeMap<ArtifactId, ArtifactEvidenceRef>> {
    let rows = tx
        .query(
            "SELECT artifact_id, digest, byte_len, media_type, schema_id, semantic_type_id, \
             producer_node_id, producer_seed_id, artifact_role FROM typed_artifacts",
            &[],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to load artifact evidence"))?;
    let mut artifacts = BTreeMap::new();
    for row in rows {
        let artifact_id: String = row.get(0);
        let artifact_id = parse_identity::<ArtifactId>(&artifact_id)?;
        let evidence = artifact_from_row(artifact_id.clone(), &row)?;
        artifacts.insert(artifact_id, evidence);
    }
    Ok(artifacts)
}

fn artifact_from_row(
    artifact_id: ArtifactId,
    row: &tokio_postgres::Row,
) -> Result<ArtifactEvidenceRef> {
    let digest: String = row.get("digest");
    let byte_len: i64 = row.get("byte_len");
    let media_type: String = row.get("media_type");
    let schema_id: Option<String> = row.get("schema_id");
    let semantic_type_id: Option<String> = row.get("semantic_type_id");
    let producer_node_id: Option<String> = row.get("producer_node_id");
    let producer_seed_id: Option<String> = row.get("producer_seed_id");
    let artifact_role: String = row.get("artifact_role");
    Ok(ArtifactEvidenceRef {
        artifact_id,
        digest: parse_identity::<ContentDigest>(&digest)?,
        byte_len: i64_to_nonnegative_u64(byte_len, "typed_artifacts.byte_len")?,
        media_type: MediaType::new(media_type)?,
        schema_id: parse_optional_identity(schema_id)?,
        semantic_type_id: parse_optional_identity(semantic_type_id)?,
        producer_node_id: parse_optional_identity(producer_node_id)?,
        producer_seed_id: parse_optional_identity(producer_seed_id)?,
        artifact_role: parse_artifact_role(&artifact_role)?,
    })
}

async fn load_logical_keys(
    tx: &Transaction<'_>,
    run_id: &RunId,
) -> Result<BTreeSet<(RunId, LogicalEventKey)>> {
    let rows = tx
        .query(
            "SELECT logical_key FROM typed_logical_keys WHERE run_id = $1",
            &[&run_id.as_str()],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to load logical keys"))?;
    let mut keys = BTreeSet::new();
    for row in rows {
        let key: String = row.get(0);
        keys.insert((run_id.clone(), LogicalEventKey::new(key)?));
    }
    Ok(keys)
}

async fn load_unique_logical_payloads(
    tx: &Transaction<'_>,
    run_id: &RunId,
) -> Result<BTreeMap<(RunId, LogicalEventKey), ContentDigest>> {
    let rows = tx
        .query(
            "SELECT logical_key, payload_hash FROM typed_unique_logical_payloads WHERE run_id = $1",
            &[&run_id.as_str()],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to load unique logical payloads"))?;
    let mut payloads = BTreeMap::new();
    for row in rows {
        let key: String = row.get(0);
        let payload_hash: String = row.get(1);
        payloads.insert(
            (run_id.clone(), LogicalEventKey::new(key)?),
            parse_identity::<ContentDigest>(&payload_hash)?,
        );
    }
    Ok(payloads)
}

async fn load_projection_snapshot_client(
    client: &mut Client,
    run_id: &RunId,
) -> Result<ProjectionSnapshot> {
    let tx = client
        .transaction()
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to start read transaction"))?;
    let snapshot = load_projection_snapshot_tx(&tx, run_id).await?;
    tx.commit()
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to commit read transaction"))?;
    Ok(snapshot)
}

async fn load_projection_snapshot_tx(
    tx: &Transaction<'_>,
    run_id: &RunId,
) -> Result<ProjectionSnapshot> {
    let mut run_states = BTreeMap::new();
    for row in tx
        .query(
            "SELECT run_state FROM typed_run_projection WHERE run_id = $1",
            &[&run_id.as_str()],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to load run projection"))?
    {
        let state: String = row.get(0);
        run_states.insert(run_id.clone(), parse_run_state(&state)?);
    }

    let mut attempts = BTreeMap::new();
    for row in tx
        .query(
            "SELECT projection_json FROM typed_attempt_projection WHERE run_id = $1",
            &[&run_id.as_str()],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to load attempt projection"))?
    {
        let json: Value = row.get(0);
        let projection = parse_attempt_projection(&json)?;
        attempts.insert(
            (projection.node_id.clone(), projection.attempt_id.clone()),
            projection,
        );
    }

    let mut cells = BTreeMap::new();
    for row in tx
        .query(
            "SELECT projection_json FROM typed_cell_projection WHERE run_id = $1",
            &[&run_id.as_str()],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to load cell projection"))?
    {
        let json: Value = row.get(0);
        let (cell_id, projection) = parse_cell_projection(&json)?;
        cells.insert(cell_id, projection);
    }

    let mut facts = BTreeMap::new();
    for row in tx
        .query(
            "SELECT projection_json FROM typed_fact_projection WHERE run_id = $1",
            &[&run_id.as_str()],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to load fact projection"))?
    {
        let json: Value = row.get(0);
        let projection = parse_fact_projection(&json)?;
        facts.insert(
            (
                projection.node_id.clone(),
                projection.attempt_id.clone(),
                projection.fact_key.clone(),
            ),
            projection,
        );
    }

    let mut side_effects = BTreeMap::new();
    for row in tx
        .query(
            "SELECT projection_json FROM typed_side_effect_projection WHERE run_id = $1",
            &[&run_id.as_str()],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to load side-effect projection"))?
    {
        let json: Value = row.get(0);
        let projection = parse_side_effect_projection(&json)?;
        side_effects.insert(projection.ledger_key.clone(), projection);
    }

    let mut public_outputs = BTreeMap::new();
    for row in tx
        .query(
            "SELECT projection_json FROM typed_public_output_projection WHERE run_id = $1",
            &[&run_id.as_str()],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to load public-output projection"))?
    {
        let json: Value = row.get(0);
        let (schema_id, projection) = parse_public_output_projection(&json)?;
        public_outputs.insert(schema_id, projection);
    }

    let mut retention = RetentionProjection::default();
    for row in tx
        .query(
            "SELECT projection_json FROM typed_retention_projection WHERE run_id = $1",
            &[&run_id.as_str()],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to load retention projection"))?
    {
        let json: Value = row.get(0);
        let retention_ref = parse_retention_ref(&json)?;
        retention
            .refs
            .insert(retention_ref.artifact_id.clone(), retention_ref);
    }
    if let Some(row) = tx
        .query_opt(
            "SELECT projection_json FROM typed_retention_manifests WHERE run_id = $1",
            &[&run_id.as_str()],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to load retention manifest"))?
    {
        let json: Value = row.get(0);
        retention.manifest = Some(parse_retention_manifest_projection(&json)?);
    }
    let mut retentions = BTreeMap::new();
    if !retention.refs.is_empty() || retention.manifest.is_some() {
        retentions.insert(run_id.clone(), retention);
    }

    Ok(ProjectionSnapshot::from_parts(
        run_states,
        attempts,
        cells,
        facts,
        side_effects,
        public_outputs,
        retentions,
    ))
}

async fn load_run_stream_client(
    client: &Client,
    run_id: &RunId,
) -> Result<Vec<KernelEventEnvelope>> {
    let rows = client
        .query(
            "SELECT run_id, seq, ordinal, event_id, event_schema_id, spec_hash, commit_key, \
             logical_key, payload_hash, payload_canonical_byte_len, payload_json \
             FROM typed_run_events WHERE run_id = $1 ORDER BY seq ASC, ordinal ASC",
            &[&run_id.as_str()],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to load typed run stream"))?;
    let events = rows
        .iter()
        .map(event_envelope_from_row)
        .collect::<Result<Vec<_>>>()?;
    ProjectionSnapshot::validate_run_stream(&events)?;
    Ok(events)
}

async fn load_run_stream_tx(
    tx: &Transaction<'_>,
    run_id: &RunId,
) -> Result<Vec<KernelEventEnvelope>> {
    let rows = tx
        .query(
            "SELECT run_id, seq, ordinal, event_id, event_schema_id, spec_hash, commit_key, \
             logical_key, payload_hash, payload_canonical_byte_len, payload_json \
             FROM typed_run_events WHERE run_id = $1 ORDER BY seq ASC, ordinal ASC",
            &[&run_id.as_str()],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to load typed run stream"))?;
    let events = rows
        .iter()
        .map(event_envelope_from_row)
        .collect::<Result<Vec<_>>>()?;
    ProjectionSnapshot::validate_run_stream(&events)?;
    Ok(events)
}

fn event_envelope_from_row(row: &tokio_postgres::Row) -> Result<KernelEventEnvelope> {
    let run_id: String = row.get("run_id");
    let seq: i64 = row.get("seq");
    let ordinal: i32 = row.get("ordinal");
    let event_id: String = row.get("event_id");
    let event_schema_id: String = row.get("event_schema_id");
    let spec_hash: String = row.get("spec_hash");
    let commit_key: String = row.get("commit_key");
    let logical_key: String = row.get("logical_key");
    let payload_hash: String = row.get("payload_hash");
    let payload_canonical_byte_len: i64 = row.get("payload_canonical_byte_len");
    let payload_json: Value = row.get("payload_json");
    let payload = payload_from_json_value(&payload_json)?;
    let ordinal = u32::try_from(ordinal).map_err(|_| {
        PostgresTypedStoreError::Corruption(
            "typed_run_events.ordinal contained a negative integer".into(),
        )
    })?;

    Ok(KernelEventEnvelope::from_persisted_record(
        PersistedKernelEventRecord {
            event_id: parse_identity::<mfm_ids::EventId>(&event_id)?,
            event_schema_id: parse_identity::<SchemaId>(&event_schema_id)?,
            run_id: parse_identity::<RunId>(&run_id)?,
            seq: StreamSeq::new(i64_to_positive_u64(seq, "typed_run_events.seq")?)?,
            ordinal: CommitOrdinal::new(ordinal),
            spec_hash: parse_identity::<mfm_ids::SpecHash>(&spec_hash)?,
            commit_key: CommitKey::new(commit_key)?,
            logical_key: LogicalEventKey::new(logical_key)?,
            payload_hash: parse_identity::<ContentDigest>(&payload_hash)?,
            payload,
            payload_canonical_byte_len: i64_to_nonnegative_u64(
                payload_canonical_byte_len,
                "typed_run_events.payload_canonical_byte_len",
            )?,
        },
    )?)
}

async fn write_projection_tables(
    tx: &Transaction<'_>,
    run_id: &RunId,
    snapshot: &ProjectionSnapshot,
) -> Result<()> {
    for table in [
        "typed_run_projection",
        "typed_attempt_projection",
        "typed_cell_projection",
        "typed_fact_projection",
        "typed_side_effect_projection",
        "typed_public_output_projection",
        "typed_retention_projection",
        "typed_retention_manifests",
    ] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE run_id = $1"),
            &[&run_id.as_str()],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to clear projection table"))?;
    }

    for (projected_run_id, state) in snapshot.run_states() {
        if projected_run_id == run_id {
            let json = serde_json::json!({
                "run_id": projected_run_id.as_str(),
                "run_state": run_state_str(*state),
            });
            tx.execute(
                "INSERT INTO typed_run_projection (run_id, run_state, projection_json) \
                 VALUES ($1,$2,$3)",
                &[&projected_run_id.as_str(), &run_state_str(*state), &json],
            )
            .await
            .map_err(|_| PostgresTypedStoreError::Database("failed to write run projection"))?;
        }
    }

    for (_, projection) in snapshot.attempts() {
        let json = attempt_projection_json(projection);
        tx.execute(
            "INSERT INTO typed_attempt_projection (run_id, node_id, attempt_id, projection_json) \
             VALUES ($1,$2,$3,$4)",
            &[
                &run_id.as_str(),
                &projection.node_id.as_str(),
                &projection.attempt_id.as_str(),
                &json,
            ],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to write attempt projection"))?;
    }

    for (cell_id, projection) in snapshot.cells() {
        let json = cell_projection_json(cell_id, projection);
        tx.execute(
            "INSERT INTO typed_cell_projection (run_id, cell_id, projection_json) VALUES ($1,$2,$3)",
            &[&run_id.as_str(), &cell_id.as_str(), &json],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to write cell projection"))?;
    }

    for (_, projection) in snapshot.facts() {
        let json = fact_projection_json(projection);
        tx.execute(
            "INSERT INTO typed_fact_projection \
             (run_id, node_id, attempt_id, fact_key, projection_json) VALUES ($1,$2,$3,$4,$5)",
            &[
                &run_id.as_str(),
                &projection.node_id.as_str(),
                &projection.attempt_id.as_str(),
                &projection.fact_key.as_str(),
                &json,
            ],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to write fact projection"))?;
    }

    for (ledger_key, projection) in snapshot.side_effects() {
        let json = side_effect_projection_json(projection);
        tx.execute(
            "INSERT INTO typed_side_effect_projection (run_id, ledger_key, projection_json) \
             VALUES ($1,$2,$3)",
            &[&run_id.as_str(), &ledger_key.as_str(), &json],
        )
        .await
        .map_err(|_| PostgresTypedStoreError::Database("failed to write side-effect projection"))?;
    }

    for (schema_id, projection) in snapshot.public_outputs() {
        let json = public_output_projection_json(schema_id, projection);
        tx.execute(
            "INSERT INTO typed_public_output_projection \
             (run_id, public_schema_id, projection_json) VALUES ($1,$2,$3)",
            &[&run_id.as_str(), &schema_id.as_str(), &json],
        )
        .await
        .map_err(|_| {
            PostgresTypedStoreError::Database("failed to write public-output projection")
        })?;
    }

    if let Some(retention) = snapshot.retention(run_id) {
        for retention_ref in retention.refs.values() {
            let json = retention_ref_json(retention_ref);
            tx.execute(
                "INSERT INTO typed_retention_projection (run_id, artifact_id, projection_json) \
                 VALUES ($1,$2,$3)",
                &[&run_id.as_str(), &retention_ref.artifact_id.as_str(), &json],
            )
            .await
            .map_err(|_| {
                PostgresTypedStoreError::Database("failed to write retention projection")
            })?;
        }
        if let Some(manifest) = &retention.manifest {
            let json = retention_manifest_projection_json(manifest);
            tx.execute(
                "INSERT INTO typed_retention_manifests (run_id, manifest_seq, projection_json) \
                 VALUES ($1,$2,$3)",
                &[
                    &run_id.as_str(),
                    &u64_to_i64(
                        manifest.manifest_seq,
                        "typed_retention_manifests.manifest_seq",
                    )?,
                    &json,
                ],
            )
            .await
            .map_err(|_| PostgresTypedStoreError::Database("failed to write retention manifest"))?;
        }
    }

    Ok(())
}

async fn rebuild_projection_snapshot_from_events(
    tx: &Transaction<'_>,
    run_id: &RunId,
) -> Result<ProjectionSnapshot> {
    let stream = load_run_stream_tx(tx, run_id).await?;
    Ok(ProjectionSnapshot::rebuild_from_run_stream(&stream)?)
}

fn canonical_payload_value(payload: &events::KernelEventPayload) -> Result<Value> {
    let canonical = mfm_store::v1::payload_canonical_json(payload)?;
    serde_json::from_slice(canonical.as_bytes()).map_err(|error| {
        PostgresTypedStoreError::Corruption(format!("canonical payload was not JSON: {error}"))
    })
}

fn attempt_projection_json(projection: &AttemptProjection) -> Value {
    let status = match &projection.status {
        AttemptStatus::Started {
            attempt_no,
            state_kind,
            state_version,
        } => serde_json::json!({
            "variant": "started",
            "attempt_no": attempt_no,
            "state_kind": state_kind.as_str(),
            "state_version": state_version.as_str(),
        }),
        AttemptStatus::Completed { output_cell_id } => serde_json::json!({
            "variant": "completed",
            "output_cell_id": output_cell_id.as_str(),
        }),
        AttemptStatus::Failed { retryable, error } => serde_json::json!({
            "variant": "failed",
            "retryable": retryable,
            "error": error_info_json(error),
        }),
    };
    serde_json::json!({
        "attempt_id": projection.attempt_id.as_str(),
        "event_id": projection.event_id.as_str(),
        "node_id": projection.node_id.as_str(),
        "status": status,
    })
}

fn parse_attempt_projection(json: &Value) -> Result<AttemptProjection> {
    let status_json = required_obj(json, "status")?;
    let status = match required_str(status_json, "variant")? {
        "started" => AttemptStatus::Started {
            attempt_no: required_u64(status_json, "attempt_no")?
                .try_into()
                .map_err(|_| PostgresTypedStoreError::Corruption("attempt_no overflow".into()))?,
            state_kind: parse_identity(required_str(status_json, "state_kind")?)?,
            state_version: required_str(status_json, "state_version")?.parse()?,
        },
        "completed" => AttemptStatus::Completed {
            output_cell_id: parse_identity(required_str(status_json, "output_cell_id")?)?,
        },
        "failed" => AttemptStatus::Failed {
            retryable: required_bool(status_json, "retryable")?,
            error: Box::new(parse_error_info(required_obj(status_json, "error")?)?),
        },
        other => {
            return Err(PostgresTypedStoreError::Corruption(format!(
                "unknown attempt status {other}"
            )));
        }
    };
    Ok(AttemptProjection {
        node_id: parse_identity(required_str(json, "node_id")?)?,
        attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
        event_id: parse_identity(required_str(json, "event_id")?)?,
        status,
    })
}

fn cell_projection_json(cell_id: &CellId, projection: &CellTerminalProjection) -> Value {
    match projection {
        CellTerminalProjection::Produced {
            event_id,
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            artifact_id,
            content_digest,
        } => serde_json::json!({
            "variant": "produced",
            "cell_id": cell_id.as_str(),
            "event_id": event_id.as_str(),
            "node_id": node_id.as_str(),
            "attempt_id": attempt_id.as_str(),
            "schema_id": schema_id.as_str(),
            "semantic_type_id": semantic_type_id.as_str(),
            "artifact_id": artifact_id.as_str(),
            "content_digest": content_digest.as_str(),
        }),
        CellTerminalProjection::Skipped {
            event_id,
            node_id,
            attempt_id,
            schema_id,
            semantic_type_id,
            skip_reason,
        } => serde_json::json!({
            "variant": "skipped",
            "cell_id": cell_id.as_str(),
            "event_id": event_id.as_str(),
            "node_id": node_id.as_str(),
            "attempt_id": attempt_id.as_str(),
            "schema_id": schema_id.as_str(),
            "semantic_type_id": semantic_type_id.as_str(),
            "skip_reason": skip_reason_json(skip_reason),
        }),
    }
}

fn parse_cell_projection(json: &Value) -> Result<(CellId, CellTerminalProjection)> {
    let cell_id = parse_identity(required_str(json, "cell_id")?)?;
    let projection = match required_str(json, "variant")? {
        "produced" => CellTerminalProjection::Produced {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            schema_id: parse_identity(required_str(json, "schema_id")?)?,
            semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
            artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
            content_digest: parse_identity(required_str(json, "content_digest")?)?,
        },
        "skipped" => CellTerminalProjection::Skipped {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            node_id: parse_identity(required_str(json, "node_id")?)?,
            attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
            schema_id: parse_identity(required_str(json, "schema_id")?)?,
            semantic_type_id: parse_identity(required_str(json, "semantic_type_id")?)?,
            skip_reason: parse_skip_reason(required_obj(json, "skip_reason")?)?,
        },
        other => {
            return Err(PostgresTypedStoreError::Corruption(format!(
                "unknown cell projection {other}"
            )));
        }
    };
    Ok((cell_id, projection))
}

fn fact_projection_json(projection: &FactProjection) -> Value {
    serde_json::json!({
        "adapter_kind": projection.adapter_kind.as_str(),
        "adapter_version": projection.adapter_version.as_str(),
        "artifact_id": projection.artifact_id.as_str(),
        "attempt_id": projection.attempt_id.as_str(),
        "capability_kind": projection.capability_kind.as_str(),
        "capability_version": projection.capability_version.as_str(),
        "event_id": projection.event_id.as_str(),
        "fact_key": projection.fact_key.as_str(),
        "node_id": projection.node_id.as_str(),
        "request_hash": projection.request_hash.as_str(),
        "request_schema_id": projection.request_schema_id.as_str(),
        "response_hash": projection.response_hash.as_str(),
        "response_schema_id": projection.response_schema_id.as_str(),
    })
}

fn parse_fact_projection(json: &Value) -> Result<FactProjection> {
    Ok(FactProjection {
        event_id: parse_identity(required_str(json, "event_id")?)?,
        node_id: parse_identity(required_str(json, "node_id")?)?,
        attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
        fact_key: events::FactKey::new(required_str(json, "fact_key")?)?,
        request_schema_id: parse_identity(required_str(json, "request_schema_id")?)?,
        request_hash: parse_identity(required_str(json, "request_hash")?)?,
        response_schema_id: parse_identity(required_str(json, "response_schema_id")?)?,
        response_hash: parse_identity(required_str(json, "response_hash")?)?,
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        capability_kind: parse_identity(required_str(json, "capability_kind")?)?,
        capability_version: required_str(json, "capability_version")?.parse()?,
        adapter_kind: parse_identity(required_str(json, "adapter_kind")?)?,
        adapter_version: required_str(json, "adapter_version")?.parse()?,
    })
}

fn side_effect_projection_json(projection: &SideEffectProjection) -> Value {
    serde_json::json!({
        "claim": projection.claim.as_ref().map(side_effect_claim_json),
        "event_id": projection.event_id.as_str(),
        "intent": side_effect_intent_json(&projection.intent),
        "ledger_key": projection.ledger_key.as_str(),
        "phase": side_effect_phase_json(&projection.phase),
    })
}

fn parse_side_effect_projection(json: &Value) -> Result<SideEffectProjection> {
    Ok(SideEffectProjection {
        ledger_key: events::SideEffectLedgerKey::new(required_str(json, "ledger_key")?)?,
        event_id: parse_identity(required_str(json, "event_id")?)?,
        intent: parse_side_effect_intent(required_obj(json, "intent")?)?,
        claim: optional_obj(json, "claim")?
            .map(parse_side_effect_claim)
            .transpose()?,
        phase: parse_side_effect_phase(required_obj(json, "phase")?)?,
    })
}

fn side_effect_intent_json(intent: &SideEffectIntentProjection) -> Value {
    serde_json::json!({
        "adapter_kind": intent.adapter_kind.as_str(),
        "adapter_version": intent.adapter_version.as_str(),
        "attempt_id": intent.attempt_id.as_str(),
        "capability_kind": intent.capability_kind.as_str(),
        "capability_version": intent.capability_version.as_str(),
        "idempotency_input_hash": intent.idempotency_input_hash.as_str(),
        "idempotency_input_schema_id": intent.idempotency_input_schema_id.as_str(),
        "idempotency_key": intent.idempotency_key.as_str(),
        "intent_artifact_id": intent.intent_artifact_id.as_str(),
        "intent_hash": intent.intent_hash.as_str(),
        "intent_schema_id": intent.intent_schema_id.as_str(),
        "invocation_epoch": intent.invocation_epoch,
        "node_id": intent.node_id.as_str(),
        "scope_id": intent.scope_id.as_str(),
    })
}

fn parse_side_effect_intent(json: &Value) -> Result<SideEffectIntentProjection> {
    Ok(SideEffectIntentProjection {
        node_id: parse_identity(required_str(json, "node_id")?)?,
        attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
        scope_id: parse_identity(required_str(json, "scope_id")?)?,
        invocation_epoch: required_u32(json, "invocation_epoch")?,
        intent_schema_id: parse_identity(required_str(json, "intent_schema_id")?)?,
        intent_hash: parse_identity(required_str(json, "intent_hash")?)?,
        intent_artifact_id: parse_identity(required_str(json, "intent_artifact_id")?)?,
        idempotency_input_schema_id: parse_identity(required_str(
            json,
            "idempotency_input_schema_id",
        )?)?,
        idempotency_input_hash: parse_identity(required_str(json, "idempotency_input_hash")?)?,
        idempotency_key: events::IdempotencyKeyRef::new(required_str(json, "idempotency_key")?)?,
        capability_kind: parse_identity(required_str(json, "capability_kind")?)?,
        capability_version: required_str(json, "capability_version")?.parse()?,
        adapter_kind: parse_identity(required_str(json, "adapter_kind")?)?,
        adapter_version: required_str(json, "adapter_version")?.parse()?,
    })
}

fn side_effect_claim_json(claim: &SideEffectClaimProjection) -> Value {
    serde_json::json!({
        "attempt_id": claim.attempt_id.as_str(),
        "claim_fencing_token": claim.claim_fencing_token.as_str(),
        "claim_generation": claim.claim_generation,
        "claim_owner": claim.claim_owner.as_str(),
        "invocation_epoch": claim.invocation_epoch,
        "node_id": claim.node_id.as_str(),
    })
}

fn parse_side_effect_claim(json: &Value) -> Result<SideEffectClaimProjection> {
    Ok(SideEffectClaimProjection {
        node_id: parse_identity(required_str(json, "node_id")?)?,
        attempt_id: parse_identity(required_str(json, "attempt_id")?)?,
        claim_owner: events::RunnerInvocationId::new(required_str(json, "claim_owner")?)?,
        invocation_epoch: required_u32(json, "invocation_epoch")?,
        claim_generation: required_u32(json, "claim_generation")?,
        claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
            json,
            "claim_fencing_token",
        )?)?,
    })
}

fn side_effect_phase_json(phase: &SideEffectPhase) -> Value {
    match phase {
        SideEffectPhase::IntentPersisted { invocation_epoch } => {
            phase_json("intent_persisted", *invocation_epoch, serde_json::json!({}))
        }
        SideEffectPhase::Claimed {
            claim_owner,
            invocation_epoch,
            claim_generation,
            claim_fencing_token,
        } => phase_json(
            "claimed",
            *invocation_epoch,
            serde_json::json!({
                "claim_owner": claim_owner.as_str(),
                "claim_generation": claim_generation,
                "claim_fencing_token": claim_fencing_token.as_str(),
            }),
        ),
        SideEffectPhase::InvocationPrepared {
            invocation_epoch,
            claim_generation,
            claim_fencing_token,
        } => phase_json(
            "invocation_prepared",
            *invocation_epoch,
            serde_json::json!({
                "claim_generation": claim_generation,
                "claim_fencing_token": claim_fencing_token.as_str(),
            }),
        ),
        SideEffectPhase::InvocationStarted {
            claim_owner,
            invocation_epoch,
            claim_generation,
            claim_fencing_token,
        } => phase_json(
            "invocation_started",
            *invocation_epoch,
            serde_json::json!({
                "claim_owner": claim_owner.as_str(),
                "claim_generation": claim_generation,
                "claim_fencing_token": claim_fencing_token.as_str(),
            }),
        ),
        SideEffectPhase::SubmissionObserved { invocation_epoch } => phase_json(
            "submission_observed",
            *invocation_epoch,
            serde_json::json!({}),
        ),
        SideEffectPhase::NotSubmittedProven { invocation_epoch } => phase_json(
            "not_submitted_proven",
            *invocation_epoch,
            serde_json::json!({}),
        ),
        SideEffectPhase::SubmissionUnknown { invocation_epoch } => phase_json(
            "submission_unknown",
            *invocation_epoch,
            serde_json::json!({}),
        ),
        SideEffectPhase::ReceiptObserved { invocation_epoch } => {
            phase_json("receipt_observed", *invocation_epoch, serde_json::json!({}))
        }
        SideEffectPhase::ConfirmationObserved { invocation_epoch } => phase_json(
            "confirmation_observed",
            *invocation_epoch,
            serde_json::json!({}),
        ),
        SideEffectPhase::Ambiguous { invocation_epoch } => {
            phase_json("ambiguous", *invocation_epoch, serde_json::json!({}))
        }
        SideEffectPhase::Failed {
            invocation_epoch,
            failure_phase,
        } => phase_json(
            "failed",
            *invocation_epoch,
            serde_json::json!({
                "failure_phase": failure_phase_str(*failure_phase),
            }),
        ),
    }
}

fn phase_json(variant: &'static str, invocation_epoch: u32, mut extra: Value) -> Value {
    extra["variant"] = serde_json::json!(variant);
    extra["invocation_epoch"] = serde_json::json!(invocation_epoch);
    extra
}

fn parse_side_effect_phase(json: &Value) -> Result<SideEffectPhase> {
    let invocation_epoch = required_u32(json, "invocation_epoch")?;
    match required_str(json, "variant")? {
        "intent_persisted" => Ok(SideEffectPhase::IntentPersisted { invocation_epoch }),
        "claimed" => Ok(SideEffectPhase::Claimed {
            claim_owner: events::RunnerInvocationId::new(required_str(json, "claim_owner")?)?,
            invocation_epoch,
            claim_generation: required_u32(json, "claim_generation")?,
            claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                json,
                "claim_fencing_token",
            )?)?,
        }),
        "invocation_prepared" => Ok(SideEffectPhase::InvocationPrepared {
            invocation_epoch,
            claim_generation: required_u32(json, "claim_generation")?,
            claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                json,
                "claim_fencing_token",
            )?)?,
        }),
        "invocation_started" => Ok(SideEffectPhase::InvocationStarted {
            claim_owner: events::RunnerInvocationId::new(required_str(json, "claim_owner")?)?,
            invocation_epoch,
            claim_generation: required_u32(json, "claim_generation")?,
            claim_fencing_token: side_effect::ClaimFencingToken::new(required_str(
                json,
                "claim_fencing_token",
            )?)?,
        }),
        "submission_observed" => Ok(SideEffectPhase::SubmissionObserved { invocation_epoch }),
        "not_submitted_proven" => Ok(SideEffectPhase::NotSubmittedProven { invocation_epoch }),
        "submission_unknown" => Ok(SideEffectPhase::SubmissionUnknown { invocation_epoch }),
        "receipt_observed" => Ok(SideEffectPhase::ReceiptObserved { invocation_epoch }),
        "confirmation_observed" => Ok(SideEffectPhase::ConfirmationObserved { invocation_epoch }),
        "ambiguous" => Ok(SideEffectPhase::Ambiguous { invocation_epoch }),
        "failed" => Ok(SideEffectPhase::Failed {
            invocation_epoch,
            failure_phase: parse_failure_phase(required_str(json, "failure_phase")?)?,
        }),
        other => Err(PostgresTypedStoreError::Corruption(format!(
            "unknown side-effect phase {other}"
        ))),
    }
}

fn public_output_projection_json(
    schema_id: &SchemaId,
    projection: &PublicOutputProjection,
) -> Value {
    match projection {
        PublicOutputProjection::Produced {
            event_id,
            rendered_digest,
            rendered_artifact_id,
        } => serde_json::json!({
            "variant": "produced",
            "public_schema_id": schema_id.as_str(),
            "event_id": event_id.as_str(),
            "rendered_digest": rendered_digest.as_str(),
            "rendered_artifact_id": rendered_artifact_id.as_ref().map(ArtifactId::as_str),
        }),
        PublicOutputProjection::RenderFailed { event_id, error } => serde_json::json!({
            "variant": "render_failed",
            "public_schema_id": schema_id.as_str(),
            "event_id": event_id.as_str(),
            "error": error_info_json(error),
        }),
    }
}

fn parse_public_output_projection(json: &Value) -> Result<(SchemaId, PublicOutputProjection)> {
    let schema_id = parse_identity(required_str(json, "public_schema_id")?)?;
    let projection = match required_str(json, "variant")? {
        "produced" => PublicOutputProjection::Produced {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            rendered_digest: parse_identity(required_str(json, "rendered_digest")?)?,
            rendered_artifact_id: optional_str(json, "rendered_artifact_id")?
                .map(parse_identity)
                .transpose()?,
        },
        "render_failed" => PublicOutputProjection::RenderFailed {
            event_id: parse_identity(required_str(json, "event_id")?)?,
            error: Box::new(parse_error_info(required_obj(json, "error")?)?),
        },
        other => {
            return Err(PostgresTypedStoreError::Corruption(format!(
                "unknown public output projection {other}"
            )));
        }
    };
    Ok((schema_id, projection))
}

fn retention_ref_json(retention_ref: &events::RetentionRef) -> Value {
    serde_json::json!({
        "artifact_id": retention_ref.artifact_id.as_str(),
        "content_digest": retention_ref.content_digest.as_str(),
        "role": artifact_role_str(retention_ref.role),
    })
}

fn parse_retention_ref(json: &Value) -> Result<events::RetentionRef> {
    Ok(events::RetentionRef {
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        role: parse_artifact_role(required_str(json, "role")?)?,
        content_digest: parse_identity(required_str(json, "content_digest")?)?,
    })
}

fn retention_manifest_projection_json(manifest: &RetentionManifestProjection) -> Value {
    serde_json::json!({
        "manifest_artifact_id": manifest.manifest_artifact_id.as_str(),
        "manifest_digest": manifest.manifest_digest.as_str(),
        "manifest_seq": manifest.manifest_seq,
    })
}

fn parse_retention_manifest_projection(json: &Value) -> Result<RetentionManifestProjection> {
    Ok(RetentionManifestProjection {
        manifest_seq: required_u64(json, "manifest_seq")?,
        manifest_digest: parse_identity(required_str(json, "manifest_digest")?)?,
        manifest_artifact_id: parse_identity(required_str(json, "manifest_artifact_id")?)?,
    })
}

fn error_info_json(error: &events::MfmErrorInfo) -> Value {
    serde_json::json!({
        "category": error_category_str(error.category),
        "code": error.code.as_str(),
        "diagnostic_ref": error.diagnostic_ref.as_ref().map(event_artifact_json),
        "public_details": error.public_details.as_ref().map(|details| {
            serde_json::json!({ "content_digest": details.content_digest.as_str() })
        }),
        "retryable": error.retryable,
        "safe_message": error.safe_message,
    })
}

fn parse_error_info(json: &Value) -> Result<events::MfmErrorInfo> {
    Ok(events::MfmErrorInfo {
        code: events::ErrorCode::new(required_str(json, "code")?)?,
        category: parse_error_category(required_str(json, "category")?)?,
        retryable: required_bool(json, "retryable")?,
        safe_message: required_str(json, "safe_message")?.to_owned(),
        public_details: optional_obj(json, "public_details")?
            .map(|details| {
                Ok::<events::RedactedJson, PostgresTypedStoreError>(events::RedactedJson {
                    content_digest: parse_identity(required_str(details, "content_digest")?)?,
                })
            })
            .transpose()?,
        diagnostic_ref: optional_obj(json, "diagnostic_ref")?
            .map(parse_event_artifact)
            .transpose()?,
    })
}

fn skip_reason_json(reason: &events::SkipReason) -> Value {
    serde_json::json!({
        "code": reason.code.as_str(),
        "safe_message": reason.safe_message,
    })
}

fn parse_skip_reason(json: &Value) -> Result<events::SkipReason> {
    Ok(events::SkipReason {
        code: events::ErrorCode::new(required_str(json, "code")?)?,
        safe_message: required_str(json, "safe_message")?.to_owned(),
    })
}

fn event_artifact_json(evidence: &events::ArtifactEvidenceRef) -> Value {
    serde_json::json!({
        "artifact_id": evidence.artifact_id.as_str(),
        "byte_len": evidence.byte_len,
        "content_digest": evidence.content_digest.as_str(),
        "media_type": evidence.media_type.as_str(),
        "role": artifact_role_str(evidence.role),
        "schema_id": evidence.schema_id.as_str(),
        "semantic_type_id": evidence.semantic_type_id.as_ref().map(SemanticTypeId::as_str),
    })
}

fn parse_event_artifact(json: &Value) -> Result<events::ArtifactEvidenceRef> {
    Ok(events::ArtifactEvidenceRef {
        artifact_id: parse_identity(required_str(json, "artifact_id")?)?,
        role: parse_artifact_role(required_str(json, "role")?)?,
        schema_id: parse_identity(required_str(json, "schema_id")?)?,
        semantic_type_id: optional_str(json, "semantic_type_id")?
            .map(parse_identity)
            .transpose()?,
        content_digest: parse_identity(required_str(json, "content_digest")?)?,
        byte_len: required_u64(json, "byte_len")?,
        media_type: MediaType::new(required_str(json, "media_type")?)?,
    })
}

fn run_state_str(state: RunState) -> &'static str {
    match state {
        RunState::Absent => "absent",
        RunState::Started => "started",
        RunState::Completed => "completed",
    }
}

fn parse_run_state(value: &str) -> Result<RunState> {
    match value {
        "absent" => Ok(RunState::Absent),
        "started" => Ok(RunState::Started),
        "completed" => Ok(RunState::Completed),
        other => Err(PostgresTypedStoreError::Corruption(format!(
            "unknown run state {other}"
        ))),
    }
}

fn artifact_role_str(role: ArtifactRole) -> &'static str {
    match role {
        ArtifactRole::TypedExecutionSpec => "typed_execution_spec",
        ArtifactRole::TypedConfig => "typed_config",
        ArtifactRole::SeedInput => "seed_input",
        ArtifactRole::StateOutput => "state_output",
        ArtifactRole::FactResponse => "fact_response",
        ArtifactRole::SideEffectIntent => "side_effect_intent",
        ArtifactRole::PreparedInvocation => "prepared_invocation",
        ArtifactRole::NotSubmittedProof => "not_submitted_proof",
        ArtifactRole::Submission => "submission",
        ArtifactRole::SubmissionUnknownEvidence => "submission_unknown_evidence",
        ArtifactRole::Receipt => "receipt",
        ArtifactRole::Confirmation => "confirmation",
        ArtifactRole::AmbiguityEvidence => "ambiguity_evidence",
        ArtifactRole::PublicOutput => "public_output",
        ArtifactRole::RedactedDiagnostic => "redacted_diagnostic",
        ArtifactRole::RetentionManifest => "retention_manifest",
    }
}

fn parse_artifact_role(value: &str) -> Result<ArtifactRole> {
    match value {
        "typed_execution_spec" => Ok(ArtifactRole::TypedExecutionSpec),
        "typed_config" => Ok(ArtifactRole::TypedConfig),
        "seed_input" => Ok(ArtifactRole::SeedInput),
        "state_output" => Ok(ArtifactRole::StateOutput),
        "fact_response" => Ok(ArtifactRole::FactResponse),
        "side_effect_intent" => Ok(ArtifactRole::SideEffectIntent),
        "prepared_invocation" => Ok(ArtifactRole::PreparedInvocation),
        "not_submitted_proof" => Ok(ArtifactRole::NotSubmittedProof),
        "submission" => Ok(ArtifactRole::Submission),
        "submission_unknown_evidence" => Ok(ArtifactRole::SubmissionUnknownEvidence),
        "receipt" => Ok(ArtifactRole::Receipt),
        "confirmation" => Ok(ArtifactRole::Confirmation),
        "ambiguity_evidence" => Ok(ArtifactRole::AmbiguityEvidence),
        "public_output" => Ok(ArtifactRole::PublicOutput),
        "redacted_diagnostic" => Ok(ArtifactRole::RedactedDiagnostic),
        "retention_manifest" => Ok(ArtifactRole::RetentionManifest),
        other => Err(PostgresTypedStoreError::Corruption(format!(
            "unknown artifact role {other}"
        ))),
    }
}

fn failure_phase_str(phase: side_effect::FailurePhase) -> &'static str {
    match phase {
        side_effect::FailurePhase::BeforeInvocationStarted => "before_invocation_started",
        side_effect::FailurePhase::AfterNotSubmittedProven => "after_not_submitted_proven",
    }
}

fn parse_failure_phase(value: &str) -> Result<side_effect::FailurePhase> {
    match value {
        "before_invocation_started" => Ok(side_effect::FailurePhase::BeforeInvocationStarted),
        "after_not_submitted_proven" => Ok(side_effect::FailurePhase::AfterNotSubmittedProven),
        other => Err(PostgresTypedStoreError::Corruption(format!(
            "unknown failure phase {other}"
        ))),
    }
}

fn error_category_str(category: events::ErrorCategory) -> &'static str {
    match category {
        events::ErrorCategory::Planning => "planning",
        events::ErrorCategory::Validation => "validation",
        events::ErrorCategory::Capability => "capability",
        events::ErrorCategory::SideEffect => "side_effect",
        events::ErrorCategory::Runtime => "runtime",
        events::ErrorCategory::Storage => "storage",
        events::ErrorCategory::Cancelled => "cancelled",
    }
}

fn parse_error_category(value: &str) -> Result<events::ErrorCategory> {
    match value {
        "planning" => Ok(events::ErrorCategory::Planning),
        "validation" => Ok(events::ErrorCategory::Validation),
        "capability" => Ok(events::ErrorCategory::Capability),
        "side_effect" => Ok(events::ErrorCategory::SideEffect),
        "runtime" => Ok(events::ErrorCategory::Runtime),
        "storage" => Ok(events::ErrorCategory::Storage),
        "cancelled" => Ok(events::ErrorCategory::Cancelled),
        other => Err(PostgresTypedStoreError::Corruption(format!(
            "unknown error category {other}"
        ))),
    }
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

fn parse_identity<T>(value: &str) -> Result<T>
where
    T: FromStr<Err = IdentityError>,
{
    value.parse().map_err(StoreError::from).map_err(Into::into)
}

fn parse_optional_identity<T>(value: Option<String>) -> Result<Option<T>>
where
    T: FromStr<Err = IdentityError>,
{
    value.as_deref().map(parse_identity).transpose()
}

fn required_str<'a>(json: &'a Value, field: &'static str) -> Result<&'a str> {
    json.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| PostgresTypedStoreError::Corruption(format!("missing string field {field}")))
}

fn optional_str<'a>(json: &'a Value, field: &'static str) -> Result<Option<&'a str>> {
    match json.get(field) {
        Some(Value::Null) | None => Ok(None),
        Some(value) => value.as_str().map(Some).ok_or_else(|| {
            PostgresTypedStoreError::Corruption(format!("field {field} was not a string"))
        }),
    }
}

fn required_obj<'a>(json: &'a Value, field: &'static str) -> Result<&'a Value> {
    let value = json.get(field).ok_or_else(|| {
        PostgresTypedStoreError::Corruption(format!("missing object field {field}"))
    })?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(PostgresTypedStoreError::Corruption(format!(
            "field {field} was not an object"
        )))
    }
}

fn optional_obj<'a>(json: &'a Value, field: &'static str) -> Result<Option<&'a Value>> {
    match json.get(field) {
        Some(Value::Null) | None => Ok(None),
        Some(value) if value.is_object() => Ok(Some(value)),
        Some(_) => Err(PostgresTypedStoreError::Corruption(format!(
            "field {field} was not an object"
        ))),
    }
}

fn required_u64(json: &Value, field: &'static str) -> Result<u64> {
    json.get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| PostgresTypedStoreError::Corruption(format!("missing u64 field {field}")))
}

fn required_u32(json: &Value, field: &'static str) -> Result<u32> {
    required_u64(json, field)?
        .try_into()
        .map_err(|_| PostgresTypedStoreError::Corruption(format!("{field} overflowed u32")))
}

fn required_bool(json: &Value, field: &'static str) -> Result<bool> {
    json.get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| PostgresTypedStoreError::Corruption(format!("missing bool field {field}")))
}

#[cfg(all(test, feature = "parity-tests"))]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use mfm_events::v1::{self as events, ArtifactRole, KernelEventPayload};
    use mfm_ids::{
        AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion,
        CellId, ContentDigest, DigestAlgorithm, DigestBytes, LoweringVersion, NodeId, RunId,
        SchemaId, ScopeId, SemanticTypeId, SpecHash, SpecVersion, StateKind, StateVersion,
    };
    use mfm_spec::v1::{CanonicalizerIdentity, MediaType, ValueLineageRef};
    use mfm_store::v1::{
        ArtifactEvidenceRef, CellTerminalProjection, CommitKey, CommitOutcome, CommitPreconditions,
        RequiredRunState, StoreError, StreamSeq,
    };
    use tokio_postgres::NoTls;

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
        let (client, connection) = tokio_postgres::connect(&database_url, NoTls)
            .await
            .expect("connect postgres");
        tokio::spawn(async move {
            let _ = connection.await;
        });

        let schema = unique_schema();
        client
            .batch_execute(&format!(
                "CREATE SCHEMA {schema}; SET search_path TO {schema};"
            ))
            .await
            .expect("create schema");
        let legacy = PostgresStreamStore {
            client: Arc::new(Mutex::new(client)),
        };
        legacy.init().await.expect("init schema");
        (legacy.typed_run_event_store(), schema)
    }

    async fn drop_schema(store: &PostgresTypedRunEventStore, schema: &str) {
        store
            .client
            .lock()
            .await
            .batch_execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE;"))
            .await
            .expect("drop schema");
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

    fn run_started(run_id: RunId) -> KernelEventPayload {
        KernelEventPayload::RunStarted(events::RunStarted {
            run_id,
            spec_hash: spec_hash(1),
            spec_artifact_id: artifact_id(2),
            spec_media_type: media_type("application/vnd.mfm.typed-execution-spec+json;version=1"),
            spec_version: SpecVersion::new("mfm.typed.execution_spec.v1").expect("spec version"),
            lowering_version: LoweringVersion::new("mfm.typed.lowering.v1")
                .expect("lowering version"),
            public_output_schema_id: schema_id("mfm.test.public_output", 3),
            descriptor_identities: Vec::new(),
            runner_executables: Vec::new(),
            adapter_executables: Vec::new(),
            canonicalizer_identity: CanonicalizerIdentity::new("mfm.jcs.v1")
                .expect("canonicalizer"),
            framework_version: events::FrameworkVersion::new("mfm.test.1")
                .expect("framework version"),
            source_revision: events::SourceRevision::new("test-revision").expect("source revision"),
            seed_cells: Vec::new(),
        })
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

    fn request(
        run_id: RunId,
        seq: u64,
        key: &str,
        payloads: Vec<KernelEventPayload>,
    ) -> mfm_store::v1::TypedCommitRequest {
        mfm_store::v1::TypedCommitRequest {
            run_id,
            expected_next_seq: StreamSeq::new(seq).expect("seq"),
            commit_key: CommitKey::new(key).expect("commit key"),
            payloads,
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        }
    }

    #[tokio::test]
    async fn typed_commit_key_sequence_and_projection_rebuild_contract() {
        let (store, schema) = test_store().await;
        let run = run_id(7);
        let artifact = artifact_id(8);
        let digest = content_digest(9);
        store
            .record_artifact_evidence(store_artifact_ref(
                artifact.clone(),
                digest.clone(),
                ArtifactRole::StateOutput,
            ))
            .await
            .expect("artifact evidence");
        store
            .record_artifact_evidence(spec_artifact_ref())
            .await
            .expect("spec artifact evidence");

        store
            .append_typed_run_commit(request(
                run.clone(),
                1,
                "run-start",
                vec![run_started(run.clone())],
            ))
            .await
            .expect("run start");
        store
            .append_typed_run_commit(request(
                run.clone(),
                2,
                "attempt-start",
                vec![state_attempt_started()],
            ))
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
        let appended = store
            .append_typed_run_commit(terminal_request.clone())
            .await
            .expect("terminal commit");
        assert!(matches!(appended, CommitOutcome::Appended(_)));

        let mut stale_retry = terminal_request;
        stale_retry.expected_next_seq = StreamSeq::new(1).expect("stale seq");
        let idempotent = store
            .append_typed_run_commit(stale_retry)
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

        {
            let client = store.client.lock().await;
            client
                .batch_execute(
                    "DELETE FROM typed_run_projection;\
                     DELETE FROM typed_attempt_projection;\
                     DELETE FROM typed_cell_projection;\
                     DELETE FROM typed_fact_projection;\
                     DELETE FROM typed_side_effect_projection;\
                     DELETE FROM typed_public_output_projection;\
                     DELETE FROM typed_retention_projection;\
                     DELETE FROM typed_retention_manifests;",
                )
                .await
                .expect("clear projections");
        }
        let rebuilt = store
            .rebuild_projections_from_events(&run)
            .await
            .expect("rebuild projections");
        assert_eq!(rebuilt, before);

        drop_schema(&store, &schema).await;
    }

    #[tokio::test]
    async fn typed_required_artifacts_and_fact_projection_are_atomic() {
        let (store, schema) = test_store().await;
        let run = run_id(10);
        store
            .record_artifact_evidence(spec_artifact_ref())
            .await
            .expect("spec artifact evidence");
        store
            .append_typed_run_commit(request(
                run.clone(),
                1,
                "run-start",
                vec![run_started(run.clone())],
            ))
            .await
            .expect("run start");

        let missing_artifact = artifact_id(11);
        let missing_digest = content_digest(12);
        let mut fact_request = request(
            run.clone(),
            2,
            "fact",
            vec![fact_recorded(
                missing_artifact.clone(),
                missing_digest.clone(),
            )],
        );
        fact_request.preconditions.required_run_state = RequiredRunState::Started;
        let err = store
            .append_typed_run_commit(fact_request.clone())
            .await
            .expect_err("missing fact artifact");
        assert!(matches!(
            err,
            PostgresTypedStoreError::Store(StoreError::MissingArtifact { .. })
        ));
        assert_eq!(
            store.expected_next_seq(&run).await.expect("next seq"),
            StreamSeq::new(2).expect("seq")
        );

        store
            .record_artifact_evidence(store_artifact_ref(
                missing_artifact.clone(),
                missing_digest,
                ArtifactRole::FactResponse,
            ))
            .await
            .expect("fact artifact evidence");
        store
            .append_typed_run_commit(fact_request)
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
}
