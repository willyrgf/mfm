//! Reusable EVM balance collection state contracts.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use alloy_primitives::{Address, U256};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_evm_capabilities::{
    EvmBlockAnchor, EvmNetworkBinding, EvmReadCapability, EvmSessionEvidence,
    EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_facts::{FactContentIdentityEvidence, MfmFactType};
use mfm_ids::LocalPublicId;
use mfm_program::{
    ExternalReadEvidenceSet, NoContext, NonEmpty, ReadState, StateError, StateResult, StateSpec,
    ValidatedConfig,
};
use mfm_program_derive::{MfmConfig, MfmFactType as DeriveMfmFactType, MfmValue};
use mfm_values::ConfigError;
use serde::{Deserialize, Serialize};

use crate::canonical::{canonical_address, parse_address};
use crate::identity::{adapter_binding, state_kind, state_version};

/// Maximum source demand admitted by one EVM balance collection.
pub const EVM_BALANCE_COLLECTION_SOURCE_LIMIT: usize = 1_024;

/// Redaction-safe EVM balance-collection state failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvmBalanceCollectionError {
    /// Certified config, state input, or retained evidence was invalid.
    #[error("EVM balance collection material was invalid: {reason}")]
    Invalid {
        /// Stable redaction-safe reason.
        reason: String,
    },
}

impl From<EvmBalanceCollectionError> for StateError {
    fn from(error: EvmBalanceCollectionError) -> Self {
        Self::Message(error.to_string())
    }
}

fn invalid(reason: impl Into<String>) -> EvmBalanceCollectionError {
    EvmBalanceCollectionError::Invalid {
        reason: reason.into(),
    }
}

/// Asset identity for one reusable EVM balance source.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash, MfmValue)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance_asset",
    schema = "mfm.evm.balance_asset"
)]
pub enum EvmBalanceAsset {
    /// The native balance for the configured account and network.
    Native,
    /// An ERC-20 balance for the configured account and network.
    Erc20 {
        /// Canonical non-zero ERC-20 contract address.
        contract_address: String,
    },
}

impl EvmBalanceAsset {
    /// Creates a checked ERC-20 balance asset.
    pub fn erc20(contract_address: Address) -> Result<Self, EvmBalanceCollectionError> {
        if contract_address.is_zero() {
            return Err(invalid("ERC-20 contract address must be non-zero"));
        }
        Ok(Self::Erc20 {
            contract_address: canonical_address(contract_address),
        })
    }

    /// Returns the token contract for an ERC-20 asset.
    pub fn contract_address(&self) -> Option<&str> {
        match self {
            Self::Native => None,
            Self::Erc20 { contract_address } => Some(contract_address.as_str()),
        }
    }

    fn validate(&self) -> Result<(), EvmBalanceCollectionError> {
        if let Some(contract_address) = self.contract_address() {
            let contract_address = parse_address(contract_address)
                .map_err(|_| invalid("EVM contract address was invalid"))?;
            if contract_address.is_zero() {
                return Err(invalid("ERC-20 contract address must be non-zero"));
            }
        }
        Ok(())
    }
}

struct PresentOptional<T> {
    value: Option<T>,
    is_present: bool,
}

impl<T> Default for PresentOptional<T> {
    fn default() -> Self {
        Self {
            value: None,
            is_present: false,
        }
    }
}

impl<'de, T> Deserialize<'de> for PresentOptional<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self {
            value: Option::<T>::deserialize(deserializer)?,
            is_present: true,
        })
    }
}

impl<'de> Deserialize<'de> for EvmBalanceAsset {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            kind: String,
            #[serde(default)]
            contract_address: PresentOptional<String>,
        }

        let wire = Wire::deserialize(deserializer)?;
        match wire.kind.as_str() {
            "native" => {
                if wire.contract_address.is_present {
                    return Err(serde::de::Error::custom(
                        "native EVM balance assets do not accept contract_address",
                    ));
                }
                Ok(Self::Native)
            }
            "erc20" => {
                let contract_address = wire.contract_address.value.ok_or_else(|| {
                    serde::de::Error::custom("erc20 EVM balance assets require contract_address")
                })?;
                let contract_address =
                    parse_address(&contract_address).map_err(serde::de::Error::custom)?;
                Self::erc20(contract_address).map_err(serde::de::Error::custom)
            }
            _ => Err(serde::de::Error::custom(
                "EVM balance asset kind must be native or erc20",
            )),
        }
    }
}

