#![warn(missing_docs)]
//! Deterministic end-to-end portfolio snapshot composition.
//!
//! [`PortfolioSnapshotOperation`] accepts exactly one normalized [`PortfolioConfig`] authority,
//! compiles family demand into reusable collector operation calls, and passes their receipt
//! handles to one [`PortfolioReportOperation`]. The report operation owns store-backed selection,
//! snapshot assembly, and public report projection.
//!
//! # Examples
//!
//! ```no_run
//! use mfm_portfolio::{portfolio_snapshot_program_draft, PortfolioConfig};
//!
//! fn draft(config: PortfolioConfig) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
//!     portfolio_snapshot_program_draft(config)
//! }
//! ```

use std::collections::{BTreeMap, BTreeSet};

use mfm_bitcoin::{
    BitcoinBalanceCollectionConfig, BitcoinBalanceCollectionOperation, BitcoinBalanceSnapshotFact,
};
use mfm_evm::{
    EvmBalanceAsset, EvmBalanceCollectionConfig, EvmBalanceCollectionOperation,
    EvmBalanceSnapshotFact, EvmBalanceSource,
};
use mfm_facts::MfmFactType;
use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_program::{
    build_root_with_registries, BridgeKey, BridgePolicy, NoContext, Operation, OperationExpansion,
    OperationInputHandles, OperationKey, PublicOutputKey, RootBuilder, ScopeKey,
    TypedProgramLaunchPlan, ValidatedConfig,
};
use mfm_program_derive::OperationOutput;
use mfm_values::ConfigError;

use crate::model::domain_key::{HoldingsDomainKey, ReportDomainKey};
use crate::state::{
    AssembleSnapshotConfig, AssembleSnapshotInputHandles, AssembleSnapshotState,
    PortfolioPublicOutputs, ProjectReportConfig, ProjectReportInputHandles, ProjectReportState,
    SelectHoldingsConfig, SelectHoldingsFactDescriptors, SelectHoldingsInput,
    SelectHoldingsInputHandles, SelectHoldingsState,
};
use crate::{HoldingSourceConfig, NetworkConfig, PortfolioConfig, ValidatedPortfolioConfig};

const OP_NAMESPACE: &str = "mfm.portfolio";
const OP_KIND_NAME: &str = "snapshot";
const OP_VERSION: &str = "mfm.portfolio.operation.snapshot.v2";
const REPORT_OP_KIND_NAME: &str = "report";
const REPORT_OP_VERSION: &str = "mfm.portfolio.operation.report.v2";
const ROOT_SCOPE: &str = "portfolio_snapshot";
const OPERATION_KEY: &str = "portfolio_snapshot";
const REPORT_OPERATION_KEY: &str = "portfolio_report";
const PUBLIC_OUTPUT_KEY: &str = "portfolio_snapshot";

/// Internal handles produced by the complete portfolio snapshot operation.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.portfolio.operation_outputs.snapshot")]
pub struct PortfolioSnapshotOperationOutputs<'program, 'scope> {
    /// Fully assembled portfolio snapshot.
    pub snapshot: mfm_program::Handle<'program, 'scope, crate::PortfolioSnapshot>,
    /// Public report projection derived from the snapshot.
    pub report: mfm_program::Handle<'program, 'scope, crate::PortfolioReport>,
}

/// Internal handles produced by receipt-pinned portfolio report composition.
#[derive(OperationOutput)]
#[mfm(schema = "mfm.portfolio.operation_outputs.report")]
pub struct PortfolioReportOperationOutputs<'program, 'scope> {
    /// Fully assembled portfolio snapshot.
    pub snapshot: mfm_program::Handle<'program, 'scope, crate::PortfolioSnapshot>,
    /// Public report projection derived from the snapshot.
    pub report: mfm_program::Handle<'program, 'scope, crate::PortfolioReport>,
}

/// Deterministic operation for one complete receipt-pinned portfolio snapshot.
pub struct PortfolioSnapshotOperation;

