//! Bitcoin address-balance collector states: joint tip → observe → record.
//!
//! Snapshot pattern only: resolve one joint tip per same-network batch, pin balance
//! reads at that height+hash, fail closed before Platform write when prove-before-write
//! invariants do not hold.

use std::future;
use std::num::NonZeroU64;
use std::str::FromStr;

use mfm_btc_capabilities::{
    BitcoinAddress, BitcoinBlockHash, BitcoinNetworkId, BitcoinNetworkTag, BitcoinSourceIdentity,
    BtcBalanceReadCapability, BtcBalanceReadResponse, BtcChainHeadReadCapability,
    BtcChainHeadResponse as CapabilityChainHeadResponse, BtcHeadSelection, BtcSourceStatus,
};
use mfm_capabilities::NoCaps;
use mfm_effects::{ManagedPlatformWrite, Pure, ReadExternal};
use mfm_fact_capabilities::FactRecordCapability;
use mfm_ids::{StateKind, StateVersion};
use mfm_portfolio_model::holding::{CoverageStatus, HoldingSourceStatus};
use mfm_program::{
    fact_descriptor_ref, AdapterBindingSpec, ExternalReadEvidenceSet, FactDescriptorRef,
    ManagedWriteState, MfmFactType, NoContext, PureState, ReadState, StateError, StateResult,
    StateSpec, ValidatedConfig,
};
use mfm_program_derive::{MfmConfig, MfmValue, StateInput};
use mfm_values::NonEmpty;
use serde::{de, Deserialize, Serialize};

use crate::address_balance::{
    address_balance_fact_visibility, BtcAddressBalanceResponse, BtcAddressBalanceSnapshotFact,
    BtcAddressBalanceSubject,
};
use crate::{
    adapter_binding, state_kind, state_version, validate_bitcoin_network,
    BtcAddressBalanceReadEvidence, BtcAddressBalanceReadPlan, BtcChainHeadReadEvidence,
    BtcChainHeadReadPlan, BtcStateError,
};

/// Exact number of source reads required to resolve a Bitcoin network collection joint tip.
pub const BTC_JOINT_TIP_SOURCE_READS: u64 = 1;
/// Exact number of source reads required for one Bitcoin address-balance observation.
pub const BTC_NATIVE_BALANCE_OBSERVE_SOURCE_READS: u64 = 1;
/// Closed coverage claim for a successful configured Bitcoin native source observation.
pub const BTC_NATIVE_BALANCE_COVERAGE: &str = "configured_only";
/// Closed source-status claim for a successful Bitcoin native source observation.
pub const BTC_NATIVE_BALANCE_SOURCE_STATUS: &str = "ok";

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
        BitcoinBlockHash::new(&block_hash).map_err(|_| BtcStateError::InvalidInput {
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
            &response.block_hash.to_string(),
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

/// Config for resolving a joint tip once per same-network collector batch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(
    schema = "mfm.bitcoin.state.config.resolve_joint_tip",
    validate = "validate_resolve_btc_joint_tip_config"
)]
pub struct ResolveBtcJointTipConfig {
    /// Semantic network id.
    pub network: String,
    /// Expected supported Bitcoin Core network tag.
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
    BitcoinNetworkId::new(&config.network).map_err(|error| error.to_string())?;
    validate_bitcoin_network(&config.bitcoin_network)?;
    BitcoinSourceIdentity::new(&config.semantic_source_identity)
        .map_err(|error| error.to_string())?;
    if config.max_source_reads.get() != BTC_JOINT_TIP_SOURCE_READS {
        return Err(format!(
            "max_source_reads must equal {BTC_JOINT_TIP_SOURCE_READS} for Bitcoin joint-tip resolution"
        ));
    }
    Ok(())
}

impl ResolveBtcJointTipConfig {
    /// Returns the fixed best-tip selection for address-balance collection.
    pub const fn selection(&self) -> BtcHeadSelection {
        BtcHeadSelection::best()
    }
}

