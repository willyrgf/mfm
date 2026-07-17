#![warn(missing_docs)]
//! Reusable EVM holding fact state contracts.
//!
//! This crate owns typed EVM native and ERC-20 balance snapshot facts used by portfolio
//! collectors and report selection, plus source-near observe/record states. It defines no
//! JSON-RPC transport, runtime source routing, workflow topology, CLI, REST, or app registration.
//!
//! Generic EVM transaction and exact-anchor validation states will replace the
//! deleted fixed contract lifecycle in this crate.
//!
//! # Examples
//!
//! ```rust
//! use mfm_states_evm::EvmAddressErc20BalanceSubject;
//!
//! let subject = EvmAddressErc20BalanceSubject::new(
//!     "ethereum-mainnet",
//!     1,
//!     "0x0000000000000000000000000000000000000001",
//!     "0x0000000000000000000000000000000000000002",
//! )?;
//! assert_eq!(subject.contract_address(), "0x0000000000000000000000000000000000000001");
//! # Ok::<(), mfm_states_evm::EvmStateError>(())
//! ```

mod erc20_balance_collect;
mod native_balance_collect;
mod network_collection_receipt;
mod source_binding;

pub use erc20_balance_collect::{
    assemble_evm_erc20_balance_batch_receipt, erc20_balance_call_request,
    erc20_balance_record_visibility, erc20_metadata_call_request,
    normalize_erc20_balance_from_capability, normalize_erc20_token_metadata_from_capability,
    validate_observe_erc20_balance_config, validate_observe_erc20_token_metadata_config,
    AssembleEvmErc20BalanceBatchReceiptConfig, AssembleEvmErc20BalanceBatchReceiptInput,
    AssembleEvmErc20BalanceBatchReceiptInputHandles, AssembleEvmErc20BalanceBatchReceiptState,
    EvmErc20BalanceBatchReceipt, EvmErc20BalanceReceiptEntry, EvmErc20BalanceSourceKey,
    EvmErc20TokenMetadata, ObserveErc20BalanceConfig, ObserveErc20BalanceInput,
    ObserveErc20BalanceInputHandles, ObserveErc20BalanceState, ObserveErc20TokenMetadataConfig,
    ObserveErc20TokenMetadataInput, ObserveErc20TokenMetadataInputHandles,
    ObserveErc20TokenMetadataState, RecordErc20BalanceFactConfig, RecordErc20BalanceFactInput,
    RecordErc20BalanceFactInputHandles, RecordErc20BalanceFactState, EVM_ERC20_BALANCE_COVERAGE,
    EVM_ERC20_BALANCE_OBSERVE_SOURCE_READS, EVM_ERC20_BALANCE_SOURCE_STATUS,
    EVM_ERC20_METADATA_OBSERVE_SOURCE_READS,
};

pub use native_balance_collect::{
    assemble_evm_native_balance_batch_receipt, evm_jsonrpc_adapter_kind,
    evm_jsonrpc_adapter_version, materialize_evm_joint_tip, native_balance_record_visibility,
    normalize_evm_native_balance_from_capability, normalize_evm_native_balance_observation,
    validate_observe_evm_native_balance_config, validate_resolve_evm_joint_tip_config,
    AssembleEvmNativeBalanceBatchReceiptConfig, AssembleEvmNativeBalanceBatchReceiptInput,
    AssembleEvmNativeBalanceBatchReceiptInputHandles, AssembleEvmNativeBalanceBatchReceiptState,
    EvmAddressNativeBalanceObservation, EvmJointTip, EvmNativeBalanceBatchReceipt,
    EvmNativeBalanceReceiptEntry, EvmNativeBalanceSourceKey, ObserveEvmNativeBalanceConfig,
    ObserveEvmNativeBalanceInput, ObserveEvmNativeBalanceInputHandles,
    ObserveEvmNativeBalanceState, RecordEvmNativeBalanceFactConfig,
    RecordEvmNativeBalanceFactInput, RecordEvmNativeBalanceFactInputHandles,
    RecordEvmNativeBalanceFactState, ResolveEvmJointTipConfig, ResolveEvmJointTipInput,
    ResolveEvmJointTipInputHandles, ResolveEvmJointTipState, EVM_JOINT_TIP_SOURCE_READS,
    EVM_NATIVE_BALANCE_COVERAGE, EVM_NATIVE_BALANCE_OBSERVE_SOURCE_READS,
    EVM_NATIVE_BALANCE_SOURCE_STATUS,
};

