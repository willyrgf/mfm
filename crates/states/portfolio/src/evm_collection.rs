//! Portfolio-owned EVM network collection state contracts.

use std::collections::{BTreeMap, BTreeSet};
use std::future;
use std::str::FromStr;

use alloy_primitives::{Address, B256, U256};
use mfm_evm_capabilities::{
    EvmNetworkBinding, EvmReadCapability, EvmSessionEvidence, EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_fact_capabilities::FactRecordCapability;
use mfm_facts::{FactAudience, FactVisibility};
use mfm_ids::LocalPublicId;
use mfm_portfolio_model::portfolio::{NetworkConfig, NetworkFamilyConfig};
use mfm_program::{
    fact_descriptor_ref, ExternalReadEvidenceSet, FactDescriptorRef, ManagedWriteState, NoContext,
    ReadState, StateError, StateResult, StateSpec, ValidatedConfig,
};
use mfm_program_derive::{MfmConfig, MfmFactType as DeriveMfmFactType, MfmValue, StateInput};
use mfm_values::ConfigError;
use serde::{Deserialize, Serialize};

use crate::{adapter_binding, state_kind, state_version};

pub use mfm_portfolio_model::portfolio::EVM_NETWORK_HOLDING_SOURCE_LIMIT;

/// Redaction-safe portfolio EVM state failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortfolioEvmError {
    /// Certified config, state input, or retained evidence was invalid.
    #[error("portfolio EVM collection material was invalid: {reason}")]
    Invalid {
        /// Stable redaction-safe reason.
        reason: String,
    },
}

impl From<PortfolioEvmError> for StateError {
    fn from(error: PortfolioEvmError) -> Self {
        Self::Message(error.to_string())
    }
}

fn invalid(reason: impl Into<String>) -> PortfolioEvmError {
    PortfolioEvmError::Invalid {
        reason: reason.into(),
    }
}

/// One balance asset in a portfolio-owned EVM collection plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "evm_balance_asset",
    schema = "mfm.portfolio.evm.balance_asset"
)]
pub enum EvmBalanceAsset {
    /// The semantic network's native asset.
    Native,
    /// One ERC-20 contract on the semantic network.
    Erc20 {
        /// Canonical non-zero ERC-20 contract address.
        contract_address: String,
    },
}

impl EvmBalanceAsset {
    /// Creates a checked ERC-20 asset.
    pub fn erc20(contract_address: impl Into<String>) -> Result<Self, PortfolioEvmError> {
        let contract_address = contract_address.into();
        let parsed = parse_address(&contract_address)?;
        if parsed.is_zero() {
            return Err(invalid("ERC-20 contract address must be non-zero"));
        }
        Ok(Self::Erc20 { contract_address })
    }

    /// Returns the ERC-20 contract address when this is a token asset.
    pub fn contract_address(&self) -> Option<&str> {
        match self {
            Self::Native => None,
            Self::Erc20 { contract_address } => Some(contract_address),
        }
    }

    fn validate(&self) -> Result<(), PortfolioEvmError> {
        match self {
            Self::Native => Ok(()),
            Self::Erc20 { contract_address } => {
                let parsed = parse_address(contract_address)?;
                if parsed.is_zero() {
                    return Err(invalid("ERC-20 contract address must be non-zero"));
                }
                Ok(())
            }
        }
    }
}

/// One unique account/asset balance source collected for a portfolio.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "evm_balance_source",
    schema = "mfm.portfolio.evm.balance_source"
)]
pub struct EvmBalanceSource {
    account: String,
    asset: EvmBalanceAsset,
}

impl EvmBalanceSource {
    /// Creates one checked balance source.
    pub fn new(
        account: impl Into<String>,
        asset: EvmBalanceAsset,
    ) -> Result<Self, PortfolioEvmError> {
        let source = Self {
            account: account.into(),
            asset,
        };
        source.validate()?;
        Ok(source)
    }

    /// Returns the canonical account address.
    pub fn account(&self) -> &str {
        &self.account
    }

    /// Returns the collected asset.
    pub const fn asset(&self) -> &EvmBalanceAsset {
        &self.asset
    }