/// Input for joint-tip resolution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.bitcoin.state.input.resolve_joint_tip")]
pub struct ResolveBtcJointTipInput {}

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

    /// Returns the certified state config.
    pub const fn config(&self) -> &ResolveBtcJointTipConfig {
        &self.config
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
    type Plan = BtcChainHeadReadPlan;
    type Evidence = BtcChainHeadReadEvidence;

    fn plan(
        &self,
        _input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Plan> {
        BtcChainHeadReadPlan::new(
            &self.config.network,
            &self.config.bitcoin_network,
            &self.config.semantic_source_identity,
            self.config.selection(),
        )
        .map_err(StateError::from)
    }

    fn reduce(
        &self,
        _input: &Self::Input,
        evidence: &ExternalReadEvidenceSet<Self::Evidence>,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        reject_fact_query_evidence(evidence)?;
        let plan = BtcChainHeadReadPlan::new(
            &self.config.network,
            &self.config.bitcoin_network,
            &self.config.semantic_source_identity,
            self.config.selection(),
        )?;
        let response = evidence.primary_evidence().response(&plan)?;
        self.materialize_response(&response)
            .map_err(StateError::from)
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
    /// Fixed coverage claim written on success (`configured_only`).
    pub coverage: String,
    /// Maximum number of source reads this bounded state may request.
    pub max_source_reads: NonZeroU64,
}

/// Validates address-balance observation config.
pub fn validate_observe_btc_address_balance_config(
    config: &ObserveBtcAddressBalanceConfig,
) -> Result<(), String> {
    BitcoinNetworkId::new(&config.network).map_err(|error| error.to_string())?;
    validate_bitcoin_network(&config.bitcoin_network)?;
    BitcoinSourceIdentity::new(&config.semantic_source_identity)
        .map_err(|error| error.to_string())?;
    let bitcoin_network = BitcoinNetworkTag::new(&config.bitcoin_network)
        .map_err(|_| "bitcoin_network must name a supported Bitcoin Core chain".to_owned())?;
    let address = BitcoinAddress::new(&config.address)
        .map_err(|_| "address is not a canonical Bitcoin address".to_owned())?;
    address
        .require_network(bitcoin_network)
        .map_err(|_| "address encoding is incompatible with bitcoin_network".to_owned())?;
    if config.max_source_reads.get() != BTC_NATIVE_BALANCE_OBSERVE_SOURCE_READS {
        return Err(format!(
            "max_source_reads must equal {BTC_NATIVE_BALANCE_OBSERVE_SOURCE_READS} for Bitcoin native balance observation"
        ));
    }
    if config.coverage != BTC_NATIVE_BALANCE_COVERAGE {
        return Err(format!(
            "coverage must equal {BTC_NATIVE_BALANCE_COVERAGE} for Bitcoin native balance observation"
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
        || balance.block_hash.to_string() != joint_tip.block_hash()
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
    type Plan = BtcAddressBalanceReadPlan;
    type Evidence = BtcAddressBalanceReadEvidence;

    fn plan(
        &self,
        input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Plan> {
        validate_observe_btc_address_balance_config(&self.config)
            .map_err(|reason| StateError::from(BtcStateError::InvalidInput { reason }))?;
        if !input.joint_tip.is_admissible_for_balance_write() {
            return Err(StateError::from(BtcStateError::InvalidInput {
                reason: "joint tip is not admissible for balance write".to_owned(),
            }));
        }
        BtcAddressBalanceReadPlan::new(
            &self.config.network,
            &self.config.bitcoin_network,
            &self.config.semantic_source_identity,
            &self.config.address,
            input.joint_tip.block_height(),
            input.joint_tip.block_hash(),
        )
        .map_err(StateError::from)
    }

    fn reduce(
        &self,
        input: &Self::Input,
        evidence: &ExternalReadEvidenceSet<Self::Evidence>,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        reject_fact_query_evidence(evidence)?;
        let plan = self.plan(input, _context)?;
        let response = evidence.primary_evidence().response(&plan)?;
        self.materialize_response(input, &response)
            .map_err(StateError::from)
    }
}

fn reject_fact_query_evidence<E>(evidence: &ExternalReadEvidenceSet<E>) -> StateResult<()> {
    if evidence.fact_query_evidence().is_empty() {
        Ok(())
    } else {
        Err(StateError::Message(
            "Bitcoin source read received unexpected fact-query evidence".to_owned(),
        ))
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

/// Exact Bitcoin native-balance source key carried by a collection receipt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "native_balance_source_key",
    version = "1",
    schema = "mfm.bitcoin.collection.native_balance_source_key"
)]
pub struct BtcNativeBalanceSourceKey {
    network: String,
    bitcoin_network: String,
    semantic_source_identity: String,
    address: String,
}

impl BtcNativeBalanceSourceKey {
    fn from_fact(fact: &BtcAddressBalanceSnapshotFact) -> Result<Self, BtcStateError> {
        let subject = fact.subject();
        let subject = BtcAddressBalanceSubject::new(
            subject.network(),
            subject.bitcoin_network(),
            subject.semantic_source_identity(),
            subject.address(),
        )?;
        let address =
            BitcoinAddress::new(subject.address()).map_err(|_| BtcStateError::InvalidInput {
                reason: "receipt fact address is not a supported Bitcoin address".to_owned(),
            })?;
        let network = BitcoinNetworkTag::new(subject.bitcoin_network())?;
        address.require_network(network)?;
        Ok(Self {
            network: subject.network().to_owned(),
            bitcoin_network: subject.bitcoin_network().to_owned(),
            semantic_source_identity: subject.semantic_source_identity().to_owned(),
            address: subject.address().to_owned(),
        })
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

    /// Returns the observed Bitcoin address.
    pub fn address(&self) -> &str {
        &self.address
    }
}

/// One checked Bitcoin native-balance fact included in a network collection receipt.
///
/// The embedded fact is private hydration material used only to re-derive and verify the content
/// identity when this receipt is decoded. It is not a report-selection shortcut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "native_balance_receipt_entry",
    version = "1",
    schema = "mfm.bitcoin.collection.native_balance_receipt_entry"
)]
pub struct BtcNativeBalanceReceiptEntry {
    source_key: BtcNativeBalanceSourceKey,
    anchor_height: u64,
    anchor_hash: String,
    coverage: String,
    source_status: String,
    fact_content_identity: mfm_facts::FactContentIdentity,
    verified_fact: BtcAddressBalanceSnapshotFact,
}

impl BtcNativeBalanceReceiptEntry {
    fn from_verified_fact(fact: &BtcAddressBalanceSnapshotFact) -> Result<Self, BtcStateError> {
        let source_key = BtcNativeBalanceSourceKey::from_fact(fact)?;
        let response = fact.response();
        let coverage = response.coverage_status()?;
        let source_status = response.holding_source_status()?;
        if coverage.as_str() != BTC_NATIVE_BALANCE_COVERAGE
            || source_status.as_str() != BTC_NATIVE_BALANCE_SOURCE_STATUS
        {
            return Err(BtcStateError::InvalidInput {
                reason: "receipt fact did not use the fixed Bitcoin native coverage/status"
                    .to_owned(),
            });
        }
        let rebuilt_response = BtcAddressBalanceResponse::new(
            response.anchor_height(),
            response.anchor_hash(),
            response.balance_sats(),
            coverage,
            source_status,
        )?;
        let rebuilt_subject = BtcAddressBalanceSubject::new(
            source_key.network(),
            source_key.bitcoin_network(),
            source_key.semantic_source_identity(),
            source_key.address(),
        )?;
        let rebuilt_fact = BtcAddressBalanceSnapshotFact::new(rebuilt_subject, rebuilt_response);
        if &rebuilt_fact != fact {
            return Err(BtcStateError::InvalidInput {
                reason: "receipt fact did not satisfy the Bitcoin address-balance fact contract"
                    .to_owned(),
            });
        }
        let descriptor = BtcAddressBalanceSnapshotFact::descriptor().map_err(|error| {
            BtcStateError::InvalidInput {
                reason: error.to_string(),
            }
        })?;
        let fact_content_identity = mfm_facts::derive_fact_content_identity_from_typed_values(
            &descriptor,
            fact.subject(),
            fact.response(),
        )
        .map_err(|error| BtcStateError::InvalidInput {
            reason: error.to_string(),
        })?;
        Ok(Self {
            source_key,
            anchor_height: response.anchor_height(),
            anchor_hash: response.anchor_hash().to_owned(),
            coverage: coverage.as_str().to_owned(),
            source_status: source_status.as_str().to_owned(),
            fact_content_identity,
            verified_fact: fact.clone(),
        })
    }

    /// Returns the exact family-specific source key.
    pub const fn source_key(&self) -> &BtcNativeBalanceSourceKey {
        &self.source_key
    }

    /// Returns the exact anchor height.
    pub const fn anchor_height(&self) -> u64 {
        self.anchor_height
    }

    /// Returns the exact anchor hash.
    pub fn anchor_hash(&self) -> &str {
        &self.anchor_hash
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

impl<'de> Deserialize<'de> for BtcNativeBalanceReceiptEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            source_key: BtcNativeBalanceSourceKey,
            anchor_height: u64,
            anchor_hash: String,
            coverage: String,
            source_status: String,
            fact_content_identity: serde_json::Value,
            verified_fact: BtcAddressBalanceSnapshotFact,
        }

        let wire = Wire::deserialize(deserializer)?;
        let expected = Self::from_verified_fact(&wire.verified_fact).map_err(de::Error::custom)?;
        let descriptor = BtcAddressBalanceSnapshotFact::descriptor().map_err(de::Error::custom)?;
        let identity = mfm_facts::verify_serialized_fact_content_identity_from_typed_values(
            &wire.fact_content_identity,
            &descriptor,
            wire.verified_fact.subject(),
            wire.verified_fact.response(),
        )
        .map_err(de::Error::custom)?;
        if wire.source_key != expected.source_key
            || wire.anchor_height != expected.anchor_height
            || wire.anchor_hash != expected.anchor_hash
            || wire.coverage != expected.coverage
            || wire.source_status != expected.source_status
            || identity != expected.fact_content_identity
        {
            return Err(de::Error::custom(
                "Bitcoin native balance receipt entry did not match verified fact material",
            ));
        }
        Ok(expected)
    }
}

/// Config for assembling one exact Bitcoin network collection receipt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(schema = "mfm.bitcoin.state.config.assemble_network_collection_receipt")]
pub struct AssembleBtcNetworkCollectionReceiptConfig {}

/// Input for assembling one exact Bitcoin network collection receipt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.bitcoin.state.input.assemble_network_collection_receipt")]
pub struct AssembleBtcNetworkCollectionReceiptInput {
    /// Joint tip shared by every fact in this network collection.
    pub joint_tip: BtcJointTip,
    /// Completed managed fact outputs for every configured Bitcoin source.
    pub balance_facts: NonEmpty<BtcAddressBalanceSnapshotFact>,
}

/// Exact successful Bitcoin native collection for one semantic network/source binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, MfmValue)]
#[mfm(
    namespace = "mfm.bitcoin",
    name = "network_collection_receipt",
    version = "1",
    schema = "mfm.bitcoin.collection.network_receipt"
)]
pub struct BtcNetworkCollectionReceipt {
    network: String,
    bitcoin_network: String,
    semantic_source_identity: String,
    anchor_height: u64,
    anchor_hash: String,
    successful_observation_count: u64,
    entries: Vec<BtcNativeBalanceReceiptEntry>,
}

impl BtcNetworkCollectionReceipt {
    fn from_entries(
        network: String,
        bitcoin_network: String,
        semantic_source_identity: String,
        anchor_height: u64,
        anchor_hash: String,
        successful_observation_count: u64,
        entries: Vec<BtcNativeBalanceReceiptEntry>,
    ) -> Result<Self, BtcStateError> {
        if entries.is_empty() {
            return Err(BtcStateError::InvalidInput {
                reason: "Bitcoin network collection receipt requires at least one entry".to_owned(),
            });
        }
        let expected_count =
            u64::try_from(entries.len()).map_err(|_| BtcStateError::InvalidInput {
                reason: "Bitcoin network collection receipt entry count overflowed u64".to_owned(),
            })?;
        if successful_observation_count != expected_count {
            return Err(BtcStateError::InvalidInput {
                reason: "Bitcoin network collection receipt successful count did not match entries"
                    .to_owned(),
            });
        }
        BitcoinBlockHash::new(&anchor_hash).map_err(|_| BtcStateError::InvalidInput {
            reason: "Bitcoin network collection receipt anchor hash was malformed".to_owned(),
        })?;
        let mut previous = None;
        for entry in &entries {
            if entry.source_key.network != network
                || entry.source_key.bitcoin_network != bitcoin_network
                || entry.source_key.semantic_source_identity != semantic_source_identity
                || entry.anchor_height != anchor_height
                || entry.anchor_hash != anchor_hash
            {
                return Err(BtcStateError::InvalidInput {
                    reason: "Bitcoin network collection receipt entry did not match network anchor binding"
                        .to_owned(),
                });
            }
            if entry.coverage != BTC_NATIVE_BALANCE_COVERAGE
                || entry.source_status != BTC_NATIVE_BALANCE_SOURCE_STATUS
            {
                return Err(BtcStateError::InvalidInput {
                    reason:
                        "Bitcoin network collection receipt entry did not use fixed coverage/status"
                            .to_owned(),
                });
            }
            if previous
                .as_ref()
                .is_some_and(|key: &&BtcNativeBalanceSourceKey| *key >= &entry.source_key)
            {
                return Err(BtcStateError::InvalidInput {
                    reason: "Bitcoin network collection receipt entries were not strictly sorted"
                        .to_owned(),
                });
            }
            previous = Some(&entry.source_key);
        }
        Ok(Self {
            network,
            bitcoin_network,
            semantic_source_identity,
            anchor_height,
            anchor_hash,
            successful_observation_count,
            entries,
        })
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

    /// Returns the shared anchor height.
    pub const fn anchor_height(&self) -> u64 {
        self.anchor_height
    }

    /// Returns the shared anchor hash.
    pub fn anchor_hash(&self) -> &str {
        &self.anchor_hash
    }

    /// Returns the completed source observation count.
    pub const fn successful_observation_count(&self) -> u64 {
        self.successful_observation_count
    }

    /// Returns entries in strict source-key order.
    pub fn entries(&self) -> &[BtcNativeBalanceReceiptEntry] {
        &self.entries
    }
}

impl<'de> Deserialize<'de> for BtcNetworkCollectionReceipt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            network: String,
            bitcoin_network: String,
            semantic_source_identity: String,
            anchor_height: u64,
            anchor_hash: String,
            successful_observation_count: u64,
            entries: Vec<BtcNativeBalanceReceiptEntry>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::from_entries(
            wire.network,
            wire.bitcoin_network,
            wire.semantic_source_identity,
            wire.anchor_height,
            wire.anchor_hash,
            wire.successful_observation_count,
            wire.entries,
        )
        .map_err(de::Error::custom)
    }
}

/// Pure state that assembles one exact Bitcoin network collection receipt.
pub struct AssembleBtcNetworkCollectionReceiptState;

impl StateSpec for AssembleBtcNetworkCollectionReceiptState {
    type Config = AssembleBtcNetworkCollectionReceiptConfig;
    type Context = NoContext;
    type Input = AssembleBtcNetworkCollectionReceiptInput;
    type Output = BtcNetworkCollectionReceipt;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        state_kind("address_balance.assemble_network_receipt")
    }

    fn version() -> mfm_program::Result<StateVersion> {
        state_version("address_balance.assemble_network_receipt")
    }

    fn name() -> &'static str {
        "mfm.bitcoin.address_balance.assemble_network_receipt"
    }

    fn new(_config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self)
    }
}