pub use network_collection_receipt::{
    assemble_evm_network_collection_receipt,
    validate_assemble_evm_network_collection_receipt_config,
    AssembleEvmNetworkCollectionReceiptConfig, AssembleEvmNetworkCollectionReceiptInput,
    AssembleEvmNetworkCollectionReceiptInputHandles, AssembleEvmNetworkCollectionReceiptState,
    EvmNetworkCollectionReceipt,
};

pub use source_binding::RedactedEvmProviderSourceBinding;

use std::str::FromStr;

use alloy_primitives::{B256, U256};
use mfm_evm_core::encoding::normalize_address;
use mfm_facts::{
    compile_fact_query_plan, CanonicalFactQueryPlan, FactAudience, FactCanonicalScalar,
    FactFieldId, FactOrderingName, FactQueryInput, FactQueryOperator, FactQueryPredicate,
    FactQueryScope, FactVisibility, FactVisibilityScope, ScopeDecisionEvidence, StoreScopeRef,
};
use mfm_portfolio_model::holding::{CoverageStatus, HoldingSourceStatus};
use mfm_program::StateError;
use mfm_program_derive::{MfmFactType as DeriveMfmFactType, MfmValue};
use serde::{Deserialize, Serialize};

/// Redaction-safe state error for EVM holding fact contracts.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvmStateError {
    /// A state contract input was invalid.
    #[error("EVM state input was invalid: {reason}")]
    InvalidInput {
        /// Stable redacted reason.
        reason: String,
    },
}

impl From<EvmStateError> for StateError {
    fn from(error: EvmStateError) -> Self {
        StateError::Message(error.to_string())
    }
}

/// Returns Platform visibility for EVM native address balance snapshot facts.
pub fn address_native_balance_fact_visibility() -> FactVisibility {
    FactVisibility::indexed_default(FactAudience::Platform)
}

/// Subject identity for an EVM native address balance snapshot fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "address_native_balance_subject",
    version = "1",
    schema = "mfm.evm.fact.address_native_balance.subject"
)]
pub struct EvmAddressNativeBalanceSubject {
    network: String,
    chain_id: u64,
    account: String,
}

impl EvmAddressNativeBalanceSubject {
    /// Creates subject material for a native balance snapshot.
    pub fn new(
        network: impl Into<String>,
        chain_id: u64,
        account: impl Into<String>,
    ) -> Result<Self, EvmStateError> {
        let network = network.into();
        let account = account.into();
        if network.trim().is_empty() || account.trim().is_empty() {
            return Err(EvmStateError::InvalidInput {
                reason: "native balance subject fields must be non-empty".to_owned(),
            });
        }
        if chain_id == 0 {
            return Err(EvmStateError::InvalidInput {
                reason: "chain_id must be non-zero".to_owned(),
            });
        }
        validate_canonical_evm_account(&account)?;
        Ok(Self {
            network,
            chain_id,
            account,
        })
    }

    /// Returns the semantic network id.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Returns the EVM chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the observed account address.
    pub fn account(&self) -> &str {
        &self.account
    }
}

pub(crate) fn validate_canonical_evm_account(account: &str) -> Result<(), EvmStateError> {
    let normalized = normalize_address(account).map_err(|_| EvmStateError::InvalidInput {
        reason: "account must be a 20-byte hex address".to_owned(),
    })?;
    if normalized != account {
        return Err(EvmStateError::InvalidInput {
            reason: "account must be a normalized lowercase 0x-prefixed EVM address".to_owned(),
        });
    }
    Ok(())
}

pub(crate) fn validate_canonical_erc20_contract_address(
    contract_address: &str,
) -> Result<(), EvmStateError> {
    validate_canonical_evm_account(contract_address)?;
    if contract_address == "0x0000000000000000000000000000000000000000" {
        return Err(EvmStateError::InvalidInput {
            reason: "erc20 contract_address must not be the zero address".to_owned(),
        });
    }
    Ok(())
}

