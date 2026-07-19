#![warn(missing_docs)]
//! Portfolio adapter runners for certified portfolio snapshots.
//!
//! The Platform fact-index binding rereads and verifies Bitcoin and generic EVM facts before
//! portfolio assembly. Family collection execution belongs to the family adapters.

use std::sync::Arc;

use mfm_events::v1 as events;
use mfm_fact_capabilities::FactIndexReadProvider;
use mfm_program::StateSpec;
use mfm_runtime::{
    load_materialized_struct_input, load_runner_config_for_node, ErasedNodeRunner, ErasedRunCtx,
    ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry, ExternalReadExecution,
    ExternalReadExecutionFuture, ExternalReadPlanExecutor, ExternalReadRunner,
    RunnerExecutableIdentityTemplate, RunnerRegistrationBuilder,
};
use mfm_state_portfolio::{
    assemble_snapshot, portfolio_adapter_kind, portfolio_adapter_version, AssembleSnapshotConfig,
    AssembleSnapshotInput, AssembleSnapshotState, ProjectReportConfig, ProjectReportInput,
    ProjectReportState, SelectHoldingsConfig, SelectHoldingsReadEvidence, SelectHoldingsReadPlan,
    SelectHoldingsState, SelectedHoldings,
};
use mfm_store::v1 as store;
use mfm_values::MfmValue;

#[path = "replay.rs"]
mod replay;
#[path = "selection.rs"]
mod selection;
pub use self::replay::verify_portfolio_replay;

const PURE_FACTORY: &str = "pure";
const READ_FACTORY: &str = "read_external";
const ADAPTER_FACTORY: &str = "portfolio_adapter";

/// Runtime capabilities used by portfolio adapter runners.
#[derive(Clone)]
pub struct PortfolioRunnerCapabilities {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    fact_index: Arc<dyn FactIndexReadProvider>,
}

impl PortfolioRunnerCapabilities {
    /// Creates portfolio runner capabilities from artifact and Platform fact-index providers.
    pub fn new(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        fact_index: Arc<dyn FactIndexReadProvider>,
    ) -> Self {
        Self {
            artifacts,
            fact_index,
        }
    }

    fn artifacts(&self) -> Arc<dyn store::RetainedArtifactReadProvider> {
        Arc::clone(&self.artifacts)
    }

    fn fact_index(&self) -> Arc<dyn FactIndexReadProvider> {
        Arc::clone(&self.fact_index)
    }
}

/// Registers receipt-pinned selection and snapshot projection runners.
pub fn register_portfolio_runners(
    registry: &mut ErasedRunnerRegistry,
    capabilities: PortfolioRunnerCapabilities,
) -> mfm_runtime::Result<()> {
    let artifacts = capabilities.artifacts();
    let fact_index = capabilities.fact_index();
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    let executable_identities = RunnerExecutableIdentityTemplate::new(
        "mfm-adapters-portfolio",
        "typed-portfolio",
        env!("CARGO_PKG_VERSION"),
    )?;
    let pure_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(PURE_FACTORY)?);
    let read_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(READ_FACTORY)?);
    let adapter_factory =
        executable_identities.factory_binding(events::RunnerFactoryId::new(ADAPTER_FACTORY)?);
    registrations.register_adapter_executable_with_factory(
        portfolio_adapter_kind()?,
        portfolio_adapter_version()?,
        &adapter_factory,
    )?;
    registrations.register_state_runner_with_factory::<SelectHoldingsState>(
        &read_factory,
        Arc::new(ExternalReadRunner::<SelectHoldingsState, _>::new(
            artifacts.clone(),
            SelectHoldingsExecutor {
                artifacts: artifacts.clone(),
                fact_index,
            },
        )),
    )?;
    registrations.register_state_runner_with_factory::<AssembleSnapshotState>(
        &pure_factory,
        Arc::new(AssembleSnapshotRunner {
            artifacts: artifacts.clone(),
        }),
    )?;
    registrations.register_state_runner_with_factory::<ProjectReportState>(
        &pure_factory,
        Arc::new(ProjectReportRunner { artifacts }),
    )?;
    Ok(())
}

struct SelectHoldingsExecutor {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    fact_index: Arc<dyn FactIndexReadProvider>,
}

