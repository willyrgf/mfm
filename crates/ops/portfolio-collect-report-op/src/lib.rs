#![warn(missing_docs)]
//! Deterministic internal portfolio collection/report composition.
//!
//! This operation derives family collection work from normalized portfolio demand. It does not
//! accept observation context, collector read policy, or independently authored resource vectors.

use std::collections::{BTreeMap, BTreeSet};

use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_op_btc_collectors::{
    BtcNativeBalancesAtAnchorConfig, BtcNetworkCollectionConfig, BtcNetworkCollectionOperation,
    BtcNetworkCollectionReceipt,
};
use mfm_op_evm_collectors::{
    EvmErc20BalanceSourceConfig, EvmNetworkCollectionConfig, EvmNetworkCollectionOperation,
    EvmNetworkCollectionReceipt,
};
use mfm_op_portfolio_tracker::{
    PortfolioInputsReady, PortfolioOperationOutputs, PortfolioTrackerWorkflowOperation,
};
use mfm_portfolio_model::ids::NormalizedEvmAddress;
use mfm_portfolio_model::portfolio::{
    NetworkConfig, NetworkFamilyConfig, PortfolioConfig as ModelPortfolioConfig,
    ValidatedPortfolioConfig,
};
use mfm_portfolio_model::symbol::HoldingSourceConfig;
use mfm_program::{
    BridgeKey, BridgePolicy, NoContext, Operation, OperationExpansion, OperationKey, ScopeKey,
    StateError, StateKey, StateResult, StateSpec,
};
use mfm_program_derive::{MfmConfig, StateInput};
use mfm_state_portfolio::{
    AssembleSnapshotState, PortfolioInputsReadyConfig, PortfolioInputsReadyState,
    ProjectReportState, ResolveSubjectsState, ResolveValuationsState, SelectHoldingsState,
};
use mfm_values::{ConfigError, MfmConfig as MfmConfigTrait};
use serde::{Deserialize, Serialize};

const OP_NAMESPACE: &str = "mfm.portfolio";
const OP_KIND_NAME: &str = "collect_then_report";
const OP_VERSION: &str = "mfm.portfolio.operation.collect_then_report.v1";

/// Complete deterministic config for the internal collection/report composition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    schema = "mfm.portfolio.operation.config.collect_then_report",
    validate = "validate_collect_then_report_config"
)]
pub struct CollectThenReportConfig {
    /// Normalized portfolio consumed by collection and report topology.
    pub portfolio: ModelPortfolioConfig,
    /// Derived, strictly sorted Bitcoin network collection children.
    pub bitcoin_collections: Vec<BtcNetworkCollectionConfig>,
    /// Derived, strictly sorted EVM network collection children.
    pub evm_collections: Vec<EvmNetworkCollectionConfig>,
}

impl CollectThenReportConfig {
    /// Returns the temporary typed receipt fan-in count contract.
    pub fn readiness_config(&self) -> Result<PortfolioInputsReadyConfig, ConfigError> {
        let bitcoin_network_count = u32::try_from(self.bitcoin_collections.len())
            .map_err(|_| ConfigError::new("too many Bitcoin collection networks"))?;
        let evm_network_count = u32::try_from(self.evm_collections.len())
            .map_err(|_| ConfigError::new("too many EVM collection networks"))?;
        Ok(PortfolioInputsReadyConfig::new(
            bitcoin_network_count,
            evm_network_count,
        ))
    }
}

/// Errors returned while deriving a complete composed-operation config.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CollectThenReportConfigError {
    /// The portfolio failed canonical validation.
    #[error("portfolio config is invalid")]
    InvalidPortfolio,
    /// A derived internal collection config failed its own validation.
    #[error("derived collection config is invalid")]
    InvalidCollectionConfig,
}

