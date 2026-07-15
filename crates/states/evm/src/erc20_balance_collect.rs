//! ERC-20 collector states: shared tip → exact calls → source-near fact.
//!
//! Each metadata or balance observation issues one hash-selected `eth_call` and
//! one hash-selected block re-verification. The state contract owns those two
//! source reads and rejects every selector, source, token, account, or anchor
//! mismatch before a Platform fact can be recorded. The adapter retains the
//! corresponding redacted external-read evidence; output values retain only
//! deterministic source-near material.

use std::future;
use std::num::NonZeroU64;
use std::str::FromStr;

use alloy_primitives::{Address, B256};
use mfm_effects::{ManagedPlatformWrite, ReadExternal};
use mfm_evm_capabilities::{
    EvmBlockReadCapability, EvmBlockReadResponse, EvmBlockSelector, EvmCallReadCapability,
    EvmCallReadRequest, EvmCallReadResponse,
};
use mfm_fact_capabilities::FactRecordCapability;
use mfm_portfolio_model::portfolio::{NetworkConfig, NetworkFamilyConfig};
use mfm_program::{
    fact_descriptor_ref, AdapterBindingSpec, FactDescriptorRef, ManagedWriteState, NoContext,
    ReadState, StateError, StateResult, StateSpec, ValidatedConfig,
};
use mfm_program_derive::{MfmConfig, MfmValue, StateInput};
use serde::{Deserialize, Serialize};

use crate::native_balance_collect::{
    adapter_binding, adapter_required_error, state_kind, state_version,
};
use crate::{
    address_erc20_balance_fact_visibility, canonical_evm_block_hash,
    validate_canonical_erc20_contract_address, EvmAddressErc20BalanceObservation,
    EvmAddressErc20BalanceResponse, EvmAddressErc20BalanceSnapshotFact,
    EvmAddressErc20BalanceSubject, EvmJointTip, EvmStateError,
};

/// Exact number of source reads for one anchored ERC-20 `decimals()` observation.
///
/// The state makes one hash-selected call and one hash-selected block
/// re-verification before accepting metadata.
pub const EVM_ERC20_METADATA_OBSERVE_SOURCE_READS: u64 = 2;
/// Exact number of source reads for one anchored ERC-20 `balanceOf(address)` observation.
///
/// The state makes one hash-selected call and one hash-selected block
/// re-verification before accepting a balance observation.
pub const EVM_ERC20_BALANCE_OBSERVE_SOURCE_READS: u64 = 2;
/// Closed coverage claim for a successful exact ERC-20 account/token observation.
pub const EVM_ERC20_BALANCE_COVERAGE: &str = "complete_at_anchor";
/// Closed source-status claim for a successful exact ERC-20 observation.
pub const EVM_ERC20_BALANCE_SOURCE_STATUS: &str = "ok";

/// Config for observing one ERC-20 token's `decimals()` at a shared EVM joint tip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.evm.state.config.observe_erc20_token_metadata",
    validate = "validate_observe_erc20_token_metadata_config"
)]
pub struct ObserveErc20TokenMetadataConfig {
    /// Semantic EVM network and its certified chain identity.
    pub network: NetworkConfig,
    /// Canonical non-zero ERC-20 contract address.
    pub contract_address: mfm_portfolio_model::ids::NormalizedEvmAddress,
    /// Exact state-owned source-read budget, including anchor re-verification.
    pub max_source_reads: NonZeroU64,
}

/// Validates the configuration for one exact ERC-20 metadata observation.
pub fn validate_observe_erc20_token_metadata_config(
    config: &ObserveErc20TokenMetadataConfig,
) -> Result<(), String> {
    let _ = config.evm_network_parts()?;
    if config.contract_address.is_zero() {
        return Err("erc20 contract_address must not be the zero address".to_owned());
    }
    if config.max_source_reads.get() != EVM_ERC20_METADATA_OBSERVE_SOURCE_READS {
        return Err(format!(
            "max_source_reads must equal {EVM_ERC20_METADATA_OBSERVE_SOURCE_READS} for erc20 metadata observation"
        ));
    }
    Ok(())
}

