#![warn(missing_docs)]
//! Deterministic portfolio collector composition followed by the shared report graph.
//!
//! The operation owns relational derivation from one normalized portfolio. It launches one typed
//! BTC or EVM collector operation per relevant network, fans into typed readiness, and then calls
//! the independent portfolio tracker operation.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU64;

use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_op_btc_collectors::{
    BtcAddressBalanceBatchSummary, BtcAddressBalanceConfig, BtcAddressBalanceObservationContext,
    BtcAddressBalanceOperation, BTC_JOINT_TIP_SOURCE_READS, BTC_NATIVE_BALANCE_COVERAGE,
};
use mfm_op_evm_collectors::{
    EvmNativeBalanceBatchSummary, EvmNativeBalanceConfig, EvmNativeBalanceOperation,
};
use mfm_op_portfolio_tracker::{
    PortfolioInputsReady, PortfolioOperationOutputs, PortfolioTrackerWorkflowOperation,
};
use mfm_portfolio_model::portfolio::{
    NetworkConfig, PortfolioConfig as ModelPortfolioConfig, ValidatedPortfolioConfig,
};
use mfm_portfolio_model::symbol::HoldingSourceConfig;
use mfm_program::{
    BridgeKey, BridgePolicy, Handle, NoContext, Operation, OperationExpansion, OperationKey,
    ScopeKey, StateError, StateKey, StateResult, StateSpec,
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

/// Complete deterministic config for the composed collector/report operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, MfmConfig)]
#[serde(deny_unknown_fields)]
#[mfm(
    schema = "mfm.portfolio.operation.config.collect_then_report",
    validate = "validate_collect_then_report_config"
)]
pub struct CollectThenReportConfig {
    /// Normalized portfolio consumed by the report graph.
    pub portfolio: ModelPortfolioConfig,
    /// One derived Bitcoin collector config per relevant Bitcoin network.
    pub bitcoin_collectors: Vec<BtcAddressBalanceConfig>,
    /// One derived EVM collector config per relevant EVM network.
    pub evm_collectors: Vec<EvmNativeBalanceConfig>,
}