/// Derives exact family collection children from normalized portfolio holding demand.
pub fn build_collect_then_report_config(
    portfolio: ModelPortfolioConfig,
) -> Result<CollectThenReportConfig, CollectThenReportConfigError> {
    let portfolio = ValidatedPortfolioConfig::new(portfolio)
        .map_err(|_| CollectThenReportConfigError::InvalidPortfolio)?
        .into_config();
    let demand = collection_demand_by_network(&portfolio)?;
    let networks = portfolio
        .networks
        .iter()
        .map(|network| (network.network_id().as_str().to_owned(), network))
        .collect::<BTreeMap<_, _>>();

    let mut bitcoin_collections = Vec::new();
    let mut evm_collections = Vec::new();
    for (network_id, network) in networks {
        let Some(network_demand) = demand.get(&network_id) else {
            continue;
        };
        match network {
            NetworkConfig::Bitcoin {
                bitcoin_network,
                source_identity,
                ..
            } => {
                if !network_demand.evm_native_accounts.is_empty()
                    || !network_demand.erc20_sources.is_empty()
                    || network_demand.bitcoin_addresses.is_empty()
                {
                    return Err(CollectThenReportConfigError::InvalidCollectionConfig);
                }
                bitcoin_collections.push(BtcNetworkCollectionConfig {
                    native_balances: BtcNativeBalancesAtAnchorConfig {
                        network: network_id,
                        bitcoin_network: bitcoin_network.clone(),
                        semantic_source_identity: source_identity.to_string(),
                        addresses: network_demand.bitcoin_addresses.iter().cloned().collect(),
                    },
                });
            }
            NetworkConfig::Evm { .. } => {
                if !network_demand.bitcoin_addresses.is_empty() {
                    return Err(CollectThenReportConfigError::InvalidCollectionConfig);
                }
                evm_collections.push(EvmNetworkCollectionConfig {
                    network: network.clone(),
                    native_accounts: network_demand.evm_native_accounts.iter().cloned().collect(),
                    erc20_sources: network_demand.erc20_sources.iter().cloned().collect(),
                });
            }
        }
    }
    let config = CollectThenReportConfig {
        portfolio,
        bitcoin_collections,
        evm_collections,
    };
    validate_collect_then_report_config(&config)
        .map_err(|_| CollectThenReportConfigError::InvalidCollectionConfig)?;
    Ok(config)
}

#[derive(Default)]
struct NetworkCollectionDemand {
    bitcoin_addresses: BTreeSet<String>,
    evm_native_accounts: BTreeSet<NormalizedEvmAddress>,
    erc20_sources: BTreeSet<EvmErc20BalanceSourceConfig>,
}

fn collection_demand_by_network(
    portfolio: &ModelPortfolioConfig,
) -> Result<BTreeMap<String, NetworkCollectionDemand>, CollectThenReportConfigError> {
    let symbols = portfolio
        .symbol_configs
        .iter()
        .map(|symbol| (symbol.symbol_id.clone(), symbol))
        .collect::<BTreeMap<_, _>>();
    let networks = portfolio
        .networks
        .iter()
        .map(|network| (network.network_id().clone(), network))
        .collect::<BTreeMap<_, _>>();
    let mut demand = BTreeMap::<String, NetworkCollectionDemand>::new();
    for wallet in &portfolio.wallets {
        let network = networks
            .get(&wallet.network_id)
            .ok_or(CollectThenReportConfigError::InvalidPortfolio)?;
        let network_demand = demand.entry(wallet.network_id.to_string()).or_default();
        for symbol_id in &wallet.symbol_ids {
            let symbol = symbols
                .get(symbol_id)
                .ok_or(CollectThenReportConfigError::InvalidPortfolio)?;
            match (&network.family(), &symbol.source) {
                (NetworkFamilyConfig::Bitcoin, HoldingSourceConfig::Native) => {
                    network_demand
                        .bitcoin_addresses
                        .insert(wallet.subject.address_str().to_owned());
                }
                (NetworkFamilyConfig::Evm, HoldingSourceConfig::Native) => {
                    let account = wallet
                        .subject
                        .evm_address()
                        .ok_or(CollectThenReportConfigError::InvalidPortfolio)?;
                    network_demand.evm_native_accounts.insert(account.clone());
                }
                (NetworkFamilyConfig::Evm, HoldingSourceConfig::Erc20 { contract_address }) => {
                    let account = wallet
                        .subject
                        .evm_address()
                        .ok_or(CollectThenReportConfigError::InvalidPortfolio)?;
                    network_demand
                        .erc20_sources
                        .insert(EvmErc20BalanceSourceConfig {
                            contract_address: contract_address.clone(),
                            account: account.clone(),
                        });
                }
                _ => return Err(CollectThenReportConfigError::InvalidPortfolio),
            }
        }
    }
    Ok(demand)
}

