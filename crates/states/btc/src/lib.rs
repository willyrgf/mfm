#![warn(missing_docs)]
//! Reusable Bitcoin fact state contracts.
//!
//! This crate owns typed Bitcoin facts and state-layer contracts used to observe bounded
//! chain-head data and carry collector checkpoints. It defines no JSON-RPC transport, runtime
//! source routing, workflow topology, CLI, REST, or app registration.

use std::future;
use std::num::NonZeroU64;

use mfm_btc_capabilities::{
    BtcCapabilityError, BtcChain, BtcChainGuard, BtcChainHeadReadCapability, BtcChainHeadRequest,
    BtcChainHeadResponse as CapabilityChainHeadResponse, BtcFinality, BtcHeadKind,
    BtcHeadSelection, BtcNetworkId, BtcSourceIdentity, BtcSourceStatus, RedactedBtcSourceEvidence,
};
use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::{CapabilitySpec, ManagedPlatformWriteRole};
use mfm_effects::{ManagedPlatformWrite, ReadExternal};
use mfm_fact_capabilities::{FactIndexReadCapability, FactIndexReadRequest, FactIndexReadResponse};
use mfm_facts::{
    compile_fact_query_plan, FactAudience, FactCanonicalScalar, FactFieldId, FactOrderingName,
    FactQueryInput, FactQueryOperator, FactQueryPredicate, FactQueryScope, FactSelectionEvidence,
    FactVisibility, FactVisibilityScope, ScopeDecisionEvidence, StoreScopeRef,
};
use mfm_ids::{AdapterKind, AdapterVersion, ContentDigest};
use mfm_ids::{DigestAlgorithm, StateKind, StateVersion};
use mfm_program::{
    fact_descriptor_ref, AdapterBindingSpec, CanonicalSeed, FactDescriptorRef, ManagedWriteState,
    MfmFactType, NoContext, ReadState, StateError, StateResult, StateSpec, ValidatedConfig,
};
use mfm_program_derive::{MfmConfig, MfmFactType, MfmValue, StateInput};
use serde::{Deserialize, Serialize};

const NAMESPACE: &str = "mfm.bitcoin";
const DEFAULT_SCOPE: &str = "default";
const DEFAULT_STORE_SCOPE: &str = "mfm.store.default";
const BTC_JSONRPC_ADAPTER_NAME: &str = "jsonrpc";
const BTC_JSONRPC_ADAPTER_VERSION: &str = "mfm.bitcoin.jsonrpc.adapter.v1";
const CHECKPOINT_QUERY_SELECTION_POLICY: &[u8] =
    b"mfm.bitcoin.collector-checkpoint.latest-selection.v1";

/// Returns the stable Bitcoin JSON-RPC adapter kind.
pub fn btc_jsonrpc_adapter_kind() -> Result<AdapterKind, mfm_ids::IdentityError> {
    AdapterKind::new(
        NAMESPACE,
        BTC_JSONRPC_ADAPTER_NAME,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.bitcoin.adapter:jsonrpc"),
    )
}

/// Returns the stable Bitcoin JSON-RPC adapter version.
pub fn btc_jsonrpc_adapter_version() -> Result<AdapterVersion, mfm_ids::IdentityError> {
    AdapterVersion::new(BTC_JSONRPC_ADAPTER_VERSION)
}

fn adapter_binding() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
    Ok(vec![AdapterBindingSpec {
        adapter_kind: btc_jsonrpc_adapter_kind().map_err(|error| {
            mfm_program::PlanError::Key(format!("Bitcoin JSON-RPC adapter kind invalid: {error}"))
        })?,
        adapter_version: btc_jsonrpc_adapter_version().map_err(|error| {
            mfm_program::PlanError::Key(format!(
                "Bitcoin JSON-RPC adapter version invalid: {error}"
            ))
        })?,
    }])
}

/// Returns the fact visibility for platform Bitcoin chain-head observations.
pub fn chain_head_fact_visibility() -> FactVisibility {
    FactVisibility::indexed_default(FactAudience::Platform)
}

/// Returns the fact visibility for internal collector checkpoint observations.
pub fn collector_checkpoint_fact_visibility() -> FactVisibility {
    FactVisibility::indexed_default(FactAudience::Control)
}

fn state_kind(name: &'static str) -> mfm_program::Result<StateKind> {
    StateKind::new(
        NAMESPACE,
        name,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.bitcoin.state:{name}").as_bytes()),
    )
    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

fn state_version(name: &'static str) -> mfm_program::Result<StateVersion> {
    StateVersion::new(format!("mfm.bitcoin.state.{name}.v1"))
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

fn adapter_required_error(state_name: &'static str) -> StateError {
    StateError::Message(format!("{state_name} requires a Bitcoin adapter runner"))
}

/// Managed platform-write capability for recording Bitcoin fact claims.
pub struct BtcFactRecordCapability;

impl CapabilitySpec for BtcFactRecordCapability {
    type Role = ManagedPlatformWriteRole;

    fn kind() -> mfm_capabilities::Result<mfm_ids::CapabilityKind> {
        mfm_ids::CapabilityKind::new(
            NAMESPACE,
            "fact.record",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.bitcoin.capability:fact.record"),
        )
        .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn version() -> mfm_capabilities::Result<mfm_ids::CapabilityVersion> {
        mfm_ids::CapabilityVersion::new("mfm.bitcoin.fact.record.v1")
            .map_err(|error| mfm_capabilities::CapabilityError::Identity(error.to_string()))
    }

    fn name() -> &'static str {
        "fact.record"
    }
}

/// Redaction-safe state error for Bitcoin fact normalization contracts.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BtcStateError {
    /// A state contract input was invalid.
    #[error("Bitcoin state input was invalid: {reason}")]
    InvalidInput {
        /// Stable redacted reason.
        reason: String,
    },
    /// A capability response failed request verification.
    #[error("Bitcoin capability response did not match the state request")]
    SourceMismatch,
    /// A capability provider failed without exposing source details.
    #[error("Bitcoin capability provider failed")]
    ProviderFailed,
}

impl From<BtcCapabilityError> for BtcStateError {
    fn from(error: BtcCapabilityError) -> Self {
        match error {
            BtcCapabilityError::InvalidRequest { reason } => Self::InvalidInput {
                reason: format!("{reason:?}"),
            },
            BtcCapabilityError::Provider { .. } => Self::ProviderFailed,
            BtcCapabilityError::SourceMismatch { .. } => Self::SourceMismatch,
        }
    }
}

impl From<BtcStateError> for StateError {
    fn from(error: BtcStateError) -> Self {
        StateError::Message(error.to_string())
    }
}

/// Subject identity for a Bitcoin chain-head platform fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "chain_head_subject",
    version = "1",
    schema = "mfm.bitcoin.fact.chain_head.subject"
)]
pub struct BtcChainHeadSubject {
    chain: String,
    network: String,
    semantic_source_identity: String,
    head_kind: String,
}

impl BtcChainHeadSubject {
    /// Creates chain-head subject material from redacted source evidence.
    pub fn from_evidence(evidence: &RedactedBtcSourceEvidence) -> Self {
        Self {
            chain: evidence.chain.as_str().to_owned(),
            network: evidence.network_id.as_str().to_owned(),
            semantic_source_identity: evidence.source_identity.as_str().to_owned(),
            head_kind: head_kind_tag(evidence.head_kind).to_owned(),
        }
    }

    /// Returns the chain tag.
    pub fn chain(&self) -> &str {
        &self.chain
    }

    /// Returns the semantic network id.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Returns the non-secret semantic source identity.
    pub fn semantic_source_identity(&self) -> &str {
        &self.semantic_source_identity
    }

    /// Returns the head-kind tag.
    pub fn head_kind(&self) -> &str {
        &self.head_kind
    }
}

/// Observed result for a Bitcoin chain-head platform fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "chain_head_response",
    version = "1",
    schema = "mfm.bitcoin.fact.chain_head.response"
)]
pub struct BtcChainHeadResponse {
    block_height: u64,
    block_hash: String,
    observed_source_status: String,
    observed_network: Option<String>,
    finality_policy: String,
    confirmation_depth: Option<u64>,
    provider_time_unix_ms: Option<u64>,
    observed_at_unix_ms: Option<u64>,
}

impl BtcChainHeadResponse {
    /// Creates fact response material from a verified capability response.
    pub fn from_capability_response(
        response: &CapabilityChainHeadResponse,
        observed_at_unix_ms: Option<u64>,
    ) -> Self {
        Self {
            block_height: response.block_height,
            block_hash: response.block_hash.as_str().to_owned(),
            observed_source_status: source_status_tag(response.evidence.source_status).to_owned(),
            observed_network: response.evidence.observed_network.clone(),
            finality_policy: finality_policy_tag(response.evidence.finality).to_owned(),
            confirmation_depth: response.evidence.finality.confirmation_depth(),
            provider_time_unix_ms: response.provider_time_unix_ms,
            observed_at_unix_ms,
        }
    }