/// One unique account/asset balance source collected on an EVM network.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "evm_balance_source",
    schema = "mfm.evm.balance_source"
)]
pub struct EvmBalanceSource {
    account: String,
    asset: EvmBalanceAsset,
}

impl EvmBalanceSource {
    /// Creates one checked balance source.
    pub fn new(
        account: Address,
        asset: EvmBalanceAsset,
    ) -> Result<Self, EvmBalanceCollectionError> {
        let source = Self {
            account: canonical_address(account),
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
    pub fn account_address(&self) -> Result<Address, EvmBalanceCollectionError> {
        parse_address(&self.account).map_err(|_| invalid("EVM account address was invalid"))
    }

    /// Returns the parsed token contract address when this is an ERC-20 source.
    pub fn contract_address_value(&self) -> Result<Option<Address>, EvmBalanceCollectionError> {
        self.asset
            .contract_address()
            .map(parse_address)
            .transpose()
            .map_err(|_| invalid("EVM contract address was invalid"))
    }

    fn validate(&self) -> Result<(), EvmBalanceCollectionError> {
        parse_address(&self.account).map_err(|_| invalid("EVM account address was invalid"))?;
        self.asset.validate()
    }
}

/// Certified demand for one reusable EVM network balance collection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    schema = "mfm.evm.config.balance_collection",
    validate = "validate_evm_balance_collection_config"
)]
pub struct EvmBalanceCollectionConfig {
    network_id: String,
    chain_id: u64,
    native_decimals: u8,
    sources: Vec<EvmBalanceSource>,
}

