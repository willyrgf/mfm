//! EVM native balance collector states: joint tip → hash-bound observe → record.
//!
//! Snapshot pattern only. Multi-subject same-network batches resolve tip once and
//! pin all balances at that block hash (EIP-1898-style). Fail closed before write.

use std::future;
use std::num::NonZeroU64;
use std::str::FromStr;

use mfm_canonical::sha256_digest_bytes;
use mfm_capabilities::NoCaps;
use mfm_effects::{ManagedPlatformWrite, Pure, ReadExternal};
use mfm_evm_capabilities::{
    EvmBalanceReadCapability, EvmBalanceReadResponse, EvmBlockReadCapability, EvmBlockReadResponse,
    EvmNetworkId,
};
use mfm_fact_capabilities::FactRecordCapability;
use mfm_ids::{AdapterKind, AdapterVersion, DigestAlgorithm, StateKind, StateVersion};
use mfm_portfolio_model::holding::{CoverageStatus, HoldingSourceStatus};
use mfm_program::{
    fact_descriptor_ref, AdapterBindingSpec, FactDescriptorRef, ManagedWriteState, NoContext,
    PureState, ReadState, StateError, StateResult, StateSpec, ValidatedConfig,
};
use mfm_program_derive::{MfmConfig, MfmValue, StateInput};
use mfm_values::NonEmpty;
use serde::{Deserialize, Serialize};

use crate::{
    address_native_balance_fact_visibility, validate_canonical_evm_account,
    EvmAddressNativeBalanceResponse, EvmAddressNativeBalanceSnapshotFact,
    EvmAddressNativeBalanceSubject, EvmStateError,
};

const NAMESPACE: &str = "mfm.evm";
const EVM_JSONRPC_ADAPTER_NAME: &str = "jsonrpc";
const EVM_JSONRPC_ADAPTER_VERSION: &str = "mfm.evm.jsonrpc.adapter.v1";
const DEFAULT_NATIVE_DECIMALS: u8 = 18;

/// Exact number of source reads required by one hash-pinned native-balance observation.
pub const EVM_NATIVE_BALANCE_OBSERVE_SOURCE_READS: u64 = 2;

/// Returns the stable EVM JSON-RPC adapter kind for native collectors.
pub fn evm_jsonrpc_adapter_kind() -> Result<AdapterKind, mfm_ids::IdentityError> {
    AdapterKind::new(
        NAMESPACE,
        EVM_JSONRPC_ADAPTER_NAME,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.evm.adapter:jsonrpc"),
    )
}

/// Returns the stable EVM JSON-RPC adapter version for native collectors.
pub fn evm_jsonrpc_adapter_version() -> Result<AdapterVersion, mfm_ids::IdentityError> {
    AdapterVersion::new(EVM_JSONRPC_ADAPTER_VERSION)
}

fn adapter_binding() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
    Ok(vec![AdapterBindingSpec {
        adapter_kind: evm_jsonrpc_adapter_kind().map_err(|error| {
            mfm_program::PlanError::Key(format!("EVM JSON-RPC adapter kind invalid: {error}"))
        })?,
        adapter_version: evm_jsonrpc_adapter_version().map_err(|error| {
            mfm_program::PlanError::Key(format!("EVM JSON-RPC adapter version invalid: {error}"))
        })?,
    }])
}

fn state_kind(name: &'static str) -> mfm_program::Result<StateKind> {
    StateKind::new(
        NAMESPACE,
        name,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.evm.state:{name}").as_bytes()),
    )
    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

fn state_version(name: &'static str) -> mfm_program::Result<StateVersion> {
    StateVersion::new(format!("mfm.evm.state.{name}.v1"))
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

fn adapter_required_error(state_name: &'static str) -> StateError {
    StateError::Message(format!("{state_name} requires an EVM adapter runner"))
}

/// Shared joint tip resolved once for a same-network multi-subject batch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "joint_tip",
    version = "1",
    schema = "mfm.evm.state.value.joint_tip"
)]
pub struct EvmJointTip {
    network: String,
    chain_id: u64,
    block_number: u64,
    block_hash: String,
}

