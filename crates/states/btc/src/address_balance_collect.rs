//! Bitcoin address-balance collector states: joint tip → observe → record.
//!
//! Snapshot pattern only: resolve one joint tip per same-network batch, pin balance
//! reads at that height+hash, fail closed before Platform write when prove-before-write
//! invariants do not hold.

use std::future;
use std::num::NonZeroU64;
use std::str::FromStr;

use mfm_btc_capabilities::{
    BtcAddress, BtcBalanceReadCapability, BtcBalanceReadResponse, BtcBlockHash,
    BtcChainHeadReadCapability, BtcChainHeadResponse as CapabilityChainHeadResponse,
    BtcHeadSelection, BtcNetworkId, BtcSourceIdentity, BtcSourceStatus,
};
use mfm_capabilities::NoCaps;
use mfm_effects::{ManagedPlatformWrite, Pure, ReadExternal};
use mfm_fact_capabilities::FactRecordCapability;
use mfm_ids::{StateKind, StateVersion};
use mfm_portfolio_model::holding::{CoverageStatus, HoldingSourceStatus};
use mfm_program::{
    fact_descriptor_ref, AdapterBindingSpec, FactDescriptorRef, ManagedWriteState, NoContext,
    PureState, ReadState, StateError, StateResult, StateSpec, ValidatedConfig,
};
use mfm_program_derive::{MfmConfig, MfmValue, StateInput};
use mfm_values::NonEmpty;
use serde::{Deserialize, Serialize};

use crate::address_balance::{
    address_balance_fact_visibility, BtcAddressBalanceResponse, BtcAddressBalanceSnapshotFact,
    BtcAddressBalanceSubject,
};
use crate::{
    adapter_binding, adapter_required_error, state_kind, state_version, validate_bitcoin_network,
    BtcStateError,
};

/// Exact number of source reads required to resolve a Bitcoin balance batch joint tip.
pub const BTC_JOINT_TIP_SOURCE_READS: u64 = 1;
/// Exact number of source reads required for one Bitcoin address-balance observation.
pub const BTC_NATIVE_BALANCE_OBSERVE_SOURCE_READS: u64 = 1;
/// Closed coverage claim for a successful configured Bitcoin native source observation.
pub const BTC_NATIVE_BALANCE_COVERAGE: &str = "configured_only";

/// Shared joint tip resolved once for a same-network multi-subject batch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "joint_tip",
    version = "1",
    schema = "mfm.bitcoin.state.value.joint_tip"
)]
pub struct BtcJointTip {
    network: String,
    bitcoin_network: String,
    semantic_source_identity: String,
    block_height: u64,
    block_hash: String,
    observed_source_status: String,
    observed_bitcoin_network: String,
}

impl BtcJointTip {
    /// Creates a joint tip from already-checked components.
    pub fn new(
        network: impl Into<String>,
        bitcoin_network: impl Into<String>,
        semantic_source_identity: impl Into<String>,
        block_height: u64,
        block_hash: impl Into<String>,
        observed_source_status: impl Into<String>,
        observed_bitcoin_network: impl Into<String>,
    ) -> Result<Self, BtcStateError> {
        let network = network.into();
        let bitcoin_network = bitcoin_network.into();
        let semantic_source_identity = semantic_source_identity.into();
        let block_hash = block_hash.into();
        let observed_source_status = observed_source_status.into();
        let observed_bitcoin_network = observed_bitcoin_network.into();
        if network.trim().is_empty()
            || bitcoin_network.trim().is_empty()
            || semantic_source_identity.trim().is_empty()
            || block_hash.trim().is_empty()
            || observed_source_status.trim().is_empty()
            || observed_bitcoin_network.trim().is_empty()
        {
            return Err(BtcStateError::InvalidInput {
                reason: "joint tip fields must be non-empty".to_owned(),
            });
        }
        validate_bitcoin_network(&bitcoin_network)
            .map_err(|reason| BtcStateError::InvalidInput { reason })?;
        validate_bitcoin_network(&observed_bitcoin_network)
            .map_err(|reason| BtcStateError::InvalidInput { reason })?;
        BtcBlockHash::new(&block_hash).map_err(|_| BtcStateError::InvalidInput {
            reason: "joint tip block_hash must be a 32-byte hex hash".to_owned(),
        })?;
        Ok(Self {
            network,
            bitcoin_network,
            semantic_source_identity,
            block_height,
            block_hash,
            observed_source_status,
            observed_bitcoin_network,
        })
    }