    /// Returns the observed block height.
    pub const fn block_height(&self) -> u64 {
        self.block_height
    }

    /// Returns the observed block hash.
    pub fn block_hash(&self) -> &str {
        &self.block_hash
    }

    /// Returns the observed source status tag.
    pub fn observed_source_status(&self) -> &str {
        &self.observed_source_status
    }

    /// Returns the provider-reported block time in Unix milliseconds.
    pub const fn provider_time_unix_ms(&self) -> Option<u64> {
        self.provider_time_unix_ms
    }

    /// Returns the state observation time in Unix milliseconds.
    pub const fn observed_at_unix_ms(&self) -> Option<u64> {
        self.observed_at_unix_ms
    }
}

/// Platform fact for a Bitcoin chain-head observation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, MfmFactType)]
#[allow(clippy::duplicated_attributes)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "chain_head_fact",
    version = "1",
    schema = "mfm.bitcoin.fact.chain_head"
)]
#[mfm_fact(kind = "chain.head")]
#[mfm_fact(field(
    id = "subject.chain",
    source = "subject",
    path = "chain",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "subject.network",
    source = "subject",
    path = "network",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "subject.semantic_source_identity",
    source = "subject",
    path = "semantic_source_identity",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(field(
    id = "subject.head_kind",
    source = "subject",
    path = "head_kind",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.block_height",
    source = "result",
    path = "block_height",
    value_type = "unsigned_integer",
    operators(
        equal,
        greater_than,
        greater_than_or_equal,
        less_than,
        less_than_or_equal
    ),
    exposure = "returnable",
    sortable
))]
#[mfm_fact(field(
    id = "result.block_hash",
    source = "result",
    path = "block_hash",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.observed_source_status",
    source = "result",
    path = "observed_source_status",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.observed_at_unix_ms",
    source = "result",
    path = "observed_at_unix_ms",
    value_type = "unsigned_integer",
    operators(equal, greater_than_or_equal, less_than_or_equal),
    exposure = "query_only",
    optional,
    sortable
))]
#[mfm_fact(field(
    id = "metadata.observed_at",
    source = "metadata",
    metadata = "observed_at",
    value_type = "timestamp",
    operators(equal, greater_than_or_equal, less_than_or_equal),
    exposure = "query_only",
    optional,
    sortable
))]
#[mfm_fact(ordering(
    name = "result.block_height.desc",
    term(
        field = "result.block_height",
        direction = "descending",
        nulls = "last"
    )
))]
pub struct BtcChainHeadFact {
    subject: BtcChainHeadSubject,
    response: BtcChainHeadResponse,
}

impl BtcChainHeadFact {
    /// Creates a chain-head fact from subject and response material.
    pub const fn new(subject: BtcChainHeadSubject, response: BtcChainHeadResponse) -> Self {
        Self { subject, response }
    }

    /// Returns the chain-head subject material.
    pub const fn subject(&self) -> &BtcChainHeadSubject {
        &self.subject
    }

    /// Returns the chain-head response material.
    pub const fn response(&self) -> &BtcChainHeadResponse {
        &self.response
    }
}

/// Subject identity for an internal collector checkpoint fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "collector_checkpoint_subject",
    version = "1",
    schema = "mfm.bitcoin.fact.collector_checkpoint.subject"
)]
pub struct CollectorCheckpointSubject {
    collector_kind: String,
    semantic_source_identity: String,
    scope: String,
    partition: String,
    chain: String,
    network: String,
}

impl CollectorCheckpointSubject {
    /// Creates checkpoint subject material.
    pub fn new(
        collector_kind: impl Into<String>,
        source_identity: &BtcSourceIdentity,
        partition: impl Into<String>,
        chain: BtcChain,
        network: &BtcNetworkId,
    ) -> Self {
        Self {
            collector_kind: collector_kind.into(),
            semantic_source_identity: source_identity.as_str().to_owned(),
            scope: DEFAULT_SCOPE.to_owned(),
            partition: partition.into(),
            chain: chain.as_str().to_owned(),
            network: network.as_str().to_owned(),
        }
    }

    /// Returns the collector kind.
    pub fn collector_kind(&self) -> &str {
        &self.collector_kind
    }

    /// Returns the non-secret semantic source identity.
    pub fn semantic_source_identity(&self) -> &str {
        &self.semantic_source_identity
    }

    /// Returns the checkpoint visibility scope tag.
    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// Returns the checkpoint partition.
    pub fn partition(&self) -> &str {
        &self.partition
    }

    /// Returns the chain tag.
    pub fn chain(&self) -> &str {
        &self.chain
    }

    /// Returns the semantic network id.
    pub fn network(&self) -> &str {
        &self.network
    }
}

/// Observed result for an internal collector checkpoint fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "collector_checkpoint_response",
    version = "1",
    schema = "mfm.bitcoin.fact.collector_checkpoint.response"
)]
pub struct CollectorCheckpointResponse {
    high_watermark_height: u64,
    high_watermark_hash: String,
    predecessor_checkpoint_ref: Option<String>,
    predecessor_checkpoint_hash: Option<String>,
    finality_policy: String,
    confirmation_depth: Option<u64>,
}

impl CollectorCheckpointResponse {
    /// Creates checkpoint response material.
    pub fn new(
        high_watermark_height: u64,
        high_watermark_hash: impl Into<String>,
        predecessor_checkpoint_ref: Option<String>,
        predecessor_checkpoint_hash: Option<String>,
        finality: BtcFinality,
    ) -> Self {
        Self {
            high_watermark_height,
            high_watermark_hash: high_watermark_hash.into(),
            predecessor_checkpoint_ref,
            predecessor_checkpoint_hash,
            finality_policy: finality_policy_tag(finality).to_owned(),
            confirmation_depth: finality.confirmation_depth(),
        }
    }

    /// Returns the high-watermark height.
    pub const fn high_watermark_height(&self) -> u64 {
        self.high_watermark_height
    }

    /// Returns the high-watermark hash.
    pub fn high_watermark_hash(&self) -> &str {
        &self.high_watermark_hash
    }

    /// Returns the predecessor checkpoint material hash, when present.
    pub fn predecessor_checkpoint_hash(&self) -> Option<&str> {
        self.predecessor_checkpoint_hash.as_deref()
    }

    /// Returns the finality policy tag.
    pub fn finality_policy(&self) -> &str {
        &self.finality_policy
    }

    /// Returns the confirmation depth attached to the finality policy.
    pub const fn confirmation_depth(&self) -> Option<u64> {
        self.confirmation_depth
    }
}

/// Control fact for a collector checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, MfmFactType)]
#[allow(clippy::duplicated_attributes)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "collector_checkpoint_fact",
    version = "1",
    schema = "mfm.bitcoin.fact.collector_checkpoint"
)]
#[mfm_fact(kind = "collector.checkpoint")]
#[mfm_fact(field(
    id = "subject.collector_kind",
    source = "subject",
    path = "collector_kind",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(field(
    id = "subject.semantic_source_identity",
    source = "subject",
    path = "semantic_source_identity",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(field(
    id = "subject.scope",
    source = "subject",
    path = "scope",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(field(
    id = "subject.partition",
    source = "subject",
    path = "partition",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(field(
    id = "subject.chain",
    source = "subject",
    path = "chain",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(field(
    id = "subject.network",
    source = "subject",
    path = "network",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(field(
    id = "result.high_watermark_height",
    source = "result",
    path = "high_watermark_height",
    value_type = "unsigned_integer",
    operators(
        equal,
        greater_than,
        greater_than_or_equal,
        less_than,
        less_than_or_equal
    ),
    exposure = "returnable",
    sortable
))]
#[mfm_fact(field(
    id = "result.high_watermark_hash",
    source = "result",
    path = "high_watermark_hash",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(field(
    id = "result.predecessor_checkpoint_ref",
    source = "result",
    path = "predecessor_checkpoint_ref",
    value_type = "string",
    exposure = "hidden",
    optional
))]
#[mfm_fact(field(
    id = "result.predecessor_checkpoint_hash",
    source = "result",
    path = "predecessor_checkpoint_hash",
    value_type = "digest",
    exposure = "hidden",
    optional
))]
#[mfm_fact(field(
    id = "result.finality_policy",
    source = "result",
    path = "finality_policy",
    value_type = "string",
    exposure = "query_only"
))]
#[mfm_fact(ordering(
    name = "result.high_watermark_height.desc",
    term(
        field = "result.high_watermark_height",
        direction = "descending",
        nulls = "last"
    )
))]
pub struct CollectorCheckpointFact {
    subject: CollectorCheckpointSubject,
    response: CollectorCheckpointResponse,
}

impl CollectorCheckpointFact {
    /// Creates a collector checkpoint fact from subject and response material.
    pub const fn new(
        subject: CollectorCheckpointSubject,
        response: CollectorCheckpointResponse,
    ) -> Self {
        Self { subject, response }
    }

