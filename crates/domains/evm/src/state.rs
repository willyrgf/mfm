//! Decomposed audited EVM read states and pure balance aggregation.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use alloy_primitives::{Address, U256};
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_ids::{ContentRef, DigestAlgorithm, SchemaId, SemanticTypeId, StableId};
use mfm_program::{
    boundary_content_ref, encode_boundary, CanonicalCodec, CapabilityOperation, EvidenceVerdict,
    FactProposal, FactSet, NoBoundaryValue, NoContext, ObservationOutcome, ObservationView,
    ProposedFactValue, Settlement, State, StateExecution, StateFrame, UnitConfig,
};
use mfm_program_derive::{MfmConfig, MfmValue, StateInput};
use mfm_values::{ConfigError, MfmValue as _, RetainedValueContract};
use serde::{de, Deserialize, Serialize};

use crate::capability::{
    EvmAnchorConfirmationRequest, EvmAnchoredSource, EvmBlockResponse, EvmChainIdentityRequest,
    EvmChainIdentityResponse, EvmCheckedSource, EvmLatestAnchorRequest, EvmNativeBalanceRequest,
    EvmNetworkBinding, EvmQuantityResponse, EvmReadFailure, EvmResponseInvalidKind,
    EvmSafeDiagnostic, EvmTokenBalanceRequest, EvmTokenDecimalsRequest, EvmTokenDecimalsResponse,
    EVM_CHAIN_ID_OPERATION_ID, EVM_CONFIRM_ANCHOR_OPERATION_ID, EVM_LATEST_ANCHOR_OPERATION_ID,
    EVM_NATIVE_BALANCE_OPERATION_ID, EVM_TOKEN_BALANCE_OPERATION_ID,
    EVM_TOKEN_DECIMALS_OPERATION_ID,
};
use crate::model::EvmBlockAnchor;

/// Maximum source demand admitted by one EVM balance collection.
pub const EVM_BALANCE_COLLECTION_SOURCE_LIMIT: usize = 1_024;
/// Maximum distinct ERC-20 metadata demand admitted by one collection.
pub const EVM_BALANCE_COLLECTION_TOKEN_LIMIT: usize = 1_024;
/// Maximum number of independently authored states in one maximal collection.
pub const EVM_BALANCE_COLLECTION_NODE_LIMIT: usize = 2_052;

/// Redaction-safe construction or aggregation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvmBalanceCollectionError {
    /// One value violated the closed EVM balance contract.
    #[error("invalid EVM balance collection value: {0}")]
    Invalid(&'static str),
    /// A typed-kernel contract could not be constructed.
    #[error("invalid EVM program contract: {0}")]
    Program(String),
}

impl From<mfm_program::ProgramError> for EvmBalanceCollectionError {
    fn from(error: mfm_program::ProgramError) -> Self {
        Self::Program(error.to_string())
    }
}

impl From<mfm_facts::FactError> for EvmBalanceCollectionError {
    fn from(error: mfm_facts::FactError) -> Self {
        Self::Program(error.to_string())
    }
}

/// Asset identity for one EVM balance request.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance-asset",
    version = "1",
    schema = "mfm.evm.balance_asset"
)]
pub enum EvmBalanceAsset {
    /// Native network asset.
    Native,
    /// ERC-20 contract asset.
    Erc20 {
        /// Canonical non-zero contract address.
        contract_address: String,
    },
}

impl EvmBalanceAsset {
    /// Creates a checked ERC-20 asset.
    pub fn erc20(contract_address: Address) -> Result<Self, EvmBalanceCollectionError> {
        if contract_address.is_zero() {
            return Err(EvmBalanceCollectionError::Invalid(
                "zero ERC-20 contract address",
            ));
        }
        Ok(Self::Erc20 {
            contract_address: canonical_address(contract_address),
        })
    }

    /// Returns the token contract for an ERC-20 asset.
    pub fn contract_address(&self) -> Option<&str> {
        match self {
            Self::Native => None,
            Self::Erc20 { contract_address } => Some(contract_address),
        }
    }

    fn validate(&self) -> Result<(), EvmBalanceCollectionError> {
        if let Some(raw) = self.contract_address() {
            let address = parse_address(raw)?;
            if address.is_zero() {
                return Err(EvmBalanceCollectionError::Invalid(
                    "zero ERC-20 contract address",
                ));
            }
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for EvmBalanceAsset {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            Native,
            Erc20 { contract_address: String },
        }

        match Wire::deserialize(deserializer)? {
            Wire::Native => Ok(Self::Native),
            Wire::Erc20 { contract_address } => {
                let address = parse_address(&contract_address).map_err(de::Error::custom)?;
                Self::erc20(address).map_err(de::Error::custom)
            }
        }
    }
}

/// One unique account/asset balance source.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance-source",
    version = "1",
    schema = "mfm.evm.balance_source"
)]
pub struct EvmBalanceSource {
    account: String,
    asset: EvmBalanceAsset,
}

impl EvmBalanceSource {
    /// Creates one checked source.
    pub fn new(
        account: Address,
        asset: EvmBalanceAsset,
    ) -> Result<Self, EvmBalanceCollectionError> {
        asset.validate()?;
        Ok(Self {
            account: canonical_address(account),
            asset,
        })
    }

    /// Returns the canonical account address.
    pub fn account(&self) -> &str {
        &self.account
    }

    /// Returns the requested asset.
    pub const fn asset(&self) -> &EvmBalanceAsset {
        &self.asset
    }

    /// Parses the checked account address.
    pub fn account_address(&self) -> Result<Address, EvmBalanceCollectionError> {
        parse_address(&self.account)
    }

    /// Parses the optional token contract.
    pub fn contract_address_value(&self) -> Result<Option<Address>, EvmBalanceCollectionError> {
        self.asset.contract_address().map(parse_address).transpose()
    }

    fn validate(&self) -> Result<(), EvmBalanceCollectionError> {
        parse_address(&self.account)?;
        self.asset.validate()
    }
}

/// Certified EVM network demand and immutable routing generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    schema = "mfm.evm.config.balance_collection",
    version = "1",
    validate = "validate_evm_balance_collection_config"
)]
pub struct EvmBalanceCollectionConfig {
    binding: EvmNetworkBinding,
    native_decimals: u8,
    sources: Vec<EvmBalanceSource>,
}