fn validate_canonical_erc20_raw_units(raw_units: &str) -> Result<(), EvmStateError> {
    if raw_units.is_empty()
        || !raw_units
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return Err(EvmStateError::InvalidInput {
            reason: "raw_units must be a non-empty decimal digit string".to_owned(),
        });
    }
    let value = U256::from_str(raw_units).map_err(|_| EvmStateError::InvalidInput {
        reason: "raw_units must fit in unsigned 256-bit range".to_owned(),
    })?;
    if value.to_string() != raw_units {
        return Err(EvmStateError::InvalidInput {
            reason: "raw_units must be a canonical decimal digit string".to_owned(),
        });
    }
    Ok(())
}

/// Observed result for an EVM native address balance snapshot fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "address_native_balance_response",
    version = "1",
    schema = "mfm.evm.fact.address_native_balance.response"
)]
pub struct EvmAddressNativeBalanceResponse {
    block_number: u64,
    block_hash: String,
    raw_wei: String,
    decimals: u8,
    coverage: String,
    source_status: String,
}

impl EvmAddressNativeBalanceResponse {
    /// Creates response material, failing closed on malformed hash or inadmissible coverage/status.
    pub fn new(
        block_number: u64,
        block_hash: impl Into<String>,
        raw_wei: impl Into<String>,
        decimals: u8,
        coverage: CoverageStatus,
        source_status: HoldingSourceStatus,
    ) -> Result<Self, EvmStateError> {
        let block_hash = canonical_evm_block_hash(block_hash)?;
        let raw_wei = raw_wei.into();
        if raw_wei.trim().is_empty() || !raw_wei.chars().all(|c| c.is_ascii_digit()) {
            return Err(EvmStateError::InvalidInput {
                reason: "raw_wei must be a non-empty decimal digit string".to_owned(),
            });
        }
        if !coverage.is_admissible_for_write() {
            return Err(EvmStateError::InvalidInput {
                reason: format!(
                    "coverage {} is not admissible for Platform write",
                    coverage.as_str()
                ),
            });
        }
        if !source_status.is_admissible_for_write() {
            return Err(EvmStateError::InvalidInput {
                reason: format!(
                    "source_status {} is not admissible for Platform write",
                    source_status.as_str()
                ),
            });
        }
        Ok(Self {
            block_number,
            block_hash,
            raw_wei,
            decimals,
            coverage: coverage.as_str().to_owned(),
            source_status: source_status.as_str().to_owned(),
        })
    }

    /// Returns the mandatory block number.
    pub const fn block_number(&self) -> u64 {
        self.block_number
    }

    /// Returns the mandatory block hash.
    pub fn block_hash(&self) -> &str {
        &self.block_hash
    }

    /// Returns the balance in wei as a decimal string.
    pub fn raw_wei(&self) -> &str {
        &self.raw_wei
    }

    /// Returns native token decimals (typically 18).
    pub const fn decimals(&self) -> u8 {
        self.decimals
    }

    /// Returns the coverage tag.
    pub fn coverage(&self) -> &str {
        &self.coverage
    }

    /// Returns the holding source status tag.
    pub fn source_status(&self) -> &str {
        &self.source_status
    }

    /// Parses coverage as a closed enum.
    pub fn coverage_status(&self) -> Result<CoverageStatus, EvmStateError> {
        self.coverage
            .parse()
            .map_err(|_| EvmStateError::InvalidInput {
                reason: format!("unknown coverage status {:?}", self.coverage),
            })
    }

    /// Parses holding source status as a closed enum.
    pub fn holding_source_status(&self) -> Result<HoldingSourceStatus, EvmStateError> {
        self.source_status
            .parse()
            .map_err(|_| EvmStateError::InvalidInput {
                reason: format!("unknown holding source status {:?}", self.source_status),
            })
    }
}