    /// Returns the checkpoint subject material.
    pub const fn subject(&self) -> &CollectorCheckpointSubject {
        &self.subject
    }

    /// Returns the checkpoint response material.
    pub const fn response(&self) -> &CollectorCheckpointResponse {
        &self.response
    }
}

/// Bounded config for a Bitcoin chain-head observation state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(
    schema = "mfm.bitcoin.state.config.observe_chain_head",
    validate = "validate_observe_chain_head_config"
)]
pub struct ObserveBtcChainHeadConfig {
    /// Semantic network id.
    pub network: String,
    /// Non-secret semantic source identity.
    pub semantic_source_identity: String,
    /// Head kind requested by the state.
    pub head_kind: String,
    /// Optional confirmation depth for confirmed-head reads.
    pub confirmation_depth: Option<NonZeroU64>,
    /// Maximum number of source reads this bounded state may request.
    pub max_source_reads: NonZeroU64,
}

/// Validates chain-head observation config.
pub fn validate_observe_chain_head_config(
    config: &ObserveBtcChainHeadConfig,
) -> Result<(), String> {
    BtcNetworkId::new(&config.network).map_err(|error| error.to_string())?;
    BtcSourceIdentity::new(&config.semantic_source_identity).map_err(|error| error.to_string())?;
    match config.head_kind.as_str() {
        "best" if config.confirmation_depth.is_none() => Ok(()),
        "confirmed" if config.confirmation_depth.is_some() => Ok(()),
        "best" => Err("best head observations must not set confirmation_depth".to_owned()),
        "confirmed" => Err("confirmed head observations require confirmation_depth".to_owned()),
        _ => Err("head_kind must be `best` or `confirmed`".to_owned()),
    }
}

impl ObserveBtcChainHeadConfig {
    /// Builds the typed capability request for this bounded observation.
    pub fn request(&self) -> Result<BtcChainHeadRequest, BtcStateError> {
        validate_observe_chain_head_config(self)
            .map_err(|reason| BtcStateError::InvalidInput { reason })?;
        let network = BtcNetworkId::new(&self.network)?;
        let source_identity = BtcSourceIdentity::new(&self.semantic_source_identity)?;
        let guard = BtcChainGuard::new(BtcChain::Bitcoin, network, source_identity);
        let selection = match self.head_kind.as_str() {
            "best" => BtcHeadSelection::best(),
            "confirmed" => BtcHeadSelection::confirmed(
                self.confirmation_depth
                    .expect("validated confirmation depth")
                    .get(),
            )?,
            _ => {
                return Err(BtcStateError::InvalidInput {
                    reason: "head_kind must be `best` or `confirmed`".to_owned(),
                })
            }
        };
        Ok(BtcChainHeadRequest { guard, selection })
    }
}

/// Input for a bounded Bitcoin chain-head observation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.bitcoin.state.input.observe_chain_head")]
pub struct ObserveBtcChainHeadInput {
    /// Loaded collector checkpoint selected by the upstream checkpoint query.
    pub loaded_checkpoint: LoadedCollectorCheckpoint,
    /// Optional observation context supplied by the adapter.
    pub context: BtcChainHeadObservationContext,
}

/// Adapter-supplied context for a bounded Bitcoin chain-head observation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "chain_head_observation_context",
    version = "1",
    schema = "mfm.bitcoin.state.value.chain_head_observation_context"
)]
pub struct BtcChainHeadObservationContext {
    /// State observation time in Unix milliseconds, when supplied by an adapter.
    pub observed_at_unix_ms: Option<u64>,
}

/// Normalized chain-head observation state output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "chain_head_observation",
    version = "1",
    schema = "mfm.bitcoin.state.output.chain_head_observation"
)]
pub struct BtcChainHeadObservation {
    subject: BtcChainHeadSubject,
    response: BtcChainHeadResponse,
    source_read_count: u64,
}

impl BtcChainHeadObservation {
    /// Creates normalized observation output.
    pub const fn new(
        subject: BtcChainHeadSubject,
        response: BtcChainHeadResponse,
        source_read_count: u64,
    ) -> Self {
        Self {
            subject,
            response,
            source_read_count,
        }
    }

    /// Returns the normalized chain-head subject.
    pub const fn subject(&self) -> &BtcChainHeadSubject {
        &self.subject
    }

    /// Returns the normalized chain-head response.
    pub const fn response(&self) -> &BtcChainHeadResponse {
        &self.response
    }

    /// Builds the platform fact from this whole observation output.
    pub fn to_fact(&self) -> BtcChainHeadFact {
        BtcChainHeadFact::new(self.subject.clone(), self.response.clone())
    }

    /// Converts this whole observation output into the platform fact.
    pub fn into_fact(self) -> BtcChainHeadFact {
        BtcChainHeadFact::new(self.subject, self.response)
    }

    /// Returns the number of source reads used by this bounded observation.
    pub const fn source_read_count(&self) -> u64 {
        self.source_read_count
    }
}

/// Normalizes a verified Bitcoin capability response into a chain-head observation output.
pub fn normalize_chain_head_response(
    request: &BtcChainHeadRequest,
    response: &CapabilityChainHeadResponse,
    input: &ObserveBtcChainHeadInput,
) -> Result<BtcChainHeadObservation, BtcStateError> {
    response.verify_request(request)?;
    validate_loaded_checkpoint_for_request(request, &input.loaded_checkpoint)?;
    let observation = BtcChainHeadObservation::new(
        BtcChainHeadSubject::from_evidence(&response.evidence),
        BtcChainHeadResponse::from_capability_response(response, input.context.observed_at_unix_ms),
        1,
    );
    reject_observation_behind_checkpoint(&observation, &input.loaded_checkpoint)?;
    Ok(observation)
}

/// State contract for a bounded Bitcoin chain-head observation.
pub struct ObserveBtcChainHeadState {
    config: ObserveBtcChainHeadConfig,
}

impl ObserveBtcChainHeadState {
    /// Builds the Bitcoin chain-head request declared by this state config.
    pub fn request(&self) -> Result<BtcChainHeadRequest, BtcStateError> {
        self.config.request()
    }

    /// Materializes a normalized observation from a verified capability response.
    pub fn materialize_response(
        &self,
        input: &ObserveBtcChainHeadInput,
        response: &CapabilityChainHeadResponse,
    ) -> Result<BtcChainHeadObservation, BtcStateError> {
        let request = self.request()?;
        normalize_chain_head_response(&request, response, input)
    }
}

impl StateSpec for ObserveBtcChainHeadState {
    type Config = ObserveBtcChainHeadConfig;
    type Context = NoContext;
    type Input = ObserveBtcChainHeadInput;
    type Output = BtcChainHeadObservation;
    type Effect = ReadExternal;
    type Caps = (BtcChainHeadReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("chain_head.observe")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("chain_head.observe")
    }

    fn name() -> &'static str {
        "mfm.bitcoin.chain_head.observe"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn new(config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ReadState for ObserveBtcChainHeadState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        if let Err(error) = self.request().and_then(|request| {
            validate_loaded_checkpoint_for_request(&request, &input.loaded_checkpoint)
        }) {
            return future::ready(Err(StateError::from(error)));
        }
        future::ready(Err(adapter_required_error(Self::name())))
    }
}

/// Config for a chain-head fact recording state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(schema = "mfm.bitcoin.state.config.record_chain_head_fact")]
pub struct RecordBtcChainHeadFactConfig {}

/// Input for recording a Bitcoin chain-head fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.bitcoin.state.input.record_chain_head_fact")]
pub struct RecordBtcChainHeadFactInput {
    /// Whole chain-head observation output to transform into a fact.
    pub observation: BtcChainHeadObservation,
}

/// State contract for recording a Bitcoin chain-head fact.
pub struct RecordBtcChainHeadFactState;

impl StateSpec for RecordBtcChainHeadFactState {
    type Config = RecordBtcChainHeadFactConfig;
    type Context = NoContext;
    type Input = RecordBtcChainHeadFactInput;
    type Output = BtcChainHeadFact;
    type Effect = ManagedPlatformWrite;
    type Caps = (BtcFactRecordCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("chain_head.record")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("chain_head.record")
    }

    fn name() -> &'static str {
        "mfm.bitcoin.chain_head.record"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn emitted_fact_descriptors() -> mfm_program::Result<Vec<FactDescriptorRef>> {
        Ok(vec![fact_descriptor_ref::<BtcChainHeadFact>()?])
    }

    fn new(_config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self)
    }
}

impl ManagedWriteState for RecordBtcChainHeadFactState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        future::ready(Ok(input.observation.into_fact()))
    }
}

/// Config for querying the latest collector checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(
    schema = "mfm.bitcoin.state.config.query_collector_checkpoint",
    validate = "validate_query_collector_checkpoint_config"
)]
pub struct QueryCollectorCheckpointConfig {
    /// Collector kind whose checkpoint should be loaded.
    pub collector_kind: String,
    /// Non-secret semantic source identity whose checkpoint should be loaded.
    pub semantic_source_identity: String,
    /// Semantic checkpoint partition.
    pub partition: String,
    /// Semantic Bitcoin network id.
    pub network: String,
    /// Non-secret store scope for this internal fact-index query.
    pub store_scope: String,
}