impl EvmBalanceCollectionConfig {
    /// Creates checked, canonical source demand.
    pub fn new(
        binding: EvmNetworkBinding,
        native_decimals: u8,
        mut sources: Vec<EvmBalanceSource>,
    ) -> Result<Self, ConfigError> {
        sources.sort();
        let config = Self {
            binding,
            native_decimals,
            sources,
        };
        validate_evm_balance_collection_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the semantic network and immutable generation.
    pub const fn binding(&self) -> &EvmNetworkBinding {
        &self.binding
    }

    /// Returns the configured native scale.
    pub const fn native_decimals(&self) -> u8 {
        self.native_decimals
    }

    /// Returns sorted, unique source demand.
    pub fn sources(&self) -> &[EvmBalanceSource] {
        &self.sources
    }

    /// Returns distinct token contracts in canonical order.
    pub fn token_contracts(&self) -> Vec<String> {
        self.sources
            .iter()
            .filter_map(|source| source.asset.contract_address().map(str::to_owned))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// Returns the exact number of graph nodes authored for this demand.
    pub fn node_count(&self) -> usize {
        self.sources.len() + self.token_contracts().len() + 4
    }
}

/// Validates collection bounds and canonical demand order.
pub fn validate_evm_balance_collection_config(
    config: &EvmBalanceCollectionConfig,
) -> Result<(), String> {
    if config.sources.is_empty() || config.sources.len() > EVM_BALANCE_COLLECTION_SOURCE_LIMIT {
        return Err("EVM source demand must contain between 1 and 1024 entries".to_owned());
    }
    let mut previous = None;
    for source in &config.sources {
        source.validate().map_err(|error| error.to_string())?;
        if previous.is_some_and(|prior: &EvmBalanceSource| prior >= source) {
            return Err("EVM source demand must be strictly sorted and unique".to_owned());
        }
        previous = Some(source);
    }
    if config.token_contracts().len() > EVM_BALANCE_COLLECTION_TOKEN_LIMIT {
        return Err("EVM token demand exceeded 1024 distinct contracts".to_owned());
    }
    if config.node_count() > EVM_BALANCE_COLLECTION_NODE_LIMIT {
        return Err("EVM collection graph exceeded 2052 nodes".to_owned());
    }
    Ok(())
}

/// Validated semantic network input to source bootstrap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, StateInput)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.evm.input.bootstrap_source", version = "1")]
pub struct EvmBootstrapInput {
    binding: EvmNetworkBinding,
}

impl EvmBootstrapInput {
    /// Creates bootstrap input from a validated collection position.
    pub fn new(binding: EvmNetworkBinding) -> Self {
        Self { binding }
    }

    /// Returns the validated semantic network binding.
    pub const fn binding(&self) -> &EvmNetworkBinding {
        &self.binding
    }
}

/// Initial-anchor input produced by exact source bootstrap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, StateInput)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.evm.input.initial_anchor", version = "1")]
pub struct EvmInitialAnchorInput {
    checked_source: EvmCheckedSource,
}

impl EvmInitialAnchorInput {
    /// Creates input from the exact bootstrapped source.
    pub fn new(checked_source: EvmCheckedSource) -> Self {
        Self { checked_source }
    }

    /// Returns the exact bootstrapped source.
    pub const fn checked_source(&self) -> &EvmCheckedSource {
        &self.checked_source
    }
}

/// One validated token-metadata fan-out input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, StateInput)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.evm.input.token_decimals", version = "1")]
pub struct EvmTokenDecimalsInput {
    anchored_source: EvmAnchoredSource,
    contract_address: String,
}

impl EvmTokenDecimalsInput {
    /// Creates a token metadata input from validated same-run values.
    pub fn new(anchored_source: EvmAnchoredSource, contract_address: String) -> Self {
        Self {
            anchored_source,
            contract_address,
        }
    }

    /// Returns the common checked source and anchor.
    pub const fn anchored_source(&self) -> &EvmAnchoredSource {
        &self.anchored_source
    }

    /// Returns the canonical token contract.
    pub fn contract_address(&self) -> &str {
        &self.contract_address
    }
}

/// One validated account/asset balance fan-out input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, StateInput)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.evm.input.balance_read", version = "1")]
pub struct EvmBalanceReadInput {
    anchored_source: EvmAnchoredSource,
    balance_source: EvmBalanceSource,
}

impl EvmBalanceReadInput {
    /// Creates one balance input from validated same-run values.
    pub fn new(anchored_source: EvmAnchoredSource, balance_source: EvmBalanceSource) -> Self {
        Self {
            anchored_source,
            balance_source,
        }
    }

    /// Returns the common checked source and anchor.
    pub const fn anchored_source(&self) -> &EvmAnchoredSource {
        &self.anchored_source
    }

    /// Returns the exact account/asset demand.
    pub const fn balance_source(&self) -> &EvmBalanceSource {
        &self.balance_source
    }
}

/// Common output type for all independently audited fan-out and confirmation reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance-graph-result",
    version = "1",
    schema = "mfm.evm.balance_graph_result"
)]
pub enum EvmBalanceGraphResult {
    /// One distinct token's decimals.
    TokenDecimals {
        /// Exact source/anchor used by the call.
        source: EvmAnchoredSource,
        /// Canonical token contract.
        contract_address: String,
        /// ABI-decoded decimal scale.
        decimals: u8,
    },
    /// One account/asset balance.
    Balance {
        /// Exact source/anchor used by the call.
        anchored_source: EvmAnchoredSource,
        /// Certified balance source.
        balance_source: EvmBalanceSource,
        /// Canonical U256 decimal quantity.
        raw_units: String,
    },
    /// Final number-to-hash confirmation.
    AnchorConfirmation {
        /// Initial source/anchor, confirmed by block number.
        source: EvmAnchoredSource,
    },
}

impl EvmBalanceGraphResult {
    fn anchored_source(&self) -> &EvmAnchoredSource {
        match self {
            Self::TokenDecimals { source, .. } | Self::AnchorConfirmation { source } => source,
            Self::Balance {
                anchored_source, ..
            } => anchored_source,
        }
    }
}

/// Complete fan-out values consumed by final anchor confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, StateInput)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.evm.input.anchor_confirmation", version = "1")]
pub struct EvmAnchorConfirmationInput {
    fanout: Vec<EvmBalanceGraphResult>,
}

impl EvmAnchorConfirmationInput {
    /// Creates confirmation input from every successful fan-out read.
    pub fn new(fanout: Vec<EvmBalanceGraphResult>) -> Self {
        Self { fanout }
    }

