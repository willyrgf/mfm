use std::sync::Arc;

use mfm_op_portfolio_snapshot::{
    assemble_portfolio_collection_receipt, AssemblePortfolioCollectionReceiptConfig,
    AssemblePortfolioCollectionReceiptInput, AssemblePortfolioCollectionReceiptState,
};
use mfm_runtime::{
    load_materialized_struct_input, load_runner_config_for_node, ErasedNodeRunner, ErasedRunCtx,
    ErasedRunnerFuture, ErasedRunnerOutput, ErasedRunnerRegistry, RunnerExecutableIdentityTemplate,
    RunnerRegistrationBuilder,
};
use mfm_store::v1 as store;

#[path = "portfolio_snapshot_replay.rs"]
mod replay;
pub(crate) use self::replay::verify_portfolio_collection_receipt_replay;

const PURE_FACTORY: &str = "pure";

pub(crate) fn register_portfolio_snapshot_runners(
    registry: &mut ErasedRunnerRegistry,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
) -> mfm_runtime::Result<()> {
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    let executable_identities = RunnerExecutableIdentityTemplate::new(
        "mfm-app",
        "portfolio-snapshot",
        env!("CARGO_PKG_VERSION"),
    )?;
    let pure_factory =
        executable_identities.factory_binding(mfm_events::v1::RunnerFactoryId::new(PURE_FACTORY)?);
    registrations.register_state_runner_with_factory::<AssemblePortfolioCollectionReceiptState>(
        &pure_factory,
        Arc::new(AssemblePortfolioCollectionReceiptRunner { artifacts }),
    )?;
    Ok(())
}

struct AssemblePortfolioCollectionReceiptRunner {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
}

impl ErasedNodeRunner for AssemblePortfolioCollectionReceiptRunner {
    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config = load_runner_config_for_node::<AssemblePortfolioCollectionReceiptConfig>(
                ctx.node(),
                self.artifacts.as_ref(),
            )
            .await?;
            let input = load_materialized_struct_input::<AssemblePortfolioCollectionReceiptInput>(
                ctx.inputs(),
                self.artifacts.as_ref(),
            )
            .await?;
            let output =
                assemble_portfolio_collection_receipt(config.as_ref(), input).map_err(|error| {
                    mfm_runtime::RuntimeError::InvalidRunnerOutput(error.to_string())
                })?;
            ErasedRunnerOutput::state_output(&ctx, &output)
        })
    }
}
