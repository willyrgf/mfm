#![warn(missing_docs)]
//! Portfolio live selection and hydration for certified portfolio snapshots.
//!
//! The store-owned fact-query binding rereads and verifies Bitcoin and generic EVM facts before
//! portfolio assembly. Family collection execution belongs to the family adapters.

use std::sync::Arc;

use mfm_events::v1 as events;
use mfm_facts::FactQueryReadCapability;
use mfm_portfolio::{
    portfolio_adapter_kind, portfolio_adapter_version, AssembleSnapshotState, ProjectReportState,
    SelectHoldingsReadEvidence, SelectHoldingsReadPlan, SelectHoldingsState,
};
use mfm_runtime::{
    register_pure_state, CapabilityImplementationId, ErasedRunCtx, ErasedRunnerRegistry,
    ExternalReadExecution, ExternalReadExecutionFuture, ExternalReadPlanExecutor,
    ExternalReadRunner, RunnerFactoryBinding, RunnerRegistrationBuilder,
};
use mfm_store::v1 as store;

mod selection;

const PURE_FACTORY: &str = "pure";
const READ_FACTORY: &str = "read_external";
const ADAPTER_FACTORY: &str = "portfolio_adapter";

/// Registers receipt-pinned selection and snapshot projection runners.
pub fn register_portfolio_live<S>(
    registry: &mut ErasedRunnerRegistry,
    store: Arc<S>,
    pure_factory: &RunnerFactoryBinding,
    read_factory: &RunnerFactoryBinding,
    adapter_factory: &RunnerFactoryBinding,
) -> mfm_runtime::Result<()>
where
    S: store::FactQueryStore + store::RetainedArtifactReadProvider + 'static,
{
    registry.register_capability_spec::<FactQueryReadCapability>(
        CapabilityImplementationId::new(store.fact_query_implementation_id())?,
    )?;
    let artifacts: Arc<dyn store::RetainedArtifactReadProvider> = store.clone();
    let mut registrations = RunnerRegistrationBuilder::new(registry);
    require_factory(pure_factory, PURE_FACTORY)?;
    require_factory(read_factory, READ_FACTORY)?;
    require_factory(adapter_factory, ADAPTER_FACTORY)?;
    registrations.register_adapter_executable_with_factory(
        portfolio_adapter_kind()?,
        portfolio_adapter_version()?,
        adapter_factory,
    )?;
    registrations.register_state_runner_with_factory::<SelectHoldingsState>(
        read_factory,
        Arc::new(ExternalReadRunner::<SelectHoldingsState, _>::new(
            artifacts.clone(),
            SelectHoldingsExecutor::new(store),
        )),
    )?;
    register_pure_state::<AssembleSnapshotState>(
        &mut registrations,
        pure_factory,
        artifacts.clone(),
        None,
    )?;
    register_pure_state::<ProjectReportState>(&mut registrations, pure_factory, artifacts, None)?;
    Ok(())
}

struct SelectHoldingsExecutor<S> {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    store: Arc<S>,
}

impl<S> SelectHoldingsExecutor<S>
where
    S: store::RetainedArtifactReadProvider + 'static,
{
    fn new(store: Arc<S>) -> Self {
        let artifacts: Arc<dyn store::RetainedArtifactReadProvider> = store.clone();
        Self { artifacts, store }
    }
}

impl<S> ExternalReadPlanExecutor<SelectHoldingsState> for SelectHoldingsExecutor<S>
where
    S: store::FactQueryStore + Send + Sync + 'static,
{
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
            let responses = self
                .store
                .execute_fact_queries(&requests)
                .await
                .map_err(fact_query_runtime_error)?;
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

fn require_factory(
    factory: &RunnerFactoryBinding,
    expected: &'static str,
) -> mfm_runtime::Result<()> {
    if factory.factory_id().as_str() != expected {
        return Err(mfm_runtime::RuntimeError::RunnerBinding(format!(
            "portfolio registration requires factory id {expected}"
        )));
    }
    Ok(())
}

fn fact_query_runtime_error<E>(_error: E) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::Blocked("fact-query store is unavailable".to_owned())
}

fn portfolio_state_runtime_error(
    error: mfm_portfolio::PortfolioHoldingSelectionError,
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
mod tests;
