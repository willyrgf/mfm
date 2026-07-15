#![warn(missing_docs)]
//! Typed portfolio tracker workflow operation (fact-backed report-only).
//!
//! Graph: ResolveSubjects → SelectHoldings → ResolveValuations → AssembleSnapshot → ProjectReport.

use mfm_ids::{DigestAlgorithm, OperationKind, OperationVersion};
use mfm_portfolio_model::domain_key::{
    HoldingsDomainKey, ReportDomainKey, SubjectDomainKey, ValuationDomainKey,
};
pub use mfm_portfolio_model::portfolio::PortfolioConfig;
use mfm_program::{Handle, NoContext, Operation, OperationExpansion, StateKey};
pub use mfm_state_portfolio::{
    portfolio_adapter_kind, portfolio_adapter_version, AssembleSnapshotConfig,
    AssembleSnapshotInput, AssembleSnapshotInputHandles, AssembleSnapshotState,
    PortfolioInputsReady, PortfolioInputsReadyConfig, PortfolioInputsReadyState,
    PortfolioOperationOutputs, PortfolioPublicOutputs, ProjectReportConfig, ProjectReportInput,
    ProjectReportInputHandles, ProjectReportState, ResolveSubjectsConfig, ResolveSubjectsState,
    ResolveValuationsConfig, ResolveValuationsState, SelectHoldingsConfig, SelectHoldingsState,
    SelectedHoldings, DEFAULT_PORTFOLIO_STORE_SCOPE,
};

const PORTFOLIO_OPERATION_KIND_NAME: &str = "tracker_workflow";
const PORTFOLIO_OPERATION_VERSION: &str = "mfm.portfolio.operation.tracker_workflow.v2";

/// Typed portfolio tracker workflow operation.
pub struct PortfolioTrackerWorkflowOperation;

/// Expands the shared fact-backed portfolio report graph from typed readiness evidence.
pub fn expand_portfolio_report<'program, 'scope>(
    builder: &mut OperationExpansion<'program, 'scope>,
    portfolio: PortfolioConfig,
    readiness: Handle<'program, 'scope, PortfolioInputsReady>,
) -> mfm_program::Result<PortfolioOperationOutputs<'program, 'scope>> {
    let portfolio = portfolio.normalized();

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
        readiness,
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

impl Operation for PortfolioTrackerWorkflowOperation {
    type Config = PortfolioConfig;
    type Input<'program, 'scope> = Handle<'program, 'scope, PortfolioInputsReady>;
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
        readiness: Self::Input<'program, 'scope>,
        builder: &mut OperationExpansion<'program, 'scope>,
        _dispatch: mfm_program::OperationExpansionDispatch<Self>,
    ) -> mfm_program::Result<Self::Output<'program, 'scope>> {
        expand_portfolio_report(builder, config.into_inner(), readiness)
    }
}

mfm_certify::define_program_descriptor_registry! {
    state_registry: pub portfolio_state_registry,
    operation_registry: pub portfolio_operation_registry,
    certification: pub register_portfolio_certification_descriptors,
    states: [
        PortfolioInputsReadyState,
        ResolveSubjectsState,
        SelectHoldingsState,
        ResolveValuationsState,
        AssembleSnapshotState,
        ProjectReportState,
    ],
    operations: [PortfolioTrackerWorkflowOperation],
}
