#![allow(clippy::disallowed_methods)]
#![warn(missing_docs)]
//! PostgreSQL-backed control-plane storage.
//!
//! This crate owns correctness-critical control-plane persistence that must share the same physical
//! PostgreSQL database and SQL transaction boundary as the shared append-only stream substrate.
//! The initial slice implements durable `rpc_source:*` stream-family appends plus a rebuildable
//! `mfm_rpc_source_state` projection table.
//!
//! # Examples
//!
//! ```no_run
//! use mfm_control_plane_postgres::{ControlPlanePostgresStore, RpcSourceRef};
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() -> Result<(), mfm_machine::errors::StorageError> {
//! let store =
//!     ControlPlanePostgresStore::connect("postgres://postgres:postgres@localhost/mfm").await?;
//! let source = RpcSourceRef::new("eth-mainnet", "primary").expect("valid source ref");
//! let _state = store.rpc_source_state(&source).await?;
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio_postgres::{Client, NoTls, Row, Transaction};
use tracing::{debug, info};

use mfm_machine::errors::{ErrorCategory, ErrorInfo, StorageError};
use mfm_machine::ids::ErrorCode;
use mfm_machine::stores::{NewStreamRecord, StreamId, StreamRecord};

/// Stream family used for source-quality and probe history.
pub const RPC_SOURCE_STREAM_FAMILY: &str = "rpc_source";

const RECORD_KIND_SOURCE_OBSERVED: &str = "source_observed";
const RECORD_KIND_SOURCE_PROBED: &str = "source_probed";

fn storage_info(code: &'static str, message: impl Into<String>) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category: ErrorCategory::Storage,
        retryable: false,
        message: message.into(),
        details: None,
    }
}

fn storage_other(code: &'static str, message: impl Into<String>) -> StorageError {
    StorageError::Other(storage_info(code, message))
}

fn storage_corruption(code: &'static str, message: impl Into<String>) -> StorageError {
    StorageError::Corruption(storage_info(code, message))
}

fn storage_concurrency(message: impl Into<String>) -> StorageError {
    StorageError::Concurrency(storage_info("control_plane_concurrency", message))
}

fn validate_component(
    name: &'static str,
    value: impl Into<String>,
) -> Result<String, RpcSourceRefError> {
    let value = value.into();
    if value.is_empty()
        || value.contains(':')
        || value
            .chars()
            .any(|ch| ch.is_ascii_control() || ch.is_whitespace())
    {
        return Err(RpcSourceRefError::InvalidComponent { name, value });
    }
    Ok(value)
}

fn u64_to_i64(value: u64, field: &'static str) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| {
        storage_other(
            "control_plane_value_out_of_range",
            format!("{field} exceeded the supported PostgreSQL bigint range"),
        )
    })
}

fn i64_to_u64(value: i64, field: &'static str) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| {
        storage_corruption(
            "control_plane_projection_invalid",
            format!("{field} contained a negative bigint value"),
        )
    })
}

fn opt_i64_to_u64(value: Option<i64>, field: &'static str) -> Result<Option<u64>, StorageError> {
    value.map(|value| i64_to_u64(value, field)).transpose()
}

/// Validation error for [`RpcSourceRef`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RpcSourceRefError {
    /// One of the id components was empty or contained a reserved character.
    InvalidComponent {
        /// The invalid field name.
        name: &'static str,
        /// The invalid value.
        value: String,
    },
    /// The stream id did not follow the `rpc_source:<network_id>:<source_id>` shape.
    InvalidStreamId(String),
}

impl fmt::Display for RpcSourceRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcSourceRefError::InvalidComponent { name, value } => {
                write!(f, "{name} must be non-empty and must not contain whitespace, control characters, or ':' (got `{value}`)")
            }
            RpcSourceRefError::InvalidStreamId(value) => write!(
                f,
                "rpc source stream id must follow `rpc_source:<network_id>:<source_id>` (got `{value}`)"
            ),
        }
    }
}

impl std::error::Error for RpcSourceRefError {}

/// Stable identity for one control-plane-managed RPC source.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RpcSourceRef {
    network_id: String,
    source_id: String,
}

impl RpcSourceRef {
    /// Creates a validated source reference.
    pub fn new(
        network_id: impl Into<String>,
        source_id: impl Into<String>,
    ) -> Result<Self, RpcSourceRefError> {
        Ok(Self {
            network_id: validate_component("network_id", network_id)?,
            source_id: validate_component("source_id", source_id)?,
        })
    }

    /// Parses a typed source reference from a stream id.
    pub fn from_stream_id(stream_id: &StreamId) -> Result<Self, RpcSourceRefError> {
        if stream_id.family() != RPC_SOURCE_STREAM_FAMILY {
            return Err(RpcSourceRefError::InvalidStreamId(
                stream_id.as_str().to_string(),
            ));
        }
        let Some((network_id, source_id)) = stream_id.key().split_once(':') else {
            return Err(RpcSourceRefError::InvalidStreamId(
                stream_id.as_str().to_string(),
            ));
        };
        Self::new(network_id, source_id)
    }

