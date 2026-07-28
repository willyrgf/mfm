//! Pure same-run portfolio snapshot and report states.
//!
//! The snapshot state consumes typed EVM collection outputs through graph
//! edges. It has no store, fact-query, replay, provider, or capability access.

mod decimal;

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU64;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_evm::{
    EvmBalanceAsset, EvmBalanceCollection, EvmBalanceSource, EvmNetworkBinding,
    EvmRoutingGenerationRef,
};
use mfm_ids::{ContentRef, DigestAlgorithm, SchemaId, SemanticTypeId, StableId};
use mfm_program::{
    boundary_content_ref, FactSet, NoBoundaryValue, NoContext, Settlement, State, StateExecution,
    StateFrame, UnitConfig,
};
use mfm_program_derive::{MfmValue, PublicOutputs, StateInput};
use mfm_values::{ConfigError, PublicOutputDescriptor};
use serde::{Deserialize, Serialize};

use self::decimal::{multiply_decimal_strings, DecimalValue};
use crate::{
    AnchoredHoldingSource, ExecutionAnchor, HoldingSourceConfig, NetworkConfig, NetworkPin,
    Observation, ObservationQuantity, ObservationValue, PortfolioConfig, PortfolioQuoteTotal,
    PortfolioReport, PortfolioSnapshot, PortfolioSnapshotSelector, QuoteCode,
    ValidatedPortfolioConfig, WalletReport, WalletSnapshot,
};

/// Exact version carried by a validated snapshot selection.
pub const VALIDATED_PORTFOLIO_SNAPSHOT_SELECTION_VERSION: &str =
    "mfm.portfolio.validated-snapshot-selection.v1";
/// Exact version carried by a qualified portfolio routing manifest.
pub const PORTFOLIO_ROUTING_MANIFEST_VERSION: &str = "mfm.portfolio.routing-manifest.v1";

/// One exact immutable EVM routing generation selected for a demanded network.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "evm-routing-binding",
    version = "1",
    schema = "mfm.portfolio.evm_routing_binding"
)]
pub struct EvmRoutingBinding {
    network_id: String,
    routing_generation_ref: EvmRoutingGenerationRef,
}

impl EvmRoutingBinding {
    /// Creates one checked semantic-network to exact-generation binding.
    pub fn new(
        network_id: impl Into<String>,
        routing_generation_ref: EvmRoutingGenerationRef,
    ) -> Result<Self, ConfigError> {
        let network_id = network_id.into();
        crate::NetworkId::new(network_id.clone())
            .map_err(|error| ConfigError::new(error.to_string()))?;
        routing_generation_ref
            .to_content_ref()
            .map_err(|error| ConfigError::new(error.to_string()))?;
        Ok(Self {
            network_id,
            routing_generation_ref,
        })
    }

    /// Returns the semantic network id.
    pub fn network_id(&self) -> &str {
        &self.network_id
    }

    /// Returns the exact immutable generation reference.
    pub const fn routing_generation_ref(&self) -> &EvmRoutingGenerationRef {
        &self.routing_generation_ref
    }
}

/// Qualified non-secret routing generations available to one snapshot run.
///
/// Application assembly selects these bindings only from an already qualified
/// deployment. The runtime validator still requires their canonical order and
/// exact agreement with the separately retained portfolio configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "routing-manifest",
    version = "1",
    schema = "mfm.portfolio.routing_manifest"
)]
pub struct PortfolioRoutingManifest {
    version: String,
    evm_routing_bindings: Vec<EvmRoutingBinding>,
}

impl PortfolioRoutingManifest {
    /// Builds a canonical manifest from qualified EVM generations.
    pub fn new(mut evm_routing_bindings: Vec<EvmRoutingBinding>) -> Result<Self, ConfigError> {
        evm_routing_bindings.sort();
        if evm_routing_bindings
            .windows(2)
            .any(|pair| pair[0].network_id == pair[1].network_id)
        {
            return Err(ConfigError::new(
                "portfolio routing manifest contains a duplicate network",
            ));
        }
        Ok(Self {
            version: PORTFOLIO_ROUTING_MANIFEST_VERSION.to_owned(),
            evm_routing_bindings,
        })
    }