    /// Returns the parsed account address for adapter execution.
    pub fn account_address(&self) -> Result<Address, PortfolioEvmError> {
        parse_address(&self.account)
    }

    /// Returns the parsed token contract address when this is an ERC-20 source.
    pub fn contract_address_value(&self) -> Result<Option<Address>, PortfolioEvmError> {
        self.asset.contract_address().map(parse_address).transpose()
    }

    fn validate(&self) -> Result<(), PortfolioEvmError> {
        parse_address(&self.account)?;
        self.asset.validate()
    }
}

/// Certified demand for one portfolio-owned EVM network collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    schema = "mfm.portfolio.config.collect_evm_network",
    validate = "validate_evm_network_collection_config"
)]
pub struct EvmNetworkCollectionConfig {
    network_id: String,
    chain_id: u64,
    native_decimals: u8,
    sources: Vec<EvmBalanceSource>,
}

impl EvmNetworkCollectionConfig {
    /// Creates a checked, sorted collection config from one EVM network.
    pub fn new(
        network: &NetworkConfig,
        mut sources: Vec<EvmBalanceSource>,
    ) -> Result<Self, ConfigError> {
        if network.family() != NetworkFamilyConfig::Evm {
            return Err(ConfigError::new(
                "EVM collection config requires an EVM network",
            ));
        }
        sources.sort();
        let config = Self {
            network_id: network.network_id().to_string(),
            chain_id: network
                .chain_id_u64()
                .ok_or_else(|| ConfigError::new("EVM network chain id was missing"))?,
            native_decimals: network
                .native_decimals()
                .ok_or_else(|| ConfigError::new("EVM network native decimals were missing"))?,
            sources,
        };
        validate_evm_network_collection_config(&config).map_err(ConfigError::new)?;
        Ok(config)
    }

    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the required chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the configured native-asset scale.
    pub const fn native_decimals(&self) -> u8 {
        self.native_decimals
    }

    /// Returns the exact sorted, unique source demand.
    pub fn sources(&self) -> &[EvmBalanceSource] {
        &self.sources
    }

    /// Returns the checked capability binding for this semantic network.
    pub fn binding(&self) -> Result<EvmNetworkBinding, PortfolioEvmError> {
        network_binding(&self.network_id, self.chain_id)
    }
}

/// Validates bounded, sorted, unique EVM network collection demand.
pub fn validate_evm_network_collection_config(
    config: &EvmNetworkCollectionConfig,
) -> Result<(), String> {
    LocalPublicId::new(&config.network_id).map_err(|error| error.to_string())?;
    if config.chain_id == 0 {
        return Err("EVM collection chain id must be non-zero".to_owned());
    }
    if config.sources.is_empty() || config.sources.len() > EVM_NETWORK_HOLDING_SOURCE_LIMIT {
        return Err(format!(
            "EVM collection sources must contain between 1 and {EVM_NETWORK_HOLDING_SOURCE_LIMIT} entries"
        ));
    }
    let mut previous: Option<&EvmBalanceSource> = None;
    for source in &config.sources {
        source.validate().map_err(|error| error.to_string())?;
        if previous.is_some_and(|prior| prior >= source) {
            return Err("EVM collection sources must be strictly sorted and unique".to_owned());
        }
        previous = Some(source);
    }
    Ok(())
}

/// Empty input for an independently planned EVM network read.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.portfolio.input.collect_evm_network")]
pub struct CollectEvmNetworkInput {}

/// Deterministic request plan for one EVM network collection attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "collect_evm_network_plan",
    schema = "mfm.portfolio.external_read.collect_evm_network.plan"
)]
pub struct CollectEvmNetworkPlan {
    network_id: String,
    chain_id: u64,
    native_decimals: u8,
    sources: Vec<EvmBalanceSource>,
}

impl CollectEvmNetworkPlan {
    /// Returns the checked source-stable session binding.
    pub fn binding(&self) -> Result<EvmNetworkBinding, PortfolioEvmError> {
        network_binding(&self.network_id, self.chain_id)
    }

    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the checked chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the exact sorted source request sequence.
    pub fn sources(&self) -> &[EvmBalanceSource] {
        &self.sources
    }