/// Platform fact for an EVM native address balance snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, DeriveMfmFactType)]
#[allow(clippy::duplicated_attributes)]
#[mfm(
    namespace = "mfm.evm",
    name = "address_native_balance_snapshot_fact",
    version = "1",
    schema = "mfm.evm.fact.address_native_balance_snapshot"
)]
#[mfm_fact(kind = "evm.address_native_balance_snapshot")]
#[mfm_fact(field(
    id = "subject.network",
    source = "subject",
    path = "network",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "subject.chain_id",
    source = "subject",
    path = "chain_id",
    value_type = "unsigned_integer",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "subject.account",
    source = "subject",
    path = "account",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.block_number",
    source = "result",
    path = "block_number",
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
    id = "result.raw_wei",
    source = "result",
    path = "raw_wei",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.decimals",
    source = "result",
    path = "decimals",
    value_type = "unsigned_integer",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.coverage",
    source = "result",
    path = "coverage",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.source_status",
    source = "result",
    path = "source_status",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "metadata.store_commit_order",
    source = "metadata",
    metadata = "store_commit_order",
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
#[mfm_fact(ordering(
    name = "result.block_number.desc",
    term(
        field = "result.block_number",
        direction = "descending",
        nulls = "last"
    )
))]
#[mfm_fact(ordering(
    name = "metadata.store_commit_order.desc",
    term(
        field = "metadata.store_commit_order",
        direction = "descending",
        nulls = "last"
    )
))]
pub struct EvmAddressNativeBalanceSnapshotFact {
    subject: EvmAddressNativeBalanceSubject,
    response: EvmAddressNativeBalanceResponse,
}

impl EvmAddressNativeBalanceSnapshotFact {
    /// Creates a native balance snapshot fact from validated subject and response material.
    pub const fn new(
        subject: EvmAddressNativeBalanceSubject,
        response: EvmAddressNativeBalanceResponse,
    ) -> Self {
        Self { subject, response }
    }

    /// Creates a fact, re-validating response admission rules for fail-closed writers.
    pub fn try_new(
        subject: EvmAddressNativeBalanceSubject,
        block_number: u64,
        block_hash: impl Into<String>,
        raw_wei: impl Into<String>,
        decimals: u8,
        coverage: CoverageStatus,
        source_status: HoldingSourceStatus,
    ) -> Result<Self, EvmStateError> {
        let response = EvmAddressNativeBalanceResponse::new(
            block_number,
            block_hash,
            raw_wei,
            decimals,
            coverage,
            source_status,
        )?;
        Ok(Self::new(subject, response))
    }

    /// Returns the subject material.
    pub const fn subject(&self) -> &EvmAddressNativeBalanceSubject {
        &self.subject
    }

    /// Returns the response material.
    pub const fn response(&self) -> &EvmAddressNativeBalanceResponse {
        &self.response
    }
}

/// Returns Platform visibility for EVM ERC-20 address balance snapshot facts.
pub fn address_erc20_balance_fact_visibility() -> FactVisibility {
    FactVisibility::indexed_default(FactAudience::Platform)
}

/// Subject identity for an EVM ERC-20 address balance snapshot fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "address_erc20_balance_subject",
    version = "1",
    schema = "mfm.evm.fact.address_erc20_balance.subject"
)]
pub struct EvmAddressErc20BalanceSubject {
    network: String,
    chain_id: u64,
    contract_address: String,
    account: String,
}

impl EvmAddressErc20BalanceSubject {
    /// Creates checked subject material for an ERC-20 address balance snapshot.
    pub fn new(
        network: impl Into<String>,
        chain_id: u64,
        contract_address: impl Into<String>,
        account: impl Into<String>,
    ) -> Result<Self, EvmStateError> {
        let network = network.into();
        let contract_address = contract_address.into();
        let account = account.into();
        if network.trim().is_empty() {
            return Err(EvmStateError::InvalidInput {
                reason: "erc20 balance subject network must be non-empty".to_owned(),
            });
        }
        if chain_id == 0 {
            return Err(EvmStateError::InvalidInput {
                reason: "chain_id must be non-zero".to_owned(),
            });
        }
        validate_canonical_erc20_contract_address(&contract_address)?;
        validate_canonical_evm_account(&account)?;
        Ok(Self {
            network,
            chain_id,
            contract_address,
            account,
        })
    }

    /// Returns the semantic network id.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Returns the EVM chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the canonical non-zero ERC-20 contract address.
    pub fn contract_address(&self) -> &str {
        &self.contract_address
    }

