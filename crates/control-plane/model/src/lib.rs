#![allow(clippy::disallowed_methods)]
#![warn(missing_docs)]
//! Backend-neutral control-plane stream-family model.
//!
//! This crate owns the typed `rpc_source:*` and `source_pool:*` stream-family records,
//! references, non-secret catalog snapshots, and rebuildable projections used by the live
//! `rpc.control` transport and concrete control-plane storage adapters.
//!
//! # Examples
//!
//! ```
//! use mfm_control_plane_model::{RpcSourceRef, RpcSourceState};
//!
//! let source =
//!     RpcSourceRef::new("shared", "eth-mainnet", "primary").expect("valid source ref");
//! let state = RpcSourceState::new(source);
//! assert_eq!(state.head_seq, 0);
//! ```

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use mfm_machine::errors::{ErrorCategory, ErrorInfo, StorageError};
use mfm_machine::hashing::{artifact_id_for_json, CanonicalJsonError};
use mfm_machine::ids::ErrorCode;
use mfm_machine::stores::{validate_new_stream_record, NewStreamRecord, StreamId, StreamRecord};

/// Stream family used for source-quality and probe history.
pub const RPC_SOURCE_STREAM_FAMILY: &str = "rpc_source";
/// Stream family used for source-pool membership and ranking snapshots.
pub const SOURCE_POOL_STREAM_FAMILY: &str = "source_pool";

const RECORD_KIND_SOURCE_OBSERVED: &str = "source_observed";
const RECORD_KIND_SOURCE_PROBED: &str = "source_probed";
const RECORD_KIND_POOL_CATALOG_DECLARED: &str = "pool_catalog_declared";
const RECORD_KIND_POOL_MEMBERSHIP_DECLARED: &str = "pool_membership_declared";
const RECORD_KIND_POOL_RANKED: &str = "pool_ranked";

/// Canonical fingerprint schema version for source-pool catalog snapshots.
pub const SOURCE_POOL_CATALOG_SCHEMA_VERSION: u64 = 1;

fn storage_info(code: &'static str, message: impl Into<String>) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode::must_new(code),
        category: ErrorCategory::Storage,
        retryable: false,
        message: message.into(),
        details: None,
    }
}

fn storage_other(code: &'static str, message: impl Into<String>) -> StorageError {
    StorageError::Other(storage_info(code, message))
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

#[derive(Clone, Copy, Debug)]
struct StreamIdTripleParts<'a> {
    control_scope: &'a str,
    network_id: &'a str,
    terminal: &'a str,
}

impl<'a> StreamIdTripleParts<'a> {
    fn parse(stream_id_key: &'a StreamId) -> Option<Self> {
        Self::parse_stream_id_key(stream_id_key.key())
    }

    fn parse_stream_id_key(stream_id_key: &'a str) -> Option<Self> {
        let mut parts = stream_id_key.split(':');
        let first = parts.next()?;
        let second = parts.next()?;
        let third = parts.next()?;
        if parts.next().is_some() || first.is_empty() || second.is_empty() || third.is_empty() {
            return None;
        }

        Some(Self {
            control_scope: first,
            network_id: second,
            terminal: third,
        })
    }
}

fn parse_3_part_stream_key(stream_id: &StreamId) -> Option<StreamIdTripleParts<'_>> {
    StreamIdTripleParts::parse(stream_id)
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
    /// The stream id did not follow the `rpc_source:<control_scope>:<network_id>:<source_id>` shape.
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
                "rpc source stream id must follow `rpc_source:<control_scope>:<network_id>:<source_id>` (got `{value}`)"
            ),
        }
    }
}

impl std::error::Error for RpcSourceRefError {}

/// Stable identity for one control-plane-managed RPC source.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RpcSourceRef {
    control_scope: String,
    network_id: String,
    source_id: String,
}

impl RpcSourceRef {
    /// Creates a validated source reference.
    pub fn new(
        control_scope: impl Into<String>,
        network_id: impl Into<String>,
        source_id: impl Into<String>,
    ) -> Result<Self, RpcSourceRefError> {
        Ok(Self {
            control_scope: validate_component("control_scope", control_scope)?,
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
        let Some(parts) = parse_3_part_stream_key(stream_id) else {
            return Err(RpcSourceRefError::InvalidStreamId(
                stream_id.as_str().to_string(),
            ));
        };
        Self::new(parts.control_scope, parts.network_id, parts.terminal)
    }

    /// Returns the control-scope identifier.
    pub fn control_scope(&self) -> &str {
        &self.control_scope
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
            "{RPC_SOURCE_STREAM_FAMILY}:{}:{}:{}",
            self.control_scope, self.network_id, self.source_id
        ))
    }
}

impl fmt::Display for RpcSourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.control_scope, self.network_id, self.source_id
        )
    }
}

/// Validation error for [`SourcePoolRef`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourcePoolRefError {
    /// One of the id components was empty or contained a reserved character.
    InvalidComponent {
        /// The invalid field name.
        name: &'static str,
        /// The invalid value.
        value: String,
    },
    /// The stream id did not follow the `source_pool:<control_scope>:<network_id>:<pool_kind>` shape.
    InvalidStreamId(String),
}