impl CollectThenReportConfig {
    /// Returns the readiness counts used by the parent fan-in state.
    pub fn readiness_config(&self) -> Result<PortfolioInputsReadyConfig, ConfigError> {
        let bitcoin_network_count = u32::try_from(self.bitcoin_collectors.len())
            .map_err(|_| ConfigError::new("too many Bitcoin collector networks"))?;
        let evm_network_count = u32::try_from(self.evm_collectors.len())
            .map_err(|_| ConfigError::new("too many EVM collector networks"))?;
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
    /// A derived child config failed its own validation.
    #[error("derived collector config is invalid")]
    InvalidCollectorConfig,
}

/// Derives native collector children from the exact normalized holding demand.
///
/// Token-only demand intentionally contributes no native collector child at this cutover point.
/// The complete token path is added by the anchored ERC-20 collector phase; no caller-selected
/// fallback collector is synthesized here.
pub fn build_collect_then_report_config(
    portfolio: ModelPortfolioConfig,
) -> Result<CollectThenReportConfig, CollectThenReportConfigError> {
    let portfolio = ValidatedPortfolioConfig::new(portfolio)
        .map_err(|_| CollectThenReportConfigError::InvalidPortfolio)?
        .into_config();
    let native_subjects = native_subjects_by_network(&portfolio);
    let mut bitcoin_collectors = Vec::new();
    let mut evm_collectors = Vec::new();
    for network in &portfolio.networks {
        let Some(subjects) = native_subjects.get(network.network_id().as_str()) else {
            continue;
        };
        match network {
            NetworkConfig::Bitcoin {
                network_id,
                bitcoin_network,
                source_identity,
                ..
            } => {
                let max_source_reads = NonZeroU64::new(BTC_JOINT_TIP_SOURCE_READS)
                    .ok_or(CollectThenReportConfigError::InvalidCollectorConfig)?;
                bitcoin_collectors.push(BtcAddressBalanceConfig {
                    network: network_id.to_string(),
                    bitcoin_network: bitcoin_network.clone(),
                    semantic_source_identity: source_identity.to_string(),
                    addresses: subjects.iter().cloned().collect(),
                    coverage: BTC_NATIVE_BALANCE_COVERAGE.to_owned(),
                    max_source_reads,
                });
            }
            NetworkConfig::Evm { .. } => evm_collectors.push(EvmNativeBalanceConfig {
                network: network.clone(),
                accounts: subjects.iter().cloned().collect(),
            }),
        }
    }
    let config = CollectThenReportConfig {
        portfolio,
        bitcoin_collectors,
        evm_collectors,
    };
    validate_collect_then_report_config(&config)
        .map_err(|_| CollectThenReportConfigError::InvalidCollectorConfig)?;
    Ok(config)
}

fn native_subjects_by_network(
    portfolio: &ModelPortfolioConfig,
) -> BTreeMap<String, BTreeSet<String>> {
    let symbols = portfolio
        .symbol_configs
        .iter()
        .map(|symbol| (symbol.symbol_id.clone(), symbol))
        .collect::<BTreeMap<_, _>>();
    let mut subjects = BTreeMap::<String, BTreeSet<String>>::new();
    for wallet in &portfolio.wallets {
        for symbol_id in &wallet.symbol_ids {
            let Some(symbol) = symbols.get(symbol_id) else {
                continue;
            };
            if !matches!(&symbol.source, HoldingSourceConfig::Native) {
                continue;
            }
            subjects
                .entry(wallet.network_id.to_string())
                .or_default()
                .insert(wallet.subject.address_str().to_owned());
        }
    }
    subjects
}

fn validate_collect_then_report_config(
    config: &CollectThenReportConfig,
) -> Result<(), ConfigError> {
    ValidatedPortfolioConfig::new(config.portfolio.clone())
        .map_err(|error| ConfigError::new(error.to_string()))?;
    for child in &config.bitcoin_collectors {
        child
            .validate()
            .map_err(|_| ConfigError::new("invalid Bitcoin collector config"))?;
    }
    for child in &config.evm_collectors {
        child
            .validate()
            .map_err(|_| ConfigError::new("invalid EVM collector config"))?;
    }
    Ok(())
}

/// Typed input consumed by the composed readiness fan-in state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, StateInput)]
#[mfm(schema = "mfm.portfolio.input.collect_then_report_ready")]
pub struct CollectThenReportReadinessInput {
    /// One typed summary from every Bitcoin child operation.
    pub bitcoin_summaries: Vec<BtcAddressBalanceBatchSummary>,
    /// One typed summary from every EVM child operation.
    pub evm_summaries: Vec<EvmNativeBalanceBatchSummary>,
}

/// Pure fan-in state proving that every collector child produced a non-empty summary.
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

/// Executes the composed readiness validation for an erased runtime runner.
pub fn collect_then_report_readiness(
    config: &PortfolioInputsReadyConfig,
    input: CollectThenReportReadinessInput,
) -> StateResult<PortfolioInputsReady> {
    let bitcoin_count = u32::try_from(input.bitcoin_summaries.len())
        .map_err(|_| StateError::Message("Bitcoin summary count overflow".to_owned()))?;
    let evm_count = u32::try_from(input.evm_summaries.len())
        .map_err(|_| StateError::Message("EVM summary count overflow".to_owned()))?;
    if bitcoin_count != config.bitcoin_network_count() || evm_count != config.evm_network_count() {
        return Err(StateError::Message(
            "collector summary count does not match composed config".to_owned(),
        ));
    }
    let mut bitcoin_networks = BTreeSet::new();
    for summary in &input.bitcoin_summaries {
        if summary.address_count() == 0 || !bitcoin_networks.insert(summary.network()) {
            return Err(StateError::Message(
                "Bitcoin collector summaries are incomplete or duplicated".to_owned(),
            ));
        }
    }
    let mut evm_networks = BTreeSet::new();
    for summary in &input.evm_summaries {
        if summary.account_count() == 0 || !evm_networks.insert(summary.network()) {
            return Err(StateError::Message(
                "EVM collector summaries are incomplete or duplicated".to_owned(),
            ));
        }
    }
    Ok(PortfolioInputsReady::new(bitcoin_count, evm_count))
}

/// Typed composed collector/report operation.
pub struct CollectThenReportOperation;

impl Operation for CollectThenReportOperation {
    type Config = CollectThenReportConfig;
    type Input<'program, 'scope> = Handle<'program, 'scope, BtcAddressBalanceObservationContext>;
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
        observation_context: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let mut bitcoin_summaries = Vec::with_capacity(config.bitcoin_collectors.len());
        for (index, child_config) in config.bitcoin_collectors.iter().enumerate() {
            let child_config = child_config.clone();
            let summary = builder.child_scope(
                ScopeKey::new(format!("bitcoin_collector_{index}"))?,
                |child| {
                    let observation_context = child.import_from_parent(
                        BridgeKey::new("observation_context")?,
                        observation_context.clone(),
                        BridgePolicy::same_run_same_value(),
                    )?;
                    let child_output = child.scope().call::<BtcAddressBalanceOperation, _>(
                        OperationKey::new("btc_address_balance")?,
                        BtcAddressBalanceOperation,
                        child_config,
                        observation_context,
                    )?;
                    let summary = child.export_to_parent(
                        BridgeKey::new("batch_summary")?,
                        child_output.batch_summary,
                        BridgePolicy::same_run_same_value(),
                    )?;
                    child.bridge_to_parent(summary)
                },
            )?;
            bitcoin_summaries.push(summary);
        }
        let mut evm_summaries = Vec::with_capacity(config.evm_collectors.len());
        for (index, child_config) in config.evm_collectors.iter().enumerate() {
            let child_config = child_config.clone();
            let summary =
                builder.child_scope(ScopeKey::new(format!("evm_collector_{index}"))?, |child| {
                    let child_output = child.scope().call::<EvmNativeBalanceOperation, _>(
                        OperationKey::new("evm_native_balance")?,
                        EvmNativeBalanceOperation,
                        child_config,
                        (),
                    )?;
                    let summary = child.export_to_parent(
                        BridgeKey::new("batch_summary")?,
                        child_output.batch_summary,
                        BridgePolicy::same_run_same_value(),
                    )?;
                    child.bridge_to_parent(summary)
                })?;
            evm_summaries.push(summary);
        }
        let readiness = builder.state::<CollectThenReportReadinessState, _>(
            StateKey::new("collectors_ready")?,
            NoContext,
            config
                .readiness_config()
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            CollectThenReportReadinessInputHandles {
                bitcoin_summaries,
                evm_summaries,
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
        mfm_op_btc_collectors::AssembleBtcAddressBalanceBatchState,
        mfm_op_evm_collectors::ResolveEvmJointTipState,
        mfm_op_evm_collectors::ObserveEvmNativeBalanceState,
        mfm_op_evm_collectors::RecordEvmNativeBalanceFactState,
        mfm_op_evm_collectors::AssembleEvmNativeBalanceBatchState,
    ],
    operations: [
        CollectThenReportOperation,
        BtcAddressBalanceOperation,
        EvmNativeBalanceOperation,
        PortfolioTrackerWorkflowOperation,
    ],
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn derives_native_collectors_from_explicit_demand() {
        let config = build_collect_then_report_config(portfolio_config(true, true, true))
            .expect("native composition");
        let btc = &config.bitcoin_collectors;
        assert_eq!(btc.len(), 1);
        assert_eq!(btc[0].coverage, BTC_NATIVE_BALANCE_COVERAGE);
        assert_eq!(btc[0].max_source_reads.get(), BTC_JOINT_TIP_SOURCE_READS);
        assert_eq!(config.evm_collectors.len(), 1);
        let evm = &config.evm_collectors[0];
        assert_eq!(evm.accounts, vec![EVM_ACCOUNT.to_owned()]);
        assert_eq!(evm.network.native_decimals(), Some(18));
        assert_eq!(
            evm.observe_config_for_account(EVM_ACCOUNT)
                .expect("derived observer")
                .evm_network_parts()
                .expect("derived EVM network"),
            ("ethereum-mainnet", 1, 18)
        );
    }

    #[test]
    fn token_only_demand_does_not_create_a_native_child() {
        let config = build_collect_then_report_config(portfolio_config(false, false, true))
            .expect("token-only model config remains valid");
        assert!(config.bitcoin_collectors.is_empty());
        assert!(config.evm_collectors.is_empty());
        config.validate().expect("derived config");
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

    const EVM_ACCOUNT: &str = "0x000000000000000000000000000000000000dead";

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
                    json!({
                        "kind": "erc20",
                        "contract_address": "0x0000000000000000000000000000000000000001"
                    }),
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