    /// Returns the network identifier.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the source identifier.
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    /// Returns the canonical stream id for this source.
    pub fn stream_id(&self) -> StreamId {
        StreamId::must_new(format!(
            "{RPC_SOURCE_STREAM_FAMILY}:{}:{}",
            self.network_id, self.source_id
        ))
    }
}

impl fmt::Display for RpcSourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.network_id, self.source_id)
    }
}

/// Success or failure outcome for an observation or probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RpcSourceOutcome {
    /// The call or probe succeeded.
    Success,
    /// The call or probe failed.
    Failure,
}

/// Probe type recorded for one RPC source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RpcSourceProbeKind {
    /// Generic liveness or readiness probe.
    Basic,
    /// Capability probe for `eth_getProof`.
    GetProof,
}

/// Durable `source_observed` payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcSourceObservedRecord {
    /// Observation timestamp in milliseconds since the Unix epoch.
    pub observed_at_ms: u64,
    /// Whether the observed call succeeded.
    pub outcome: RpcSourceOutcome,
    /// Latest head observed from the source, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_block_number: Option<u64>,
    /// End-to-end latency in milliseconds, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    /// Cooldown deadline in milliseconds since the Unix epoch, when a failure triggered cooldown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_until_ms: Option<u64>,
    /// Stable diagnostic code describing the failure class, when relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic_code: Option<String>,
}

/// Durable `source_probed` payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcSourceProbedRecord {
    /// Probe timestamp in milliseconds since the Unix epoch.
    pub probed_at_ms: u64,
    /// Probe type that produced the record.
    pub probe_kind: RpcSourceProbeKind,
    /// Whether the probe succeeded.
    pub outcome: RpcSourceOutcome,
    /// Probe latency in milliseconds, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    /// Whether the source supports `eth_getProof`, when the probe established that fact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_get_proof: Option<bool>,
    /// Cooldown deadline in milliseconds since the Unix epoch, when a failed probe triggered cooldown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown_until_ms: Option<u64>,
    /// Stable diagnostic code describing the probe failure, when relevant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic_code: Option<String>,
}

/// Typed `rpc_source:*` stream-family record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RpcSourceRecord {
    /// `source_observed`
    Observed(RpcSourceObservedRecord),
    /// `source_probed`
    Probed(RpcSourceProbedRecord),
}

impl RpcSourceRecord {
    fn kind(&self) -> &'static str {
        match self {
            RpcSourceRecord::Observed(_) => RECORD_KIND_SOURCE_OBSERVED,
            RpcSourceRecord::Probed(_) => RECORD_KIND_SOURCE_PROBED,
        }
    }

    fn recorded_at_ms(&self) -> u64 {
        match self {
            RpcSourceRecord::Observed(record) => record.observed_at_ms,
            RpcSourceRecord::Probed(record) => record.probed_at_ms,
        }
    }

    fn to_new_stream_record(&self) -> Result<NewStreamRecord, StorageError> {
        let payload = match self {
            RpcSourceRecord::Observed(record) => serde_json::to_value(record),
            RpcSourceRecord::Probed(record) => serde_json::to_value(record),
        }
        .map_err(|err| {
            storage_other(
                "control_plane_record_encode_failed",
                format!("failed to encode rpc_source record payload: {err}"),
            )
        })?;

        Ok(NewStreamRecord {
            ts_millis: Some(self.recorded_at_ms()),
            kind: self.kind().to_string(),
            payload,
        })
    }

    fn from_stream_record(record: &StreamRecord) -> Result<Self, RpcSourceProjectionError> {
        match record.kind.as_str() {
            RECORD_KIND_SOURCE_OBSERVED => serde_json::from_value(record.payload.clone())
                .map(RpcSourceRecord::Observed)
                .map_err(|err| RpcSourceProjectionError::InvalidPayload {
                    kind: record.kind.clone(),
                    message: err.to_string(),
                }),
            RECORD_KIND_SOURCE_PROBED => serde_json::from_value(record.payload.clone())
                .map(RpcSourceRecord::Probed)
                .map_err(|err| RpcSourceProjectionError::InvalidPayload {
                    kind: record.kind.clone(),
                    message: err.to_string(),
                }),
            other => Err(RpcSourceProjectionError::UnsupportedRecordKind(
                other.to_string(),
            )),
        }
    }
}