impl EvmBalanceCollectionConfig {
    /// Creates a checked, sorted collection config for one EVM network.
    pub fn new(
        network_id: impl Into<String>,
        chain_id: u64,
        native_decimals: u8,
        mut sources: Vec<EvmBalanceSource>,
    ) -> Result<Self, ConfigError> {
        sources.sort();
        let config = Self {
            network_id: network_id.into(),
            chain_id,
            native_decimals,
            sources,
        };
        validate_evm_balance_collection_config(&config).map_err(ConfigError::new)?;
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
    pub fn binding(&self) -> Result<EvmNetworkBinding, EvmBalanceCollectionError> {
        network_binding(&self.network_id, self.chain_id)
    }
}

/// Validates bounded, sorted, unique EVM network collection demand.
pub fn validate_evm_balance_collection_config(
    config: &EvmBalanceCollectionConfig,
) -> Result<(), String> {
    LocalPublicId::new(&config.network_id).map_err(|error| error.to_string())?;
    if config.chain_id == 0 {
        return Err("EVM collection chain id must be non-zero".to_owned());
    }
    if config.sources.is_empty() || config.sources.len() > EVM_BALANCE_COLLECTION_SOURCE_LIMIT {
        return Err(format!(
            "EVM collection sources must contain between 1 and {EVM_BALANCE_COLLECTION_SOURCE_LIMIT} entries"
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

/// Deterministic request plan for one EVM network collection attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance_collection_plan",
    schema = "mfm.evm.external_read.balance_collection.plan"
)]
pub struct EvmBalanceCollectionPlan {
    network_id: String,
    chain_id: u64,
    native_decimals: u8,
    sources: Vec<EvmBalanceSource>,
}

impl EvmBalanceCollectionPlan {
    /// Returns the checked source-stable session binding.
    pub fn binding(&self) -> Result<EvmNetworkBinding, EvmBalanceCollectionError> {
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
            .filter_map(|source| source.asset.contract_address().map(str::to_owned))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn config(&self) -> EvmBalanceCollectionConfig {
        EvmBalanceCollectionConfig {
            network_id: self.network_id.clone(),
            chain_id: self.chain_id,
            native_decimals: self.native_decimals,
            sources: self.sources.clone(),
        }
    }
}

/// One exact token-decimals result retained once per distinct contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "evm_token_decimals_evidence",
    schema = "mfm.evm.token_decimals_evidence"
)]
pub struct EvmTokenDecimalsEvidence {
    contract_address: String,
    decimals: u8,
}

impl EvmTokenDecimalsEvidence {
    /// Creates checked token metadata evidence.
    pub fn new(contract_address: Address, decimals: u8) -> Result<Self, EvmBalanceCollectionError> {
        if contract_address.is_zero() {
            return Err(invalid("ERC-20 contract address must be non-zero"));
        }
        Ok(Self {
            contract_address: canonical_address(contract_address),
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
    namespace = "mfm.evm",
    name = "evm_balance_read_evidence",
    schema = "mfm.evm.balance_read_evidence"
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
    namespace = "mfm.evm",
    name = "balance_collection_evidence",
    schema = "mfm.evm.external_read.balance_collection.evidence"
)]
pub struct EvmBalanceCollectionEvidence {
    session: EvmSessionEvidence,
    anchor: EvmBlockAnchor,
    token_decimals: Vec<EvmTokenDecimalsEvidence>,
    balances: Vec<EvmBalanceReadEvidence>,
    final_canonical_block: EvmBlockAnchor,
}

impl EvmBalanceCollectionEvidence {
    /// Creates canonical evidence from one live checked session attempt.
    pub fn new(
        session: &EvmSessionEvidence,
        anchor: EvmBlockAnchor,
        mut token_decimals: Vec<EvmTokenDecimalsEvidence>,
        mut balances: Vec<EvmBalanceReadEvidence>,
        final_canonical_block: EvmBlockAnchor,
    ) -> Self {
        token_decimals.sort();
        balances.sort();
        Self {
            session: session.clone(),
            anchor,
            token_decimals,
            balances,
            final_canonical_block,
        }
    }
}

/// State that collects every configured EVM holding at one exact canonical anchor.
pub struct CollectEvmBalancesState {
    config: EvmBalanceCollectionConfig,
}

impl CollectEvmBalancesState {
    /// Returns the certified collection config.
    pub const fn config(&self) -> &EvmBalanceCollectionConfig {
        &self.config
    }
}

impl StateSpec for CollectEvmBalancesState {
    type Config = EvmBalanceCollectionConfig;
    type Context = NoContext;
    type Input = ();
    type Output = EvmBalanceCollectionReceipt;
    type Effect = mfm_capabilities::ReadExternal;
    type Caps = (EvmReadCapability,);

    fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
        state_kind("collect_balances")
    }

    fn version() -> mfm_program::Result<mfm_ids::StateVersion> {
        state_version("collect_balances")
    }

    fn name() -> &'static str {
        "mfm.evm.collect_balances"
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

impl ReadState for CollectEvmBalancesState {
    type Plan = EvmBalanceCollectionPlan;
    type Evidence = EvmBalanceCollectionEvidence;
    type Facts = NonEmpty<EvmBalanceSnapshotFact>;

    fn plan(
        &self,
        _input: &Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Plan> {
        validate_evm_balance_collection_config(&self.config).map_err(StateError::Message)?;
        Ok(EvmBalanceCollectionPlan {
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
    ) -> StateResult<(Self::Output, Self::Facts)> {
        if !evidence.fact_query_evidence().is_empty() {
            return Err(StateError::Message(
                "EVM network collection carried unexpected fact-query evidence".to_owned(),
            ));
        }
        let plan = self.plan(input, context)?;
        reduce_evm_balance_collection(&plan, evidence.primary_evidence()).map_err(StateError::from)
    }
}

/// Reduces live or replay evidence through the one deterministic collection contract.
pub fn reduce_evm_balance_collection(
    plan: &EvmBalanceCollectionPlan,
    evidence: &EvmBalanceCollectionEvidence,
) -> Result<
    (
        EvmBalanceCollectionReceipt,
        NonEmpty<EvmBalanceSnapshotFact>,
    ),
    EvmBalanceCollectionError,
> {
    validate_evm_balance_collection_config(&plan.config()).map_err(invalid)?;
    validate_session(&evidence.session, plan)?;
    if evidence.anchor != evidence.final_canonical_block {
        return Err(invalid(
            "EVM collection anchor was no longer canonical after observations",
        ));
    }
    validate_anchor(&evidence.anchor)?;

    let expected_contracts = plan.token_contracts();
    if evidence.token_decimals.len() != expected_contracts.len() {
        return Err(invalid("EVM token metadata coverage was not exact"));
    }
    let mut decimals = BTreeMap::new();
    for (expected, observed) in expected_contracts.iter().zip(&evidence.token_decimals) {
        if &observed.contract_address != expected
            || decimals
                .insert(&observed.contract_address, observed.decimals)
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
    let mut facts = Vec::with_capacity(plan.sources.len());
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
                .get(contract_address)
                .ok_or_else(|| invalid("ERC-20 balance lacked token metadata"))?,
        };
        facts.push(EvmBalanceSnapshotFact::new(
            EvmBalanceSnapshotSubject::from_source(
                &plan.network_id,
                plan.chain_id,
                &observed.source,
            ),
            EvmBalanceSnapshotResponse {
                block_anchor: evidence.anchor.clone(),
                raw_units: observed.raw_units.clone(),
                decimals,
            },
        ));
    }

    let facts = NonEmpty::try_from_vec(facts)
        .map_err(|error| invalid(format!("EVM fact batch was empty: {error}")))?;
    let receipt =
        EvmBalanceCollectionReceipt::from_verified_facts(plan, &evidence.anchor, facts.values())?;
    Ok((receipt, facts))
}

/// Checked receipt for one atomic EVM balance-fact publication.
///
/// The receipt carries only collection authority. Balance material remains in the fact store and
/// must be hydrated and reverified against the corresponding content-identity evidence before use.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance_collection_receipt",
    schema = "mfm.evm.balance_collection_receipt"
)]
pub struct EvmBalanceCollectionReceipt {
    network_id: String,
    chain_id: u64,
    block_anchor: EvmBlockAnchor,
    sources: Vec<EvmBalanceSource>,
    fact_content_identities: Vec<FactContentIdentityEvidence>,
}

impl EvmBalanceCollectionReceipt {
    fn from_verified_facts(
        plan: &EvmBalanceCollectionPlan,
        anchor: &EvmBlockAnchor,
        facts: &[EvmBalanceSnapshotFact],
    ) -> Result<Self, EvmBalanceCollectionError> {
        validate_evm_balance_collection_config(&plan.config()).map_err(invalid)?;
        validate_anchor(anchor)?;
        if facts.len() != plan.sources.len() {
            return Err(invalid(
                "EVM balance fact coverage did not match collection demand",
            ));
        }
        let descriptor =
            EvmBalanceSnapshotFact::descriptor().map_err(|error| invalid(error.to_string()))?;
        let mut sources = Vec::with_capacity(facts.len());
        let mut fact_content_identities = Vec::with_capacity(facts.len());
        for (source, fact) in plan.sources.iter().zip(facts) {
            if fact.subject.network_id != plan.network_id
                || fact.subject.chain_id != plan.chain_id
                || fact.subject.account != source.account
                || fact.subject.asset != source.asset
                || &fact.response.block_anchor != anchor
            {
                return Err(invalid(
                    "EVM balance fact did not match reduced collection evidence",
                ));
            }
            parse_quantity(&fact.response.raw_units)?;
            let identity = mfm_facts::derive_fact_content_identity_from_typed_values(
                &descriptor,
                fact.subject(),
                fact.response(),
            )
            .map_err(|error| invalid(error.to_string()))?;
            sources.push(source.clone());
            fact_content_identities.push(FactContentIdentityEvidence::from_verified(&identity));
        }
        Self::from_evidence(
            plan.network_id.clone(),
            plan.chain_id,
            anchor.clone(),
            sources,
            fact_content_identities,
        )
    }

    fn from_evidence(
        network_id: String,
        chain_id: u64,
        block_anchor: EvmBlockAnchor,
        sources: Vec<EvmBalanceSource>,
        fact_content_identities: Vec<FactContentIdentityEvidence>,
    ) -> Result<Self, EvmBalanceCollectionError> {
        LocalPublicId::new(&network_id)
            .map_err(|_| invalid("EVM balance receipt network id was invalid"))?;
        if chain_id == 0 {
            return Err(invalid("EVM balance receipt chain id must be non-zero"));
        }
        validate_anchor(&block_anchor)?;
        if sources.is_empty() || sources.len() > EVM_BALANCE_COLLECTION_SOURCE_LIMIT {
            return Err(invalid(format!(
                "EVM balance receipt sources must contain between 1 and {EVM_BALANCE_COLLECTION_SOURCE_LIMIT} entries"
            )));
        }
        if sources.len() != fact_content_identities.len() {
            return Err(invalid(
                "EVM balance receipt source and content-identity counts differed",
            ));
        }
        let mut previous = None;
        for source in &sources {
            source.validate()?;
            if previous.is_some_and(|prior: &EvmBalanceSource| prior >= source) {
                return Err(invalid(
                    "EVM balance receipt sources were not strictly sorted and unique",
                ));
            }
            previous = Some(source);
        }
        Ok(Self {
            network_id,
            chain_id,
            block_anchor,
            sources,
            fact_content_identities,
        })
    }

    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the checked chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the common exact canonical block anchor.
    pub const fn block_anchor(&self) -> &EvmBlockAnchor {
        &self.block_anchor
    }

    /// Returns every published source in strict canonical order.
    pub fn sources(&self) -> &[EvmBalanceSource] {
        &self.sources
    }

    /// Returns content-identity evidence in the same order as [`Self::sources`].
    pub fn fact_content_identities(&self) -> &[FactContentIdentityEvidence] {
        &self.fact_content_identities
    }
}

impl<'de> Deserialize<'de> for EvmBalanceCollectionReceipt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            network_id: String,
            chain_id: u64,
            block_anchor: EvmBlockAnchor,
            sources: Vec<EvmBalanceSource>,
            fact_content_identities: Vec<FactContentIdentityEvidence>,
        }

        let wire = Wire::deserialize(deserializer)?;
        Self::from_evidence(
            wire.network_id,
            wire.chain_id,
            wire.block_anchor,
            wire.sources,
            wire.fact_content_identities,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Subject identity for the reusable EVM balance fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance_snapshot_subject",
    schema = "mfm.evm.fact.balance_snapshot.subject"
)]
pub struct EvmBalanceSnapshotSubject {
    network_id: String,
    chain_id: u64,
    account: String,
    asset: EvmBalanceAsset,
}

impl EvmBalanceSnapshotSubject {
    fn from_source(network: &str, chain_id: u64, source: &EvmBalanceSource) -> Self {
        Self {
            network_id: network.to_owned(),
            chain_id,
            account: source.account.clone(),
            asset: source.asset.clone(),
        }
    }

    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the non-zero chain id.
    pub const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    /// Returns the observed account.
    pub fn account(&self) -> &str {
        &self.account
    }

    /// Returns the observed asset.
    pub const fn asset(&self) -> &EvmBalanceAsset {
        &self.asset
    }
}

/// Response material for the reusable EVM balance fact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance_snapshot_response",
    schema = "mfm.evm.fact.balance_snapshot.response"
)]
pub struct EvmBalanceSnapshotResponse {
    block_anchor: EvmBlockAnchor,
    raw_units: String,
    decimals: u8,
}