impl fmt::Display for SourcePoolRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourcePoolRefError::InvalidComponent { name, value } => {
                write!(f, "{name} must be non-empty and must not contain whitespace, control characters, or ':' (got `{value}`)")
            }
            SourcePoolRefError::InvalidStreamId(value) => write!(
                f,
                "source pool stream id must follow `source_pool:<control_scope>:<network_id>:<pool_kind>` (got `{value}`)"
            ),
        }
    }
}

impl std::error::Error for SourcePoolRefError {}

/// Stable identity for one control-plane-managed source pool.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourcePoolRef {
    control_scope: String,
    network_id: String,
    pool_kind: String,
}

impl SourcePoolRef {
    /// Creates a validated source-pool reference.
    pub fn new(
        control_scope: impl Into<String>,
        network_id: impl Into<String>,
        pool_kind: impl Into<String>,
    ) -> Result<Self, SourcePoolRefError> {
        Ok(Self {
            control_scope: validate_component("control_scope", control_scope).map_err(|err| {
                match err {
                    RpcSourceRefError::InvalidComponent { name, value } => {
                        SourcePoolRefError::InvalidComponent { name, value }
                    }
                    RpcSourceRefError::InvalidStreamId(value) => {
                        SourcePoolRefError::InvalidStreamId(value)
                    }
                }
            })?,
            network_id: validate_component("network_id", network_id).map_err(|err| match err {
                RpcSourceRefError::InvalidComponent { name, value } => {
                    SourcePoolRefError::InvalidComponent { name, value }
                }
                RpcSourceRefError::InvalidStreamId(value) => {
                    SourcePoolRefError::InvalidStreamId(value)
                }
            })?,
            pool_kind: validate_component("pool_kind", pool_kind).map_err(|err| match err {
                RpcSourceRefError::InvalidComponent { name, value } => {
                    SourcePoolRefError::InvalidComponent { name, value }
                }
                RpcSourceRefError::InvalidStreamId(value) => {
                    SourcePoolRefError::InvalidStreamId(value)
                }
            })?,
        })
    }

    /// Parses a typed source-pool reference from a stream id.
    pub fn from_stream_id(stream_id: &StreamId) -> Result<Self, SourcePoolRefError> {
        if stream_id.family() != SOURCE_POOL_STREAM_FAMILY {
            return Err(SourcePoolRefError::InvalidStreamId(
                stream_id.as_str().to_string(),
            ));
        }
        let Some(parts) = parse_3_part_stream_key(stream_id) else {
            return Err(SourcePoolRefError::InvalidStreamId(
                stream_id.as_str().to_string(),
            ));
        };
        Self::new(parts.control_scope, parts.network_id, parts.terminal)
    }

    /// Returns the control-scope identifier.
    pub fn control_scope(&self) -> &str {
        &self.control_scope
    }

    /// Returns the network identifier.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the pool kind identifier.
    pub fn pool_kind(&self) -> &str {
        &self.pool_kind
    }

    /// Returns the canonical stream id for this pool.
    pub fn stream_id(&self) -> StreamId {
        StreamId::must_new(format!(
            "{SOURCE_POOL_STREAM_FAMILY}:{}:{}:{}",
            self.control_scope, self.network_id, self.pool_kind
        ))
    }
}

impl fmt::Display for SourcePoolRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}",
            self.control_scope, self.network_id, self.pool_kind
        )
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

    /// Encodes the typed record into a generic append-only stream record payload.
    pub fn to_new_stream_record(&self) -> Result<NewStreamRecord, StorageError> {
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

        let record = NewStreamRecord {
            ts_millis: Some(self.recorded_at_ms()),
            kind: self.kind().to_string(),
            payload,
        };
        validate_new_stream_record(&record)?;
        Ok(record)
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

    /// Applies one typed stream record to this projection at `seq`.
    pub fn apply_record(&mut self, seq: u64, record: &RpcSourceRecord) {
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

/// Validates a source-id snapshot used by `source_pool:*` membership and ranking records.
pub fn validate_source_id_snapshot(source_ids: &[String]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for source_id in source_ids {
        validate_component("source_id", source_id.clone()).map_err(|err| match err {
            RpcSourceRefError::InvalidComponent { value, .. } => {
                format!("invalid source_id `{value}` in source snapshot")
            }
            RpcSourceRefError::InvalidStreamId(value) => {
                format!("invalid source_id `{value}` in source snapshot")
            }
        })?;
        if !seen.insert(source_id.clone()) {
            return Err(format!(
                "duplicate source_id `{source_id}` in source snapshot"
            ));
        }
    }
    Ok(())
}

/// Durable `pool_membership_declared` payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePoolMembershipDeclaredRecord {
    /// Snapshot timestamp in milliseconds since the Unix epoch.
    pub declared_at_ms: u64,
    /// Full desired pool membership snapshot, as stable source ids.
    pub member_source_ids: Vec<String>,
}

/// Durable `pool_ranked` payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePoolRankedRecord {
    /// Ranking timestamp in milliseconds since the Unix epoch.
    pub ranked_at_ms: u64,
    /// Full desired ranking snapshot, as stable source ids.
    pub ranked_source_ids: Vec<String>,
}