/// Rebuildable projection for one `rpc_source:*` stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcSourceState {
    /// Stable source identity.
    pub source_ref: RpcSourceRef,
    /// Canonical stream id backing this projection.
    pub stream_id: StreamId,
    /// Last projected stream sequence.
    pub head_seq: u64,
    /// Timestamp of the most recent record applied to this projection.
    pub last_recorded_at_ms: Option<u64>,
    /// Timestamp of the most recent `source_observed` record.
    pub last_observed_at_ms: Option<u64>,
    /// Timestamp of the most recent `source_probed` record.
    pub last_probed_at_ms: Option<u64>,
    /// Most recent observed head block number.
    pub last_observed_head: Option<u64>,
    /// Most recent call latency sample.
    pub last_latency_ms: Option<u64>,
    /// Most recent probe latency sample.
    pub last_probe_latency_ms: Option<u64>,
    /// Most recent known `eth_getProof` capability bit.
    pub supports_get_proof: Option<bool>,
    /// Total success samples applied to this source.
    pub success_count: u64,
    /// Total failure samples applied to this source.
    pub failure_count: u64,
    /// Consecutive failures since the last success sample.
    pub consecutive_failures: u64,
    /// Current cooldown deadline, if the source is cooling down.
    pub cooldown_until_ms: Option<u64>,
    /// Stable diagnostic code from the most recent failure, if any.
    pub last_error_code: Option<String>,
}

impl RpcSourceState {
    /// Creates an empty projection for `source_ref`.
    pub fn new(source_ref: RpcSourceRef) -> Self {
        let stream_id = source_ref.stream_id();
        Self {
            source_ref,
            stream_id,
            head_seq: 0,
            last_recorded_at_ms: None,
            last_observed_at_ms: None,
            last_probed_at_ms: None,
            last_observed_head: None,
            last_latency_ms: None,
            last_probe_latency_ms: None,
            supports_get_proof: None,
            success_count: 0,
            failure_count: 0,
            consecutive_failures: 0,
            cooldown_until_ms: None,
            last_error_code: None,
        }
    }

    fn apply_record(&mut self, seq: u64, record: &RpcSourceRecord) {
        self.head_seq = seq;
        self.last_recorded_at_ms = Some(record.recorded_at_ms());

        match record {
            RpcSourceRecord::Observed(observed) => {
                self.last_observed_at_ms = Some(observed.observed_at_ms);
                if let Some(head_block_number) = observed.head_block_number {
                    self.last_observed_head = Some(head_block_number);
                }
                if let Some(latency_ms) = observed.latency_ms {
                    self.last_latency_ms = Some(latency_ms);
                }
                self.apply_outcome(
                    observed.outcome,
                    observed.cooldown_until_ms,
                    observed.diagnostic_code.as_deref(),
                );
            }
            RpcSourceRecord::Probed(probed) => {
                self.last_probed_at_ms = Some(probed.probed_at_ms);
                if let Some(latency_ms) = probed.latency_ms {
                    self.last_probe_latency_ms = Some(latency_ms);
                }
                if let Some(supports_get_proof) = probed.supports_get_proof {
                    self.supports_get_proof = Some(supports_get_proof);
                }
                self.apply_outcome(
                    probed.outcome,
                    probed.cooldown_until_ms,
                    probed.diagnostic_code.as_deref(),
                );
            }
        }
    }

    fn apply_outcome(
        &mut self,
        outcome: RpcSourceOutcome,
        cooldown_until_ms: Option<u64>,
        diagnostic_code: Option<&str>,
    ) {
        match outcome {
            RpcSourceOutcome::Success => {
                self.success_count = self.success_count.saturating_add(1);
                self.consecutive_failures = 0;
                self.cooldown_until_ms = None;
                self.last_error_code = None;
            }
            RpcSourceOutcome::Failure => {
                self.failure_count = self.failure_count.saturating_add(1);
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
                if let Some(cooldown_until_ms) = cooldown_until_ms {
                    self.cooldown_until_ms = Some(cooldown_until_ms);
                }
                self.last_error_code = diagnostic_code.map(ToOwned::to_owned);
            }
        }
    }
}

/// Rebuild error for `rpc_source:*` projections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RpcSourceProjectionError {
    /// The stream id did not belong to the expected `RpcSourceRef`.
    StreamIdMismatch {
        /// Expected stream id.
        expected: StreamId,
        /// Actual stream id.
        actual: StreamId,
    },
    /// The records were not contiguous from `seq = 1`.
    NonContiguousSeq {
        /// Expected next sequence number.
        expected: u64,
        /// Observed sequence number.
        actual: u64,
    },
    /// The stream id did not parse as `rpc_source:<network_id>:<source_id>`.
    InvalidStreamId(String),
    /// The record kind was not one of the v1 `rpc_source:*` kinds.
    UnsupportedRecordKind(String),
    /// The record payload did not decode into the typed v1 shape.
    InvalidPayload {
        /// Record kind being decoded.
        kind: String,
        /// Serde error message.
        message: String,
    },
}