    /// Returns fan-out values in certified producer order.
    pub fn fanout(&self) -> &[EvmBalanceGraphResult] {
        &self.fanout
    }
}

/// Validated collection demand and complete confirmed read results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, StateInput)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.evm.input.balance_aggregation", version = "1")]
pub struct EvmBalanceAggregationInput {
    binding: EvmNetworkBinding,
    native_decimals: u8,
    sources: Vec<EvmBalanceSource>,
    token_contracts: Vec<String>,
    results: Vec<EvmBalanceGraphResult>,
}

impl EvmBalanceAggregationInput {
    /// Creates exact aggregation input from validated demand and read outputs.
    pub fn new(
        binding: EvmNetworkBinding,
        native_decimals: u8,
        sources: Vec<EvmBalanceSource>,
        token_contracts: Vec<String>,
        results: Vec<EvmBalanceGraphResult>,
    ) -> Self {
        Self {
            binding,
            native_decimals,
            sources,
            token_contracts,
            results,
        }
    }

    /// Returns the validated network binding.
    pub const fn binding(&self) -> &EvmNetworkBinding {
        &self.binding
    }

    /// Returns the validated native scale.
    pub const fn native_decimals(&self) -> u8 {
        self.native_decimals
    }

    /// Returns strict sorted unique balance demand.
    pub fn sources(&self) -> &[EvmBalanceSource] {
        &self.sources
    }

    /// Returns strict sorted unique token metadata demand.
    pub fn token_contracts(&self) -> &[String] {
        &self.token_contracts
    }

    /// Returns all fan-out values followed by anchor confirmation.
    pub fn results(&self) -> &[EvmBalanceGraphResult] {
        &self.results
    }
}

/// One final typed balance produced by pure aggregation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "collected-balance",
    version = "1",
    schema = "mfm.evm.collected_balance"
)]
pub struct EvmCollectedBalance {
    source: EvmBalanceSource,
    raw_units: String,
    decimals: u8,
}

impl EvmCollectedBalance {
    /// Returns the account/asset demand.
    pub const fn source(&self) -> &EvmBalanceSource {
        &self.source
    }

    /// Returns the canonical U256 decimal quantity.
    pub fn raw_units(&self) -> &str {
        &self.raw_units
    }

    /// Returns the native or token scale.
    pub const fn decimals(&self) -> u8 {
        self.decimals
    }
}

/// Complete same-run EVM collection consumed directly by portfolio states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance-collection",
    version = "1",
    schema = "mfm.evm.balance_collection"
)]
pub struct EvmBalanceCollection {
    source: EvmAnchoredSource,
    balances: Vec<EvmCollectedBalance>,
}

impl EvmBalanceCollection {
    /// Returns the exact checked source and common anchor.
    pub const fn source(&self) -> &EvmAnchoredSource {
        &self.source
    }

    /// Returns balances in certified config order.
    pub fn balances(&self) -> &[EvmCollectedBalance] {
        &self.balances
    }
}

/// Same-run fact value emitted for each aggregated balance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance-snapshot-fact",
    version = "1",
    schema = "mfm.evm.fact.balance_snapshot"
)]
pub struct EvmBalanceSnapshotFact {
    network_id: String,
    chain_id: u64,
    account: String,
    asset: EvmBalanceAsset,
    block_anchor: EvmBlockAnchor,
    raw_units: String,
    decimals: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance-snapshot-subject",
    version = "1",
    schema = "mfm.evm.fact.balance_snapshot_subject"
)]
pub(crate) struct EvmBalanceSnapshotSubject {
    network_id: String,
    chain_id: u64,
    account: String,
    asset: EvmBalanceAsset,
}

impl EvmBalanceSnapshotFact {
    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the checked chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the canonical account address.
    pub fn account(&self) -> &str {
        &self.account
    }

    /// Returns the asset identity.
    pub const fn asset(&self) -> &EvmBalanceAsset {
        &self.asset
    }

    /// Returns the common block anchor.
    pub const fn block_anchor(&self) -> &EvmBlockAnchor {
        &self.block_anchor
    }

    /// Returns the raw U256 decimal quantity.
    pub fn raw_units(&self) -> &str {
        &self.raw_units
    }

    /// Returns the scale.
    pub const fn decimals(&self) -> u8 {
        self.decimals
    }
}

/// Audited exact-generation source/chain bootstrap.
pub struct BootstrapEvmSourceState;
/// Audited initial latest number/hash read.
pub struct ReadEvmInitialAnchorState;
/// Audited one-contract ERC-20 decimals read.
pub struct ReadEvmTokenDecimalsState;
/// Audited one-account native balance read.
pub struct ReadEvmNativeBalanceState;
/// Audited one-account/contract ERC-20 balance read.
pub struct ReadEvmTokenBalanceState;
/// Audited final number-to-hash confirmation.
pub struct ConfirmEvmAnchorState;
/// Pure exact-coverage EVM aggregation.
pub struct AggregateEvmBalancesState;

impl State for BootstrapEvmSourceState {
    type Config = UnitConfig;
    type Context = NoContext;
    type Input = EvmBootstrapInput;
    type Output = EvmCheckedSource;
    type Failure = EvmReadFailure;
    type Request = EvmChainIdentityRequest;
    type Observation = EvmChainIdentityResponse;
    type SafeDiagnostic = EvmSafeDiagnostic;

    fn state_contract_ref() -> mfm_program::Result<ContentRef> {
        state_contract_ref("bootstrap_source")
    }
}

fn bootstrap_request(frame: StateFrame<'_, BootstrapEvmSourceState>) -> EvmChainIdentityRequest {
    EvmChainIdentityRequest::new(frame.input().value().binding.clone())
}

fn bootstrap_apply(
    frame: StateFrame<'_, BootstrapEvmSourceState>,
    observation: ObservationView<'_, EvmChainIdentityResponse, EvmSafeDiagnostic>,
) -> EvidenceVerdict<Settlement<BootstrapEvmSourceState>> {
    let response = match returned_or_failure::<BootstrapEvmSourceState>(observation) {
        Ok(response) => response,
        Err(verdict) => return verdict,
    };
    let binding = &frame.input().value().binding;
    if response.chain_id != binding.chain_id() {
        return EvidenceVerdict::Settlement(Settlement::failed(EvmReadFailure::SourceMismatch));
    }
    let source = match EvmCheckedSource::new(
        binding.clone(),
        response.source_scope.clone(),
        response.implementation_id.clone(),
    ) {
        Ok(source) => source,
        Err(_) => return EvidenceVerdict::InvalidEvidence,
    };
    succeeded_read(source)
}