    /// Returns the exact manifest contract version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns exact generations in semantic-network order.
    pub fn evm_routing_bindings(&self) -> &[EvmRoutingBinding] {
        &self.evm_routing_bindings
    }
}

/// Complete value-only input to snapshot selection validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, StateInput)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.portfolio.input.snapshot_selection", version = "1")]
pub struct PortfolioSnapshotSelectionInput {
    selector: PortfolioSnapshotSelector,
    portfolio: PortfolioConfig,
    routing_manifest: PortfolioRoutingManifest,
}

impl PortfolioSnapshotSelectionInput {
    /// Binds the public selector to its exact configured value and routes.
    pub fn new(
        selector: PortfolioSnapshotSelector,
        portfolio: PortfolioConfig,
        routing_manifest: PortfolioRoutingManifest,
    ) -> Self {
        Self {
            selector,
            portfolio,
            routing_manifest,
        }
    }

    /// Returns the public configured-value selector.
    pub const fn selector(&self) -> &PortfolioSnapshotSelector {
        &self.selector
    }

    /// Returns the exact separately retained portfolio configuration.
    pub const fn portfolio(&self) -> &PortfolioConfig {
        &self.portfolio
    }

    /// Returns the exact qualified non-secret routing manifest.
    pub const fn routing_manifest(&self) -> &PortfolioRoutingManifest {
        &self.routing_manifest
    }
}

/// One semantically validated EVM collection position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "validated-evm-collection-position",
    version = "1",
    schema = "mfm.portfolio.validated_evm_collection_position"
)]
pub struct ValidatedEvmCollectionPosition {
    position_ordinal: u32,
    binding: EvmNetworkBinding,
    native_decimals: u8,
    sources: Vec<EvmBalanceSource>,
    token_contracts: Vec<String>,
}

impl ValidatedEvmCollectionPosition {
    /// Returns the zero-based position in semantic-network order.
    pub const fn position_ordinal(&self) -> u32 {
        self.position_ordinal
    }

    /// Returns the checked semantic network and immutable generation.
    pub const fn binding(&self) -> &EvmNetworkBinding {
        &self.binding
    }

    /// Returns the configured scale of the native asset.
    pub const fn native_decimals(&self) -> u8 {
        self.native_decimals
    }

    /// Returns strict sorted unique account/asset reads.
    pub fn sources(&self) -> &[EvmBalanceSource] {
        &self.sources
    }

    /// Returns strict sorted unique ERC-20 metadata reads.
    pub fn token_contracts(&self) -> &[String] {
        &self.token_contracts
    }

    /// Projects the exact collection config consumed by pure aggregation.
    pub fn collection_config(&self) -> Result<mfm_evm::EvmBalanceCollectionConfig, ConfigError> {
        mfm_evm::EvmBalanceCollectionConfig::new(
            self.binding.clone(),
            self.native_decimals,
            self.sources.clone(),
        )
    }
}

/// Stronger selection authority emitted only by the pure validator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "validated-snapshot-selection",
    version = "1",
    schema = "mfm.portfolio.validated_snapshot_selection"
)]
pub struct ValidatedPortfolioSnapshotSelection {
    version: String,
    target: crate::PortfolioId,
    portfolio: PortfolioConfig,
    positions: Vec<ValidatedEvmCollectionPosition>,
}

impl ValidatedPortfolioSnapshotSelection {
    /// Returns the exact validated-selection contract version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the configured-value target agreed by selector and portfolio.
    pub const fn target(&self) -> &crate::PortfolioId {
        &self.target
    }

    /// Returns the normalized exact portfolio configuration.
    pub const fn portfolio(&self) -> &PortfolioConfig {
        &self.portfolio
    }

    /// Returns non-empty collection positions in strict network order.
    pub fn positions(&self) -> &[ValidatedEvmCollectionPosition] {
        &self.positions
    }
}

/// Redaction-safe typed failure of snapshot selection validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(deny_unknown_fields)]
#[mfm(
    namespace = "mfm.portfolio",
    name = "invalid-snapshot-selection",
    version = "1",
    schema = "mfm.portfolio.invalid_snapshot_selection"
)]
pub struct InvalidPortfolioSnapshotSelection {}