impl fmt::Display for RpcSourceProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcSourceProjectionError::StreamIdMismatch { expected, actual } => write!(
                f,
                "rpc_source rebuild expected stream `{expected}` but found `{actual}`"
            ),
            RpcSourceProjectionError::NonContiguousSeq { expected, actual } => write!(
                f,
                "rpc_source rebuild expected seq {expected} but found {actual}"
            ),
            RpcSourceProjectionError::InvalidStreamId(value) => {
                write!(f, "invalid rpc_source stream id `{value}`")
            }
            RpcSourceProjectionError::UnsupportedRecordKind(kind) => {
                write!(f, "unsupported rpc_source record kind `{kind}`")
            }
            RpcSourceProjectionError::InvalidPayload { kind, message } => {
                write!(f, "invalid `{kind}` payload: {message}")
            }
        }
    }
}

impl std::error::Error for RpcSourceProjectionError {}

/// Rebuilds one `rpc_source:*` projection from append-only stream records.
pub fn rebuild_rpc_source_state(
    source_ref: RpcSourceRef,
    records: &[StreamRecord],
) -> Result<RpcSourceState, RpcSourceProjectionError> {
    let expected_stream_id = source_ref.stream_id();
    let mut expected_seq = 1_u64;
    let mut state = RpcSourceState::new(source_ref);

    for record in records {
        if record.stream_id != expected_stream_id {
            return Err(RpcSourceProjectionError::StreamIdMismatch {
                expected: expected_stream_id.clone(),
                actual: record.stream_id.clone(),
            });
        }
        if record.seq != expected_seq {
            return Err(RpcSourceProjectionError::NonContiguousSeq {
                expected: expected_seq,
                actual: record.seq,
            });
        }
        let typed = RpcSourceRecord::from_stream_record(record)?;
        state.apply_record(record.seq, &typed);
        expected_seq = expected_seq.saturating_add(1);
    }

    Ok(state)
}

/// Correctness-critical Postgres storage for control-plane stream families.
#[derive(Clone)]
pub struct ControlPlanePostgresStore {
    client: Arc<Mutex<Client>>,
}

impl ControlPlanePostgresStore {
    /// Connects to PostgreSQL, ensures the control-plane schema, and returns a ready store.
    pub async fn connect(database_url: &str) -> Result<Self, StorageError> {
        info!("connecting postgres control-plane store");
        let (client, connection) = tokio_postgres::connect(database_url, NoTls)
            .await
            .map_err(|_| storage_other("control_plane_pg_connect_failed", "connect failed"))?;

        tokio::spawn(async move {
            let _ = connection.await;
        });

        let store = Self {
            client: Arc::new(Mutex::new(client)),
        };
        store.init().await?;
        info!("postgres control-plane store connected");
        Ok(store)
    }

    /// Connects using the `DATABASE_URL` environment variable.
    pub async fn connect_env() -> Result<Self, StorageError> {
        let database_url = std::env::var("DATABASE_URL")
            .map_err(|_| storage_other("control_plane_pg_missing_env", "missing DATABASE_URL"))?;
        Self::connect(&database_url).await
    }

    async fn init(&self) -> Result<(), StorageError> {
        let ddl = r#"
CREATE TABLE IF NOT EXISTS mfm_streams (
  stream_id TEXT PRIMARY KEY,
  head_seq BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS mfm_stream_records (
  stream_id TEXT NOT NULL,
  seq BIGINT NOT NULL,
  ts_millis BIGINT NULL,
  kind TEXT NOT NULL,
  payload JSONB NOT NULL,
  PRIMARY KEY (stream_id, seq),
  CONSTRAINT mfm_stream_records_stream_fk FOREIGN KEY (stream_id) REFERENCES mfm_streams(stream_id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS mfm_rpc_source_state (
  network_id TEXT NOT NULL,
  source_id TEXT NOT NULL,
  stream_id TEXT NOT NULL UNIQUE,
  head_seq BIGINT NOT NULL,
  last_recorded_at_ms BIGINT NULL,
  last_observed_at_ms BIGINT NULL,
  last_probed_at_ms BIGINT NULL,
  last_observed_head BIGINT NULL,
  last_latency_ms BIGINT NULL,
  last_probe_latency_ms BIGINT NULL,
  supports_get_proof BOOLEAN NULL,
  success_count BIGINT NOT NULL,
  failure_count BIGINT NOT NULL,
  consecutive_failures BIGINT NOT NULL,
  cooldown_until_ms BIGINT NULL,
  last_error_code TEXT NULL,
  PRIMARY KEY (network_id, source_id),
  CONSTRAINT mfm_rpc_source_state_stream_fk FOREIGN KEY (stream_id) REFERENCES mfm_streams(stream_id) ON DELETE CASCADE
);
"#;

        self.client
            .lock()
            .await
            .batch_execute(ddl)
            .await
            .map_err(|_| storage_other("control_plane_pg_init_failed", "init failed"))?;
        debug!("postgres control-plane schema ensured");
        Ok(())
    }

    async fn read_head_for_update(
        tx: &Transaction<'_>,
        stream_id: &StreamId,
    ) -> Result<u64, StorageError> {
        let row = tx
            .query_one(
                "SELECT head_seq FROM mfm_streams WHERE stream_id = $1 FOR UPDATE",
                &[&stream_id.as_str()],
            )
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_query_failed",
                    "failed to read control-plane stream head",
                )
            })?;

        i64_to_u64(row.get::<_, i64>(0), "mfm_streams.head_seq")
    }