    /// Returns distinct token contracts in canonical order.
    pub fn token_contracts(&self) -> Vec<String> {
        self.sources
            .iter()
            .filter_map(|source| source.asset.contract_address().map(ToOwned::to_owned))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn config(&self) -> EvmNetworkCollectionConfig {
        EvmNetworkCollectionConfig {
            network_id: self.network_id.clone(),
            chain_id: self.chain_id,
            native_decimals: self.native_decimals,
            sources: self.sources.clone(),
        }
    }
}

/// Canonical block anchor retained for collection replay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "evm_collection_anchor",
    schema = "mfm.portfolio.evm.collection_anchor"
)]
pub struct EvmCollectionAnchor {
    block_number: u64,
    block_hash: String,
}

impl EvmCollectionAnchor {
    /// Creates a checked canonical anchor.
    pub fn new(
        block_number: u64,
        block_hash: impl Into<String>,
    ) -> Result<Self, PortfolioEvmError> {
        let block_hash = block_hash.into();
        parse_hash(&block_hash)?;
        Ok(Self {
            block_number,
            block_hash,
        })
    }

    /// Returns the block number.
    pub const fn block_number(&self) -> u64 {
        self.block_number
    }

    /// Returns the canonical block hash.
    pub fn block_hash(&self) -> &str {
        &self.block_hash
    }

    /// Returns the parsed block hash for an exact EIP-1898 selector.
    pub fn block_hash_value(&self) -> Result<B256, PortfolioEvmError> {
        parse_hash(&self.block_hash)
    }
}

/// Redacted provenance for one source-bound EVM collection session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "evm_collection_session",
    schema = "mfm.portfolio.evm.collection_session"
)]
pub struct EvmCollectionSession {
    network_id: String,
    chain_id: u64,
    source_ref: String,
    implementation_id: String,
}

impl EvmCollectionSession {
    /// Converts checked bind-time evidence into retained redacted provenance.
    pub fn from_session(evidence: &EvmSessionEvidence) -> Self {
        Self {
            network_id: evidence.network_id().as_str().to_owned(),
            chain_id: evidence.chain_id(),
            source_ref: evidence.source_ref().as_str().to_owned(),
            implementation_id: evidence.implementation_id().as_str().to_owned(),
        }
    }

    fn validate(&self, plan: &CollectEvmNetworkPlan) -> Result<(), PortfolioEvmError> {
        if self.network_id != plan.network_id
            || self.chain_id != plan.chain_id
            || self.implementation_id != EVM_JSONRPC_SESSION_IMPLEMENTATION_ID
        {
            return Err(invalid(
                "EVM collection session did not match certified network authority",
            ));
        }
        LocalPublicId::new(&self.source_ref)
            .map_err(|_| invalid("EVM collection source reference was invalid"))?;
        LocalPublicId::new(&self.implementation_id)
            .map_err(|_| invalid("EVM collection implementation identity was invalid"))?;
        Ok(())
    }
}

/// One exact token-decimals result retained once per distinct contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "evm_token_decimals_evidence",
    schema = "mfm.portfolio.evm.token_decimals_evidence"
)]
pub struct EvmTokenDecimalsEvidence {
    contract_address: String,
    decimals: u8,
}

impl EvmTokenDecimalsEvidence {
    /// Creates checked token metadata evidence.
    pub fn new(
        contract_address: impl Into<String>,
        decimals: u8,
    ) -> Result<Self, PortfolioEvmError> {
        let contract_address = contract_address.into();
        EvmBalanceAsset::erc20(contract_address.clone())?;
        Ok(Self {
            contract_address,
            decimals,
        })
    }

    /// Returns the canonical token contract address.
    pub fn contract_address(&self) -> &str {
        &self.contract_address
    }

    /// Returns the exact observed decimal scale.
    pub const fn decimals(&self) -> u8 {
        self.decimals
    }
}

/// One exact hash-selected balance result retained for replay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "evm_balance_read_evidence",
    schema = "mfm.portfolio.evm.balance_read_evidence"
)]
pub struct EvmBalanceReadEvidence {
    source: EvmBalanceSource,
    raw_units: String,
}