impl ObserveErc20TokenMetadataConfig {
    /// Returns the semantic EVM network id and required chain id.
    pub fn evm_network_parts(&self) -> Result<(&str, u64), String> {
        if self.network.family() != NetworkFamilyConfig::Evm {
            return Err("erc20 metadata observation requires an EVM network".to_owned());
        }
        let chain_id = self
            .network
            .chain_id_u64()
            .ok_or_else(|| "EVM network is missing chain_id".to_owned())?;
        Ok((self.network.network_id().as_str(), chain_id))
    }
}

/// Input for an ERC-20 token metadata observation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.evm.state.input.observe_erc20_token_metadata")]
pub struct ObserveErc20TokenMetadataInput {
    /// Joint tip resolved once for this EVM network batch.
    pub joint_tip: EvmJointTip,
}

/// Exact observed ERC-20 token metadata at a shared EVM anchor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.evm",
    name = "erc20_token_metadata",
    version = "1",
    schema = "mfm.evm.state.output.erc20_token_metadata"
)]
pub struct EvmErc20TokenMetadata {
    network: String,
    chain_id: u64,
    contract_address: String,
    decimals: u8,
    block_number: u64,
    block_hash: String,
    source_read_count: u64,
}

impl EvmErc20TokenMetadata {
    /// Creates checked metadata from one exact anchored `decimals()` observation.
    pub fn new(
        network: impl Into<String>,
        chain_id: u64,
        contract_address: impl Into<String>,
        decimals: u8,
        block_number: u64,
        block_hash: impl Into<String>,
    ) -> Result<Self, EvmStateError> {
        let network = network.into();
        let contract_address = contract_address.into();
        if network.trim().is_empty() {
            return Err(EvmStateError::InvalidInput {
                reason: "erc20 metadata network must be non-empty".to_owned(),
            });
        }
        if chain_id == 0 {
            return Err(EvmStateError::InvalidInput {
                reason: "erc20 metadata chain_id must be non-zero".to_owned(),
            });
        }
        validate_canonical_erc20_contract_address(&contract_address)?;
        let block_hash = canonical_evm_block_hash(block_hash)?;
        Ok(Self {
            network,
            chain_id,
            contract_address,
            decimals,
            block_number,
            block_hash,
            source_read_count: EVM_ERC20_METADATA_OBSERVE_SOURCE_READS,
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

    /// Returns the canonical ERC-20 contract address.
    pub fn contract_address(&self) -> &str {
        &self.contract_address
    }

    /// Returns the observed ERC-20 decimals value.
    pub const fn decimals(&self) -> u8 {
        self.decimals
    }

    /// Returns the shared anchor block number.
    pub const fn block_number(&self) -> u64 {
        self.block_number
    }

    /// Returns the canonical shared anchor block hash.
    pub fn block_hash(&self) -> &str {
        &self.block_hash
    }

    /// Returns the exact state-owned source-read count.
    pub const fn source_read_count(&self) -> u64 {
        self.source_read_count
    }

    fn validate_for_balance(
        &self,
        config: &ObserveErc20BalanceConfig,
        joint_tip: &EvmJointTip,
    ) -> Result<(), EvmStateError> {
        let rebuilt = Self::new(
            self.network.clone(),
            self.chain_id,
            self.contract_address.clone(),
            self.decimals,
            self.block_number,
            self.block_hash.clone(),
        )?;
        if &rebuilt != self {
            return Err(EvmStateError::InvalidInput {
                reason:
                    "erc20 metadata was not produced by the exact metadata observation contract"
                        .to_owned(),
            });
        }
        let (network, chain_id) = config
            .evm_network_parts()
            .map_err(|reason| EvmStateError::InvalidInput { reason })?;
        if self.network != network
            || self.chain_id != chain_id
            || self.contract_address != config.contract_address.as_str()
            || self.block_number != joint_tip.block_number()
            || self.block_hash != joint_tip.block_hash()
        {
            return Err(EvmStateError::InvalidInput {
                reason: "erc20 metadata does not match balance config and shared joint tip"
                    .to_owned(),
            });
        }
        Ok(())
    }
}

/// Builds an exact hash-selected `decimals()` request from certified state intent.
pub fn erc20_metadata_call_request(
    config: &ObserveErc20TokenMetadataConfig,
    joint_tip: &EvmJointTip,
) -> Result<EvmCallReadRequest, EvmStateError> {
    validate_observe_erc20_token_metadata_config(config)
        .map_err(|reason| EvmStateError::InvalidInput { reason })?;
    let (network, chain_id) = config
        .evm_network_parts()
        .map_err(|reason| EvmStateError::InvalidInput { reason })?;
    require_joint_tip_binding(joint_tip, network, chain_id, "erc20 metadata")?;
    let contract_address = parse_evm_address(config.contract_address.as_str())?;
    let calldata =
        mfm_evm_core::hex::hex_to_bytes(&mfm_evm_core::encoding::encode_erc20_decimals()).map_err(
            |_| EvmStateError::InvalidInput {
                reason: "erc20 decimals calldata could not be encoded".to_owned(),
            },
        )?;
    let block_hash = parse_joint_tip_hash(joint_tip)?;
    Ok(EvmCallReadRequest::new(
        contract_address,
        calldata,
        EvmBlockSelector::Hash(block_hash),
    ))
}

/// Normalizes an exact ERC-20 metadata capability response after anchor re-verification.
pub fn normalize_erc20_token_metadata_from_capability(
    config: &ObserveErc20TokenMetadataConfig,
    joint_tip: &EvmJointTip,
    request: &EvmCallReadRequest,
    response: &EvmCallReadResponse,
    verification: &EvmBlockReadResponse,
) -> Result<EvmErc20TokenMetadata, EvmStateError> {
    let expected_request = erc20_metadata_call_request(config, joint_tip)?;
    if request != &expected_request {
        return Err(EvmStateError::InvalidInput {
            reason: "erc20 metadata call request did not match certified state intent".to_owned(),
        });
    }
    let (network, chain_id) = config
        .evm_network_parts()
        .map_err(|reason| EvmStateError::InvalidInput { reason })?;
    require_call_source_binding(&response.evidence, network, chain_id, "erc20 metadata")?;
    verify_joint_tip_response(network, chain_id, joint_tip, verification, "erc20 metadata")?;
    let decimals = mfm_evm_core::encoding::decode_erc20_decimals_result(&response.return_data)
        .map_err(|_| EvmStateError::InvalidInput {
            reason: "erc20 decimals result was malformed".to_owned(),
        })?;
    EvmErc20TokenMetadata::new(
        network,
        chain_id,
        config.contract_address.as_str(),
        decimals,
        joint_tip.block_number(),
        joint_tip.block_hash(),
    )
}

/// State contract for observing one ERC-20 token's decimals at a shared hash anchor.
pub struct ObserveErc20TokenMetadataState {
    config: ObserveErc20TokenMetadataConfig,
}

impl ObserveErc20TokenMetadataState {
    /// Builds the only admissible generic EVM call request for this state input.
    pub fn call_request(
        &self,
        input: &ObserveErc20TokenMetadataInput,
    ) -> Result<EvmCallReadRequest, EvmStateError> {
        erc20_metadata_call_request(&self.config, &input.joint_tip)
    }

    /// Materializes metadata only after exact call and block re-verification responses pass.
    pub fn materialize_response(
        &self,
        input: &ObserveErc20TokenMetadataInput,
        request: &EvmCallReadRequest,
        response: &EvmCallReadResponse,
        verification: &EvmBlockReadResponse,
    ) -> Result<EvmErc20TokenMetadata, EvmStateError> {
        normalize_erc20_token_metadata_from_capability(
            &self.config,
            &input.joint_tip,
            request,
            response,
            verification,
        )
    }

    /// Returns the certified configuration.
    pub fn config(&self) -> &ObserveErc20TokenMetadataConfig {
        &self.config
    }
}

impl StateSpec for ObserveErc20TokenMetadataState {
    type Config = ObserveErc20TokenMetadataConfig;
    type Context = NoContext;
    type Input = ObserveErc20TokenMetadataInput;
    type Output = EvmErc20TokenMetadata;
    type Effect = ReadExternal;
    type Caps = (EvmCallReadCapability, EvmBlockReadCapability);

    fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
        state_kind("erc20_token_metadata.observe")
    }

    fn version() -> mfm_program::Result<mfm_ids::StateVersion> {
        state_version("erc20_token_metadata.observe")
    }

    fn name() -> &'static str {
        "mfm.evm.erc20_token_metadata.observe"
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

impl ReadState for ObserveErc20TokenMetadataState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        if let Err(error) = self.call_request(&input) {
            return future::ready(Err(StateError::from(error)));
        }
        future::ready(Err(adapter_required_error(Self::name())))
    }
}

/// Config for observing one ERC-20 account balance at a shared EVM joint tip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[mfm(
    schema = "mfm.evm.state.config.observe_erc20_balance",
    validate = "validate_observe_erc20_balance_config"
)]
pub struct ObserveErc20BalanceConfig {
    /// Semantic EVM network and its certified chain identity.
    pub network: NetworkConfig,
    /// Canonical non-zero ERC-20 contract address.
    pub contract_address: mfm_portfolio_model::ids::NormalizedEvmAddress,
    /// Canonical EVM account whose token balance is observed.
    pub account: mfm_portfolio_model::ids::NormalizedEvmAddress,
    /// Exact state-owned source-read budget, including anchor re-verification.
    pub max_source_reads: NonZeroU64,
}