    /// Returns the canonical observed account address.
    pub fn account(&self) -> &str {
        &self.account
    }
}

/// Observed result for an EVM ERC-20 address balance snapshot fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "address_erc20_balance_response",
    version = "1",
    schema = "mfm.evm.fact.address_erc20_balance.response"
)]
pub struct EvmAddressErc20BalanceResponse {
    block_number: u64,
    block_hash: String,
    raw_units: String,
    decimals: u8,
    coverage: String,
    source_status: String,
}

impl EvmAddressErc20BalanceResponse {
    /// Creates a checked response with the state-owned complete/ok semantics.
    pub fn new(
        block_number: u64,
        block_hash: impl Into<String>,
        raw_units: impl Into<String>,
        decimals: u8,
    ) -> Result<Self, EvmStateError> {
        let block_hash = canonical_evm_block_hash(block_hash)?;
        let raw_units = raw_units.into();
        validate_canonical_erc20_raw_units(&raw_units)?;
        Ok(Self {
            block_number,
            block_hash,
            raw_units,
            decimals,
            coverage: EVM_ERC20_BALANCE_COVERAGE.to_owned(),
            source_status: EVM_ERC20_BALANCE_SOURCE_STATUS.to_owned(),
        })
    }

    /// Returns the mandatory block number.
    pub const fn block_number(&self) -> u64 {
        self.block_number
    }

    /// Returns the canonical mandatory block hash.
    pub fn block_hash(&self) -> &str {
        &self.block_hash
    }

    /// Returns the complete unsigned balance as a canonical decimal digit string.
    pub fn raw_units(&self) -> &str {
        &self.raw_units
    }

    /// Returns token decimals observed at this exact anchor.
    pub const fn decimals(&self) -> u8 {
        self.decimals
    }

    /// Returns the closed coverage tag, always `complete_at_anchor`.
    pub fn coverage(&self) -> &str {
        &self.coverage
    }

    /// Returns the closed source-status tag, always `ok`.
    pub fn source_status(&self) -> &str {
        &self.source_status
    }

    /// Returns the closed complete-at-anchor coverage status after validating wire material.
    pub fn coverage_status(&self) -> Result<CoverageStatus, EvmStateError> {
        if self.coverage != EVM_ERC20_BALANCE_COVERAGE {
            return Err(EvmStateError::InvalidInput {
                reason: "erc20 balance coverage must be complete_at_anchor".to_owned(),
            });
        }
        Ok(CoverageStatus::CompleteAtAnchor)
    }

    /// Returns the closed successful source status after validating wire material.
    pub fn holding_source_status(&self) -> Result<HoldingSourceStatus, EvmStateError> {
        if self.source_status != EVM_ERC20_BALANCE_SOURCE_STATUS {
            return Err(EvmStateError::InvalidInput {
                reason: "erc20 balance source_status must be ok".to_owned(),
            });
        }
        Ok(HoldingSourceStatus::Ok)
    }

    fn revalidated(&self) -> Result<Self, EvmStateError> {
        let rebuilt = Self::new(
            self.block_number,
            self.block_hash.clone(),
            self.raw_units.clone(),
            self.decimals,
        )?;
        if rebuilt != *self {
            return Err(EvmStateError::InvalidInput {
                reason: "erc20 balance response did not satisfy closed state semantics".to_owned(),
            });
        }
        Ok(rebuilt)
    }
}