/// Non-secret routing summary for one source inside a declared pool catalog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePoolCatalogSource {
    /// Stable source identifier.
    pub id: String,
    /// Stable source kind label.
    pub kind: String,
    /// Whether the source requires a `eth_getProof` probe for healthy selection.
    pub require_get_proof_probe: bool,
}

/// Canonical non-secret routing catalog snapshot for one scoped source pool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePoolCatalogSnapshot {
    /// Fingerprint schema version.
    pub schema_version: u64,
    /// Stable control-plane scope.
    pub control_scope: String,
    /// Stable network identifier.
    pub network_id: String,
    /// Stable pool kind.
    pub pool_kind: String,
    /// Effective candidate sources for the pool, sorted by source id.
    pub sources: Vec<SourcePoolCatalogSource>,
    /// Effective preferred order projected onto the candidate-source ids.
    pub preferred_source_ids: Vec<String>,
}

impl SourcePoolCatalogSnapshot {
    /// Validates catalog-shape invariants used for storage and hashing.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SOURCE_POOL_CATALOG_SCHEMA_VERSION {
            return Err(format!(
                "unsupported source_pool catalog schema version `{}`",
                self.schema_version
            ));
        }
        validate_component("control_scope", self.control_scope.clone()).map_err(
            |err| match err {
                RpcSourceRefError::InvalidComponent { value, .. } => {
                    format!("invalid control_scope `{value}` in catalog snapshot")
                }
                RpcSourceRefError::InvalidStreamId(value) => {
                    format!("invalid control_scope `{value}` in catalog snapshot")
                }
            },
        )?;
        validate_component("network_id", self.network_id.clone()).map_err(|err| match err {
            RpcSourceRefError::InvalidComponent { value, .. } => {
                format!("invalid network_id `{value}` in catalog snapshot")
            }
            RpcSourceRefError::InvalidStreamId(value) => {
                format!("invalid network_id `{value}` in catalog snapshot")
            }
        })?;
        validate_component("pool_kind", self.pool_kind.clone()).map_err(|err| match err {
            RpcSourceRefError::InvalidComponent { value, .. } => {
                format!("invalid pool_kind `{value}` in catalog snapshot")
            }
            RpcSourceRefError::InvalidStreamId(value) => {
                format!("invalid pool_kind `{value}` in catalog snapshot")
            }
        })?;

        let mut seen = BTreeSet::new();
        for source in &self.sources {
            validate_component("source_id", source.id.clone()).map_err(|err| match err {
                RpcSourceRefError::InvalidComponent { value, .. } => {
                    format!("invalid source_id `{value}` in catalog snapshot")
                }
                RpcSourceRefError::InvalidStreamId(value) => {
                    format!("invalid source_id `{value}` in catalog snapshot")
                }
            })?;
            if source.kind.trim().is_empty() {
                return Err(format!(
                    "source `{}` in catalog snapshot must declare kind",
                    source.id
                ));
            }
            if !seen.insert(source.id.clone()) {
                return Err(format!(
                    "duplicate source_id `{}` in catalog snapshot",
                    source.id
                ));
            }
        }

        let source_ids = self
            .sources
            .iter()
            .map(|source| source.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut preferred_seen = BTreeSet::new();
        for source_id in &self.preferred_source_ids {
            validate_component("source_id", source_id.clone()).map_err(|err| match err {
                RpcSourceRefError::InvalidComponent { value, .. } => {
                    format!("invalid preferred source_id `{value}` in catalog snapshot")
                }
                RpcSourceRefError::InvalidStreamId(value) => {
                    format!("invalid preferred source_id `{value}` in catalog snapshot")
                }
            })?;
            if !source_ids.contains(source_id.as_str()) {
                return Err(format!(
                    "preferred source_id `{source_id}` not present in catalog snapshot"
                ));
            }
            if !preferred_seen.insert(source_id.clone()) {
                return Err(format!(
                    "duplicate preferred source_id `{source_id}` in catalog snapshot"
                ));
            }
        }

        Ok(())
    }

    /// Returns the canonical SHA-256 fingerprint for this catalog snapshot.
    pub fn fingerprint(&self) -> Result<String, CanonicalJsonError> {
        let value = serde_json::to_value(self)
            .expect("SourcePoolCatalogSnapshot should always serialize to JSON");
        Ok(artifact_id_for_json(&value)?.into_string())
    }
}

/// Durable `pool_catalog_declared` payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePoolCatalogDeclaredRecord {
    /// Declaration timestamp in milliseconds since the Unix epoch.
    pub declared_at_ms: u64,
    /// Canonical fingerprint for the declared non-secret routing catalog.
    pub catalog_fingerprint: String,
    /// Canonical non-secret routing catalog snapshot.
    pub catalog_snapshot: SourcePoolCatalogSnapshot,
}

/// Typed `source_pool:*` stream-family record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourcePoolRecord {
    /// `pool_catalog_declared`
    CatalogDeclared(SourcePoolCatalogDeclaredRecord),
    /// `pool_membership_declared`
    MembershipDeclared(SourcePoolMembershipDeclaredRecord),
    /// `pool_ranked`
    Ranked(SourcePoolRankedRecord),
}