fn validate_collect_then_report_config(
    config: &CollectThenReportConfig,
) -> Result<(), ConfigError> {
    let normalized = ValidatedPortfolioConfig::new(config.portfolio.clone())
        .map_err(|error| ConfigError::new(error.to_string()))?
        .into_config();
    if normalized != config.portfolio {
        return Err(ConfigError::new(
            "portfolio must be normalized before internal collection expansion",
        ));
    }
    for child in &config.bitcoin_collections {
        child
            .validate()
            .map_err(|_| ConfigError::new("invalid Bitcoin network collection config"))?;
    }
    for child in &config.evm_collections {
        child
            .validate()
            .map_err(|_| ConfigError::new("invalid EVM network collection config"))?;
    }
    Ok(())
}

/// Typed input consumed by the transitional collection-completion fan-in state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.portfolio.input.collect_then_report_ready")]
pub struct CollectThenReportReadinessInput {
    /// One exact receipt from every Bitcoin network child.
    pub bitcoin_receipts: Vec<BtcNetworkCollectionReceipt>,
    /// One exact receipt from every EVM network child.
    pub evm_receipts: Vec<EvmNetworkCollectionReceipt>,
}

/// Pure fan-in state proving that every configured child completed one non-empty exact receipt.
pub struct CollectThenReportReadinessState {
    config: PortfolioInputsReadyConfig,
}

impl StateSpec for CollectThenReportReadinessState {
    type Config = PortfolioInputsReadyConfig;
    type Context = NoContext;
    type Input = CollectThenReportReadinessInput;
    type Output = PortfolioInputsReady;
    type Effect = mfm_effects::Pure;
    type Caps = mfm_capabilities::NoCaps;