impl EvmBalanceSnapshotResponse {
    /// Returns the exact canonical block anchor.
    pub const fn block_anchor(&self) -> &EvmBlockAnchor {
        &self.block_anchor
    }

    /// Returns the canonical decimal quantity.
    pub fn raw_units(&self) -> &str {
        &self.raw_units
    }

    /// Returns the exact asset decimal scale.
    pub const fn decimals(&self) -> u8 {
        self.decimals
    }
}

/// Decodes one canonical, closed EVM fact response and revalidates its domain invariants.
pub fn decode_evm_balance_snapshot_response(
    bytes: &[u8],
) -> Result<EvmBalanceSnapshotResponse, EvmBalanceCollectionError> {
    let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|_| invalid("EVM fact response was not canonical JSON"))?;
    let response: EvmBalanceSnapshotResponse = serde_json::from_slice(canonical.as_bytes())
        .map_err(|_| invalid("EVM fact response did not match the closed schema"))?;
    validate_anchor(&response.block_anchor)?;
    parse_quantity(&response.raw_units)?;
    Ok(response)
}

/// Unified native/ERC-20 EVM balance fact recorded by reusable collectors.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue, DeriveMfmFactType)]
#[allow(clippy::duplicated_attributes)]
#[mfm(
    namespace = "mfm.evm",
    name = "balance_snapshot_fact",
    schema = "mfm.evm.fact.balance_snapshot"
)]
#[mfm_fact(kind = "evm.balance_snapshot")]
#[mfm_fact(field(
    id = "subject.network_id",
    source = "subject",
    path = "network_id",
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
    id = "subject.asset.kind",
    source = "subject",
    path = "asset.kind",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "subject.asset.contract_address",
    source = "subject",
    path = "asset.contract_address",
    value_type = "string",
    exposure = "returnable",
    optional
))]
#[mfm_fact(field(
    id = "result.block_anchor.number",
    source = "result",
    path = "block_anchor.number",
    value_type = "string",
    exposure = "returnable"
))]
#[mfm_fact(field(
    id = "result.block_anchor.hash",
    source = "result",
    path = "block_anchor.hash",
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
    name = "metadata.store_commit_order.desc",
    term(
        field = "metadata.store_commit_order",
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

    /// Returns the typed subject material.
    pub const fn subject(&self) -> &EvmBalanceSnapshotSubject {
        &self.subject
    }

    /// Returns the typed response material.
    pub const fn response(&self) -> &EvmBalanceSnapshotResponse {
        &self.response
    }
}