    /// Materializes a joint tip from a verified chain-head capability response.
    pub fn from_capability_response(
        response: &CapabilityChainHeadResponse,
    ) -> Result<Self, BtcStateError> {
        let evidence = &response.evidence;
        Self::new(
            evidence.network_id.as_str(),
            evidence.bitcoin_network.as_str(),
            evidence.source_identity.as_str(),
            response.block_height,
            response.block_hash.as_str(),
            evidence.source_status.as_str(),
            evidence.observed_bitcoin_network.as_str(),
        )
    }

    /// Returns the semantic network id.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Returns the Bitcoin Core network tag.
    pub fn bitcoin_network(&self) -> &str {
        &self.bitcoin_network
    }

    /// Returns the non-secret semantic source identity.
    pub fn semantic_source_identity(&self) -> &str {
        &self.semantic_source_identity
    }

    /// Returns the joint tip block height.
    pub const fn block_height(&self) -> u64 {
        self.block_height
    }

    /// Returns the joint tip block hash.
    pub fn block_hash(&self) -> &str {
        &self.block_hash
    }

    /// Returns the provider source status tag (`synced`, `initial_block_download`, `unknown`).
    pub fn observed_source_status(&self) -> &str {
        &self.observed_source_status
    }

    /// Returns the provider-observed Bitcoin network tag.
    pub fn observed_bitcoin_network(&self) -> &str {
        &self.observed_bitcoin_network
    }

    /// Returns whether this tip is admissible as a balance-write anchor.
    pub fn is_admissible_for_balance_write(&self) -> bool {
        self.observed_source_status == BtcSourceStatus::Synced.as_str()
            && self.observed_bitcoin_network == self.bitcoin_network
            && !self.block_hash.trim().is_empty()
    }
}

/// Requires every observation in a same-network batch to share one joint tip anchor.
pub fn require_shared_joint_tip(
    observations: &[&BtcAddressBalanceObservation],
) -> Result<(), BtcStateError> {
    let Some(first) = observations.first() else {
        return Err(BtcStateError::InvalidInput {
            reason: "shared tip batch requires at least one observation".to_owned(),
        });
    };
    let height = first.response().anchor_height();
    let hash = first.response().anchor_hash();
    let network = first.subject().network();
    for observation in observations.iter().skip(1) {
        if observation.subject().network() != network {
            return Err(BtcStateError::InvalidInput {
                reason: "shared tip batch subjects must share one network".to_owned(),
            });
        }
        if observation.response().anchor_height() != height
            || observation.response().anchor_hash() != hash
        {
            return Err(BtcStateError::InvalidInput {
                reason: "multi-subject same-network batch must share one joint tip".to_owned(),
            });
        }
    }
    Ok(())
}

/// Config for resolving a joint tip once per same-network collector batch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(
    schema = "mfm.bitcoin.state.config.resolve_joint_tip",
    validate = "validate_resolve_btc_joint_tip_config"
)]
pub struct ResolveBtcJointTipConfig {
    /// Semantic network id.
    pub network: String,
    /// Expected Bitcoin Core network tag (`main`, `test`, `signet`, or `regtest`).
    pub bitcoin_network: String,
    /// Non-secret semantic source identity.
    pub semantic_source_identity: String,
    /// Maximum number of source reads this bounded state may request.
    pub max_source_reads: NonZeroU64,
}

/// Validates joint-tip resolve config.
pub fn validate_resolve_btc_joint_tip_config(
    config: &ResolveBtcJointTipConfig,
) -> Result<(), String> {
    BtcNetworkId::new(&config.network).map_err(|error| error.to_string())?;
    validate_bitcoin_network(&config.bitcoin_network)?;
    BtcSourceIdentity::new(&config.semantic_source_identity).map_err(|error| error.to_string())?;
    Ok(())
}

impl ResolveBtcJointTipConfig {
    /// Returns the fixed best-tip selection for address-balance collection.
    pub const fn selection(&self) -> BtcHeadSelection {
        BtcHeadSelection::best()
    }
}