impl EvmBalanceReadEvidence {
    /// Creates canonical balance evidence from a checked quantity.
    pub fn new(source: EvmBalanceSource, raw_units: U256) -> Self {
        Self {
            source,
            raw_units: raw_units.to_string(),
        }
    }

    /// Returns the exact source.
    pub const fn source(&self) -> &EvmBalanceSource {
        &self.source
    }

    /// Returns the canonical decimal balance.
    pub fn raw_units(&self) -> &str {
        &self.raw_units
    }
}

/// Complete retained evidence for one source-bound EVM network read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "collect_evm_network_evidence",
    schema = "mfm.portfolio.external_read.collect_evm_network.evidence"
)]
pub struct CollectEvmNetworkEvidence {
    session: EvmCollectionSession,
    anchor: EvmCollectionAnchor,
    token_decimals: Vec<EvmTokenDecimalsEvidence>,
    balances: Vec<EvmBalanceReadEvidence>,
    final_canonical_block: EvmCollectionAnchor,
}

impl CollectEvmNetworkEvidence {
    /// Creates canonical evidence from one live checked session attempt.
    pub fn new(
        session: &EvmSessionEvidence,
        anchor: EvmCollectionAnchor,
        mut token_decimals: Vec<EvmTokenDecimalsEvidence>,
        mut balances: Vec<EvmBalanceReadEvidence>,
        final_canonical_block: EvmCollectionAnchor,
    ) -> Self {
        token_decimals.sort();
        balances.sort();
        Self {
            session: EvmCollectionSession::from_session(session),
            anchor,
            token_decimals,
            balances,
            final_canonical_block,
        }
    }
}

/// One normalized balance in a reduced collection batch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "evm_collected_balance",
    schema = "mfm.portfolio.evm.collected_balance"
)]
pub struct EvmCollectedBalance {
    source: EvmBalanceSource,
    raw_units: String,
    decimals: u8,
}

impl EvmCollectedBalance {
    /// Returns the exact source.
    pub const fn source(&self) -> &EvmBalanceSource {
        &self.source
    }

    /// Returns the canonical raw balance.
    pub fn raw_units(&self) -> &str {
        &self.raw_units
    }

    /// Returns the exact asset decimal scale.
    pub const fn decimals(&self) -> u8 {
        self.decimals
    }
}

/// Reduced, canonical observation batch produced by the external-read state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "evm_collection_batch",
    schema = "mfm.portfolio.evm.collection_batch"
)]
pub struct EvmCollectionBatch {
    network_id: String,
    chain_id: u64,
    anchor: EvmCollectionAnchor,
    balances: Vec<EvmCollectedBalance>,
}

impl EvmCollectionBatch {
    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the checked chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the common exact canonical anchor.
    pub const fn anchor(&self) -> &EvmCollectionAnchor {
        &self.anchor
    }

    /// Returns every collected balance in source order.
    pub fn balances(&self) -> &[EvmCollectedBalance] {
        &self.balances
    }