impl EvmJointTip {
    /// Creates a joint tip from checked components.
    ///
    /// Block hashes are stored in canonical lowercase `0x`-prefixed form so tip
    /// equality and portfolio anchor intersection use one identity.
    pub fn new(
        network: impl Into<String>,
        chain_id: u64,
        block_number: u64,
        block_hash: impl Into<String>,
    ) -> Result<Self, EvmStateError> {
        let network = network.into();
        if network.trim().is_empty() {
            return Err(EvmStateError::InvalidInput {
                reason: "joint tip network must be non-empty".to_owned(),
            });
        }
        if chain_id == 0 {
            return Err(EvmStateError::InvalidInput {
                reason: "chain_id must be non-zero".to_owned(),
            });
        }
        let block_hash = crate::canonical_evm_block_hash(block_hash)?;
        Ok(Self {
            network,
            chain_id,
            block_number,
            block_hash,
        })
    }

    /// Materializes a joint tip from a verified block-read capability response.
    pub fn from_block_response(
        network: &str,
        chain_id: u64,
        response: &EvmBlockReadResponse,
    ) -> Result<Self, EvmStateError> {
        if response.evidence.expected_chain_id != chain_id
            || response.evidence.observed_chain_id != chain_id
        {
            return Err(EvmStateError::InvalidInput {
                reason: "block response chain id does not match collector config".to_owned(),
            });
        }
        if response.evidence.network_id.as_str() != network {
            return Err(EvmStateError::InvalidInput {
                reason: "block response network does not match collector config".to_owned(),
            });
        }
        Self::new(
            network,
            chain_id,
            response.block_number,
            format_block_hash(&response.block_hash),
        )
    }

    /// Returns the semantic network id.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Returns the EVM chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the joint tip block number.
    pub const fn block_number(&self) -> u64 {
        self.block_number
    }

    /// Returns the joint tip block hash.
    pub fn block_hash(&self) -> &str {
        &self.block_hash
    }

    /// Returns whether this tip is admissible for balance write.
    pub fn is_admissible_for_balance_write(&self) -> bool {
        self.chain_id != 0 && !self.block_hash.trim().is_empty()
    }
}

fn format_block_hash(hash: &alloy_primitives::B256) -> String {
    format!("{hash:#x}")
}

/// Requires every observation in a same-network batch to share one joint tip anchor.
pub fn require_shared_evm_joint_tip(
    observations: &[&EvmAddressNativeBalanceObservation],
) -> Result<(), EvmStateError> {
    let Some(first) = observations.first() else {
        return Err(EvmStateError::InvalidInput {
            reason: "shared tip batch requires at least one observation".to_owned(),
        });
    };
    let number = first.response().block_number();
    let hash = first.response().block_hash();
    let network = first.subject().network();
    let chain_id = first.subject().chain_id();
    for observation in observations.iter().skip(1) {
        if observation.subject().network() != network
            || observation.subject().chain_id() != chain_id
        {
            return Err(EvmStateError::InvalidInput {
                reason: "shared tip batch subjects must share one network/chain".to_owned(),
            });
        }
        if observation.response().block_number() != number
            || observation.response().block_hash() != hash
        {
            return Err(EvmStateError::InvalidInput {
                reason: "multi-subject same-network batch must share one joint tip".to_owned(),
            });
        }
    }
    Ok(())
}

/// Config for resolving a joint tip once per same-network collector batch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(
    schema = "mfm.evm.state.config.resolve_joint_tip",
    validate = "validate_resolve_evm_joint_tip_config"
)]
pub struct ResolveEvmJointTipConfig {
    /// Semantic network id.
    pub network: String,
    /// Expected EVM chain id.
    pub chain_id: u64,
    /// Maximum number of source reads this bounded state may request.
    pub max_source_reads: NonZeroU64,
}

/// Validates joint-tip resolve config.
pub fn validate_resolve_evm_joint_tip_config(
    config: &ResolveEvmJointTipConfig,
) -> Result<(), String> {
    EvmNetworkId::new(&config.network).map_err(|error| error.to_string())?;
    if config.chain_id == 0 {
        return Err("chain_id must be non-zero".to_owned());
    }
    Ok(())
}

/// Input for joint-tip resolution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.evm.state.input.resolve_joint_tip")]
pub struct ResolveEvmJointTipInput {}