    fn kind() -> mfm_program::Result<mfm_ids::StateKind> {
        mfm_ids::StateKind::new(
            OP_NAMESPACE,
            "collect_then_report_ready",
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.portfolio.state:collect_then_report_ready"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<mfm_ids::StateVersion> {
        mfm_ids::StateVersion::new("mfm.portfolio.state.collect_then_report_ready.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.portfolio.collect_then_report_ready"
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl mfm_program::PureState for CollectThenReportReadinessState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        collect_then_report_readiness(&self.config, input)
    }
}

/// Executes the temporary typed collection completion validation for a runtime runner.
pub fn collect_then_report_readiness(
    config: &PortfolioInputsReadyConfig,
    input: CollectThenReportReadinessInput,
) -> StateResult<PortfolioInputsReady> {
    let bitcoin_count = u32::try_from(input.bitcoin_receipts.len())
        .map_err(|_| StateError::Message("Bitcoin receipt count overflow".to_owned()))?;
    let evm_count = u32::try_from(input.evm_receipts.len())
        .map_err(|_| StateError::Message("EVM receipt count overflow".to_owned()))?;
    if bitcoin_count != config.bitcoin_network_count() || evm_count != config.evm_network_count() {
        return Err(StateError::Message(
            "collection receipt count does not match composed config".to_owned(),
        ));
    }
    let mut bitcoin_networks = BTreeSet::new();
    for receipt in &input.bitcoin_receipts {
        if receipt.entries().is_empty() || !bitcoin_networks.insert(receipt.network()) {
            return Err(StateError::Message(
                "Bitcoin collection receipts are incomplete or duplicated".to_owned(),
            ));
        }
    }
    let mut evm_networks = BTreeSet::new();
    for receipt in &input.evm_receipts {
        if receipt.native_balance_receipt().is_none() && receipt.erc20_balance_receipt().is_none()
            || !evm_networks.insert(receipt.network())
        {
            return Err(StateError::Message(
                "EVM collection receipts are incomplete or duplicated".to_owned(),
            ));
        }
    }
    Ok(PortfolioInputsReady::new(bitcoin_count, evm_count))
}

/// Typed internal collection/report operation.
pub struct CollectThenReportOperation;

impl Operation for CollectThenReportOperation {
    type Config = CollectThenReportConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = PortfolioOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            OP_NAMESPACE,
            OP_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.portfolio.operation:collect_then_report"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(OP_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.portfolio.collect_then_report"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let mut bitcoin_receipts = Vec::with_capacity(config.bitcoin_collections.len());
        for (index, child_config) in config.bitcoin_collections.iter().enumerate() {
            let child_config = child_config.clone();
            bitcoin_receipts.push(builder.child_scope(
                ScopeKey::new(format!("bitcoin_collection_{index}"))?,
                |child| {
                    let output = child.scope().call::<BtcNetworkCollectionOperation, _>(
                        OperationKey::new("btc_network_collection")?,
                        BtcNetworkCollectionOperation,
                        child_config,
                        (),
                    )?;
                    let receipt = child.export_to_parent(
                        BridgeKey::new("network_receipt")?,
                        output.receipt,
                        BridgePolicy::same_run_same_value(),
                    )?;
                    child.bridge_to_parent(receipt)
                },
            )?);
        }
        let mut evm_receipts = Vec::with_capacity(config.evm_collections.len());
        for (index, child_config) in config.evm_collections.iter().enumerate() {
            let child_config = child_config.clone();
            evm_receipts.push(builder.child_scope(
                ScopeKey::new(format!("evm_collection_{index}"))?,
                |child| {
                    let output = child.scope().call::<EvmNetworkCollectionOperation, _>(
                        OperationKey::new("evm_network_collection")?,
                        EvmNetworkCollectionOperation,
                        child_config,
                        (),
                    )?;
                    let receipt = child.export_to_parent(
                        BridgeKey::new("network_receipt")?,
                        output.receipt,
                        BridgePolicy::same_run_same_value(),
                    )?;
                    child.bridge_to_parent(receipt)
                },
            )?);
        }
        let readiness = builder.state::<CollectThenReportReadinessState, _>(
            StateKey::new("collections_ready")?,
            NoContext,
            config
                .readiness_config()
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            CollectThenReportReadinessInputHandles {
                bitcoin_receipts,
                evm_receipts,
            },
        )?;
        builder.call::<PortfolioTrackerWorkflowOperation, _>(
            OperationKey::new("portfolio_report")?,
            PortfolioTrackerWorkflowOperation,
            config.portfolio,
            readiness,
        )
    }
}

mfm_certify::define_program_descriptor_registry! {
    state_registry: pub collect_then_report_state_registry,
    operation_registry: pub collect_then_report_operation_registry,
    certification: pub register_collect_then_report_certification_descriptors,
    states: [
        CollectThenReportReadinessState,
        PortfolioInputsReadyState,
        ResolveSubjectsState,
        SelectHoldingsState,
        ResolveValuationsState,
        AssembleSnapshotState,
        ProjectReportState,
        mfm_op_btc_collectors::ResolveBtcJointTipState,
        mfm_op_btc_collectors::ObserveBtcAddressBalanceState,
        mfm_op_btc_collectors::RecordBtcAddressBalanceFactState,
        mfm_op_btc_collectors::AssembleBtcNetworkCollectionReceiptState,
        mfm_op_evm_collectors::ResolveEvmJointTipState,
        mfm_op_evm_collectors::ObserveEvmNativeBalanceState,
        mfm_op_evm_collectors::RecordEvmNativeBalanceFactState,
        mfm_op_evm_collectors::AssembleEvmNativeBalanceBatchReceiptState,
        mfm_op_evm_collectors::ObserveErc20TokenMetadataState,
        mfm_op_evm_collectors::ObserveErc20BalanceState,
        mfm_op_evm_collectors::RecordErc20BalanceFactState,
        mfm_op_evm_collectors::AssembleEvmErc20BalanceBatchReceiptState,
        mfm_op_evm_collectors::AssembleEvmNetworkCollectionReceiptState,
    ],
    operations: [
        CollectThenReportOperation,
        BtcNetworkCollectionOperation,
        mfm_op_btc_collectors::BtcNativeBalancesAtAnchorOperation,
        EvmNetworkCollectionOperation,
        mfm_op_evm_collectors::EvmNativeBalancesAtAnchorOperation,
        mfm_op_evm_collectors::EvmErc20BalancesAtAnchorOperation,
        PortfolioTrackerWorkflowOperation,
    ],
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const EVM_ACCOUNT: &str = "0x000000000000000000000000000000000000dead";
    const TOKEN: &str = "0x0000000000000000000000000000000000000001";

    #[test]
    fn derives_sorted_family_collections_from_explicit_demand() {
        let config = build_collect_then_report_config(portfolio_config(true, true, true))
            .expect("collection composition");
        assert_eq!(config.bitcoin_collections.len(), 1);
        assert_eq!(
            config.bitcoin_collections[0]
                .native_balances
                .addresses
                .len(),
            1
        );
        assert_eq!(config.evm_collections.len(), 1);
        let evm = &config.evm_collections[0];
        assert_eq!(evm.native_accounts.len(), 1);
        assert_eq!(evm.erc20_sources.len(), 1);
        assert_eq!(evm.erc20_sources[0].contract_address.as_str(), TOKEN);
        assert_eq!(evm.erc20_sources[0].account.as_str(), EVM_ACCOUNT);
    }

    #[test]
    fn token_only_demand_creates_an_evm_collection_without_native_accounts() {
        let config = build_collect_then_report_config(portfolio_config(false, false, true))
            .expect("token-only config");
        assert!(config.bitcoin_collections.is_empty());
        assert_eq!(config.evm_collections.len(), 1);
        assert!(config.evm_collections[0].native_accounts.is_empty());
        assert_eq!(config.evm_collections[0].erc20_sources.len(), 1);
    }

    #[test]
    fn rejects_portfolios_without_explicit_holding_demand() {
        let mut value = serde_json::to_value(portfolio_config(false, true, false))
            .expect("fixture serialization");
        value["wallets"][0]["symbol_ids"] = json!([]);
        let invalid = serde_json::from_value(value).expect("deserializable invalid aggregate");
        assert_eq!(
            build_collect_then_report_config(invalid).expect_err("empty demand is invalid"),
            CollectThenReportConfigError::InvalidPortfolio
        );
    }

    fn portfolio_config(
        include_btc: bool,
        include_evm_native: bool,
        include_evm_token: bool,
    ) -> ModelPortfolioConfig {
        let mut networks = Vec::new();
        let mut wallets = Vec::new();
        let mut symbols = Vec::new();
        if include_evm_native || include_evm_token {
            networks.push(json!({
                "network_id": "ethereum-mainnet",
                "family": "evm",
                "chain_id": 1,
                "native_decimals": 18,
                "metadata": {}
            }));
            let mut symbol_ids = Vec::new();
            if include_evm_native {
                symbol_ids.push("eth.native.ethereum-mainnet");
                symbols.push(symbol(
                    "eth.native.ethereum-mainnet",
                    "ethereum-mainnet",
                    json!({"kind": "native"}),
                ));
            }
            if include_evm_token {
                symbol_ids.push("usdc.wallet.ethereum-mainnet");
                symbols.push(symbol(
                    "usdc.wallet.ethereum-mainnet",
                    "ethereum-mainnet",
                    json!({"kind": "erc20", "contract_address": TOKEN}),
                ));
            }
            wallets.push(json!({
                "wallet_id": "wallet_eth",
                "network_id": "ethereum-mainnet",
                "symbol_ids": symbol_ids,
                "subject": {"kind": "evm_address", "address": EVM_ACCOUNT},
                "implementation": {"kind": "address_only"},
                "metadata": {}
            }));
        }
        if include_btc {
            networks.push(json!({
                "network_id": "bitcoin-mainnet",
                "family": "bitcoin",
                "bitcoin_network": "main",
                "source_identity": "public-bitcoin-core",
                "metadata": {}
            }));
            wallets.push(json!({
                "wallet_id": "wallet_btc",
                "network_id": "bitcoin-mainnet",
                "symbol_ids": ["btc.native.bitcoin-mainnet"],
                "subject": {"kind": "bitcoin_address", "address": "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh"},
                "implementation": {"kind": "address_only"},
                "metadata": {}
            }));
            symbols.push(symbol(
                "btc.native.bitcoin-mainnet",
                "bitcoin-mainnet",
                json!({"kind": "native"}),
            ));
        }
        serde_json::from_value(json!({
            "portfolio_id": "composition-test",
            "quote_codes": ["USD"],
            "networks": networks,
            "wallets": wallets,
            "symbol_configs": symbols,
            "metadata": {}
        }))
        .expect("portfolio fixture")
    }

    fn symbol(symbol_id: &str, network_id: &str, source: serde_json::Value) -> serde_json::Value {
        json!({
            "symbol_id": symbol_id,
            "display_symbol": symbol_id,
            "network_id": network_id,
            "source": source,
            "valuation": {
                "quotes": [{
                    "quote": "USD",
                    "priced_symbol_id": symbol_id,
                    "unit_price_dec": "1.00"
                }]
            },
            "metadata": {}
        })
    }
}