impl State for ReadEvmInitialAnchorState {
    type Config = UnitConfig;
    type Context = NoContext;
    type Input = EvmInitialAnchorInput;
    type Output = EvmAnchoredSource;
    type Failure = EvmReadFailure;
    type Request = EvmLatestAnchorRequest;
    type Observation = EvmBlockResponse;
    type SafeDiagnostic = EvmSafeDiagnostic;

    fn state_contract_ref() -> mfm_program::Result<ContentRef> {
        state_contract_ref("read_initial_anchor")
    }
}

fn initial_anchor_request(
    frame: StateFrame<'_, ReadEvmInitialAnchorState>,
) -> EvmLatestAnchorRequest {
    EvmLatestAnchorRequest::new(frame.input().value().checked_source.clone())
}

fn initial_anchor_apply(
    frame: StateFrame<'_, ReadEvmInitialAnchorState>,
    observation: ObservationView<'_, EvmBlockResponse, EvmSafeDiagnostic>,
) -> EvidenceVerdict<Settlement<ReadEvmInitialAnchorState>> {
    let response = match returned_or_failure::<ReadEvmInitialAnchorState>(observation) {
        Ok(response) => response,
        Err(verdict) => return verdict,
    };
    match EvmAnchoredSource::new(
        frame.input().value().checked_source.clone(),
        response.anchor.clone(),
    ) {
        Ok(source) => succeeded_read(source),
        Err(_) => EvidenceVerdict::InvalidEvidence,
    }
}

impl State for ReadEvmTokenDecimalsState {
    type Config = UnitConfig;
    type Context = NoContext;
    type Input = EvmTokenDecimalsInput;
    type Output = EvmBalanceGraphResult;
    type Failure = EvmReadFailure;
    type Request = EvmTokenDecimalsRequest;
    type Observation = EvmTokenDecimalsResponse;
    type SafeDiagnostic = EvmSafeDiagnostic;

    fn state_contract_ref() -> mfm_program::Result<ContentRef> {
        state_contract_ref("read_token_decimals")
    }
}

fn token_decimals_request(
    frame: StateFrame<'_, ReadEvmTokenDecimalsState>,
) -> EvmTokenDecimalsRequest {
    let input = frame.input().value();
    let contract = parse_address(&input.contract_address).unwrap_or(Address::ZERO);
    EvmTokenDecimalsRequest::new(input.anchored_source.clone(), contract)
}

fn token_decimals_apply(
    frame: StateFrame<'_, ReadEvmTokenDecimalsState>,
    observation: ObservationView<'_, EvmTokenDecimalsResponse, EvmSafeDiagnostic>,
) -> EvidenceVerdict<Settlement<ReadEvmTokenDecimalsState>> {
    let response = match returned_or_failure::<ReadEvmTokenDecimalsState>(observation) {
        Ok(response) => response,
        Err(verdict) => return verdict,
    };
    let input = frame.input().value();
    if parse_address(&input.contract_address).is_err() {
        return EvidenceVerdict::InvalidEvidence;
    }
    succeeded_read(EvmBalanceGraphResult::TokenDecimals {
        source: input.anchored_source.clone(),
        contract_address: input.contract_address.clone(),
        decimals: response.decimals,
    })
}

impl State for ReadEvmNativeBalanceState {
    type Config = UnitConfig;
    type Context = NoContext;
    type Input = EvmBalanceReadInput;
    type Output = EvmBalanceGraphResult;
    type Failure = EvmReadFailure;
    type Request = EvmNativeBalanceRequest;
    type Observation = EvmQuantityResponse;
    type SafeDiagnostic = EvmSafeDiagnostic;

    fn state_contract_ref() -> mfm_program::Result<ContentRef> {
        state_contract_ref("read_native_balance")
    }
}

fn native_balance_request(
    frame: StateFrame<'_, ReadEvmNativeBalanceState>,
) -> EvmNativeBalanceRequest {
    let input = frame.input().value();
    let account = input
        .balance_source
        .account_address()
        .unwrap_or(Address::ZERO);
    EvmNativeBalanceRequest::new(input.anchored_source.clone(), account)
}

fn native_balance_apply(
    frame: StateFrame<'_, ReadEvmNativeBalanceState>,
    observation: ObservationView<'_, EvmQuantityResponse, EvmSafeDiagnostic>,
) -> EvidenceVerdict<Settlement<ReadEvmNativeBalanceState>> {
    let response = match returned_or_failure::<ReadEvmNativeBalanceState>(observation) {
        Ok(response) => response,
        Err(verdict) => return verdict,
    };
    let input = frame.input().value();
    if response.quantity().is_err() || !valid_native_source(&input.balance_source) {
        return EvidenceVerdict::InvalidEvidence;
    }
    succeeded_read(EvmBalanceGraphResult::Balance {
        anchored_source: input.anchored_source.clone(),
        balance_source: input.balance_source.clone(),
        raw_units: response.quantity_dec().to_owned(),
    })
}

impl State for ReadEvmTokenBalanceState {
    type Config = UnitConfig;
    type Context = NoContext;
    type Input = EvmBalanceReadInput;
    type Output = EvmBalanceGraphResult;
    type Failure = EvmReadFailure;
    type Request = EvmTokenBalanceRequest;
    type Observation = EvmQuantityResponse;
    type SafeDiagnostic = EvmSafeDiagnostic;

    fn state_contract_ref() -> mfm_program::Result<ContentRef> {
        state_contract_ref("read_token_balance")
    }
}

fn token_balance_request(
    frame: StateFrame<'_, ReadEvmTokenBalanceState>,
) -> EvmTokenBalanceRequest {
    let input = frame.input().value();
    let source = &input.balance_source;
    let account = source.account_address().unwrap_or(Address::ZERO);
    let contract = source
        .contract_address_value()
        .ok()
        .flatten()
        .unwrap_or(Address::ZERO);
    EvmTokenBalanceRequest::new(input.anchored_source.clone(), account, contract)
}