/// Materializes and admits a joint tip from a verified block response.
pub fn materialize_evm_joint_tip(
    config: &ResolveEvmJointTipConfig,
    response: &EvmBlockReadResponse,
) -> Result<EvmJointTip, EvmStateError> {
    validate_resolve_evm_joint_tip_config(config)
        .map_err(|reason| EvmStateError::InvalidInput { reason })?;
    let tip = EvmJointTip::from_block_response(&config.network, config.chain_id, response)?;
    if !tip.is_admissible_for_balance_write() {
        return Err(EvmStateError::InvalidInput {
            reason: "joint tip is not admissible for balance write".to_owned(),
        });
    }
    Ok(tip)
}

/// State contract for resolving one joint tip for a same-network batch.
pub struct ResolveEvmJointTipState {
    config: ResolveEvmJointTipConfig,
}

impl ResolveEvmJointTipState {
    /// Materializes a joint tip from a verified block-read response.
    pub fn materialize_response(
        &self,
        response: &EvmBlockReadResponse,
    ) -> Result<EvmJointTip, EvmStateError> {
        materialize_evm_joint_tip(&self.config, response)
    }

    /// Returns the certified config.
    pub fn config(&self) -> &ResolveEvmJointTipConfig {
        &self.config
    }
}

impl StateSpec for ResolveEvmJointTipState {
    type Config = ResolveEvmJointTipConfig;
    type Context = NoContext;
    type Input = ResolveEvmJointTipInput;
    type Output = EvmJointTip;
    type Effect = ReadExternal;
    type Caps = (EvmBlockReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("joint_tip.resolve")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("joint_tip.resolve")
    }

    fn name() -> &'static str {
        "mfm.evm.joint_tip.resolve"
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

impl ReadState for ResolveEvmJointTipState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        _input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        if let Err(error) = validate_resolve_evm_joint_tip_config(&self.config) {
            return future::ready(Err(StateError::from(EvmStateError::InvalidInput {
                reason: error,
            })));
        }
        future::ready(Err(adapter_required_error(Self::name())))
    }
}

/// Config for an EVM native address-balance observation state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(
    schema = "mfm.evm.state.config.observe_native_balance",
    validate = "validate_observe_evm_native_balance_config"
)]
pub struct ObserveEvmNativeBalanceConfig {
    /// Semantic network id.
    pub network: String,
    /// Expected EVM chain id.
    pub chain_id: u64,
    /// Account address to observe.
    pub account: String,
    /// Coverage claim written on success.
    pub coverage: String,
    /// Native token decimals (typically 18).
    pub decimals: u8,
    /// Maximum number of source reads this bounded state may request.
    pub max_source_reads: NonZeroU64,
}

/// Validates native-balance observation config.
pub fn validate_observe_evm_native_balance_config(
    config: &ObserveEvmNativeBalanceConfig,
) -> Result<(), String> {
    EvmNetworkId::new(&config.network).map_err(|error| error.to_string())?;
    if config.chain_id == 0 {
        return Err("chain_id must be non-zero".to_owned());
    }
    validate_canonical_evm_account(&config.account).map_err(|error| error.to_string())?;
    if config.max_source_reads.get() != EVM_NATIVE_BALANCE_OBSERVE_SOURCE_READS {
        return Err(format!(
            "max_source_reads must equal {EVM_NATIVE_BALANCE_OBSERVE_SOURCE_READS} for native balance observation"
        ));
    }
    let coverage = CoverageStatus::from_str(&config.coverage)
        .map_err(|_| format!("unknown coverage status {:?}", config.coverage))?;
    if !coverage.is_admissible_for_write() {
        return Err(format!(
            "coverage {} is not admissible for Platform write",
            coverage.as_str()
        ));
    }
    Ok(())
}

impl ObserveEvmNativeBalanceConfig {
    /// Parses the configured coverage claim.
    pub fn coverage_status(&self) -> Result<CoverageStatus, EvmStateError> {
        validate_observe_evm_native_balance_config(self)
            .map_err(|reason| EvmStateError::InvalidInput { reason })?;
        CoverageStatus::from_str(&self.coverage).map_err(|_| EvmStateError::InvalidInput {
            reason: format!("unknown coverage status {:?}", self.coverage),
        })
    }
}

/// Input for a hash-pinned native balance observation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.evm.state.input.observe_native_balance")]
pub struct ObserveEvmNativeBalanceInput {
    /// Joint tip resolved once for this same-network batch.
    pub joint_tip: EvmJointTip,
}