impl SourcePoolRecord {
    fn kind(&self) -> &'static str {
        match self {
            SourcePoolRecord::CatalogDeclared(_) => RECORD_KIND_POOL_CATALOG_DECLARED,
            SourcePoolRecord::MembershipDeclared(_) => RECORD_KIND_POOL_MEMBERSHIP_DECLARED,
            SourcePoolRecord::Ranked(_) => RECORD_KIND_POOL_RANKED,
        }
    }

    fn recorded_at_ms(&self) -> u64 {
        match self {
            SourcePoolRecord::CatalogDeclared(record) => record.declared_at_ms,
            SourcePoolRecord::MembershipDeclared(record) => record.declared_at_ms,
            SourcePoolRecord::Ranked(record) => record.ranked_at_ms,
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            SourcePoolRecord::CatalogDeclared(record) => {
                record.catalog_snapshot.validate()?;
                let actual_fingerprint = record.catalog_snapshot.fingerprint().map_err(|err| {
                    format!("catalog snapshot was not canonical-json-hashable: {err}")
                })?;
                if record.catalog_fingerprint != actual_fingerprint {
                    return Err(format!(
                        "catalog fingerprint `{}` did not match canonical snapshot fingerprint `{actual_fingerprint}`",
                        record.catalog_fingerprint
                    ));
                }
                Ok(())
            }
            SourcePoolRecord::MembershipDeclared(record) => {
                validate_source_id_snapshot(&record.member_source_ids)
            }
            SourcePoolRecord::Ranked(record) => {
                validate_source_id_snapshot(&record.ranked_source_ids)
            }
        }
    }

    /// Encodes the typed record into a generic append-only stream record payload.
    pub fn to_new_stream_record(&self) -> Result<NewStreamRecord, StorageError> {
        self.validate().map_err(|message| {
            storage_other(
                "control_plane_record_invalid",
                format!("invalid source_pool record payload: {message}"),
            )
        })?;

        let payload = match self {
            SourcePoolRecord::CatalogDeclared(record) => serde_json::to_value(record),
            SourcePoolRecord::MembershipDeclared(record) => serde_json::to_value(record),
            SourcePoolRecord::Ranked(record) => serde_json::to_value(record),
        }
        .map_err(|err| {
            storage_other(
                "control_plane_record_encode_failed",
                format!("failed to encode source_pool record payload: {err}"),
            )
        })?;

        let record = NewStreamRecord {
            ts_millis: Some(self.recorded_at_ms()),
            kind: self.kind().to_string(),
            payload,
        };
        validate_new_stream_record(&record)?;
        Ok(record)
    }

    fn from_stream_record(record: &StreamRecord) -> Result<Self, SourcePoolProjectionError> {
        match record.kind.as_str() {
            RECORD_KIND_POOL_CATALOG_DECLARED => serde_json::from_value(record.payload.clone())
                .map(SourcePoolRecord::CatalogDeclared)
                .map_err(|err| SourcePoolProjectionError::InvalidPayload {
                    kind: record.kind.clone(),
                    message: err.to_string(),
                }),
            RECORD_KIND_POOL_MEMBERSHIP_DECLARED => serde_json::from_value(record.payload.clone())
                .map(SourcePoolRecord::MembershipDeclared)
                .map_err(|err| SourcePoolProjectionError::InvalidPayload {
                    kind: record.kind.clone(),
                    message: err.to_string(),
                }),
            RECORD_KIND_POOL_RANKED => serde_json::from_value(record.payload.clone())
                .map(SourcePoolRecord::Ranked)
                .map_err(|err| SourcePoolProjectionError::InvalidPayload {
                    kind: record.kind.clone(),
                    message: err.to_string(),
                }),
            other => Err(SourcePoolProjectionError::UnsupportedRecordKind(
                other.to_string(),
            )),
        }
    }
}

/// Rebuildable projection for one `source_pool:*` stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePoolState {
    /// Stable source-pool identity.
    pub pool_ref: SourcePoolRef,
    /// Canonical stream id backing this projection.
    pub stream_id: StreamId,
    /// Last projected stream sequence.
    pub head_seq: u64,
    /// Timestamp of the most recent record applied to this projection.
    pub last_recorded_at_ms: Option<u64>,
    /// Timestamp of the most recent `pool_membership_declared` record.
    pub last_membership_declared_at_ms: Option<u64>,
    /// Timestamp of the most recent `pool_ranked` record.
    pub last_ranked_at_ms: Option<u64>,
    /// Timestamp of the most recent `pool_catalog_declared` record.
    pub last_catalog_declared_at_ms: Option<u64>,
    /// Fingerprint for the declared non-secret routing catalog.
    pub catalog_fingerprint: Option<String>,
    /// Declared non-secret routing catalog snapshot.
    pub catalog_snapshot: Option<SourcePoolCatalogSnapshot>,
    /// Latest membership snapshot for this pool.
    pub member_source_ids: Vec<String>,
    /// Latest raw ranking snapshot for this pool.
    pub ranked_source_ids: Vec<String>,
}