/// Input for joint-tip resolution (adapter-supplied observation context only).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.bitcoin.state.input.resolve_joint_tip")]
pub struct ResolveBtcJointTipInput {
    /// Optional observation context supplied by the adapter.
    pub context: BtcAddressBalanceObservationContext,
}

/// Adapter-supplied context for balance observation materialization.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "address_balance_observation_context",
    version = "1",
    schema = "mfm.bitcoin.state.value.address_balance_observation_context"
)]
pub struct BtcAddressBalanceObservationContext {
    /// State observation time in Unix milliseconds, when supplied by an adapter.
    pub observed_at_unix_ms: Option<u64>,
}

/// State contract for resolving one joint tip for a same-network batch.
pub struct ResolveBtcJointTipState {
    config: ResolveBtcJointTipConfig,
}

impl ResolveBtcJointTipState {
    /// Materializes a joint tip from a verified chain-head capability response.
    pub fn materialize_response(
        &self,
        response: &CapabilityChainHeadResponse,
    ) -> Result<BtcJointTip, BtcStateError> {
        materialize_btc_joint_tip(&self.config, response)
    }
}

/// Materializes and admits a joint tip from a verified chain-head response.
pub fn materialize_btc_joint_tip(
    config: &ResolveBtcJointTipConfig,
    response: &CapabilityChainHeadResponse,
) -> Result<BtcJointTip, BtcStateError> {
    validate_resolve_btc_joint_tip_config(config)
        .map_err(|reason| BtcStateError::InvalidInput { reason })?;
    let tip = BtcJointTip::from_capability_response(response)?;
    if tip.network() != config.network
        || tip.bitcoin_network() != config.bitcoin_network
        || tip.semantic_source_identity() != config.semantic_source_identity
    {
        return Err(BtcStateError::SourceMismatch);
    }
    if !tip.is_admissible_for_balance_write() {
        return Err(BtcStateError::InvalidInput {
            reason: "joint tip is not admissible for balance write (source must be synced)"
                .to_owned(),
        });
    }
    Ok(tip)
}

impl StateSpec for ResolveBtcJointTipState {
    type Config = ResolveBtcJointTipConfig;
    type Context = NoContext;
    type Input = ResolveBtcJointTipInput;
    type Output = BtcJointTip;
    type Effect = ReadExternal;
    type Caps = (BtcChainHeadReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("joint_tip.resolve")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("joint_tip.resolve")
    }

    fn name() -> &'static str {
        "mfm.bitcoin.joint_tip.resolve"
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

impl ReadState for ResolveBtcJointTipState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        _input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        future::ready(Err(adapter_required_error(Self::name())))
    }
}

/// Config for a Bitcoin address-balance observation state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(
    schema = "mfm.bitcoin.state.config.observe_address_balance",
    validate = "validate_observe_btc_address_balance_config"
)]
pub struct ObserveBtcAddressBalanceConfig {
    /// Semantic network id.
    pub network: String,
    /// Expected Bitcoin Core network tag.
    pub bitcoin_network: String,
    /// Non-secret semantic source identity.
    pub semantic_source_identity: String,
    /// Public Bitcoin address to observe.
    pub address: String,
    /// Coverage claim written on success (`configured_only` or `complete_at_anchor`).
    pub coverage: String,
    /// Maximum number of source reads this bounded state may request.
    pub max_source_reads: NonZeroU64,
}

/// Validates address-balance observation config.
pub fn validate_observe_btc_address_balance_config(
    config: &ObserveBtcAddressBalanceConfig,
) -> Result<(), String> {
    BtcNetworkId::new(&config.network).map_err(|error| error.to_string())?;
    validate_bitcoin_network(&config.bitcoin_network)?;
    BtcSourceIdentity::new(&config.semantic_source_identity).map_err(|error| error.to_string())?;
    BtcAddress::new(&config.address)
        .map_err(|_| "address is not a supported Bitcoin address".to_owned())?;
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

impl ObserveBtcAddressBalanceConfig {
    /// Parses the configured coverage claim.
    pub fn coverage_status(&self) -> Result<CoverageStatus, BtcStateError> {
        validate_observe_btc_address_balance_config(self)
            .map_err(|reason| BtcStateError::InvalidInput { reason })?;
        CoverageStatus::from_str(&self.coverage).map_err(|_| BtcStateError::InvalidInput {
            reason: format!("unknown coverage status {:?}", self.coverage),
        })
    }
}

/// Input for a pinned address-balance observation (joint tip is required).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.bitcoin.state.input.observe_address_balance")]
pub struct ObserveBtcAddressBalanceInput {
    /// Joint tip resolved once for this same-network batch.
    pub joint_tip: BtcJointTip,
    /// Optional observation context supplied by the adapter.
    pub context: BtcAddressBalanceObservationContext,
}

/// Normalized address-balance observation state output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "address_balance_observation",
    version = "1",
    schema = "mfm.bitcoin.state.output.address_balance_observation"
)]
pub struct BtcAddressBalanceObservation {
    subject: BtcAddressBalanceSubject,
    response: BtcAddressBalanceResponse,
    source_read_count: u64,
}