impl ExternalReadPlanExecutor<SelectHoldingsState> for SelectHoldingsExecutor {
    fn validate_ingress<'a>(
        &'a self,
        _ctx: mfm_runtime::RunnerIngressContext<'a>,
        _state: &'a SelectHoldingsState,
    ) -> mfm_runtime::RunnerIngressFuture<'a> {
        Box::pin(async { Ok(()) })
    }

    fn execute<'a>(
        &'a self,
        plan: &'a SelectHoldingsReadPlan,
        _ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, SelectHoldingsReadEvidence> {
        Box::pin(async move {
            let requests = plan.requests().map_err(portfolio_state_runtime_error)?;
            let responses = if requests.is_empty() {
                Vec::new()
            } else {
                self.fact_index
                    .read_fact_index_batch(&requests)
                    .await
                    .map_err(fact_index_runtime_error)?
            };
            plan.validate_query_results(&responses)
                .map_err(portfolio_state_runtime_error)?;
            let hydrated =
                selection::hydrate_holding_responses(plan, &responses, self.artifacts.as_ref())
                    .await?;
            let queries = plan
                .query_evidence(&responses, &hydrated)
                .map_err(portfolio_state_runtime_error)?;
            let primary = SelectHoldingsReadEvidence::new(&queries, hydrated)
                .map_err(portfolio_state_runtime_error)?;
            Ok(ExternalReadExecution::new(primary, queries))
        })
    }
}

struct AssembleSnapshotRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for AssembleSnapshotRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_runner_config_for_node::<AssembleSnapshotConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let input = load_materialized_struct_input::<AssembleSnapshotInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output = assemble_snapshot(config.as_ref(), input).map_err(|error| {
                mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
            })?;
            state_output(ctx, &output)
        })
    }
}

struct ProjectReportRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for ProjectReportRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let _config = load_runner_config_for_node::<ProjectReportConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let input = load_materialized_struct_input::<ProjectReportInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output = mfm_state_portfolio::project_report_from_snapshot(input.snapshot)
                .map_err(|error| {
                    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
                })?;
            state_output(ctx, &output)
        })
    }
}

fn state_output<T>(ctx: ErasedRunCtx<'_>, value: &T) -> mfm_runtime::Result<ErasedRunnerOutput>
where
    T: MfmValue,
{
    ErasedRunnerOutput::state_output(&ctx, value)
}

fn fact_index_runtime_error(
    error: mfm_fact_capabilities::FactIndexReadError,
) -> mfm_runtime::RuntimeError {
    match error {
        mfm_fact_capabilities::FactIndexReadError::Provider { .. } => {
            mfm_runtime::RuntimeError::Blocked("fact-index provider is unavailable".to_owned())
        }
        mfm_fact_capabilities::FactIndexReadError::InvalidRequest { .. } => {
            mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
        }
    }
}

fn portfolio_state_runtime_error(
    error: mfm_state_portfolio::PortfolioHoldingSelectionError,
) -> mfm_runtime::RuntimeError {
    let code = error.code.as_str();
    let failure = mfm_runtime::RuntimeFailure::new(
        events::ErrorCode::new(code).expect("portfolio error code is a checked public code"),
        events::ErrorCategory::Validation,
        format!("{code}: portfolio holding selection failed"),
        Vec::new(),
    )
    .expect("portfolio failure metadata is a checked public contract");
    mfm_runtime::RuntimeError::Failure(failure)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fact_index_provider_failures_block_without_terminalizing_the_attempt() {
        let provider = mfm_fact_capabilities::FactIndexReadError::redacted_provider_failure(
            "private backend detail",
        );
        assert_eq!(
            fact_index_runtime_error(provider),
            mfm_runtime::RuntimeError::Blocked("fact-index provider is unavailable".to_owned())
        );

        let invalid = mfm_fact_capabilities::FactIndexReadError::InvalidRequest {
            reason: mfm_fact_capabilities::FactIndexInvalidRequest::UnsupportedAudience,
        };
        assert!(matches!(
            fact_index_runtime_error(invalid),
            mfm_runtime::RuntimeError::InvalidRunnerOutput(_)
        ));
    }
}