/// Validates collector checkpoint query config.
pub fn validate_query_collector_checkpoint_config(
    config: &QueryCollectorCheckpointConfig,
) -> Result<(), String> {
    validate_collector_checkpoint_common(
        &config.collector_kind,
        &config.semantic_source_identity,
        &config.partition,
        &config.network,
    )?;
    StoreScopeRef::new(&config.store_scope).map_err(|error| error.to_string())?;
    Ok(())
}

impl QueryCollectorCheckpointConfig {
    /// Builds the subject identity for this checkpoint query.
    pub fn checkpoint_subject(&self) -> Result<CollectorCheckpointSubject, BtcStateError> {
        validate_query_collector_checkpoint_config(self)
            .map_err(|reason| BtcStateError::InvalidInput { reason })?;
        let source_identity = BtcSourceIdentity::new(&self.semantic_source_identity)?;
        let network = BtcNetworkId::new(&self.network)?;
        Ok(CollectorCheckpointSubject::new(
            self.collector_kind.clone(),
            &source_identity,
            self.partition.clone(),
            BtcChain::Bitcoin,
            &network,
        ))
    }

    /// Builds the Control fact-index read request for the latest checkpoint.
    pub fn request(&self) -> Result<FactIndexReadRequest, BtcStateError> {
        validate_query_collector_checkpoint_config(self)
            .map_err(|reason| BtcStateError::InvalidInput { reason })?;
        let descriptor =
            CollectorCheckpointFact::descriptor().map_err(|error| BtcStateError::InvalidInput {
                reason: format!("collector checkpoint descriptor invalid: {error}"),
            })?;
        let plan = compile_fact_query_plan(&descriptor, collector_checkpoint_query_input(self)?)
            .map_err(|error| BtcStateError::InvalidInput {
                reason: error.to_string(),
            })?;
        FactIndexReadRequest::new(plan).map_err(|error| BtcStateError::InvalidInput {
            reason: error.to_string(),
        })
    }

    /// Materializes a checkpoint fact from retained response material.
    pub fn checkpoint_fact_from_response(
        &self,
        response: CollectorCheckpointResponse,
    ) -> Result<CollectorCheckpointFact, BtcStateError> {
        Ok(CollectorCheckpointFact::new(
            self.checkpoint_subject()?,
            response,
        ))
    }

    /// Rebuilds loaded checkpoint output from recorded query evidence and retained response material.
    pub fn loaded_checkpoint_from_replay_evidence(
        &self,
        evidence: &mfm_facts::FactQueryEvidence,
        response: Option<CollectorCheckpointResponse>,
    ) -> Result<LoadedCollectorCheckpoint, BtcStateError> {
        mfm_facts::validate_fact_query_evidence(evidence).map_err(|error| {
            BtcStateError::InvalidInput {
                reason: error.to_string(),
            }
        })?;
        match (
            evidence.receipt().returned_refs().len(),
            evidence.selection().selected_indices(),
            response,
        ) {
            (0, [], None) => Ok(LoadedCollectorCheckpoint::new(None)),
            (1, [0], Some(response)) => self
                .checkpoint_fact_from_response(response)
                .map(|checkpoint| LoadedCollectorCheckpoint::new(Some(checkpoint))),
            _ => Err(BtcStateError::InvalidInput {
                reason: "checkpoint replay evidence did not match latest-checkpoint selection"
                    .to_owned(),
            }),
        }
    }
}

impl Default for QueryCollectorCheckpointConfig {
    fn default() -> Self {
        Self {
            collector_kind: "btc-chain-head".to_owned(),
            semantic_source_identity: "public-bitcoin-core".to_owned(),
            partition: "chain-head".to_owned(),
            network: "bitcoin-mainnet".to_owned(),
            store_scope: DEFAULT_STORE_SCOPE.to_owned(),
        }
    }
}

/// Input for querying a collector checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.bitcoin.state.input.query_collector_checkpoint")]
pub struct QueryCollectorCheckpointInput {}

/// Output from loading a collector checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "loaded_collector_checkpoint",
    version = "1",
    schema = "mfm.bitcoin.state.output.loaded_collector_checkpoint"
)]
pub struct LoadedCollectorCheckpoint {
    checkpoint: Option<CollectorCheckpointFact>,
}

impl LoadedCollectorCheckpoint {
    /// Creates loaded checkpoint output.
    pub const fn new(checkpoint: Option<CollectorCheckpointFact>) -> Self {
        Self { checkpoint }
    }

    /// Returns the loaded checkpoint fact, when present.
    pub const fn checkpoint(&self) -> Option<&CollectorCheckpointFact> {
        self.checkpoint.as_ref()
    }
}

/// State contract for querying the latest Control collector checkpoint.
pub struct QueryCollectorCheckpointState {
    config: QueryCollectorCheckpointConfig,
}

impl QueryCollectorCheckpointState {
    /// Builds the fact-index request declared by this state config.
    pub fn request(&self) -> Result<FactIndexReadRequest, BtcStateError> {
        self.config.request()
    }

    /// Materializes a loaded checkpoint from an authenticated fact-index response.
    pub fn materialize_response(
        &self,
        response: &FactIndexReadResponse,
        checkpoint: Option<CollectorCheckpointFact>,
    ) -> Result<LoadedCollectorCheckpoint, BtcStateError> {
        match (has_latest_checkpoint_row(response)?, checkpoint) {
            (false, None) => Ok(LoadedCollectorCheckpoint::new(None)),
            (true, Some(checkpoint)) => Ok(LoadedCollectorCheckpoint::new(Some(checkpoint))),
            (false, Some(_)) => Err(BtcStateError::InvalidInput {
                reason: "checkpoint material was supplied for an empty fact-index receipt"
                    .to_owned(),
            }),
            (true, None) => Err(BtcStateError::InvalidInput {
                reason: "checkpoint receipt row requires materialized checkpoint fact".to_owned(),
            }),
        }
    }

    /// Returns the retained response fact ref selected by the latest-checkpoint policy.
    pub fn selected_checkpoint_response_ref<'a>(
        &self,
        response: &'a FactIndexReadResponse,
    ) -> Result<Option<&'a mfm_facts::InternalFactRef>, BtcStateError> {
        latest_checkpoint_response_ref(response)
    }

    /// Materializes a checkpoint fact from retained response material.
    pub fn checkpoint_fact_from_response(
        &self,
        response: CollectorCheckpointResponse,
    ) -> Result<CollectorCheckpointFact, BtcStateError> {
        self.config.checkpoint_fact_from_response(response)
    }

    /// Builds selection evidence for the state-owned latest-checkpoint policy.
    pub fn selection_evidence(
        &self,
        response: &FactIndexReadResponse,
    ) -> Result<FactSelectionEvidence, BtcStateError> {
        let selected_indices = if has_latest_checkpoint_row(response)? {
            vec![0]
        } else {
            Vec::new()
        };
        FactSelectionEvidence::new(selection_policy_hash(), selected_indices, None).map_err(
            |error| BtcStateError::InvalidInput {
                reason: error.to_string(),
            },
        )
    }
}

fn has_latest_checkpoint_row(response: &FactIndexReadResponse) -> Result<bool, BtcStateError> {
    latest_checkpoint_response_ref(response).map(|row| row.is_some())
}

fn latest_checkpoint_response_ref(
    response: &FactIndexReadResponse,
) -> Result<Option<&mfm_facts::InternalFactRef>, BtcStateError> {
    match response.rows() {
        [] => Ok(None),
        [row] => Ok(Some(row.fact_ref())),
        _ => Err(BtcStateError::InvalidInput {
            reason: "latest checkpoint query must return at most one row".to_owned(),
        }),
    }
}

impl StateSpec for QueryCollectorCheckpointState {
    type Config = QueryCollectorCheckpointConfig;
    type Context = NoContext;
    type Input = QueryCollectorCheckpointInput;
    type Output = LoadedCollectorCheckpoint;
    type Effect = ReadExternal;
    type Caps = (FactIndexReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("collector_checkpoint.query")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("collector_checkpoint.query")
    }

    fn name() -> &'static str {
        "mfm.bitcoin.collector_checkpoint.query"
    }

    fn new(config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ReadState for QueryCollectorCheckpointState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        _input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        let _request = self.request();
        future::ready(Err(adapter_required_error(Self::name())))
    }
}

/// Config for a collector checkpoint fact recording state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(
    schema = "mfm.bitcoin.state.config.record_collector_checkpoint",
    validate = "validate_record_collector_checkpoint_config"
)]
pub struct RecordCollectorCheckpointConfig {
    /// Collector kind whose checkpoint should be recorded.
    pub collector_kind: String,
    /// Semantic checkpoint partition.
    pub partition: String,
}