impl BtcAddressBalanceObservation {
    /// Creates normalized observation output.
    pub const fn new(
        subject: BtcAddressBalanceSubject,
        response: BtcAddressBalanceResponse,
        source_read_count: u64,
    ) -> Self {
        Self {
            subject,
            response,
            source_read_count,
        }
    }

    /// Returns the normalized subject.
    pub const fn subject(&self) -> &BtcAddressBalanceSubject {
        &self.subject
    }

    /// Returns the normalized response.
    pub const fn response(&self) -> &BtcAddressBalanceResponse {
        &self.response
    }

    /// Builds the platform fact from this observation.
    pub fn to_fact(&self) -> BtcAddressBalanceSnapshotFact {
        BtcAddressBalanceSnapshotFact::new(self.subject.clone(), self.response.clone())
    }

    /// Converts this observation into the platform fact.
    pub fn into_fact(self) -> BtcAddressBalanceSnapshotFact {
        BtcAddressBalanceSnapshotFact::new(self.subject, self.response)
    }

    /// Returns the number of source reads used by this bounded observation.
    pub const fn source_read_count(&self) -> u64 {
        self.source_read_count
    }
}

/// Proves balance@joint-tip before constructing an admissible observation DTO.
///
/// Fail closed on missing hash, tip/hash mismatch, address mismatch, inadmissible
/// coverage/status, or unsynced joint tip.
pub fn normalize_btc_address_balance_observation(
    config: &ObserveBtcAddressBalanceConfig,
    joint_tip: &BtcJointTip,
    balance: &BtcBalanceReadResponse,
) -> Result<BtcAddressBalanceObservation, BtcStateError> {
    validate_observe_btc_address_balance_config(config)
        .map_err(|reason| BtcStateError::InvalidInput { reason })?;
    if !joint_tip.is_admissible_for_balance_write() {
        return Err(BtcStateError::InvalidInput {
            reason: "joint tip is not admissible for balance write".to_owned(),
        });
    }
    if joint_tip.network() != config.network
        || joint_tip.bitcoin_network() != config.bitcoin_network
        || joint_tip.semantic_source_identity() != config.semantic_source_identity
    {
        return Err(BtcStateError::InvalidInput {
            reason: "joint tip binding does not match address balance config".to_owned(),
        });
    }
    if balance.address.as_str() != config.address {
        return Err(BtcStateError::InvalidInput {
            reason: "balance response address does not match observation config".to_owned(),
        });
    }
    if balance.block_height != joint_tip.block_height()
        || balance.block_hash.as_str() != joint_tip.block_hash()
    {
        return Err(BtcStateError::InvalidInput {
            reason: "balance tip drift or hash mismatch before Platform write".to_owned(),
        });
    }
    let evidence = &balance.evidence;
    if evidence.network_id.as_str() != config.network
        || evidence.source_identity.as_str() != config.semantic_source_identity
        || evidence.bitcoin_network != config.bitcoin_network
        || evidence.observed_bitcoin_network != config.bitcoin_network
    {
        return Err(BtcStateError::SourceMismatch);
    }
    if evidence.source_status != BtcSourceStatus::Synced {
        return Err(BtcStateError::InvalidInput {
            reason: "balance source must be synced for Platform write".to_owned(),
        });
    }
    let coverage = config.coverage_status()?;
    let subject = BtcAddressBalanceSubject::new(
        config.network.clone(),
        config.bitcoin_network.clone(),
        config.semantic_source_identity.clone(),
        config.address.clone(),
    )?;
    let response = BtcAddressBalanceResponse::new(
        joint_tip.block_height(),
        joint_tip.block_hash(),
        balance.balance_sats,
        coverage,
        HoldingSourceStatus::Ok,
    )?;
    Ok(BtcAddressBalanceObservation::new(subject, response, 1))
}