/// Platform fact for an EVM ERC-20 address balance snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, DeriveMfmFactType)]
#[allow(clippy::duplicated_attributes)]
#[mfm(
    namespace = "mfm.evm",
    name = "address_erc20_balance_snapshot_fact",
    version = "1",
    schema = "mfm.evm.fact.address_erc20_balance_snapshot"
)]
#[mfm_fact(kind = "evm.address_erc20_balance_snapshot")]
#[mfm_fact(field(
    id = "subject.network",
    source = "subject",
    path = "network",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "subject.chain_id",
    source = "subject",
    path = "chain_id",
    value_type = "unsigned_integer",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "subject.contract_address",
    source = "subject",
    path = "contract_address",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "subject.account",
    source = "subject",
    path = "account",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.block_number",
    source = "result",
    path = "block_number",
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
    id = "result.raw_units",
    source = "result",
    path = "raw_units",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.decimals",
    source = "result",
    path = "decimals",
    value_type = "unsigned_integer",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.coverage",
    source = "result",
    path = "coverage",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.source_status",
    source = "result",
    path = "source_status",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "metadata.store_commit_order",
    source = "metadata",
    metadata = "store_commit_order",
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
#[mfm_fact(ordering(
    name = "result.block_number.desc",
    term(
        field = "result.block_number",
        direction = "descending",
        nulls = "last"
    )
))]
#[mfm_fact(ordering(
    name = "metadata.store_commit_order.desc",
    term(
        field = "metadata.store_commit_order",
        direction = "descending",
        nulls = "last"
    )
))]
pub struct EvmAddressErc20BalanceSnapshotFact {
    subject: EvmAddressErc20BalanceSubject,
    response: EvmAddressErc20BalanceResponse,
}

impl EvmAddressErc20BalanceSnapshotFact {
    /// Creates a fact from already checked subject and response material.
    pub const fn new(
        subject: EvmAddressErc20BalanceSubject,
        response: EvmAddressErc20BalanceResponse,
    ) -> Self {
        Self { subject, response }
    }

    /// Creates a fact while enforcing the closed ERC-20 response semantics.
    pub fn try_new(
        subject: EvmAddressErc20BalanceSubject,
        block_number: u64,
        block_hash: impl Into<String>,
        raw_units: impl Into<String>,
        decimals: u8,
    ) -> Result<Self, EvmStateError> {
        let response =
            EvmAddressErc20BalanceResponse::new(block_number, block_hash, raw_units, decimals)?;
        Ok(Self::new(subject, response))
    }

    /// Returns the checked subject material.
    pub const fn subject(&self) -> &EvmAddressErc20BalanceSubject {
        &self.subject
    }

    /// Returns the checked response material.
    pub const fn response(&self) -> &EvmAddressErc20BalanceResponse {
        &self.response
    }
}

/// Source-near ERC-20 balance output prior to managed fact recording.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "erc20_balance_observation",
    version = "1",
    schema = "mfm.evm.state.output.erc20_balance_observation"
)]
pub struct EvmAddressErc20BalanceObservation {
    subject: EvmAddressErc20BalanceSubject,
    response: EvmAddressErc20BalanceResponse,
    source_read_count: u64,
}

impl EvmAddressErc20BalanceObservation {
    /// Creates a source-near ERC-20 observation with the exact closed read count.
    pub const fn new(
        subject: EvmAddressErc20BalanceSubject,
        response: EvmAddressErc20BalanceResponse,
    ) -> Self {
        Self {
            subject,
            response,
            source_read_count: EVM_ERC20_BALANCE_OBSERVE_SOURCE_READS,
        }
    }

    /// Returns the source-near subject.
    pub const fn subject(&self) -> &EvmAddressErc20BalanceSubject {
        &self.subject
    }

    /// Returns the source-near response.
    pub const fn response(&self) -> &EvmAddressErc20BalanceResponse {
        &self.response
    }

    /// Returns the exact state-owned source-read count.
    pub const fn source_read_count(&self) -> u64 {
        self.source_read_count
    }

    /// Revalidates this observation and builds its Platform fact.
    pub fn try_to_fact(&self) -> Result<EvmAddressErc20BalanceSnapshotFact, EvmStateError> {
        self.clone().try_into_fact()
    }

    /// Revalidates this observation and converts it into its Platform fact.
    pub fn try_into_fact(self) -> Result<EvmAddressErc20BalanceSnapshotFact, EvmStateError> {
        let subject = EvmAddressErc20BalanceSubject::new(
            self.subject.network,
            self.subject.chain_id,
            self.subject.contract_address,
            self.subject.account,
        )?;
        let response = self.response.revalidated()?;
        if self.source_read_count != EVM_ERC20_BALANCE_OBSERVE_SOURCE_READS {
            return Err(EvmStateError::InvalidInput {
                reason: "erc20 balance observation source_read_count was not state-owned"
                    .to_owned(),
            });
        }
        Ok(EvmAddressErc20BalanceSnapshotFact::new(subject, response))
    }
}