/// Normalized native-balance observation state output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "native_balance_observation",
    version = "1",
    schema = "mfm.evm.state.output.native_balance_observation"
)]
pub struct EvmAddressNativeBalanceObservation {
    subject: EvmAddressNativeBalanceSubject,
    response: EvmAddressNativeBalanceResponse,
    source_read_count: u64,
}

impl EvmAddressNativeBalanceObservation {
    /// Creates normalized observation output.
    pub const fn new(
        subject: EvmAddressNativeBalanceSubject,
        response: EvmAddressNativeBalanceResponse,
        source_read_count: u64,
    ) -> Self {
        Self {
            subject,
            response,
            source_read_count,
        }
    }

    /// Returns the normalized subject.
    pub const fn subject(&self) -> &EvmAddressNativeBalanceSubject {
        &self.subject
    }

    /// Returns the normalized response.
    pub const fn response(&self) -> &EvmAddressNativeBalanceResponse {
        &self.response
    }

    /// Builds the platform fact from this observation.
    pub fn to_fact(&self) -> EvmAddressNativeBalanceSnapshotFact {
        EvmAddressNativeBalanceSnapshotFact::new(self.subject.clone(), self.response.clone())
    }

    /// Converts this observation into the platform fact.
    pub fn into_fact(self) -> EvmAddressNativeBalanceSnapshotFact {
        EvmAddressNativeBalanceSnapshotFact::new(self.subject, self.response)
    }

    /// Returns the number of source reads used by this bounded observation.
    pub const fn source_read_count(&self) -> u64 {
        self.source_read_count
    }
}

/// Proves balance@joint-tip hash before constructing an admissible observation DTO.
///
/// `balance_wei_decimal` must be the verified balance at the joint tip hash.
/// `verified_tip` is the tip re-checked after the balance read (must match joint tip).
pub fn normalize_evm_native_balance_observation(
    config: &ObserveEvmNativeBalanceConfig,
    joint_tip: &EvmJointTip,
    verified_tip: &EvmJointTip,
    balance_wei_decimal: &str,
    balance_evidence_chain_id: u64,
    balance_evidence_network: &str,
) -> Result<EvmAddressNativeBalanceObservation, EvmStateError> {
    validate_observe_evm_native_balance_config(config)
        .map_err(|reason| EvmStateError::InvalidInput { reason })?;
    if !joint_tip.is_admissible_for_balance_write() {
        return Err(EvmStateError::InvalidInput {
            reason: "joint tip is not admissible for balance write".to_owned(),
        });
    }
    if joint_tip.network() != config.network || joint_tip.chain_id() != config.chain_id {
        return Err(EvmStateError::InvalidInput {
            reason: "joint tip binding does not match native balance config".to_owned(),
        });
    }
    if verified_tip.block_number() != joint_tip.block_number()
        || verified_tip.block_hash() != joint_tip.block_hash()
        || verified_tip.network() != joint_tip.network()
        || verified_tip.chain_id() != joint_tip.chain_id()
    {
        return Err(EvmStateError::InvalidInput {
            reason: "tip drift or hash mismatch before Platform write".to_owned(),
        });
    }
    if balance_evidence_chain_id != config.chain_id || balance_evidence_network != config.network {
        return Err(EvmStateError::InvalidInput {
            reason: "balance response evidence does not match collector config".to_owned(),
        });
    }
    if balance_wei_decimal.trim().is_empty()
        || !balance_wei_decimal.chars().all(|c| c.is_ascii_digit())
    {
        return Err(EvmStateError::InvalidInput {
            reason: "raw_wei must be a non-empty decimal digit string".to_owned(),
        });
    }
    let coverage = config.coverage_status()?;
    let subject = EvmAddressNativeBalanceSubject::new(
        config.network.clone(),
        config.chain_id,
        config.account.clone(),
    )?;
    let response = EvmAddressNativeBalanceResponse::new(
        joint_tip.block_number(),
        joint_tip.block_hash(),
        balance_wei_decimal,
        config.decimals,
        coverage,
        HoldingSourceStatus::Ok,
    )?;
    Ok(EvmAddressNativeBalanceObservation::new(
        subject,
        response,
        EVM_NATIVE_BALANCE_OBSERVE_SOURCE_READS,
    ))
}

