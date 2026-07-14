#![warn(missing_docs)]
//! Deterministic portfolio collector composition followed by the shared report graph.
//!
//! The operation owns relational derivation from one normalized portfolio. It launches one typed
//! BTC or EVM collector operation per relevant network, fans into typed readiness, and then calls
//! the independent portfolio tracker operation.

use std::collections::BTreeSet;
use std::num::NonZeroU64;

use mfm_catalog_model::CatalogRef;
use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_op_btc_collectors::{
    BtcAddressBalanceBatchSummary, BtcAddressBalanceConfig, BtcAddressBalanceObservationContext,
    BtcAddressBalanceOperation,
};
use mfm_op_evm_collectors::{
    EvmNativeBalanceBatchSummary, EvmNativeBalanceConfig, EvmNativeBalanceOperation,
};
use mfm_op_portfolio_tracker::{
    PortfolioInputsReady, PortfolioOperationOutputs, PortfolioPublicOutputs,
    PortfolioTrackerWorkflowOperation,
};
use mfm_portfolio_model::portfolio::{
    NetworkConfig, NetworkFamilyConfig, PortfolioConfig as ModelPortfolioConfig,
    ValidatedPortfolioConfig,
};
use mfm_portfolio_model::symbol::{BalanceReaderConfig, SymbolKind, SymbolRole};
use mfm_portfolio_model::wallet::WalletSubjectKind;
use mfm_program::{
    build_root_with_registries, BridgeKey, BridgePolicy, CanonicalSeed, Handle, NoContext,
    Operation, OperationExpansion, OperationKey, PublicOutputKey, RootBuilder, ScopeKey, SeedKey,
    StateError, StateKey, StateResult, StateSpec, TypedProgramLaunchPlan,
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
const ROOT_SCOPE: &str = "portfolio_collect_then_report";
const OP_KEY: &str = "collect_then_report";
const PUBLIC_OUTPUT_KEY: &str = "portfolio";
const BTC_CONTEXT_SEED_KEY: &str = "btc_observation_context";

/// Explicit read and coverage policy for Bitcoin collector batches.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BitcoinCollectorPolicy {
    /// Coverage claim written by each successful collector.
    pub coverage: String,
    /// Maximum source reads used for joint-tip resolution.
    pub max_source_reads: NonZeroU64,
}

impl BitcoinCollectorPolicy {
    /// Creates an explicit Bitcoin collector policy.
    pub fn new(coverage: impl Into<String>, max_source_reads: NonZeroU64) -> Self {
        Self {
            coverage: coverage.into(),
            max_source_reads,
        }
    }
}

/// Explicit read and coverage policy for EVM native-balance batches.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EvmCollectorPolicy {
    /// Coverage claim written by each successful collector.
    pub coverage: String,
    /// Native-asset decimals required to render collected quantities.
    pub decimals: Option<u8>,
    /// Maximum source reads used for joint-tip resolution.
    pub max_source_reads: NonZeroU64,
}

impl EvmCollectorPolicy {
    /// Creates an explicit EVM collector policy.
    pub fn new(
        coverage: impl Into<String>,
        decimals: Option<u8>,
        max_source_reads: NonZeroU64,
    ) -> Self {
        Self {
            coverage: coverage.into(),
            decimals,
            max_source_reads,
        }
    }
}

/// Strict public request shape for the composed entry point.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CollectThenReportRequest {
    /// Exact catalog reference to the portfolio config.
    pub portfolio: CatalogRef<ModelPortfolioConfig>,
    /// Explicit Bitcoin collector policy.
    pub bitcoin_policy: BitcoinCollectorPolicy,
    /// Explicit EVM collector policy.
    pub evm_policy: EvmCollectorPolicy,
}

impl CollectThenReportRequest {
    /// Returns the portfolio catalog reference.
    pub const fn portfolio(&self) -> &CatalogRef<ModelPortfolioConfig> {
        &self.portfolio
    }

    /// Returns the explicit Bitcoin collector policy.
    pub const fn bitcoin_policy(&self) -> &BitcoinCollectorPolicy {
        &self.bitcoin_policy
    }

    /// Returns the explicit EVM collector policy.
    pub const fn evm_policy(&self) -> &EvmCollectorPolicy {
        &self.evm_policy
    }
}

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
    /// A configured network did not have the required identity.
    #[error("network identity is missing")]
    MissingNetworkIdentity,
    /// A network has no supported native symbol.
    #[error("native symbol is missing or unsupported")]
    MissingNativeSymbol,
    /// A network has no wallet subject joined to its native symbol.
    #[error("collector collection is empty")]
    EmptyCollection,
    /// A wallet subject kind does not match its network family.
    #[error("wallet subject does not match network family")]
    UnsupportedWalletSubject,
    /// A wallet references a symbol that cannot join its network.
    #[error("wallet and symbol join is unsupported")]
    UnsupportedWalletSymbolJoin,
    /// Two configured wallets resolve to the same collector subject.
    #[error("collector subject is duplicated")]
    DuplicateSubject,
    /// The request omitted EVM native decimals.
    #[error("EVM native decimals are required")]
    MissingNativeDecimals,
    /// A collector policy is empty or unsupported.
    #[error("collector policy is invalid")]
    InvalidPolicy,
    /// A derived child config failed its own validation.
    #[error("derived collector config is invalid")]
    InvalidCollectorConfig,
}