fn token_balance_apply(
    frame: StateFrame<'_, ReadEvmTokenBalanceState>,
    observation: ObservationView<'_, EvmQuantityResponse, EvmSafeDiagnostic>,
) -> EvidenceVerdict<Settlement<ReadEvmTokenBalanceState>> {
    let response = match returned_or_failure::<ReadEvmTokenBalanceState>(observation) {
        Ok(response) => response,
        Err(verdict) => return verdict,
    };
    let input = frame.input().value();
    if response.quantity().is_err() || !valid_token_source(&input.balance_source) {
        return EvidenceVerdict::InvalidEvidence;
    }
    succeeded_read(EvmBalanceGraphResult::Balance {
        anchored_source: input.anchored_source.clone(),
        balance_source: input.balance_source.clone(),
        raw_units: response.quantity_dec().to_owned(),
    })
}

fn valid_native_source(source: &EvmBalanceSource) -> bool {
    source.validate().is_ok() && matches!(source.asset, EvmBalanceAsset::Native)
}

fn valid_token_source(source: &EvmBalanceSource) -> bool {
    source.validate().is_ok() && matches!(source.asset, EvmBalanceAsset::Erc20 { .. })
}

impl State for ConfirmEvmAnchorState {
    type Config = UnitConfig;
    type Context = NoContext;
    type Input = EvmAnchorConfirmationInput;
    type Output = EvmBalanceGraphResult;
    type Failure = EvmReadFailure;
    type Request = EvmAnchorConfirmationRequest;
    type Observation = EvmBlockResponse;
    type SafeDiagnostic = EvmSafeDiagnostic;

    fn state_contract_ref() -> mfm_program::Result<ContentRef> {
        state_contract_ref("confirm_anchor")
    }
}

fn confirmation_request(
    frame: StateFrame<'_, ConfirmEvmAnchorState>,
) -> EvmAnchorConfirmationRequest {
    match common_fanout_source(&frame.input().value().fanout) {
        Some(source) => EvmAnchorConfirmationRequest::new(source.clone()),
        None => EvmAnchorConfirmationRequest::invalid_input(),
    }
}

fn confirmation_apply(
    frame: StateFrame<'_, ConfirmEvmAnchorState>,
    observation: ObservationView<'_, EvmBlockResponse, EvmSafeDiagnostic>,
) -> EvidenceVerdict<Settlement<ConfirmEvmAnchorState>> {
    let source = match common_fanout_source(&frame.input().value().fanout) {
        Some(source) => source,
        None => return EvidenceVerdict::InvalidEvidence,
    };
    let response = match returned_or_failure::<ConfirmEvmAnchorState>(observation) {
        Ok(response) => response,
        Err(verdict) => return verdict,
    };
    match check_anchor_confirmation(source, &response.anchor) {
        Ok(true) => succeeded_read(EvmBalanceGraphResult::AnchorConfirmation {
            source: source.clone(),
        }),
        Ok(false) => EvidenceVerdict::Settlement(Settlement::failed(EvmReadFailure::AnchorChanged)),
        Err(()) => EvidenceVerdict::InvalidEvidence,
    }
}

impl State for AggregateEvmBalancesState {
    type Config = UnitConfig;
    type Context = NoContext;
    type Input = EvmBalanceAggregationInput;
    type Output = EvmBalanceCollection;
    type Failure = EvmReadFailure;
    type Request = NoBoundaryValue;
    type Observation = NoBoundaryValue;
    type SafeDiagnostic = NoBoundaryValue;

    fn state_contract_ref() -> mfm_program::Result<ContentRef> {
        state_contract_ref("aggregate_balances")
    }
}

fn aggregate_apply(
    frame: StateFrame<'_, AggregateEvmBalancesState>,
) -> Settlement<AggregateEvmBalancesState> {
    let input = frame.input().value();
    let config = match EvmBalanceCollectionConfig::new(
        input.binding.clone(),
        input.native_decimals,
        input.sources.clone(),
    ) {
        Ok(config) if config.token_contracts() == input.token_contracts => config,
        Ok(_) | Err(_) => return Settlement::failed(EvmReadFailure::InvalidAggregate),
    };
    let (collection, facts) = match aggregate_collection(&config, &input.results) {
        Ok(result) => result,
        Err(_) => return Settlement::failed(EvmReadFailure::InvalidAggregate),
    };
    Settlement::succeeded(collection, facts)
}

fn aggregate_collection(
    config: &EvmBalanceCollectionConfig,
    values: &[EvmBalanceGraphResult],
) -> Result<(EvmBalanceCollection, FactSet), EvmBalanceCollectionError> {
    validate_evm_balance_collection_config(config)
        .map_err(|_| EvmBalanceCollectionError::Invalid("collection config"))?;

    let mut confirmation = None;
    let mut decimals = BTreeMap::new();
    let mut balances = BTreeMap::new();
    let mut common_source = None;
    for value in values {
        let source = value.anchored_source();
        if let Some(expected) = common_source {
            if expected != source {
                return Err(EvmBalanceCollectionError::Invalid(
                    "source or anchor disagreement",
                ));
            }
        } else {
            common_source = Some(source);
        }
        match value {
            EvmBalanceGraphResult::TokenDecimals {
                contract_address,
                decimals: scale,
                ..
            } => {
                let contract = parse_address(contract_address)?;
                if contract.is_zero() || decimals.insert(contract_address.clone(), *scale).is_some()
                {
                    return Err(EvmBalanceCollectionError::Invalid(
                        "duplicate or invalid token metadata",
                    ));
                }
            }
            EvmBalanceGraphResult::Balance {
                balance_source,
                raw_units,
                ..
            } => {
                balance_source.validate()?;
                parse_quantity(raw_units)?;
                if balances
                    .insert(balance_source.clone(), raw_units.clone())
                    .is_some()
                {
                    return Err(EvmBalanceCollectionError::Invalid(
                        "duplicate balance result",
                    ));
                }
            }
            EvmBalanceGraphResult::AnchorConfirmation { source } => {
                if confirmation.replace(source).is_some() {
                    return Err(EvmBalanceCollectionError::Invalid(
                        "duplicate anchor confirmation",
                    ));
                }
            }
        }
    }

    let source = confirmation
        .cloned()
        .ok_or(EvmBalanceCollectionError::Invalid(
            "missing anchor confirmation",
        ))?;
    if source.source().binding() != &config.binding {
        return Err(EvmBalanceCollectionError::Invalid(
            "collection binding disagreement",
        ));
    }
    let expected_tokens = config.token_contracts();
    if decimals.len() != expected_tokens.len()
        || expected_tokens
            .iter()
            .any(|contract| !decimals.contains_key(contract))
        || balances.len() != config.sources.len()
    {
        return Err(EvmBalanceCollectionError::Invalid(
            "incomplete collection coverage",
        ));
    }

    let mut collected = Vec::with_capacity(config.sources.len());
    let mut proposals = Vec::with_capacity(config.sources.len());
    for requested in &config.sources {
        let raw_units = balances
            .remove(requested)
            .ok_or(EvmBalanceCollectionError::Invalid("missing balance result"))?;
        let scale = match requested.asset() {
            EvmBalanceAsset::Native => config.native_decimals,
            EvmBalanceAsset::Erc20 { contract_address } => *decimals
                .get(contract_address)
                .ok_or(EvmBalanceCollectionError::Invalid("missing token metadata"))?,
        };
        let balance = EvmCollectedBalance {
            source: requested.clone(),
            raw_units: raw_units.clone(),
            decimals: scale,
        };
        let fact = EvmBalanceSnapshotFact {
            network_id: config.binding.network_id().to_owned(),
            chain_id: config.binding.chain_id(),
            account: requested.account.clone(),
            asset: requested.asset.clone(),
            block_anchor: source.anchor().clone(),
            raw_units,
            decimals: scale,
        };
        proposals.push(fact_proposal(&fact)?);
        collected.push(balance);
    }
    let facts = FactSet::try_from_iter(proposals)?;
    Ok((
        EvmBalanceCollection {
            source,
            balances: collected,
        },
        facts,
    ))
}