/// Pure semantic validation before every portfolio read graph.
pub struct ValidatePortfolioSnapshotSelectionState;

/// Validated portfolio authority and exact same-run collection outputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, StateInput)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.portfolio.input.snapshot_assembly", version = "1")]
pub struct PortfolioSnapshotAssemblyInput {
    selection: ValidatedPortfolioSnapshotSelection,
    collections: Vec<EvmBalanceCollection>,
}

impl PortfolioSnapshotAssemblyInput {
    /// Creates final assembly input from the validator and every EVM child.
    pub fn new(
        selection: ValidatedPortfolioSnapshotSelection,
        collections: Vec<EvmBalanceCollection>,
    ) -> Self {
        Self {
            selection,
            collections,
        }
    }

    /// Returns the exact successful validator output.
    pub const fn selection(&self) -> &ValidatedPortfolioSnapshotSelection {
        &self.selection
    }

    /// Returns EVM child outputs in certified position order.
    pub fn collections(&self) -> &[EvmBalanceCollection] {
        &self.collections
    }
}

/// Exact snapshot consumed by the pure report projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, StateInput)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.portfolio.input.report_projection", version = "1")]
pub struct PortfolioReportProjectionInput {
    snapshot: PortfolioSnapshot,
}

impl PortfolioReportProjectionInput {
    /// Wraps one same-run portfolio snapshot for report projection.
    pub const fn new(snapshot: PortfolioSnapshot) -> Self {
        Self { snapshot }
    }

    /// Returns the exact same-run snapshot.
    pub const fn snapshot(&self) -> &PortfolioSnapshot {
        &self.snapshot
    }
}

/// Typed semantic failure of a pure portfolio state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MfmValue)]
#[serde(rename_all = "snake_case")]
#[mfm(
    namespace = "mfm.portfolio",
    name = "snapshot-failure",
    version = "1",
    schema = "mfm.portfolio.snapshot_failure"
)]
pub enum PortfolioSnapshotFailure {
    /// Same-run collection values did not exactly realize certified demand.
    InvalidCollection,
    /// The snapshot could not be projected under the canonical decimal rules.
    InvalidSnapshot,
}

/// Value-only public output contract for the sole product entry point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, PublicOutputs)]
#[serde(deny_unknown_fields)]
#[mfm(schema = "mfm.portfolio.public_outputs")]
pub struct PortfolioPublicOutputs {
    /// Canonical same-run portfolio snapshot.
    pub snapshot: PortfolioSnapshot,
    /// Pure report projection of that exact snapshot.
    pub report: PortfolioReport,
}

impl State for ValidatePortfolioSnapshotSelectionState {
    type Config = UnitConfig;
    type Context = NoContext;
    type Input = PortfolioSnapshotSelectionInput;
    type Output = ValidatedPortfolioSnapshotSelection;
    type Failure = InvalidPortfolioSnapshotSelection;
    type Request = NoBoundaryValue;
    type Observation = NoBoundaryValue;
    type AccessFailure = NoBoundaryValue;

    fn state_contract_ref() -> mfm_program::Result<ContentRef> {
        portfolio_state_contract_ref("validate_snapshot_selection")
    }
}

fn validate_snapshot_selection_apply(
    frame: StateFrame<'_, ValidatePortfolioSnapshotSelectionState>,
) -> Settlement<ValidatePortfolioSnapshotSelectionState> {
    match validate_snapshot_selection(frame.input().value()) {
        Ok(selection) => Settlement::succeeded(selection, FactSet::empty()),
        Err(()) => Settlement::failed(InvalidPortfolioSnapshotSelection {}),
    }
}

/// Pure exact-coverage snapshot assembly.
pub struct AssemblePortfolioSnapshotState;

impl State for AssemblePortfolioSnapshotState {
    type Config = UnitConfig;
    type Context = NoContext;
    type Input = PortfolioSnapshotAssemblyInput;
    type Output = PortfolioSnapshot;
    type Failure = PortfolioSnapshotFailure;
    type Request = NoBoundaryValue;
    type Observation = NoBoundaryValue;
    type AccessFailure = NoBoundaryValue;

