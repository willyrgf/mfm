#![warn(missing_docs)]
//! Typed portfolio tracker workflow operation (fact-backed report-only).
//!
//! Graph: ResolveSubjects → SelectHoldings → ResolveValuations → AssembleSnapshot → ProjectReport.
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

use mfm_authored_config::{EntryPointDescriptor, TOML_JSON_AUTHORED_CONFIG_FORMATS};
use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_portfolio_config::{
    canonicalize_portfolio_snapshot_authored_config, PortfolioSnapshotAuthoredConfig,
    PortfolioSnapshotConfigError,
};
use mfm_portfolio_model::domain_key::{
    HoldingsDomainKey, ReportDomainKey, SubjectDomainKey, ValuationDomainKey,
};
use mfm_program::{
    build_root_with_registries, NoContext, Operation, OperationExpansion, OperationKey,
    PublicOutputKey, RootBuilder, ScopeKey, StateKey, TypedProgramLaunchPlan,
};
pub use mfm_state_portfolio::{
    balance_reader_kind, portfolio_adapter_kind, portfolio_adapter_version, AssembleSnapshotConfig,
    AssembleSnapshotInput, AssembleSnapshotInputHandles, AssembleSnapshotState,
    PortfolioOperationOutputs, PortfolioPublicOutputs, PortfolioWorkflowConfig,
    ProjectReportConfig, ProjectReportInput, ProjectReportInputHandles, ProjectReportState,
    ResolveSubjectsConfig, ResolveSubjectsState, ResolveValuationsConfig, ResolveValuationsState,
    SelectHoldingsConfig, SelectHoldingsState, SelectedHoldings, DEFAULT_PORTFOLIO_STORE_SCOPE,
};

const PORTFOLIO_OPERATION_KIND_NAME: &str = "tracker_workflow";
const PORTFOLIO_OPERATION_VERSION: &str = "mfm.portfolio.operation.tracker_workflow.v2";
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

        let subject_key = SubjectDomainKey::new("portfolio_subjects")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let holdings_key = HoldingsDomainKey::new("portfolio_holdings")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let valuation_key = ValuationDomainKey::new("portfolio_valuations")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;
        let report_key = ReportDomainKey::new("portfolio_report")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?;

        let subjects = builder.state_with_domain_keys::<ResolveSubjectsState, _, _>(
            StateKey::new("resolve_subjects")?,
            NoContext,
            ResolveSubjectsConfig::new(portfolio.wallets.clone())
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            (),
            vec![subject_key],
        )?;
        let holdings = builder.state_with_domain_keys::<SelectHoldingsState, _, _>(
            StateKey::new("select_holdings")?,
            NoContext,
            SelectHoldingsConfig::with_default_store_scope(portfolio.clone())
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            subjects.clone(),
            vec![holdings_key],
        )?;
        let valuations = builder.state_with_domain_keys::<ResolveValuationsState, _, _>(
            StateKey::new("resolve_valuations")?,
            NoContext,
            ResolveValuationsConfig::new(portfolio.symbol_configs.clone())
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            (),
            vec![valuation_key],
        )?;
        let snapshot = builder.state::<AssembleSnapshotState, _>(
            StateKey::new("assemble_snapshot")?,
            NoContext,
            AssembleSnapshotConfig::new(2, portfolio.clone())
                .map_err(|error| mfm_program::PlanError::Key(error.to_string()))?,
            AssembleSnapshotInputHandles {
                subjects,
                holdings,
                valuations,
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
        ResolveSubjectsState,
        SelectHoldingsState,
        ResolveValuationsState,
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

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_certify::certify_program_draft;
    use mfm_portfolio_model::metadata::PublicMetadata;
    use mfm_portfolio_model::portfolio::{NetworkConfig, NetworkFamilyConfig, PortfolioConfig};
    use mfm_portfolio_model::symbol::{
        BalanceReaderConfig, QuoteCode, QuoteValuationConfig, SymbolConfig, SymbolKind, SymbolRole,
        SymbolValuationConfig,
    };
    use mfm_portfolio_model::wallet::{
        WalletConfig, WalletImplementationConfig, WalletSubject, WalletSubjectKind,
    };
    use serde_json::Value;
    use std::collections::BTreeMap;

    #[test]
    fn portfolio_program_lowers_to_select_centric_graph() {
        let draft = portfolio_program_draft(sample_workflow_config()).expect("draft");
        assert_eq!(draft.state_nodes().len(), 5);
        let state_keys = draft
            .state_nodes()
            .iter()
            .map(|node| node.key.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            state_keys,
            [
                "resolve_subjects",
                "select_holdings",
                "resolve_valuations",
                "assemble_snapshot",
                "project_report",
            ]
        );
        assert!(
            !state_keys.iter().any(|key| {
                key.contains("pin_views")
                    || key.contains("observe")
                    || key.contains("merge_observations")
            }),
            "live pin/observe/merge must be absent: {state_keys:?}"
        );

        let certified = certify_program_draft(&draft).expect("certified portfolio spec");
        certified.envelope().verify_hash().expect("hash verifies");
        let spec_json = certified
            .envelope()
            .spec
            .canonical_json()
            .expect("canonical spec");
        let spec_value: Value = serde_json::from_slice(spec_json.as_bytes()).expect("spec json");
        let as_text = spec_value.to_string();
        assert!(as_text.contains("select_holdings"));
        assert!(!as_text.contains("pin_views"));
        assert!(!as_text.contains("observe_batch"));
        assert!(!as_text.contains("merge_observations"));
    }

    #[test]
    fn portfolio_snapshot_entry_point_plan_is_draft_only() {
        let authored = mfm_portfolio_config::PortfolioSnapshotAuthoredConfig {
            portfolio: sample_portfolio_config(),
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
        PortfolioWorkflowConfig::new(sample_portfolio_config()).expect("workflow config")
    }

    fn sample_portfolio_config() -> PortfolioConfig {
        PortfolioConfig {
            portfolio_id: "portfolio_main".parse().expect("valid portfolio id"),
            quote_codes: vec![QuoteCode::Usd],
            networks: vec![NetworkConfig::new(
                "ethereum-mainnet".to_owned(),
                NetworkFamilyConfig::Evm,
                Some(1),
                None,
                None,
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
                        unit_price_dec: "1800.00".parse().expect("valid unit price"),
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