/// Source-near holding material normalized from an EVM native balance fact.
///
/// Contains no portfolio wallet_id/symbol_id; report join happens later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedEvmNativeHolding {
    /// Semantic network id.
    pub network: String,
    /// EVM chain id.
    pub chain_id: u64,
    /// Observed account.
    pub account: String,
    /// Balance in wei as a decimal string.
    pub raw_wei: String,
    /// Native token decimals.
    pub decimals: u8,
    /// Anchor block number.
    pub block_number: u64,
    /// Anchor block hash.
    pub block_hash: String,
    /// Coverage claim.
    pub coverage: CoverageStatus,
    /// Holding source status.
    pub source_status: HoldingSourceStatus,
}

/// Pure normalize: fact → source-near holding fields.
pub fn normalize_evm_address_native_balance_fact(
    fact: &EvmAddressNativeBalanceSnapshotFact,
) -> Result<NormalizedEvmNativeHolding, EvmStateError> {
    normalize_evm_address_native_balance(fact.subject(), fact.response())
}

/// Builds the receipt-pinned Platform fact-index plan for one EVM native-balance source.
///
/// The receipt fixes subject, exact block number/hash, coverage, status, and the closed N + 1
/// scan limit. A saturated response is deliberately not a successful report input.
#[allow(clippy::too_many_arguments)]
pub fn platform_native_balance_at_anchor_plan(
    store_scope: &StoreScopeRef,
    scope_decision: ScopeDecisionEvidence,
    subject: &EvmAddressNativeBalanceSubject,
    block_number: u64,
    block_hash: &str,
    coverage: &str,
    source_status: &str,
    limit: u64,
) -> Result<CanonicalFactQueryPlan, EvmStateError> {
    use mfm_program::MfmFactType;

    let descriptor = EvmAddressNativeBalanceSnapshotFact::descriptor().map_err(|error| {
        EvmStateError::InvalidInput {
            reason: error.to_string(),
        }
    })?;
    let field = |id: &str| -> Result<FactFieldId, EvmStateError> {
        FactFieldId::new(id).map_err(|error| EvmStateError::InvalidInput {
            reason: error.to_string(),
        })
    };
    let eq_str = |id: &str, value: &str| -> Result<FactQueryPredicate, EvmStateError> {
        Ok(FactQueryPredicate::new(
            field(id)?,
            FactQueryOperator::Equal,
            FactCanonicalScalar::string(value),
        ))
    };
    let eq_u64 = |id: &str, value: u64| -> Result<FactQueryPredicate, EvmStateError> {
        Ok(FactQueryPredicate::new(
            field(id)?,
            FactQueryOperator::Equal,
            FactCanonicalScalar::UnsignedInteger(value),
        ))
    };
    let input = FactQueryInput::new(
        store_scope.clone(),
        FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
        scope_decision,
        vec![
            eq_str("subject.network", subject.network())?,
            eq_u64("subject.chain_id", subject.chain_id())?,
            eq_str("subject.account", subject.account())?,
            eq_u64("result.block_number", block_number)?,
            eq_str("result.block_hash", block_hash)?,
            eq_str("result.coverage", coverage)?,
            eq_str("result.source_status", source_status)?,
        ],
        vec![
            field("result.block_number")?,
            field("result.block_hash")?,
            field("result.raw_wei")?,
            field("result.decimals")?,
            field("result.coverage")?,
            field("result.source_status")?,
            field("metadata.store_commit_order")?,
        ],
        FactOrderingName::new("metadata.store_commit_order.desc").map_err(|error| {
            EvmStateError::InvalidInput {
                reason: error.to_string(),
            }
        })?,
        Some(limit),
    )
    .map_err(|error| EvmStateError::InvalidInput {
        reason: error.to_string(),
    })?;
    compile_fact_query_plan(&descriptor, input).map_err(|error| EvmStateError::InvalidInput {
        reason: error.to_string(),
    })
}