impl SourcePoolState {
    /// Creates an empty projection for `pool_ref`.
    pub fn new(pool_ref: SourcePoolRef) -> Self {
        let stream_id = pool_ref.stream_id();
        Self {
            pool_ref,
            stream_id,
            head_seq: 0,
            last_recorded_at_ms: None,
            last_membership_declared_at_ms: None,
            last_ranked_at_ms: None,
            last_catalog_declared_at_ms: None,
            catalog_fingerprint: None,
            catalog_snapshot: None,
            member_source_ids: Vec::new(),
            ranked_source_ids: Vec::new(),
        }
    }

    /// Applies one typed stream record to this projection at `seq`.
    pub fn apply_record(
        &mut self,
        seq: u64,
        record: &SourcePoolRecord,
    ) -> Result<(), SourcePoolProjectionError> {
        record.validate().map_err(|message| match record {
            SourcePoolRecord::CatalogDeclared(_) => {
                SourcePoolProjectionError::InvalidCatalogSnapshot(message)
            }
            SourcePoolRecord::MembershipDeclared(_) | SourcePoolRecord::Ranked(_) => {
                SourcePoolProjectionError::InvalidSourceSnapshot(message)
            }
        })?;
        self.head_seq = seq;
        self.last_recorded_at_ms = Some(record.recorded_at_ms());

        match record {
            SourcePoolRecord::CatalogDeclared(declared) => {
                if declared.catalog_snapshot.control_scope != self.pool_ref.control_scope()
                    || declared.catalog_snapshot.network_id != self.pool_ref.network_id()
                    || declared.catalog_snapshot.pool_kind != self.pool_ref.pool_kind()
                {
                    return Err(SourcePoolProjectionError::InvalidCatalogSnapshot(format!(
                        "catalog snapshot identity `{}/{}/{}` did not match pool `{}`",
                        declared.catalog_snapshot.control_scope,
                        declared.catalog_snapshot.network_id,
                        declared.catalog_snapshot.pool_kind,
                        self.pool_ref
                    )));
                }
                self.last_catalog_declared_at_ms = Some(declared.declared_at_ms);
                self.catalog_fingerprint = Some(declared.catalog_fingerprint.clone());
                self.catalog_snapshot = Some(declared.catalog_snapshot.clone());
            }
            SourcePoolRecord::MembershipDeclared(declared) => {
                if let Some(snapshot) = &self.catalog_snapshot {
                    let declared_source_ids = snapshot
                        .sources
                        .iter()
                        .map(|source| source.id.as_str())
                        .collect::<BTreeSet<_>>();
                    for source_id in &declared.member_source_ids {
                        if !declared_source_ids.contains(source_id.as_str()) {
                            return Err(SourcePoolProjectionError::InvalidCatalogSnapshot(
                                format!(
                                    "membership source_id `{source_id}` was not declared in the pool catalog"
                                ),
                            ));
                        }
                    }
                }
                self.last_membership_declared_at_ms = Some(declared.declared_at_ms);
                self.member_source_ids = declared.member_source_ids.clone();
            }
            SourcePoolRecord::Ranked(ranked) => {
                if let Some(snapshot) = &self.catalog_snapshot {
                    let declared_source_ids = snapshot
                        .sources
                        .iter()
                        .map(|source| source.id.as_str())
                        .collect::<BTreeSet<_>>();
                    for source_id in &ranked.ranked_source_ids {
                        if !declared_source_ids.contains(source_id.as_str()) {
                            return Err(SourcePoolProjectionError::InvalidCatalogSnapshot(
                                format!(
                                    "ranked source_id `{source_id}` was not declared in the pool catalog"
                                ),
                            ));
                        }
                    }
                }
                self.last_ranked_at_ms = Some(ranked.ranked_at_ms);
                self.ranked_source_ids = ranked.ranked_source_ids.clone();
            }
        }

        Ok(())
    }

    /// Returns the effective ordered source ids for this pool.
    ///
    /// Ranking is filtered to the current membership snapshot. Any current member that is absent
    /// from the latest ranking snapshot is appended in membership order.
    pub fn ordered_source_ids(&self) -> Vec<String> {
        let member_ids = self
            .member_source_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut ordered = Vec::with_capacity(self.member_source_ids.len());
        let mut seen = BTreeSet::new();

        for source_id in &self.ranked_source_ids {
            if member_ids.contains(source_id) && seen.insert(source_id.clone()) {
                ordered.push(source_id.clone());
            }
        }

        for source_id in &self.member_source_ids {
            if seen.insert(source_id.clone()) {
                ordered.push(source_id.clone());
            }
        }

        ordered
    }

    /// Returns the effective ordered source refs for this pool.
    pub fn ordered_source_refs(&self) -> Result<Vec<RpcSourceRef>, RpcSourceRefError> {
        self.ordered_source_ids()
            .into_iter()
            .map(|source_id| {
                RpcSourceRef::new(
                    self.pool_ref.control_scope(),
                    self.pool_ref.network_id(),
                    source_id,
                )
            })
            .collect()
    }
}

