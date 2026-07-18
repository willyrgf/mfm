#![warn(missing_docs)]
//! Reusable Bitcoin fact state contracts.
//!
//! This crate owns typed Bitcoin facts and state-layer contracts used to observe bounded
//! chain-head data, address balance snapshots, and collector checkpoints. It defines no
//! JSON-RPC transport, runtime source routing, workflow topology, CLI, REST, or app
//! registration.

mod address_balance;
mod address_balance_collect;
mod chain_head;
mod collector_checkpoint;
mod external_read;

pub use address_balance::{
    address_balance_fact_visibility, normalize_btc_address_balance,
    normalize_btc_address_balance_fact, platform_address_balance_at_anchor_plan,
    BtcAddressBalanceResponse, BtcAddressBalanceSnapshotFact, BtcAddressBalanceSubject,
    NormalizedBtcAddressHolding,
};
pub use address_balance_collect::{
    address_balance_record_visibility, assemble_btc_network_collection_receipt,
    materialize_btc_joint_tip, normalize_btc_address_balance_observation,
    validate_observe_btc_address_balance_config, validate_resolve_btc_joint_tip_config,
    AssembleBtcNetworkCollectionReceiptConfig, AssembleBtcNetworkCollectionReceiptInput,
    AssembleBtcNetworkCollectionReceiptInputHandles, AssembleBtcNetworkCollectionReceiptState,
    BtcAddressBalanceObservation, BtcJointTip, BtcNativeBalanceReceiptEntry,
    BtcNativeBalanceSourceKey, BtcNetworkCollectionReceipt, ObserveBtcAddressBalanceConfig,
    ObserveBtcAddressBalanceInput, ObserveBtcAddressBalanceInputHandles,
    ObserveBtcAddressBalanceState, RecordBtcAddressBalanceFactConfig,
    RecordBtcAddressBalanceFactInput, RecordBtcAddressBalanceFactInputHandles,
    RecordBtcAddressBalanceFactState, ResolveBtcJointTipConfig, ResolveBtcJointTipInput,
    ResolveBtcJointTipInputHandles, ResolveBtcJointTipState, BTC_JOINT_TIP_SOURCE_READS,
    BTC_NATIVE_BALANCE_COVERAGE, BTC_NATIVE_BALANCE_OBSERVE_SOURCE_READS,
    BTC_NATIVE_BALANCE_SOURCE_STATUS,
};

pub use chain_head::{
    normalize_chain_head_response, validate_observe_chain_head_config, BtcChainHeadFact,
    BtcChainHeadObservation, BtcChainHeadObservationContext, BtcChainHeadResponse,
    BtcChainHeadSubject, ObserveBtcChainHeadConfig, ObserveBtcChainHeadInput,
    ObserveBtcChainHeadInputHandles, ObserveBtcChainHeadState, RecordBtcChainHeadFactConfig,
    RecordBtcChainHeadFactInput, RecordBtcChainHeadFactInputHandles, RecordBtcChainHeadFactState,
};
pub use collector_checkpoint::{
    record_collector_checkpoint_from_outputs, validate_query_collector_checkpoint_config,
    validate_record_collector_checkpoint_config, CollectorCheckpointFact,
    CollectorCheckpointResponse, CollectorCheckpointSubject, LoadedCollectorCheckpoint,
    QueryCollectorCheckpointConfig, QueryCollectorCheckpointInput,
    QueryCollectorCheckpointInputHandles, QueryCollectorCheckpointReadEvidence,
    QueryCollectorCheckpointReadPlan, QueryCollectorCheckpointState,
    RecordCollectorCheckpointConfig, RecordCollectorCheckpointInput,
    RecordCollectorCheckpointInputHandles, RecordCollectorCheckpointState,
};
pub use external_read::{
    BtcAddressBalanceReadEvidence, BtcAddressBalanceReadPlan, BtcChainHeadReadEvidence,
    BtcChainHeadReadPlan,
};

use std::future;
use std::num::NonZeroU64;

use mfm_btc_capabilities::{
    BtcCapabilityError, BtcChainHeadReadCapability,
    BtcChainHeadResponse as CapabilityChainHeadResponse, BtcFinality, BtcHeadKind,
    BtcHeadSelection, BtcNetworkId, BtcSourceIdentity, BtcSourceStatus,
};
use mfm_canonical::sha256_digest_bytes;
use mfm_effects::{ManagedPlatformWrite, ReadExternal};
use mfm_fact_capabilities::{FactIndexReadCapability, FactIndexReadRequest, FactRecordCapability};
use mfm_facts::{
    compile_fact_query_plan, FactAudience, FactCanonicalScalar, FactFieldId, FactOrderingName,
    FactQueryInput, FactQueryOperator, FactQueryPredicate, FactQueryScope, FactSelectionEvidence,
    FactVisibility, FactVisibilityScope, ScopeDecisionEvidence, StoreScopeRef,
};
use mfm_ids::{AdapterKind, AdapterVersion, ContentDigest};
use mfm_ids::{DigestAlgorithm, StateKind, StateVersion};
use mfm_program::{
    fact_descriptor_ref, AdapterBindingSpec, CanonicalSeed, ExternalReadEvidenceSet,
    FactDescriptorRef, ManagedWriteState, MfmFactType, NoContext, ReadState, StateError,
    StateResult, StateSpec, ValidatedConfig,
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

/// Redaction-safe state error for Bitcoin fact normalization contracts.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BtcStateError {
    /// A state contract input was invalid.
    #[error("Bitcoin state input was invalid: {reason}")]
    InvalidInput {
        /// Stable redacted reason.
        reason: String,
    },
    /// A capability provider reported source mismatch.
    #[error("Bitcoin capability provider reported source mismatch")]
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

fn checkpoint_material_hash(
    checkpoint: &CollectorCheckpointFact,
) -> Result<ContentDigest, BtcStateError> {
    CanonicalSeed::from_value(checkpoint)
        .map(|seed| seed.content_digest().clone())
        .map_err(|error| BtcStateError::InvalidInput {
            reason: format!("checkpoint material canonicalization failed: {error}"),
        })
}

fn validate_bitcoin_network(value: &str) -> Result<(), String> {
    match value {
        "main" | "test" | "signet" | "regtest" => Ok(()),
        _ => Err("bitcoin_network must be `main`, `test`, `signet`, or `regtest`".to_owned()),
    }
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
mod tests;
