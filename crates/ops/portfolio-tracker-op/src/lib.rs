#![warn(missing_docs)]
//! Typed portfolio tracker workflow operation.
//!
//! The portfolio tracker workflow is authored through `mfm-program` and lowers to certified typed
//! state programs.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_portfolio_tracker::{portfolio_program_draft, PortfolioWorkflowConfig};
//!
//! # fn demo(config: PortfolioWorkflowConfig) -> mfm_program::Result<()> {
//! let draft = portfolio_program_draft(config)?;
//! assert!(!draft.state_nodes().is_empty());
//! # Ok(())
//! # }
//! ```

use std::collections::{BTreeMap, BTreeSet};

use mfm_authored_config::{EntryPointDescriptor, TOML_JSON_AUTHORED_CONFIG_FORMATS};
use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_portfolio_config::{
    canonicalize_portfolio_snapshot_authored_config, PortfolioSnapshotAuthoredConfig,
    PortfolioSnapshotConfigError,
};
use mfm_portfolio_model::domain_key::{
    ObservationBatchDomainKey, ReportDomainKey, SourceDomainKey, SubjectDomainKey,
    ValuationDomainKey, ViewDomainKey,
};
use mfm_portfolio_model::portfolio::{NetworkConfig, PortfolioConfig};
use mfm_portfolio_model::symbol::SymbolConfig;
use mfm_portfolio_model::wallet::WalletConfig;
use mfm_program::{
    build_root_with_registries, DomainKeyedNonEmptyHandles, NoContext, Operation,
    OperationExpansion, OperationKey, PublicOutputKey, RootBuilder, ScopeKey, StateKey,
    TypedProgramLaunchPlan,
};
pub use mfm_state_portfolio::{
    balance_reader_kind, observation_batch_id, portfolio_adapter_kind, portfolio_adapter_version,
    AssembleSnapshotConfig, AssembleSnapshotInput, AssembleSnapshotInputHandles,
    AssembleSnapshotState, MergeObservationsConfig, MergeObservationsState, ObservationBatch,
    ObserveBatchConfig, ObserveBatchInput, ObserveBatchInputHandles, ObserveBatchState,
    PinViewsConfig, PinViewsState, PortfolioOperationOutputs, PortfolioPublicOutputs,
    PortfolioWorkflowConfig, PrepareSourcesConfig, PrepareSourcesState, ProjectReportConfig,
    ProjectReportInput, ProjectReportInputHandles, ProjectReportState, ResolveSubjectsConfig,
    ResolveSubjectsState, ResolveValuationsConfig, ResolveValuationsState,
};

const PORTFOLIO_OPERATION_KIND_NAME: &str = "tracker_workflow";
const PORTFOLIO_OPERATION_VERSION: &str = "mfm.portfolio.operation.tracker_workflow.v1";
const ROOT_SCOPE: &str = "portfolio";
const OP_KEY: &str = "portfolio_tracker";
const PUBLIC_OUTPUT_KEY: &str = "portfolio";

/// Public portfolio snapshot entry-point descriptor.
pub const PORTFOLIO_SNAPSHOT_ENTRY_POINT: EntryPointDescriptor = EntryPointDescriptor {
    namespace: "mfm.portfolio",
    name: "portfolio_snapshot",
    public_name: "portfolio_snapshot",
    version: 1,
    accepted_config_formats: TOML_JSON_AUTHORED_CONFIG_FORMATS,
};

/// Typed portfolio tracker workflow operation.
pub struct PortfolioTrackerWorkflowOperation;

impl Operation for PortfolioTrackerWorkflowOperation {
    type Config = PortfolioWorkflowConfig;
    type Input<'program, 'scope> = ();
    type Output<'program, 'scope> = PortfolioOperationOutputs<'program, 'scope>;