/// Rebuild error for `source_pool:*` projections.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourcePoolProjectionError {
    /// The stream id did not belong to the expected `SourcePoolRef`.
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
    /// The stream id did not parse as `source_pool:<control_scope>:<network_id>:<pool_kind>`.
    InvalidStreamId(String),
    /// The record kind was not one of the v1 `source_pool:*` kinds.
    UnsupportedRecordKind(String),
    /// The record payload did not decode into the typed v1 shape.
    InvalidPayload {
        /// Record kind being decoded.
        kind: String,
        /// Serde error message.
        message: String,
    },
    /// The catalog snapshot carried invalid non-secret routing metadata.
    InvalidCatalogSnapshot(String),
    /// The membership or ranking snapshot carried invalid source ids.
    InvalidSourceSnapshot(String),
}

impl fmt::Display for SourcePoolProjectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourcePoolProjectionError::StreamIdMismatch { expected, actual } => write!(
                f,
                "source_pool rebuild expected stream `{expected}` but found `{actual}`"
            ),
            SourcePoolProjectionError::NonContiguousSeq { expected, actual } => write!(
                f,
                "source_pool rebuild expected seq {expected} but found {actual}"
            ),
            SourcePoolProjectionError::InvalidStreamId(value) => {
                write!(f, "invalid source_pool stream id `{value}`")
            }
            SourcePoolProjectionError::UnsupportedRecordKind(kind) => {
                write!(f, "unsupported source_pool record kind `{kind}`")
            }
            SourcePoolProjectionError::InvalidPayload { kind, message } => {
                write!(f, "invalid `{kind}` payload: {message}")
            }
            SourcePoolProjectionError::InvalidCatalogSnapshot(message) => {
                write!(f, "invalid catalog snapshot: {message}")
            }
            SourcePoolProjectionError::InvalidSourceSnapshot(message) => {
                write!(f, "invalid source snapshot: {message}")
            }
        }
    }
}

impl std::error::Error for SourcePoolProjectionError {}