/// Validates collector checkpoint record config.
pub fn validate_record_collector_checkpoint_config(
    config: &RecordCollectorCheckpointConfig,
) -> Result<(), String> {
    validate_non_secret_label("collector_kind", &config.collector_kind)?;
    validate_non_secret_label("partition", &config.partition)
}

impl RecordCollectorCheckpointConfig {
    /// Builds the checkpoint subject identity for a recorded chain-head fact.
    pub fn checkpoint_subject(
        &self,
        chain_head_fact: &BtcChainHeadFact,
    ) -> Result<CollectorCheckpointSubject, BtcStateError> {
        validate_record_collector_checkpoint_config(self)
            .map_err(|reason| BtcStateError::InvalidInput { reason })?;
        Ok(CollectorCheckpointSubject {
            collector_kind: self.collector_kind.clone(),
            semantic_source_identity: chain_head_fact.subject().semantic_source_identity.clone(),
            scope: DEFAULT_SCOPE.to_owned(),
            partition: self.partition.clone(),
            chain: chain_head_fact.subject().chain.clone(),
            network: chain_head_fact.subject().network.clone(),
        })
    }
}

/// Input for recording a collector checkpoint fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.bitcoin.state.input.record_collector_checkpoint")]
pub struct RecordCollectorCheckpointInput {
    /// Recorded upstream chain-head fact output.
    pub chain_head_fact: BtcChainHeadFact,
    /// Whole upstream checkpoint query output.
    pub loaded_checkpoint: LoadedCollectorCheckpoint,
}

/// State contract for recording a collector checkpoint fact.
pub struct RecordCollectorCheckpointState {
    config: RecordCollectorCheckpointConfig,
}

impl StateSpec for RecordCollectorCheckpointState {
    type Config = RecordCollectorCheckpointConfig;
    type Context = NoContext;
    type Input = RecordCollectorCheckpointInput;
    type Output = CollectorCheckpointFact;
    type Effect = ManagedPlatformWrite;
    type Caps = (BtcFactRecordCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("collector_checkpoint.record")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("collector_checkpoint.record")
    }

    fn name() -> &'static str {
        "mfm.bitcoin.collector_checkpoint.record"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn emitted_fact_descriptors() -> mfm_program::Result<Vec<FactDescriptorRef>> {
        Ok(vec![fact_descriptor_ref::<CollectorCheckpointFact>()?])
    }

    fn new(config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ManagedWriteState for RecordCollectorCheckpointState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        future::ready(
            build_checkpoint_fact_from_outputs(
                &self.config,
                &input.chain_head_fact,
                &input.loaded_checkpoint,
            )
            .map_err(StateError::from),
        )
    }
}

fn build_checkpoint_fact_from_outputs(
    config: &RecordCollectorCheckpointConfig,
    chain_head_fact: &BtcChainHeadFact,
    loaded_checkpoint: &LoadedCollectorCheckpoint,
) -> Result<CollectorCheckpointFact, BtcStateError> {
    validate_loaded_checkpoint_for_recorded_fact(config, chain_head_fact, loaded_checkpoint)?;

    let subject = config.checkpoint_subject(chain_head_fact)?;
    let predecessor_checkpoint_hash = loaded_checkpoint
        .checkpoint()
        .map(checkpoint_material_hash)
        .transpose()?
        .map(|digest| digest.as_str().to_owned());
    let response = CollectorCheckpointResponse {
        high_watermark_height: chain_head_fact.response().block_height,
        high_watermark_hash: chain_head_fact.response().block_hash.clone(),
        predecessor_checkpoint_ref: None,
        predecessor_checkpoint_hash,
        finality_policy: chain_head_fact.response().finality_policy.clone(),
        confirmation_depth: chain_head_fact.response().confirmation_depth,
    };
    Ok(CollectorCheckpointFact::new(subject, response))
}

fn validate_loaded_checkpoint_for_request(
    request: &BtcChainHeadRequest,
    loaded_checkpoint: &LoadedCollectorCheckpoint,
) -> Result<(), BtcStateError> {
    let Some(checkpoint) = loaded_checkpoint.checkpoint() else {
        return Ok(());
    };
    let subject = checkpoint.subject();
    let response = checkpoint.response();
    if subject.chain() != request.guard.chain().as_str()
        || subject.network() != request.guard.network_id().as_str()
        || subject.semantic_source_identity() != request.guard.source_identity().as_str()
        || subject.scope() != DEFAULT_SCOPE
        || response.finality_policy() != finality_policy_tag(request.selection.finality())
        || response.confirmation_depth() != request.selection.finality().confirmation_depth()
    {
        return Err(BtcStateError::InvalidInput {
            reason: "loaded checkpoint is incompatible with requested Bitcoin source".to_owned(),
        });
    }
    Ok(())
}

fn reject_observation_behind_checkpoint(
    observation: &BtcChainHeadObservation,
    loaded_checkpoint: &LoadedCollectorCheckpoint,
) -> Result<(), BtcStateError> {
    if let Some(previous) = loaded_checkpoint.checkpoint() {
        if observation.response.block_height < previous.response.high_watermark_height {
            return Err(BtcStateError::InvalidInput {
                reason: "observed chain head must not be behind loaded checkpoint".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_loaded_checkpoint_for_recorded_fact(
    config: &RecordCollectorCheckpointConfig,
    chain_head_fact: &BtcChainHeadFact,
    loaded_checkpoint: &LoadedCollectorCheckpoint,
) -> Result<(), BtcStateError> {
    let Some(previous) = loaded_checkpoint.checkpoint() else {
        return Ok(());
    };
    let previous_subject = previous.subject();
    if previous_subject.collector_kind() != config.collector_kind
        || previous_subject.partition() != config.partition
        || previous_subject.scope() != DEFAULT_SCOPE
        || previous_subject.chain() != chain_head_fact.subject().chain()
        || previous_subject.network() != chain_head_fact.subject().network()
        || previous_subject.semantic_source_identity()
            != chain_head_fact.subject().semantic_source_identity()
        || previous.response().finality_policy() != chain_head_fact.response().finality_policy
        || previous.response().confirmation_depth() != chain_head_fact.response().confirmation_depth
    {
        return Err(BtcStateError::InvalidInput {
            reason: "loaded checkpoint is incompatible with recorded chain-head fact".to_owned(),
        });
    }
    if chain_head_fact.response().block_height < previous.response().high_watermark_height {
        return Err(BtcStateError::InvalidInput {
            reason: "new checkpoint height must not move behind loaded checkpoint".to_owned(),
        });
    }
    Ok(())
}

fn checkpoint_material_hash(
    checkpoint: &CollectorCheckpointFact,
) -> Result<ContentDigest, BtcStateError> {
    CanonicalSeed::from_value(checkpoint)
        .map(|seed| seed.content_digest().clone())
        .map_err(|error| BtcStateError::InvalidInput {
            reason: format!("checkpoint material canonicalization failed: {error}"),
        })
}

fn collector_checkpoint_query_input(
    config: &QueryCollectorCheckpointConfig,
) -> Result<FactQueryInput, BtcStateError> {
    FactQueryInput::new(
        StoreScopeRef::new(&config.store_scope).map_err(|error| BtcStateError::InvalidInput {
            reason: error.to_string(),
        })?,
        FactQueryScope::new(FactAudience::Control, FactVisibilityScope::Default),
        ScopeDecisionEvidence::new(control_scope_decision_hash()),
        vec![
            query_predicate(
                "subject.chain",
                FactCanonicalScalar::string(BtcChain::Bitcoin.as_str()),
            )?,
            query_predicate(
                "subject.collector_kind",
                FactCanonicalScalar::string(&config.collector_kind),
            )?,
            query_predicate(
                "subject.network",
                FactCanonicalScalar::string(&config.network),
            )?,
            query_predicate(
                "subject.partition",
                FactCanonicalScalar::string(&config.partition),
            )?,
            query_predicate("subject.scope", FactCanonicalScalar::string(DEFAULT_SCOPE))?,
            query_predicate(
                "subject.semantic_source_identity",
                FactCanonicalScalar::string(&config.semantic_source_identity),
            )?,
        ],
        vec![query_return_field("result.high_watermark_height")?],
        FactOrderingName::new("result.high_watermark_height.desc").map_err(|error| {
            BtcStateError::InvalidInput {
                reason: error.to_string(),
            }
        })?,
        Some(1),
    )
    .map_err(|error| BtcStateError::InvalidInput {
        reason: error.to_string(),
    })
}

fn query_return_field(field_id: &str) -> Result<FactFieldId, BtcStateError> {
    FactFieldId::new(field_id).map_err(|error| BtcStateError::InvalidInput {
        reason: error.to_string(),
    })
}

fn query_predicate(
    field_id: &str,
    value: FactCanonicalScalar,
) -> Result<FactQueryPredicate, BtcStateError> {
    let field_id = FactFieldId::new(field_id).map_err(|error| BtcStateError::InvalidInput {
        reason: error.to_string(),
    })?;
    Ok(FactQueryPredicate::new(
        field_id,
        FactQueryOperator::Equal,
        value,
    ))
}

fn validate_collector_checkpoint_common(
    collector_kind: &str,
    semantic_source_identity: &str,
    partition: &str,
    network: &str,
) -> Result<(), String> {
    validate_non_secret_label("collector_kind", collector_kind)?;
    validate_non_secret_label("partition", partition)?;
    BtcSourceIdentity::new(semantic_source_identity).map_err(|error| error.to_string())?;
    BtcNetworkId::new(network).map_err(|error| error.to_string())?;
    Ok(())
}

fn validate_non_secret_label(name: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err(format!("{name} must not be empty"));
    }
    for forbidden in [
        "http://",
        "https://",
        "password",
        "auth",
        "token",
        "secret",
        "localhost",
        "127.0.0.1",
    ] {
        if value.contains(forbidden) {
            return Err(format!(
                "{name} contains forbidden runtime routing or secret material"
            ));
        }
    }
    Ok(())
}

fn control_scope_decision_hash() -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.bitcoin.collector-checkpoint.control-scope.default.v1"),
    )
}

fn selection_policy_hash() -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(CHECKPOINT_QUERY_SELECTION_POLICY),
    )
}

