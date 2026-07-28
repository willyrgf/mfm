mod hostile;
mod model;
mod persistence;
mod restore;

pub(super) use hostile::{
    authority_effect, pending_effect, second_resource_effect, CandidateMutation,
};

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use mfm_canonical::{sha256_digest_bytes, CanonicalBytes};
use sqlx::{postgres::PgPoolOptions, AssertSqlSafe, PgPool, Postgres, Row, Transaction};
use thiserror::Error;
use tokio::sync::oneshot;

use model::{
    AppendIdentity, AttemptAuthorization, AttemptObservation, AttemptOutcome, BoundResourceIntent,
    EffectFrontier, EffectProjection, EffectRecord, EffectRecordBody, EvidenceContract,
    FoldedEffect, FoldedResource, ModelError, ResourceAppendIdentity, ResourceHeadProjection,
    ResourceLink, ResourcePolicyConfiguration, ResourceRecord, Tombstone,
};
use persistence::{
    compare_and_swap_resource_head, default_evidence_contract, fold_effect_records,
    insert_effect_record_row, insert_resource_record_row, load_effect_records, load_folded_effect,
    load_resource_fold, persist_effect_derivations,
};

const FENCE_ID: &str = "executor-prototype";
const EXECUTOR_BINDING_REF: &str = "executor-binding:reference-destination:v1";
const DESTINATION_IDENTITY_REF: &str = "destination:reference-account-create:v1";
const EVIDENCE_CONTRACT_REF: &str = "evidence-contract:postgres-prototype:v1";
const EXCLUSIVE_RESOURCE_OWNERSHIP_REF: &str = "resource-owner:postgres-prototype-exclusive:v1";
const DESTINATION_INVENTORY_RESOURCE_OWNERSHIP_REF: &str =
    "resource-owner:postgres-prototype-destination-inventory:v1";
const ACCOUNT_SEQUENCE_RESOURCE_OWNERSHIP_REF: &str =
    "resource-owner:postgres-prototype-account-sequence:v1";
const DESTINATION_INVENTORY_RESOURCE_KEY_REF: &str =
    "resource-key:reference-destination-account-inventory:v1";
const LEDGER_GENERATION: i64 = 73;
pub(super) const MAX_EFFECT_ATTEMPTS: i64 = 64;
const MAX_EFFECT_RECORDS: i64 = 256;
const MAX_EFFECT_RETAINED_BYTES: i64 = 4 * 1024 * 1024;
const COMPLETION_RESERVE_RECORDS: i64 = 2;
const COMPLETION_RESERVE_BYTES: i64 = 16 * 1024;
pub(super) const MAX_EFFECT_ID_UTF8_BYTES: usize = model::MAX_EFFECT_ID_UTF8_BYTES;
static SCHEMA_COUNTER: AtomicU64 = AtomicU64::new(0);

type Result<T> = std::result::Result<T, PrototypeError>;