/// Convenience normalize from a capability balance response + joint tip verification.
pub fn normalize_evm_native_balance_from_capability(
    config: &ObserveEvmNativeBalanceConfig,
    joint_tip: &EvmJointTip,
    verified_tip: &EvmJointTip,
    balance: &EvmBalanceReadResponse,
) -> Result<EvmAddressNativeBalanceObservation, EvmStateError> {
    normalize_evm_native_balance_observation(
        config,
        joint_tip,
        verified_tip,
        &balance.balance_wei.to_string(),
        balance.evidence.observed_chain_id,
        balance.evidence.network_id.as_str(),
    )
}

/// State contract for a hash-pinned EVM native balance observation.
pub struct ObserveEvmNativeBalanceState {
    config: ObserveEvmNativeBalanceConfig,
}

impl ObserveEvmNativeBalanceState {
    /// Materializes a normalized observation from joint tip + verified balance material.
    pub fn materialize_response(
        &self,
        input: &ObserveEvmNativeBalanceInput,
        verified_tip: &EvmJointTip,
        balance: &EvmBalanceReadResponse,
    ) -> Result<EvmAddressNativeBalanceObservation, EvmStateError> {
        normalize_evm_native_balance_from_capability(
            &self.config,
            &input.joint_tip,
            verified_tip,
            balance,
        )
    }

    /// Returns the certified config.
    pub fn config(&self) -> &ObserveEvmNativeBalanceConfig {
        &self.config
    }
}

impl StateSpec for ObserveEvmNativeBalanceState {
    type Config = ObserveEvmNativeBalanceConfig;
    type Context = NoContext;
    type Input = ObserveEvmNativeBalanceInput;
    type Output = EvmAddressNativeBalanceObservation;
    type Effect = ReadExternal;
    type Caps = (EvmBalanceReadCapability, EvmBlockReadCapability);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("native_balance.observe")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("native_balance.observe")
    }

    fn name() -> &'static str {
        "mfm.evm.native_balance.observe"
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

impl ReadState for ObserveEvmNativeBalanceState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        if let Err(error) = validate_observe_evm_native_balance_config(&self.config) {
            return future::ready(Err(StateError::from(EvmStateError::InvalidInput {
                reason: error,
            })));
        }
        if !input.joint_tip.is_admissible_for_balance_write() {
            return future::ready(Err(StateError::from(EvmStateError::InvalidInput {
                reason: "joint tip is not admissible for balance write".to_owned(),
            })));
        }
        future::ready(Err(adapter_required_error(Self::name())))
    }
}

/// Config for recording an EVM native balance Platform fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(schema = "mfm.evm.state.config.record_native_balance_fact")]
pub struct RecordEvmNativeBalanceFactConfig {}

/// Input for recording an EVM native balance fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.evm.state.input.record_native_balance_fact")]
pub struct RecordEvmNativeBalanceFactInput {
    /// Whole native-balance observation output to transform into a fact.
    pub observation: EvmAddressNativeBalanceObservation,
}

/// State contract for recording an EVM native balance Platform fact.
pub struct RecordEvmNativeBalanceFactState;

impl StateSpec for RecordEvmNativeBalanceFactState {
    type Config = RecordEvmNativeBalanceFactConfig;
    type Context = NoContext;
    type Input = RecordEvmNativeBalanceFactInput;
    type Output = EvmAddressNativeBalanceSnapshotFact;
    type Effect = ManagedPlatformWrite;
    type Caps = (FactRecordCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("native_balance.record")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("native_balance.record")
    }

    fn name() -> &'static str {
        "mfm.evm.native_balance.record"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn emitted_fact_descriptors() -> mfm_program::Result<Vec<FactDescriptorRef>> {
        Ok(vec![fact_descriptor_ref::<
            EvmAddressNativeBalanceSnapshotFact,
        >()?])
    }

    fn new(_config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self)
    }
}

impl ManagedWriteState for RecordEvmNativeBalanceFactState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        let response = input.observation.response();
        match EvmAddressNativeBalanceResponse::new(
            response.block_number(),
            response.block_hash(),
            response.raw_wei(),
            response.decimals(),
            match response.coverage_status() {
                Ok(coverage) => coverage,
                Err(error) => return future::ready(Err(StateError::from(error))),
            },
            match response.holding_source_status() {
                Ok(status) => status,
                Err(error) => return future::ready(Err(StateError::from(error))),
            },
        ) {
            Ok(_) => future::ready(Ok(input.observation.into_fact())),
            Err(error) => future::ready(Err(StateError::from(error))),
        }
    }
}