    fn state_contract_ref() -> mfm_program::Result<ContentRef> {
        portfolio_state_contract_ref("assemble_snapshot")
    }
}

fn assemble_snapshot_apply(
    frame: StateFrame<'_, AssemblePortfolioSnapshotState>,
) -> Settlement<AssemblePortfolioSnapshotState> {
    let input = frame.input().value();
    match assemble_snapshot(&input.selection, &input.collections) {
        Ok(snapshot) => Settlement::succeeded(snapshot, FactSet::empty()),
        Err(()) => Settlement::failed(PortfolioSnapshotFailure::InvalidCollection),
    }
}

/// Pure report projection from the exact same-run snapshot.
pub struct ProjectPortfolioReportState;

impl State for ProjectPortfolioReportState {
    type Config = UnitConfig;
    type Context = NoContext;
    type Input = PortfolioReportProjectionInput;
    type Output = PortfolioReport;
    type Failure = PortfolioSnapshotFailure;
    type Request = NoBoundaryValue;
    type Observation = NoBoundaryValue;
    type AccessFailure = NoBoundaryValue;

    fn state_contract_ref() -> mfm_program::Result<ContentRef> {
        portfolio_state_contract_ref("project_report")
    }
}

pub(crate) const fn validation_execution() -> StateExecution<ValidatePortfolioSnapshotSelectionState>
{
    StateExecution::pure(validate_snapshot_selection_apply)
}

pub(crate) const fn snapshot_assembly_execution() -> StateExecution<AssemblePortfolioSnapshotState>
{
    StateExecution::pure(assemble_snapshot_apply)
}

pub(crate) const fn report_projection_execution() -> StateExecution<ProjectPortfolioReportState> {
    StateExecution::pure(project_report_apply)
}

fn project_report_apply(
    frame: StateFrame<'_, ProjectPortfolioReportState>,
) -> Settlement<ProjectPortfolioReportState> {
    match project_report(frame.input().value().snapshot()) {
        Ok(report) => Settlement::succeeded(report, FactSet::empty()),
        Err(()) => Settlement::failed(PortfolioSnapshotFailure::InvalidSnapshot),
    }
}

/// Returns the public-output schema for the sole snapshot entry point.
pub fn portfolio_snapshot_public_output_schema_id() -> mfm_values::Result<SchemaId> {
    PortfolioPublicOutputs::public_schema_id()
}

fn validate_snapshot_selection(
    input: &PortfolioSnapshotSelectionInput,
) -> Result<ValidatedPortfolioSnapshotSelection, ()> {
    if input.selector.target().as_str() != input.portfolio.portfolio_id.as_str()
        || input.routing_manifest.version != PORTFOLIO_ROUTING_MANIFEST_VERSION
    {
        return Err(());
    }

    let portfolio = ValidatedPortfolioConfig::new(input.portfolio.clone())
        .map_err(|_| ())?
        .into_config();
    if portfolio != input.portfolio {
        return Err(());
    }
    let bindings = &input.routing_manifest.evm_routing_bindings;
    if bindings.is_empty()
        || bindings
            .windows(2)
            .any(|pair| pair[0].network_id.as_str() >= pair[1].network_id.as_str())
        || bindings.iter().any(|binding| {
            crate::NetworkId::new(binding.network_id.clone()).is_err()
                || binding.routing_generation_ref.to_content_ref().is_err()
        })
    {
        return Err(());
    }

    let demanded = demanded_networks(&portfolio).map_err(|_| ())?;
    if demanded.is_empty()
        || demanded
            .values()
            .any(|network| matches!(network, NetworkConfig::Bitcoin { .. }))
        || bindings
            .iter()
            .map(|binding| binding.network_id.as_str())
            .ne(demanded.keys().copied())
    {
        return Err(());
    }

    let collections = compile_evm_collections_from_parts(&portfolio, bindings).map_err(|_| ())?;
    let mut positions = Vec::with_capacity(collections.len());
    for (index, collection) in collections.into_iter().enumerate() {
        if collection.sources().is_empty() {
            return Err(());
        }
        let position_ordinal = u32::try_from(index).map_err(|_| ())?;
        positions.push(ValidatedEvmCollectionPosition {
            position_ordinal,
            binding: collection.binding().clone(),
            native_decimals: collection.native_decimals(),
            sources: collection.sources().to_vec(),
            token_contracts: collection.token_contracts(),
        });
    }
    if positions.is_empty()
        || positions.iter().enumerate().any(|(index, position)| {
            usize::try_from(position.position_ordinal) != Ok(index)
                || position.sources.windows(2).any(|pair| pair[0] >= pair[1])
                || position
                    .token_contracts
                    .windows(2)
                    .any(|pair| pair[0] >= pair[1])
                || position.token_contracts
                    != position
                        .sources
                        .iter()
                        .filter_map(|source| source.asset().contract_address().map(str::to_owned))
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect::<Vec<_>>()
        })
    {
        return Err(());
    }

    Ok(ValidatedPortfolioSnapshotSelection {
        version: VALIDATED_PORTFOLIO_SNAPSHOT_SELECTION_VERSION.to_owned(),
        target: input.selector.target().clone(),
        portfolio,
        positions,
    })
}