/// Rebuilds one `source_pool:*` projection from append-only stream records.
pub fn rebuild_source_pool_state(
    pool_ref: SourcePoolRef,
    records: &[StreamRecord],
) -> Result<SourcePoolState, SourcePoolProjectionError> {
    let expected_stream_id = pool_ref.stream_id();
    let mut expected_seq = 1_u64;
    let mut state = SourcePoolState::new(pool_ref);

    for record in records {
        if record.stream_id != expected_stream_id {
            return Err(SourcePoolProjectionError::StreamIdMismatch {
                expected: expected_stream_id.clone(),
                actual: record.stream_id.clone(),
            });
        }
        if record.seq != expected_seq {
            return Err(SourcePoolProjectionError::NonContiguousSeq {
                expected: expected_seq,
                actual: record.seq,
            });
        }
        let typed = SourcePoolRecord::from_stream_record(record)?;
        state.apply_record(record.seq, &typed)?;
        expected_seq = expected_seq.saturating_add(1);
    }

    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_ref() -> RpcSourceRef {
        RpcSourceRef::new("shared", "eth-mainnet", "primary").expect("valid source ref")
    }

    fn pool_ref() -> SourcePoolRef {
        SourcePoolRef::new("shared", "eth-mainnet", "default").expect("valid source pool ref")
    }

    fn catalog_snapshot() -> SourcePoolCatalogSnapshot {
        SourcePoolCatalogSnapshot {
            schema_version: SOURCE_POOL_CATALOG_SCHEMA_VERSION,
            control_scope: "shared".to_string(),
            network_id: "eth-mainnet".to_string(),
            pool_kind: "default".to_string(),
            sources: vec![
                SourcePoolCatalogSource {
                    id: "archive_local".to_string(),
                    kind: "local".to_string(),
                    require_get_proof_probe: false,
                },
                SourcePoolCatalogSource {
                    id: "fallback_local".to_string(),
                    kind: "remote_public".to_string(),
                    require_get_proof_probe: false,
                },
                SourcePoolCatalogSource {
                    id: "local_rpc".to_string(),
                    kind: "local".to_string(),
                    require_get_proof_probe: false,
                },
                SourcePoolCatalogSource {
                    id: "reth_local".to_string(),
                    kind: "local".to_string(),
                    require_get_proof_probe: false,
                },
                SourcePoolCatalogSource {
                    id: "unknown_local".to_string(),
                    kind: "remote_public".to_string(),
                    require_get_proof_probe: false,
                },
            ],
            preferred_source_ids: vec!["reth_local".to_string(), "local_rpc".to_string()],
        }
    }

    #[test]
    fn rpc_source_ref_round_trips_through_stream_id() {
        let source_ref = source_ref();
        let stream_id = source_ref.stream_id();
        let decoded = RpcSourceRef::from_stream_id(&stream_id).expect("decode stream id");
        assert_eq!(decoded, source_ref);
        assert_eq!(stream_id.as_str(), "rpc_source:shared:eth-mainnet:primary");
    }

    #[test]
    fn rpc_source_ref_rejects_reserved_characters() {
        let err =
            RpcSourceRef::new("shared", "eth:mainnet", "primary").expect_err("invalid network id");
        assert_eq!(
            err,
            RpcSourceRefError::InvalidComponent {
                name: "network_id",
                value: "eth:mainnet".to_string(),
            }
        );
    }

    #[test]
    fn rpc_source_ref_rejects_malformed_stream_key() {
        let malformed = StreamId::new("rpc_source:shared:eth-mainnet:primary:extra")
            .expect("stream id constructor accepts literal key");
        let err = RpcSourceRef::from_stream_id(&malformed).expect_err("malformed child path");
        assert!(matches!(
            err,
            RpcSourceRefError::InvalidStreamId(value) if value == malformed.as_str()
        ));

        let malformed = StreamId::new("rpc_source:shared::primary")
            .expect("stream id constructor accepts empty segment in stream key");
        let err = RpcSourceRef::from_stream_id(&malformed).expect_err("empty network id");
        assert!(matches!(
            err,
            RpcSourceRefError::InvalidStreamId(value) if value == malformed.as_str()
        ));
    }

    #[test]
    fn source_pool_ref_round_trips_through_stream_id() {
        let pool_ref = pool_ref();
        let stream_id = pool_ref.stream_id();
        let decoded = SourcePoolRef::from_stream_id(&stream_id).expect("decode stream id");
        assert_eq!(decoded, pool_ref);
        assert_eq!(stream_id.as_str(), "source_pool:shared:eth-mainnet:default");
    }

    #[test]
    fn source_pool_ref_rejects_reserved_characters() {
        let err = SourcePoolRef::new("shared", "eth-mainnet", "default pool")
            .expect_err("invalid pool kind");
        assert_eq!(
            err,
            SourcePoolRefError::InvalidComponent {
                name: "pool_kind",
                value: "default pool".to_string(),
            }
        );
    }

    #[test]
    fn source_pool_ref_rejects_malformed_stream_key() {
        let malformed = StreamId::new("source_pool:shared:eth-mainnet:default:extra")
            .expect("stream id constructor accepts literal key");
        let err = SourcePoolRef::from_stream_id(&malformed).expect_err("malformed source kind");
        assert!(matches!(
            err,
            SourcePoolRefError::InvalidStreamId(value) if value == malformed.as_str()
        ));

        let malformed = StreamId::new("source_pool:shared::default")
            .expect("stream id constructor accepts empty segment in stream key");
        let err = SourcePoolRef::from_stream_id(&malformed).expect_err("empty network id");
        assert!(matches!(
            err,
            SourcePoolRefError::InvalidStreamId(value) if value == malformed.as_str()
        ));
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

    #[test]
    fn rebuild_source_pool_state_applies_membership_and_ranking() {
        let pool_ref = pool_ref();
        let stream_id = pool_ref.stream_id();
        let catalog_snapshot = catalog_snapshot();
        let catalog_fingerprint = catalog_snapshot.fingerprint().expect("catalog fingerprint");
        let records = vec![
            StreamRecord {
                stream_id: stream_id.clone(),
                seq: 1,
                ts_millis: Some(100),
                kind: RECORD_KIND_POOL_CATALOG_DECLARED.to_string(),
                payload: serde_json::json!({
                    "declared_at_ms": 100_u64,
                    "catalog_fingerprint": catalog_fingerprint,
                    "catalog_snapshot": catalog_snapshot,
                }),
            },
            StreamRecord {
                stream_id: stream_id.clone(),
                seq: 2,
                ts_millis: Some(100),
                kind: RECORD_KIND_POOL_MEMBERSHIP_DECLARED.to_string(),
                payload: serde_json::json!({
                    "declared_at_ms": 100_u64,
                    "member_source_ids": ["reth_local", "local_rpc", "archive_local"],
                }),
            },
            StreamRecord {
                stream_id: stream_id.clone(),
                seq: 3,
                ts_millis: Some(110),
                kind: RECORD_KIND_POOL_RANKED.to_string(),
                payload: serde_json::json!({
                    "ranked_at_ms": 110_u64,
                    "ranked_source_ids": ["local_rpc", "reth_local"],
                }),
            },
            StreamRecord {
                stream_id,
                seq: 4,
                ts_millis: Some(120),
                kind: RECORD_KIND_POOL_MEMBERSHIP_DECLARED.to_string(),
                payload: serde_json::json!({
                    "declared_at_ms": 120_u64,
                    "member_source_ids": ["reth_local", "local_rpc", "archive_local"],
                }),
            },
        ];

        let state = rebuild_source_pool_state(pool_ref, &records).expect("rebuild succeeds");
        assert_eq!(state.head_seq, 4);
        assert_eq!(state.last_recorded_at_ms, Some(120));
        assert_eq!(state.last_catalog_declared_at_ms, Some(100));
        assert_eq!(state.last_membership_declared_at_ms, Some(120));
        assert_eq!(state.last_ranked_at_ms, Some(110));
        assert_eq!(
            state.catalog_fingerprint.as_deref(),
            Some(catalog_fingerprint.as_str())
        );
        assert!(state.catalog_snapshot.is_some());
        assert_eq!(
            state.member_source_ids,
            vec![
                "reth_local".to_string(),
                "local_rpc".to_string(),
                "archive_local".to_string()
            ]
        );
        assert_eq!(
            state.ranked_source_ids,
            vec!["local_rpc".to_string(), "reth_local".to_string()]
        );
        assert_eq!(
            state.ordered_source_ids(),
            vec![
                "local_rpc".to_string(),
                "reth_local".to_string(),
                "archive_local".to_string()
            ]
        );
        assert_eq!(
            state
                .ordered_source_refs()
                .expect("ordered source refs should be valid"),
            vec![
                RpcSourceRef::new("shared", "eth-mainnet", "local_rpc").expect("valid"),
                RpcSourceRef::new("shared", "eth-mainnet", "reth_local").expect("valid"),
                RpcSourceRef::new("shared", "eth-mainnet", "archive_local").expect("valid"),
            ]
        );
    }

    #[test]
    fn rebuild_source_pool_state_rejects_catalog_fingerprint_mismatch() {
        let pool_ref = pool_ref();
        let snapshot = catalog_snapshot();
        let expected_fingerprint = snapshot.fingerprint().expect("catalog fingerprint");
        let err = rebuild_source_pool_state(
            pool_ref.clone(),
            &[StreamRecord {
                stream_id: pool_ref.stream_id(),
                seq: 1,
                ts_millis: Some(100),
                kind: RECORD_KIND_POOL_CATALOG_DECLARED.to_string(),
                payload: serde_json::json!({
                    "declared_at_ms": 100_u64,
                    "catalog_fingerprint": "bad-fingerprint",
                    "catalog_snapshot": snapshot,
                }),
            }],
        )
        .expect_err("bad catalog fingerprint must fail");
        assert_eq!(
            err,
            SourcePoolProjectionError::InvalidCatalogSnapshot(
                format!(
                    "catalog fingerprint `bad-fingerprint` did not match canonical snapshot fingerprint `{expected_fingerprint}`"
                )
            )
        );
    }

    #[test]
    fn rebuild_source_pool_state_rejects_membership_outside_declared_catalog() {
        let pool_ref = pool_ref();
        let snapshot = catalog_snapshot();
        let fingerprint = snapshot.fingerprint().expect("catalog fingerprint");
        let err = rebuild_source_pool_state(
            pool_ref.clone(),
            &[
                StreamRecord {
                    stream_id: pool_ref.stream_id(),
                    seq: 1,
                    ts_millis: Some(100),
                    kind: RECORD_KIND_POOL_CATALOG_DECLARED.to_string(),
                    payload: serde_json::json!({
                        "declared_at_ms": 100_u64,
                        "catalog_fingerprint": fingerprint,
                        "catalog_snapshot": snapshot,
                    }),
                },
                StreamRecord {
                    stream_id: pool_ref.stream_id(),
                    seq: 2,
                    ts_millis: Some(110),
                    kind: RECORD_KIND_POOL_MEMBERSHIP_DECLARED.to_string(),
                    payload: serde_json::json!({
                        "declared_at_ms": 110_u64,
                        "member_source_ids": ["reth_local", "rogue_local"],
                    }),
                },
            ],
        )
        .expect_err("membership outside declared catalog must fail");
        assert_eq!(
            err,
            SourcePoolProjectionError::InvalidCatalogSnapshot(
                "membership source_id `rogue_local` was not declared in the pool catalog"
                    .to_string()
            )
        );
    }

    #[test]
    fn rebuild_source_pool_state_rejects_duplicate_source_ids() {
        let pool_ref = pool_ref();
        let err = rebuild_source_pool_state(
            pool_ref.clone(),
            &[StreamRecord {
                stream_id: pool_ref.stream_id(),
                seq: 1,
                ts_millis: Some(100),
                kind: RECORD_KIND_POOL_MEMBERSHIP_DECLARED.to_string(),
                payload: serde_json::json!({
                    "declared_at_ms": 100_u64,
                    "member_source_ids": ["reth_local", "reth_local"],
                }),
            }],
        )
        .expect_err("duplicate source ids must fail");
        assert_eq!(
            err,
            SourcePoolProjectionError::InvalidSourceSnapshot(
                "duplicate source_id `reth_local` in source snapshot".to_string()
            )
        );
    }

    #[test]
    fn rebuild_source_pool_state_rejects_unknown_kind() {
        let pool_ref = pool_ref();
        let err = rebuild_source_pool_state(
            pool_ref.clone(),
            &[StreamRecord {
                stream_id: pool_ref.stream_id(),
                seq: 1,
                ts_millis: None,
                kind: "unexpected_kind".to_string(),
                payload: serde_json::json!({}),
            }],
        )
        .expect_err("unknown kind must fail");
        assert_eq!(
            err,
            SourcePoolProjectionError::UnsupportedRecordKind("unexpected_kind".to_string())
        );
    }

    #[test]
    fn rebuild_source_pool_state_rejects_seq_gaps() {
        let pool_ref = pool_ref();
        let err = rebuild_source_pool_state(
            pool_ref.clone(),
            &[StreamRecord {
                stream_id: pool_ref.stream_id(),
                seq: 2,
                ts_millis: None,
                kind: RECORD_KIND_POOL_RANKED.to_string(),
                payload: serde_json::json!({
                    "ranked_at_ms": 100_u64,
                    "ranked_source_ids": ["reth_local"],
                }),
            }],
        )
        .expect_err("seq gap must fail");
        assert_eq!(
            err,
            SourcePoolProjectionError::NonContiguousSeq {
                expected: 1,
                actual: 2,
            }
        );
    }
}