/// Validates the configuration for one exact ERC-20 account balance observation.
pub fn validate_observe_erc20_balance_config(
    config: &ObserveErc20BalanceConfig,
) -> Result<(), String> {
    let _ = config.evm_network_parts()?;
    if config.contract_address.is_zero() {
        return Err("erc20 contract_address must not be the zero address".to_owned());
    }
    if config.max_source_reads.get() != EVM_ERC20_BALANCE_OBSERVE_SOURCE_READS {
        return Err(format!(
            "max_source_reads must equal {EVM_ERC20_BALANCE_OBSERVE_SOURCE_READS} for erc20 balance observation"
        ));
    }
    Ok(())
}

impl ObserveErc20BalanceConfig {
    /// Returns the semantic EVM network id and required chain id.
    pub fn evm_network_parts(&self) -> Result<(&str, u64), String> {
        if self.network.family() != NetworkFamilyConfig::Evm {
            return Err("erc20 balance observation requires an EVM network".to_owned());
        }
        let chain_id = self
            .network
            .chain_id_u64()
            .ok_or_else(|| "EVM network is missing chain_id".to_owned())?;
        Ok((self.network.network_id().as_str(), chain_id))
    }
}

/// Input for a hash-pinned ERC-20 account balance observation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.evm.state.input.observe_erc20_balance")]
pub struct ObserveErc20BalanceInput {
    /// Joint tip resolved once for this EVM network batch.
    pub joint_tip: EvmJointTip,
    /// Token metadata observed at this same joint tip.
    pub metadata: EvmErc20TokenMetadata,
}

