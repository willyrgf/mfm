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
use mfm_evm_capabilities::{EvmBlock, EvmReadCapability, EvmSessionEvidence};
use mfm_fact_capabilities::FactRecordCapability;
use mfm_ids::{
    AdapterKind, AdapterVersion, DigestAlgorithm, LocalPublicId, StateKind, StateVersion,
};
use mfm_portfolio_model::holding::{CoverageStatus, HoldingSourceStatus};
use mfm_portfolio_model::portfolio::{NetworkConfig, NetworkFamilyConfig};
use mfm_program::{
    fact_descriptor_ref, AdapterBindingSpec, FactDescriptorRef, ManagedWriteState, MfmFactType,
    NoContext, PureState, ReadState, StateError, StateResult, StateSpec, ValidatedConfig,
};
use mfm_program_derive::{MfmConfig, MfmValue, StateInput};
use mfm_values::NonEmpty;
use serde::{de, Deserialize, Serialize};

use crate::{
    address_native_balance_fact_visibility, validate_canonical_evm_account,
    EvmAddressNativeBalanceResponse, EvmAddressNativeBalanceSnapshotFact,
    EvmAddressNativeBalanceSubject, EvmStateError, RedactedEvmSessionEvidence,
};

pub(crate) const NAMESPACE: &str = "mfm.evm";
const EVM_JSONRPC_ADAPTER_NAME: &str = "jsonrpc";
const EVM_JSONRPC_ADAPTER_VERSION: &str = "mfm.evm.jsonrpc.adapter.v1";
/// Exact number of source reads required by one hash-pinned native-balance observation.
pub const EVM_NATIVE_BALANCE_OBSERVE_SOURCE_READS: u64 = 2;
/// Exact number of source reads required to resolve an EVM network collection joint tip.
pub const EVM_JOINT_TIP_SOURCE_READS: u64 = 1;
/// Closed coverage claim for a successful configured native source observation.
pub const EVM_NATIVE_BALANCE_COVERAGE: &str = "configured_only";
/// Closed source-status claim for a successful EVM native source observation.
pub const EVM_NATIVE_BALANCE_SOURCE_STATUS: &str = "ok";

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

pub(crate) fn adapter_binding() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
    Ok(vec![AdapterBindingSpec {
        adapter_kind: evm_jsonrpc_adapter_kind().map_err(|error| {
            mfm_program::PlanError::Key(format!("EVM JSON-RPC adapter kind invalid: {error}"))
        })?,
        adapter_version: evm_jsonrpc_adapter_version().map_err(|error| {
            mfm_program::PlanError::Key(format!("EVM JSON-RPC adapter version invalid: {error}"))
        })?,
    }])
}