fn head_kind_tag(kind: BtcHeadKind) -> &'static str {
    match kind {
        BtcHeadKind::Best => "best",
        BtcHeadKind::Confirmed => "confirmed",
    }
}

fn source_status_tag(status: BtcSourceStatus) -> &'static str {
    match status {
        BtcSourceStatus::Synced => "synced",
        BtcSourceStatus::InitialBlockDownload => "initial_block_download",
        BtcSourceStatus::Unknown => "unknown",
    }
}

fn finality_policy_tag(finality: BtcFinality) -> &'static str {
    match finality {
        BtcFinality::BestAvailable => "best_available",
        BtcFinality::Confirmations(_) => "confirmations",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_btc_capabilities::BtcBlockHash;
    use mfm_capabilities::CapabilitySpec;
    use mfm_effects::EffectSpec;
    use mfm_facts::{
        fact_descriptor_hash, fact_query_plan_hash, CanonicalFactQueryPlan,
        DescriptorCatalogWatermark, FactClaimId, FactFieldExposure, FactFieldExtraction,
        FactFieldValueType, FactProducerProvenance, FactProjectionGeneration, FactQueryReceipt,
        FactQueryReceiptMaterial, FactResponseEvidence, FactSubjectRef, InternalFactRef,
        InternalFactRefParts, StoreCommitWatermark, StoreIdentity, StoreKeyId, StoreReadFrontier,
        StoreReadFrontierType, StoreReceiptAuthentication, StoreReceiptAuthenticationScheme,
    };
    use mfm_ids::{
        ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest, DigestBytes, EventId, RunId,
        SchemaId,
    };

    fn poll_ready<F>(future: F) -> F::Output
    where
        F: std::future::Future,
    {
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        let mut future = std::pin::pin!(future);
        match std::future::Future::poll(future.as_mut(), &mut cx) {
            std::task::Poll::Ready(output) => output,
            std::task::Poll::Pending => panic!("test future unexpectedly pending"),
        }
    }

    fn observe_config() -> ObserveBtcChainHeadConfig {
        ObserveBtcChainHeadConfig {
            network: "bitcoin-mainnet".to_owned(),
            semantic_source_identity: "public-bitcoin-core".to_owned(),
            head_kind: "best".to_owned(),
            confirmation_depth: None,
            max_source_reads: NonZeroU64::new(1).expect("nonzero"),
        }
    }

    fn capability_response() -> (BtcChainHeadRequest, CapabilityChainHeadResponse) {
        let request = observe_config().request().expect("request");
        let evidence = RedactedBtcSourceEvidence::from_request(
            &request,
            Some("main".to_owned()),
            BtcSourceStatus::Synced,
        );
        let response = CapabilityChainHeadResponse {
            evidence,
            block_height: 850_000,
            block_hash: BtcBlockHash::new(
                "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc",
            )
            .expect("block hash"),
            provider_time_unix_ms: Some(1_720_000_000_000),
        };
        (request, response)
    }

    fn observe_input(checkpoint: Option<CollectorCheckpointFact>) -> ObserveBtcChainHeadInput {
        ObserveBtcChainHeadInput {
            loaded_checkpoint: LoadedCollectorCheckpoint::new(checkpoint),
            context: BtcChainHeadObservationContext {
                observed_at_unix_ms: Some(1_720_000_001_000),
            },
        }
    }

    fn checkpoint_query_config() -> QueryCollectorCheckpointConfig {
        QueryCollectorCheckpointConfig {
            collector_kind: "btc-chain-head".to_owned(),
            semantic_source_identity: "public-bitcoin-core".to_owned(),
            partition: "chain-head".to_owned(),
            network: "bitcoin-mainnet".to_owned(),
            store_scope: "mfm.store.default".to_owned(),
        }
    }

    fn checkpoint_record_config() -> RecordCollectorCheckpointConfig {
        RecordCollectorCheckpointConfig {
            collector_kind: "btc-chain-head".to_owned(),
            partition: "chain-head".to_owned(),
        }
    }

    #[test]
    fn chain_head_fact_descriptor_declares_platform_subject_and_result_fields() {
        let descriptor = BtcChainHeadFact::descriptor().expect("descriptor");

        assert_eq!(descriptor.fact_kind().as_str(), "chain.head");
        assert!(descriptor.fields().iter().any(|field| {
            field.field_id().as_str() == "subject.semantic_source_identity"
                && matches!(field.extraction(), FactFieldExtraction::Subject(_))
                && field.exposure() == FactFieldExposure::QueryOnly
        }));
        assert!(descriptor.fields().iter().any(|field| {
            field.field_id().as_str() == "result.block_height"
                && field.value_type() == FactFieldValueType::UnsignedInteger
                && field.sortable()
        }));
        assert!(descriptor.fields().iter().any(|field| {
            field.field_id().as_str() == "metadata.observed_at"
                && matches!(field.extraction(), FactFieldExtraction::Metadata(_))
                && !field.required()
        }));
        assert_eq!(
            descriptor.orderings()[0].name().as_str(),
            "result.block_height.desc"
        );
    }

    #[test]
    fn collector_checkpoint_descriptor_declares_control_subject_and_high_watermark() {
        let descriptor = CollectorCheckpointFact::descriptor().expect("descriptor");

        assert_eq!(descriptor.fact_kind().as_str(), "collector.checkpoint");
        for field_id in [
            "subject.collector_kind",
            "subject.semantic_source_identity",
            "subject.scope",
            "subject.partition",
            "subject.chain",
            "subject.network",
        ] {
            assert!(
                descriptor
                    .fields()
                    .iter()
                    .any(|field| field.field_id().as_str() == field_id
                        && matches!(field.extraction(), FactFieldExtraction::Subject(_))),
                "missing subject field {field_id}"
            );
        }
        assert!(descriptor.fields().iter().any(|field| {
            field.field_id().as_str() == "result.high_watermark_height"
                && field.value_type() == FactFieldValueType::UnsignedInteger
                && field.exposure() == FactFieldExposure::Returnable
                && field.sortable()
        }));
        assert!(descriptor.fields().iter().any(|field| {
            field.field_id().as_str() == "result.predecessor_checkpoint_hash"
                && field.value_type() == FactFieldValueType::Digest
                && field.exposure() == FactFieldExposure::Hidden
                && !field.required()
        }));
    }

    #[test]
    fn normalized_chain_head_fact_material_contains_no_floats_or_runtime_routes() {
        let (request, response) = capability_response();
        let observation = normalize_chain_head_response(&request, &response, &observe_input(None))
            .expect("observation");
        let fact = observation.to_fact();
        let value = serde_json::to_value(&fact).expect("json");

        assert_no_float_numbers(&value);
        let text = serde_json::to_string(&value).expect("json text");
        for forbidden in [
            "rpc_url",
            "auth_header",
            "username",
            "password",
            "runtime_source_ref",
            "http://",
            "https://",
        ] {
            assert!(
                !text.contains(forbidden),
                "fact material leaked forbidden token {forbidden}: {text}"
            );
        }
        assert!(text.contains("public-bitcoin-core"));
    }

    #[test]
    fn checkpoint_query_request_is_control_latest_checkpoint_plan() {
        let config = checkpoint_query_config();
        let request = config.request().expect("request");
        let plan = request.plan();
        let descriptor_hash =
            fact_descriptor_hash(&CollectorCheckpointFact::descriptor().expect("descriptor"))
                .expect("descriptor hash");

        assert_eq!(plan.query_scope().audience(), FactAudience::Control);
        assert_eq!(plan.query_scope().scope(), FactVisibilityScope::Default);
        assert_eq!(plan.resolved_descriptor(), &descriptor_hash);
        assert_eq!(
            plan.ordering().name().as_str(),
            "result.high_watermark_height.desc"
        );
        assert_eq!(plan.limit(), Some(1));
        assert_eq!(plan.store_scope().as_str(), "mfm.store.default");

        let query: serde_json::Value =
            serde_json::from_slice(plan.canonical_query().as_bytes()).expect("query json");
        assert_eq!(query["fact_kind"], "collector.checkpoint");
        assert!(query.get("audience").is_none());
        assert!(query.get("scope").is_none());
        assert_eq!(query["limit"], 1);
        assert_eq!(query["ordering"], "result.high_watermark_height.desc");
        assert_eq!(query["return_fields"][0], "result.high_watermark_height");
        let parsed_shape = mfm_facts::parse_canonical_fact_query_shape(plan).expect("query shape");
        assert_eq!(parsed_shape.return_fields().len(), 1);
        assert!(query["predicates"]
            .as_array()
            .expect("predicates")
            .iter()
            .any(
                |predicate| predicate["field_id"] == "subject.semantic_source_identity"
                    && predicate["value"] == "public-bitcoin-core"
            ));
        assert!(query["predicates"]
            .as_array()
            .expect("predicates")
            .iter()
            .any(|predicate| predicate["field_id"] == "subject.partition"
                && predicate["value"] == "chain-head"));
    }

    #[test]
    fn checkpoint_query_config_rejects_runtime_routes() {
        let config = QueryCollectorCheckpointConfig {
            semantic_source_identity: "http://user:password@localhost:8332".to_owned(),
            ..checkpoint_query_config()
        };

        let error = validate_query_collector_checkpoint_config(&config).expect_err("route");
        assert!(!error.contains("localhost:8332"));
        assert!(!error.contains("password"));
    }

    #[test]
    fn checkpoint_query_materializes_empty_and_single_row_receipts() {
        let config = checkpoint_query_config();
        let state = QueryCollectorCheckpointState::new(
            ValidatedConfig::new(config.clone()).expect("validated config"),
        )
        .expect("state");
        let request = config.request().expect("request");

        let empty_response = FactIndexReadResponse::from_receipt(
            fact_query_receipt(request.plan(), Vec::new()),
            trust_root(),
        );
        let empty = state
            .materialize_response(&empty_response, None)
            .expect("empty materialization");
        assert!(empty.checkpoint().is_none());
        assert_eq!(
            state
                .selection_evidence(&empty_response)
                .expect("empty selection")
                .selected_indices(),
            &[] as &[u64]
        );

        let checkpoint = checkpoint_fact(850_000);
        let single_response = FactIndexReadResponse::from_receipt(
            fact_query_receipt(request.plan(), vec![internal_fact_ref(1)]),
            trust_root(),
        );
        let loaded = state
            .materialize_response(&single_response, Some(checkpoint.clone()))
            .expect("single materialization");
        assert_eq!(loaded.checkpoint(), Some(&checkpoint));
        assert_eq!(
            state
                .selection_evidence(&single_response)
                .expect("single selection")
                .selected_indices(),
            &[0]
        );
        assert!(state.materialize_response(&single_response, None).is_err());
    }

    #[test]
    fn observe_validates_checkpoint_source_compatibility() {
        let (_request, response) = capability_response();
        let state =
            ObserveBtcChainHeadState::new(ValidatedConfig::new(observe_config()).expect("config"))
                .expect("state");
        let incompatible = checkpoint_fact_for_source(
            849_999,
            "different-semantic-source",
            "bitcoin-mainnet",
            BtcFinality::BestAvailable,
        );

        let error = state
            .materialize_response(&observe_input(Some(incompatible)), &response)
            .expect_err("incompatible checkpoint");

        assert!(error
            .to_string()
            .contains("incompatible with requested Bitcoin source"));
    }

    #[test]
    fn observe_rejects_head_behind_loaded_checkpoint() {
        let (_request, response) = capability_response();
        let state =
            ObserveBtcChainHeadState::new(ValidatedConfig::new(observe_config()).expect("config"))
                .expect("state");

        let error = state
            .materialize_response(&observe_input(Some(checkpoint_fact(850_001))), &response)
            .expect_err("head behind checkpoint");

        assert!(error
            .to_string()
            .contains("observed chain head must not be behind loaded checkpoint"));
    }

    #[test]
    fn record_states_transform_typed_upstream_outputs() {
        let (request, response) = capability_response();
        let observation = normalize_chain_head_response(&request, &response, &observe_input(None))
            .expect("observation");

        let chain_head_state = RecordBtcChainHeadFactState::new(
            ValidatedConfig::new(RecordBtcChainHeadFactConfig {}).expect("config"),
        )
        .expect("state");
        let caps = (BtcFactRecordCapability,);
        let context = mfm_program::CertifiedContext::no_context();
        let fact = poll_ready(chain_head_state.run(
            RecordBtcChainHeadFactInput {
                observation: observation.clone(),
            },
            &caps,
            &context,
        ))
        .expect("chain head fact");
        assert_eq!(fact.subject(), observation.subject());
        assert_eq!(fact.response(), observation.response());

        let checkpoint_state = RecordCollectorCheckpointState::new(
            ValidatedConfig::new(checkpoint_record_config()).expect("config"),
        )
        .expect("state");
        let checkpoint = poll_ready(checkpoint_state.run(
            RecordCollectorCheckpointInput {
                chain_head_fact: fact.clone(),
                loaded_checkpoint: LoadedCollectorCheckpoint::new(None),
            },
            &caps,
            &context,
        ))
        .expect("checkpoint fact");

        assert_eq!(checkpoint.subject().collector_kind(), "btc-chain-head");
        assert_eq!(checkpoint.subject().partition(), "chain-head");
        assert_eq!(
            checkpoint.response().high_watermark_height(),
            observation.response().block_height()
        );
        assert_eq!(
            checkpoint.response().high_watermark_hash(),
            fact.response().block_hash()
        );
    }

    #[test]
    fn checkpoint_record_uses_recorded_fact_and_loaded_checkpoint() {
        let (request, response) = capability_response();
        let observation = normalize_chain_head_response(&request, &response, &observe_input(None))
            .expect("observation");
        let fact = observation.to_fact();
        let previous = checkpoint_fact(849_999);
        let state = RecordCollectorCheckpointState::new(
            ValidatedConfig::new(checkpoint_record_config()).expect("config"),
        )
        .expect("state");

        let caps = (BtcFactRecordCapability,);
        let context = mfm_program::CertifiedContext::no_context();
        let checkpoint = poll_ready(state.run(
            RecordCollectorCheckpointInput {
                chain_head_fact: fact,
                loaded_checkpoint: LoadedCollectorCheckpoint::new(Some(previous.clone())),
            },
            &caps,
            &context,
        ))
        .expect("checkpoint");

        assert_eq!(checkpoint.response().high_watermark_height(), 850_000);
        assert_eq!(
            checkpoint.response().predecessor_checkpoint_hash(),
            Some(checkpoint_material_hash(&previous).expect("hash").as_str())
        );
    }

    #[test]
    fn checkpoint_record_rejects_incompatible_loaded_checkpoint() {
        let (request, response) = capability_response();
        let observation = normalize_chain_head_response(&request, &response, &observe_input(None))
            .expect("observation");
        let state = RecordCollectorCheckpointState::new(
            ValidatedConfig::new(checkpoint_record_config()).expect("config"),
        )
        .expect("state");

        let caps = (BtcFactRecordCapability,);
        let context = mfm_program::CertifiedContext::no_context();
        let error = poll_ready(state.run(
            RecordCollectorCheckpointInput {
                chain_head_fact: observation.to_fact(),
                loaded_checkpoint: LoadedCollectorCheckpoint::new(Some(
                    checkpoint_fact_for_source(
                        849_999,
                        "different-semantic-source",
                        "bitcoin-mainnet",
                        BtcFinality::BestAvailable,
                    ),
                )),
            },
            &caps,
            &context,
        ))
        .expect_err("incompatible checkpoint");

        assert!(error
            .to_string()
            .contains("incompatible with recorded chain-head fact"));
    }

    #[test]
    fn checkpoint_material_contains_no_floats_or_runtime_routes() {
        let subject = CollectorCheckpointSubject::new(
            "btc-chain-head",
            &BtcSourceIdentity::new("public-bitcoin-core").expect("source"),
            "chain-head",
            BtcChain::Bitcoin,
            &BtcNetworkId::new("bitcoin-mainnet").expect("network"),
        );
        let response = CollectorCheckpointResponse::new(
            850_000,
            "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc",
            Some("internal-fact-ref".to_owned()),
            Some(
                "content:sha256-jcs-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_owned(),
            ),
            BtcFinality::BestAvailable,
        );
        let value =
            serde_json::to_value(CollectorCheckpointFact::new(subject, response)).expect("json");

        assert_no_float_numbers(&value);
        let text = serde_json::to_string(&value).expect("json text");
        assert!(!text.contains("http://"));
        assert!(!text.contains("password"));
        assert!(text.contains("\"scope\":\"default\""));
    }

    #[test]
    fn source_identity_rejects_runtime_route_strings() {
        let config = ObserveBtcChainHeadConfig {
            semantic_source_identity: "http://user:password@localhost:8332".to_owned(),
            ..observe_config()
        };

        let error = validate_observe_chain_head_config(&config).expect_err("route must fail");
        assert!(
            !error.contains("localhost:8332"),
            "unredacted error: {error}"
        );
    }

    #[test]
    fn state_error_from_capability_provider_failure_is_redacted() {
        let error = BtcStateError::from(BtcCapabilityError::redacted_provider_failure(
            "http://user:password@localhost:8332",
        ));
        let text = error.to_string();

        assert_eq!(error, BtcStateError::ProviderFailed);
        assert!(!text.contains("password"));
        assert!(!text.contains("localhost"));
        assert!(!text.contains("http://"));
    }

    #[test]
    fn record_states_advertise_only_their_fact_descriptors() {
        let observe =
            ObserveBtcChainHeadState::emitted_fact_descriptors().expect("observe descriptors");
        let query =
            QueryCollectorCheckpointState::emitted_fact_descriptors().expect("query descriptors");
        let chain_head = RecordBtcChainHeadFactState::emitted_fact_descriptors()
            .expect("chain head descriptors");
        let checkpoint = RecordCollectorCheckpointState::emitted_fact_descriptors()
            .expect("checkpoint descriptors");

        assert!(observe.is_empty());
        assert!(query.is_empty());
        assert_eq!(chain_head.len(), 1);
        assert_eq!(checkpoint.len(), 1);
        assert_ne!(chain_head[0].descriptor_hash, checkpoint[0].descriptor_hash);
    }

    #[test]
    fn query_state_declares_fact_index_read_capability() {
        let descriptor =
            mfm_program::registered_state_descriptor::<QueryCollectorCheckpointState>()
                .expect("descriptor");

        assert_eq!(descriptor.effect().class, ReadExternal::class());
        assert_eq!(descriptor.capabilities().len(), 1);
        assert_eq!(
            descriptor.capabilities().capabilities[0].name.as_str(),
            mfm_fact_capabilities::FactIndexReadCapability::name()
        );
    }

    #[test]
    fn adapter_bound_states_advertise_btc_jsonrpc_adapter() {
        let expected_kind = btc_jsonrpc_adapter_kind().expect("adapter kind");
        let expected_version = btc_jsonrpc_adapter_version().expect("adapter version");

        for bindings in [
            ObserveBtcChainHeadState::adapter_bindings().expect("observe bindings"),
            RecordBtcChainHeadFactState::adapter_bindings().expect("chain head record bindings"),
            RecordCollectorCheckpointState::adapter_bindings().expect("checkpoint record bindings"),
        ] {
            assert_eq!(bindings.len(), 1);
            assert_eq!(bindings[0].adapter_kind, expected_kind);
            assert_eq!(bindings[0].adapter_version, expected_version);
        }
    }

    #[test]
    fn state_crate_manifest_stays_inside_state_boundaries() {
        let manifest = include_str!("../Cargo.toml");

        for forbidden in [
            "mfm-app",
            "mfm-runtime",
            "mfm-store",
            "mfm-stream-store-postgres",
            "mfm-btc-jsonrpc-http",
            "mfm-adapters-btc-jsonrpc",
            "mfm-op-btc-chain-head-collector",
        ] {
            assert!(
                !manifest.contains(forbidden),
                "state crate must not depend on forbidden boundary crate {forbidden}"
            );
        }
    }

    fn checkpoint_fact(height: u64) -> CollectorCheckpointFact {
        checkpoint_fact_for_source(
            height,
            "public-bitcoin-core",
            "bitcoin-mainnet",
            BtcFinality::BestAvailable,
        )
    }

    fn checkpoint_fact_for_source(
        height: u64,
        source_identity: &str,
        network: &str,
        finality: BtcFinality,
    ) -> CollectorCheckpointFact {
        let subject = CollectorCheckpointSubject::new(
            "btc-chain-head",
            &BtcSourceIdentity::new(source_identity).expect("source"),
            "chain-head",
            BtcChain::Bitcoin,
            &BtcNetworkId::new(network).expect("network"),
        );
        let response = CollectorCheckpointResponse::new(
            height,
            "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc",
            None,
            None,
            finality,
        );
        CollectorCheckpointFact::new(subject, response)
    }

    fn fact_query_receipt(
        plan: &CanonicalFactQueryPlan,
        returned_refs: Vec<InternalFactRef>,
    ) -> FactQueryReceipt {
        let rows = returned_refs
            .into_iter()
            .map(|fact_ref| mfm_facts::FactQueryResultRow::new(fact_ref, Vec::new()))
            .collect::<Vec<_>>();
        let read_frontier = StoreReadFrontier::new(
            StoreScopeRef::new("mfm.store.default").expect("store scope"),
            FactQueryScope::new(FactAudience::Control, FactVisibilityScope::Default),
            DescriptorCatalogWatermark::new(1),
            FactProjectionGeneration::new(1),
            11,
            StoreCommitWatermark::new(11),
        );
        let plan_hash = fact_query_plan_hash(plan).expect("plan hash");
        let material = FactQueryReceiptMaterial::from_rows(
            &plan_hash,
            read_frontier,
            StoreReadFrontierType::Snapshot,
            &rows,
            false,
            None,
        )
        .expect("receipt material");
        material.into_receipt(
            StoreReceiptAuthentication::new(
                StoreIdentity::new("store.default").expect("store identity"),
                StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
                Some(StoreKeyId::new("fact.read.key").expect("key id")),
                vec![0x11; 64],
            )
            .expect("auth"),
        )
    }

    fn trust_root() -> mfm_fact_capabilities::FactQueryReceiptTrustRootMaterial {
        mfm_fact_capabilities::FactQueryReceiptTrustRootMaterial::new(
            StoreIdentity::new("store.default").expect("store identity"),
            StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
            StoreKeyId::new("fact.read.key").expect("key id"),
            [7_u8; 32],
        )
    }

    fn internal_fact_ref(seed: u8) -> InternalFactRef {
        InternalFactRef::new(InternalFactRefParts {
            fact_claim_id: FactClaimId::new(run_id(seed), 9, 0).expect("claim id"),
            source_event_id: EventId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(seed + 1),
            ),
            recorded_at: "2026-07-02T00:00:00Z".to_owned(),
            producer_node_id: mfm_ids::NodeId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(seed + 2),
            ),
            observed_at: Some("2026-07-02T00:00:00Z".to_owned()),
            visibility: FactVisibility::indexed_default(FactAudience::Control),
            fact_kind: mfm_facts::FactKind::new("collector.checkpoint").expect("kind"),
            fact_descriptor_hash: digest(seed + 2),
            subject: FactSubjectRef::new(
                digest(seed + 3),
                mfm_facts::FactKey::from_digest(digest(seed + 4)),
                digest(seed + 5),
            ),
            request: None,
            response: FactResponseEvidence::new(
                schema_id(seed + 6),
                digest(seed + 7),
                ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed + 8)),
                digest(seed + 9),
            ),
            producer: FactProducerProvenance::new(
                CapabilityKind::new(
                    "mfm.fact",
                    "index.read",
                    DigestAlgorithm::Sha256JcsV1,
                    digest_bytes(seed + 10),
                )
                .expect("capability kind"),
                CapabilityVersion::new("mfm.fact.index.read.v1").expect("capability version"),
                AdapterKind::new(
                    "mfm.fact",
                    "index.adapter",
                    DigestAlgorithm::Sha256JcsV1,
                    digest_bytes(seed + 11),
                )
                .expect("adapter kind"),
                AdapterVersion::new("mfm.fact.index.adapter.v1").expect("adapter version"),
            ),
        })
        .expect("fact ref")
    }

    fn schema_id(seed: u8) -> SchemaId {
        SchemaId::new(
            "mfm.fact.test",
            "v1",
            DigestAlgorithm::Sha256JcsV1,
            digest_bytes(seed),
        )
        .expect("schema id")
    }

    fn run_id(seed: u8) -> RunId {
        RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed))
    }

    fn digest(seed: u8) -> ContentDigest {
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(seed))
    }

    fn digest_bytes(seed: u8) -> DigestBytes {
        DigestBytes::from_array([seed; 32])
    }

    fn assert_no_float_numbers(value: &serde_json::Value) {
        match value {
            serde_json::Value::Array(values) => {
                for value in values {
                    assert_no_float_numbers(value);
                }
            }
            serde_json::Value::Object(values) => {
                for value in values.values() {
                    assert_no_float_numbers(value);
                }
            }
            serde_json::Value::Number(number) => {
                assert!(!number.is_f64(), "float-like number found: {number}");
            }
            serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::String(_) => {
            }
        }
    }
}