/// Builds an exact hash-selected `balanceOf(address)` request from certified state intent.
pub fn erc20_balance_call_request(
    config: &ObserveErc20BalanceConfig,
    input: &ObserveErc20BalanceInput,
) -> Result<EvmCallReadRequest, EvmStateError> {
    validate_observe_erc20_balance_config(config)
        .map_err(|reason| EvmStateError::InvalidInput { reason })?;
    let (network, chain_id) = config
        .evm_network_parts()
        .map_err(|reason| EvmStateError::InvalidInput { reason })?;
    require_joint_tip_binding(&input.joint_tip, network, chain_id, "erc20 balance")?;
    input
        .metadata
        .validate_for_balance(config, &input.joint_tip)?;
    let contract_address = parse_evm_address(config.contract_address.as_str())?;
    let account = parse_evm_address(config.account.as_str())?;
    let calldata =
        mfm_evm_core::hex::hex_to_bytes(&mfm_evm_core::encoding::encode_erc20_balance_of(&account))
            .map_err(|_| EvmStateError::InvalidInput {
                reason: "erc20 balance calldata could not be encoded".to_owned(),
            })?;
    let block_hash = parse_joint_tip_hash(&input.joint_tip)?;
    Ok(EvmCallReadRequest::new(
        contract_address,
        calldata,
        EvmBlockSelector::Hash(block_hash),
    ))
}