impl Operation for PortfolioSnapshotOperation {
    type Config = PortfolioConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = PortfolioSnapshotOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            OP_NAMESPACE,
            OP_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.portfolio.operation:snapshot"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(OP_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.portfolio.snapshot"
    }

    fn expand<'program, 'scope>(
        &self,
        config: ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let supplied = config.into_inner();
        let portfolio = ValidatedPortfolioConfig::new(supplied.clone())
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?
            .into_config();
        if supplied != portfolio {
            return Err(mfm_program::PlanError::Key(
                "portfolio operation config must be normalized before expansion".to_owned(),
            ));
        }
        let compiled = compile_collection(&portfolio)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;

        let mut bitcoin_receipts = Vec::with_capacity(compiled.bitcoin_collections.len());
        for (index, child_config) in compiled.bitcoin_collections.iter().enumerate() {
            let child_config = child_config.clone();
            bitcoin_receipts.push(builder.child_scope(
                ScopeKey::new(format!("bitcoin_collection_{index}"))?,
                |child| {
                    let output = child.scope().call::<BitcoinBalanceCollectionOperation, _>(
                        OperationKey::new("bitcoin_balance_collection")?,
                        BitcoinBalanceCollectionOperation,
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
        let mut evm_receipts = Vec::with_capacity(compiled.evm_collections.len());
        for (index, child_config) in compiled.evm_collections.iter().enumerate() {
            let child_config = child_config.clone();
            evm_receipts.push(builder.child_scope(
                ScopeKey::new(format!("evm_collection_{index}"))?,
                |child| {
                    let output = child.scope().call::<EvmBalanceCollectionOperation, _>(
                        OperationKey::new("evm_balance_collection")?,
                        EvmBalanceCollectionOperation,
                        child_config,
                        (),
                    )?;
                    let receipt = child.export_to_parent(
                        BridgeKey::new("collection_receipt")?,
                        output.receipt,
                        BridgePolicy::same_run_same_value(),
                    )?;
                    child.bridge_to_parent(receipt)
                },
            )?);
        }
        let report = builder.call::<PortfolioReportOperation, _>(
            OperationKey::new(REPORT_OPERATION_KEY)?,
            PortfolioReportOperation,
            portfolio,
            OperationInputHandles::new(SelectHoldingsInputHandles {
                bitcoin_receipts,
                evm_receipts,
            }),
        )?;
        Ok(PortfolioSnapshotOperationOutputs {
            snapshot: report.snapshot,
            report: report.report,
        })
    }
}

/// Deterministic receipt-pinned portfolio report operation.
///
/// Its structured operation input is passed unchanged to [`SelectHoldingsState`]. The receipt
/// edges are therefore the exact collector-settlement readiness barrier; this operation creates no
/// aggregate receipt value or alternate replay surface.
pub struct PortfolioReportOperation;

impl Operation for PortfolioReportOperation {
    type Config = PortfolioConfig;
    type Input<'program, 'scope> =
        OperationInputHandles<SelectHoldingsInput, SelectHoldingsInputHandles<'program, 'scope>>;
    type Output<'program, 'scope> = PortfolioReportOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            OP_NAMESPACE,
            REPORT_OP_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.portfolio.operation:report"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(REPORT_OP_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.portfolio.report"
    }

    fn expand<'program, 'scope>(
        &self,
        config: ValidatedConfig<Self::Config>,
        input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let portfolio = config.into_inner();
        let normalized = ValidatedPortfolioConfig::new(portfolio.clone())
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?
            .into_config();
        if normalized != portfolio {
            return Err(mfm_program::PlanError::Key(
                "portfolio report config must be normalized before expansion".to_owned(),
            ));
        }

        let holdings_key = HoldingsDomainKey::new("portfolio_holdings")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let report_key = ReportDomainKey::new("portfolio_report")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;

        let holdings = builder.state_with_domain_keys::<SelectHoldingsState, _, _>(
            mfm_program::StateKey::new("select_holdings")?,
            NoContext,
            SelectHoldingsConfig::new(portfolio.clone(), holding_fact_descriptors()?)
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            input.into_handles(),
            vec![holdings_key],
        )?;
        let snapshot = builder.state::<AssembleSnapshotState, _>(
            mfm_program::StateKey::new("assemble_snapshot")?,
            NoContext,
            AssembleSnapshotConfig::new(portfolio.clone())
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            AssembleSnapshotInputHandles { holdings },
        )?;
        let report = builder.state_with_domain_keys::<ProjectReportState, _, _>(
            mfm_program::StateKey::new("project_report")?,
            NoContext,
            ProjectReportConfig::default(),
            ProjectReportInputHandles {
                snapshot: snapshot.clone(),
            },
            vec![report_key],
        )?;

        Ok(PortfolioReportOperationOutputs { snapshot, report })
    }
}

fn holding_fact_descriptors() -> mfm_program::Result<SelectHoldingsFactDescriptors> {
    SelectHoldingsFactDescriptors::new(
        &BitcoinBalanceSnapshotFact::descriptor()
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
        &EvmBalanceSnapshotFact::descriptor()
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
    )
    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
}

/// Builds one complete typed portfolio snapshot program draft.
///
/// The draft binds exactly one [`PortfolioPublicOutputs`] value. Application ingress uses this
/// exact helper after resolving the sole `mfm.portfolio/snapshot@2` portfolio reference; it does
/// not maintain a parallel app-owned graph builder.
pub fn portfolio_snapshot_program_draft(
    config: PortfolioConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_root_with_registries(
        ScopeKey::new(ROOT_SCOPE)?,
        portfolio_snapshot_state_registry()?,
        portfolio_snapshot_operation_registry()?,
        |root: &mut RootBuilder<'_, '_>| {
            let output = root.scope().call::<PortfolioSnapshotOperation, _>(
                OperationKey::new(OPERATION_KEY)?,
                PortfolioSnapshotOperation,
                config,
                (),
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new(PUBLIC_OUTPUT_KEY)?,
                &PortfolioPublicOutputs {
                    snapshot: output.snapshot,
                    report: output.report,
                },
            )
        },
    )
}

/// Builds the launch plan for one complete typed portfolio snapshot program.
pub fn portfolio_snapshot_program_launch_plan(
    config: PortfolioConfig,
) -> mfm_program::Result<TypedProgramLaunchPlan> {
    TypedProgramLaunchPlan::from_draft(portfolio_snapshot_program_draft(config)?)
}

struct CompiledCollection {
    bitcoin_collections: Vec<BitcoinBalanceCollectionConfig>,
    evm_collections: Vec<EvmBalanceCollectionConfig>,
}

#[derive(Default)]
struct NetworkCollectionDemand {
    bitcoin_addresses: BTreeSet<String>,
    evm_sources: BTreeSet<EvmBalanceSource>,
}

fn compile_collection(portfolio: &PortfolioConfig) -> Result<CompiledCollection, ConfigError> {
    let symbols = portfolio
        .symbol_configs
        .iter()
        .map(|symbol| (symbol.symbol_id.as_str(), symbol))
        .collect::<BTreeMap<_, _>>();
    let networks = portfolio
        .networks
        .iter()
        .map(|network| (network.network_id().as_str(), network))
        .collect::<BTreeMap<_, _>>();
    let mut demand = BTreeMap::<String, NetworkCollectionDemand>::new();

    for wallet in &portfolio.wallets {
        let network = networks
            .get(wallet.network_id.as_str())
            .copied()
            .ok_or_else(|| ConfigError::new("portfolio wallet referenced an unknown network"))?;
        for symbol_id in &wallet.symbol_ids {
            let network_demand = demand.entry(wallet.network_id.to_string()).or_default();
            let symbol = symbols
                .get(symbol_id.as_str())
                .copied()
                .ok_or_else(|| ConfigError::new("portfolio wallet referenced an unknown symbol"))?;
            match (network, &symbol.source) {
                (NetworkConfig::Bitcoin { .. }, HoldingSourceConfig::Native) => {
                    let address = wallet.subject.address_str().to_owned();
                    network_demand.bitcoin_addresses.insert(address.clone());
                }
                (NetworkConfig::Evm { .. }, HoldingSourceConfig::Native) => {
                    let account = wallet.subject.evm_address().ok_or_else(|| {
                        ConfigError::new("EVM wallet did not contain an EVM address")
                    })?;
                    network_demand.evm_sources.insert(
                        EvmBalanceSource::new(
                            account
                                .to_address()
                                .map_err(|error| ConfigError::new(error.to_string()))?,
                            EvmBalanceAsset::Native,
                        )
                        .map_err(|error| ConfigError::new(error.to_string()))?,
                    );
                }
                (NetworkConfig::Evm { .. }, HoldingSourceConfig::Erc20 { contract_address }) => {
                    let account = wallet.subject.evm_address().ok_or_else(|| {
                        ConfigError::new("EVM wallet did not contain an EVM address")
                    })?;
                    network_demand.evm_sources.insert(
                        EvmBalanceSource::new(
                            account
                                .to_address()
                                .map_err(|error| ConfigError::new(error.to_string()))?,
                            EvmBalanceAsset::erc20(
                                contract_address
                                    .to_address()
                                    .map_err(|error| ConfigError::new(error.to_string()))?,
                            )
                            .map_err(|error| ConfigError::new(error.to_string()))?,
                        )
                        .map_err(|error| ConfigError::new(error.to_string()))?,
                    );
                }
                _ => {
                    return Err(ConfigError::new(
                        "portfolio source did not match network family",
                    ))
                }
            }
        }
    }
    let mut bitcoin_collections = Vec::new();
    let mut evm_collections = Vec::new();
    for (network_id, network_demand) in demand {
        let network = networks
            .get(network_id.as_str())
            .copied()
            .ok_or_else(|| ConfigError::new("compiled demand referenced an unknown network"))?;
        match network {
            NetworkConfig::Bitcoin {
                bitcoin_network,
                source_identity,
                ..
            } => {
                if network_demand.bitcoin_addresses.is_empty()
                    || !network_demand.evm_sources.is_empty()
                {
                    return Err(ConfigError::new(
                        "Bitcoin demand did not match native source shape",
                    ));
                }
                bitcoin_collections.push(BitcoinBalanceCollectionConfig::new(
                    network_id,
                    bitcoin_network.clone(),
                    source_identity.to_string(),
                    network_demand.bitcoin_addresses.into_iter().collect(),
                )?);
            }
            NetworkConfig::Evm {
                chain_id,
                native_decimals,
                ..
            } => {
                if !network_demand.bitcoin_addresses.is_empty()
                    || network_demand.evm_sources.is_empty()
                {
                    return Err(ConfigError::new("EVM demand did not match source shape"));
                }
                evm_collections.push(EvmBalanceCollectionConfig::new(
                    network_id,
                    chain_id.get(),
                    *native_decimals,
                    network_demand.evm_sources.into_iter().collect(),
                )?);
            }
        }
    }
    Ok(CompiledCollection {
        bitcoin_collections,
        evm_collections,
    })
}

mfm_certify::define_program_descriptor_registry! {
    state_registry: pub portfolio_snapshot_state_registry,
    operation_registry: pub portfolio_snapshot_operation_registry,
    certification: pub register_portfolio_snapshot_certification_descriptors,
    authoring_catalog: pub portfolio_snapshot_authoring_catalog,
    includes: [
        {
            state_registry: mfm_bitcoin::bitcoin_collectors_state_registry,
            operation_registry: mfm_bitcoin::bitcoin_collectors_operation_registry,
            certification: mfm_bitcoin::register_bitcoin_collectors_certification_descriptors,
            authoring_catalog: mfm_bitcoin::bitcoin_collectors_authoring_catalog,
        },
        {
            state_registry: mfm_evm::evm_collectors_state_registry,
            operation_registry: mfm_evm::evm_collectors_operation_registry,
            certification: mfm_evm::register_evm_collectors_certification_descriptors,
            authoring_catalog: mfm_evm::evm_collectors_authoring_catalog,
        },
    ],
    states: [
        crate::state::SelectHoldingsState,
        crate::state::AssembleSnapshotState,
        crate::state::ProjectReportState,
    ],
    operations: [
        PortfolioSnapshotOperation,
        PortfolioReportOperation,
    ],
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_program::StateSpec;
    use serde_json::json;

    const EVM_ACCOUNT: &str = "0x000000000000000000000000000000000000dead";
    const TOKEN: &str = "0x0000000000000000000000000000000000000001";
    const BTC_ADDRESS: &str = "bc1qxy2kgdygjrsqtzq2n0yrf2493p83kkfjhx0wlh";

    #[test]
    fn compiler_derives_bounded_family_work() {
        let portfolio = portfolio_config(true, true, true);
        let compiled = compile_collection(&portfolio).expect("compile collection");
        assert_eq!(compiled.bitcoin_collections.len(), 1);
        assert_eq!(compiled.evm_collections.len(), 1);
        let sources = compiled.evm_collections[0].sources();
        assert_eq!(sources.len(), 2);
        assert!(sources
            .iter()
            .any(|source| matches!(source.asset(), EvmBalanceAsset::Native)));
        assert!(sources.iter().any(|source| {
            matches!(
                source.asset(),
                EvmBalanceAsset::Erc20 { contract_address }
                    if contract_address.as_str() == TOKEN
            )
        }));
    }

    #[test]
    fn compiler_keeps_token_only_network_without_native_child_demand() {
        let compiled = compile_collection(&portfolio_config(false, false, true))
            .expect("compile token-only collection");
        assert!(compiled.bitcoin_collections.is_empty());
        assert_eq!(compiled.evm_collections.len(), 1);
        assert_eq!(compiled.evm_collections[0].sources().len(), 1);
        assert!(matches!(
            compiled.evm_collections[0].sources()[0].asset(),
            EvmBalanceAsset::Erc20 { contract_address }
                if contract_address.as_str() == TOKEN
        ));
    }

    #[test]
    fn compiler_skips_a_zero_symbol_wallet_on_an_undemanded_network() {
        let mut value =
            serde_json::to_value(portfolio_config(false, true, false)).expect("portfolio value");
        value["networks"]
            .as_array_mut()
            .expect("portfolio networks")
            .push(json!({
                "network_id": "bitcoin-mainnet",
                "family": "bitcoin",
                "bitcoin_network": "main",
                "source_identity": "public-bitcoin-core",
                "metadata": {}
            }));
        value["wallets"]
            .as_array_mut()
            .expect("portfolio wallets")
            .push(json!({
                "wallet_id": "wallet_btc_zero",
                "network_id": "bitcoin-mainnet",
                "symbol_ids": [],
                "subject": {"kind": "bitcoin_address", "address": BTC_ADDRESS},
                "implementation": {"kind": "address_only"},
                "metadata": {}
            }));
        let portfolio = ValidatedPortfolioConfig::new(
            serde_json::from_value(value).expect("zero-symbol wallet portfolio"),
        )
        .expect("zero-symbol wallet remains model-valid with another demanded edge")
        .into_config();

        let compiled = compile_collection(&portfolio).expect("compile explicit demand only");
        assert!(compiled.bitcoin_collections.is_empty());
        assert_eq!(compiled.evm_collections.len(), 1);
    }

    #[test]
    fn snapshot_root_owns_the_complete_objective_and_one_public_output_binding() {
        let config = ValidatedPortfolioConfig::new(portfolio_config(true, true, true))
            .expect("normalize portfolio")
            .into_config();
        let first = portfolio_snapshot_program_draft(config.clone()).expect("first snapshot draft");
        let second =
            portfolio_snapshot_program_draft(config.clone()).expect("second snapshot draft");

        assert_eq!(first, second, "snapshot expansion must be deterministic");
        assert_eq!(first.root_key().as_str(), ROOT_SCOPE);
        assert_eq!(first.public_output_spec().key().as_str(), PUBLIC_OUTPUT_KEY);
        assert_eq!(
            first
                .public_output_spec()
                .outputs()
                .iter()
                .map(|output| output.public_field_path().as_str())
                .collect::<Vec<_>>(),
            ["snapshot", "report"]
        );
        assert!(first
            .operation_lineage()
            .iter()
            .any(|operation| operation.operation_name == "mfm.portfolio.snapshot"));
        let collect_kind = mfm_evm::CollectEvmBalancesState::kind().expect("collect state kind");
        assert_eq!(
            first
                .state_nodes()
                .iter()
                .filter(|node| node.state_kind == collect_kind)
                .count(),
            1
        );
        mfm_certify::certify_program_draft(&first).expect("snapshot draft certifies");
        let launch = portfolio_snapshot_program_launch_plan(config).expect("snapshot launch plan");
        assert_eq!(launch.draft, first);
        assert!(!launch.config_material.is_empty());
    }

    #[test]
    fn snapshot_registries_compose_child_collector_inventories() {
        let states = portfolio_snapshot_state_registry().expect("snapshot states");
        let btc_states = mfm_bitcoin::bitcoin_collectors_state_registry().expect("Bitcoin states");
        let evm_states = mfm_evm::evm_collectors_state_registry().expect("EVM states");
        assert_eq!(states.len(), btc_states.len() + evm_states.len() + 3);
        states
            .state_descriptor::<mfm_bitcoin::CollectBitcoinBalancesState>()
            .expect("composed Bitcoin state");
        states
            .state_descriptor::<mfm_evm::CollectEvmBalancesState>()
            .expect("composed EVM state");

        let operations = portfolio_snapshot_operation_registry().expect("snapshot operations");
        let btc_operations =
            mfm_bitcoin::bitcoin_collectors_operation_registry().expect("Bitcoin operations");
        let evm_operations = mfm_evm::evm_collectors_operation_registry().expect("EVM operations");
        assert_eq!(
            operations.len(),
            btc_operations.len() + evm_operations.len() + 2
        );
        operations
            .operation_descriptor::<mfm_bitcoin::BitcoinBalanceCollectionOperation>()
            .expect("composed Bitcoin operation");
        operations
            .operation_descriptor::<EvmBalanceCollectionOperation>()
            .expect("composed EVM operation");

        let mut actual = mfm_certify::CertificationRegistry::new();
        register_portfolio_snapshot_certification_descriptors(&mut actual)
            .expect("snapshot certification descriptors");
        let mut expected = mfm_certify::CertificationRegistry::new();
        mfm_bitcoin::register_bitcoin_collectors_certification_descriptors(&mut expected)
            .expect("Bitcoin certification descriptors");
        mfm_evm::register_evm_collectors_certification_descriptors(&mut expected)
            .expect("EVM certification descriptors");
        expected
            .register_state::<crate::state::SelectHoldingsState>()
            .expect("selection state");
        expected
            .register_state::<crate::state::AssembleSnapshotState>()
            .expect("assembly state");
        expected
            .register_state::<crate::state::ProjectReportState>()
            .expect("report state");
        expected
            .register_operation::<PortfolioSnapshotOperation>()
            .expect("snapshot operation");
        expected
            .register_operation::<PortfolioReportOperation>()
            .expect("report operation");
        assert_eq!(actual, expected);

        let catalog = portfolio_snapshot_authoring_catalog().expect("snapshot authoring catalog");
        assert_eq!(catalog.state_descriptors().len(), states.len());
        assert_eq!(catalog.operation_descriptors().len(), operations.len());
        assert_eq!(
            catalog.emitted_fact_descriptors().len(),
            mfm_bitcoin::bitcoin_collectors_authoring_catalog()
                .expect("Bitcoin authoring catalog")
                .emitted_fact_descriptors()
                .len()
                + mfm_evm::evm_collectors_authoring_catalog()
                    .expect("EVM authoring catalog")
                    .emitted_fact_descriptors()
                    .len()
        );
        assert_eq!(catalog.side_effect_state_descriptor_ids().len(), 0);
    }

    #[test]
    fn family_work_vectors_are_exact_for_all_evm_and_empty_demand() {
        let native_only = compile_collection(&portfolio_config(false, true, false))
            .expect("compile native-only collection");
        assert!(native_only.bitcoin_collections.is_empty());
        assert_eq!(native_only.evm_collections.len(), 1);

        let empty = compile_collection(&portfolio_config(false, false, false))
            .expect("empty portfolio has no collection work");
        assert!(empty.bitcoin_collections.is_empty());
        assert!(empty.evm_collections.is_empty());
    }

    fn portfolio_config(
        include_btc: bool,
        include_evm_native: bool,
        include_evm_token: bool,
    ) -> PortfolioConfig {
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