pub(crate) fn state_kind(name: &'static str) -> mfm_program::Result<StateKind> {
    StateKind::new(
        NAMESPACE,
        name,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.evm.state:{name}").as_bytes()),
    )
    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

pub(crate) fn state_version(name: &'static str) -> mfm_program::Result<StateVersion> {
    StateVersion::new(format!("mfm.evm.state.{name}.v1"))
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

pub(crate) fn adapter_required_error(state_name: &'static str) -> StateError {
    StateError::Message(format!("{state_name} requires an EVM adapter runner"))
}

/// Shared joint tip resolved once for a same-network multi-subject batch.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, MfmValue)]
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
    source_binding: RedactedEvmSessionEvidence,
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
        source_binding: RedactedEvmSessionEvidence,
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
        if !source_binding.is_bound_to(&network, chain_id) {
            return Err(EvmStateError::InvalidInput {
                reason: "joint tip source binding does not match its network and chain".to_owned(),
            });
        }
        let block_hash = crate::canonical_evm_block_hash(block_hash)?;
        Ok(Self {
            network,
            chain_id,
            block_number,
            block_hash,
            source_binding,
        })
    }

    /// Materializes a joint tip from one checked session block read.
    pub fn from_session_block(
        network: &str,
        chain_id: u64,
        block: &EvmBlock,
        evidence: &EvmSessionEvidence,
    ) -> Result<Self, EvmStateError> {
        let source_binding = RedactedEvmSessionEvidence::from_session(evidence)?;
        if !source_binding.is_bound_to(network, chain_id) {
            return Err(EvmStateError::InvalidInput {
                reason: "block response source binding does not match collector config".to_owned(),
            });
        }
        let block_number =
            u64::try_from(block.number).map_err(|_| EvmStateError::InvalidInput {
                reason: "EVM block number exceeded the supported u64 range".to_owned(),
            })?;
        Self::new(
            network,
            chain_id,
            block_number,
            format_block_hash(&block.hash),
            source_binding,
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

    /// Returns the exact redacted provider source used to resolve this tip.
    pub const fn source_binding(&self) -> &RedactedEvmSessionEvidence {
        &self.source_binding
    }

    /// Returns whether this tip is admissible for balance write.
    pub fn is_admissible_for_balance_write(&self) -> bool {
        self.chain_id != 0
            && !self.block_hash.trim().is_empty()
            && self
                .source_binding
                .is_bound_to(&self.network, self.chain_id)
    }
}

impl<'de> Deserialize<'de> for EvmJointTip {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            network: String,
            chain_id: u64,
            block_number: u64,
            block_hash: String,
            source_binding: RedactedEvmSessionEvidence,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.network,
            wire.chain_id,
            wire.block_number,
            wire.block_hash,
            wire.source_binding,
        )
        .map_err(de::Error::custom)
    }
}

fn format_block_hash(hash: &alloy_primitives::B256) -> String {
    format!("{hash:#x}")
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
    /// Exact state-owned source-read budget for joint-tip resolution.
    ///
    /// This must equal [`EVM_JOINT_TIP_SOURCE_READS`].
    pub max_source_reads: NonZeroU64,
}

/// Validates joint-tip resolve config.
pub fn validate_resolve_evm_joint_tip_config(
    config: &ResolveEvmJointTipConfig,
) -> Result<(), String> {
    LocalPublicId::new(&config.network).map_err(|error| error.to_string())?;
    if config.chain_id == 0 {
        return Err("chain_id must be non-zero".to_owned());
    }
    if config.max_source_reads.get() != EVM_JOINT_TIP_SOURCE_READS {
        return Err(format!(
            "max_source_reads must equal {EVM_JOINT_TIP_SOURCE_READS} for EVM joint-tip resolution"
        ));
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
    block: &EvmBlock,
    evidence: &EvmSessionEvidence,
) -> Result<EvmJointTip, EvmStateError> {
    validate_resolve_evm_joint_tip_config(config)
        .map_err(|reason| EvmStateError::InvalidInput { reason })?;
    let tip = EvmJointTip::from_session_block(&config.network, config.chain_id, block, evidence)?;
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
        block: &EvmBlock,
        evidence: &EvmSessionEvidence,
    ) -> Result<EvmJointTip, EvmStateError> {
        materialize_evm_joint_tip(&self.config, block, evidence)
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
    type Caps = (EvmReadCapability,);

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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.evm.state.config.observe_native_balance",
    validate = "validate_observe_evm_native_balance_config"
)]
pub struct ObserveEvmNativeBalanceConfig {
    /// Exact semantic EVM network that owns the native-asset scale.
    pub network: NetworkConfig,
    /// Account address to observe.
    pub account: String,
    /// Fixed coverage claim written on success (`configured_only`).
    pub coverage: String,
    /// Maximum number of source reads this bounded state may request.
    pub max_source_reads: NonZeroU64,
}