/// Normalizes an exact ERC-20 balance capability response after anchor re-verification.
pub fn normalize_erc20_balance_from_capability(
    config: &ObserveErc20BalanceConfig,
    input: &ObserveErc20BalanceInput,
    request: &EvmCallReadRequest,
    response: &EvmCallReadResponse,
    verification: &EvmBlockReadResponse,
) -> Result<EvmAddressErc20BalanceObservation, EvmStateError> {
    let expected_request = erc20_balance_call_request(config, input)?;
    if request != &expected_request {
        return Err(EvmStateError::InvalidInput {
            reason: "erc20 balance call request did not match certified state intent".to_owned(),
        });
    }
    let (network, chain_id) = config
        .evm_network_parts()
        .map_err(|reason| EvmStateError::InvalidInput { reason })?;
    require_call_source_binding(&response.evidence, network, chain_id, "erc20 balance")?;
    verify_joint_tip_response(
        network,
        chain_id,
        &input.joint_tip,
        verification,
        "erc20 balance",
    )?;
    let raw_units = mfm_evm_core::encoding::decode_erc20_balance_result(&response.return_data)
        .map_err(|_| EvmStateError::InvalidInput {
            reason: "erc20 balance result was malformed".to_owned(),
        })?
        .to_string();
    let subject = EvmAddressErc20BalanceSubject::new(
        network,
        chain_id,
        config.contract_address.as_str(),
        config.account.as_str(),
    )?;
    let response = EvmAddressErc20BalanceResponse::new(
        input.joint_tip.block_number(),
        input.joint_tip.block_hash(),
        raw_units,
        input.metadata.decimals(),
    )?;
    Ok(EvmAddressErc20BalanceObservation::new(subject, response))
}

/// State contract for observing one ERC-20 account balance at a shared hash anchor.
pub struct ObserveErc20BalanceState {
    config: ObserveErc20BalanceConfig,
}

impl ObserveErc20BalanceState {
    /// Builds the only admissible generic EVM call request for this state input.
    pub fn call_request(
        &self,
        input: &ObserveErc20BalanceInput,
    ) -> Result<EvmCallReadRequest, EvmStateError> {
        erc20_balance_call_request(&self.config, input)
    }

    /// Materializes a balance observation only after exact call and anchor checks pass.
    pub fn materialize_response(
        &self,
        input: &ObserveErc20BalanceInput,
        request: &EvmCallReadRequest,
        response: &EvmCallReadResponse,
        verification: &EvmBlockReadResponse,
    ) -> Result<EvmAddressErc20BalanceObservation, EvmStateError> {
        normalize_erc20_balance_from_capability(
            &self.config,
            input,
            request,
            response,
            verification,
        )
    }

    /// Returns the certified configuration.
    pub fn config(&self) -> &ObserveErc20BalanceConfig {
        &self.config
    }
}

impl StateSpec for ObserveErc20BalanceState {
    type Config = ObserveErc20BalanceConfig;
    type Context = NoContext;
    type Input = ObserveErc20BalanceInput;
    type Output = EvmAddressErc20BalanceObservation;
    type Effect = ReadExternal;
    type Caps = (EvmCallReadCapability, EvmBlockReadCapability);

    fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
        state_kind("erc20_balance.observe")
    }

    fn version() -> mfm_program::Result<mfm_ids::StateVersion> {
        state_version("erc20_balance.observe")
    }

    fn name() -> &'static str {
        "mfm.evm.erc20_balance.observe"
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

impl ReadState for ObserveErc20BalanceState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        if let Err(error) = self.call_request(&input) {
            return future::ready(Err(StateError::from(error)));
        }
        future::ready(Err(adapter_required_error(Self::name())))
    }
}

/// Config for recording one ERC-20 account balance Platform fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[mfm(schema = "mfm.evm.state.config.record_erc20_balance_fact")]
pub struct RecordErc20BalanceFactConfig {}

/// Input for recording one ERC-20 account balance Platform fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.evm.state.input.record_erc20_balance_fact")]
pub struct RecordErc20BalanceFactInput {
    /// Whole normalized ERC-20 balance observation to transform into a fact.
    pub observation: EvmAddressErc20BalanceObservation,
}