/// State contract for a pinned Bitcoin address-balance observation.
pub struct ObserveBtcAddressBalanceState {
    config: ObserveBtcAddressBalanceConfig,
}

impl ObserveBtcAddressBalanceState {
    /// Materializes a normalized observation from joint tip + verified balance response.
    pub fn materialize_response(
        &self,
        input: &ObserveBtcAddressBalanceInput,
        balance: &BtcBalanceReadResponse,
    ) -> Result<BtcAddressBalanceObservation, BtcStateError> {
        normalize_btc_address_balance_observation(&self.config, &input.joint_tip, balance)
    }

    /// Returns the certified config.
    pub fn config(&self) -> &ObserveBtcAddressBalanceConfig {
        &self.config
    }
}

impl StateSpec for ObserveBtcAddressBalanceState {
    type Config = ObserveBtcAddressBalanceConfig;
    type Context = NoContext;
    type Input = ObserveBtcAddressBalanceInput;
    type Output = BtcAddressBalanceObservation;
    type Effect = ReadExternal;
    type Caps = (BtcBalanceReadCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("address_balance.observe")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("address_balance.observe")
    }

    fn name() -> &'static str {
        "mfm.bitcoin.address_balance.observe"
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

impl ReadState for ObserveBtcAddressBalanceState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        if let Err(error) = validate_observe_btc_address_balance_config(&self.config) {
            return future::ready(Err(StateError::from(BtcStateError::InvalidInput {
                reason: error,
            })));
        }
        if !input.joint_tip.is_admissible_for_balance_write() {
            return future::ready(Err(StateError::from(BtcStateError::InvalidInput {
                reason: "joint tip is not admissible for balance write".to_owned(),
            })));
        }
        future::ready(Err(adapter_required_error(Self::name())))
    }
}

/// Config for recording a Bitcoin address-balance Platform fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(schema = "mfm.bitcoin.state.config.record_address_balance_fact")]
pub struct RecordBtcAddressBalanceFactConfig {}

/// Input for recording a Bitcoin address-balance fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.bitcoin.state.input.record_address_balance_fact")]
pub struct RecordBtcAddressBalanceFactInput {
    /// Whole address-balance observation output to transform into a fact.
    pub observation: BtcAddressBalanceObservation,
}

/// State contract for recording a Bitcoin address-balance Platform fact.
pub struct RecordBtcAddressBalanceFactState;

impl StateSpec for RecordBtcAddressBalanceFactState {
    type Config = RecordBtcAddressBalanceFactConfig;
    type Context = NoContext;
    type Input = RecordBtcAddressBalanceFactInput;
    type Output = BtcAddressBalanceSnapshotFact;
    type Effect = ManagedPlatformWrite;
    type Caps = (FactRecordCapability,);

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("address_balance.record")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("address_balance.record")
    }

    fn name() -> &'static str {
        "mfm.bitcoin.address_balance.record"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn emitted_fact_descriptors() -> mfm_program::Result<Vec<FactDescriptorRef>> {
        Ok(vec![fact_descriptor_ref::<BtcAddressBalanceSnapshotFact>()?])
    }

    fn new(_config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self)
    }
}