/// Validates native-balance observation config.
pub fn validate_observe_evm_native_balance_config(
    config: &ObserveEvmNativeBalanceConfig,
) -> Result<(), String> {
    let _ = config.evm_network_parts()?;
    validate_canonical_evm_account(&config.account).map_err(|error| error.to_string())?;
    if config.max_source_reads.get() != EVM_NATIVE_BALANCE_OBSERVE_SOURCE_READS {
        return Err(format!(
            "max_source_reads must equal {EVM_NATIVE_BALANCE_OBSERVE_SOURCE_READS} for native balance observation"
        ));
    }
    if config.coverage != EVM_NATIVE_BALANCE_COVERAGE {
        return Err(format!(
            "coverage must equal {EVM_NATIVE_BALANCE_COVERAGE} for native balance observation"
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
    /// Returns the semantic network id, EVM chain id, and native scale.
    pub fn evm_network_parts(&self) -> Result<(&str, u64, u8), String> {
        if self.network.family() != NetworkFamilyConfig::Evm {
            return Err("native EVM balance observation requires an EVM network".to_owned());
        }
        let chain_id = self
            .network
            .chain_id_u64()
            .ok_or_else(|| "EVM network is missing chain_id".to_owned())?;
        let native_decimals = self
            .network
            .native_decimals()
            .ok_or_else(|| "EVM network is missing native_decimals".to_owned())?;
        Ok((
            self.network.network_id().as_str(),
            chain_id,
            native_decimals,
        ))
    }

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
    balance_evidence: &EvmSessionEvidence,
) -> Result<EvmAddressNativeBalanceObservation, EvmStateError> {
    validate_observe_evm_native_balance_config(config)
        .map_err(|reason| EvmStateError::InvalidInput { reason })?;
    if !joint_tip.is_admissible_for_balance_write() {
        return Err(EvmStateError::InvalidInput {
            reason: "joint tip is not admissible for balance write".to_owned(),
        });
    }
    let (network_id, chain_id, native_decimals) = config
        .evm_network_parts()
        .map_err(|reason| EvmStateError::InvalidInput { reason })?;
    if joint_tip.network() != network_id || joint_tip.chain_id() != chain_id {
        return Err(EvmStateError::InvalidInput {
            reason: "joint tip binding does not match native balance config".to_owned(),
        });
    }
    if verified_tip != joint_tip {
        return Err(EvmStateError::InvalidInput {
            reason: "tip drift, hash, or provider-source mismatch before Platform write".to_owned(),
        });
    }
    if !joint_tip.source_binding().matches_session(balance_evidence) {
        return Err(EvmStateError::InvalidInput {
            reason: "balance response evidence did not match the shared provider source".to_owned(),
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
        network_id.to_owned(),
        chain_id,
        config.account.clone(),
    )?;
    let response = EvmAddressNativeBalanceResponse::new(
        joint_tip.block_number(),
        joint_tip.block_hash(),
        balance_wei_decimal,
        native_decimals,
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
    balance_wei: &alloy_primitives::U256,
    evidence: &EvmSessionEvidence,
) -> Result<EvmAddressNativeBalanceObservation, EvmStateError> {
    normalize_evm_native_balance_observation(
        config,
        joint_tip,
        verified_tip,
        &balance_wei.to_string(),
        evidence,
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
        balance_wei: &alloy_primitives::U256,
        evidence: &EvmSessionEvidence,
    ) -> Result<EvmAddressNativeBalanceObservation, EvmStateError> {
        normalize_evm_native_balance_from_capability(
            &self.config,
            &input.joint_tip,
            verified_tip,
            balance_wei,
            evidence,
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
    type Caps = (EvmReadCapability,);

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

/// Exact EVM native-balance source key carried by a collection receipt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "native_balance_source_key",
    version = "1",
    schema = "mfm.evm.collection.native_balance_source_key"
)]
pub struct EvmNativeBalanceSourceKey {
    network: String,
    chain_id: u64,
    account: String,
}

impl EvmNativeBalanceSourceKey {
    fn from_fact(fact: &EvmAddressNativeBalanceSnapshotFact) -> Result<Self, EvmStateError> {
        let subject = fact.subject();
        let subject = EvmAddressNativeBalanceSubject::new(
            subject.network(),
            subject.chain_id(),
            subject.account(),
        )?;
        Ok(Self {
            network: subject.network().to_owned(),
            chain_id: subject.chain_id(),
            account: subject.account().to_owned(),
        })
    }

    /// Returns the semantic EVM network id.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Returns the EVM chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the canonical observed account.
    pub fn account(&self) -> &str {
        &self.account
    }
}

/// One checked EVM native-balance fact included in a resource receipt.
///
/// The embedded fact is private hydration material used only to re-derive and verify the content
/// identity when this receipt is decoded. It is not a report-selection shortcut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "native_balance_receipt_entry",
    version = "1",
    schema = "mfm.evm.collection.native_balance_receipt_entry"
)]
pub struct EvmNativeBalanceReceiptEntry {
    source_key: EvmNativeBalanceSourceKey,
    block_number: u64,
    block_hash: String,
    coverage: String,
    source_status: String,
    fact_content_identity: mfm_facts::FactContentIdentity,
    verified_fact: EvmAddressNativeBalanceSnapshotFact,
}

impl EvmNativeBalanceReceiptEntry {
    fn from_verified_fact(
        fact: &EvmAddressNativeBalanceSnapshotFact,
    ) -> Result<Self, EvmStateError> {
        let source_key = EvmNativeBalanceSourceKey::from_fact(fact)?;
        let response = fact.response();
        let coverage = response.coverage_status()?;
        let source_status = response.holding_source_status()?;
        if coverage.as_str() != EVM_NATIVE_BALANCE_COVERAGE
            || source_status.as_str() != EVM_NATIVE_BALANCE_SOURCE_STATUS
        {
            return Err(EvmStateError::InvalidInput {
                reason: "receipt fact did not use the fixed EVM native coverage/status".to_owned(),
            });
        }
        let rebuilt_response = EvmAddressNativeBalanceResponse::new(
            response.block_number(),
            response.block_hash(),
            response.raw_wei(),
            response.decimals(),
            coverage,
            source_status,
        )?;
        let rebuilt_subject = EvmAddressNativeBalanceSubject::new(
            source_key.network(),
            source_key.chain_id(),
            source_key.account(),
        )?;
        let rebuilt_fact =
            EvmAddressNativeBalanceSnapshotFact::new(rebuilt_subject, rebuilt_response);
        if &rebuilt_fact != fact {
            return Err(EvmStateError::InvalidInput {
                reason: "receipt fact did not satisfy the EVM native-balance fact contract"
                    .to_owned(),
            });
        }
        let descriptor = EvmAddressNativeBalanceSnapshotFact::descriptor().map_err(|error| {
            EvmStateError::InvalidInput {
                reason: error.to_string(),
            }
        })?;
        let fact_content_identity = mfm_facts::derive_fact_content_identity_from_typed_values(
            &descriptor,
            fact.subject(),
            fact.response(),
        )
        .map_err(|error| EvmStateError::InvalidInput {
            reason: error.to_string(),
        })?;
        Ok(Self {
            source_key,
            block_number: response.block_number(),
            block_hash: response.block_hash().to_owned(),
            coverage: coverage.as_str().to_owned(),
            source_status: source_status.as_str().to_owned(),
            fact_content_identity,
            verified_fact: fact.clone(),
        })
    }

    /// Returns the exact family-specific source key.
    pub const fn source_key(&self) -> &EvmNativeBalanceSourceKey {
        &self.source_key
    }

    /// Returns the exact anchor block number.
    pub const fn block_number(&self) -> u64 {
        self.block_number
    }

    /// Returns the exact canonical anchor block hash.
    pub fn block_hash(&self) -> &str {
        &self.block_hash
    }

    /// Returns the fixed admissible coverage tag.
    pub fn coverage(&self) -> &str {
        &self.coverage
    }

    /// Returns the fixed successful source-status tag.
    pub fn source_status(&self) -> &str {
        &self.source_status
    }

    /// Returns the checked fact-content identity.
    pub const fn fact_content_identity(&self) -> &mfm_facts::FactContentIdentity {
        &self.fact_content_identity
    }
}

impl<'de> Deserialize<'de> for EvmNativeBalanceReceiptEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            source_key: EvmNativeBalanceSourceKey,
            block_number: u64,
            block_hash: String,
            coverage: String,
            source_status: String,
            fact_content_identity: serde_json::Value,
            verified_fact: EvmAddressNativeBalanceSnapshotFact,
        }

        let wire = Wire::deserialize(deserializer)?;
        let expected = Self::from_verified_fact(&wire.verified_fact).map_err(de::Error::custom)?;
        let descriptor =
            EvmAddressNativeBalanceSnapshotFact::descriptor().map_err(de::Error::custom)?;
        let identity = mfm_facts::verify_serialized_fact_content_identity_from_typed_values(
            &wire.fact_content_identity,
            &descriptor,
            wire.verified_fact.subject(),
            wire.verified_fact.response(),
        )
        .map_err(de::Error::custom)?;
        if wire.source_key != expected.source_key
            || wire.block_number != expected.block_number
            || wire.block_hash != expected.block_hash
            || wire.coverage != expected.coverage
            || wire.source_status != expected.source_status
            || identity != expected.fact_content_identity
        {
            return Err(de::Error::custom(
                "EVM native balance receipt entry did not match verified fact material",
            ));
        }
        Ok(expected)
    }
}

/// Config for assembling an exact EVM native-balance resource receipt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(schema = "mfm.evm.state.config.assemble_native_balance_receipt")]
pub struct AssembleEvmNativeBalanceBatchReceiptConfig {}

/// Input for assembling an exact EVM native-balance resource receipt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.evm.state.input.assemble_native_balance_receipt")]
pub struct AssembleEvmNativeBalanceBatchReceiptInput {
    /// Joint tip shared by every recorded native fact.
    pub joint_tip: EvmJointTip,
    /// Completed managed fact outputs for every configured EVM native source.
    pub balance_facts: NonEmpty<EvmAddressNativeBalanceSnapshotFact>,
}

/// Exact successful EVM native-balance collection at one shared network anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "native_balance_batch_receipt",
    version = "1",
    schema = "mfm.evm.collection.native_balance_batch_receipt"
)]
pub struct EvmNativeBalanceBatchReceipt {
    network: String,
    chain_id: u64,
    block_number: u64,
    block_hash: String,
    source_binding: RedactedEvmSessionEvidence,
    successful_observation_count: u64,
    entries: Vec<EvmNativeBalanceReceiptEntry>,
}