fn demanded_networks(
    portfolio: &PortfolioConfig,
) -> Result<BTreeMap<&str, &NetworkConfig>, String> {
    let networks = portfolio
        .networks
        .iter()
        .map(|network| (network.network_id().as_str(), network))
        .collect::<BTreeMap<_, _>>();
    let mut demanded = BTreeMap::new();
    for wallet in &portfolio.wallets {
        if wallet.symbol_ids.is_empty() {
            continue;
        }
        let network = networks
            .get(wallet.network_id.as_str())
            .copied()
            .ok_or_else(|| "portfolio wallet referenced an unknown network".to_owned())?;
        demanded.insert(wallet.network_id.as_str(), network);
    }
    Ok(demanded)
}

pub(crate) fn compile_evm_collections_from_parts(
    portfolio: &PortfolioConfig,
    evm_routing_bindings: &[EvmRoutingBinding],
) -> Result<Vec<mfm_evm::EvmBalanceCollectionConfig>, ConfigError> {
    let symbols = portfolio
        .symbol_configs
        .iter()
        .map(|symbol| (symbol.symbol_id.as_str(), symbol))
        .collect::<BTreeMap<_, _>>();
    let networks = demanded_networks(portfolio).map_err(ConfigError::new)?;
    let generations = evm_routing_bindings
        .iter()
        .map(|binding| {
            (
                binding.network_id.as_str(),
                binding.routing_generation_ref.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut sources = BTreeMap::<&str, BTreeSet<EvmBalanceSource>>::new();

    for wallet in &portfolio.wallets {
        if wallet.symbol_ids.is_empty() {
            continue;
        }
        let account = wallet.subject.evm_address().ok_or_else(|| {
            ConfigError::new("demanded EVM wallet did not contain an EVM address")
        })?;
        for symbol_id in &wallet.symbol_ids {
            let symbol = symbols
                .get(symbol_id.as_str())
                .copied()
                .ok_or_else(|| ConfigError::new("portfolio wallet referenced an unknown symbol"))?;
            let asset = match &symbol.source {
                HoldingSourceConfig::Native => EvmBalanceAsset::Native,
                HoldingSourceConfig::Erc20 { contract_address } => EvmBalanceAsset::erc20(
                    contract_address
                        .to_address()
                        .map_err(|error| ConfigError::new(error.to_string()))?,
                )
                .map_err(|error| ConfigError::new(error.to_string()))?,
            };
            sources
                .entry(wallet.network_id.as_str())
                .or_default()
                .insert(
                    EvmBalanceSource::new(
                        account
                            .to_address()
                            .map_err(|error| ConfigError::new(error.to_string()))?,
                        asset,
                    )
                    .map_err(|error| ConfigError::new(error.to_string()))?,
                );
        }
    }

    networks
        .into_iter()
        .map(|(network_id, network)| {
            let NetworkConfig::Evm {
                chain_id,
                native_decimals,
                ..
            } = network
            else {
                return Err(ConfigError::new("Bitcoin holding demand is not executable"));
            };
            let generation = generations
                .get(network_id)
                .ok_or_else(|| ConfigError::new("demanded EVM routing generation was missing"))?;
            let binding = EvmNetworkBinding::new(network_id, chain_id.get(), generation.clone())
                .map_err(|error| ConfigError::new(error.to_string()))?;
            mfm_evm::EvmBalanceCollectionConfig::new(
                binding,
                *native_decimals,
                sources
                    .remove(network_id)
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
            )
        })
        .collect()
}

fn assemble_snapshot(
    selection: &ValidatedPortfolioSnapshotSelection,
    collections: &[EvmBalanceCollection],
) -> Result<PortfolioSnapshot, ()> {
    if selection.version != VALIDATED_PORTFOLIO_SNAPSHOT_SELECTION_VERSION
        || selection.target.as_str() != selection.portfolio.portfolio_id.as_str()
        || selection.positions.is_empty()
        || selection
            .positions
            .iter()
            .enumerate()
            .any(|(index, position)| {
                usize::try_from(position.position_ordinal) != Ok(index)
                    || (index > 0
                        && selection.positions[index - 1].binding.network_id()
                            >= position.binding.network_id())
            })
    {
        return Err(());
    }
    let expected_configs = selection
        .positions
        .iter()
        .map(ValidatedEvmCollectionPosition::collection_config)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ())?;
    if collections.len() != expected_configs.len() {
        return Err(());
    }

    let mut collections_by_network = BTreeMap::new();
    for collection in collections {
        let network_id = collection.source().source().binding().network_id();
        if collections_by_network
            .insert(network_id, collection)
            .is_some()
        {
            return Err(());
        }
    }

    let symbols = selection
        .portfolio
        .symbol_configs
        .iter()
        .map(|symbol| (symbol.symbol_id.as_str(), symbol))
        .collect::<BTreeMap<_, _>>();
    let mut observations_by_wallet = BTreeMap::<String, Vec<Observation>>::new();
    let mut network_pins = Vec::with_capacity(expected_configs.len());
    let mut observed_requirements = BTreeSet::new();

    for expected in &expected_configs {
        let network_id = expected.binding().network_id();
        let collection = collections_by_network.remove(network_id).ok_or(())?;
        if collection.source().source().binding() != expected.binding()
            || collection.balances().len() != expected.sources().len()
        {
            return Err(());
        }
        let chain_id = NonZeroU64::new(expected.binding().chain_id()).ok_or(())?;
        let anchor = ExecutionAnchor::Evm {
            chain_id,
            block: collection.source().anchor().clone(),
        };
        network_pins.push(NetworkPin {
            network_id: network_id.to_owned(),
            anchor: anchor.clone(),
        });

        let balances = collection
            .balances()
            .iter()
            .map(|balance| (balance.source(), balance))
            .collect::<BTreeMap<_, _>>();
        if balances.len() != expected.sources().len()
            || !expected
                .sources()
                .iter()
                .all(|source| balances.contains_key(source))
        {
            return Err(());
        }

        for wallet in selection
            .portfolio
            .wallets
            .iter()
            .filter(|wallet| wallet.network_id.as_str() == network_id)
        {
            let account = wallet
                .subject
                .evm_address()
                .ok_or(())?
                .to_address()
                .map_err(|_| ())?;
            for symbol_id in &wallet.symbol_ids {
                let symbol = symbols.get(symbol_id.as_str()).copied().ok_or(())?;
                let asset = source_asset(&symbol.source)?;
                let source = EvmBalanceSource::new(account, asset).map_err(|_| ())?;
                let balance = balances.get(&source).copied().ok_or(())?;
                if !observed_requirements
                    .insert((wallet.wallet_id.as_str(), symbol.symbol_id.as_str()))
                {
                    return Err(());
                }
                let amount_dec = amount_dec_from_raw(balance.raw_units(), balance.decimals())?;
                let mut values = symbol
                    .valuation
                    .quotes
                    .iter()
                    .map(|quote| {
                        Ok(ObservationValue {
                            quote: quote.quote,
                            priced_symbol_id: quote.priced_symbol_id.to_string(),
                            value_dec: multiply_decimal_strings(
                                &amount_dec,
                                quote.unit_price_dec.as_str(),
                            )
                            .map_err(|_| ())?,
                            unit_price_dec: quote.unit_price_dec.to_string(),
                        })
                    })
                    .collect::<Result<Vec<_>, ()>>()?;
                values.sort_by_key(|value| value.quote);
                observations_by_wallet
                    .entry(wallet.wallet_id.to_string())
                    .or_default()
                    .push(Observation {
                        wallet_id: wallet.wallet_id.to_string(),
                        symbol_id: symbol.symbol_id.to_string(),
                        display_symbol: symbol.display_symbol.clone(),
                        network_id: network_id.to_owned(),
                        quantity: ObservationQuantity {
                            raw_dec: balance.raw_units().to_owned(),
                            decimals: balance.decimals(),
                            amount_dec,
                        },
                        values,
                        source: AnchoredHoldingSource {
                            holding: symbol.source.clone(),
                            anchor: anchor.clone(),
                        },
                        metadata: symbol.metadata.clone(),
                    });
            }
        }
    }
    if !collections_by_network.is_empty()
        || observed_requirements.len()
            != selection
                .portfolio
                .wallets
                .iter()
                .map(|wallet| wallet.symbol_ids.len())
                .sum::<usize>()
    {
        return Err(());
    }

    let mut wallets = selection
        .portfolio
        .wallets
        .iter()
        .map(|wallet| {
            let mut wallet_snapshot = WalletSnapshot {
                wallet_id: wallet.wallet_id.to_string(),
                subject: wallet.subject.clone(),
                network_id: wallet.network_id.to_string(),
                observations: observations_by_wallet
                    .remove(wallet.wallet_id.as_str())
                    .unwrap_or_default(),
            };
            wallet_snapshot.normalize();
            wallet_snapshot
        })
        .collect::<Vec<_>>();
    wallets.sort_by(|left, right| left.wallet_id.cmp(&right.wallet_id));
    network_pins.sort_by(|left, right| left.network_id.cmp(&right.network_id));

    let mut snapshot = PortfolioSnapshot {
        schema_version: PortfolioSnapshot::SCHEMA_VERSION,
        portfolio_id: selection.portfolio.portfolio_id.to_string(),
        network_pins,
        wallets,
        symbol_configs: selection.portfolio.symbol_configs.clone(),
    };
    snapshot.normalize();
    Ok(snapshot)
}

fn source_asset(holding: &HoldingSourceConfig) -> Result<EvmBalanceAsset, ()> {
    match holding {
        HoldingSourceConfig::Native => Ok(EvmBalanceAsset::Native),
        HoldingSourceConfig::Erc20 { contract_address } => {
            EvmBalanceAsset::erc20(contract_address.to_address().map_err(|_| ())?).map_err(|_| ())
        }
    }
}

fn amount_dec_from_raw(raw: &str, decimals: u8) -> Result<String, ()> {
    if raw.is_empty()
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
        || (raw.len() > 1 && raw.starts_with('0'))
    {
        return Err(());
    }
    let scale = usize::from(decimals);
    if scale == 0 {
        return Ok(raw.to_owned());
    }
    if raw.len() <= scale {
        Ok(format!("0.{}{}", "0".repeat(scale - raw.len()), raw))
    } else {
        let split = raw.len() - scale;
        Ok(format!("{}.{}", &raw[..split], &raw[split..]))
    }
}

fn project_report(snapshot: &PortfolioSnapshot) -> Result<PortfolioReport, ()> {
    if snapshot.schema_version != PortfolioSnapshot::SCHEMA_VERSION {
        return Err(());
    }
    let mut portfolio_totals = initialized_quote_totals(snapshot);
    let mut wallet_summaries = Vec::with_capacity(snapshot.wallets.len());
    for wallet in &snapshot.wallets {
        let mut wallet_totals = initialized_quote_totals(snapshot);
        for observation in &wallet.observations {
            for value in &observation.values {
                let amount = DecimalValue::parse_non_negative(&value.value_dec).map_err(|_| ())?;
                wallet_totals
                    .entry(value.quote)
                    .or_default()
                    .add_assign(&amount);
                portfolio_totals
                    .entry(value.quote)
                    .or_default()
                    .add_assign(&amount);
            }
        }
        wallet_summaries.push(WalletReport {
            wallet_id: wallet.wallet_id.clone(),
            network_id: wallet.network_id.clone(),
            totals_by_quote: quote_totals(wallet_totals),
        });
    }
    let mut report = PortfolioReport {
        schema_version: PortfolioReport::SCHEMA_VERSION,
        portfolio_id: snapshot.portfolio_id.clone(),
        network_pins: snapshot.network_pins.clone(),
        wallet_summaries,
        totals_by_quote: quote_totals(portfolio_totals),
    };
    report.normalize();
    Ok(report)
}

fn initialized_quote_totals(snapshot: &PortfolioSnapshot) -> BTreeMap<QuoteCode, DecimalValue> {
    snapshot
        .symbol_configs
        .iter()
        .flat_map(|symbol| {
            symbol
                .valuation
                .quotes
                .iter()
                .map(|valuation| valuation.quote)
        })
        .map(|quote| (quote, DecimalValue::default()))
        .collect()
}

fn quote_totals(totals: BTreeMap<QuoteCode, DecimalValue>) -> Vec<PortfolioQuoteTotal> {
    totals
        .into_iter()
        .map(|(quote, value)| PortfolioQuoteTotal {
            quote,
            total_value_dec: value.to_canonical_string(),
        })
        .collect()
}

pub(crate) fn portfolio_state_contract_canonical(
    name: &'static str,
) -> mfm_program::Result<PlainCanonicalJsonBytes> {
    PlainCanonicalJsonBytes::from_json_str(&format!(
        r#"{{"name":"{name}","version":"mfm.portfolio.state.{name}.v1"}}"#
    ))
    .map_err(|error| mfm_program::ProgramError::Codec(error.to_string()))
}

fn portfolio_state_contract_ref(name: &'static str) -> mfm_program::Result<ContentRef> {
    boundary_content_ref(
        portfolio_state_contract_schema_id()?,
        &portfolio_state_contract_canonical(name)?,
    )
}

pub(crate) fn portfolio_state_contract_schema_id() -> mfm_program::Result<SchemaId> {
    SchemaId::new(
        "mfm.portfolio.state-contract",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.portfolio.state-contract@1"),
    )
    .map_err(|error| mfm_program::ProgramError::Spec(error.to_string()))
}

pub(crate) fn portfolio_state_contract_support_contract(
    role: StableId,
    evidence_contract_ref: ContentRef,
) -> mfm_program::Result<mfm_values::RetainedValueContract> {
    let semantic_type_id = SemanticTypeId::new(
        "mfm.portfolio",
        "state-contract",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"semantic:mfm.portfolio:state-contract:1"),
    )
    .map_err(|error| mfm_program::ProgramError::Spec(error.to_string()))?;
    mfm_values::RetainedValueContract::new(
        portfolio_state_contract_schema_id()?,
        semantic_type_id,
        role,
        "application/json",
        evidence_contract_ref,
    )
    .map_err(|error| mfm_program::ProgramError::Codec(error.to_string()))
}

pub(crate) fn portfolio_contract_schema_id(name: &'static str) -> mfm_program::Result<SchemaId> {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("schema:{name}:1").as_bytes()),
    )
    .map_err(|error| mfm_program::ProgramError::Spec(error.to_string()))
}

pub(crate) fn portfolio_support_contract(
    schema_name: &'static str,
    semantic_name: &'static str,
    role: StableId,
    evidence_contract_ref: ContentRef,
) -> mfm_program::Result<mfm_values::RetainedValueContract> {
    let semantic_type_id = SemanticTypeId::new(
        "mfm.portfolio",
        semantic_name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("semantic:mfm.portfolio:{semantic_name}:1").as_bytes()),
    )
    .map_err(|error| mfm_program::ProgramError::Spec(error.to_string()))?;
    mfm_values::RetainedValueContract::new(
        portfolio_contract_schema_id(schema_name)?,
        semantic_type_id,
        role,
        "application/json",
        evidence_contract_ref,
    )
    .map_err(|error| mfm_program::ProgramError::Codec(error.to_string()))
}

#[cfg(test)]
mod tests;