/// State contract for recording one ERC-20 account balance Platform fact.
pub struct RecordErc20BalanceFactState;

impl StateSpec for RecordErc20BalanceFactState {
    type Config = RecordErc20BalanceFactConfig;
    type Context = NoContext;
    type Input = RecordErc20BalanceFactInput;
    type Output = EvmAddressErc20BalanceSnapshotFact;
    type Effect = ManagedPlatformWrite;
    type Caps = (FactRecordCapability,);

    fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
        state_kind("erc20_balance.record")
    }

    fn version() -> mfm_program::Result<mfm_ids::StateVersion> {
        state_version("erc20_balance.record")
    }

    fn name() -> &'static str {
        "mfm.evm.erc20_balance.record"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<AdapterBindingSpec>> {
        adapter_binding()
    }

    fn emitted_fact_descriptors() -> mfm_program::Result<Vec<FactDescriptorRef>> {
        Ok(vec![fact_descriptor_ref::<
            EvmAddressErc20BalanceSnapshotFact,
        >()?])
    }

    fn new(_config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self)
    }
}

impl ManagedWriteState for RecordErc20BalanceFactState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        future::ready(input.observation.try_into_fact().map_err(StateError::from))
    }
}

/// Returns Platform visibility for ERC-20 balance facts in adapter record runners.
pub fn erc20_balance_record_visibility() -> mfm_facts::FactVisibility {
    address_erc20_balance_fact_visibility()
}

fn require_joint_tip_binding(
    joint_tip: &EvmJointTip,
    network: &str,
    chain_id: u64,
    observation: &'static str,
) -> Result<(), EvmStateError> {
    if !joint_tip.is_admissible_for_balance_write()
        || joint_tip.network() != network
        || joint_tip.chain_id() != chain_id
    {
        return Err(EvmStateError::InvalidInput {
            reason: format!("{observation} joint tip does not match certified network binding"),
        });
    }
    Ok(())
}

fn parse_evm_address(value: &str) -> Result<Address, EvmStateError> {
    Address::from_str(value).map_err(|_| EvmStateError::InvalidInput {
        reason: "erc20 address must be a normalized 20-byte EVM address".to_owned(),
    })
}

fn parse_joint_tip_hash(joint_tip: &EvmJointTip) -> Result<B256, EvmStateError> {
    let hash = joint_tip
        .block_hash()
        .strip_prefix("0x")
        .or_else(|| joint_tip.block_hash().strip_prefix("0X"))
        .unwrap_or(joint_tip.block_hash());
    B256::from_str(hash).map_err(|_| EvmStateError::InvalidInput {
        reason: "erc20 joint tip block hash was malformed".to_owned(),
    })
}

fn require_call_source_binding(
    evidence: &mfm_evm_capabilities::RedactedEvmSourceEvidence,
    network: &str,
    chain_id: u64,
    observation: &'static str,
) -> Result<(), EvmStateError> {
    if evidence.network_id.as_str() != network
        || evidence.expected_chain_id != chain_id
        || evidence.observed_chain_id != chain_id
        || evidence.source_ref.as_str().is_empty()
        || evidence.policy_id.as_str().is_empty()
    {
        return Err(EvmStateError::InvalidInput {
            reason: format!("{observation} call response did not match certified source binding"),
        });
    }
    Ok(())
}

fn verify_joint_tip_response(
    network: &str,
    chain_id: u64,
    joint_tip: &EvmJointTip,
    verification: &EvmBlockReadResponse,
    observation: &'static str,
) -> Result<(), EvmStateError> {
    let verified_tip = EvmJointTip::from_block_response(network, chain_id, verification)?;
    if verified_tip.network() != joint_tip.network()
        || verified_tip.chain_id() != joint_tip.chain_id()
        || verified_tip.block_number() != joint_tip.block_number()
        || verified_tip.block_hash() != joint_tip.block_hash()
    {
        return Err(EvmStateError::InvalidInput {
            reason: format!("{observation} anchor re-verification drifted from shared joint tip"),
        });
    }
    Ok(())
}

#[cfg(test)]
#[path = "erc20_balance_collect_tests.rs"]
mod tests;