impl ManagedWriteState for RecordBtcAddressBalanceFactState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        // Re-admit response before write (fail closed on tampered observation DTOs).
        let response = input.observation.response();
        match BtcAddressBalanceResponse::new(
            response.anchor_height(),
            response.anchor_hash(),
            response.balance_sats(),
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

/// Returns Platform visibility for address-balance facts (adapter record runner).
pub fn address_balance_record_visibility() -> mfm_facts::FactVisibility {
    address_balance_fact_visibility()
}

/// Config for assembling a multi-address balance batch summary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(schema = "mfm.bitcoin.state.config.assemble_address_balance_batch")]
pub struct AssembleBtcAddressBalanceBatchConfig {}

/// Input for assembling a multi-address balance batch summary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.bitcoin.state.input.assemble_address_balance_batch")]
pub struct AssembleBtcAddressBalanceBatchInput {
    /// Joint tip shared by the batch.
    pub joint_tip: BtcJointTip,
    /// Recorded address-balance facts (at least one).
    pub balance_facts: NonEmpty<BtcAddressBalanceSnapshotFact>,
}

/// Summary of a multi-address same-network balance collector batch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "address_balance_batch_summary",
    version = "1",
    schema = "mfm.bitcoin.state.output.address_balance_batch_summary"
)]
pub struct BtcAddressBalanceBatchSummary {
    network: String,
    bitcoin_network: String,
    semantic_source_identity: String,
    joint_tip_height: u64,
    joint_tip_hash: String,
    address_count: u64,
}

impl BtcAddressBalanceBatchSummary {
    /// Returns the semantic network id.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// Returns the joint tip height shared by the batch.
    pub const fn joint_tip_height(&self) -> u64 {
        self.joint_tip_height
    }

    /// Returns the joint tip hash shared by the batch.
    pub fn joint_tip_hash(&self) -> &str {
        &self.joint_tip_hash
    }

    /// Returns the number of addresses collected.
    pub const fn address_count(&self) -> u64 {
        self.address_count
    }
}

/// Pure state that verifies shared tip and summarizes a balance batch.
pub struct AssembleBtcAddressBalanceBatchState;

impl StateSpec for AssembleBtcAddressBalanceBatchState {
    type Config = AssembleBtcAddressBalanceBatchConfig;
    type Context = NoContext;
    type Input = AssembleBtcAddressBalanceBatchInput;
    type Output = BtcAddressBalanceBatchSummary;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("address_balance.assemble_batch")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("address_balance.assemble_batch")
    }

    fn name() -> &'static str {
        "mfm.bitcoin.address_balance.assemble_batch"
    }

    fn new(_config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self)
    }
}

impl PureState for AssembleBtcAddressBalanceBatchState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        assemble_btc_address_balance_batch(input)
    }
}

/// Verifies shared tip across recorded facts and builds a batch summary.
pub fn assemble_btc_address_balance_batch(
    input: AssembleBtcAddressBalanceBatchInput,
) -> StateResult<BtcAddressBalanceBatchSummary> {
    let facts = input.balance_facts.values();
    let observations: Vec<BtcAddressBalanceObservation> = facts
        .iter()
        .map(|fact| {
            BtcAddressBalanceObservation::new(fact.subject().clone(), fact.response().clone(), 1)
        })
        .collect();
    let refs: Vec<&BtcAddressBalanceObservation> = observations.iter().collect();
    require_shared_joint_tip(&refs).map_err(StateError::from)?;
    let first = facts
        .first()
        .expect("NonEmpty guarantees at least one fact");
    if first.response().anchor_height() != input.joint_tip.block_height()
        || first.response().anchor_hash() != input.joint_tip.block_hash()
    {
        return Err(StateError::from(BtcStateError::InvalidInput {
            reason: "recorded facts do not match batch joint tip".to_owned(),
        }));
    }
    if first.subject().network() != input.joint_tip.network()
        || first.subject().bitcoin_network() != input.joint_tip.bitcoin_network()
        || first.subject().semantic_source_identity() != input.joint_tip.semantic_source_identity()
    {
        return Err(StateError::from(BtcStateError::InvalidInput {
            reason: "recorded facts do not match joint tip binding".to_owned(),
        }));
    }
    let address_count = u64::try_from(facts.len()).map_err(|_| {
        StateError::from(BtcStateError::InvalidInput {
            reason: "address count overflow".to_owned(),
        })
    })?;
    Ok(BtcAddressBalanceBatchSummary {
        network: input.joint_tip.network().to_owned(),
        bitcoin_network: input.joint_tip.bitcoin_network().to_owned(),
        semantic_source_identity: input.joint_tip.semantic_source_identity().to_owned(),
        joint_tip_height: input.joint_tip.block_height(),
        joint_tip_hash: input.joint_tip.block_hash().to_owned(),
        address_count,
    })
}

#[cfg(test)]
#[path = "address_balance_collect_tests.rs"]
mod tests;
