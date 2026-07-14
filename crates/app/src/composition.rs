use std::sync::Arc;

use mfm_op_portfolio_collect_report::{
    collect_then_report_readiness, CollectThenReportReadinessInput, CollectThenReportReadinessState,
};
use mfm_runtime::{
    load_materialized_struct_input, load_runner_config_for_node, ErasedNodeRunner, ErasedRunCtx,
    ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry, RunnerExecutableIdentityTemplate,
    RunnerRegistrationBuilder,
};
use mfm_state_portfolio::PortfolioInputsReadyConfig;
use mfm_store::v1 as store;

const PURE_FACTORY: &str = "pure";

pub(crate) fn register_collect_then_report_runners(
    registry: &mut ErasedRunnerRegistry,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
) -> mfm_runtime::Result<()> {
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    let executable_identities = RunnerExecutableIdentityTemplate::new(
        "mfm-app",
        "portfolio-collect-report",
        env!("CARGO_PKG_VERSION"),
    )?;
    let pure_factory =
        executable_identities.factory_binding(mfm_events::v1::RunnerFactoryId::new(PURE_FACTORY)?);
    registrations.register_state_runner_with_factory::<CollectThenReportReadinessState>(
        &pure_factory,
        Arc::new(CollectThenReportReadinessRunner { artifacts }),
    )?;
    Ok(())
}

struct CollectThenReportReadinessRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for CollectThenReportReadinessRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_runner_config_for_node::<PortfolioInputsReadyConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let input = load_materialized_struct_input::<CollectThenReportReadinessInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output =
                collect_then_report_readiness(config.as_ref(), input).map_err(|error| {
                    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
                })?;
            ErasedRunnerOutput::state_output(&ctx, &output)
        })
    }
}