/// Derives one complete composed-operation config from a validated portfolio and explicit policy.
pub fn build_collect_then_report_config(
    portfolio: ModelPortfolioConfig,
    bitcoin_policy: BitcoinCollectorPolicy,
    evm_policy: EvmCollectorPolicy,
) -> Result<CollectThenReportConfig, CollectThenReportConfigError> {
    if bitcoin_policy.coverage.is_empty() || evm_policy.coverage.is_empty() {
        return Err(CollectThenReportConfigError::InvalidPolicy);
    }
    let portfolio = ValidatedPortfolioConfig::new(portfolio)
        .map_err(|_| CollectThenReportConfigError::InvalidPortfolio)?
        .into_config();

    let mut bitcoin_collectors = Vec::new();
    let mut evm_collectors = Vec::new();
    let mut seen_any_subject = false;

    for network in &portfolio.networks {
        let native_symbol = native_symbol_for_network(&portfolio, network)?;
        let has_network_wallet = portfolio
            .wallets
            .iter()
            .any(|wallet| wallet.network_id == *network.network_id());
        let mut subjects = Vec::new();
        let mut seen_subjects = BTreeSet::new();

        for wallet in portfolio
            .wallets
            .iter()
            .filter(|wallet| wallet.network_id == *network.network_id())
        {
            let joins_native = wallet.symbol_ids.iter().any(|symbol_id| {
                native_symbol
                    .as_ref()
                    .is_some_and(|symbol| symbol.symbol_id == *symbol_id)
            });
            if !joins_native {
                continue;
            }
            for symbol_id in &wallet.symbol_ids {
                let symbol = portfolio
                    .symbol_configs
                    .iter()
                    .find(|symbol| symbol.symbol_id == *symbol_id)
                    .ok_or(CollectThenReportConfigError::UnsupportedWalletSymbolJoin)?;
                if symbol.network_id != wallet.network_id {
                    return Err(CollectThenReportConfigError::UnsupportedWalletSymbolJoin);
                }
            }
            let expected_subject = match network.family() {
                NetworkFamilyConfig::Bitcoin => WalletSubjectKind::BitcoinAddress,
                NetworkFamilyConfig::Evm => WalletSubjectKind::EvmAddress,
            };
            if wallet.subject.kind() != expected_subject {
                return Err(CollectThenReportConfigError::UnsupportedWalletSubject);
            }
            let address = wallet.subject.address_str().to_owned();
            if !seen_subjects.insert(address.clone()) {
                return Err(CollectThenReportConfigError::DuplicateSubject);
            }
            subjects.push(address);
        }

        if subjects.is_empty() {
            if native_symbol.is_some() {
                return Err(CollectThenReportConfigError::EmptyCollection);
            }
            if has_network_wallet {
                return Err(CollectThenReportConfigError::MissingNativeSymbol);
            }
            continue;
        }
        seen_any_subject = true;

        match network {
            NetworkConfig::Bitcoin {
                network_id,
                bitcoin_network,
                source_identity,
                ..
            } => {
                let Some(native_symbol) = native_symbol else {
                    return Err(CollectThenReportConfigError::MissingNativeSymbol);
                };
                let _ = native_symbol;
                let semantic_source_identity = source_identity.to_string();
                if semantic_source_identity.is_empty() || network_id.as_str().is_empty() {
                    return Err(CollectThenReportConfigError::MissingNetworkIdentity);
                }
                bitcoin_collectors.push(BtcAddressBalanceConfig {
                    network: network_id.to_string(),
                    bitcoin_network: bitcoin_network.clone(),
                    semantic_source_identity,
                    addresses: subjects,
                    coverage: bitcoin_policy.coverage.clone(),
                    max_source_reads: bitcoin_policy.max_source_reads,
                });
            }
            NetworkConfig::Evm {
                network_id,
                chain_id,
                ..
            } => {
                let Some(native_symbol) = native_symbol else {
                    return Err(CollectThenReportConfigError::MissingNativeSymbol);
                };
                let _ = native_symbol;
                let decimals = evm_policy
                    .decimals
                    .ok_or(CollectThenReportConfigError::MissingNativeDecimals)?;
                if network_id.as_str().is_empty() || chain_id.get() == 0 {
                    return Err(CollectThenReportConfigError::MissingNetworkIdentity);
                }
                evm_collectors.push(EvmNativeBalanceConfig {
                    network: network_id.to_string(),
                    chain_id: chain_id.get(),
                    accounts: subjects,
                    coverage: evm_policy.coverage.clone(),
                    decimals,
                    max_source_reads: evm_policy.max_source_reads,
                });
            }
        }
    }

    if !seen_any_subject {
        return Err(CollectThenReportConfigError::EmptyCollection);
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

fn native_symbol_for_network(
    portfolio: &ModelPortfolioConfig,
    network: &NetworkConfig,
) -> Result<Option<mfm_portfolio_model::symbol::SymbolConfig>, CollectThenReportConfigError> {
    let candidates = portfolio
        .symbol_configs
        .iter()
        .filter(|symbol| {
            symbol.network_id == *network.network_id() && symbol.kind == SymbolKind::NativeBalance
        })
        .cloned()
        .collect::<Vec<_>>();
    if candidates.len() > 1 {
        return Err(CollectThenReportConfigError::MissingNativeSymbol);
    }
    let Some(symbol) = candidates.into_iter().next() else {
        return Ok(None);
    };
    if symbol.role != SymbolRole::Native
        || !matches!(symbol.balance_reader, BalanceReaderConfig::NativeBalance {})
    {
        return Err(CollectThenReportConfigError::MissingNativeSymbol);
    }
    Ok(Some(symbol))
}

fn validate_collect_then_report_config(
    config: &CollectThenReportConfig,
) -> Result<(), ConfigError> {
    if config.bitcoin_collectors.is_empty() && config.evm_collectors.is_empty() {
        return Err(ConfigError::new("at least one collector is required"));
    }
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

/// Builds a typed draft for the composed collector/report operation.
pub fn collect_then_report_program_draft(
    config: CollectThenReportConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_root_with_registries(
        ScopeKey::new(ROOT_SCOPE)?,
        collect_then_report_state_registry()?,
        collect_then_report_operation_registry()?,
        |root: &mut RootBuilder<'_, '_>| {
            let observation_context = root.seed(
                SeedKey::new(BTC_CONTEXT_SEED_KEY)?,
                CanonicalSeed::from_value(&BtcAddressBalanceObservationContext {
                    observed_at_unix_ms: None,
                })?,
            )?;
            let result = root.scope().call::<CollectThenReportOperation, _>(
                OperationKey::new(OP_KEY)?,
                CollectThenReportOperation,
                config,
                observation_context,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
                &PortfolioPublicOutputs {
                    snapshot: result.snapshot,
                    report: result.report,
                },
            )
        },
    )
}

/// Builds launch material for the composed collector/report operation.
pub fn collect_then_report_program_launch_plan(
    config: CollectThenReportConfig,
) -> mfm_program::Result<TypedProgramLaunchPlan> {
    let draft = collect_then_report_program_draft(config)?;
    let mut seeds = std::collections::BTreeMap::new();
    for seed in draft.seeds() {
        seeds.insert(
            seed.seed_id.clone(),
            CanonicalSeed::from_value(&BtcAddressBalanceObservationContext {
                observed_at_unix_ms: None,
            })?
            .canonical_json()
            .clone(),
        );
    }
    TypedProgramLaunchPlan::from_draft_and_seed_material(draft, seeds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn policy_wire_shape_is_strict() {
        let policy =
            BitcoinCollectorPolicy::new("configured_only", NonZeroU64::new(1).expect("non-zero"));
        let json = serde_json::to_value(&policy).expect("policy json");
        assert_eq!(json["coverage"], "configured_only");
        assert!(
            serde_json::from_value::<BitcoinCollectorPolicy>(serde_json::json!({
                "coverage": "configured_only",
                "max_source_reads": 1,
                "unexpected": true
            }))
            .is_err()
        );
    }

    #[test]
    fn derives_btc_only_evm_only_and_dual_compositions() {
        let btc = build_collect_then_report_config(
            portfolio_config(true, false),
            bitcoin_policy(),
            evm_policy(Some(18)),
        )
        .expect("Bitcoin composition");
        assert_eq!(btc.bitcoin_collectors.len(), 1);
        assert!(btc.evm_collectors.is_empty());

        let evm = build_collect_then_report_config(
            portfolio_config(false, true),
            bitcoin_policy(),
            evm_policy(Some(18)),
        )
        .expect("EVM composition");
        assert!(evm.bitcoin_collectors.is_empty());
        assert_eq!(evm.evm_collectors.len(), 1);

        let dual = build_collect_then_report_config(
            portfolio_config(true, true),
            bitcoin_policy(),
            evm_policy(Some(18)),
        )
        .expect("dual composition");
        assert_eq!(dual.bitcoin_collectors.len(), 1);
        assert_eq!(dual.evm_collectors.len(), 1);
    }

    #[test]
    fn requires_evm_decimals_and_a_native_symbol() {
        let missing_decimals = build_collect_then_report_config(
            portfolio_config(false, true),
            bitcoin_policy(),
            evm_policy(None),
        )
        .expect_err("EVM decimals are required");
        assert_eq!(
            missing_decimals,
            CollectThenReportConfigError::MissingNativeDecimals
        );

        let mut without_native_json =
            serde_json::to_value(portfolio_config(false, true)).expect("portfolio json");
        without_native_json["symbol_configs"][0]["kind"] = json!("erc20_balance");
        without_native_json["symbol_configs"][0]["role"] = json!("asset");
        without_native_json["symbol_configs"][0]["balance_reader"] = json!({
            "kind": "erc20_balance",
            "token_address": "0x0000000000000000000000000000000000000001"
        });
        let without_native =
            serde_json::from_value(without_native_json).expect("portfolio without native symbol");
        let missing_symbol = build_collect_then_report_config(
            without_native,
            bitcoin_policy(),
            evm_policy(Some(18)),
        )
        .expect_err("native symbol is required");
        assert!(
            matches!(
                missing_symbol,
                CollectThenReportConfigError::EmptyCollection
                    | CollectThenReportConfigError::MissingNativeSymbol
            ),
            "unexpected error: {missing_symbol:?}"
        );
    }

    #[test]
    fn composed_draft_records_parent_child_and_tracker_lineage() {
        let config = build_collect_then_report_config(
            portfolio_config(true, true),
            bitcoin_policy(),
            evm_policy(Some(18)),
        )
        .expect("dual composition");
        let draft = collect_then_report_program_draft(config).expect("composed draft");
        let names = draft
            .operation_lineage()
            .iter()
            .map(|frame| frame.operation_name.as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"mfm.portfolio.collect_then_report"));
        assert!(names.contains(&"mfm.bitcoin.btc_address_balance"));
        assert!(names.contains(&"mfm.evm.evm_native_balance"));
        assert!(names.contains(&"mfm.portfolio.tracker_workflow"));
        let state_names = draft
            .state_nodes()
            .iter()
            .map(|node| node.state_kind.as_str().to_owned())
            .collect::<Vec<_>>();
        assert!(
            draft.state_nodes().iter().any(|node| {
                node.state_kind
                    .as_str()
                    .contains("collect_then_report_ready")
            }),
            "state names: {state_names:?}"
        );
    }

    fn bitcoin_policy() -> BitcoinCollectorPolicy {
        BitcoinCollectorPolicy::new("configured_only", NonZeroU64::new(1).expect("non-zero"))
    }

    fn evm_policy(decimals: Option<u8>) -> EvmCollectorPolicy {
        EvmCollectorPolicy::new(
            "configured_only",
            decimals,
            NonZeroU64::new(1).expect("non-zero"),
        )
    }

    fn portfolio_config(include_btc: bool, include_evm: bool) -> ModelPortfolioConfig {
        let mut networks = Vec::new();
        let mut wallets = Vec::new();
        let mut symbols = Vec::new();
        if include_evm {
            networks.push(json!({
                "network_id": "ethereum-mainnet",
                "family": "evm",
                "chain_id": 1,
                "metadata": {}
            }));
            wallets.push(json!({
                "wallet_id": "wallet_eth",
                "network_id": "ethereum-mainnet",
                "symbol_ids": ["eth.native.ethereum-mainnet"],
                "subject": {"kind": "evm_address", "address": "0x000000000000000000000000000000000000dead"},
                "implementation": {"kind": "address_only"},
                "metadata": {}
            }));
            symbols.push(json!({
                "symbol_id": "eth.native.ethereum-mainnet",
                "display_symbol": "ETH",
                "kind": "native_balance",
                "role": "native",
                "network_id": "ethereum-mainnet",
                "balance_reader": {"kind": "native_balance"},
                "valuation": {"quotes": [{"quote": "USD", "priced_symbol_id": "eth.native.ethereum-mainnet", "unit_price_dec": "1800.00"}]},
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
            symbols.push(json!({
                "symbol_id": "btc.native.bitcoin-mainnet",
                "display_symbol": "BTC",
                "kind": "native_balance",
                "role": "native",
                "network_id": "bitcoin-mainnet",
                "balance_reader": {"kind": "native_balance"},
                "valuation": {"quotes": [{"quote": "USD", "priced_symbol_id": "btc.native.bitcoin-mainnet", "unit_price_dec": "50000.00"}]},
                "metadata": {}
            }));
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
}