    fn validate_against(
        &self,
        config: &EvmNetworkCollectionConfig,
    ) -> Result<(), PortfolioEvmError> {
        validate_evm_network_collection_config(config).map_err(invalid)?;
        parse_hash(&self.anchor.block_hash)?;
        if self.network_id != config.network_id
            || self.chain_id != config.chain_id
            || self.balances.len() != config.sources.len()
        {
            return Err(invalid(
                "EVM collection batch did not match certified collection demand",
            ));
        }
        let token_decimals = self
            .balances
            .iter()
            .filter_map(|balance| {
                balance
                    .source
                    .asset
                    .contract_address()
                    .map(|contract| (contract, balance.decimals))
            })
            .collect::<BTreeMap<_, _>>();
        for (expected, balance) in config.sources.iter().zip(&self.balances) {
            balance.source.validate()?;
            parse_quantity(&balance.raw_units)?;
            if &balance.source != expected {
                return Err(invalid(
                    "EVM collection batch source coverage was not exact",
                ));
            }
            match balance.source.asset() {
                EvmBalanceAsset::Native if balance.decimals != config.native_decimals => {
                    return Err(invalid(
                        "native balance decimals did not match network config",
                    ));
                }
                EvmBalanceAsset::Erc20 { contract_address }
                    if token_decimals.get(contract_address.as_str()) != Some(&balance.decimals) =>
                {
                    return Err(invalid(
                        "ERC-20 balances disagreed on one contract decimal scale",
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// State that collects every configured EVM holding at one exact canonical anchor.
pub struct CollectEvmNetworkState {
    config: EvmNetworkCollectionConfig,
}

impl CollectEvmNetworkState {
    /// Returns the certified collection config.
    pub const fn config(&self) -> &EvmNetworkCollectionConfig {
        &self.config
    }
}

impl StateSpec for CollectEvmNetworkState {
    type Config = EvmNetworkCollectionConfig;
    type Context = NoContext;
    type Input = CollectEvmNetworkInput;
    type Output = EvmCollectionBatch;
    type Effect = mfm_effects::ReadExternal;
    type Caps = (EvmReadCapability,);

    fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
        state_kind("collect_evm_network")
    }

    fn version() -> mfm_program::Result<mfm_ids::StateVersion> {
        state_version("collect_evm_network")
    }

    fn name() -> &'static str {
        "mfm.portfolio.collect_evm_network"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<mfm_program::AdapterBindingSpec>> {
        adapter_binding()
    }

    fn new(config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ReadState for CollectEvmNetworkState {
    type Plan = CollectEvmNetworkPlan;
    type Evidence = CollectEvmNetworkEvidence;

    fn plan(
        &self,
        _input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Plan> {
        validate_evm_network_collection_config(&self.config).map_err(StateError::Message)?;
        Ok(CollectEvmNetworkPlan {
            network_id: self.config.network_id.clone(),
            chain_id: self.config.chain_id,
            native_decimals: self.config.native_decimals,
            sources: self.config.sources.clone(),
        })
    }

    fn reduce(
        &self,
        input: &Self::Input,
        evidence: &ExternalReadEvidenceSet<Self::Evidence>,
        context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        if !evidence.fact_query_evidence().is_empty() {
            return Err(StateError::Message(
                "EVM network collection carried unexpected fact-query evidence".to_owned(),
            ));
        }
        let plan = self.plan(input, context)?;
        reduce_evm_network_collection(&plan, evidence.primary_evidence()).map_err(StateError::from)
    }
}

/// Reduces live or replay evidence through the one deterministic collection contract.
pub fn reduce_evm_network_collection(
    plan: &CollectEvmNetworkPlan,
    evidence: &CollectEvmNetworkEvidence,
) -> Result<EvmCollectionBatch, PortfolioEvmError> {
    validate_evm_network_collection_config(&plan.config()).map_err(invalid)?;
    evidence.session.validate(plan)?;
    if evidence.anchor != evidence.final_canonical_block {
        return Err(invalid(
            "EVM collection anchor was no longer canonical after observations",
        ));
    }
    parse_hash(&evidence.anchor.block_hash)?;

    let expected_contracts = plan.token_contracts();
    if evidence.token_decimals.len() != expected_contracts.len() {
        return Err(invalid("EVM token metadata coverage was not exact"));
    }
    let mut decimals = BTreeMap::new();
    for (expected, observed) in expected_contracts.iter().zip(&evidence.token_decimals) {
        EvmBalanceAsset::erc20(observed.contract_address.clone())?;
        if &observed.contract_address != expected
            || decimals
                .insert(observed.contract_address.as_str(), observed.decimals)
                .is_some()
        {
            return Err(invalid(
                "EVM token metadata was not sorted, unique, and exact",
            ));
        }
    }

    if evidence.balances.len() != plan.sources.len() {
        return Err(invalid("EVM balance evidence coverage was not exact"));
    }
    let mut balances = Vec::with_capacity(plan.sources.len());
    for (expected, observed) in plan.sources.iter().zip(&evidence.balances) {
        if &observed.source != expected {
            return Err(invalid(
                "EVM balance evidence was not in exact source order",
            ));
        }
        parse_quantity(&observed.raw_units)?;
        let decimals = match observed.source.asset() {
            EvmBalanceAsset::Native => plan.native_decimals,
            EvmBalanceAsset::Erc20 { contract_address } => *decimals
                .get(contract_address.as_str())
                .ok_or_else(|| invalid("ERC-20 balance lacked token metadata"))?,
        };
        balances.push(EvmCollectedBalance {
            source: observed.source.clone(),
            raw_units: observed.raw_units.clone(),
            decimals,
        });
    }

    let batch = EvmCollectionBatch {
        network_id: plan.network_id.clone(),
        chain_id: plan.chain_id,
        anchor: evidence.anchor.clone(),
        balances,
    };
    batch.validate_against(&plan.config())?;
    Ok(batch)
}

/// Input for the atomic EVM fact-publication state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.portfolio.input.publish_evm_holdings")]
pub struct PublishEvmHoldingsInput {
    /// Complete reduced collection batch.
    pub batch: EvmCollectionBatch,
}

/// Direct typed output consumed by portfolio snapshot assembly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "evm_network_snapshot",
    schema = "mfm.portfolio.evm.network_snapshot"
)]
pub struct EvmNetworkSnapshot {
    network_id: String,
    chain_id: u64,
    anchor: EvmCollectionAnchor,
    balances: Vec<EvmCollectedBalance>,
}

impl EvmNetworkSnapshot {
    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the checked chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the common exact canonical anchor.
    pub const fn anchor(&self) -> &EvmCollectionAnchor {
        &self.anchor
    }

    /// Returns every direct collected balance in source order.
    pub fn balances(&self) -> &[EvmCollectedBalance] {
        &self.balances
    }

    /// Revalidates this output against the exact certified network demand.
    pub fn validate_against(
        &self,
        config: &EvmNetworkCollectionConfig,
    ) -> Result<(), PortfolioEvmError> {
        EvmCollectionBatch {
            network_id: self.network_id.clone(),
            chain_id: self.chain_id,
            anchor: self.anchor.clone(),
            balances: self.balances.clone(),
        }
        .validate_against(config)
    }

    /// Returns the unified facts published atomically with this output.
    pub fn facts(&self) -> Vec<EvmBalanceSnapshotFact> {
        self.balances
            .iter()
            .map(|balance| {
                EvmBalanceSnapshotFact::new(
                    EvmBalanceSnapshotSubject::from_source(
                        &self.network_id,
                        self.chain_id,
                        &balance.source,
                    ),
                    EvmBalanceSnapshotResponse {
                        block_number: self.anchor.block_number,
                        block_hash: self.anchor.block_hash.clone(),
                        raw_units: balance.raw_units.clone(),
                        decimals: balance.decimals,
                    },
                )
            })
            .collect()
    }
}

/// State that atomically records a complete EVM fact batch and returns its direct snapshot.
pub struct PublishEvmHoldingsState {
    config: EvmNetworkCollectionConfig,
}

impl StateSpec for PublishEvmHoldingsState {
    type Config = EvmNetworkCollectionConfig;
    type Context = NoContext;
    type Input = PublishEvmHoldingsInput;
    type Output = EvmNetworkSnapshot;
    type Effect = mfm_effects::ManagedPlatformWrite;
    type Caps = (FactRecordCapability,);

    fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
        state_kind("publish_evm_holdings")
    }

    fn version() -> mfm_program::Result<mfm_ids::StateVersion> {
        state_version("publish_evm_holdings")
    }

    fn name() -> &'static str {
        "mfm.portfolio.publish_evm_holdings"
    }

    fn adapter_bindings() -> mfm_program::Result<Vec<mfm_program::AdapterBindingSpec>> {
        adapter_binding()
    }

    fn emitted_fact_descriptors() -> mfm_program::Result<Vec<FactDescriptorRef>> {
        Ok(vec![fact_descriptor_ref::<EvmBalanceSnapshotFact>()?])
    }

    fn new(config: ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl ManagedWriteState for PublishEvmHoldingsState {
    type RunFuture<'a> = future::Ready<StateResult<Self::Output>>;

    fn run<'a>(
        &'a self,
        input: Self::Input,
        _caps: &'a Self::Caps,
        _context: &'a mfm_program::CertifiedContext<Self::Context>,
    ) -> Self::RunFuture<'a> {
        let result = publish_evm_holdings(&self.config, input.batch).map_err(StateError::from);
        future::ready(result)
    }
}

/// Validates a complete batch and projects the direct network snapshot.
pub fn publish_evm_holdings(
    config: &EvmNetworkCollectionConfig,
    batch: EvmCollectionBatch,
) -> Result<EvmNetworkSnapshot, PortfolioEvmError> {
    batch.validate_against(config)?;
    Ok(EvmNetworkSnapshot {
        network_id: batch.network_id,
        chain_id: batch.chain_id,
        anchor: batch.anchor,
        balances: batch.balances,
    })
}

/// Subject identity for the unified portfolio-owned EVM balance fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "evm_balance_snapshot_subject",
    schema = "mfm.portfolio.fact.evm_balance_snapshot.subject"
)]
pub struct EvmBalanceSnapshotSubject {
    network: String,
    chain_id: u64,
    account: String,
    asset: String,
}

impl EvmBalanceSnapshotSubject {
    fn from_source(network: &str, chain_id: u64, source: &EvmBalanceSource) -> Self {
        let asset = match source.asset() {
            EvmBalanceAsset::Native => "native".to_owned(),
            EvmBalanceAsset::Erc20 { contract_address } => {
                format!("erc20:{contract_address}")
            }
        };
        Self {
            network: network.to_owned(),
            chain_id,
            account: source.account.clone(),
            asset,
        }
    }
}

/// Response material for the unified portfolio-owned EVM balance fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "evm_balance_snapshot_response",
    schema = "mfm.portfolio.fact.evm_balance_snapshot.response"
)]
pub struct EvmBalanceSnapshotResponse {
    block_number: u64,
    block_hash: String,
    raw_units: String,
    decimals: u8,
}

/// Unified native/ERC-20 EVM balance fact recorded by the portfolio vertical slice.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, DeriveMfmFactType)]
#[allow(clippy::duplicated_attributes)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "evm_balance_snapshot_fact",
    schema = "mfm.portfolio.fact.evm_balance_snapshot"
)]
#[mfm_fact(kind = "portfolio.evm_balance_snapshot")]
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
    id = "subject.asset",
    source = "subject",
    path = "asset",
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
pub struct EvmBalanceSnapshotFact {
    subject: EvmBalanceSnapshotSubject,
    response: EvmBalanceSnapshotResponse,
}