/// Builds the receipt-pinned Platform fact-index plan for one EVM ERC-20 balance source.
///
/// The token contract, account, exact hash anchor, complete coverage, successful status, and the
/// closed N + 1 scan limit are all certified before the query is issued.
#[allow(clippy::too_many_arguments)]
pub fn platform_erc20_balance_at_anchor_plan(
    store_scope: &StoreScopeRef,
    scope_decision: ScopeDecisionEvidence,
    subject: &EvmAddressErc20BalanceSubject,
    block_number: u64,
    block_hash: &str,
    coverage: &str,
    source_status: &str,
    limit: u64,
) -> Result<CanonicalFactQueryPlan, EvmStateError> {
    use mfm_program::MfmFactType;

    let descriptor = EvmAddressErc20BalanceSnapshotFact::descriptor().map_err(|error| {
        EvmStateError::InvalidInput {
            reason: error.to_string(),
        }
    })?;
    let field = |id: &str| -> Result<FactFieldId, EvmStateError> {
        FactFieldId::new(id).map_err(|error| EvmStateError::InvalidInput {
            reason: error.to_string(),
        })
    };
    let eq_str = |id: &str, value: &str| -> Result<FactQueryPredicate, EvmStateError> {
        Ok(FactQueryPredicate::new(
            field(id)?,
            FactQueryOperator::Equal,
            FactCanonicalScalar::string(value),
        ))
    };
    let eq_u64 = |id: &str, value: u64| -> Result<FactQueryPredicate, EvmStateError> {
        Ok(FactQueryPredicate::new(
            field(id)?,
            FactQueryOperator::Equal,
            FactCanonicalScalar::UnsignedInteger(value),
        ))
    };
    let input = FactQueryInput::new(
        store_scope.clone(),
        FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
        scope_decision,
        vec![
            eq_str("subject.network", subject.network())?,
            eq_u64("subject.chain_id", subject.chain_id())?,
            eq_str("subject.contract_address", subject.contract_address())?,
            eq_str("subject.account", subject.account())?,
            eq_u64("result.block_number", block_number)?,
            eq_str("result.block_hash", block_hash)?,
            eq_str("result.coverage", coverage)?,
            eq_str("result.source_status", source_status)?,
        ],
        vec![
            field("result.block_number")?,
            field("result.block_hash")?,
            field("result.raw_units")?,
            field("result.decimals")?,
            field("result.coverage")?,
            field("result.source_status")?,
            field("metadata.store_commit_order")?,
        ],
        FactOrderingName::new("metadata.store_commit_order.desc").map_err(|error| {
            EvmStateError::InvalidInput {
                reason: error.to_string(),
            }
        })?,
        Some(limit),
    )
    .map_err(|error| EvmStateError::InvalidInput {
        reason: error.to_string(),
    })?;
    compile_fact_query_plan(&descriptor, input).map_err(|error| EvmStateError::InvalidInput {
        reason: error.to_string(),
    })
}

/// Pure normalize from subject + response material.
pub fn normalize_evm_address_native_balance(
    subject: &EvmAddressNativeBalanceSubject,
    response: &EvmAddressNativeBalanceResponse,
) -> Result<NormalizedEvmNativeHolding, EvmStateError> {
    let block_hash = canonical_evm_block_hash(response.block_hash())?;
    let coverage = response.coverage_status()?;
    let source_status = response.holding_source_status()?;
    Ok(NormalizedEvmNativeHolding {
        network: subject.network().to_owned(),
        chain_id: subject.chain_id(),
        account: subject.account().to_owned(),
        raw_wei: response.raw_wei().to_owned(),
        decimals: response.decimals(),
        block_number: response.block_number(),
        block_hash,
        coverage,
        source_status,
    })
}

/// Canonicalizes a 32-byte EVM block hash to lowercase `0x`-prefixed hex.
pub fn canonical_evm_block_hash(block_hash: impl Into<String>) -> Result<String, EvmStateError> {
    let block_hash = block_hash.into();
    let hex = block_hash
        .strip_prefix("0x")
        .or_else(|| block_hash.strip_prefix("0X"))
        .unwrap_or(block_hash.as_str());
    let parsed = hex
        .parse::<B256>()
        .map_err(|_| EvmStateError::InvalidInput {
            reason: "block_hash must be 32-byte hex (optional 0x prefix)".to_owned(),
        })?;
    Ok(format!("{parsed:#x}"))
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