fn common_fanout_source(values: &[EvmBalanceGraphResult]) -> Option<&EvmAnchoredSource> {
    let first = values.first()?.anchored_source();
    if values.iter().all(|value| {
        !matches!(value, EvmBalanceGraphResult::AnchorConfirmation { .. })
            && value.anchored_source() == first
    }) {
        Some(first)
    } else {
        None
    }
}

fn check_anchor_confirmation(
    source: &EvmAnchoredSource,
    returned: &EvmBlockAnchor,
) -> std::result::Result<bool, ()> {
    returned.validate().map_err(|_| ())?;
    if returned.number() != source.anchor().number() {
        return Err(());
    }
    Ok(returned.hash() == source.anchor().hash())
}

fn returned_or_failure<'a, S>(
    observation: ObservationView<'a, S::Observation, EvmSafeDiagnostic>,
) -> std::result::Result<&'a S::Observation, EvidenceVerdict<Settlement<S>>>
where
    S: State<Failure = EvmReadFailure, SafeDiagnostic = EvmSafeDiagnostic>,
{
    match observation.outcome() {
        ObservationOutcome::Returned(response) => Ok(response),
        ObservationOutcome::DidNotEnter(failure) => Err(failure_verdict::<S>(failure, false)),
        ObservationOutcome::Indeterminate(failure) => Err(failure_verdict::<S>(failure, true)),
    }
}

fn failure_verdict<S>(
    failure: &mfm_program::ObservedSafeFailure<EvmSafeDiagnostic>,
    entered_or_indeterminate: bool,
) -> EvidenceVerdict<Settlement<S>>
where
    S: State<Failure = EvmReadFailure>,
{
    failure_verdict_projection::<S>(
        failure.stable_code().as_str(),
        entered_or_indeterminate,
        failure.diagnostic(),
    )
}

fn failure_verdict_projection<S>(
    stable_code: &str,
    entered_or_indeterminate: bool,
    diagnostic: Option<&EvmSafeDiagnostic>,
) -> EvidenceVerdict<Settlement<S>>
where
    S: State<Failure = EvmReadFailure>,
{
    match (stable_code, entered_or_indeterminate, diagnostic) {
        (
            "routing_generation_unavailable" | "configuration_invalid" | "request_invalid",
            false,
            None,
        )
        | (
            "response_invalid",
            true,
            Some(EvmSafeDiagnostic::ResponseInvalid {
                kind:
                    EvmResponseInvalidKind::MalformedEnvelope | EvmResponseInvalidKind::InvalidResult,
            }),
        )
        | (
            "response_missing_result",
            true,
            Some(EvmSafeDiagnostic::ResponseInvalid {
                kind: EvmResponseInvalidKind::MissingResult,
            }),
        )
        | (
            "response_too_large",
            true,
            Some(EvmSafeDiagnostic::ResponseInvalid {
                kind: EvmResponseInvalidKind::TooLarge,
            }),
        ) => EvidenceVerdict::InvalidEvidence,
        ("access_cancelled" | "transport_failed", _, None)
        | ("unclassified_failure", true, None) => EvidenceVerdict::InsufficientEvidence,
        ("http_status", true, Some(EvmSafeDiagnostic::HttpStatus { status }))
            if http_status_is_insufficient_evidence(*status) =>
        {
            EvidenceVerdict::InsufficientEvidence
        }
        ("json_rpc_error", true, Some(EvmSafeDiagnostic::JsonRpcError { code }))
            if json_rpc_code_is_insufficient_evidence(*code) =>
        {
            EvidenceVerdict::InsufficientEvidence
        }
        ("http_status", true, Some(EvmSafeDiagnostic::HttpStatus { .. }))
        | ("json_rpc_error", true, Some(EvmSafeDiagnostic::JsonRpcError { .. })) => {
            EvidenceVerdict::Settlement(Settlement::failed(EvmReadFailure::DestinationRejected))
        }
        _ => EvidenceVerdict::InvalidEvidence,
    }
}

const fn http_status_is_insufficient_evidence(status: u16) -> bool {
    matches!(status, 408 | 425 | 429 | 500 | 502 | 503 | 504 | 507)
}

const fn json_rpc_code_is_insufficient_evidence(code: i64) -> bool {
    matches!(code, -32603 | -32001 | -32002 | -32005)
}

fn succeeded_read<S>(output: S::Output) -> EvidenceVerdict<Settlement<S>>
where
    S: State<Failure = EvmReadFailure>,
{
    EvidenceVerdict::Settlement(Settlement::succeeded(output, FactSet::empty()))
}