impl PureState for AssembleBtcNetworkCollectionReceiptState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        assemble_btc_network_collection_receipt(input)
    }
}

/// Derives one exact Bitcoin network collection receipt from completed managed fact outputs.
pub fn assemble_btc_network_collection_receipt(
    input: AssembleBtcNetworkCollectionReceiptInput,
) -> StateResult<BtcNetworkCollectionReceipt> {
    let mut entries = input
        .balance_facts
        .values()
        .iter()
        .map(BtcNativeBalanceReceiptEntry::from_verified_fact)
        .collect::<Result<Vec<_>, _>>()
        .map_err(StateError::from)?;
    entries.sort_by(|left, right| left.source_key.cmp(&right.source_key));
    let count = u64::try_from(entries.len()).map_err(|_| {
        StateError::from(BtcStateError::InvalidInput {
            reason: "Bitcoin network collection receipt entry count overflowed u64".to_owned(),
        })
    })?;
    BtcNetworkCollectionReceipt::from_entries(
        input.joint_tip.network().to_owned(),
        input.joint_tip.bitcoin_network().to_owned(),
        input.joint_tip.semantic_source_identity().to_owned(),
        input.joint_tip.block_height(),
        input.joint_tip.block_hash().to_owned(),
        count,
        entries,
    )
    .map_err(StateError::from)
}

#[cfg(test)]
#[path = "address_balance_collect_tests.rs"]
mod tests;
