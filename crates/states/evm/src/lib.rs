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
    ResolveEvmJointTipState,
};

use mfm_facts::{CoverageStatus, FactAudience, FactVisibility, HoldingSourceStatus};
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
    /// Creates response material, failing closed on missing hash or inadmissible coverage/status.
    pub fn new(
        block_number: u64,
        block_hash: impl Into<String>,
        raw_wei: impl Into<String>,
        decimals: u8,
        coverage: CoverageStatus,
        source_status: HoldingSourceStatus,
    ) -> Result<Self, EvmStateError> {
        let block_hash = block_hash.into();
        let raw_wei = raw_wei.into();
        if block_hash.trim().is_empty() {
            return Err(EvmStateError::InvalidInput {
                reason: "native balance block_hash is required".to_owned(),
            });
        }
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

/// Pure normalize from subject + response material.
pub fn normalize_evm_address_native_balance(
    subject: &EvmAddressNativeBalanceSubject,
    response: &EvmAddressNativeBalanceResponse,
) -> Result<NormalizedEvmNativeHolding, EvmStateError> {
    if response.block_hash().trim().is_empty() {
        return Err(EvmStateError::InvalidInput {
            reason: "native balance block_hash is required".to_owned(),
        });
    }
    let coverage = response.coverage_status()?;
    let source_status = response.holding_source_status()?;
    if !coverage.is_acceptable_for_report() {
        return Err(EvmStateError::InvalidInput {
            reason: format!(
                "coverage {} is not acceptable for report selection",
                coverage.as_str()
            ),
        });
    }
    if !source_status.is_acceptable_for_report() {
        return Err(EvmStateError::InvalidInput {
            reason: format!(
                "source_status {} is not acceptable for report selection",
                source_status.as_str()
            ),
        });
    }
    Ok(NormalizedEvmNativeHolding {
        network: subject.network().to_owned(),
        chain_id: subject.chain_id(),
        account: subject.account().to_owned(),
        raw_wei: response.raw_wei().to_owned(),
        decimals: response.decimals(),
        block_number: response.block_number(),
        block_hash: response.block_hash().to_owned(),
        coverage,
        source_status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_facts::{FactFieldExposure, FactFieldExtraction, FactFieldValueType};
    use mfm_program::MfmFactType;

    fn valid_subject() -> EvmAddressNativeBalanceSubject {
        EvmAddressNativeBalanceSubject::new(
            "ethereum-mainnet",
            1,
            "0x0000000000000000000000000000000000000001",
        )
        .expect("subject")
    }

    #[test]
    fn descriptor_declares_kind_anchors_coverage_and_store_commit_order() {
        let descriptor = EvmAddressNativeBalanceSnapshotFact::descriptor().expect("descriptor");
        assert_eq!(
            descriptor.fact_kind().as_str(),
            "evm.address_native_balance_snapshot"
        );
        for field_id in [
            "subject.network",
            "subject.chain_id",
            "subject.account",
            "result.block_number",
            "result.block_hash",
            "result.raw_wei",
            "result.decimals",
            "result.coverage",
            "result.source_status",
        ] {
            assert!(
                descriptor.fields().iter().any(|field| {
                    field.field_id().as_str() == field_id
                        && field.exposure() == FactFieldExposure::Returnable
                }),
                "missing returnable field {field_id}"
            );
        }
        assert!(descriptor.fields().iter().any(|field| {
            field.field_id().as_str() == "result.block_number"
                && field.value_type() == FactFieldValueType::UnsignedInteger
                && field.sortable()
        }));
        assert!(descriptor.fields().iter().any(|field| {
            field.field_id().as_str() == "metadata.store_commit_order"
                && matches!(field.extraction(), FactFieldExtraction::Metadata(_))
                && field.sortable()
        }));
    }

    #[test]
    fn write_admission_requires_hash_and_admissible_coverage_status() {
        assert!(EvmAddressNativeBalanceResponse::new(
            1,
            "",
            "0",
            18,
            CoverageStatus::ConfiguredOnly,
            HoldingSourceStatus::Ok,
        )
        .is_err());
        assert!(EvmAddressNativeBalanceResponse::new(
            1,
            "0x".to_owned() + &"ab".repeat(32),
            "not-digits",
            18,
            CoverageStatus::ConfiguredOnly,
            HoldingSourceStatus::Ok,
        )
        .is_err());
        assert!(EvmAddressNativeBalanceResponse::new(
            1,
            "0x".to_owned() + &"ab".repeat(32),
            "1000",
            18,
            CoverageStatus::Truncated,
            HoldingSourceStatus::Ok,
        )
        .is_err());
        let response = EvmAddressNativeBalanceResponse::new(
            21_000_000,
            "0x".to_owned() + &"cd".repeat(32),
            "1000000000000000000",
            18,
            CoverageStatus::ConfiguredOnly,
            HoldingSourceStatus::Ok,
        )
        .expect("admissible");
        let fact = EvmAddressNativeBalanceSnapshotFact::new(valid_subject(), response);
        let normalized = normalize_evm_address_native_balance_fact(&fact).expect("normalize");
        assert_eq!(normalized.raw_wei, "1000000000000000000");
        assert_eq!(normalized.decimals, 18);
        assert_eq!(normalized.block_number, 21_000_000);
        assert_eq!(normalized.chain_id, 1);
    }

    #[test]
    fn normalize_rejects_inadmissible_decoded_payloads() {
        let missing_hash: EvmAddressNativeBalanceResponse =
            serde_json::from_value(serde_json::json!({
                "block_number": 10,
                "block_hash": "",
                "raw_wei": "1",
                "decimals": 18,
                "coverage": "configured_only",
                "source_status": "ok",
            }))
            .expect("decode");
        assert!(normalize_evm_address_native_balance(&valid_subject(), &missing_hash).is_err());

        let failed: EvmAddressNativeBalanceResponse = serde_json::from_value(serde_json::json!({
            "block_number": 10,
            "block_hash": "0xhh",
            "raw_wei": "1",
            "decimals": 18,
            "coverage": "configured_only",
            "source_status": "failed",
        }))
        .expect("decode");
        assert!(normalize_evm_address_native_balance(&valid_subject(), &failed).is_err());
    }

    #[test]
    fn response_json_round_trip_has_no_floats_or_secrets() {
        let response = EvmAddressNativeBalanceResponse::new(
            1,
            "0x".to_owned() + &"11".repeat(32),
            "42",
            18,
            CoverageStatus::CompleteAtAnchor,
            HoldingSourceStatus::Ok,
        )
        .expect("response");
        let fact = EvmAddressNativeBalanceSnapshotFact::new(valid_subject(), response);
        let value = serde_json::to_value(&fact).expect("json");
        let text = serde_json::to_string(&value).expect("text");
        for forbidden in ["rpc_url", "password", "http://", "wallet_id", "symbol_id"] {
            assert!(
                !text.contains(forbidden),
                "leaked forbidden token {forbidden}: {text}"
            );
        }
        fn assert_no_floats(value: &serde_json::Value) {
            match value {
                serde_json::Value::Number(n) => assert!(n.is_u64() || n.is_i64(), "{n}"),
                serde_json::Value::Array(items) => items.iter().for_each(assert_no_floats),
                serde_json::Value::Object(map) => map.values().for_each(assert_no_floats),
                _ => {}
            }
        }
        assert_no_floats(&value);
        let decoded: EvmAddressNativeBalanceSnapshotFact =
            serde_json::from_value(value).expect("round-trip");
        assert_eq!(decoded, fact);
    }

    #[test]
    fn platform_candidate_query_plan_is_exact_full_set_not_limit_one() {
        use mfm_facts::{
            compile_fact_query_plan, FactAudience, FactCanonicalScalar, FactFieldId,
            FactOrderingName, FactQueryInput, FactQueryOperator, FactQueryPredicate,
            FactQueryScope, FactVisibilityScope, ScopeDecisionEvidence, StoreScopeRef,
        };
        use mfm_ids::{ContentDigest, DigestAlgorithm, DigestBytes};

        let descriptor = EvmAddressNativeBalanceSnapshotFact::descriptor().expect("descriptor");
        let subject = valid_subject();
        let input = FactQueryInput::new(
            StoreScopeRef::new("mfm.store.default").expect("store"),
            FactQueryScope::new(FactAudience::Platform, FactVisibilityScope::Default),
            ScopeDecisionEvidence::new(ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x51; 32]),
            )),
            vec![
                FactQueryPredicate::new(
                    FactFieldId::new("subject.network").expect("field"),
                    FactQueryOperator::Equal,
                    FactCanonicalScalar::string(subject.network()),
                ),
                FactQueryPredicate::new(
                    FactFieldId::new("subject.chain_id").expect("field"),
                    FactQueryOperator::Equal,
                    FactCanonicalScalar::UnsignedInteger(subject.chain_id()),
                ),
                FactQueryPredicate::new(
                    FactFieldId::new("subject.account").expect("field"),
                    FactQueryOperator::Equal,
                    FactCanonicalScalar::string(subject.account()),
                ),
            ],
            vec![
                FactFieldId::new("result.block_number").expect("field"),
                FactFieldId::new("result.block_hash").expect("field"),
                FactFieldId::new("result.raw_wei").expect("field"),
                FactFieldId::new("result.coverage").expect("field"),
                FactFieldId::new("result.source_status").expect("field"),
            ],
            FactOrderingName::new("result.block_number.desc").expect("ordering"),
            None,
        )
        .expect("query input");
        let plan = compile_fact_query_plan(&descriptor, input).expect("plan");
        assert_eq!(plan.query_scope().audience(), FactAudience::Platform);
        assert_eq!(plan.limit(), None);
    }
}