fn network_binding(
    network_id: &str,
    chain_id: u64,
) -> Result<EvmNetworkBinding, EvmBalanceCollectionError> {
    let network_id = LocalPublicId::new(network_id)
        .map_err(|_| invalid("EVM collection network id was invalid"))?;
    EvmNetworkBinding::new(network_id, chain_id)
        .map_err(|_| invalid("EVM collection network binding was invalid"))
}

fn validate_session(
    session: &EvmSessionEvidence,
    plan: &EvmBalanceCollectionPlan,
) -> Result<(), EvmBalanceCollectionError> {
    if session.network_id() != plan.network_id
        || session.chain_id() != plan.chain_id
        || session.implementation_id() != EVM_JSONRPC_SESSION_IMPLEMENTATION_ID
    {
        return Err(invalid(
            "EVM collection session did not match certified network authority",
        ));
    }
    Ok(())
}

fn validate_anchor(anchor: &EvmBlockAnchor) -> Result<(), EvmBalanceCollectionError> {
    anchor
        .validate()
        .map_err(|_| invalid("EVM collection block anchor was invalid"))
}

fn parse_quantity(value: &str) -> Result<U256, EvmBalanceCollectionError> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid("EVM balance was not canonical decimal"));
    }
    let quantity = U256::from_str(value).map_err(|_| invalid("EVM balance exceeded U256"))?;
    if quantity.to_string() != value {
        return Err(invalid("EVM balance was not canonical decimal"));
    }
    Ok(quantity)
}
