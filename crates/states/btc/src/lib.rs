#![warn(missing_docs)]
//! Reusable Bitcoin balance-collection state contracts.
//!
//! This crate owns typed Bitcoin balance facts and state-layer contracts used to resolve a shared
//! internal tip and collect exact-anchor address balance snapshots. It defines no JSON-RPC
//! transport, runtime source routing, workflow topology, CLI, REST, or app registration.

mod address_balance;
mod address_balance_collect;
mod external_read;

pub use address_balance::{
    address_balance_fact_visibility, normalize_btc_address_balance,
    normalize_btc_address_balance_fact, BtcAddressBalanceResponse, BtcAddressBalanceSnapshotFact,
    BtcAddressBalanceSubject, NormalizedBtcAddressHolding,
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

pub use external_read::{
    BtcAddressBalanceReadEvidence, BtcAddressBalanceReadPlan, BtcChainHeadReadEvidence,
    BtcChainHeadReadPlan,
};

use mfm_btc_capabilities::{BitcoinNetworkTag, BtcCapabilityError};
use mfm_canonical::sha256_digest_bytes;
use mfm_ids::{AdapterKind, AdapterVersion, DigestAlgorithm, StateKind, StateVersion};
use mfm_program::{AdapterBindingSpec, StateError};

const NAMESPACE: &str = "mfm.bitcoin";
const BTC_JSONRPC_ADAPTER_NAME: &str = "jsonrpc";
const BTC_JSONRPC_ADAPTER_VERSION: &str = "mfm.bitcoin.jsonrpc.adapter.v1";

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

fn validate_bitcoin_network(value: &str) -> Result<(), String> {
    BitcoinNetworkTag::new(value)
        .map(|_| ())
        .map_err(|_| "bitcoin_network must name a supported Bitcoin Core chain".to_owned())
}

#[cfg(test)]
mod tests;
