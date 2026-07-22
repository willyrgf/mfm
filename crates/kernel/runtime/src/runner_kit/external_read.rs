use super::*;

/// Boxed future returned by an adapter that executes one state-authored external-read plan.
pub type ExternalReadExecutionFuture<'a, E> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<ExternalReadExecution<E>>> + Send + 'a>,
>;

/// Canonical live evidence returned by an external-read plan executor.
pub struct ExternalReadExecution<E> {
    primary: E,
    fact_queries: Vec<mfm_facts::FactQueryEvidence>,
}

impl<E> ExternalReadExecution<E> {
    /// Creates an execution result with one primary evidence value and no fact queries.
    pub fn primary(primary: E) -> Self {
        Self {
            primary,
            fact_queries: Vec::new(),
        }
    }

    /// Creates an execution result with primary and auxiliary fact-query evidence.
    pub fn new(primary: E, fact_queries: Vec<mfm_facts::FactQueryEvidence>) -> Self {
        Self {
            primary,
            fact_queries,
        }
    }

    fn into_evidence_set(self) -> mfm_program::ExternalReadEvidenceSet<E> {
        mfm_program::ExternalReadEvidenceSet::new(self.primary, self.fact_queries)
    }
}

/// Adapter-owned live executor for one typed external-read plan.
///
/// The executor owns only capability binding and plan execution. Config/input/context loading,
/// deterministic reduction, evidence retention, and output recording remain in the generic
/// runner below.
pub trait ExternalReadPlanExecutor<S>: Send + Sync
where
    S: ReadState,
{
    /// Asynchronously validates process-local resources before the run is admitted.
    ///
    /// Implementations must offload blocking resource discovery rather than blocking the async
    /// runtime worker that polls this future.
    fn validate_ingress<'a>(
        &'a self,
        ctx: RunnerIngressContext<'a>,
        state: &'a S,
    ) -> crate::RunnerIngressFuture<'a>;

    /// Executes a deterministic plan and returns canonical evidence only.
    fn execute<'a>(
        &'a self,
        plan: &'a S::Plan,
        ctx: &'a ErasedRunCtx<'_>,
    ) -> ExternalReadExecutionFuture<'a, S::Evidence>;
}

/// Generic erased runner shared by every external-read state.
pub struct ExternalReadRunner<S, E>
where
    S: ReadState,
    E: ExternalReadPlanExecutor<S>,
{
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    executor: E,
    output_extractor: Option<Arc<dyn ContextOutputExtractor>>,
    _state: PhantomData<fn(S) -> S>,
}

impl<S, E> ExternalReadRunner<S, E>
where
    S: ReadState,
    E: ExternalReadPlanExecutor<S>,
{
    /// Creates a generic runner from retained-artifact authority and a live plan executor.
    pub fn new(artifacts: Arc<dyn store::RetainedArtifactReadProvider>, executor: E) -> Self {
        Self {
            artifacts,
            executor,
            output_extractor: None,
            _state: PhantomData,
        }
    }

    /// Adds type-aware validation for a context-bound state output.
    pub fn with_context_output_extractor(
        mut self,
        extractor: Arc<dyn ContextOutputExtractor>,
    ) -> Self {
        self.output_extractor = Some(extractor);
        self
    }
}

impl<S, E> ErasedNodeRunner for ExternalReadRunner<S, E>
where
    S: ReadState,
    S::Config: DeserializeOwned,
    S::Input: DeserializeOwned,
    S::Facts: StageReadFactBatch,
    E: ExternalReadPlanExecutor<S> + 'static,
{
    fn validate_ingress<'a>(
        &'a self,
        ctx: RunnerIngressContext<'a>,
    ) -> crate::RunnerIngressFuture<'a> {
        Box::pin(async move {
            let config = load_launch_config_for_node::<S::Config>(&ctx, ctx.node())?;
            let state =
                S::new(config).map_err(|error| RuntimeError::RunnerBinding(error.to_string()))?;
            self.executor.validate_ingress(ctx, &state).await
        })
    }

    fn context_output_extractor(&self) -> Option<&dyn ContextOutputExtractor> {
        self.output_extractor.as_deref()
    }

    fn run_erased<'a>(&'a self, ctx: ErasedRunCtx<'a>) -> crate::ErasedRunnerFuture<'a> {
        Box::pin(async move {
            let config =
                load_runner_config_for_node::<S::Config>(ctx.node(), self.artifacts.as_ref())
                    .await?;
            let state = S::new(config)
                .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
            let input =
                load_materialized_input::<S::Input>(ctx.inputs(), self.artifacts.as_ref()).await?;
            let context = ctx.certified_context::<S::Context>()?;
            let plan = state
                .plan(&input, &context)
                .map_err(state_execution_error)?;
            let evidence = self
                .executor
                .execute(&plan, &ctx)
                .await?
                .into_evidence_set();
            let (output_value, facts) = state
                .reduce(&input, &evidence, &context)
                .map_err(state_execution_error)?;

            let mut output = RunnerOutputBuilder::new(&ctx);
            output.record_external_read_evidence(evidence.primary_evidence())?;
            for query in evidence.fact_query_evidence() {
                output.record_fact_query_evidence(query.clone())?;
            }
            facts.stage(&mut output)?;
            output.state_output(&output_value)?;
            Ok(output.finish())
        })
    }
}

trait StageReadFactBatch {
    fn stage(self, output: &mut RunnerOutputBuilder<'_, '_>) -> Result<()>;
}

impl StageReadFactBatch for () {
    fn stage(self, _output: &mut RunnerOutputBuilder<'_, '_>) -> Result<()> {
        Ok(())
    }
}

impl<F> StageReadFactBatch for mfm_values::NonEmpty<F>
where
    F: mfm_facts::MfmFactType,
{
    fn stage(self, output: &mut RunnerOutputBuilder<'_, '_>) -> Result<()> {
        for fact in self.values() {
            output.record_read_fact(fact)?;
        }
        Ok(())
    }
}

fn state_execution_error(error: mfm_program::StateError) -> RuntimeError {
    RuntimeError::InvalidRunnerOutput(error.to_string())
}