#[allow(clippy::too_many_arguments)]
fn read_execution<S>(
    operation_id: &'static str,
    binding_ref: ContentRef,
    request_contract: RetainedValueContract,
    returned_contract: RetainedValueContract,
    safe_failure_contract: RetainedValueContract,
    request: mfm_program::RequestAuthor<S>,
    apply: mfm_program::ReadApply<S>,
) -> mfm_program::Result<StateExecution<S>>
where
    S: State<SafeDiagnostic = EvmSafeDiagnostic>,
    S::Request: mfm_values::MfmValue,
    S::Observation: mfm_values::MfmValue,
{
    Ok(StateExecution::read(
        CapabilityOperation::new(
            evm_read_capability_contract_ref()?,
            StableId::new(operation_id)
                .map_err(|error| mfm_program::ProgramError::Codec(error.to_string()))?,
            binding_ref,
        ),
        CanonicalCodec::mfm_value(request_contract)?,
        CanonicalCodec::mfm_value(returned_contract)?,
        CanonicalCodec::mfm_value(safe_failure_contract)?,
        request,
        apply,
    ))
}

pub(crate) fn bootstrap_execution(
    binding_ref: ContentRef,
    request_contract: RetainedValueContract,
    returned_contract: RetainedValueContract,
    safe_failure_contract: RetainedValueContract,
) -> mfm_program::Result<StateExecution<BootstrapEvmSourceState>> {
    read_execution(
        EVM_CHAIN_ID_OPERATION_ID,
        binding_ref,
        request_contract,
        returned_contract,
        safe_failure_contract,
        bootstrap_request,
        bootstrap_apply,
    )
}

pub(crate) fn initial_anchor_execution(
    binding_ref: ContentRef,
    request_contract: RetainedValueContract,
    returned_contract: RetainedValueContract,
    safe_failure_contract: RetainedValueContract,
) -> mfm_program::Result<StateExecution<ReadEvmInitialAnchorState>> {
    read_execution(
        EVM_LATEST_ANCHOR_OPERATION_ID,
        binding_ref,
        request_contract,
        returned_contract,
        safe_failure_contract,
        initial_anchor_request,
        initial_anchor_apply,
    )
}

pub(crate) fn token_decimals_execution(
    binding_ref: ContentRef,
    request_contract: RetainedValueContract,
    returned_contract: RetainedValueContract,
    safe_failure_contract: RetainedValueContract,
) -> mfm_program::Result<StateExecution<ReadEvmTokenDecimalsState>> {
    read_execution(
        EVM_TOKEN_DECIMALS_OPERATION_ID,
        binding_ref,
        request_contract,
        returned_contract,
        safe_failure_contract,
        token_decimals_request,
        token_decimals_apply,
    )
}

pub(crate) fn native_balance_execution(
    binding_ref: ContentRef,
    request_contract: RetainedValueContract,
    returned_contract: RetainedValueContract,
    safe_failure_contract: RetainedValueContract,
) -> mfm_program::Result<StateExecution<ReadEvmNativeBalanceState>> {
    read_execution(
        EVM_NATIVE_BALANCE_OPERATION_ID,
        binding_ref,
        request_contract,
        returned_contract,
        safe_failure_contract,
        native_balance_request,
        native_balance_apply,
    )
}

pub(crate) fn token_balance_execution(
    binding_ref: ContentRef,
    request_contract: RetainedValueContract,
    returned_contract: RetainedValueContract,
    safe_failure_contract: RetainedValueContract,
) -> mfm_program::Result<StateExecution<ReadEvmTokenBalanceState>> {
    read_execution(
        EVM_TOKEN_BALANCE_OPERATION_ID,
        binding_ref,
        request_contract,
        returned_contract,
        safe_failure_contract,
        token_balance_request,
        token_balance_apply,
    )
}

pub(crate) fn confirmation_execution(
    binding_ref: ContentRef,
    request_contract: RetainedValueContract,
    returned_contract: RetainedValueContract,
    safe_failure_contract: RetainedValueContract,
) -> mfm_program::Result<StateExecution<ConfirmEvmAnchorState>> {
    read_execution(
        EVM_CONFIRM_ANCHOR_OPERATION_ID,
        binding_ref,
        request_contract,
        returned_contract,
        safe_failure_contract,
        confirmation_request,
        confirmation_apply,
    )
}

pub(crate) const fn aggregate_execution() -> StateExecution<AggregateEvmBalancesState> {
    StateExecution::pure(aggregate_apply)
}

/// Returns the exact domain-owned EVM read capability contract.
pub fn evm_read_capability_contract_canonical() -> mfm_program::Result<PlainCanonicalJsonBytes> {
    PlainCanonicalJsonBytes::from_json_str(
        r#"{"operations":["eth_call_erc20_balance_of","eth_call_erc20_decimals","eth_chain_id","eth_get_balance","eth_get_block_by_number_confirm","eth_get_block_by_number_latest"],"version":"mfm.evm.read-capability.v1"}"#,
    )
    .map_err(|error| mfm_program::ProgramError::Codec(error.to_string()))
}

/// Returns the exact domain-owned EVM read capability contract identity.
pub fn evm_read_capability_contract_ref() -> mfm_program::Result<ContentRef> {
    boundary_content_ref(
        contract_schema_id("mfm.evm.read-capability")?,
        &evm_read_capability_contract_canonical()?,
    )
}

/// Builds retained metadata for the EVM capability-contract support object.
pub fn evm_read_capability_support_contract(
    role: StableId,
    evidence_contract_ref: ContentRef,
) -> mfm_program::Result<RetainedValueContract> {
    support_contract(
        "mfm.evm.read-capability",
        "read-capability-contract",
        role,
        evidence_contract_ref,
    )
}

/// Returns exact canonical bytes of the closed EVM safe-failure contract.
pub fn evm_safe_failure_contract_canonical() -> mfm_program::Result<PlainCanonicalJsonBytes> {
    PlainCanonicalJsonBytes::from_json_str(
        r#"{"codes":["access_cancelled","configuration_invalid","http_status","json_rpc_error","request_invalid","response_invalid","response_missing_result","response_too_large","routing_generation_unavailable","transport_failed","unclassified_failure"],"diagnostic_schema":"mfm.evm.safe_diagnostic","version":"mfm.evm.safe-failure-contract.v1"}"#,
    )
    .map_err(|error| mfm_program::ProgramError::Codec(error.to_string()))
}

/// Returns the exact closed safe-failure contract used by EVM read bindings.
pub fn evm_safe_failure_contract_ref() -> mfm_program::Result<ContentRef> {
    boundary_content_ref(
        contract_schema_id("mfm.evm.safe-failure-contract")?,
        &evm_safe_failure_contract_canonical()?,
    )
}