impl EvmNativeBalanceBatchReceipt {
    fn from_entries(
        network: String,
        chain_id: u64,
        block_number: u64,
        block_hash: String,
        source_binding: RedactedEvmSessionEvidence,
        successful_observation_count: u64,
        entries: Vec<EvmNativeBalanceReceiptEntry>,
    ) -> Result<Self, EvmStateError> {
        if network.trim().is_empty() || chain_id == 0 || entries.is_empty() {
            return Err(EvmStateError::InvalidInput {
                reason:
                    "EVM native balance receipt requires a non-empty network, chain, and entries"
                        .to_owned(),
            });
        }
        if !source_binding.is_bound_to(&network, chain_id) {
            return Err(EvmStateError::InvalidInput {
                reason: "EVM native balance receipt source binding did not match its network"
                    .to_owned(),
            });
        }
        let expected_count =
            u64::try_from(entries.len()).map_err(|_| EvmStateError::InvalidInput {
                reason: "EVM native balance receipt entry count overflowed u64".to_owned(),
            })?;
        if successful_observation_count != expected_count {
            return Err(EvmStateError::InvalidInput {
                reason: "EVM native balance receipt successful count did not match entries"
                    .to_owned(),
            });
        }
        let block_hash = crate::canonical_evm_block_hash(block_hash)?;
        let mut previous = None;
        for entry in &entries {
            if entry.source_key.network != network
                || entry.source_key.chain_id != chain_id
                || entry.block_number != block_number
                || entry.block_hash != block_hash
            {
                return Err(EvmStateError::InvalidInput {
                    reason: "EVM native balance receipt entry did not match network anchor binding"
                        .to_owned(),
                });
            }
            if entry.coverage != EVM_NATIVE_BALANCE_COVERAGE
                || entry.source_status != EVM_NATIVE_BALANCE_SOURCE_STATUS
            {
                return Err(EvmStateError::InvalidInput {
                    reason: "EVM native balance receipt entry did not use fixed coverage/status"
                        .to_owned(),
                });
            }
            if previous
                .as_ref()
                .is_some_and(|key: &&EvmNativeBalanceSourceKey| *key >= &entry.source_key)
            {
                return Err(EvmStateError::InvalidInput {
                    reason: "EVM native balance receipt entries were not strictly sorted"
                        .to_owned(),
                });
            }
            previous = Some(&entry.source_key);
        }
        Ok(Self {
            network,
            chain_id,
            block_number,
            block_hash,
            source_binding,
            successful_observation_count,
            entries,
        })
    }