#[derive(Debug, Error)]
pub(super) enum PrototypeError {
    #[error("DATABASE_URL is required for the PostgreSQL executor prototype")]
    DatabaseUrlUnavailable,
    #[error("PostgreSQL rejected a prototype operation")]
    Database(#[from] sqlx::Error),
    #[error("the prototype schema registry is unavailable")]
    SchemaRegistryUnavailable,
    #[error("the writer or store instance is fenced")]
    WriterFenced,
    #[error("the writer generation changed from {expected} to {actual}")]
    GenerationChanged { expected: i64, actual: i64 },
    #[error("the journal head changed")]
    HeadMismatch,
    #[error("the journal sequence is exhausted")]
    SequenceOverflow,
    #[error("the effect does not exist")]
    MissingEffect,
    #[error("the effect is no longer authorizable")]
    EffectNotAuthorizable,
    #[error("the requested resource is held by another effect")]
    ResourceBusy,
    #[error("the injected transaction failure rolled back")]
    SimulatedRollback,
    #[error("the append outcome was not legal for this executor stage")]
    UnexpectedAppendOutcome,
    #[error("the destination generation is stale")]
    DestinationGenerationFenced,
    #[error("the destination already contains different request content")]
    DestinationConflict,
    #[error("the destination credential was rejected")]
    DestinationCredentialRejected,
    #[error("the finite destination inventory is exhausted")]
    InventoryExhausted,
    #[error("the account-sequence allocation conflicts with its durable binding")]
    SequenceAllocationConflict,
    #[error("the candidate belongs to another lineage")]
    CandidateLineageMismatch,
    #[error("the candidate belongs to another executor binding")]
    CandidateBindingMismatch,
    #[error("the candidate belongs to another durable ledger generation")]
    CandidateLedgerGenerationMismatch,
    #[error("the candidate is missing required stream {stream_id}")]
    MissingCandidate { stream_id: String },
    #[error("the candidate is an ancestor for stream {stream_id}")]
    AncestorCandidate { stream_id: String },
    #[error("the candidate has an extra suffix or stream {stream_id}")]
    ExtraCandidate { stream_id: String },
    #[error("the candidate contains a corrupt chain for stream {stream_id}")]
    CorruptCandidate { stream_id: String },
    #[error("the candidate forks required stream {stream_id}")]
    ForkedCandidate { stream_id: String },
    #[error("the synchronization probe was dropped")]
    ProbeDropped,
    #[error("the inventory size exceeds the prototype integer range")]
    InventorySizeTooLarge,
    #[error("canonical candidate encoding failed")]
    Canonical(#[from] mfm_canonical::CanonicalError),
    #[error("candidate rows did not survive canonical logical round-trip")]
    CanonicalRoundTripMismatch,
    #[error("the restore authority envelope is invalid")]
    RestoreEnvelopeInvalid,
    #[error("candidate row projection failed")]
    Json(#[from] serde_json::Error),
    #[error("forbidden secret or bearer material reached a retained surface")]
    ForbiddenMaterialPersisted,
    #[error("immutable executor record validation failed")]
    Model(#[from] ModelError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct JournalHead {
    sequence: i64,
    digest: Vec<u8>,
}

impl JournalHead {
    pub(super) fn sequence(&self) -> i64 {
        self.sequence
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AppendCommit {
    head: JournalHead,
}

impl AppendCommit {
    pub(super) fn head(&self) -> &JournalHead {
        &self.head
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CommitAcknowledgement {
    DirectlyObserved,
    LostAfterCommit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AuthorizationFailurePoint {
    None,
    RollbackAfterEffectAndResourceCas,
}

#[derive(Clone, Debug)]
pub(super) struct AuthorizationRequest {
    effect_id: String,
    expected_head: JournalHead,
    append_request_id: String,
    opaque_payload: Vec<u8>,
    attempt_ordinal: i64,
    attempt_id: Vec<u8>,
    target_operation_ref: String,
}

impl AuthorizationRequest {
    pub(super) fn with_payload(&self, opaque_payload: &[u8]) -> Self {
        Self {
            effect_id: self.effect_id.clone(),
            expected_head: self.expected_head.clone(),
            append_request_id: self.append_request_id.clone(),
            opaque_payload: opaque_payload.to_vec(),
            attempt_ordinal: self.attempt_ordinal,
            attempt_id: self.attempt_id.clone(),
            target_operation_ref: self.target_operation_ref.clone(),
        }
    }
}

#[derive(Debug)]
pub(super) enum AuthorizationOutcome {
    NewlyAppended {
        commit: AppendCommit,
        target_entry: TargetEntryWitness,
    },
    ExistingSame {
        commit: AppendCommit,
    },
    Conflict {
        commit: AppendCommit,
    },
    OutcomeUnknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Reconciliation {
    Committed { commit: AppendCommit },
    Conflict { commit: AppendCommit },
    NotCommitted,
}

#[must_use]
#[derive(Debug)]
pub(super) struct TargetEntryWitness {
    effect_id: String,
    executor_binding_ref: String,
    ledger_generation: i64,
    effect_key: Vec<u8>,
    request_digest: Vec<u8>,
    attempt_id: Vec<u8>,
    target_operation_ref: String,
    destination_identity: String,
    destination_generation: i64,
}

impl TargetEntryWitness {
    pub(super) fn attempt_id(&self) -> &[u8] {
        &self.attempt_id
    }

    pub(super) fn into_conflicting_request(mut self) -> Self {
        if let Some(first) = self.request_digest.first_mut() {
            *first ^= 0xff;
        }
        self
    }
}

#[must_use]
#[derive(Debug, Eq, PartialEq)]
pub(super) struct DestinationReceipt {
    effect_id: String,
    executor_binding_ref: String,
    ledger_generation: i64,
    effect_key: Vec<u8>,
    request_digest: Vec<u8>,
    attempt_id: Vec<u8>,
    target_operation_ref: String,
    destination_identity: String,
    destination_generation: i64,
    account_id: i64,
}

impl DestinationReceipt {
    pub(super) const fn account_id(&self) -> i64 {
        self.account_id
    }

    pub(super) fn attempt_id(&self) -> &[u8] {
        &self.attempt_id
    }
}

#[must_use]
#[derive(Debug, Eq, PartialEq)]
pub(super) enum DestinationMutation {
    NewlyApplied { receipt: DestinationReceipt },
    ExistingSame { receipt: DestinationReceipt },
}

impl DestinationMutation {
    pub(super) const fn account_id(&self) -> i64 {
        match self {
            Self::NewlyApplied { receipt } | Self::ExistingSame { receipt } => receipt.account_id,
        }
    }

    pub(super) fn into_receipt(self) -> DestinationReceipt {
        match self {
            Self::NewlyApplied { receipt } | Self::ExistingSame { receipt } => receipt,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SequenceAcknowledgement {
    DirectlyObserved,
    LostAfterCommit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SequenceAllocationOutcome {
    NewlyAllocated { sequence: i64 },
    ExistingSame { sequence: i64 },
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EffectState {
    Pending,
    Authorized,
    Observed,
    Tombstoned,
}

impl EffectState {
    fn from_database(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "authorized" => Ok(Self::Authorized),
            "observed" => Ok(Self::Observed),
            "tombstoned" => Ok(Self::Tombstoned),
            _ => Err(PrototypeError::EffectNotAuthorizable),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EffectSnapshot {
    pub(super) state: EffectState,
    pub(super) resource_id: Option<String>,
    pub(super) destination_account: Option<i64>,
    pub(super) authorization_count: i32,
    pub(super) observation_count: i32,
    pub(super) late_observation_count: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CrashPoint {
    Never,
    BeforeAuthorization,
    AfterAuthorization,
    BeforeDestinationMutation,
    AfterDestinationMutation,
    BeforeObservation,
    AfterObservation,
    BeforeTombstone,
    AfterTombstone,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DriveOutcome {
    Completed,
    AlreadyComplete,
    Crashed(CrashPoint),
}

#[derive(Debug)]
pub(super) struct Pause {
    entered: Option<oneshot::Sender<()>>,
    release: oneshot::Receiver<()>,
}

impl Pause {
    async fn wait(mut self) -> Result<()> {
        self.entered
            .take()
            .ok_or(PrototypeError::ProbeDropped)?
            .send(())
            .map_err(|_| PrototypeError::ProbeDropped)?;
        self.release.await.map_err(|_| PrototypeError::ProbeDropped)
    }
}

pub(super) fn pause_pair() -> (Pause, oneshot::Receiver<()>, oneshot::Sender<()>) {
    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    (
        Pause {
            entered: Some(entered_tx),
            release: release_rx,
        },
        entered_rx,
        release_tx,
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SchemaName(String);

impl SchemaName {
    fn generated(label: &str) -> Self {
        assert!(
            label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_'),
            "prototype schema labels are fixed lowercase identifiers"
        );
        let process = std::process::id();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let counter = SCHEMA_COUNTER.fetch_add(1, Ordering::Relaxed);
        Self(format!("mfm_exec_{label}_{process}_{nanos}_{counter}"))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug)]
struct StoreInstance {
    schema: SchemaName,
    lineage_id: String,
    instance_id: String,
    executor_binding_ref: String,
    ledger_generation: i64,
}

#[derive(Clone, Debug)]
pub(super) struct RestoreCandidate {
    instance: StoreInstance,
    canonical_authority: Vec<u8>,
}

#[derive(Clone, Debug)]
struct WriterToken {
    generation: i64,
    writer_id: String,
    destination_generation: i64,
    executor_binding_ref: String,
    ledger_generation: i64,
}

#[derive(Debug)]
struct TopologyInner {
    pool: PgPool,
    control_schema: SchemaName,
    destination_schema: SchemaName,
    schemas: Mutex<Vec<SchemaName>>,
}

#[derive(Clone, Debug)]
pub(super) struct TestTopology {
    inner: Arc<TopologyInner>,
}

#[derive(Clone, Debug)]
pub(super) struct PromotedWriter {
    inner: Arc<TopologyInner>,
    pool: PgPool,
    instance: StoreInstance,
    token: WriterToken,
}

#[derive(Clone, Debug)]
pub(super) struct Executor {
    writer: PromotedWriter,
}

impl Executor {
    pub(super) const fn new(writer: PromotedWriter) -> Self {
        Self { writer }
    }
}

impl TestTopology {
    pub(super) async fn new(inventory_size: usize) -> Result<(Self, PromotedWriter)> {
        let database_url =
            std::env::var("DATABASE_URL").map_err(|_| PrototypeError::DatabaseUrlUnavailable)?;
        let pool = PgPoolOptions::new()
            .max_connections(24)
            .connect(&database_url)
            .await?;
        let control_schema = SchemaName::generated("control");
        let destination_schema = SchemaName::generated("destination");
        let primary_schema = SchemaName::generated("primary");
        let lineage_id = format!("lineage:{}", control_schema.as_str());
        let primary_instance_id = format!("instance:{}", primary_schema.as_str());

        create_control_schema(&pool, &control_schema).await?;
        create_destination_schema(&pool, &destination_schema).await?;
        create_store_schema(
            &pool,
            &primary_schema,
            &lineage_id,
            &primary_instance_id,
            EXECUTOR_BINDING_REF,
            LEDGER_GENERATION,
            true,
        )
        .await?;
        let inventory_size =
            i64::try_from(inventory_size).map_err(|_| PrototypeError::InventorySizeTooLarge)?;
        let inventory_configuration = ResourcePolicyConfiguration::FiniteInventory {
            ordered_allocation_values: (1..=inventory_size)
                .map(|ordinal| 10_000_i64 + ordinal)
                .collect(),
        };
        insert_resource_configuration(
            &pool,
            &primary_schema,
            DESTINATION_INVENTORY_RESOURCE_OWNERSHIP_REF,
            DESTINATION_INVENTORY_RESOURCE_KEY_REF,
            &inventory_configuration,
        )
        .await?;
        let required_configuration_sql = format!(
            "INSERT INTO {}.prototype_required_heads
             (stream_id, sequence, commit_digest)
             VALUES ($1, 1, $2)",
            control_schema.as_str()
        );
        sqlx::query(audited_sql(required_configuration_sql))
            .bind(model::resource_configuration_stream_id(
                DESTINATION_INVENTORY_RESOURCE_OWNERSHIP_REF,
                DESTINATION_INVENTORY_RESOURCE_KEY_REF,
            ))
            .bind(model::resource_policy_configuration_ref(
                &inventory_configuration,
            ))
            .execute(&pool)
            .await?;
        let bootstrap_sql = format!(
            "INSERT INTO {}.prototype_writer_fence
             (fence_id, generation, writer_id, active_instance_id, lineage_id,
              destination_generation, executor_binding_ref, ledger_generation)
             VALUES ($1, 1, $2, $3, $4, 1, $5, $6)",
            control_schema.as_str()
        );
        sqlx::query(audited_sql(bootstrap_sql))
            .bind(FENCE_ID)
            .bind("writer:primary")
            .bind(&primary_instance_id)
            .bind(&lineage_id)
            .bind(EXECUTOR_BINDING_REF)
            .bind(LEDGER_GENERATION)
            .execute(&pool)
            .await?;

        let inner = Arc::new(TopologyInner {
            pool,
            control_schema: control_schema.clone(),
            destination_schema: destination_schema.clone(),
            schemas: Mutex::new(vec![
                control_schema,
                destination_schema,
                primary_schema.clone(),
            ]),
        });
        let topology = Self {
            inner: Arc::clone(&inner),
        };
        let writer = PromotedWriter {
            pool: inner.pool.clone(),
            inner,
            instance: StoreInstance {
                schema: primary_schema,
                lineage_id,
                instance_id: primary_instance_id,
                executor_binding_ref: EXECUTOR_BINDING_REF.to_owned(),
                ledger_generation: LEDGER_GENERATION,
            },
            token: WriterToken {
                generation: 1,
                writer_id: "writer:primary".to_owned(),
                destination_generation: 1,
                executor_binding_ref: EXECUTOR_BINDING_REF.to_owned(),
                ledger_generation: LEDGER_GENERATION,
            },
        };
        Ok((topology, writer))
    }

    pub(super) async fn cleanup(self) -> Result<()> {
        let schemas = self
            .inner
            .schemas
            .lock()
            .map_err(|_| PrototypeError::SchemaRegistryUnavailable)?
            .clone();
        for schema in schemas.into_iter().rev() {
            let sql = format!("DROP SCHEMA IF EXISTS {} CASCADE", schema.as_str());
            sqlx::raw_sql(audited_sql(sql))
                .execute(&self.inner.pool)
                .await?;
        }
        self.inner.pool.close().await;
        Ok(())
    }
}

async fn create_control_schema(pool: &PgPool, schema: &SchemaName) -> Result<()> {
    let sql = format!(
        "CREATE SCHEMA {schema};
         CREATE TABLE {schema}.prototype_writer_fence (
           fence_id TEXT PRIMARY KEY,
           generation BIGINT NOT NULL CHECK (generation > 0),
           writer_id TEXT NOT NULL,
           active_instance_id TEXT NOT NULL,
           lineage_id TEXT NOT NULL,
           destination_generation BIGINT NOT NULL CHECK (destination_generation > 0),
           executor_binding_ref TEXT NOT NULL,
           ledger_generation BIGINT NOT NULL CHECK (ledger_generation > 0)
         );
         CREATE TABLE {schema}.prototype_required_heads (
           stream_id TEXT PRIMARY KEY,
           sequence BIGINT NOT NULL CHECK (sequence > 0),
           commit_digest BYTEA NOT NULL CHECK (octet_length(commit_digest) = 32)
         )",
        schema = schema.as_str()
    );
    sqlx::raw_sql(audited_sql(sql)).execute(pool).await?;
    Ok(())
}

async fn create_destination_schema(pool: &PgPool, schema: &SchemaName) -> Result<()> {
    let sql = format!(
        "CREATE SCHEMA {schema};
         CREATE TABLE {schema}.prototype_destination_fence (
           singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
           generation BIGINT NOT NULL CHECK (generation > 0)
         );
         INSERT INTO {schema}.prototype_destination_fence (singleton, generation)
         VALUES (TRUE, 1);
         CREATE TABLE {schema}.prototype_destination_mutations (
           effect_id TEXT PRIMARY KEY,
           executor_binding_ref TEXT NOT NULL,
           ledger_generation BIGINT NOT NULL CHECK (ledger_generation > 0),
           effect_key BYTEA NOT NULL CHECK (octet_length(effect_key) = 32),
           request_digest BYTEA NOT NULL CHECK (octet_length(request_digest) = 32),
           attempt_id BYTEA NOT NULL CHECK (octet_length(attempt_id) = 32),
           target_operation_ref TEXT NOT NULL,
           destination_identity TEXT NOT NULL,
           destination_generation BIGINT NOT NULL CHECK (destination_generation > 0),
           account_id BIGINT NOT NULL UNIQUE,
           mutation_count INTEGER NOT NULL DEFAULT 1 CHECK (mutation_count = 1)
         );
         CREATE TABLE {schema}.prototype_destination_receipts (
           effect_id TEXT NOT NULL,
           executor_binding_ref TEXT NOT NULL,
           ledger_generation BIGINT NOT NULL CHECK (ledger_generation > 0),
           effect_key BYTEA NOT NULL CHECK (octet_length(effect_key) = 32),
           request_digest BYTEA NOT NULL CHECK (octet_length(request_digest) = 32),
           attempt_id BYTEA NOT NULL CHECK (octet_length(attempt_id) = 32),
           target_operation_ref TEXT NOT NULL,
           destination_identity TEXT NOT NULL,
           destination_generation BIGINT NOT NULL CHECK (destination_generation > 0),
           account_id BIGINT NOT NULL,
           receipt_count INTEGER NOT NULL DEFAULT 1 CHECK (receipt_count = 1),
           PRIMARY KEY (effect_id, attempt_id)
         )",
        schema = schema.as_str()
    );
    sqlx::raw_sql(audited_sql(sql)).execute(pool).await?;
    Ok(())
}

async fn create_store_schema(
    pool: &PgPool,
    schema: &SchemaName,
    lineage_id: &str,
    instance_id: &str,
    executor_binding_ref: &str,
    ledger_generation: i64,
    seed_authority: bool,
) -> Result<()> {
    let sql = format!(
        "CREATE SCHEMA {schema};
         CREATE TABLE {schema}.prototype_instance_identity (
           singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
           lineage_id TEXT NOT NULL,
           instance_id TEXT NOT NULL UNIQUE,
           executor_binding_ref TEXT NOT NULL,
           ledger_generation BIGINT NOT NULL CHECK (ledger_generation > 0)
         );
         CREATE TABLE {schema}.prototype_evidence_contracts (
           contract_ref TEXT PRIMARY KEY,
           max_attempts BIGINT NOT NULL CHECK (max_attempts >= 0),
           max_records BIGINT NOT NULL CHECK (max_records > 0),
           max_retained_bytes BIGINT NOT NULL CHECK (max_retained_bytes > 0),
           completion_reserve_records BIGINT NOT NULL
             CHECK (completion_reserve_records = 2),
           completion_reserve_bytes BIGINT NOT NULL
             CHECK (completion_reserve_bytes > 0)
         );
         CREATE TABLE {schema}.prototype_evidence_objects (
           object_digest BYTEA PRIMARY KEY CHECK (octet_length(object_digest) = 32),
           opaque_bytes BYTEA NOT NULL
         );
         CREATE TABLE {schema}.prototype_journal_records (
           stream_id TEXT NOT NULL,
           sequence BIGINT NOT NULL CHECK (sequence > 0),
           predecessor_digest BYTEA,
           record_kind TEXT NOT NULL,
           opaque_payload BYTEA NOT NULL,
           commit_digest BYTEA NOT NULL CHECK (octet_length(commit_digest) = 32),
           executor_binding_ref TEXT NOT NULL,
           ledger_generation BIGINT NOT NULL CHECK (ledger_generation > 0),
           evidence_object_digest BYTEA NOT NULL
             CHECK (octet_length(evidence_object_digest) = 32),
           proof_digest BYTEA NOT NULL CHECK (octet_length(proof_digest) = 32),
           PRIMARY KEY (stream_id, sequence),
           CHECK (
             (sequence = 1 AND predecessor_digest IS NULL)
             OR
             (sequence > 1 AND octet_length(predecessor_digest) = 32)
           )
         );
         CREATE TABLE {schema}.prototype_effect_records (
           stream_id TEXT NOT NULL,
           sequence BIGINT NOT NULL CHECK (sequence > 0),
           predecessor_digest BYTEA,
           opaque_payload BYTEA NOT NULL,
           executor_binding_ref TEXT NOT NULL,
           ledger_generation BIGINT NOT NULL CHECK (ledger_generation > 0),
           effect_key BYTEA NOT NULL CHECK (octet_length(effect_key) = 32),
           request_digest BYTEA NOT NULL CHECK (octet_length(request_digest) = 32),
           destination_identity TEXT NOT NULL,
           append_request_id TEXT,
           append_predecessor_sequence BIGINT,
           append_predecessor_digest BYTEA,
           append_purpose TEXT,
           append_candidate_digest BYTEA,
           record_kind TEXT NOT NULL CHECK (
             record_kind IN (
               'effect_bound',
               'resource_allocated',
               'delivery_attempt_authorized',
               'delivery_attempt_observed',
               'terminal_tombstone'
             )
           ),
           resource_ownership_ref TEXT,
           policy_ref TEXT,
           policy_configuration_ref BYTEA,
           resource_key_ref TEXT,
           allocation_state_ref BYTEA,
           allocation_value BIGINT,
           fencing_ref BYTEA,
           resource_append_request_id TEXT,
           resource_record_ref BYTEA,
           attempt_ordinal BIGINT,
           attempt_id BYTEA,
           target_operation_ref TEXT,
           observation_outcome TEXT CHECK (
             observation_outcome IS NULL
             OR observation_outcome IN ('returned', 'did_not_enter', 'indeterminate')
           ),
           safe_outcome_ref BYTEA,
           destination_account BIGINT,
           terminal_attempt_id BYTEA,
           external_operation_ref TEXT,
           terminal_outcome_ref BYTEA,
           terminal_proof_ref BYTEA,
           evidence_object_digest BYTEA NOT NULL
             CHECK (octet_length(evidence_object_digest) = 32),
           retained_bytes BIGINT NOT NULL CHECK (retained_bytes > 0),
           commit_digest BYTEA NOT NULL CHECK (octet_length(commit_digest) = 32),
           proof_digest BYTEA NOT NULL CHECK (octet_length(proof_digest) = 32),
           PRIMARY KEY (stream_id, sequence),
           CHECK (
             (sequence = 1 AND predecessor_digest IS NULL)
             OR
             (sequence > 1 AND octet_length(predecessor_digest) = 32)
           ),
           CHECK (
             (
               sequence = 1
               AND append_request_id IS NULL
               AND append_predecessor_sequence IS NULL
               AND append_predecessor_digest IS NULL
               AND append_purpose IS NULL
               AND append_candidate_digest IS NULL
             )
             OR
             (
               sequence > 1
               AND append_request_id IS NOT NULL
               AND append_predecessor_sequence IS NOT NULL
               AND append_predecessor_sequence > 0
               AND append_predecessor_digest IS NOT NULL
               AND octet_length(append_predecessor_digest) = 32
               AND append_purpose IS NOT NULL
               AND append_candidate_digest IS NOT NULL
               AND octet_length(append_candidate_digest) = 32
             )
           )
         );
         CREATE TABLE {schema}.prototype_journal_heads (
           stream_id TEXT PRIMARY KEY,
           sequence BIGINT NOT NULL CHECK (sequence > 0),
           commit_digest BYTEA NOT NULL CHECK (octet_length(commit_digest) = 32)
         );
         CREATE TABLE {schema}.prototype_append_requests (
           stream_id TEXT NOT NULL,
           append_request_id TEXT NOT NULL,
           predecessor_sequence BIGINT NOT NULL CHECK (predecessor_sequence > 0),
           predecessor_digest BYTEA NOT NULL CHECK (octet_length(predecessor_digest) = 32),
           purpose TEXT NOT NULL,
           candidate_digest BYTEA NOT NULL CHECK (octet_length(candidate_digest) = 32),
           committed_sequence BIGINT NOT NULL CHECK (committed_sequence > 1),
           committed_digest BYTEA NOT NULL CHECK (octet_length(committed_digest) = 32),
           PRIMARY KEY (stream_id, append_request_id)
         );
         CREATE TABLE {schema}.prototype_effect_frontiers (
           stream_id TEXT PRIMARY KEY,
           executor_binding_ref TEXT NOT NULL,
           ledger_generation BIGINT NOT NULL CHECK (ledger_generation > 0),
           effect_key BYTEA NOT NULL CHECK (octet_length(effect_key) = 32),
           request_digest BYTEA NOT NULL CHECK (octet_length(request_digest) = 32),
           destination_identity TEXT NOT NULL,
           sequence BIGINT NOT NULL CHECK (sequence > 0),
           predecessor_digest BYTEA,
           evidence_object_digest BYTEA NOT NULL
             CHECK (octet_length(evidence_object_digest) = 32),
           retained_bytes BIGINT NOT NULL CHECK (retained_bytes > 0),
           commit_digest BYTEA NOT NULL CHECK (octet_length(commit_digest) = 32),
           proof_digest BYTEA NOT NULL CHECK (octet_length(proof_digest) = 32)
         );
         CREATE TABLE {schema}.prototype_effect_intents (
           effect_id TEXT PRIMARY KEY,
           stream_id TEXT NOT NULL UNIQUE,
           requested_resource_ownership_ref TEXT,
           requested_resource_key_ref TEXT,
           requested_policy_ref TEXT,
           requested_policy_configuration_ref BYTEA,
           semantic_request_digest BYTEA NOT NULL
             CHECK (octet_length(semantic_request_digest) = 32),
           CHECK (
             (requested_resource_ownership_ref IS NULL
               AND requested_resource_key_ref IS NULL
               AND requested_policy_ref IS NULL
               AND requested_policy_configuration_ref IS NULL)
             OR
             (requested_resource_ownership_ref IS NOT NULL
               AND requested_resource_key_ref IS NOT NULL
               AND requested_policy_ref IS NOT NULL
               AND octet_length(requested_policy_configuration_ref) = 32)
           )
         );
         CREATE TABLE {schema}.prototype_effects (
           effect_id TEXT PRIMARY KEY,
           stream_id TEXT NOT NULL UNIQUE,
           effect_key BYTEA NOT NULL CHECK (octet_length(effect_key) = 32),
           request_digest BYTEA NOT NULL CHECK (octet_length(request_digest) = 32),
           destination_identity TEXT NOT NULL,
           state TEXT NOT NULL CHECK (
             state IN ('pending', 'authorized', 'observed', 'tombstoned')
           ),
           resource_id TEXT,
           resource_ownership_ref TEXT,
           policy_ref TEXT,
           policy_configuration_ref BYTEA,
           allocation_state_ref BYTEA,
           allocation_value BIGINT,
           fencing_ref BYTEA,
           resource_append_request_id TEXT,
           resource_record_ref BYTEA,
           semantic_request_digest BYTEA NOT NULL
             CHECK (octet_length(semantic_request_digest) = 32),
           destination_account BIGINT,
           authorization_count BIGINT NOT NULL DEFAULT 0
             CHECK (authorization_count >= 0),
           observation_count BIGINT NOT NULL DEFAULT 0
             CHECK (observation_count >= 0),
           late_observation_count BIGINT NOT NULL DEFAULT 0
             CHECK (late_observation_count >= 0)
         );
         CREATE TABLE {schema}.prototype_resource_configurations (
           resource_ownership_ref TEXT NOT NULL,
           resource_key_ref TEXT NOT NULL,
           policy_ref TEXT NOT NULL,
           policy_configuration_kind TEXT NOT NULL CHECK (
             policy_configuration_kind IN (
               'finite_inventory',
               'account_sequence',
               'exclusive'
             )
           ),
           finite_allocation_values BIGINT[],
           account_sequence_initial_value BIGINT,
           policy_configuration_ref BYTEA NOT NULL
             CHECK (octet_length(policy_configuration_ref) = 32),
           PRIMARY KEY (resource_ownership_ref, resource_key_ref),
           CHECK (
             (
               policy_configuration_kind = 'finite_inventory'
               AND finite_allocation_values IS NOT NULL
               AND account_sequence_initial_value IS NULL
             )
             OR
             (
               policy_configuration_kind = 'account_sequence'
               AND finite_allocation_values IS NULL
               AND account_sequence_initial_value IS NOT NULL
             )
             OR
             (
               policy_configuration_kind = 'exclusive'
               AND finite_allocation_values IS NULL
               AND account_sequence_initial_value IS NULL
             )
           )
         );
         CREATE TABLE {schema}.prototype_resource_records (
           resource_ownership_ref TEXT NOT NULL,
           policy_ref TEXT NOT NULL,
           policy_configuration_kind TEXT NOT NULL CHECK (
             policy_configuration_kind IN (
               'finite_inventory',
               'account_sequence',
               'exclusive'
             )
           ),
           finite_allocation_values BIGINT[],
           account_sequence_initial_value BIGINT,
           policy_configuration_ref BYTEA NOT NULL
             CHECK (octet_length(policy_configuration_ref) = 32),
           resource_key_ref TEXT NOT NULL,
           sequence BIGINT NOT NULL CHECK (sequence > 0),
           predecessor_ref BYTEA,
           append_request_id TEXT NOT NULL,
           append_predecessor_sequence BIGINT NOT NULL
             CHECK (append_predecessor_sequence >= 0),
           append_predecessor_ref BYTEA,
           linked_effect_stream_id TEXT NOT NULL,
           linked_effect_append_request_id TEXT NOT NULL,
           append_candidate_digest BYTEA NOT NULL
             CHECK (octet_length(append_candidate_digest) = 32),
           effect_key BYTEA NOT NULL CHECK (octet_length(effect_key) = 32),
           allocation_state_ref BYTEA NOT NULL
             CHECK (octet_length(allocation_state_ref) = 32),
           allocation_value BIGINT NOT NULL,
           fencing_ref BYTEA,
           executor_binding_ref TEXT NOT NULL,
           ledger_generation BIGINT NOT NULL CHECK (ledger_generation > 0),
           record_ref BYTEA NOT NULL CHECK (octet_length(record_ref) = 32),
           proof_digest BYTEA NOT NULL CHECK (octet_length(proof_digest) = 32),
           PRIMARY KEY (resource_ownership_ref, resource_key_ref, sequence),
           UNIQUE (resource_ownership_ref, resource_key_ref, append_request_id),
           UNIQUE (record_ref),
           CHECK (
             (sequence = 1 AND predecessor_ref IS NULL)
             OR
             (sequence > 1 AND octet_length(predecessor_ref) = 32)
           ),
           CHECK (
             (append_predecessor_sequence = 0 AND append_predecessor_ref IS NULL)
             OR
             (
               append_predecessor_sequence > 0
               AND octet_length(append_predecessor_ref) = 32
             )
           )
         );
         CREATE TABLE {schema}.prototype_resource_heads (
           resource_ownership_ref TEXT NOT NULL,
           policy_ref TEXT NOT NULL,
           policy_configuration_ref BYTEA NOT NULL
             CHECK (octet_length(policy_configuration_ref) = 32),
           resource_key_ref TEXT NOT NULL,
           sequence BIGINT NOT NULL CHECK (sequence > 0),
           record_ref BYTEA NOT NULL CHECK (octet_length(record_ref) = 32),
           PRIMARY KEY (resource_ownership_ref, resource_key_ref)
         )",
        schema = schema.as_str(),
    );
    sqlx::raw_sql(audited_sql(sql)).execute(pool).await?;
    if !seed_authority {
        return Ok(());
    }
    let identity_sql = format!(
        "INSERT INTO {}.prototype_instance_identity
         (singleton, lineage_id, instance_id, executor_binding_ref, ledger_generation)
         VALUES (TRUE, $1, $2, $3, $4)",
        schema.as_str()
    );
    sqlx::query(audited_sql(identity_sql))
        .bind(lineage_id)
        .bind(instance_id)
        .bind(executor_binding_ref)
        .bind(ledger_generation)
        .execute(pool)
        .await?;
    let contract = default_evidence_contract();
    let contract_sql = format!(
        "INSERT INTO {}.prototype_evidence_contracts
         (contract_ref, max_attempts, max_records, max_retained_bytes,
          completion_reserve_records, completion_reserve_bytes)
         VALUES ($1, $2, $3, $4, $5, $6)",
        schema.as_str()
    );
    sqlx::query(audited_sql(contract_sql))
        .bind(&contract.contract_ref)
        .bind(contract.max_attempts)
        .bind(contract.max_records)
        .bind(contract.max_retained_bytes)
        .bind(contract.completion_reserve_records)
        .bind(contract.completion_reserve_bytes)
        .execute(pool)
        .await?;
    Ok(())
}

fn resource_configuration_columns(
    configuration: &ResourcePolicyConfiguration,
) -> (&'static str, Option<Vec<i64>>, Option<i64>) {
    match configuration {
        ResourcePolicyConfiguration::FiniteInventory {
            ordered_allocation_values,
        } => (
            "finite_inventory",
            Some(ordered_allocation_values.clone()),
            None,
        ),
        ResourcePolicyConfiguration::AccountSequence { initial_value } => {
            ("account_sequence", None, Some(*initial_value))
        }
        ResourcePolicyConfiguration::Exclusive => ("exclusive", None, None),
    }
}

fn resource_configuration_from_row(
    row: &sqlx::postgres::PgRow,
) -> Result<ResourcePolicyConfiguration> {
    let kind: String = row.try_get("policy_configuration_kind")?;
    let finite_values: Option<Vec<i64>> = row.try_get("finite_allocation_values")?;
    let initial_value: Option<i64> = row.try_get("account_sequence_initial_value")?;
    match (kind.as_str(), finite_values, initial_value) {
        ("finite_inventory", Some(ordered_allocation_values), None) => {
            Ok(ResourcePolicyConfiguration::FiniteInventory {
                ordered_allocation_values,
            })
        }
        ("account_sequence", None, Some(initial_value)) => {
            Ok(ResourcePolicyConfiguration::AccountSequence { initial_value })
        }
        ("exclusive", None, None) => Ok(ResourcePolicyConfiguration::Exclusive),
        _ => Err(ModelError::ResourcePolicy.into()),
    }
}

async fn ensure_resource_configuration(
    transaction: &mut Transaction<'_, Postgres>,
    schema: &SchemaName,
    resource_ownership_ref: &str,
    resource_key_ref: &str,
    configuration: &ResourcePolicyConfiguration,
) -> Result<()> {
    let policy_ref = configuration.policy_ref();
    let policy_configuration_ref = model::resource_policy_configuration_ref(configuration);
    let (configuration_kind, finite_values, initial_value) =
        resource_configuration_columns(configuration);
    let insert_sql = format!(
        "INSERT INTO {}.prototype_resource_configurations
         (resource_ownership_ref, resource_key_ref, policy_ref,
          policy_configuration_kind, finite_allocation_values,
          account_sequence_initial_value, policy_configuration_ref)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (resource_ownership_ref, resource_key_ref) DO NOTHING",
        schema.as_str()
    );
    sqlx::query(audited_sql(insert_sql))
        .bind(resource_ownership_ref)
        .bind(resource_key_ref)
        .bind(policy_ref)
        .bind(configuration_kind)
        .bind(finite_values)
        .bind(initial_value)
        .bind(&policy_configuration_ref)
        .execute(&mut **transaction)
        .await?;
    let select_sql = format!(
        "SELECT policy_ref, policy_configuration_kind, finite_allocation_values,
                account_sequence_initial_value, policy_configuration_ref
         FROM {}.prototype_resource_configurations
         WHERE resource_ownership_ref = $1 AND resource_key_ref = $2
         FOR SHARE",
        schema.as_str()
    );
    let row = sqlx::query(audited_sql(select_sql))
        .bind(resource_ownership_ref)
        .bind(resource_key_ref)
        .fetch_one(&mut **transaction)
        .await?;
    let persisted_policy_ref: String = row.try_get("policy_ref")?;
    let persisted_configuration_ref: Vec<u8> = row.try_get("policy_configuration_ref")?;
    if persisted_policy_ref != policy_ref
        || persisted_configuration_ref != policy_configuration_ref
        || resource_configuration_from_row(&row)? != configuration.clone()
    {
        return Err(ModelError::ResourcePolicy.into());
    }
    Ok(())
}

async fn insert_resource_configuration(
    pool: &PgPool,
    schema: &SchemaName,
    resource_ownership_ref: &str,
    resource_key_ref: &str,
    configuration: &ResourcePolicyConfiguration,
) -> Result<()> {
    let mut transaction = pool.begin().await?;
    ensure_resource_configuration(
        &mut transaction,
        schema,
        resource_ownership_ref,
        resource_key_ref,
        configuration,
    )
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn lock_resource_configuration(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    resource_ownership_ref: &str,
    resource_key_ref: &str,
) -> Result<ResourcePolicyConfiguration> {
    let sql = format!(
        "SELECT policy_ref, policy_configuration_kind, finite_allocation_values,
                account_sequence_initial_value, policy_configuration_ref
         FROM {}.prototype_resource_configurations
         WHERE resource_ownership_ref = $1 AND resource_key_ref = $2
         FOR UPDATE",
        instance.schema.as_str()
    );
    let row = sqlx::query(audited_sql(sql))
        .bind(resource_ownership_ref)
        .bind(resource_key_ref)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(ModelError::ResourcePolicy)?;
    let configuration = resource_configuration_from_row(&row)?;
    if row.try_get::<String, _>("policy_ref")? != configuration.policy_ref()
        || row.try_get::<Vec<u8>, _>("policy_configuration_ref")?
            != model::resource_policy_configuration_ref(&configuration)
    {
        return Err(ModelError::ResourcePolicy.into());
    }
    Ok(configuration)
}

fn push_bound_request_part(encoded: &mut Vec<u8>, part: &[u8]) {
    encoded.extend_from_slice(&(part.len() as u64).to_be_bytes());
    encoded.extend_from_slice(part);
}

fn bound_request_payload(
    opaque_semantic_request: &[u8],
    resource: Option<&BoundResourceIntent>,
) -> Vec<u8> {
    let mut encoded = b"mfm.pg-prototype.bound-request.v1".to_vec();
    push_bound_request_part(&mut encoded, opaque_semantic_request);
    if let Some(resource) = resource {
        encoded.push(1);
        push_bound_request_part(&mut encoded, resource.resource_ownership_ref.as_bytes());
        push_bound_request_part(&mut encoded, resource.resource_key_ref.as_bytes());
        push_bound_request_part(&mut encoded, resource.policy_ref.as_bytes());
        push_bound_request_part(&mut encoded, &resource.policy_configuration_ref);
    } else {
        encoded.push(0);
    }
    encoded
}

fn read_bound_request_part<'a>(encoded: &'a [u8], cursor: &mut usize) -> Option<&'a [u8]> {
    let length_end = cursor.checked_add(8)?;
    let length_bytes: [u8; 8] = encoded.get(*cursor..length_end)?.try_into().ok()?;
    *cursor = length_end;
    let length = usize::try_from(u64::from_be_bytes(length_bytes)).ok()?;
    let part_end = cursor.checked_add(length)?;
    let part = encoded.get(*cursor..part_end)?;
    *cursor = part_end;
    Some(part)
}

fn decode_bound_resource_intent(encoded: &[u8]) -> Result<Option<BoundResourceIntent>> {
    const DOMAIN: &[u8] = b"mfm.pg-prototype.bound-request.v1";
    if !encoded.starts_with(DOMAIN) {
        return Err(ModelError::EffectIdentity.into());
    }
    let mut cursor = DOMAIN.len();
    read_bound_request_part(encoded, &mut cursor).ok_or(ModelError::EffectIdentity)?;
    let tag = *encoded.get(cursor).ok_or(ModelError::EffectIdentity)?;
    cursor += 1;
    let resource = match tag {
        0 => None,
        1 => {
            let resource_ownership_ref = String::from_utf8(
                read_bound_request_part(encoded, &mut cursor)
                    .ok_or(ModelError::EffectIdentity)?
                    .to_vec(),
            )
            .map_err(|_| ModelError::EffectIdentity)?;
            let resource_key_ref = String::from_utf8(
                read_bound_request_part(encoded, &mut cursor)
                    .ok_or(ModelError::EffectIdentity)?
                    .to_vec(),
            )
            .map_err(|_| ModelError::EffectIdentity)?;
            let policy_ref = String::from_utf8(
                read_bound_request_part(encoded, &mut cursor)
                    .ok_or(ModelError::EffectIdentity)?
                    .to_vec(),
            )
            .map_err(|_| ModelError::EffectIdentity)?;
            let policy_configuration_ref = read_bound_request_part(encoded, &mut cursor)
                .ok_or(ModelError::EffectIdentity)?
                .to_vec();
            Some(BoundResourceIntent {
                resource_ownership_ref,
                resource_key_ref,
                policy_ref,
                policy_configuration_ref,
            })
        }
        _ => return Err(ModelError::EffectIdentity.into()),
    };
    if cursor != encoded.len() {
        return Err(ModelError::EffectIdentity.into());
    }
    Ok(resource)
}

fn audited_sql(sql: String) -> AssertSqlSafe<String> {
    AssertSqlSafe(sql)
}

fn linked_digest(record_kind: &str, predecessor: Option<&[u8]>, opaque_payload: &[u8]) -> Vec<u8> {
    let mut preimage = Vec::with_capacity(
        record_kind.len() + predecessor.map_or(0, <[u8]>::len) + opaque_payload.len() + 25,
    );
    preimage.extend_from_slice(&(record_kind.len() as u64).to_be_bytes());
    preimage.extend_from_slice(record_kind.as_bytes());
    match predecessor {
        Some(predecessor) => {
            preimage.push(1);
            preimage.extend_from_slice(predecessor);
        }
        None => preimage.push(0),
    }
    preimage.extend_from_slice(&(opaque_payload.len() as u64).to_be_bytes());
    preimage.extend_from_slice(opaque_payload);
    sha256_digest_bytes(&preimage).as_bytes().to_vec()
}

fn evidence_object_digest(opaque_payload: &[u8]) -> Vec<u8> {
    sha256_digest_bytes(opaque_payload).as_bytes().to_vec()
}

fn lower_hex(bytes: &[u8]) -> Vec<u8> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = Vec::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[usize::from(byte >> 4)]);
        encoded.push(HEX[usize::from(byte & 0x0f)]);
    }
    encoded
}

pub(super) fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

pub(super) fn forbidden_encodings(secret: &[u8]) -> Vec<Vec<u8>> {
    let fingerprint = sha256_digest_bytes(secret);
    vec![
        secret.to_vec(),
        lower_hex(secret),
        CanonicalBytes::new(secret).encoded().as_bytes().to_vec(),
        fingerprint.as_bytes().to_vec(),
        lower_hex(fingerprint.as_bytes()),
        CanonicalBytes::new(fingerprint.as_bytes())
            .encoded()
            .as_bytes()
            .to_vec(),
    ]
}

fn binding_proof_digest(
    instance: &StoreInstance,
    stream_id: &str,
    sequence: i64,
    commit_digest: &[u8],
    object_digest: &[u8],
) -> Vec<u8> {
    let mut preimage = Vec::with_capacity(
        instance.executor_binding_ref.len()
            + stream_id.len()
            + commit_digest.len()
            + object_digest.len()
            + 40,
    );
    preimage.extend_from_slice(&(instance.executor_binding_ref.len() as u64).to_be_bytes());
    preimage.extend_from_slice(instance.executor_binding_ref.as_bytes());
    preimage.extend_from_slice(&instance.ledger_generation.to_be_bytes());
    preimage.extend_from_slice(&(stream_id.len() as u64).to_be_bytes());
    preimage.extend_from_slice(stream_id.as_bytes());
    preimage.extend_from_slice(&sequence.to_be_bytes());
    preimage.extend_from_slice(commit_digest);
    preimage.extend_from_slice(object_digest);
    sha256_digest_bytes(&preimage).as_bytes().to_vec()
}

async fn retain_evidence_object(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
    opaque_payload: &[u8],
) -> Result<Vec<u8>> {
    let object_digest = evidence_object_digest(opaque_payload);
    let sql = format!(
        "INSERT INTO {}.prototype_evidence_objects
         (object_digest, opaque_bytes)
         VALUES ($1, $2)
         ON CONFLICT (object_digest) DO NOTHING",
        instance.schema.as_str()
    );
    sqlx::query(audited_sql(sql))
        .bind(&object_digest)
        .bind(opaque_payload)
        .execute(&mut **transaction)
        .await?;
    Ok(object_digest)
}

async fn delete_unreferenced_evidence_objects(
    transaction: &mut Transaction<'_, Postgres>,
    instance: &StoreInstance,
) -> Result<()> {
    let sql = format!(
        "DELETE FROM {schema}.prototype_evidence_objects AS objects
         WHERE NOT EXISTS (
           SELECT 1
           FROM {schema}.prototype_journal_records AS records
           WHERE records.evidence_object_digest = objects.object_digest
         )",
        schema = instance.schema.as_str()
    );
    sqlx::query(audited_sql(sql))
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

#[derive(Clone, Debug)]
struct PreparedAppend {
    stream_id: String,
    predecessor: JournalHead,
    append_request_id: String,
    purpose: String,
    opaque_payload: Vec<u8>,
    candidate_digest: Vec<u8>,
}

impl PreparedAppend {
    fn for_effect(
        stream_id: String,
        predecessor: JournalHead,
        append_request_id: String,
        opaque_payload: &[u8],
        body: &EffectRecordBody,
    ) -> Self {
        let purpose = body.kind().to_owned();
        let digest = model::effect_candidate_digest(
            predecessor.sequence,
            &predecessor.digest,
            &purpose,
            opaque_payload,
            body,
        );
        Self {
            stream_id,
            predecessor,
            append_request_id,
            purpose,
            opaque_payload: opaque_payload.to_vec(),
            candidate_digest: digest,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ExistingAppend {
    Same(AppendCommit),
    Conflict(AppendCommit),
}

fn seal_effect_record(mut record: EffectRecord) -> EffectRecord {
    record.evidence_object_digest = evidence_object_digest(&record.opaque_payload);
    record.retained_bytes = model::retained_effect_record_bytes(&record);
    record.commit_digest = model::effect_record_digest(&record);
    record.proof_digest = model::effect_proof_digest(&record);
    record
}

fn seal_resource_record(mut record: ResourceRecord) -> ResourceRecord {
    record.append.candidate_digest = model::resource_candidate_digest(&record);
    record.record_ref = model::resource_record_ref(&record);
    record.proof_digest = model::resource_proof_digest(&record);
    record
}

fn effect_append_record(
    folded: &FoldedEffect,
    actual_predecessor: &JournalHead,
    candidate: &PreparedAppend,
    body: EffectRecordBody,
) -> Result<EffectRecord> {
    if candidate.stream_id != folded.projection.stream_id || candidate.purpose != body.kind() {
        return Err(ModelError::AppendIdentity.into());
    }
    let next_sequence = actual_predecessor
        .sequence
        .checked_add(1)
        .ok_or(PrototypeError::SequenceOverflow)?;
    Ok(seal_effect_record(EffectRecord {
        stream_id: candidate.stream_id.clone(),
        sequence: next_sequence,
        predecessor_digest: Some(actual_predecessor.digest.clone()),
        opaque_payload: candidate.opaque_payload.clone(),
        executor_binding_ref: folded.frontier.executor_binding_ref.clone(),
        ledger_generation: folded.frontier.ledger_generation,
        effect_key: folded.projection.effect_key.clone(),
        request_digest: folded.projection.request_digest.clone(),
        destination_identity: folded.projection.destination_identity.clone(),
        append: Some(AppendIdentity {
            append_request_id: candidate.append_request_id.clone(),
            expected_predecessor_sequence: candidate.predecessor.sequence,
            expected_predecessor_digest: candidate.predecessor.digest.clone(),
            purpose: candidate.purpose.clone(),
            candidate_digest: candidate.candidate_digest.clone(),
        }),
        body,
        evidence_object_digest: Vec::new(),
        retained_bytes: 0,
        commit_digest: Vec::new(),
        proof_digest: Vec::new(),
    }))
}

impl PromotedWriter {
    pub(super) const fn generation(&self) -> i64 {
        self.token.generation
    }

    pub(super) fn executor_binding_ref(&self) -> &str {
        &self.token.executor_binding_ref
    }

    pub(super) const fn ledger_generation(&self) -> i64 {
        self.token.ledger_generation
    }

    pub(super) async fn independent_pool_writer(&self) -> Result<Self> {
        let options = (*self.pool.connect_options()).clone();
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        Ok(Self {
            inner: Arc::clone(&self.inner),
            pool,
            instance: self.instance.clone(),
            token: self.token.clone(),
        })
    }

    pub(super) async fn seed_effect(
        &self,
        effect_id: &str,
        resource_id: Option<&str>,
        opaque_semantic_request: &[u8],
    ) -> Result<JournalHead> {
        let exclusive_configuration = ResourcePolicyConfiguration::Exclusive;
        match resource_id {
            Some(resource_id) => {
                self.seed_effect_with_resource_intent(
                    effect_id,
                    Some(EXCLUSIVE_RESOURCE_OWNERSHIP_REF),
                    Some(resource_id),
                    Some(exclusive_configuration.policy_ref()),
                    Some(&exclusive_configuration),
                    opaque_semantic_request,
                )
                .await
            }
            None => {
                self.seed_effect_with_resource_intent(
                    effect_id,
                    None,
                    None,
                    None,
                    None,
                    opaque_semantic_request,
                )
                .await
            }
        }
    }

    pub(super) async fn seed_destination_effect(
        &self,
        effect_id: &str,
        opaque_semantic_request: &[u8],
    ) -> Result<JournalHead> {
        self.seed_effect_with_resource_intent(
            effect_id,
            Some(DESTINATION_INVENTORY_RESOURCE_OWNERSHIP_REF),
            Some(DESTINATION_INVENTORY_RESOURCE_KEY_REF),
            Some(
                ResourcePolicyConfiguration::FiniteInventory {
                    ordered_allocation_values: Vec::new(),
                }
                .policy_ref(),
            ),
            None,
            opaque_semantic_request,
        )
        .await
    }

    pub(super) async fn seed_account_sequence_effect(
        &self,
        effect_id: &str,
        account_key: &str,
        opaque_semantic_request: &[u8],
    ) -> Result<JournalHead> {
        self.seed_effect_with_resource_intent(
            effect_id,
            Some(ACCOUNT_SEQUENCE_RESOURCE_OWNERSHIP_REF),
            Some(account_key),
            Some(ResourcePolicyConfiguration::AccountSequence { initial_value: 0 }.policy_ref()),
            None,
            opaque_semantic_request,
        )
        .await
    }

    async fn seed_effect_with_resource_intent(
        &self,
        effect_id: &str,
        resource_ownership_ref: Option<&str>,
        resource_key_ref: Option<&str>,
        expected_policy_ref: Option<&str>,
        configuration_to_ensure: Option<&ResourcePolicyConfiguration>,
        opaque_semantic_request: &[u8],
    ) -> Result<JournalHead> {
        // This precedes the transaction and every durable row construction. The byte bound closes
        // the fixed observation/tombstone completion reserve for all accepted UTF-8 identifiers.
        model::validate_effect_id(effect_id)?;
        if resource_ownership_ref.is_some() != resource_key_ref.is_some()
            || resource_ownership_ref.is_some() != expected_policy_ref.is_some()
        {
            return Err(ModelError::ResourcePolicy.into());
        }
        let stream_id = effect_stream_id(effect_id);
        let mut transaction = self.pool.begin().await?;
        self.validate_writer(&mut transaction).await?;

        if let Some(configuration) = configuration_to_ensure {
            let resource_ownership_ref =
                resource_ownership_ref.ok_or(ModelError::ResourcePolicy)?;
            let resource_key_ref = resource_key_ref.ok_or(ModelError::ResourcePolicy)?;
            ensure_resource_configuration(
                &mut transaction,
                &self.instance.schema,
                resource_ownership_ref,
                resource_key_ref,
                configuration,
            )
            .await?;
            self.retain_required_head(
                &mut transaction,
                &model::resource_configuration_stream_id(resource_ownership_ref, resource_key_ref),
                &JournalHead {
                    sequence: 1,
                    digest: model::resource_policy_configuration_ref(configuration),
                },
            )
            .await?;
        }
        let bound_resource_intent = if let (
            Some(resource_ownership_ref),
            Some(resource_key_ref),
            Some(expected_policy_ref),
        ) = (
            resource_ownership_ref,
            resource_key_ref,
            expected_policy_ref,
        ) {
            let configuration = lock_resource_configuration(
                &mut transaction,
                &self.instance,
                resource_ownership_ref,
                resource_key_ref,
            )
            .await?;
            if configuration.policy_ref() != expected_policy_ref {
                return Err(ModelError::ResourcePolicy.into());
            }
            Some(BoundResourceIntent {
                resource_ownership_ref: resource_ownership_ref.to_owned(),
                resource_key_ref: resource_key_ref.to_owned(),
                policy_ref: configuration.policy_ref().to_owned(),
                policy_configuration_ref: model::resource_policy_configuration_ref(&configuration),
            })
        } else {
            None
        };
        let opaque_payload =
            bound_request_payload(opaque_semantic_request, bound_resource_intent.as_ref());
        let semantic_request_digest =
            model::request_digest(DESTINATION_IDENTITY_REF, &opaque_payload);
        let record = seal_effect_record(EffectRecord {
            stream_id: stream_id.clone(),
            sequence: 1,
            predecessor_digest: None,
            opaque_payload,
            executor_binding_ref: self.instance.executor_binding_ref.clone(),
            ledger_generation: self.instance.ledger_generation,
            effect_key: model::effect_key(
                &self.instance.executor_binding_ref,
                self.instance.ledger_generation,
                effect_id,
            ),
            request_digest: semantic_request_digest.clone(),
            destination_identity: DESTINATION_IDENTITY_REF.to_owned(),
            append: None,
            body: EffectRecordBody::EffectBound,
            evidence_object_digest: Vec::new(),
            retained_bytes: 0,
            commit_digest: Vec::new(),
            proof_digest: Vec::new(),
        });
        let intent_sql = format!(
            "INSERT INTO {}.prototype_effect_intents
             (effect_id, stream_id, requested_resource_ownership_ref,
              requested_resource_key_ref, requested_policy_ref,
              requested_policy_configuration_ref, semantic_request_digest)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            self.instance.schema.as_str()
        );
        sqlx::query(audited_sql(intent_sql))
            .bind(effect_id)
            .bind(&stream_id)
            .bind(
                bound_resource_intent
                    .as_ref()
                    .map(|intent| &intent.resource_ownership_ref),
            )
            .bind(
                bound_resource_intent
                    .as_ref()
                    .map(|intent| &intent.resource_key_ref),
            )
            .bind(
                bound_resource_intent
                    .as_ref()
                    .map(|intent| &intent.policy_ref),
            )
            .bind(
                bound_resource_intent
                    .as_ref()
                    .map(|intent| &intent.policy_configuration_ref),
            )
            .bind(&semantic_request_digest)
            .execute(&mut *transaction)
            .await?;
        insert_effect_record_row(&mut transaction, &self.instance, &record).await?;
        let head = JournalHead {
            sequence: 1,
            digest: record.commit_digest.clone(),
        };
        let head_sql = format!(
            "INSERT INTO {}.prototype_journal_heads
             (stream_id, sequence, commit_digest)
             VALUES ($1, $2, $3)",
            self.instance.schema.as_str()
        );
        sqlx::query(audited_sql(head_sql))
            .bind(&stream_id)
            .bind(head.sequence)
            .bind(&head.digest)
            .execute(&mut *transaction)
            .await?;
        let folded =
            model::fold_effect(std::slice::from_ref(&record), &default_evidence_contract())?;
        persist_effect_derivations(&mut transaction, &self.instance, &folded).await?;
        self.retain_required_head(&mut transaction, &stream_id, &head)
            .await?;
        transaction.commit().await?;
        Ok(head)
    }

    pub(super) async fn current_head(&self, effect_id: &str) -> Result<JournalHead> {
        self.current_stream_head(&effect_stream_id(effect_id)).await
    }

    pub(super) async fn journal_payload(&self, effect_id: &str, sequence: i64) -> Result<Vec<u8>> {
        let sql = format!(
            "SELECT opaque_payload
             FROM {}.prototype_journal_records
             WHERE stream_id = $1 AND sequence = $2",
            self.instance.schema.as_str()
        );
        sqlx::query_scalar(audited_sql(sql))
            .bind(effect_stream_id(effect_id))
            .bind(sequence)
            .fetch_one(&self.pool)
            .await
            .map_err(PrototypeError::from)
    }

    pub(super) async fn authorization_request(
        &self,
        effect_id: &str,
        append_request_id: &str,
        opaque_payload: &[u8],
    ) -> Result<AuthorizationRequest> {
        let mut transaction = self.pool.begin().await?;
        let folded = load_folded_effect(&mut transaction, &self.instance, effect_id).await?;
        self.validate_folded_resource_link(&mut transaction, &folded)
            .await?;
        transaction.commit().await?;
        let attempt_ordinal = folded.projection.authorization_count;
        let target_operation_ref = format!("destination-operation:{effect_id}");
        let attempt_id = model::derive_attempt_id(
            &self.instance.executor_binding_ref,
            self.instance.ledger_generation,
            &folded.projection.effect_key,
            &folded.projection.request_digest,
            attempt_ordinal,
            &target_operation_ref,
        );
        Ok(AuthorizationRequest {
            effect_id: effect_id.to_owned(),
            expected_head: JournalHead {
                sequence: folded.frontier.sequence,
                digest: folded.frontier.commit_digest,
            },
            append_request_id: append_request_id.to_owned(),
            opaque_payload: opaque_payload.to_vec(),
            attempt_ordinal,
            attempt_id,
            target_operation_ref,
        })
    }

    async fn append_configured_resource(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        folded: FoldedEffect,
        resource_ownership_ref: &str,
        resource_key_ref: &str,
        append_identity: &str,
        allocation_append_request_id: &str,
    ) -> Result<(FoldedEffect, i64)> {
        if folded.projection.resource_record_ref.is_some() {
            return Err(ModelError::ResourceLink.into());
        }
        // Cross-stream mutations always lock the writer fence and effect head before this
        // immutable configuration row, then advance the rebuildable resource head by CAS.
        let policy_configuration = lock_resource_configuration(
            transaction,
            &self.instance,
            resource_ownership_ref,
            resource_key_ref,
        )
        .await?;
        let existing = load_resource_fold(
            transaction,
            &self.instance,
            resource_ownership_ref,
            resource_key_ref,
        )
        .await?;
        if let Some(existing) = &existing {
            let first = existing.records.first().ok_or(ModelError::ResourceChain)?;
            if first.policy_configuration != policy_configuration {
                return Err(ModelError::ResourcePolicy.into());
            }
        }
        let sequence = existing.as_ref().map_or(Ok(1_i64), |resource| {
            resource
                .head
                .sequence
                .checked_add(1)
                .ok_or(PrototypeError::SequenceOverflow)
        })?;
        let allocation_value = match &policy_configuration {
            ResourcePolicyConfiguration::FiniteInventory {
                ordered_allocation_values,
            } => {
                let index = usize::try_from(sequence - 1)
                    .map_err(|_| PrototypeError::InventoryExhausted)?;
                *ordered_allocation_values
                    .get(index)
                    .ok_or(PrototypeError::InventoryExhausted)?
            }
            ResourcePolicyConfiguration::AccountSequence { initial_value } => initial_value
                .checked_add(sequence - 1)
                .ok_or(PrototypeError::SequenceOverflow)?,
            ResourcePolicyConfiguration::Exclusive => {
                if existing.is_some() {
                    return Err(PrototypeError::ResourceBusy);
                }
                1
            }
        };
        let expected_head = existing.as_ref().map(|resource| resource.head.clone());
        let predecessor_ref = expected_head.as_ref().map(|head| head.record_ref.clone());
        let policy_ref = policy_configuration.policy_ref().to_owned();
        let policy_configuration_ref =
            model::resource_policy_configuration_ref(&policy_configuration);
        let resource_append_request_id = format!("resource-record:{append_identity}");
        let allocation_append_request_id = allocation_append_request_id.to_owned();
        let fencing_ref = matches!(
            &policy_configuration,
            ResourcePolicyConfiguration::Exclusive
        )
        .then(|| {
            sha256_digest_bytes(
                format!(
                    "{}:{}:{}:{}",
                    self.instance.executor_binding_ref,
                    self.instance.ledger_generation,
                    resource_ownership_ref,
                    resource_key_ref
                )
                .as_bytes(),
            )
            .as_bytes()
            .to_vec()
        });
        let resource_record = seal_resource_record(ResourceRecord {
            resource_ownership_ref: resource_ownership_ref.to_owned(),
            policy_ref: policy_ref.clone(),
            policy_configuration,
            policy_configuration_ref: policy_configuration_ref.clone(),
            resource_key_ref: resource_key_ref.to_owned(),
            sequence,
            predecessor_ref: predecessor_ref.clone(),
            append: ResourceAppendIdentity {
                append_request_id: resource_append_request_id.clone(),
                expected_predecessor_sequence: sequence - 1,
                expected_predecessor_ref: predecessor_ref,
                linked_effect_stream_id: folded.projection.stream_id.clone(),
                linked_effect_append_request_id: allocation_append_request_id.clone(),
                candidate_digest: Vec::new(),
            },
            effect_key: folded.projection.effect_key.clone(),
            typed_allocation_state_ref: model::allocation_state_ref(
                &policy_ref,
                &policy_configuration_ref,
                resource_key_ref,
                allocation_value,
            ),
            allocation_value,
            fencing_ref: fencing_ref.clone(),
            executor_binding_ref: self.instance.executor_binding_ref.clone(),
            ledger_generation: self.instance.ledger_generation,
            record_ref: Vec::new(),
            proof_digest: Vec::new(),
        });
        insert_resource_record_row(transaction, &self.instance, &resource_record).await?;
        let mut resource_records = existing.map_or_else(Vec::new, |resource| resource.records);
        resource_records.push(resource_record.clone());
        let folded_resource = model::fold_resource(&resource_records)?;
        compare_and_swap_resource_head(
            transaction,
            &self.instance,
            expected_head.as_ref(),
            &folded_resource,
        )
        .await?;
        self.retain_required_head(
            transaction,
            &model::resource_stream_id(resource_ownership_ref, resource_key_ref),
            &JournalHead {
                sequence: folded_resource.head.sequence,
                digest: folded_resource.head.record_ref.clone(),
            },
        )
        .await?;

        let allocation_body = EffectRecordBody::ResourceAllocated(ResourceLink {
            resource_ownership_ref: resource_ownership_ref.to_owned(),
            policy_ref,
            policy_configuration_ref,
            resource_key_ref: resource_key_ref.to_owned(),
            typed_allocation_state_ref: resource_record.typed_allocation_state_ref.clone(),
            allocation_value,
            fencing_ref,
            resource_append_request_id,
            resource_record_ref: resource_record.record_ref,
        });
        let predecessor = JournalHead {
            sequence: folded.frontier.sequence,
            digest: folded.frontier.commit_digest.clone(),
        };
        let allocation_candidate = PreparedAppend::for_effect(
            folded.projection.stream_id.clone(),
            predecessor.clone(),
            allocation_append_request_id,
            resource_key_ref.as_bytes(),
            &allocation_body,
        );
        self.insert_effect_append(
            transaction,
            &folded,
            &predecessor,
            &allocation_candidate,
            allocation_body,
        )
        .await?;
        let records =
            load_effect_records(transaction, &self.instance, &folded.projection.stream_id).await?;
        let allocated_fold = fold_effect_records(transaction, &self.instance, &records).await?;
        persist_effect_derivations(transaction, &self.instance, &allocated_fold).await?;
        self.validate_folded_resource_link(transaction, &allocated_fold)
            .await?;
        Ok((allocated_fold, allocation_value))
    }

    pub(super) async fn authorize_effect(
        &self,
        request: &AuthorizationRequest,
        acknowledgement: CommitAcknowledgement,
        failure_point: AuthorizationFailurePoint,
    ) -> Result<AuthorizationOutcome> {
        let stream_id = effect_stream_id(&request.effect_id);
        let authorization_body =
            EffectRecordBody::DeliveryAttemptAuthorized(AttemptAuthorization {
                attempt_ordinal: request.attempt_ordinal,
                attempt_id: request.attempt_id.clone(),
                target_operation_ref: request.target_operation_ref.clone(),
            });
        let candidate = PreparedAppend::for_effect(
            stream_id.clone(),
            request.expected_head.clone(),
            request.append_request_id.clone(),
            &request.opaque_payload,
            &authorization_body,
        );
        let mut transaction = self.pool.begin().await?;
        self.validate_writer(&mut transaction).await?;

        if let Some(existing) = self
            .classify_existing_append(&mut transaction, &candidate)
            .await?
        {
            let existing_fold =
                load_folded_effect(&mut transaction, &self.instance, &request.effect_id).await?;
            self.validate_folded_resource_link(&mut transaction, &existing_fold)
                .await?;
            transaction.commit().await?;
            return Ok(match existing {
                ExistingAppend::Same(commit) => AuthorizationOutcome::ExistingSame { commit },
                ExistingAppend::Conflict(commit) => AuthorizationOutcome::Conflict { commit },
            });
        }
        if let Err(error) = self
            .lock_and_require_head(&mut transaction, &candidate)
            .await
        {
            if matches!(error, PrototypeError::HeadMismatch) {
                if let Some(existing) = self
                    .classify_existing_append(&mut transaction, &candidate)
                    .await?
                {
                    transaction.commit().await?;
                    return Ok(match existing {
                        ExistingAppend::Same(commit) => {
                            AuthorizationOutcome::ExistingSame { commit }
                        }
                        ExistingAppend::Conflict(commit) => {
                            AuthorizationOutcome::Conflict { commit }
                        }
                    });
                }
            }
            return Err(error);
        }

        let folded =
            load_folded_effect(&mut transaction, &self.instance, &request.effect_id).await?;
        if folded.frontier.sequence != request.expected_head.sequence
            || folded.frontier.commit_digest != request.expected_head.digest
            || folded.projection.authorization_count != request.attempt_ordinal
            || request.attempt_id
                != model::derive_attempt_id(
                    &self.instance.executor_binding_ref,
                    self.instance.ledger_generation,
                    &folded.projection.effect_key,
                    &folded.projection.request_digest,
                    request.attempt_ordinal,
                    &request.target_operation_ref,
                )
        {
            return Err(PrototypeError::HeadMismatch);
        }
        let records = load_effect_records(&mut transaction, &self.instance, &stream_id).await?;
        let bound_record = records.first().ok_or(ModelError::EffectIdentity)?;
        let authenticated_intent = decode_bound_resource_intent(&bound_record.opaque_payload)?;
        let mut working_fold = folded;
        match authenticated_intent.as_ref() {
            Some(resource_intent) => {
                if working_fold.projection.resource_record_ref.is_none() {
                    let append_identity =
                        format!("{}:{}", request.effect_id, request.append_request_id);
                    let allocation_append_request_id =
                        model::bundled_resource_allocation_append_request_id(
                            &request.effect_id,
                            &request.append_request_id,
                        );
                    (working_fold, _) = self
                        .append_configured_resource(
                            &mut transaction,
                            working_fold,
                            &resource_intent.resource_ownership_ref,
                            &resource_intent.resource_key_ref,
                            &append_identity,
                            &allocation_append_request_id,
                        )
                        .await?;
                }
                if working_fold.projection.resource_ownership_ref.as_deref()
                    != Some(resource_intent.resource_ownership_ref.as_str())
                    || working_fold.projection.resource_key_ref.as_deref()
                        != Some(resource_intent.resource_key_ref.as_str())
                    || working_fold.projection.policy_ref.as_deref()
                        != Some(resource_intent.policy_ref.as_str())
                    || working_fold.projection.policy_configuration_ref.as_deref()
                        != Some(resource_intent.policy_configuration_ref.as_slice())
                {
                    return Err(ModelError::ResourceLink.into());
                }
                self.validate_folded_resource_link(&mut transaction, &working_fold)
                    .await?;
            }
            None if working_fold.projection.resource_record_ref.is_none() => {}
            _ => return Err(ModelError::ResourceLink.into()),
        }
        let actual_predecessor = JournalHead {
            sequence: working_fold.frontier.sequence,
            digest: working_fold.frontier.commit_digest.clone(),
        };
        let preview = effect_append_record(
            &working_fold,
            &actual_predecessor,
            &candidate,
            authorization_body.clone(),
        )?;
        model::ensure_authorization_capacity(
            &working_fold,
            &default_evidence_contract(),
            preview.retained_bytes,
        )?;
        let commit = self
            .insert_effect_append(
                &mut transaction,
                &working_fold,
                &actual_predecessor,
                &candidate,
                authorization_body,
            )
            .await?;
        let records = load_effect_records(&mut transaction, &self.instance, &stream_id).await?;
        let committed_fold =
            fold_effect_records(&mut transaction, &self.instance, &records).await?;
        persist_effect_derivations(&mut transaction, &self.instance, &committed_fold).await?;
        self.validate_folded_resource_link(&mut transaction, &committed_fold)
            .await?;
        if failure_point == AuthorizationFailurePoint::RollbackAfterEffectAndResourceCas {
            transaction.rollback().await?;
            return Err(PrototypeError::SimulatedRollback);
        }
        transaction.commit().await?;
        Ok(match acknowledgement {
            CommitAcknowledgement::DirectlyObserved => AuthorizationOutcome::NewlyAppended {
                commit,
                target_entry: TargetEntryWitness {
                    effect_id: request.effect_id.clone(),
                    executor_binding_ref: committed_fold.frontier.executor_binding_ref.clone(),
                    ledger_generation: committed_fold.frontier.ledger_generation,
                    effect_key: committed_fold.projection.effect_key.clone(),
                    request_digest: committed_fold.projection.request_digest.clone(),
                    attempt_id: request.attempt_id.clone(),
                    target_operation_ref: request.target_operation_ref.clone(),
                    destination_identity: committed_fold.projection.destination_identity.clone(),
                    destination_generation: self.token.destination_generation,
                },
            },
            CommitAcknowledgement::LostAfterCommit => AuthorizationOutcome::OutcomeUnknown,
        })
    }

    pub(super) async fn reconcile_authorization(
        &self,
        request: &AuthorizationRequest,
    ) -> Result<Reconciliation> {
        let body = EffectRecordBody::DeliveryAttemptAuthorized(AttemptAuthorization {
            attempt_ordinal: request.attempt_ordinal,
            attempt_id: request.attempt_id.clone(),
            target_operation_ref: request.target_operation_ref.clone(),
        });
        let candidate = PreparedAppend::for_effect(
            effect_stream_id(&request.effect_id),
            request.expected_head.clone(),
            request.append_request_id.clone(),
            &request.opaque_payload,
            &body,
        );
        let mut transaction = self.pool.begin().await?;
        let classification = self
            .classify_existing_append(&mut transaction, &candidate)
            .await?;
        if classification.is_some() {
            let folded =
                load_folded_effect(&mut transaction, &self.instance, &request.effect_id).await?;
            self.validate_folded_resource_link(&mut transaction, &folded)
                .await?;
        }
        transaction.commit().await?;
        Ok(match classification {
            Some(ExistingAppend::Same(commit)) => Reconciliation::Committed { commit },
            Some(ExistingAppend::Conflict(commit)) => Reconciliation::Conflict { commit },
            None => Reconciliation::NotCommitted,
        })
    }

    pub(super) async fn reload_authorization(
        &self,
        effect_id: &str,
        append_request_id: &str,
    ) -> Result<Option<AppendCommit>> {
        let sql = format!(
            "SELECT committed_sequence, committed_digest
             FROM {}.prototype_append_requests
             WHERE stream_id = $1 AND append_request_id = $2
               AND purpose = 'delivery_attempt_authorized'",
            self.instance.schema.as_str()
        );
        let row = sqlx::query(audited_sql(sql))
            .bind(effect_stream_id(effect_id))
            .bind(append_request_id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|row| {
            Ok(AppendCommit {
                head: JournalHead {
                    sequence: row.try_get("committed_sequence")?,
                    digest: row.try_get("committed_digest")?,
                },
            })
        })
        .transpose()
    }

    pub(super) async fn append_request_count(&self, effect_id: &str) -> Result<i64> {
        let sql = format!(
            "SELECT COUNT(*)
             FROM {}.prototype_append_requests
             WHERE stream_id = $1",
            self.instance.schema.as_str()
        );
        sqlx::query_scalar(audited_sql(sql))
            .bind(effect_stream_id(effect_id))
            .fetch_one(&self.pool)
            .await
            .map_err(PrototypeError::from)
    }

    pub(super) async fn effect_intent_count(&self, effect_id: &str) -> Result<i64> {
        let sql = format!(
            "SELECT COUNT(*)
             FROM {}.prototype_effect_intents
             WHERE effect_id = $1",
            self.instance.schema.as_str()
        );
        sqlx::query_scalar(audited_sql(sql))
            .bind(effect_id)
            .fetch_one(&self.pool)
            .await
            .map_err(PrototypeError::from)
    }

    pub(super) async fn completion_record_bytes(&self, effect_id: &str) -> Result<(i64, i64)> {
        let sql = format!(
            "SELECT retained_bytes
             FROM {}.prototype_effect_records
             WHERE stream_id = $1 AND record_kind = $2",
            self.instance.schema.as_str()
        );
        let stream_id = effect_stream_id(effect_id);
        let observation_bytes = sqlx::query_scalar(audited_sql(sql.clone()))
            .bind(&stream_id)
            .bind("delivery_attempt_observed")
            .fetch_one(&self.pool)
            .await?;
        let tombstone_bytes = sqlx::query_scalar(audited_sql(sql))
            .bind(stream_id)
            .bind("terminal_tombstone")
            .fetch_one(&self.pool)
            .await?;
        Ok((observation_bytes, tombstone_bytes))
    }

    pub(super) async fn effect_snapshot(&self, effect_id: &str) -> Result<EffectSnapshot> {
        let mut transaction = self.pool.begin().await?;
        let folded = load_folded_effect(&mut transaction, &self.instance, effect_id).await?;
        self.validate_folded_resource_link(&mut transaction, &folded)
            .await?;
        transaction.commit().await?;
        Ok(EffectSnapshot {
            state: EffectState::from_database(&folded.projection.state)?,
            resource_id: folded.projection.resource_key_ref,
            destination_account: folded.projection.destination_account,
            authorization_count: i32::try_from(folded.projection.authorization_count)
                .map_err(|_| PrototypeError::SequenceOverflow)?,
            observation_count: i32::try_from(folded.projection.observation_count)
                .map_err(|_| PrototypeError::SequenceOverflow)?,
            late_observation_count: i32::try_from(folded.projection.late_observation_count)
                .map_err(|_| PrototypeError::SequenceOverflow)?,
        })
    }

    pub(super) async fn corrupt_effect_intent_resource_key(&self, effect_id: &str) -> Result<()> {
        let sql = format!(
            "UPDATE {}.prototype_effect_intents
             SET requested_resource_key_ref = requested_resource_key_ref || ':corrupt'
             WHERE effect_id = $1",
            self.instance.schema.as_str()
        );
        sqlx::query(audited_sql(sql))
            .bind(effect_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub(super) async fn resource_holder(&self, resource_id: &str) -> Result<Option<String>> {
        let sql = format!(
            "SELECT effect_id
             FROM {}.prototype_effects
             WHERE resource_ownership_ref = $1
               AND resource_id = $2
               AND resource_record_ref IS NOT NULL
             ORDER BY effect_id",
            self.instance.schema.as_str()
        );
        let holders: Vec<String> = sqlx::query_scalar(audited_sql(sql))
            .bind(EXCLUSIVE_RESOURCE_OWNERSHIP_REF)
            .bind(resource_id)
            .fetch_all(&self.pool)
            .await?;
        match holders.as_slice() {
            [] => Ok(None),
            [effect_id] => {
                let mut transaction = self.pool.begin().await?;
                let folded =
                    load_folded_effect(&mut transaction, &self.instance, effect_id).await?;
                self.validate_folded_resource_link(&mut transaction, &folded)
                    .await?;
                transaction.commit().await?;
                if folded.projection.resource_key_ref.as_deref() != Some(resource_id) {
                    return Err(ModelError::Projection.into());
                }
                Ok(Some(effect_id.clone()))
            }
            _ => Err(ModelError::Projection.into()),
        }
    }

    pub(super) async fn append_marker(
        &self,
        effect_id: &str,
        append_request_id: &str,
        opaque_payload: &[u8],
    ) -> Result<AppendCommit> {
        let head = self.current_head(effect_id).await?;
        self.append_marker_from_head(effect_id, head, append_request_id, opaque_payload, None)
            .await
    }

    pub(super) async fn append_marker_from_head(
        &self,
        effect_id: &str,
        predecessor: JournalHead,
        append_request_id: &str,
        opaque_payload: &[u8],
        pause: Option<Pause>,
    ) -> Result<AppendCommit> {
        let mut transaction = self.pool.begin().await?;
        self.validate_writer(&mut transaction).await?;
        let folded = load_folded_effect(&mut transaction, &self.instance, effect_id).await?;
        let target_operation_ref = format!("prototype-marker:{append_request_id}");
        let body = EffectRecordBody::DeliveryAttemptAuthorized(AttemptAuthorization {
            attempt_ordinal: folded.projection.authorization_count,
            attempt_id: model::derive_attempt_id(
                &self.instance.executor_binding_ref,
                self.instance.ledger_generation,
                &folded.projection.effect_key,
                &folded.projection.request_digest,
                folded.projection.authorization_count,
                &target_operation_ref,
            ),
            target_operation_ref,
        });
        let candidate = PreparedAppend::for_effect(
            effect_stream_id(effect_id),
            predecessor.clone(),
            append_request_id.to_owned(),
            opaque_payload,
            &body,
        );
        if let Some(existing) = self
            .classify_existing_append(&mut transaction, &candidate)
            .await?
        {
            transaction.commit().await?;
            return match existing {
                ExistingAppend::Same(commit) => Ok(commit),
                ExistingAppend::Conflict(_) => Err(PrototypeError::UnexpectedAppendOutcome),
            };
        }
        self.lock_and_require_head(&mut transaction, &candidate)
            .await?;
        if let Some(pause) = pause {
            pause.wait().await?;
        }
        let commit = self
            .insert_effect_append(&mut transaction, &folded, &predecessor, &candidate, body)
            .await?;
        let records =
            load_effect_records(&mut transaction, &self.instance, &candidate.stream_id).await?;
        let committed_fold =
            fold_effect_records(&mut transaction, &self.instance, &records).await?;
        persist_effect_derivations(&mut transaction, &self.instance, &committed_fold).await?;
        self.validate_folded_resource_link(&mut transaction, &committed_fold)
            .await?;
        transaction.commit().await?;
        Ok(commit)
    }

    async fn current_stream_head(&self, stream_id: &str) -> Result<JournalHead> {
        let sql = format!(
            "SELECT sequence, commit_digest
             FROM {}.prototype_journal_heads
             WHERE stream_id = $1",
            self.instance.schema.as_str()
        );
        let row = sqlx::query(audited_sql(sql))
            .bind(stream_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(PrototypeError::HeadMismatch)?;
        Ok(JournalHead {
            sequence: row.try_get("sequence")?,
            digest: row.try_get("commit_digest")?,
        })
    }

    async fn validate_writer(&self, transaction: &mut Transaction<'_, Postgres>) -> Result<()> {
        let fence_sql = format!(
            "SELECT generation, writer_id, active_instance_id, lineage_id,
                    destination_generation, executor_binding_ref, ledger_generation
             FROM {}.prototype_writer_fence
             WHERE fence_id = $1
             FOR SHARE",
            self.inner.control_schema.as_str()
        );
        let fence = sqlx::query(audited_sql(fence_sql))
            .bind(FENCE_ID)
            .fetch_one(&mut **transaction)
            .await?;
        let generation: i64 = fence.try_get("generation")?;
        let writer_id: String = fence.try_get("writer_id")?;
        let active_instance_id: String = fence.try_get("active_instance_id")?;
        let lineage_id: String = fence.try_get("lineage_id")?;
        let destination_generation: i64 = fence.try_get("destination_generation")?;
        let executor_binding_ref: String = fence.try_get("executor_binding_ref")?;
        let ledger_generation: i64 = fence.try_get("ledger_generation")?;
        if generation != self.token.generation
            || writer_id != self.token.writer_id
            || active_instance_id != self.instance.instance_id
            || lineage_id != self.instance.lineage_id
            || destination_generation != self.token.destination_generation
            || executor_binding_ref != self.token.executor_binding_ref
            || ledger_generation != self.token.ledger_generation
        {
            return Err(PrototypeError::WriterFenced);
        }

        let identity_sql = format!(
            "SELECT lineage_id, instance_id, executor_binding_ref, ledger_generation
             FROM {}.prototype_instance_identity
             WHERE singleton = TRUE",
            self.instance.schema.as_str()
        );
        let identity = sqlx::query(audited_sql(identity_sql))
            .fetch_one(&mut **transaction)
            .await?;
        let stored_lineage: String = identity.try_get("lineage_id")?;
        let stored_instance: String = identity.try_get("instance_id")?;
        let stored_binding: String = identity.try_get("executor_binding_ref")?;
        let stored_ledger_generation: i64 = identity.try_get("ledger_generation")?;
        if stored_lineage != self.instance.lineage_id
            || stored_instance != self.instance.instance_id
            || stored_binding != self.instance.executor_binding_ref
            || stored_ledger_generation != self.instance.ledger_generation
        {
            return Err(PrototypeError::WriterFenced);
        }
        Ok(())
    }

    async fn classify_existing_append(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        candidate: &PreparedAppend,
    ) -> Result<Option<ExistingAppend>> {
        let sql = format!(
            "SELECT predecessor_sequence, predecessor_digest, purpose, candidate_digest,
                    committed_sequence, committed_digest
             FROM {}.prototype_append_requests
             WHERE stream_id = $1 AND append_request_id = $2",
            self.instance.schema.as_str()
        );
        let Some(row) = sqlx::query(audited_sql(sql))
            .bind(&candidate.stream_id)
            .bind(&candidate.append_request_id)
            .fetch_optional(&mut **transaction)
            .await?
        else {
            return Ok(None);
        };
        let committed = AppendCommit {
            head: JournalHead {
                sequence: row.try_get("committed_sequence")?,
                digest: row.try_get("committed_digest")?,
            },
        };
        let predecessor_sequence: i64 = row.try_get("predecessor_sequence")?;
        let predecessor_digest: Vec<u8> = row.try_get("predecessor_digest")?;
        let purpose: String = row.try_get("purpose")?;
        let persisted_candidate: Vec<u8> = row.try_get("candidate_digest")?;
        if predecessor_sequence == candidate.predecessor.sequence
            && predecessor_digest == candidate.predecessor.digest
            && purpose == candidate.purpose
            && persisted_candidate == candidate.candidate_digest
        {
            Ok(Some(ExistingAppend::Same(committed)))
        } else {
            Ok(Some(ExistingAppend::Conflict(committed)))
        }
    }

    async fn lock_and_require_head(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        candidate: &PreparedAppend,
    ) -> Result<()> {
        let sql = format!(
            "SELECT sequence, commit_digest
             FROM {}.prototype_journal_heads
             WHERE stream_id = $1
             FOR UPDATE",
            self.instance.schema.as_str()
        );
        let row = sqlx::query(audited_sql(sql))
            .bind(&candidate.stream_id)
            .fetch_optional(&mut **transaction)
            .await?
            .ok_or(PrototypeError::HeadMismatch)?;
        let sequence: i64 = row.try_get("sequence")?;
        let digest: Vec<u8> = row.try_get("commit_digest")?;
        if sequence != candidate.predecessor.sequence || digest != candidate.predecessor.digest {
            return Err(PrototypeError::HeadMismatch);
        }
        Ok(())
    }

    async fn insert_effect_append(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        folded: &FoldedEffect,
        actual_predecessor: &JournalHead,
        candidate: &PreparedAppend,
        body: EffectRecordBody,
    ) -> Result<AppendCommit> {
        let record = effect_append_record(folded, actual_predecessor, candidate, body)?;
        insert_effect_record_row(transaction, &self.instance, &record).await?;
        let head_sql = format!(
            "UPDATE {}.prototype_journal_heads
             SET sequence = $1, commit_digest = $2
             WHERE stream_id = $3",
            self.instance.schema.as_str()
        );
        sqlx::query(audited_sql(head_sql))
            .bind(record.sequence)
            .bind(&record.commit_digest)
            .bind(&record.stream_id)
            .execute(&mut **transaction)
            .await?;
        let request_sql = format!(
            "INSERT INTO {}.prototype_append_requests
             (stream_id, append_request_id, predecessor_sequence, predecessor_digest,
              purpose, candidate_digest, committed_sequence, committed_digest)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            self.instance.schema.as_str()
        );
        sqlx::query(audited_sql(request_sql))
            .bind(&candidate.stream_id)
            .bind(&candidate.append_request_id)
            .bind(candidate.predecessor.sequence)
            .bind(&candidate.predecessor.digest)
            .bind(&candidate.purpose)
            .bind(&candidate.candidate_digest)
            .bind(record.sequence)
            .bind(&record.commit_digest)
            .execute(&mut **transaction)
            .await?;
        let commit = AppendCommit {
            head: JournalHead {
                sequence: record.sequence,
                digest: record.commit_digest,
            },
        };
        self.retain_required_head(transaction, &record.stream_id, &commit.head)
            .await?;
        Ok(commit)
    }

    async fn retain_required_head(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        stream_id: &str,
        head: &JournalHead,
    ) -> Result<()> {
        let sql = format!(
            "INSERT INTO {}.prototype_required_heads
             (stream_id, sequence, commit_digest)
             VALUES ($1, $2, $3)
             ON CONFLICT (stream_id) DO UPDATE
             SET sequence = EXCLUDED.sequence,
                 commit_digest = EXCLUDED.commit_digest",
            self.inner.control_schema.as_str()
        );
        sqlx::query(audited_sql(sql))
            .bind(stream_id)
            .bind(head.sequence)
            .bind(&head.digest)
            .execute(&mut **transaction)
            .await?;
        Ok(())
    }

    async fn validate_folded_resource_link(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        folded: &FoldedEffect,
    ) -> Result<()> {
        let Some(resource_key_ref) = folded.projection.resource_key_ref.as_deref() else {
            return Ok(());
        };
        let resource_ownership_ref = folded
            .projection
            .resource_ownership_ref
            .as_deref()
            .ok_or(ModelError::ResourceLink)?;
        let resource = load_resource_fold(
            transaction,
            &self.instance,
            resource_ownership_ref,
            resource_key_ref,
        )
        .await?
        .ok_or(ModelError::ResourceLink)?;
        let mut linked_effects = Vec::new();
        let mut linked_streams = BTreeSet::new();
        for record in &resource.records {
            if linked_streams.insert(record.append.linked_effect_stream_id.clone()) {
                let effect_id = record
                    .append
                    .linked_effect_stream_id
                    .strip_prefix("effect:")
                    .filter(|effect_id| !effect_id.is_empty())
                    .ok_or(ModelError::ResourceLink)?;
                linked_effects
                    .push(load_folded_effect(transaction, &self.instance, effect_id).await?);
            }
        }
        if !linked_streams.contains(&folded.projection.stream_id) {
            return Err(ModelError::ResourceLink.into());
        }
        model::validate_resource_links(&linked_effects, std::slice::from_ref(&resource))?;
        Ok(())
    }
}

fn effect_stream_id(effect_id: &str) -> String {
    format!("effect:{effect_id}")
}

fn latest_returned_attempt(records: &[EffectRecord]) -> Option<(Vec<u8>, String, Vec<u8>)> {
    records.iter().rev().find_map(|record| {
        let EffectRecordBody::DeliveryAttemptObserved(observation) = &record.body else {
            return None;
        };
        let AttemptOutcome::Returned {
            safe_outcome_ref, ..
        } = &observation.outcome
        else {
            return None;
        };
        let target_operation_ref = records.iter().find_map(|candidate| match &candidate.body {
            EffectRecordBody::DeliveryAttemptAuthorized(authorization)
                if authorization.attempt_id == observation.attempt_id =>
            {
                Some(authorization.target_operation_ref.clone())
            }
            _ => None,
        })?;
        Some((
            observation.attempt_id.clone(),
            target_operation_ref,
            safe_outcome_ref.clone(),
        ))
    })
}

fn destination_receipt_from_row(
    effect_id: &str,
    row: &sqlx::postgres::PgRow,
) -> Result<DestinationReceipt> {
    Ok(DestinationReceipt {
        effect_id: effect_id.to_owned(),
        executor_binding_ref: row.try_get("executor_binding_ref")?,
        ledger_generation: row.try_get("ledger_generation")?,
        effect_key: row.try_get("effect_key")?,
        request_digest: row.try_get("request_digest")?,
        attempt_id: row.try_get("attempt_id")?,
        target_operation_ref: row.try_get("target_operation_ref")?,
        destination_identity: row.try_get("destination_identity")?,
        destination_generation: row.try_get("destination_generation")?,
        account_id: row.try_get("account_id")?,
    })
}

fn receipt_authorization<'a>(
    receipt: &DestinationReceipt,
    records: &'a [EffectRecord],
) -> Option<&'a AttemptAuthorization> {
    records.iter().find_map(|record| match &record.body {
        EffectRecordBody::DeliveryAttemptAuthorized(authorization)
            if authorization.attempt_id == receipt.attempt_id
                && authorization.target_operation_ref == receipt.target_operation_ref =>
        {
            Some(authorization)
        }
        _ => None,
    })
}

fn returned_outcome_ref(effect_id: &str, attempt_id: &[u8], account_id: i64) -> Vec<u8> {
    let mut preimage = Vec::with_capacity(effect_id.len() + attempt_id.len() + 32);
    preimage.extend_from_slice(b"mfm.pg-prototype.returned-outcome.v1");
    preimage.extend_from_slice(effect_id.as_bytes());
    preimage.extend_from_slice(attempt_id);
    preimage.extend_from_slice(&account_id.to_be_bytes());
    sha256_digest_bytes(&preimage).as_bytes().to_vec()
}

impl PromotedWriter {
    async fn select_destination_receipt(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        effect_id: &str,
        attempt_id: &[u8],
    ) -> Result<Option<DestinationReceipt>> {
        let sql = format!(
            "SELECT executor_binding_ref, ledger_generation, effect_key,
                    request_digest, attempt_id, target_operation_ref,
                    destination_identity, destination_generation, account_id
             FROM {}.prototype_destination_receipts
             WHERE effect_id = $1 AND attempt_id = $2
             FOR SHARE",
            self.inner.destination_schema.as_str()
        );
        sqlx::query(audited_sql(sql))
            .bind(effect_id)
            .bind(attempt_id)
            .fetch_optional(&mut **transaction)
            .await?
            .map(|row| destination_receipt_from_row(effect_id, &row))
            .transpose()
    }

    fn validate_destination_receipt(
        &self,
        receipt: &DestinationReceipt,
        folded: &FoldedEffect,
        records: &[EffectRecord],
    ) -> Result<()> {
        if receipt.executor_binding_ref != self.instance.executor_binding_ref
            || receipt.ledger_generation != self.instance.ledger_generation
            || folded.frontier.executor_binding_ref != receipt.executor_binding_ref
            || folded.frontier.ledger_generation != receipt.ledger_generation
            || folded.projection.effect_id != receipt.effect_id
            || folded.projection.effect_key != receipt.effect_key
            || folded.projection.request_digest != receipt.request_digest
            || folded.projection.destination_identity != receipt.destination_identity
            || folded.projection.allocation_value != Some(receipt.account_id)
            || receipt_authorization(receipt, records).is_none()
        {
            return Err(PrototypeError::DestinationConflict);
        }
        Ok(())
    }

    async fn persist_destination_receipt(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        witness: &TargetEntryWitness,
        destination_generation: i64,
        account_id: i64,
    ) -> Result<DestinationReceipt> {
        let insert_sql = format!(
            "INSERT INTO {}.prototype_destination_receipts
             (effect_id, executor_binding_ref, ledger_generation, effect_key,
              request_digest, attempt_id, target_operation_ref, destination_identity,
              destination_generation, account_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (effect_id, attempt_id) DO NOTHING",
            self.inner.destination_schema.as_str()
        );
        sqlx::query(audited_sql(insert_sql))
            .bind(&witness.effect_id)
            .bind(&witness.executor_binding_ref)
            .bind(witness.ledger_generation)
            .bind(&witness.effect_key)
            .bind(&witness.request_digest)
            .bind(&witness.attempt_id)
            .bind(&witness.target_operation_ref)
            .bind(&witness.destination_identity)
            .bind(destination_generation)
            .bind(account_id)
            .execute(&mut **transaction)
            .await?;
        let receipt = self
            .select_destination_receipt(transaction, &witness.effect_id, &witness.attempt_id)
            .await?
            .ok_or(PrototypeError::DestinationConflict)?;
        if receipt.executor_binding_ref != witness.executor_binding_ref
            || receipt.ledger_generation != witness.ledger_generation
            || receipt.effect_key != witness.effect_key
            || receipt.request_digest != witness.request_digest
            || receipt.target_operation_ref != witness.target_operation_ref
            || receipt.destination_identity != witness.destination_identity
            || receipt.destination_generation != destination_generation
            || receipt.account_id != account_id
        {
            return Err(PrototypeError::DestinationConflict);
        }
        Ok(receipt)
    }

    pub(super) async fn destination_receipt(
        &self,
        effect_id: &str,
        attempt_id: &[u8],
    ) -> Result<Option<DestinationReceipt>> {
        let mut transaction = self.pool.begin().await?;
        self.validate_writer(&mut transaction).await?;
        let folded = load_folded_effect(&mut transaction, &self.instance, effect_id).await?;
        let records = load_effect_records(
            &mut transaction,
            &self.instance,
            &effect_stream_id(effect_id),
        )
        .await?;
        let receipt = self
            .select_destination_receipt(&mut transaction, effect_id, attempt_id)
            .await?;
        if let Some(receipt) = receipt.as_ref() {
            self.validate_destination_receipt(receipt, &folded, &records)?;
        }
        transaction.commit().await?;
        Ok(receipt)
    }

    async fn next_unobserved_destination_receipt(
        &self,
        effect_id: &str,
    ) -> Result<Option<DestinationReceipt>> {
        let mut transaction = self.pool.begin().await?;
        self.validate_writer(&mut transaction).await?;
        let folded = load_folded_effect(&mut transaction, &self.instance, effect_id).await?;
        let records = load_effect_records(
            &mut transaction,
            &self.instance,
            &effect_stream_id(effect_id),
        )
        .await?;
        let sql = format!(
            "SELECT executor_binding_ref, ledger_generation, effect_key,
                    request_digest, attempt_id, target_operation_ref,
                    destination_identity, destination_generation, account_id
             FROM {}.prototype_destination_receipts
             WHERE effect_id = $1
             ORDER BY attempt_id
             FOR SHARE",
            self.inner.destination_schema.as_str()
        );
        let rows = sqlx::query(audited_sql(sql))
            .bind(effect_id)
            .fetch_all(&mut *transaction)
            .await?;
        let mut receipts = rows
            .iter()
            .map(|row| destination_receipt_from_row(effect_id, row))
            .collect::<Result<Vec<_>>>()?;
        for receipt in &receipts {
            self.validate_destination_receipt(receipt, &folded, &records)?;
        }
        let observed_attempts = records
            .iter()
            .filter_map(|record| match &record.body {
                EffectRecordBody::DeliveryAttemptObserved(observation) => {
                    Some(observation.attempt_id.as_slice())
                }
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        // Recovery walks authorization order only to choose among explicit durable receipts.
        // An authorization without its own receipt never implies successful target entry.
        let receipt = records
            .iter()
            .filter_map(|record| match &record.body {
                EffectRecordBody::DeliveryAttemptAuthorized(authorization)
                    if !observed_attempts.contains(authorization.attempt_id.as_slice()) =>
                {
                    Some(authorization.attempt_id.as_slice())
                }
                _ => None,
            })
            .find_map(|attempt_id| {
                receipts
                    .iter()
                    .position(|receipt| receipt.attempt_id == attempt_id)
                    .map(|index| receipts.swap_remove(index))
            });
        transaction.commit().await?;
        Ok(receipt)
    }

    pub(super) async fn observed_attempt_ids(&self, effect_id: &str) -> Result<Vec<Vec<u8>>> {
        let mut transaction = self.pool.begin().await?;
        self.validate_writer(&mut transaction).await?;
        let records = load_effect_records(
            &mut transaction,
            &self.instance,
            &effect_stream_id(effect_id),
        )
        .await?;
        let attempt_ids = records
            .into_iter()
            .filter_map(|record| match record.body {
                EffectRecordBody::DeliveryAttemptObserved(observation) => {
                    Some(observation.attempt_id)
                }
                _ => None,
            })
            .collect();
        transaction.commit().await?;
        Ok(attempt_ids)
    }

    pub(super) async fn terminal_proof_targets_attempt(
        &self,
        effect_id: &str,
        expected_attempt_id: &[u8],
    ) -> Result<bool> {
        let mut transaction = self.pool.begin().await?;
        self.validate_writer(&mut transaction).await?;
        let folded = load_folded_effect(&mut transaction, &self.instance, effect_id).await?;
        let records = load_effect_records(
            &mut transaction,
            &self.instance,
            &effect_stream_id(effect_id),
        )
        .await?;
        self.validate_folded_resource_link(&mut transaction, &folded)
            .await?;
        let exact = records.iter().find_map(|record| match &record.body {
            EffectRecordBody::TerminalTombstone(tombstone) => Some(
                tombstone.terminal_attempt_id == expected_attempt_id
                    && tombstone.terminal_proof_ref
                        == model::terminal_proof_ref(
                            &tombstone.terminal_attempt_id,
                            &tombstone.external_operation_ref,
                            &tombstone.terminal_outcome_ref,
                        ),
            ),
            _ => None,
        });
        transaction.commit().await?;
        Ok(exact.unwrap_or(false))
    }

    pub(super) async fn append_observation(
        &self,
        receipt: DestinationReceipt,
    ) -> Result<AppendCommit> {
        let stream_id = effect_stream_id(&receipt.effect_id);
        let mut transaction = self.pool.begin().await?;
        self.validate_writer(&mut transaction).await?;
        let effect_lock_sql = format!(
            "SELECT sequence
             FROM {}.prototype_journal_heads
             WHERE stream_id = $1
             FOR UPDATE",
            self.instance.schema.as_str()
        );
        sqlx::query_scalar::<_, i64>(audited_sql(effect_lock_sql))
            .bind(&stream_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(PrototypeError::MissingEffect)?;
        let durable_receipt = self
            .select_destination_receipt(&mut transaction, &receipt.effect_id, &receipt.attempt_id)
            .await?
            .ok_or(PrototypeError::DestinationConflict)?;
        if durable_receipt != receipt {
            return Err(PrototypeError::DestinationConflict);
        }
        let folded =
            load_folded_effect(&mut transaction, &self.instance, &receipt.effect_id).await?;
        let records = load_effect_records(&mut transaction, &self.instance, &stream_id).await?;
        self.validate_destination_receipt(&receipt, &folded, &records)?;
        let authorization = receipt_authorization(&receipt, &records)
            .cloned()
            .ok_or(PrototypeError::EffectNotAuthorizable)?;
        self.validate_folded_resource_link(&mut transaction, &folded)
            .await?;
        let outcome_ref = returned_outcome_ref(
            &receipt.effect_id,
            &authorization.attempt_id,
            receipt.account_id,
        );
        if let Some(observed) = records.iter().find(|record| {
            matches!(
                &record.body,
                EffectRecordBody::DeliveryAttemptObserved(observation)
                    if observation.attempt_id == receipt.attempt_id
            )
        }) {
            let exact_return = matches!(
                &observed.body,
                EffectRecordBody::DeliveryAttemptObserved(AttemptObservation {
                    outcome: AttemptOutcome::Returned {
                        safe_outcome_ref,
                        destination_account,
                    },
                    ..
                }) if safe_outcome_ref == &outcome_ref
                    && *destination_account == receipt.account_id
            );
            if !exact_return {
                return Err(PrototypeError::DestinationConflict);
            }
            transaction.commit().await?;
            return Ok(AppendCommit {
                head: JournalHead {
                    sequence: observed.sequence,
                    digest: observed.commit_digest.clone(),
                },
            });
        }
        let body = EffectRecordBody::DeliveryAttemptObserved(AttemptObservation {
            attempt_id: authorization.attempt_id,
            outcome: AttemptOutcome::Returned {
                safe_outcome_ref: outcome_ref,
                destination_account: receipt.account_id,
            },
        });
        let predecessor = JournalHead {
            sequence: folded.frontier.sequence,
            digest: folded.frontier.commit_digest.clone(),
        };
        let payload = receipt.account_id.to_be_bytes();
        let candidate = PreparedAppend::for_effect(
            stream_id.clone(),
            predecessor.clone(),
            format!(
                "observation:{}:{}",
                receipt.effect_id, authorization.attempt_ordinal
            ),
            &payload,
            &body,
        );
        if let Some(existing) = self
            .classify_existing_append(&mut transaction, &candidate)
            .await?
        {
            transaction.commit().await?;
            return match existing {
                ExistingAppend::Same(commit) => Ok(commit),
                ExistingAppend::Conflict(_) => Err(PrototypeError::UnexpectedAppendOutcome),
            };
        }
        self.lock_and_require_head(&mut transaction, &candidate)
            .await?;
        let commit = self
            .insert_effect_append(&mut transaction, &folded, &predecessor, &candidate, body)
            .await?;
        let records = load_effect_records(&mut transaction, &self.instance, &stream_id).await?;
        let committed_fold =
            fold_effect_records(&mut transaction, &self.instance, &records).await?;
        persist_effect_derivations(&mut transaction, &self.instance, &committed_fold).await?;
        self.validate_folded_resource_link(&mut transaction, &committed_fold)
            .await?;
        transaction.commit().await?;
        Ok(commit)
    }

    pub(super) async fn append_tombstone(&self, effect_id: &str) -> Result<AppendCommit> {
        let stream_id = effect_stream_id(effect_id);
        let mut transaction = self.pool.begin().await?;
        self.validate_writer(&mut transaction).await?;
        let folded = load_folded_effect(&mut transaction, &self.instance, effect_id).await?;
        let records = load_effect_records(&mut transaction, &self.instance, &stream_id).await?;
        let (terminal_attempt_id, external_operation_ref, terminal_outcome_ref) =
            latest_returned_attempt(&records).ok_or(PrototypeError::EffectNotAuthorizable)?;
        let terminal_proof_ref = model::terminal_proof_ref(
            &terminal_attempt_id,
            &external_operation_ref,
            &terminal_outcome_ref,
        );
        let body = EffectRecordBody::TerminalTombstone(Tombstone {
            terminal_attempt_id,
            external_operation_ref,
            terminal_outcome_ref,
            terminal_proof_ref,
        });
        let predecessor = JournalHead {
            sequence: folded.frontier.sequence,
            digest: folded.frontier.commit_digest.clone(),
        };
        let candidate = PreparedAppend::for_effect(
            stream_id.clone(),
            predecessor.clone(),
            format!("tombstone:{effect_id}"),
            b"",
            &body,
        );
        if let Some(existing) = self
            .classify_existing_append(&mut transaction, &candidate)
            .await?
        {
            transaction.commit().await?;
            return match existing {
                ExistingAppend::Same(commit) => Ok(commit),
                ExistingAppend::Conflict(_) => Err(PrototypeError::UnexpectedAppendOutcome),
            };
        }
        self.lock_and_require_head(&mut transaction, &candidate)
            .await?;
        let commit = self
            .insert_effect_append(&mut transaction, &folded, &predecessor, &candidate, body)
            .await?;
        let records = load_effect_records(&mut transaction, &self.instance, &stream_id).await?;
        let committed_fold =
            fold_effect_records(&mut transaction, &self.instance, &records).await?;
        persist_effect_derivations(&mut transaction, &self.instance, &committed_fold).await?;
        self.validate_folded_resource_link(&mut transaction, &committed_fold)
            .await?;
        transaction.commit().await?;
        Ok(commit)
    }

    pub(super) async fn enter_destination(
        &self,
        witness: TargetEntryWitness,
        pause: Option<Pause>,
    ) -> Result<DestinationMutation> {
        self.enter_destination_with_fence_pause(witness, pause, None)
            .await
    }

    pub(super) async fn enter_destination_holding_fence(
        &self,
        witness: TargetEntryWitness,
        fence_pause: Pause,
    ) -> Result<DestinationMutation> {
        self.enter_destination_with_fence_pause(witness, None, Some(fence_pause))
            .await
    }

    async fn enter_destination_with_fence_pause(
        &self,
        witness: TargetEntryWitness,
        pre_transaction_pause: Option<Pause>,
        fence_pause: Option<Pause>,
    ) -> Result<DestinationMutation> {
        if let Some(pause) = pre_transaction_pause {
            // The deliberate simulated IO wait occurs before any PostgreSQL transaction begins.
            pause.wait().await?;
        }
        let mut transaction = self.pool.begin().await?;
        if let Err(error) = self.validate_writer(&mut transaction).await {
            if matches!(error, PrototypeError::WriterFenced) {
                let fence_sql = format!(
                    "SELECT generation
                     FROM {}.prototype_destination_fence
                     WHERE singleton = TRUE",
                    self.inner.destination_schema.as_str()
                );
                let generation: i64 = sqlx::query_scalar(audited_sql(fence_sql))
                    .fetch_one(&mut *transaction)
                    .await?;
                if generation != witness.destination_generation {
                    return Err(PrototypeError::DestinationGenerationFenced);
                }
            }
            return Err(error);
        }
        let stream_id = effect_stream_id(&witness.effect_id);
        let effect_lock_sql = format!(
            "SELECT sequence
             FROM {}.prototype_journal_heads
             WHERE stream_id = $1
             FOR UPDATE",
            self.instance.schema.as_str()
        );
        sqlx::query_scalar::<_, i64>(audited_sql(effect_lock_sql))
            .bind(&stream_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(PrototypeError::DestinationConflict)?;
        let fence_sql = format!(
            "SELECT generation
             FROM {}.prototype_destination_fence
             WHERE singleton = TRUE
             FOR SHARE",
            self.inner.destination_schema.as_str()
        );
        let generation: i64 = sqlx::query_scalar(audited_sql(fence_sql))
            .fetch_one(&mut *transaction)
            .await?;
        if generation != witness.destination_generation {
            return Err(PrototypeError::DestinationGenerationFenced);
        }
        if let Some(pause) = fence_pause {
            // This probe is deliberately inside the transaction after the shared
            // destination-generation fence has been acquired.
            pause.wait().await?;
        }
        let folded =
            load_folded_effect(&mut transaction, &self.instance, &witness.effect_id).await?;
        let records = load_effect_records(&mut transaction, &self.instance, &stream_id).await?;
        let authorized = records.iter().any(|record| {
            matches!(
                &record.body,
                EffectRecordBody::DeliveryAttemptAuthorized(authorization)
                    if authorization.attempt_id == witness.attempt_id
                        && authorization.target_operation_ref
                            == witness.target_operation_ref
            )
        });
        let attempt_observed = records.iter().any(|record| {
            matches!(
                &record.body,
                EffectRecordBody::DeliveryAttemptObserved(observation)
                    if observation.attempt_id == witness.attempt_id
            )
        });
        let tombstoned = records
            .iter()
            .any(|record| matches!(&record.body, EffectRecordBody::TerminalTombstone(_)));
        if witness.executor_binding_ref != self.instance.executor_binding_ref
            || witness.ledger_generation != self.instance.ledger_generation
            || folded.frontier.executor_binding_ref != witness.executor_binding_ref
            || folded.frontier.ledger_generation != witness.ledger_generation
            || folded.projection.effect_key != witness.effect_key
            || folded.projection.request_digest != witness.request_digest
            || folded.projection.destination_identity != witness.destination_identity
            || !authorized
            || attempt_observed
            || tombstoned
            || folded.projection.resource_ownership_ref.as_deref()
                != Some(DESTINATION_INVENTORY_RESOURCE_OWNERSHIP_REF)
            || folded.projection.resource_key_ref.as_deref()
                != Some(DESTINATION_INVENTORY_RESOURCE_KEY_REF)
        {
            return Err(PrototypeError::DestinationConflict);
        }
        self.validate_folded_resource_link(&mut transaction, &folded)
            .await?;
        let account_id = folded
            .projection
            .allocation_value
            .ok_or(PrototypeError::DestinationConflict)?;

        let existing_sql = format!(
            "SELECT executor_binding_ref, ledger_generation, effect_key,
                    request_digest, target_operation_ref, destination_identity,
                    account_id
             FROM {}.prototype_destination_mutations
             WHERE effect_id = $1",
            self.inner.destination_schema.as_str()
        );
        if let Some(row) = sqlx::query(audited_sql(existing_sql))
            .bind(&witness.effect_id)
            .fetch_optional(&mut *transaction)
            .await?
        {
            let existing_same = row.try_get::<String, _>("executor_binding_ref")?
                == witness.executor_binding_ref
                && row.try_get::<i64, _>("ledger_generation")? == witness.ledger_generation
                && row.try_get::<Vec<u8>, _>("effect_key")? == witness.effect_key
                && row.try_get::<Vec<u8>, _>("request_digest")? == witness.request_digest
                && row.try_get::<String, _>("target_operation_ref")?
                    == witness.target_operation_ref
                && row.try_get::<String, _>("destination_identity")?
                    == witness.destination_identity
                && row.try_get::<i64, _>("account_id")? == account_id;
            if !existing_same {
                return Err(PrototypeError::DestinationConflict);
            }
            let receipt = self
                .persist_destination_receipt(&mut transaction, &witness, generation, account_id)
                .await?;
            transaction.commit().await?;
            return Ok(DestinationMutation::ExistingSame { receipt });
        }

        let mutation_sql = format!(
            "INSERT INTO {}.prototype_destination_mutations
             (effect_id, executor_binding_ref, ledger_generation, effect_key,
              request_digest, attempt_id, target_operation_ref, destination_identity,
              destination_generation, account_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT DO NOTHING",
            self.inner.destination_schema.as_str()
        );
        let inserted = sqlx::query(audited_sql(mutation_sql))
            .bind(&witness.effect_id)
            .bind(&witness.executor_binding_ref)
            .bind(witness.ledger_generation)
            .bind(&witness.effect_key)
            .bind(&witness.request_digest)
            .bind(&witness.attempt_id)
            .bind(&witness.target_operation_ref)
            .bind(&witness.destination_identity)
            .bind(generation)
            .bind(account_id)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
        if inserted != 1 {
            return Err(PrototypeError::DestinationConflict);
        }
        let receipt = self
            .persist_destination_receipt(&mut transaction, &witness, generation, account_id)
            .await?;
        transaction.commit().await?;
        Ok(DestinationMutation::NewlyApplied { receipt })
    }

    pub(super) async fn seed_account_sequence(
        &self,
        account_key: &str,
        next_sequence: i64,
    ) -> Result<()> {
        let mut transaction = self.pool.begin().await?;
        self.validate_writer(&mut transaction).await?;
        ensure_resource_configuration(
            &mut transaction,
            &self.instance.schema,
            ACCOUNT_SEQUENCE_RESOURCE_OWNERSHIP_REF,
            account_key,
            &ResourcePolicyConfiguration::AccountSequence {
                initial_value: next_sequence,
            },
        )
        .await?;
        let configuration = ResourcePolicyConfiguration::AccountSequence {
            initial_value: next_sequence,
        };
        self.retain_required_head(
            &mut transaction,
            &model::resource_configuration_stream_id(
                ACCOUNT_SEQUENCE_RESOURCE_OWNERSHIP_REF,
                account_key,
            ),
            &JournalHead {
                sequence: 1,
                digest: model::resource_policy_configuration_ref(&configuration),
            },
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(super) async fn allocate_account_sequence(
        &self,
        effect_id: &str,
        account_key: &str,
        acknowledgement: SequenceAcknowledgement,
    ) -> Result<SequenceAllocationOutcome> {
        let mut transaction = self.pool.begin().await?;
        self.validate_writer(&mut transaction).await?;
        let stream_id = effect_stream_id(effect_id);
        let head_sql = format!(
            "SELECT sequence
             FROM {}.prototype_journal_heads
             WHERE stream_id = $1
             FOR UPDATE",
            self.instance.schema.as_str()
        );
        sqlx::query_scalar::<_, i64>(audited_sql(head_sql))
            .bind(&stream_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(PrototypeError::MissingEffect)?;
        let folded = load_folded_effect(&mut transaction, &self.instance, effect_id).await?;
        let records = load_effect_records(&mut transaction, &self.instance, &stream_id).await?;
        let authenticated_intent = records
            .first()
            .ok_or(PrototypeError::SequenceAllocationConflict)
            .and_then(|record| {
                decode_bound_resource_intent(&record.opaque_payload)
                    .map_err(|_| PrototypeError::SequenceAllocationConflict)
            })?;
        let Some(authenticated_intent) = authenticated_intent else {
            return Err(PrototypeError::SequenceAllocationConflict);
        };
        if authenticated_intent.resource_ownership_ref != ACCOUNT_SEQUENCE_RESOURCE_OWNERSHIP_REF
            || authenticated_intent.resource_key_ref != account_key
            || authenticated_intent.policy_ref
                != (ResourcePolicyConfiguration::AccountSequence { initial_value: 0 }).policy_ref()
        {
            return Err(PrototypeError::SequenceAllocationConflict);
        }
        if let Some(sequence) = folded.projection.allocation_value {
            if folded.projection.resource_ownership_ref.as_deref()
                != Some(ACCOUNT_SEQUENCE_RESOURCE_OWNERSHIP_REF)
                || folded.projection.resource_key_ref.as_deref() != Some(account_key)
                || folded.projection.policy_configuration_ref.as_deref()
                    != Some(authenticated_intent.policy_configuration_ref.as_slice())
            {
                return Err(PrototypeError::SequenceAllocationConflict);
            }
            self.validate_folded_resource_link(&mut transaction, &folded)
                .await?;
            transaction.commit().await?;
            return Ok(SequenceAllocationOutcome::ExistingSame { sequence });
        }
        if folded.projection.authorization_count != 0 {
            return Err(PrototypeError::SequenceAllocationConflict);
        }
        let append_identity = format!("{effect_id}:account-sequence");
        let allocation_append_request_id = format!("resource-allocation:{append_identity}");
        let (_, sequence) = self
            .append_configured_resource(
                &mut transaction,
                folded,
                ACCOUNT_SEQUENCE_RESOURCE_OWNERSHIP_REF,
                account_key,
                &append_identity,
                &allocation_append_request_id,
            )
            .await?;
        let configuration = lock_resource_configuration(
            &mut transaction,
            &self.instance,
            ACCOUNT_SEQUENCE_RESOURCE_OWNERSHIP_REF,
            account_key,
        )
        .await?;
        if model::resource_policy_configuration_ref(&configuration)
            != authenticated_intent.policy_configuration_ref
        {
            return Err(PrototypeError::SequenceAllocationConflict);
        }
        transaction.commit().await?;
        Ok(match acknowledgement {
            SequenceAcknowledgement::DirectlyObserved => {
                SequenceAllocationOutcome::NewlyAllocated { sequence }
            }
            SequenceAcknowledgement::LostAfterCommit => SequenceAllocationOutcome::OutcomeUnknown,
        })
    }

    pub(super) fn rejected_credential_probe(&self, credential: &[u8]) -> Result<()> {
        // The credential is deliberately consumed below all durable row construction.
        let _credential_was_supplied = !credential.is_empty();
        Err(PrototypeError::DestinationCredentialRejected)
    }

    pub(super) async fn enter_destination_with_ephemeral_credential(
        &self,
        witness: TargetEntryWitness,
        credential: &[u8],
    ) -> Result<DestinationMutation> {
        // A live adapter would inject these bytes into transport IO here. They are never hashed,
        // formatted, or included in a PostgreSQL statement.
        let _credential_was_supplied = !credential.is_empty();
        self.enter_destination(witness, None).await
    }
}

impl TestTopology {
    pub(super) async fn destination_mutation_count(&self, effect_id: &str) -> Result<i64> {
        let sql = format!(
            "SELECT COUNT(*)
             FROM {}.prototype_destination_mutations
             WHERE effect_id = $1",
            self.inner.destination_schema.as_str()
        );
        sqlx::query_scalar(audited_sql(sql))
            .bind(effect_id)
            .fetch_one(&self.inner.pool)
            .await
            .map_err(PrototypeError::from)
    }

    pub(super) async fn destination_receipt_count(&self, effect_id: &str) -> Result<i64> {
        let sql = format!(
            "SELECT COUNT(*)
             FROM {}.prototype_destination_receipts
             WHERE effect_id = $1",
            self.inner.destination_schema.as_str()
        );
        sqlx::query_scalar(audited_sql(sql))
            .bind(effect_id)
            .fetch_one(&self.inner.pool)
            .await
            .map_err(PrototypeError::from)
    }

    pub(super) async fn allocated_accounts(&self, writer: &PromotedWriter) -> Result<Vec<i64>> {
        let mut transaction = writer.pool.begin().await?;
        let Some(resource) = load_resource_fold(
            &mut transaction,
            &writer.instance,
            DESTINATION_INVENTORY_RESOURCE_OWNERSHIP_REF,
            DESTINATION_INVENTORY_RESOURCE_KEY_REF,
        )
        .await?
        else {
            transaction.commit().await?;
            return Ok(Vec::new());
        };
        let first_effect_id = resource
            .records
            .first()
            .and_then(|record| {
                record
                    .append
                    .linked_effect_stream_id
                    .strip_prefix("effect:")
            })
            .ok_or(ModelError::ResourceLink)?;
        let folded =
            load_folded_effect(&mut transaction, &writer.instance, first_effect_id).await?;
        writer
            .validate_folded_resource_link(&mut transaction, &folded)
            .await?;
        let allocations = resource
            .records
            .into_iter()
            .map(|record| record.allocation_value)
            .collect();
        transaction.commit().await?;
        Ok(allocations)
    }

    pub(super) async fn account_sequence_allocations(
        &self,
        writer: &PromotedWriter,
        account_key: &str,
    ) -> Result<Vec<(String, i64)>> {
        let mut transaction = writer.pool.begin().await?;
        let Some(resource) = load_resource_fold(
            &mut transaction,
            &writer.instance,
            ACCOUNT_SEQUENCE_RESOURCE_OWNERSHIP_REF,
            account_key,
        )
        .await?
        else {
            transaction.commit().await?;
            return Ok(Vec::new());
        };
        let first_effect_id = resource
            .records
            .first()
            .and_then(|record| {
                record
                    .append
                    .linked_effect_stream_id
                    .strip_prefix("effect:")
            })
            .ok_or(ModelError::ResourceLink)?;
        let folded =
            load_folded_effect(&mut transaction, &writer.instance, first_effect_id).await?;
        writer
            .validate_folded_resource_link(&mut transaction, &folded)
            .await?;
        let allocations = resource
            .records
            .into_iter()
            .map(|record| {
                let effect_id = record
                    .append
                    .linked_effect_stream_id
                    .strip_prefix("effect:")
                    .ok_or(ModelError::ResourceLink)?
                    .to_owned();
                Ok((effect_id, record.allocation_value))
            })
            .collect::<Result<Vec<_>>>()?;
        transaction.commit().await?;
        Ok(allocations)
    }
}

impl Executor {
    async fn drain_unobserved_destination_receipts(&self, effect_id: &str) -> Result<()> {
        while let Some(receipt) = self
            .writer
            .next_unobserved_destination_receipt(effect_id)
            .await?
        {
            self.writer.append_observation(receipt).await?;
        }
        Ok(())
    }

    pub(super) async fn drive_effect(
        &self,
        effect_id: &str,
        crash_point: CrashPoint,
    ) -> Result<DriveOutcome> {
        let snapshot = self.writer.effect_snapshot(effect_id).await?;
        if snapshot.state == EffectState::Tombstoned {
            // A target entry may have committed just before the tombstone while its acknowledgement
            // was lost. Only its explicit durable receipt can strengthen the terminal audit; no
            // post-terminal authorization or target entry is permitted.
            self.drain_unobserved_destination_receipts(effect_id)
                .await?;
            return Ok(DriveOutcome::AlreadyComplete);
        }

        if snapshot.state == EffectState::Observed {
            snapshot
                .destination_account
                .ok_or(PrototypeError::UnexpectedAppendOutcome)?;
            self.drain_unobserved_destination_receipts(effect_id)
                .await?;
            if crash_point == CrashPoint::BeforeTombstone
                || crash_point == CrashPoint::AfterObservation
            {
                return Ok(DriveOutcome::Crashed(crash_point));
            }
        } else {
            let destination_receipt = if let Some(receipt) = self
                .writer
                .next_unobserved_destination_receipt(effect_id)
                .await?
            {
                receipt
            } else {
                if crash_point == CrashPoint::BeforeAuthorization {
                    return Ok(DriveOutcome::Crashed(crash_point));
                }
                let append_request_id = format!(
                    "authorization:{effect_id}:{}",
                    snapshot.authorization_count + 1
                );
                let request = self
                    .writer
                    .authorization_request(
                        effect_id,
                        &append_request_id,
                        b"opaque authorization payload",
                    )
                    .await?;
                let outcome = self
                    .writer
                    .authorize_effect(
                        &request,
                        CommitAcknowledgement::DirectlyObserved,
                        AuthorizationFailurePoint::None,
                    )
                    .await?;
                let AuthorizationOutcome::NewlyAppended { target_entry, .. } = outcome else {
                    return Err(PrototypeError::UnexpectedAppendOutcome);
                };
                if crash_point == CrashPoint::AfterAuthorization
                    || crash_point == CrashPoint::BeforeDestinationMutation
                {
                    return Ok(DriveOutcome::Crashed(crash_point));
                }
                self.writer
                    .enter_destination(target_entry, None)
                    .await?
                    .into_receipt()
            };
            if crash_point == CrashPoint::AfterDestinationMutation
                || crash_point == CrashPoint::BeforeObservation
            {
                return Ok(DriveOutcome::Crashed(crash_point));
            }
            self.writer.append_observation(destination_receipt).await?;
            if crash_point == CrashPoint::AfterObservation {
                return Ok(DriveOutcome::Crashed(crash_point));
            }
            self.drain_unobserved_destination_receipts(effect_id)
                .await?;
            if crash_point == CrashPoint::BeforeTombstone {
                return Ok(DriveOutcome::Crashed(crash_point));
            }
        }
        self.writer.append_tombstone(effect_id).await?;
        if crash_point == CrashPoint::AfterTombstone {
            return Ok(DriveOutcome::Crashed(crash_point));
        }
        Ok(DriveOutcome::Completed)
    }
}

impl TestTopology {
    pub(super) async fn assert_retained_surfaces_exclude(&self, secret: &[u8]) -> Result<()> {
        let forbidden = forbidden_encodings(secret);
        let schemas = self
            .inner
            .schemas
            .lock()
            .map_err(|_| PrototypeError::SchemaRegistryUnavailable)?
            .clone();
        for schema in schemas {
            let table_sql = "SELECT table_name
                             FROM information_schema.tables
                             WHERE table_schema = $1 AND table_type = 'BASE TABLE'
                             ORDER BY table_name";
            let tables: Vec<String> = sqlx::query_scalar(table_sql)
                .bind(schema.as_str())
                .fetch_all(&self.inner.pool)
                .await?;
            for table in tables {
                let row_sql = format!(
                    "SELECT row_to_json(retained_row)::TEXT
                     FROM {}.{} AS retained_row",
                    schema.as_str(),
                    table
                );
                let rows: Vec<String> = sqlx::query_scalar(audited_sql(row_sql))
                    .fetch_all(&self.inner.pool)
                    .await?;
                if rows.iter().any(|row| {
                    forbidden
                        .iter()
                        .any(|needle| contains_bytes(row.as_bytes(), needle))
                }) {
                    return Err(PrototypeError::ForbiddenMaterialPersisted);
                }
            }
        }
        Ok(())
    }

    pub(super) async fn candidate_canonical_round_trip(
        &self,
        candidate: &RestoreCandidate,
    ) -> Result<Vec<u8>> {
        let mut transaction = self.inner.pool.begin().await?;
        restore::validate_derived(&mut transaction, &candidate.instance).await?;
        let canonical = restore::export_authority(&mut transaction, &candidate.instance).await?;
        let decoded = restore::decode_authority(&canonical)?;
        if !restore::decoded_identity_matches_instance(&decoded, &candidate.instance)
            || canonical != candidate.canonical_authority
        {
            return Err(PrototypeError::CanonicalRoundTripMismatch);
        }
        transaction.commit().await?;
        Ok(canonical)
    }

    pub(super) async fn snapshot_clone(&self, source: &PromotedWriter) -> Result<RestoreCandidate> {
        let mut export_transaction = self.inner.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *export_transaction)
            .await?;
        let canonical_authority =
            restore::export_authority(&mut export_transaction, &source.instance).await?;
        export_transaction.commit().await?;
        let decoded = restore::decode_authority(&canonical_authority)?;

        let schema = SchemaName::generated("candidate");
        create_store_schema(&self.inner.pool, &schema, "", "", "", 1, false).await?;
        self.inner
            .schemas
            .lock()
            .map_err(|_| PrototypeError::SchemaRegistryUnavailable)?
            .push(schema.clone());
        let instance = StoreInstance {
            schema,
            lineage_id: decoded.lineage_id.clone(),
            instance_id: decoded.instance_id.clone(),
            executor_binding_ref: decoded.executor_binding_ref.clone(),
            ledger_generation: decoded.ledger_generation,
        };
        let mut import_transaction = self.inner.pool.begin().await?;
        restore::import_authority(&mut import_transaction, &instance, &decoded).await?;
        restore::rebuild_derivations(&mut import_transaction, &instance).await?;
        let reexported = restore::export_authority(&mut import_transaction, &instance).await?;
        if reexported != canonical_authority {
            return Err(PrototypeError::CanonicalRoundTripMismatch);
        }
        import_transaction.commit().await?;
        Ok(RestoreCandidate {
            instance,
            canonical_authority,
        })
    }

    pub(super) async fn promote(
        &self,
        candidate: RestoreCandidate,
        expected_generation: i64,
        writer_id: &str,
    ) -> Result<PromotedWriter> {
        self.promote_with_probe(candidate, expected_generation, writer_id, None)
            .await
    }

    pub(super) async fn promote_with_probe(
        &self,
        candidate: RestoreCandidate,
        expected_generation: i64,
        writer_id: &str,
        started: Option<oneshot::Sender<()>>,
    ) -> Result<PromotedWriter> {
        let mut transaction = self.inner.pool.begin().await?;
        if let Some(started) = started {
            started.send(()).map_err(|_| PrototypeError::ProbeDropped)?;
        }
        let fence_sql = format!(
            "SELECT generation, lineage_id, destination_generation,
                    executor_binding_ref, ledger_generation
             FROM {}.prototype_writer_fence
             WHERE fence_id = $1
             FOR UPDATE",
            self.inner.control_schema.as_str()
        );
        let fence = sqlx::query(audited_sql(fence_sql))
            .bind(FENCE_ID)
            .fetch_one(&mut *transaction)
            .await?;
        let generation: i64 = fence.try_get("generation")?;
        if generation != expected_generation {
            return Err(PrototypeError::GenerationChanged {
                expected: expected_generation,
                actual: generation,
            });
        }
        let lineage_id: String = fence.try_get("lineage_id")?;
        if lineage_id != candidate.instance.lineage_id {
            return Err(PrototypeError::CandidateLineageMismatch);
        }
        let executor_binding_ref: String = fence.try_get("executor_binding_ref")?;
        if executor_binding_ref != candidate.instance.executor_binding_ref {
            return Err(PrototypeError::CandidateBindingMismatch);
        }
        let ledger_generation: i64 = fence.try_get("ledger_generation")?;
        if ledger_generation != candidate.instance.ledger_generation {
            return Err(PrototypeError::CandidateLedgerGenerationMismatch);
        }
        self.validate_candidate(&mut transaction, &candidate)
            .await?;

        let destination_generation: i64 = fence.try_get("destination_generation")?;
        let destination_fence_sql = format!(
            "SELECT generation
             FROM {}.prototype_destination_fence
             WHERE singleton = TRUE
             FOR UPDATE",
            self.inner.destination_schema.as_str()
        );
        let persisted_destination_generation: i64 =
            sqlx::query_scalar(audited_sql(destination_fence_sql))
                .fetch_one(&mut *transaction)
                .await?;
        if persisted_destination_generation != destination_generation {
            return Err(PrototypeError::DestinationGenerationFenced);
        }
        let next_generation = generation
            .checked_add(1)
            .ok_or(PrototypeError::SequenceOverflow)?;
        let next_destination_generation = destination_generation
            .checked_add(1)
            .ok_or(PrototypeError::SequenceOverflow)?;
        let destination_update_sql = format!(
            "UPDATE {}.prototype_destination_fence
             SET generation = $1
             WHERE singleton = TRUE",
            self.inner.destination_schema.as_str()
        );
        sqlx::query(audited_sql(destination_update_sql))
            .bind(next_destination_generation)
            .execute(&mut *transaction)
            .await?;
        let fence_update_sql = format!(
            "UPDATE {}.prototype_writer_fence
             SET generation = $1,
                 writer_id = $2,
                 active_instance_id = $3,
                 destination_generation = $4
             WHERE fence_id = $5",
            self.inner.control_schema.as_str()
        );
        sqlx::query(audited_sql(fence_update_sql))
            .bind(next_generation)
            .bind(writer_id)
            .bind(&candidate.instance.instance_id)
            .bind(next_destination_generation)
            .bind(FENCE_ID)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;

        Ok(PromotedWriter {
            inner: Arc::clone(&self.inner),
            pool: self.inner.pool.clone(),
            instance: candidate.instance,
            token: WriterToken {
                generation: next_generation,
                writer_id: writer_id.to_owned(),
                destination_generation: next_destination_generation,
                executor_binding_ref,
                ledger_generation,
            },
        })
    }

    pub(super) async fn writer_generation(&self) -> Result<i64> {
        let sql = format!(
            "SELECT generation
             FROM {}.prototype_writer_fence
             WHERE fence_id = $1",
            self.inner.control_schema.as_str()
        );
        sqlx::query_scalar(audited_sql(sql))
            .bind(FENCE_ID)
            .fetch_one(&self.inner.pool)
            .await
            .map_err(PrototypeError::from)
    }

    pub(super) async fn candidate_remove_stream(
        &self,
        candidate: &RestoreCandidate,
        effect_id: &str,
    ) -> Result<()> {
        let stream_id = effect_stream_id(effect_id);
        let sql = format!(
            "DELETE FROM {schema}.prototype_journal_heads WHERE stream_id = $1;
             DELETE FROM {schema}.prototype_journal_records WHERE stream_id = $1;
             DELETE FROM {schema}.prototype_append_requests WHERE stream_id = $1",
            schema = candidate.instance.schema.as_str()
        );
        let mut transaction = self.inner.pool.begin().await?;
        for statement in sql
            .split(';')
            .filter(|statement| !statement.trim().is_empty())
        {
            sqlx::query(audited_sql(statement.to_owned()))
                .bind(&stream_id)
                .execute(&mut *transaction)
                .await?;
        }
        delete_unreferenced_evidence_objects(&mut transaction, &candidate.instance).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(super) async fn candidate_make_ancestor(
        &self,
        candidate: &RestoreCandidate,
        effect_id: &str,
    ) -> Result<()> {
        let stream_id = effect_stream_id(effect_id);
        let head = candidate_head(&self.inner.pool, &candidate.instance, &stream_id).await?;
        let previous_sequence = head
            .sequence
            .checked_sub(1)
            .filter(|sequence| *sequence > 0)
            .ok_or(PrototypeError::HeadMismatch)?;
        let digest_sql = format!(
            "SELECT commit_digest
             FROM {}.prototype_journal_records
             WHERE stream_id = $1 AND sequence = $2",
            candidate.instance.schema.as_str()
        );
        let previous_digest: Vec<u8> = sqlx::query_scalar(audited_sql(digest_sql))
            .bind(&stream_id)
            .bind(previous_sequence)
            .fetch_one(&self.inner.pool)
            .await?;
        let mut transaction = self.inner.pool.begin().await?;
        let record_sql = format!(
            "DELETE FROM {}.prototype_journal_records
             WHERE stream_id = $1 AND sequence > $2",
            candidate.instance.schema.as_str()
        );
        sqlx::query(audited_sql(record_sql))
            .bind(&stream_id)
            .bind(previous_sequence)
            .execute(&mut *transaction)
            .await?;
        let request_sql = format!(
            "DELETE FROM {}.prototype_append_requests
             WHERE stream_id = $1 AND committed_sequence > $2",
            candidate.instance.schema.as_str()
        );
        sqlx::query(audited_sql(request_sql))
            .bind(&stream_id)
            .bind(previous_sequence)
            .execute(&mut *transaction)
            .await?;
        let head_sql = format!(
            "UPDATE {}.prototype_journal_heads
             SET sequence = $1, commit_digest = $2
             WHERE stream_id = $3",
            candidate.instance.schema.as_str()
        );
        sqlx::query(audited_sql(head_sql))
            .bind(previous_sequence)
            .bind(previous_digest)
            .bind(&stream_id)
            .execute(&mut *transaction)
            .await?;
        delete_unreferenced_evidence_objects(&mut transaction, &candidate.instance).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(super) async fn candidate_append_extra(
        &self,
        candidate: &RestoreCandidate,
        effect_id: &str,
    ) -> Result<()> {
        let stream_id = effect_stream_id(effect_id);
        let head = candidate_head(&self.inner.pool, &candidate.instance, &stream_id).await?;
        let sequence = head
            .sequence
            .checked_add(1)
            .ok_or(PrototypeError::SequenceOverflow)?;
        let payload = b"opaque extra candidate payload";
        let digest = linked_digest("candidate_extra", Some(&head.digest), payload);
        let mut transaction = self.inner.pool.begin().await?;
        let object_digest =
            retain_evidence_object(&mut transaction, &candidate.instance, payload).await?;
        let proof_digest = binding_proof_digest(
            &candidate.instance,
            &stream_id,
            sequence,
            &digest,
            &object_digest,
        );
        let record_sql = format!(
            "INSERT INTO {}.prototype_journal_records
             (stream_id, sequence, predecessor_digest, record_kind, opaque_payload,
              commit_digest, executor_binding_ref, ledger_generation,
              evidence_object_digest, proof_digest)
             VALUES ($1, $2, $3, 'candidate_extra', $4, $5, $6, $7, $8, $9)",
            candidate.instance.schema.as_str()
        );
        sqlx::query(audited_sql(record_sql))
            .bind(&stream_id)
            .bind(sequence)
            .bind(&head.digest)
            .bind(payload.as_slice())
            .bind(&digest)
            .bind(&candidate.instance.executor_binding_ref)
            .bind(candidate.instance.ledger_generation)
            .bind(&object_digest)
            .bind(&proof_digest)
            .execute(&mut *transaction)
            .await?;
        let head_sql = format!(
            "UPDATE {}.prototype_journal_heads
             SET sequence = $1, commit_digest = $2
             WHERE stream_id = $3",
            candidate.instance.schema.as_str()
        );
        sqlx::query(audited_sql(head_sql))
            .bind(sequence)
            .bind(digest)
            .bind(stream_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(super) async fn candidate_corrupt_payload(
        &self,
        candidate: &RestoreCandidate,
        effect_id: &str,
    ) -> Result<()> {
        let stream_id = effect_stream_id(effect_id);
        let head = candidate_head(&self.inner.pool, &candidate.instance, &stream_id).await?;
        let sql = format!(
            "UPDATE {}.prototype_journal_records
             SET opaque_payload = $1
             WHERE stream_id = $2 AND sequence = $3",
            candidate.instance.schema.as_str()
        );
        sqlx::query(audited_sql(sql))
            .bind(b"corrupt payload".as_slice())
            .bind(stream_id)
            .bind(head.sequence)
            .execute(&self.inner.pool)
            .await?;
        Ok(())
    }

    pub(super) async fn candidate_fork_head(
        &self,
        candidate: &RestoreCandidate,
        effect_id: &str,
    ) -> Result<()> {
        let stream_id = effect_stream_id(effect_id);
        let head = candidate_head(&self.inner.pool, &candidate.instance, &stream_id).await?;
        let record_sql = format!(
            "SELECT predecessor_digest, record_kind, evidence_object_digest
             FROM {}.prototype_journal_records
             WHERE stream_id = $1 AND sequence = $2",
            candidate.instance.schema.as_str()
        );
        let row = sqlx::query(audited_sql(record_sql))
            .bind(&stream_id)
            .bind(head.sequence)
            .fetch_one(&self.inner.pool)
            .await?;
        let predecessor: Option<Vec<u8>> = row.try_get("predecessor_digest")?;
        let record_kind: String = row.try_get("record_kind")?;
        let old_object_digest: Vec<u8> = row.try_get("evidence_object_digest")?;
        let payload = b"opaque valid fork payload";
        let fork_digest = linked_digest(&record_kind, predecessor.as_deref(), payload);
        let mut transaction = self.inner.pool.begin().await?;
        let object_digest =
            retain_evidence_object(&mut transaction, &candidate.instance, payload).await?;
        let proof_digest = binding_proof_digest(
            &candidate.instance,
            &stream_id,
            head.sequence,
            &fork_digest,
            &object_digest,
        );
        let update_record_sql = format!(
            "UPDATE {}.prototype_journal_records
             SET opaque_payload = $1,
                 commit_digest = $2,
                 evidence_object_digest = $3,
                 proof_digest = $4
             WHERE stream_id = $5 AND sequence = $6",
            candidate.instance.schema.as_str()
        );
        sqlx::query(audited_sql(update_record_sql))
            .bind(payload.as_slice())
            .bind(&fork_digest)
            .bind(&object_digest)
            .bind(&proof_digest)
            .bind(&stream_id)
            .bind(head.sequence)
            .execute(&mut *transaction)
            .await?;
        let delete_old_object_sql = format!(
            "DELETE FROM {schema}.prototype_evidence_objects AS objects
             WHERE objects.object_digest = $1
               AND NOT EXISTS (
                 SELECT 1
                 FROM {schema}.prototype_journal_records AS records
                 WHERE records.evidence_object_digest = objects.object_digest
               )",
            schema = candidate.instance.schema.as_str()
        );
        sqlx::query(audited_sql(delete_old_object_sql))
            .bind(old_object_digest)
            .execute(&mut *transaction)
            .await?;
        let update_head_sql = format!(
            "UPDATE {}.prototype_journal_heads
             SET commit_digest = $1
             WHERE stream_id = $2",
            candidate.instance.schema.as_str()
        );
        sqlx::query(audited_sql(update_head_sql))
            .bind(fork_digest)
            .bind(stream_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(super) async fn candidate_change_binding(
        &self,
        candidate: &mut RestoreCandidate,
    ) -> Result<()> {
        candidate.instance.executor_binding_ref = "executor-binding:hostile-copy".to_owned();
        let sql = format!(
            "UPDATE {}.prototype_instance_identity
             SET executor_binding_ref = $1
             WHERE singleton = TRUE",
            candidate.instance.schema.as_str()
        );
        sqlx::query(audited_sql(sql))
            .bind(&candidate.instance.executor_binding_ref)
            .execute(&self.inner.pool)
            .await?;
        Ok(())
    }

    pub(super) async fn candidate_change_ledger_generation(
        &self,
        candidate: &mut RestoreCandidate,
    ) -> Result<()> {
        candidate.instance.ledger_generation = candidate
            .instance
            .ledger_generation
            .checked_add(1)
            .ok_or(PrototypeError::SequenceOverflow)?;
        let sql = format!(
            "UPDATE {}.prototype_instance_identity
             SET ledger_generation = $1
             WHERE singleton = TRUE",
            candidate.instance.schema.as_str()
        );
        sqlx::query(audited_sql(sql))
            .bind(candidate.instance.ledger_generation)
            .execute(&self.inner.pool)
            .await?;
        Ok(())
    }

    pub(super) async fn candidate_corrupt_record_binding(
        &self,
        candidate: &RestoreCandidate,
        effect_id: &str,
    ) -> Result<()> {
        let sql = format!(
            "UPDATE {}.prototype_journal_records
             SET executor_binding_ref = 'executor-binding:hostile-record-copy'
             WHERE stream_id = $1 AND sequence = (
               SELECT MAX(sequence)
               FROM {}.prototype_journal_records
               WHERE stream_id = $1
             )",
            candidate.instance.schema.as_str(),
            candidate.instance.schema.as_str()
        );
        sqlx::query(audited_sql(sql))
            .bind(effect_stream_id(effect_id))
            .execute(&self.inner.pool)
            .await?;
        Ok(())
    }

    pub(super) async fn candidate_corrupt_record_generation(
        &self,
        candidate: &RestoreCandidate,
        effect_id: &str,
    ) -> Result<()> {
        let sql = format!(
            "UPDATE {}.prototype_journal_records
             SET ledger_generation = ledger_generation + 1
             WHERE stream_id = $1 AND sequence = (
               SELECT MAX(sequence)
               FROM {}.prototype_journal_records
               WHERE stream_id = $1
             )",
            candidate.instance.schema.as_str(),
            candidate.instance.schema.as_str()
        );
        sqlx::query(audited_sql(sql))
            .bind(effect_stream_id(effect_id))
            .execute(&self.inner.pool)
            .await?;
        Ok(())
    }

    pub(super) async fn candidate_corrupt_proof(
        &self,
        candidate: &RestoreCandidate,
        effect_id: &str,
    ) -> Result<()> {
        let sql = format!(
            "UPDATE {}.prototype_journal_records
             SET proof_digest = decode(repeat('00', 32), 'hex')
             WHERE stream_id = $1 AND sequence = (
               SELECT MAX(sequence)
               FROM {}.prototype_journal_records
               WHERE stream_id = $1
             )",
            candidate.instance.schema.as_str(),
            candidate.instance.schema.as_str()
        );
        sqlx::query(audited_sql(sql))
            .bind(effect_stream_id(effect_id))
            .execute(&self.inner.pool)
            .await?;
        Ok(())
    }

    pub(super) async fn candidate_break_object_closure(
        &self,
        candidate: &RestoreCandidate,
        effect_id: &str,
    ) -> Result<()> {
        let sql = format!(
            "UPDATE {}.prototype_journal_records
             SET evidence_object_digest = decode(repeat('ff', 32), 'hex')
             WHERE stream_id = $1 AND sequence = (
               SELECT MAX(sequence)
               FROM {}.prototype_journal_records
               WHERE stream_id = $1
             )",
            candidate.instance.schema.as_str(),
            candidate.instance.schema.as_str()
        );
        sqlx::query(audited_sql(sql))
            .bind(effect_stream_id(effect_id))
            .execute(&self.inner.pool)
            .await?;
        Ok(())
    }

    async fn validate_candidate(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        candidate: &RestoreCandidate,
    ) -> Result<()> {
        let identity_sql = format!(
            "SELECT lineage_id, instance_id, executor_binding_ref, ledger_generation
             FROM {}.prototype_instance_identity
             WHERE singleton = TRUE",
            candidate.instance.schema.as_str()
        );
        let identity = sqlx::query(audited_sql(identity_sql))
            .fetch_one(&mut **transaction)
            .await?;
        let lineage_id: String = identity.try_get("lineage_id")?;
        let instance_id: String = identity.try_get("instance_id")?;
        let executor_binding_ref: String = identity.try_get("executor_binding_ref")?;
        let ledger_generation: i64 = identity.try_get("ledger_generation")?;
        if lineage_id != candidate.instance.lineage_id
            || instance_id != candidate.instance.instance_id
        {
            return Err(PrototypeError::CandidateLineageMismatch);
        }
        if executor_binding_ref != candidate.instance.executor_binding_ref {
            return Err(PrototypeError::CandidateBindingMismatch);
        }
        if ledger_generation != candidate.instance.ledger_generation {
            return Err(PrototypeError::CandidateLedgerGenerationMismatch);
        }

        let required_sql = format!(
            "SELECT stream_id, sequence, commit_digest
             FROM {}.prototype_required_heads
             ORDER BY stream_id",
            self.inner.control_schema.as_str()
        );
        let mut required = BTreeMap::new();
        for row in sqlx::query(audited_sql(required_sql))
            .fetch_all(&mut **transaction)
            .await?
        {
            required.insert(
                row.try_get::<String, _>("stream_id")?,
                JournalHead {
                    sequence: row.try_get("sequence")?,
                    digest: row.try_get("commit_digest")?,
                },
            );
        }
        let candidate_sql = format!(
            "SELECT stream_id, sequence, commit_digest
             FROM {}.prototype_journal_heads
             ORDER BY stream_id",
            candidate.instance.schema.as_str()
        );
        let mut candidate_heads = BTreeMap::new();
        for row in sqlx::query(audited_sql(candidate_sql))
            .fetch_all(&mut **transaction)
            .await?
        {
            let stream_id: String = row.try_get("stream_id")?;
            let head = JournalHead {
                sequence: row.try_get("sequence")?,
                digest: row.try_get("commit_digest")?,
            };
            candidate_heads.insert(stream_id, head);
        }
        let resource_head_sql = format!(
            "SELECT resource_ownership_ref, resource_key_ref, sequence, record_ref
             FROM {}.prototype_resource_heads
             ORDER BY resource_ownership_ref, resource_key_ref",
            candidate.instance.schema.as_str()
        );
        for row in sqlx::query(audited_sql(resource_head_sql))
            .fetch_all(&mut **transaction)
            .await?
        {
            let owner: String = row.try_get("resource_ownership_ref")?;
            let key: String = row.try_get("resource_key_ref")?;
            candidate_heads.insert(
                model::resource_stream_id(&owner, &key),
                JournalHead {
                    sequence: row.try_get("sequence")?,
                    digest: row.try_get("record_ref")?,
                },
            );
        }
        let configuration_head_sql = format!(
            "SELECT resource_ownership_ref, resource_key_ref, policy_configuration_ref
             FROM {}.prototype_resource_configurations
             ORDER BY resource_ownership_ref, resource_key_ref",
            candidate.instance.schema.as_str()
        );
        for row in sqlx::query(audited_sql(configuration_head_sql))
            .fetch_all(&mut **transaction)
            .await?
        {
            let owner: String = row.try_get("resource_ownership_ref")?;
            let key: String = row.try_get("resource_key_ref")?;
            candidate_heads.insert(
                model::resource_configuration_stream_id(&owner, &key),
                JournalHead {
                    sequence: 1,
                    digest: row.try_get("policy_configuration_ref")?,
                },
            );
        }
        classify_candidate_heads(&required, &candidate_heads)?;

        let authoritative_heads = restore::authoritative_heads(transaction, &candidate.instance)
            .await?
            .into_iter()
            .map(|(stream_id, sequence, digest)| (stream_id, JournalHead { sequence, digest }))
            .collect::<BTreeMap<_, _>>();
        classify_candidate_heads(&required, &authoritative_heads)?;
        restore::validate_derived(transaction, &candidate.instance)
            .await
            .map_err(|_| PrototypeError::CorruptCandidate {
                stream_id: "restore-projection-equality".to_owned(),
            })?;
        let exported = restore::export_authority(transaction, &candidate.instance).await?;
        let decoded = restore::decode_authority(&exported)?;
        if !restore::decoded_identity_matches_instance(&decoded, &candidate.instance)
            || exported != candidate.canonical_authority
        {
            return Err(PrototypeError::CorruptCandidate {
                stream_id: "restore-authority-envelope".to_owned(),
            });
        }
        Ok(())
    }
}

fn classify_candidate_heads(
    required: &BTreeMap<String, JournalHead>,
    candidate: &BTreeMap<String, JournalHead>,
) -> Result<()> {
    for (stream_id, required_head) in required {
        let Some(candidate_head) = candidate.get(stream_id) else {
            return Err(PrototypeError::MissingCandidate {
                stream_id: stream_id.clone(),
            });
        };
        if candidate_head.sequence < required_head.sequence {
            return Err(PrototypeError::AncestorCandidate {
                stream_id: stream_id.clone(),
            });
        }
        if candidate_head.sequence > required_head.sequence {
            return Err(PrototypeError::ExtraCandidate {
                stream_id: stream_id.clone(),
            });
        }
        if candidate_head.digest != required_head.digest {
            return Err(PrototypeError::ForkedCandidate {
                stream_id: stream_id.clone(),
            });
        }
    }
    if let Some(extra) = candidate
        .keys()
        .find(|stream_id| !required.contains_key(*stream_id))
    {
        return Err(PrototypeError::ExtraCandidate {
            stream_id: extra.clone(),
        });
    }
    Ok(())
}

async fn candidate_head(
    pool: &PgPool,
    instance: &StoreInstance,
    stream_id: &str,
) -> Result<JournalHead> {
    let sql = format!(
        "SELECT sequence, commit_digest
         FROM {}.prototype_journal_heads
         WHERE stream_id = $1",
        instance.schema.as_str()
    );
    let row = sqlx::query(audited_sql(sql))
        .bind(stream_id)
        .fetch_optional(pool)
        .await?
        .ok_or(PrototypeError::HeadMismatch)?;
    Ok(JournalHead {
        sequence: row.try_get("sequence")?,
        digest: row.try_get("commit_digest")?,
    })
}