/// Builds retained metadata for the closed safe-failure contract support object.
pub fn evm_safe_failure_support_contract(
    role: StableId,
    evidence_contract_ref: ContentRef,
) -> mfm_program::Result<RetainedValueContract> {
    support_contract(
        "mfm.evm.safe-failure-contract",
        "safe-failure-contract",
        role,
        evidence_contract_ref,
    )
}

pub(crate) fn state_contract_canonical(
    name: &'static str,
) -> mfm_program::Result<PlainCanonicalJsonBytes> {
    PlainCanonicalJsonBytes::from_json_str(&format!(
        r#"{{"name":"{name}","version":"mfm.evm.state.{name}.v1"}}"#
    ))
    .map_err(|error| mfm_program::ProgramError::Codec(error.to_string()))
}

fn state_contract_ref(name: &'static str) -> mfm_program::Result<ContentRef> {
    boundary_content_ref(
        contract_schema_id("mfm.evm.state-contract")?,
        &state_contract_canonical(name)?,
    )
}

pub(crate) fn balance_fact_descriptor_canonical() -> mfm_program::Result<PlainCanonicalJsonBytes> {
    let descriptor = serde_json::json!({
        "fact_kind": "mfm.evm/balance-snapshot@1",
        "response_schema": EvmBalanceSnapshotFact::schema_id()
            .map_err(|error| mfm_program::ProgramError::Codec(error.to_string()))?
            .as_str(),
        "subject_schema": EvmBalanceSnapshotSubject::schema_id()
            .map_err(|error| mfm_program::ProgramError::Codec(error.to_string()))?
            .as_str(),
        "version": "mfm.fact-descriptor.v1",
    });
    PlainCanonicalJsonBytes::from_json_str(&descriptor.to_string())
        .map_err(|error| mfm_program::ProgramError::Codec(error.to_string()))
}

pub(crate) fn balance_fact_descriptor_ref() -> mfm_program::Result<ContentRef> {
    boundary_content_ref(
        contract_schema_id("mfm.fact-descriptor")?,
        &balance_fact_descriptor_canonical()?,
    )
}

pub(crate) fn fact_evidence_contract_canonical(
    component: &str,
) -> mfm_program::Result<PlainCanonicalJsonBytes> {
    let contract = serde_json::json!({
        "component": component,
        "version": "mfm.evm.fact-object-evidence-contract.v1",
    });
    PlainCanonicalJsonBytes::from_json_str(&contract.to_string())
        .map_err(|error| mfm_program::ProgramError::Codec(error.to_string()))
}

pub(crate) fn fact_evidence_contract_ref(component: &str) -> mfm_program::Result<ContentRef> {
    boundary_content_ref(
        contract_schema_id("mfm.object-evidence-contract")?,
        &fact_evidence_contract_canonical(component)?,
    )
}

pub(crate) fn contract_schema_id(name: &'static str) -> mfm_program::Result<SchemaId> {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("schema:{name}:1").as_bytes()),
    )
    .map_err(|error| mfm_program::ProgramError::Codec(error.to_string()))
}

pub(crate) fn support_contract(
    schema_name: &'static str,
    semantic_name: &'static str,
    role: StableId,
    evidence_contract_ref: ContentRef,
) -> mfm_program::Result<RetainedValueContract> {
    let semantic_type_id = SemanticTypeId::new(
        "mfm.evm",
        semantic_name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("semantic:mfm.evm:{semantic_name}:1").as_bytes()),
    )
    .map_err(|error| mfm_program::ProgramError::Codec(error.to_string()))?;
    RetainedValueContract::new(
        contract_schema_id(schema_name)?,
        semantic_type_id,
        role,
        "application/json",
        evidence_contract_ref,
    )
    .map_err(|error| mfm_program::ProgramError::Codec(error.to_string()))
}

fn fact_proposal(fact: &EvmBalanceSnapshotFact) -> Result<FactProposal, EvmBalanceCollectionError> {
    let subject = EvmBalanceSnapshotSubject {
        network_id: fact.network_id.clone(),
        chain_id: fact.chain_id,
        account: fact.account.clone(),
        asset: fact.asset.clone(),
    };
    let subject = ProposedFactValue::new(
        EvmBalanceSnapshotSubject::schema_id()
            .map_err(|error| EvmBalanceCollectionError::Program(error.to_string()))?,
        EvmBalanceSnapshotSubject::semantic_id()
            .map_err(|error| EvmBalanceCollectionError::Program(error.to_string()))?,
        StableId::new("mfm.evm.fact.balance-snapshot.subject")
            .map_err(|error| EvmBalanceCollectionError::Program(error.to_string()))?,
        "application/json",
        fact_evidence_contract_ref("subject")?,
        encode_boundary(&subject)?,
    )?;
    let response = ProposedFactValue::new(
        EvmBalanceSnapshotFact::schema_id()
            .map_err(|error| EvmBalanceCollectionError::Program(error.to_string()))?,
        EvmBalanceSnapshotFact::semantic_id()
            .map_err(|error| EvmBalanceCollectionError::Program(error.to_string()))?,
        StableId::new("mfm.evm.fact.balance-snapshot.response")
            .map_err(|error| EvmBalanceCollectionError::Program(error.to_string()))?,
        "application/json",
        fact_evidence_contract_ref("response")?,
        encode_boundary(fact)?,
    )?;
    FactProposal::new(0, balance_fact_descriptor_ref()?, subject, response).map_err(Into::into)
}

fn canonical_address(address: Address) -> String {
    format!("{address:#x}")
}

fn parse_address(raw: &str) -> Result<Address, EvmBalanceCollectionError> {
    let address =
        Address::from_str(raw).map_err(|_| EvmBalanceCollectionError::Invalid("address"))?;
    if canonical_address(address) != raw {
        return Err(EvmBalanceCollectionError::Invalid("address"));
    }
    Ok(address)
}

fn parse_quantity(raw: &str) -> Result<U256, EvmBalanceCollectionError> {
    if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(EvmBalanceCollectionError::Invalid("quantity"));
    }
    let quantity =
        U256::from_str(raw).map_err(|_| EvmBalanceCollectionError::Invalid("quantity"))?;
    if quantity.to_string() != raw {
        return Err(EvmBalanceCollectionError::Invalid("quantity"));
    }
    Ok(quantity)
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