    fn kind() -> mfm_program::Result<OperationKind> {
        OperationKind::new(
            "mfm.portfolio",
            PORTFOLIO_OPERATION_KIND_NAME,
            DigestAlgorithm::Sha256JcsV1,
            mfm_canonical::sha256_digest_bytes(b"mfm.portfolio.operation:tracker_workflow"),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<OperationVersion> {
        OperationVersion::new(PORTFOLIO_OPERATION_VERSION)
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.portfolio.tracker_workflow"
    }

    fn expand<'program, 'scope>(
        &self,
        config: mfm_program::ValidatedConfig<Self::Config>,
        _input: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        let config = config.into_inner();
        let portfolio = config.portfolio().clone().normalized();
        let valuation_source_registry = config.valuation_source_registry().clone().normalized();
        let networks_by_id = networks_by_id(&portfolio.networks)?;

        let source_key = SourceDomainKey::new("portfolio_sources")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let subject_key = SubjectDomainKey::new("portfolio_subjects")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let view_key = ViewDomainKey::new("portfolio_views")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let valuation_key = ValuationDomainKey::new("portfolio_valuations")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let report_key = ReportDomainKey::new("portfolio_report")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;

        let prepared = builder.state_with_domain_keys::<PrepareSourcesState, _, _>(
            StateKey::new("prepare_sources")?,
            NoContext,
            PrepareSourcesConfig::new(portfolio.networks.clone())
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            (),
            vec![source_key],
        )?;
        let subjects = builder.state_with_domain_keys::<ResolveSubjectsState, _, _>(
            StateKey::new("resolve_subjects")?,
            NoContext,
            ResolveSubjectsConfig::new(portfolio.wallets.clone())
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            prepared.clone(),
            vec![subject_key],
        )?;
        let views = builder.state_with_domain_keys::<PinViewsState, _, _>(
            StateKey::new("pin_views")?,
            NoContext,
            PinViewsConfig::new(portfolio.networks.clone())
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            prepared,
            vec![view_key],
        )?;
        let valuations = builder.state_with_domain_keys::<ResolveValuationsState, _, _>(
            StateKey::new("resolve_valuations")?,
            NoContext,
            ResolveValuationsConfig::new(
                portfolio.symbol_configs.clone(),
                valuation_source_registry,
            )
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            views.clone(),
            vec![valuation_key],
        )?;

        let mut observation_handles = Vec::new();
        let mut seen_observation_keys = BTreeSet::new();
        for (wallet, symbol) in observation_targets(&portfolio)? {
            let network = networks_by_id
                .get(symbol.network_id.as_str())
                .copied()
                .ok_or_else(|| {
                    mfm_program::PlanError::Key(format!(
                        "missing network `{}` for symbol `{}`",
                        symbol.network_id, symbol.symbol_id
                    ))
                })?;
            let batch_key = observation_batch_id(&wallet.wallet_id, &symbol.symbol_id);
            if !seen_observation_keys.insert(batch_key.clone()) {
                return Err(mfm_program::PlanError::DuplicateDomainKey(batch_key));
            }
            let observation_key = ObservationBatchDomainKey::new(batch_key.clone())
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
            let handle = builder.state_with_domain_keys::<ObserveBatchState, _, _>(
                StateKey::new(format!("observe/{batch_key}"))?,
                NoContext,
                ObserveBatchConfig::new(wallet.clone(), symbol.clone(), network.clone())
                    .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
                ObserveBatchInputHandles {
                    subjects: subjects.clone(),
                    views: views.clone(),
                    valuations: valuations.clone(),
                },
                vec![observation_key.clone()],
            )?;
            observation_handles.push((observation_key, handle));
        }

        let observations = builder.state::<MergeObservationsState, _>(
            StateKey::new("merge_observations")?,
            NoContext,
            MergeObservationsConfig::new(),
            DomainKeyedNonEmptyHandles::<ObservationBatchDomainKey, ObservationBatch>::new(
                observation_handles,
            )?,
        )?;
        let snapshot = builder.state::<AssembleSnapshotState, _>(
            StateKey::new("assemble_snapshot")?,
            NoContext,
            AssembleSnapshotConfig::new(2, portfolio.clone())
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            AssembleSnapshotInputHandles {
                subjects,
                views,
                observations,
            },
        )?;
        let report = builder.state_with_domain_keys::<ProjectReportState, _, _>(
            StateKey::new("project_report")?,
            NoContext,
            ProjectReportConfig::new(2)
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            ProjectReportInputHandles {
                snapshot: snapshot.clone(),
            },
            vec![report_key],
        )?;

        Ok(PortfolioOperationOutputs { snapshot, report })
    }
}

mfm_certify::define_program_descriptor_registry! {
    state_registry: pub portfolio_state_registry,
    operation_registry: pub portfolio_operation_registry,
    certification: pub register_portfolio_certification_descriptors,
    states: [
        PrepareSourcesState,
        ResolveSubjectsState,
        PinViewsState,
        ResolveValuationsState,
        ObserveBatchState,
        MergeObservationsState,
        AssembleSnapshotState,
        ProjectReportState,
    ],
    operations: [PortfolioTrackerWorkflowOperation],
}

/// Builds a typed portfolio program draft.
pub fn portfolio_program_draft(
    config: PortfolioWorkflowConfig,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    build_root_with_registries(
        ScopeKey::new(ROOT_SCOPE)?,
        portfolio_state_registry()?,
        portfolio_operation_registry()?,
        |root: &mut RootBuilder<'_, '_>| {
            let result = root.scope().call::<PortfolioTrackerWorkflowOperation, _>(
                OperationKey::new(OP_KEY)?,
                PortfolioTrackerWorkflowOperation,
                config,
                (),
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

/// Plans a portfolio snapshot entry-point program from human-authored config.
pub fn plan_portfolio_snapshot_entry_point(
    authored: PortfolioSnapshotAuthoredConfig,
) -> Result<TypedProgramLaunchPlan, PortfolioSnapshotPlanError> {
    let canonical = canonicalize_portfolio_snapshot_authored_config(authored)?;
    let workflow_config: PortfolioWorkflowConfig = canonical.into();
    let draft = portfolio_program_draft(workflow_config)?;
    Ok(TypedProgramLaunchPlan::from_draft(draft)?)
}

/// Error returned while planning a portfolio snapshot entry point.
#[derive(Debug, thiserror::Error)]
pub enum PortfolioSnapshotPlanError {
    /// Authored portfolio snapshot config failed canonical validation.
    #[error("portfolio snapshot config failed: {0}")]
    Config(#[from] PortfolioSnapshotConfigError),
    /// Program drafting or config material selection failed.
    #[error("portfolio snapshot planning failed: {0}")]
    Plan(#[from] mfm_program::PlanError),
}

fn networks_by_id(
    networks: &[NetworkConfig],
) -> mfm_program::Result<BTreeMap<&str, &NetworkConfig>> {
    let mut by_id = BTreeMap::new();
    for network in networks {
        if by_id
            .insert(network.network_id().as_str(), network)
            .is_some()
        {
            return Err(mfm_program::PlanError::DuplicateDomainKey(
                network.network_id().to_string(),
            ));
        }
    }
    Ok(by_id)
}

fn symbols_by_id(symbols: &[SymbolConfig]) -> mfm_program::Result<BTreeMap<&str, &SymbolConfig>> {
    let mut by_id = BTreeMap::new();
    for symbol in symbols {
        if by_id.insert(symbol.symbol_id.as_str(), symbol).is_some() {
            return Err(mfm_program::PlanError::DuplicateDomainKey(
                symbol.symbol_id.to_string(),
            ));
        }
    }
    Ok(by_id)
}

fn observation_targets(
    portfolio: &PortfolioConfig,
) -> mfm_program::Result<Vec<(&WalletConfig, &SymbolConfig)>> {
    let symbols = symbols_by_id(&portfolio.symbol_configs)?;
    let mut targets = Vec::new();
    for wallet in &portfolio.wallets {
        for symbol_id in &wallet.symbol_ids {
            let symbol = symbols.get(symbol_id.as_str()).copied().ok_or_else(|| {
                mfm_program::PlanError::Key(format!(
                    "wallet `{}` referenced unknown symbol `{symbol_id}`",
                    wallet.wallet_id
                ))
            })?;
            targets.push((wallet, symbol));
        }
    }
    targets.sort_by(|left, right| {
        (
            left.0.wallet_id.as_str(),
            left.1.network_id.as_str(),
            left.1.symbol_id.as_str(),
        )
            .cmp(&(
                right.0.wallet_id.as_str(),
                right.1.network_id.as_str(),
                right.1.symbol_id.as_str(),
            ))
    });
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_certify::certify_program_draft;
    use mfm_portfolio_model::metadata::PublicMetadata;
    use mfm_portfolio_model::portfolio::NetworkFamilyConfig;
    use mfm_portfolio_model::symbol::{
        BalanceReaderConfig, QuoteCode, QuoteValuationConfig, SymbolKind, SymbolRole,
        SymbolValuationConfig, ValuationReaderConfig, ValuationSourceRegistry,
    };
    use mfm_portfolio_model::wallet::{
        WalletImplementationConfig, WalletSubject, WalletSubjectKind,
    };
    use serde_json::Value;

    #[test]
    fn portfolio_program_lowers_to_typed_state_contracts() {
        let draft = portfolio_program_draft(sample_workflow_config()).expect("draft");
        assert_eq!(draft.state_nodes().len(), 8);
        assert_eq!(
            draft
                .state_nodes()
                .iter()
                .filter(|node| !node.output_domain_keys.is_empty())
                .count(),
            6,
            "source, subject, view, valuation, observation batch, and report outputs need domain-key lineage"
        );
        let state_keys = draft
            .state_nodes()
            .iter()
            .map(|node| node.key.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            state_keys,
            [
                "prepare_sources",
                "resolve_subjects",
                "pin_views",
                "resolve_valuations",
                "observe/wallet/wallet_main/symbol/eth.native.ethereum-mainnet",
                "merge_observations",
                "assemble_snapshot",
                "project_report",
            ]
        );

        let certified = certify_program_draft(&draft).expect("certified portfolio spec");
        assert_eq!(
            certified
                .envelope()
                .spec
                .nodes
                .iter()
                .map(|node| node.stable_key.as_str())
                .collect::<Vec<_>>(),
            [
                "prepare_sources",
                "resolve_subjects",
                "pin_views",
                "resolve_valuations",
                "observe/wallet/wallet_main/symbol/eth.native.ethereum-mainnet",
                "merge_observations",
                "assemble_snapshot",
                "project_report",
                "portfolio",
                "framework/project-retention-manifest",
                "framework/complete-run",
                "framework/resolve-saga-terminal",
            ]
        );
        assert_eq!(
            draft
                .public_output_spec()
                .outputs()
                .iter()
                .map(|output| output.public_field_path().as_str())
                .collect::<Vec<_>>(),
            ["snapshot", "report"]
        );
        assert_eq!(
            certified
                .envelope()
                .spec
                .value_lineages
                .iter()
                .filter(|lineage| !lineage.domain_keys.is_empty())
                .count(),
            6,
            "certified portfolio spec must retain value-lineage domain keys"
        );
        certified.envelope().verify_hash().expect("hash verifies");
        let spec_json = certified
            .envelope()
            .spec
            .canonical_json()
            .expect("canonical spec");
        let spec_value: Value = serde_json::from_slice(spec_json.as_bytes()).expect("spec json");
        assert!(spec_value
            .to_string()
            .contains("\"ordering\":\"stable_domain_key\""));
    }

    #[test]
    fn duplicate_observation_domain_key_is_rejected() {
        let mut portfolio = sample_portfolio_config();
        portfolio.wallets[0].symbol_ids.push(
            "eth.native.ethereum-mainnet"
                .parse()
                .expect("valid symbol id"),
        );
        let config = PortfolioWorkflowConfig::new(portfolio, sample_valuation_source_registry())
            .expect("workflow config");
        let err = portfolio_program_draft(config).expect_err("duplicate domain key");
        assert!(matches!(err, mfm_program::PlanError::DuplicateDomainKey(_)));
    }

    #[test]
    fn portfolio_snapshot_entry_point_plan_is_draft_only() {
        let authored = mfm_portfolio_config::PortfolioSnapshotAuthoredConfig {
            portfolio: sample_portfolio_config(),
            valuation_source_registry: sample_valuation_source_registry(),
        };

        let planned = plan_portfolio_snapshot_entry_point(authored).expect("entry-point plan");

        assert!(!planned.draft.state_nodes().is_empty());
        assert!(!planned.config_material.is_empty());
        assert!(planned.seed_material.is_empty());
    }

    #[test]
    fn portfolio_snapshot_entry_point_descriptor_is_public_launch_surface() {
        assert_eq!(PORTFOLIO_SNAPSHOT_ENTRY_POINT.namespace, "mfm.portfolio");
        assert_eq!(PORTFOLIO_SNAPSHOT_ENTRY_POINT.name, "portfolio_snapshot");
        assert_eq!(
            PORTFOLIO_SNAPSHOT_ENTRY_POINT.public_name,
            "portfolio_snapshot"
        );
        assert_eq!(PORTFOLIO_SNAPSHOT_ENTRY_POINT.version, 1);
        assert_eq!(
            PORTFOLIO_SNAPSHOT_ENTRY_POINT.accepted_config_formats,
            TOML_JSON_AUTHORED_CONFIG_FORMATS
        );
    }

    fn sample_workflow_config() -> PortfolioWorkflowConfig {
        PortfolioWorkflowConfig::new(
            sample_portfolio_config(),
            sample_valuation_source_registry(),
        )
        .expect("workflow config")
    }

    fn sample_valuation_source_registry() -> ValuationSourceRegistry {
        ValuationSourceRegistry {
            sources: Vec::new(),
        }
    }

    fn sample_portfolio_config() -> PortfolioConfig {
        PortfolioConfig {
            portfolio_id: "portfolio_main".parse().expect("valid portfolio id"),
            quote_codes: vec![QuoteCode::Usd],
            networks: vec![NetworkConfig::new(
                "ethereum-mainnet".to_owned(),
                NetworkFamilyConfig::Evm,
                Some(1),
                "shared".to_owned(),
                BTreeMap::new(),
            )
            .expect("valid network config")],
            wallets: vec![WalletConfig {
                wallet_id: "wallet_main".parse().expect("valid wallet id"),
                subject: WalletSubject::new(
                    "0x000000000000000000000000000000000000dead",
                    WalletSubjectKind::EvmAddress,
                )
                .expect("valid wallet subject"),
                network_id: "ethereum-mainnet".parse().expect("valid network id"),
                implementation: WalletImplementationConfig::AddressOnly {},
                symbol_ids: vec!["eth.native.ethereum-mainnet"
                    .parse()
                    .expect("valid symbol id")],
                metadata: PublicMetadata::default(),
            }],
            symbol_configs: vec![SymbolConfig {
                symbol_id: "eth.native.ethereum-mainnet"
                    .parse()
                    .expect("valid symbol id"),
                display_symbol: Some("ETH".to_owned()),
                kind: SymbolKind::NativeBalance,
                role: SymbolRole::Native,
                network_id: "ethereum-mainnet".parse().expect("valid network id"),
                protocol: None,
                balance_reader: BalanceReaderConfig::NativeBalance {},
                valuation: SymbolValuationConfig {
                    quotes: vec![QuoteValuationConfig {
                        quote: QuoteCode::Usd,
                        priced_symbol_id: "eth.native.ethereum-mainnet"
                            .parse()
                            .expect("valid priced symbol id"),
                        reader: ValuationReaderConfig::FixedUnitPrice {
                            unit_price_dec: "1800.00".parse().expect("valid unit price"),
                        },
                    }],
                },
                decimals: Some(18),
                underlying_symbol_id: None,
                metadata: PublicMetadata::default(),
            }],
            metadata: PublicMetadata::default(),
        }
    }
}
