use super::*;
use mfm_program::PureState;

const PURE_FACTORY_ID: &str = "pure";

struct PureStateRunner<S>
where
    S: PureState,
{
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    output_extractor: Option<Arc<dyn ContextOutputExtractor>>,
    _state: PhantomData<fn(S) -> S>,
}

impl<S> PureStateRunner<S>
where
    S: PureState,
{
    fn new(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        output_extractor: Option<Arc<dyn ContextOutputExtractor>>,
    ) -> Self {
        Self {
            artifacts,
            output_extractor,
            _state: PhantomData,
        }
    }
}

impl<S> ErasedNodeRunner for PureStateRunner<S>
where
    S: PureState,
    S::Config: DeserializeOwned,
    S::Input: DeserializeOwned,
{
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
            let output_value = state
                .run(input, &context)
                .map_err(|error| RuntimeError::InvalidRunnerOutput(error.to_string()))?;
            let mut output = RunnerOutputBuilder::new(&ctx);
            output.state_output(&output_value)?;
            Ok(output.finish())
        })
    }
}

/// Registers one ordinary pure state with retained-artifact and executable authority supplied by
/// the caller.
pub fn register_pure_state<S>(
    registrations: &mut RunnerRegistrationBuilder<'_>,
    factory: &RunnerFactoryBinding,
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    context_output: Option<Arc<dyn ContextOutputExtractor>>,
) -> Result<mfm_program::StateDescriptorIdentity>
where
    S: PureState,
    S::Config: DeserializeOwned,
    S::Input: DeserializeOwned,
{
    if factory.factory_id().as_str() != PURE_FACTORY_ID {
        return Err(RuntimeError::RunnerBinding(
            "pure state registration requires factory id pure".to_owned(),
        ));
    }
    registrations.register_state_runner_with_factory::<S>(
        factory,
        Arc::new(PureStateRunner::<S>::new(artifacts, context_output)),
    )
}