impl EvmBalanceSnapshotFact {
    /// Creates a fact from checked collection material.
    pub const fn new(
        subject: EvmBalanceSnapshotSubject,
        response: EvmBalanceSnapshotResponse,
    ) -> Self {
        Self { subject, response }
    }
}

/// Returns Platform visibility for unified EVM balance facts.
pub fn evm_balance_fact_visibility() -> FactVisibility {
    FactVisibility::indexed_default(FactAudience::Platform)
}

fn network_binding(
    network_id: &str,
    chain_id: u64,
) -> Result<EvmNetworkBinding, PortfolioEvmError> {
    let network_id = LocalPublicId::new(network_id)
        .map_err(|_| invalid("EVM collection network id was invalid"))?;
    EvmNetworkBinding::new(network_id, chain_id)
        .map_err(|_| invalid("EVM collection network binding was invalid"))
}

fn parse_address(value: &str) -> Result<Address, PortfolioEvmError> {
    let address = Address::from_str(value).map_err(|_| invalid("EVM address was invalid"))?;
    if format!("{address:#x}") != value {
        return Err(invalid("EVM address was not canonical"));
    }
    Ok(address)
}

fn parse_hash(value: &str) -> Result<B256, PortfolioEvmError> {
    let hash = B256::from_str(value).map_err(|_| invalid("EVM block hash was invalid"))?;
    if format!("{hash:#x}") != value {
        return Err(invalid("EVM block hash was not canonical"));
    }
    Ok(hash)
}

fn parse_quantity(value: &str) -> Result<U256, PortfolioEvmError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid("EVM balance was not canonical decimal"));
    }
    let quantity = U256::from_str(value).map_err(|_| invalid("EVM balance exceeded U256"))?;
    if quantity.to_string() != value {
        return Err(invalid("EVM balance was not canonical decimal"));
    }
    Ok(quantity)
}