/// Returns Platform visibility for native-balance facts (adapter record runner).
pub fn native_balance_record_visibility() -> mfm_facts::FactVisibility {
    address_native_balance_fact_visibility()
}

/// Config for assembling a multi-account native balance batch summary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(schema = "mfm.evm.state.config.assemble_native_balance_batch")]
pub struct AssembleEvmNativeBalanceBatchConfig {}

/// Input for assembling a multi-account native balance batch summary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.evm.state.input.assemble_native_balance_batch")]
pub struct AssembleEvmNativeBalanceBatchInput {
    /// Joint tip shared by the batch.
    pub joint_tip: EvmJointTip,
    /// Recorded native-balance facts (at least one).
    pub balance_facts: NonEmpty<EvmAddressNativeBalanceSnapshotFact>,
}

/// Summary of a multi-account same-network native balance collector batch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "native_balance_batch_summary",
    version = "1",
    schema = "mfm.evm.state.output.native_balance_batch_summary"
)]
pub struct EvmNativeBalanceBatchSummary {
    network: String,
    chain_id: u64,
    joint_tip_block_number: u64,
    joint_tip_block_hash: String,
    account_count: u64,
}

impl EvmNativeBalanceBatchSummary {
    /// Returns the semantic network id.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Returns the joint tip block number.
    pub const fn joint_tip_block_number(&self) -> u64 {
        self.joint_tip_block_number
    }

    /// Returns the joint tip block hash.
    pub fn joint_tip_block_hash(&self) -> &str {
        &self.joint_tip_block_hash
    }

    /// Returns the number of accounts collected.
    pub const fn account_count(&self) -> u64 {
        self.account_count
    }
}

/// Pure state that verifies shared tip and summarizes a native balance batch.
pub struct AssembleEvmNativeBalanceBatchState;

impl StateSpec for AssembleEvmNativeBalanceBatchState {
    type Config = AssembleEvmNativeBalanceBatchConfig;
    type Context = NoContext;
    type Input = AssembleEvmNativeBalanceBatchInput;
    type Output = EvmNativeBalanceBatchSummary;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("native_balance.assemble_batch")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("native_balance.assemble_batch")
    }

    fn name() -> &'static str {
        "mfm.evm.native_balance.assemble_batch"
    }

    fn new(_config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self)
    }
}

impl PureState for AssembleEvmNativeBalanceBatchState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        assemble_evm_native_balance_batch(input)
    }
}

/// Verifies shared tip across recorded facts and builds a batch summary.
pub fn assemble_evm_native_balance_batch(
    input: AssembleEvmNativeBalanceBatchInput,
) -> StateResult<EvmNativeBalanceBatchSummary> {
    let facts = input.balance_facts.values();
    let observations: Vec<EvmAddressNativeBalanceObservation> = facts
        .iter()
        .map(|fact| {
            EvmAddressNativeBalanceObservation::new(
                fact.subject().clone(),
                fact.response().clone(),
                1,
            )
        })
        .collect();
    let refs: Vec<&EvmAddressNativeBalanceObservation> = observations.iter().collect();
    require_shared_evm_joint_tip(&refs).map_err(StateError::from)?;
    let first = facts
        .first()
        .expect("NonEmpty guarantees at least one fact");
    if first.response().block_number() != input.joint_tip.block_number()
        || first.response().block_hash() != input.joint_tip.block_hash()
    {
        return Err(StateError::from(EvmStateError::InvalidInput {
            reason: "recorded facts do not match batch joint tip".to_owned(),
        }));
    }
    let account_count = u64::try_from(facts.len()).map_err(|_| {
        StateError::from(EvmStateError::InvalidInput {
            reason: "account count overflow".to_owned(),
        })
    })?;
    Ok(EvmNativeBalanceBatchSummary {
        network: input.joint_tip.network().to_owned(),
        chain_id: input.joint_tip.chain_id(),
        joint_tip_block_number: input.joint_tip.block_number(),
        joint_tip_block_hash: input.joint_tip.block_hash().to_owned(),
        account_count,
    })
}

/// Default native token decimals used when config omits a custom value.
pub const fn default_native_decimals() -> u8 {
    DEFAULT_NATIVE_DECIMALS
}

#[cfg(test)]
#[path = "native_balance_collect_tests.rs"]
mod tests;