    /// Returns the semantic EVM network id.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Returns the EVM chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the shared anchor block number.
    pub const fn block_number(&self) -> u64 {
        self.block_number
    }

    /// Returns the shared canonical anchor block hash.
    pub fn block_hash(&self) -> &str {
        &self.block_hash
    }

    /// Returns the exact redacted provider source for this collection receipt.
    pub const fn source_binding(&self) -> &RedactedEvmSessionEvidence {
        &self.source_binding
    }

    /// Returns the completed source observation count.
    pub const fn successful_observation_count(&self) -> u64 {
        self.successful_observation_count
    }

    /// Returns entries in strict source-key order.
    pub fn entries(&self) -> &[EvmNativeBalanceReceiptEntry] {
        &self.entries
    }
}

impl<'de> Deserialize<'de> for EvmNativeBalanceBatchReceipt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            network: String,
            chain_id: u64,
            block_number: u64,
            block_hash: String,
            source_binding: RedactedEvmSessionEvidence,
            successful_observation_count: u64,
            entries: Vec<EvmNativeBalanceReceiptEntry>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::from_entries(
            wire.network,
            wire.chain_id,
            wire.block_number,
            wire.block_hash,
            wire.source_binding,
            wire.successful_observation_count,
            wire.entries,
        )
        .map_err(de::Error::custom)
    }
}