    async fn load_rpc_source_state_tx(
        tx: &Transaction<'_>,
        source_ref: &RpcSourceRef,
    ) -> Result<Option<RpcSourceState>, StorageError> {
        let row = tx
            .query_opt(
                r#"
SELECT
  network_id,
  source_id,
  stream_id,
  head_seq,
  last_recorded_at_ms,
  last_observed_at_ms,
  last_probed_at_ms,
  last_observed_head,
  last_latency_ms,
  last_probe_latency_ms,
  supports_get_proof,
  success_count,
  failure_count,
  consecutive_failures,
  cooldown_until_ms,
  last_error_code
FROM mfm_rpc_source_state
WHERE network_id = $1 AND source_id = $2
"#,
                &[&source_ref.network_id(), &source_ref.source_id()],
            )
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_query_failed",
                    "failed to load rpc_source projection row",
                )
            })?;

        row.map(Self::rpc_source_state_from_row).transpose()
    }

    fn rpc_source_state_from_row(row: Row) -> Result<RpcSourceState, StorageError> {
        let network_id: String = row.get(0);
        let source_id: String = row.get(1);
        let source_ref = RpcSourceRef::new(network_id, source_id).map_err(|err| {
            storage_corruption(
                "control_plane_projection_invalid",
                format!("invalid rpc_source projection identity: {err}"),
            )
        })?;
        let stream_id = StreamId::new(row.get::<_, String>(2)).map_err(|err| {
            storage_corruption(
                "control_plane_projection_invalid",
                format!("invalid rpc_source projection stream id: {err}"),
            )
        })?;
        Ok(RpcSourceState {
            source_ref,
            stream_id,
            head_seq: i64_to_u64(row.get::<_, i64>(3), "mfm_rpc_source_state.head_seq")?,
            last_recorded_at_ms: opt_i64_to_u64(
                row.get::<_, Option<i64>>(4),
                "mfm_rpc_source_state.last_recorded_at_ms",
            )?,
            last_observed_at_ms: opt_i64_to_u64(
                row.get::<_, Option<i64>>(5),
                "mfm_rpc_source_state.last_observed_at_ms",
            )?,
            last_probed_at_ms: opt_i64_to_u64(
                row.get::<_, Option<i64>>(6),
                "mfm_rpc_source_state.last_probed_at_ms",
            )?,
            last_observed_head: opt_i64_to_u64(
                row.get::<_, Option<i64>>(7),
                "mfm_rpc_source_state.last_observed_head",
            )?,
            last_latency_ms: opt_i64_to_u64(
                row.get::<_, Option<i64>>(8),
                "mfm_rpc_source_state.last_latency_ms",
            )?,
            last_probe_latency_ms: opt_i64_to_u64(
                row.get::<_, Option<i64>>(9),
                "mfm_rpc_source_state.last_probe_latency_ms",
            )?,
            supports_get_proof: row.get(10),
            success_count: i64_to_u64(row.get::<_, i64>(11), "mfm_rpc_source_state.success_count")?,
            failure_count: i64_to_u64(row.get::<_, i64>(12), "mfm_rpc_source_state.failure_count")?,
            consecutive_failures: i64_to_u64(
                row.get::<_, i64>(13),
                "mfm_rpc_source_state.consecutive_failures",
            )?,
            cooldown_until_ms: opt_i64_to_u64(
                row.get::<_, Option<i64>>(14),
                "mfm_rpc_source_state.cooldown_until_ms",
            )?,
            last_error_code: row.get(15),
        })
    }

    async fn upsert_rpc_source_state_tx(
        tx: &Transaction<'_>,
        state: &RpcSourceState,
    ) -> Result<(), StorageError> {
        tx.execute(
            r#"
INSERT INTO mfm_rpc_source_state (
  network_id,
  source_id,
  stream_id,
  head_seq,
  last_recorded_at_ms,
  last_observed_at_ms,
  last_probed_at_ms,
  last_observed_head,
  last_latency_ms,
  last_probe_latency_ms,
  supports_get_proof,
  success_count,
  failure_count,
  consecutive_failures,
  cooldown_until_ms,
  last_error_code
) VALUES (
  $1, $2, $3, $4, $5, $6, $7, $8,
  $9, $10, $11, $12, $13, $14, $15, $16
)
ON CONFLICT (network_id, source_id) DO UPDATE SET
  stream_id = EXCLUDED.stream_id,
  head_seq = EXCLUDED.head_seq,
  last_recorded_at_ms = EXCLUDED.last_recorded_at_ms,
  last_observed_at_ms = EXCLUDED.last_observed_at_ms,
  last_probed_at_ms = EXCLUDED.last_probed_at_ms,
  last_observed_head = EXCLUDED.last_observed_head,
  last_latency_ms = EXCLUDED.last_latency_ms,
  last_probe_latency_ms = EXCLUDED.last_probe_latency_ms,
  supports_get_proof = EXCLUDED.supports_get_proof,
  success_count = EXCLUDED.success_count,
  failure_count = EXCLUDED.failure_count,
  consecutive_failures = EXCLUDED.consecutive_failures,
  cooldown_until_ms = EXCLUDED.cooldown_until_ms,
  last_error_code = EXCLUDED.last_error_code
"#,
            &[
                &state.source_ref.network_id(),
                &state.source_ref.source_id(),
                &state.stream_id.as_str(),
                &u64_to_i64(state.head_seq, "head_seq")?,
                &state
                    .last_recorded_at_ms
                    .map(|value| u64_to_i64(value, "last_recorded_at_ms"))
                    .transpose()?,
                &state
                    .last_observed_at_ms
                    .map(|value| u64_to_i64(value, "last_observed_at_ms"))
                    .transpose()?,
                &state
                    .last_probed_at_ms
                    .map(|value| u64_to_i64(value, "last_probed_at_ms"))
                    .transpose()?,
                &state
                    .last_observed_head
                    .map(|value| u64_to_i64(value, "last_observed_head"))
                    .transpose()?,
                &state
                    .last_latency_ms
                    .map(|value| u64_to_i64(value, "last_latency_ms"))
                    .transpose()?,
                &state
                    .last_probe_latency_ms
                    .map(|value| u64_to_i64(value, "last_probe_latency_ms"))
                    .transpose()?,
                &state.supports_get_proof,
                &u64_to_i64(state.success_count, "success_count")?,
                &u64_to_i64(state.failure_count, "failure_count")?,
                &u64_to_i64(state.consecutive_failures, "consecutive_failures")?,
                &state
                    .cooldown_until_ms
                    .map(|value| u64_to_i64(value, "cooldown_until_ms"))
                    .transpose()?,
                &state.last_error_code,
            ],
        )
        .await
        .map_err(|_| {
            storage_other(
                "control_plane_pg_update_failed",
                "failed to upsert rpc_source projection row",
            )
        })?;
        Ok(())
    }

    /// Appends one atomic batch of `rpc_source:*` records and updates the projection in the same transaction.
    pub async fn append_rpc_source_records(
        &self,
        source_ref: &RpcSourceRef,
        expected_seq: u64,
        records: Vec<RpcSourceRecord>,
    ) -> Result<RpcSourceState, StorageError> {
        if records.is_empty() {
            return Err(storage_other(
                "control_plane_append_invalid",
                "rpc_source append must include at least one record",
            ));
        }

        let stream_id = source_ref.stream_id();
        let encoded_records = records
            .iter()
            .map(RpcSourceRecord::to_new_stream_record)
            .collect::<Result<Vec<_>, _>>()?;

        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(|_| {
            storage_other("control_plane_pg_tx_failed", "failed to start transaction")
        })?;

        tx.execute(
            "INSERT INTO mfm_streams (stream_id, head_seq) VALUES ($1, 0) ON CONFLICT (stream_id) DO NOTHING",
            &[&stream_id.as_str()],
        )
        .await
        .map_err(|_| storage_other("control_plane_pg_insert_failed", "failed to insert stream head"))?;

        let head = Self::read_head_for_update(&tx, &stream_id).await?;
        if head != expected_seq {
            return Err(storage_concurrency(format!(
                "rpc_source head seq did not match expected seq for `{}`",
                stream_id
            )));
        }

        let existing = Self::load_rpc_source_state_tx(&tx, source_ref).await?;
        if let Some(existing) = &existing {
            if existing.head_seq != head {
                return Err(storage_corruption(
                    "control_plane_projection_desynced",
                    format!(
                        "rpc_source projection head {} did not match stream head {} for `{}`",
                        existing.head_seq, head, stream_id
                    ),
                ));
            }
        }

        let mut state = existing.unwrap_or_else(|| RpcSourceState::new(source_ref.clone()));
        let mut seq = expected_seq;
        for (record, encoded) in records.iter().zip(encoded_records.iter()) {
            seq = seq.saturating_add(1);
            tx.execute(
                "INSERT INTO mfm_stream_records (stream_id, seq, ts_millis, kind, payload) VALUES ($1, $2, $3, $4, $5)",
                &[
                    &stream_id.as_str(),
                    &u64_to_i64(seq, "seq")?,
                    &encoded.ts_millis.map(|value| u64_to_i64(value, "ts_millis")).transpose()?,
                    &encoded.kind,
                    &encoded.payload,
                ],
            )
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_insert_failed",
                    "failed to insert rpc_source stream record",
                )
            })?;
            state.apply_record(seq, record);
        }

        Self::upsert_rpc_source_state_tx(&tx, &state).await?;
        tx.execute(
            "UPDATE mfm_streams SET head_seq = $2 WHERE stream_id = $1",
            &[&stream_id.as_str(), &u64_to_i64(seq, "head_seq")?],
        )
        .await
        .map_err(|_| {
            storage_other(
                "control_plane_pg_update_failed",
                "failed to update rpc_source stream head",
            )
        })?;

        tx.commit().await.map_err(|_| {
            storage_other("control_plane_pg_tx_failed", "failed to commit transaction")
        })?;

        Ok(state)
    }

    /// Loads the current projection row for one RPC source.
    pub async fn rpc_source_state(
        &self,
        source_ref: &RpcSourceRef,
    ) -> Result<Option<RpcSourceState>, StorageError> {
        let client = self.client.lock().await;
        let row = client
            .query_opt(
                r#"
SELECT
  network_id,
  source_id,
  stream_id,
  head_seq,
  last_recorded_at_ms,
  last_observed_at_ms,
  last_probed_at_ms,
  last_observed_head,
  last_latency_ms,
  last_probe_latency_ms,
  supports_get_proof,
  success_count,
  failure_count,
  consecutive_failures,
  cooldown_until_ms,
  last_error_code
FROM mfm_rpc_source_state
WHERE network_id = $1 AND source_id = $2
"#,
                &[&source_ref.network_id(), &source_ref.source_id()],
            )
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_query_failed",
                    "failed to query rpc_source projection row",
                )
            })?;
        row.map(Self::rpc_source_state_from_row).transpose()
    }

    /// Rebuilds every `mfm_rpc_source_state` row from append-only `rpc_source:*` stream records.
    pub async fn rebuild_all_rpc_source_states(&self) -> Result<Vec<RpcSourceState>, StorageError> {
        let mut client = self.client.lock().await;
        let tx = client.transaction().await.map_err(|_| {
            storage_other("control_plane_pg_tx_failed", "failed to start transaction")
        })?;

        let rows = tx
            .query(
                r#"
SELECT stream_id, seq, ts_millis, kind, payload
FROM mfm_stream_records
WHERE stream_id LIKE 'rpc_source:%'
ORDER BY stream_id ASC, seq ASC
"#,
                &[],
            )
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_query_failed",
                    "failed to query rpc_source stream records",
                )
            })?;

        let mut by_stream = BTreeMap::<String, Vec<StreamRecord>>::new();
        for row in rows {
            let stream_id = StreamId::new(row.get::<_, String>(0)).map_err(|err| {
                storage_corruption(
                    "control_plane_stream_invalid",
                    format!("invalid rpc_source stream id in record table: {err}"),
                )
            })?;
            let record = StreamRecord {
                stream_id: stream_id.clone(),
                seq: i64_to_u64(row.get::<_, i64>(1), "mfm_stream_records.seq")?,
                ts_millis: opt_i64_to_u64(
                    row.get::<_, Option<i64>>(2),
                    "mfm_stream_records.ts_millis",
                )?,
                kind: row.get(3),
                payload: row.get(4),
            };
            by_stream
                .entry(stream_id.as_str().to_string())
                .or_default()
                .push(record);
        }

        let mut rebuilt = Vec::with_capacity(by_stream.len());
        for records in by_stream.into_values() {
            let source_ref =
                RpcSourceRef::from_stream_id(&records[0].stream_id).map_err(|err| {
                    storage_corruption(
                        "control_plane_stream_invalid",
                        format!("invalid rpc_source stream id in rebuild: {err}"),
                    )
                })?;
            let state = rebuild_rpc_source_state(source_ref, &records).map_err(|err| {
                storage_corruption(
                    "control_plane_projection_rebuild_failed",
                    format!("failed to rebuild rpc_source projection: {err}"),
                )
            })?;
            rebuilt.push(state);
        }

        tx.execute("DELETE FROM mfm_rpc_source_state", &[])
            .await
            .map_err(|_| {
                storage_other(
                    "control_plane_pg_delete_failed",
                    "failed to clear rpc_source projection table during rebuild",
                )
            })?;

        for state in &rebuilt {
            Self::upsert_rpc_source_state_tx(&tx, state).await?;
        }

        tx.commit().await.map_err(|_| {
            storage_other("control_plane_pg_tx_failed", "failed to commit transaction")
        })?;

        Ok(rebuilt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_ref() -> RpcSourceRef {
        RpcSourceRef::new("eth-mainnet", "primary").expect("valid source ref")
    }

    #[test]
    fn rpc_source_ref_round_trips_through_stream_id() {
        let source_ref = source_ref();
        let stream_id = source_ref.stream_id();
        let decoded = RpcSourceRef::from_stream_id(&stream_id).expect("decode stream id");
        assert_eq!(decoded, source_ref);
        assert_eq!(stream_id.as_str(), "rpc_source:eth-mainnet:primary");
    }

    #[test]
    fn rpc_source_ref_rejects_reserved_characters() {
        let err = RpcSourceRef::new("eth:mainnet", "primary").expect_err("invalid network id");
        assert_eq!(
            err,
            RpcSourceRefError::InvalidComponent {
                name: "network_id",
                value: "eth:mainnet".to_string(),
            }
        );
    }

    #[test]
    fn rebuild_rpc_source_state_accumulates_quality_and_cooldown() {
        let source_ref = source_ref();
        let stream_id = source_ref.stream_id();
        let records = vec![
            StreamRecord {
                stream_id: stream_id.clone(),
                seq: 1,
                ts_millis: Some(100),
                kind: RECORD_KIND_SOURCE_OBSERVED.to_string(),
                payload: serde_json::json!({
                    "observed_at_ms": 100_u64,
                    "outcome": "failure",
                    "latency_ms": 12_u64,
                    "cooldown_until_ms": 150_u64,
                    "diagnostic_code": "rate_limited",
                }),
            },
            StreamRecord {
                stream_id: stream_id.clone(),
                seq: 2,
                ts_millis: Some(110),
                kind: RECORD_KIND_SOURCE_PROBED.to_string(),
                payload: serde_json::json!({
                    "probed_at_ms": 110_u64,
                    "probe_kind": "get_proof",
                    "outcome": "success",
                    "latency_ms": 4_u64,
                    "supports_get_proof": true,
                }),
            },
            StreamRecord {
                stream_id,
                seq: 3,
                ts_millis: Some(120),
                kind: RECORD_KIND_SOURCE_OBSERVED.to_string(),
                payload: serde_json::json!({
                    "observed_at_ms": 120_u64,
                    "outcome": "success",
                    "head_block_number": 22_000_123_u64,
                    "latency_ms": 9_u64,
                }),
            },
        ];

        let state = rebuild_rpc_source_state(source_ref, &records).expect("rebuild succeeds");
        assert_eq!(state.head_seq, 3);
        assert_eq!(state.last_recorded_at_ms, Some(120));
        assert_eq!(state.last_observed_at_ms, Some(120));
        assert_eq!(state.last_probed_at_ms, Some(110));
        assert_eq!(state.last_observed_head, Some(22_000_123));
        assert_eq!(state.last_latency_ms, Some(9));
        assert_eq!(state.last_probe_latency_ms, Some(4));
        assert_eq!(state.supports_get_proof, Some(true));
        assert_eq!(state.success_count, 2);
        assert_eq!(state.failure_count, 1);
        assert_eq!(state.consecutive_failures, 0);
        assert_eq!(state.cooldown_until_ms, None);
        assert_eq!(state.last_error_code, None);
    }

    #[test]
    fn rebuild_rpc_source_state_keeps_last_head_when_new_sample_has_none() {
        let source_ref = source_ref();
        let stream_id = source_ref.stream_id();
        let records = vec![
            StreamRecord {
                stream_id: stream_id.clone(),
                seq: 1,
                ts_millis: Some(100),
                kind: RECORD_KIND_SOURCE_OBSERVED.to_string(),
                payload: serde_json::json!({
                    "observed_at_ms": 100_u64,
                    "outcome": "success",
                    "head_block_number": 42_u64,
                }),
            },
            StreamRecord {
                stream_id,
                seq: 2,
                ts_millis: Some(110),
                kind: RECORD_KIND_SOURCE_OBSERVED.to_string(),
                payload: serde_json::json!({
                    "observed_at_ms": 110_u64,
                    "outcome": "failure",
                    "diagnostic_code": "timeout",
                }),
            },
        ];

        let state = rebuild_rpc_source_state(source_ref, &records).expect("rebuild succeeds");
        assert_eq!(state.last_observed_head, Some(42));
        assert_eq!(state.failure_count, 1);
        assert_eq!(state.last_error_code.as_deref(), Some("timeout"));
    }

    #[test]
    fn rebuild_rpc_source_state_rejects_unknown_kind() {
        let source_ref = source_ref();
        let err = rebuild_rpc_source_state(
            source_ref.clone(),
            &[StreamRecord {
                stream_id: source_ref.stream_id(),
                seq: 1,
                ts_millis: None,
                kind: "unexpected_kind".to_string(),
                payload: serde_json::json!({}),
            }],
        )
        .expect_err("unknown kind must fail");
        assert_eq!(
            err,
            RpcSourceProjectionError::UnsupportedRecordKind("unexpected_kind".to_string())
        );
    }

    #[test]
    fn rebuild_rpc_source_state_rejects_seq_gaps() {
        let source_ref = source_ref();
        let err = rebuild_rpc_source_state(
            source_ref.clone(),
            &[StreamRecord {
                stream_id: source_ref.stream_id(),
                seq: 2,
                ts_millis: None,
                kind: RECORD_KIND_SOURCE_OBSERVED.to_string(),
                payload: serde_json::json!({
                    "observed_at_ms": 100_u64,
                    "outcome": "success",
                }),
            }],
        )
        .expect_err("seq gap must fail");
        assert_eq!(
            err,
            RpcSourceProjectionError::NonContiguousSeq {
                expected: 1,
                actual: 2,
            }
        );
    }
}
