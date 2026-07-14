#![warn(missing_docs)]
//! Reusable EVM holding fact state contracts.
//!
//! This crate owns typed EVM native balance snapshot facts used by portfolio collectors
//! and report selection, plus native balance collector observe/record states. It defines
//! no JSON-RPC transport, runtime source routing, workflow topology, CLI, REST, or app
//! registration.
//!
//! Contract lifecycle states live in `mfm-state-evm-contracts`, not here.

mod native_balance_collect;

pub use native_balance_collect::{
    assemble_evm_native_balance_batch, default_native_decimals, evm_jsonrpc_adapter_kind,
    evm_jsonrpc_adapter_version, materialize_evm_joint_tip, native_balance_record_visibility,
    normalize_evm_native_balance_from_capability, normalize_evm_native_balance_observation,
    require_shared_evm_joint_tip, validate_observe_evm_native_balance_config,
    validate_resolve_evm_joint_tip_config, AssembleEvmNativeBalanceBatchConfig,
    AssembleEvmNativeBalanceBatchInput, AssembleEvmNativeBalanceBatchInputHandles,
    AssembleEvmNativeBalanceBatchState, EvmAddressNativeBalanceObservation, EvmJointTip,
    EvmNativeBalanceBatchSummary, ObserveEvmNativeBalanceConfig, ObserveEvmNativeBalanceInput,
    ObserveEvmNativeBalanceInputHandles, ObserveEvmNativeBalanceState,
    RecordEvmNativeBalanceFactConfig, RecordEvmNativeBalanceFactInput,
    RecordEvmNativeBalanceFactInputHandles, RecordEvmNativeBalanceFactState,
    ResolveEvmJointTipConfig, ResolveEvmJointTipInput, ResolveEvmJointTipInputHandles,
    ResolveEvmJointTipState, EVM_NATIVE_BALANCE_OBSERVE_SOURCE_READS,
};

use alloy_primitives::B256;
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

/// Builds the Platform fact-index plan for portfolio (or other) candidate selection.
///
/// Exact subject predicates, full return set including `metadata.store_commit_order`,
/// block_number-desc ordering, and **`limit: None`** so selection sees the full set.
pub fn platform_native_balance_candidate_plan(
    store_scope: &StoreScopeRef,
    scope_decision: ScopeDecisionEvidence,
    subject: &EvmAddressNativeBalanceSubject,
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
        FactOrderingName::new("result.block_number.desc").map_err(|error| {
            EvmStateError::InvalidInput {
                reason: error.to_string(),
            }
        })?,
        None,
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