/// Pure state that assembles one exact EVM native-balance resource receipt.
pub struct AssembleEvmNativeBalanceBatchReceiptState;

impl StateSpec for AssembleEvmNativeBalanceBatchReceiptState {
    type Config = AssembleEvmNativeBalanceBatchReceiptConfig;
    type Context = NoContext;
    type Input = AssembleEvmNativeBalanceBatchReceiptInput;
    type Output = EvmNativeBalanceBatchReceipt;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("native_balance.assemble_receipt")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("native_balance.assemble_receipt")
    }

    fn name() -> &'static str {
        "mfm.evm.native_balance.assemble_receipt"
    }

    fn new(_config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self)
    }
}

impl PureState for AssembleEvmNativeBalanceBatchReceiptState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        assemble_evm_native_balance_batch_receipt(input)
    }
}

/// Derives one exact EVM native-balance receipt from completed managed fact outputs.
pub fn assemble_evm_native_balance_batch_receipt(
    input: AssembleEvmNativeBalanceBatchReceiptInput,
) -> StateResult<EvmNativeBalanceBatchReceipt> {
    let mut entries = input
        .balance_facts
        .values()
        .iter()
        .map(EvmNativeBalanceReceiptEntry::from_verified_fact)
        .collect::<Result<Vec<_>, _>>()
        .map_err(StateError::from)?;
    entries.sort_by(|left, right| left.source_key.cmp(&right.source_key));
    let count = u64::try_from(entries.len()).map_err(|_| {
        StateError::from(EvmStateError::InvalidInput {
            reason: "EVM native balance receipt entry count overflowed u64".to_owned(),
        })
    })?;
    EvmNativeBalanceBatchReceipt::from_entries(
        input.joint_tip.network().to_owned(),
        input.joint_tip.chain_id(),
        input.joint_tip.block_number(),
        input.joint_tip.block_hash().to_owned(),
        input.joint_tip.source_binding().clone(),
        count,
        entries,
    )
    .map_err(StateError::from)
}

#[cfg(test)]
#[path = "native_balance_collect_tests.rs"]
mod tests;
