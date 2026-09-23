//! Monomorphized State calls without run or transition authority.

use super::{decode, encode, encode_failure, task_failure, CallbackFailure};
use crate::{
    Classification, ClassifyError, EffectState, ProposedStateOutcome, PureState, ReadState,
};
use mfm_capabilities::{
    EffectCapabilityContract, EffectImplementation, ReadCapabilityContract, ReadImplementation,
};
use mfm_ids::{ContentRef, EffectId, ExecutionPosition};
use mfm_values::{InvocationDiagnostic, MfmValue, Object};
use std::{future::Future, pin::Pin, sync::Arc};

/// One immediately awaited invocation, with no retained mutation authority.
pub type Job<T, E = CallbackFailure> = Pin<Box<dyn Future<Output = Result<T, E>> + Send>>;
/// An encoded State outcome. Failure is still unclassified.
pub type Outcome = ProposedStateOutcome<Object, Object>;
/// Exact original decoding and classification, invoked after acknowledgement by Runtime.
pub type Classifier = fn(Object) -> Job<Classification>;
/// Deterministic preparation of a canonical intent or command.
pub type Prepare = fn(Object) -> Job<Object>;
/// Evaluation with the exact original contract captured at construction.
pub type Evaluate = Arc<dyn Fn(Object, ExecutionPosition) -> Job<Outcome> + Send + Sync>;
/// Read completion preserves binding versus State interpretation provenance.
pub type ReadComplete = Arc<
    dyn Fn(Object, Object, Object, ExecutionPosition) -> Job<Outcome, ReadCompletionFailure>
        + Send
        + Sync,
>;
/// Internal completion boundary; Runtime projects its operation without replacing the cause.
#[derive(Debug)]
pub enum ReadCompletionFailure {
    /// Native evidence decoding, projection or semantic binding failed.
    Bind(CallbackFailure),
    /// State input decoding, interpretation or outcome encoding failed.
    Interpret(CallbackFailure),
}
/// Effect interpretation retains the acknowledged command and Effect identity for projection.
pub type EffectInterpret =
    Arc<dyn Fn(Object, EffectId, Object, Object, ExecutionPosition) -> Job<Outcome> + Send + Sync>;
/// Effect evidence binding uses Runtime's retained Effect identity.
pub type EffectBind = Arc<dyn Fn(EffectId, Object, Object) -> Job<()> + Send + Sync>;
/// Validates selected native extraction before Runtime can acknowledge a command.
pub type ValidateCommand = Arc<dyn Fn(Object) -> Job<()> + Send + Sync>;

/// Pure invocation functions; no execution or classification occurs during construction.
#[derive(Clone)]
pub struct PureCallbacks {
    /// Evaluate and encode exactly one proposed outcome.
    pub evaluate: Evaluate,
    /// Classify an acknowledged State original.
    pub classify: Classifier,
}
/// Read invocation functions separated by operation provenance.
#[derive(Clone)]
pub struct ReadCallbacks {
    /// Prepare an exact intent.
    pub prepare: Prepare,
    /// Bind native evidence once, interpret the typed result and encode its outcome.
    pub complete: ReadComplete,
    /// Classify an acknowledged State original.
    pub classify: Classifier,
    /// Classify an acknowledged adapter original.
    pub classify_operational: Classifier,
}
/// Effect invocation functions with no prepare, settlement or recovery authority.
#[derive(Clone)]
pub struct EffectCallbacks {
    /// Reject local native command mismatches before acknowledgement.
    pub validate_command: ValidateCommand,
    /// Prepare an exact command.
    pub prepare: Prepare,
    /// Check evidence against the acknowledged command and Effect identity.
    pub bind: EffectBind,
    /// Interpret acknowledged evidence and encode its outcome.
    pub interpret: EffectInterpret,
    /// Classify an acknowledged State original.
    pub classify: Classifier,
    /// Classify an acknowledged adapter original.
    pub classify_operational: Classifier,
}

pub(super) async fn execute<T: Send + 'static>(
    job: impl FnOnce() -> Result<T, InvocationDiagnostic> + Send + 'static,
) -> Result<T, CallbackFailure> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|cause| CallbackFailure::Execute(task_failure("execute", cause)))?
        .map_err(CallbackFailure::Execute)
}

async fn outcome<O: MfmValue, F: MfmValue>(
    proposed: ProposedStateOutcome<O, F>,
    position: ExecutionPosition,
    contract: &ContentRef,
) -> Result<Outcome, CallbackFailure> {
    Ok(match proposed {
        ProposedStateOutcome::Success { output } => ProposedStateOutcome::Success {
            output: encode(output).await?,
        },
        ProposedStateOutcome::Failure { failure } => ProposedStateOutcome::Failure {
            failure: encode_failure(failure, position, contract).await?,
        },
    })
}

fn classify<E: ClassifyError>(original: Object) -> Job<Classification> {
    Box::pin(async move {
        let original = decode::<E>(original).await?;
        execute(move || Ok(original.classify())).await
    })
}

impl PureCallbacks {
    /// Captures the declared original contract for the exact Pure State.
    pub fn new<S: PureState>(failure_contract: ContentRef) -> Self {
        Self {
            evaluate: Arc::new(move |input, position| {
                let contract = failure_contract.clone();
                Box::pin(async move {
                    let input = decode::<S::Input>(input).await?;
                    let proposed = execute(move || S::evaluate(input)).await?;
                    outcome(proposed, position, &contract).await
                })
            }),
            classify: classify::<S::Failure>,
        }
    }
}

impl ReadCallbacks {
    /// Captures the exact State/native ABI and immutable public binding without invoking either.
    pub fn new<S, C, I>(
        failure_contract: ContentRef,
        implementation: ContentRef,
        binding_ref: ContentRef,
        binding: Arc<I::Binding>,
    ) -> Self
    where
        S: ReadState<C>,
        C: ReadCapabilityContract,
        I: ReadImplementation<C>,
        I::OperationalError: ClassifyError,
    {
        Self {
            prepare: |input| {
                Box::pin(async move {
                    let input = decode::<S::Input>(input).await?;
                    encode(execute(move || S::prepare(&input)).await?).await
                })
            },
            complete: Arc::new(move |input, intent, evidence, position| {
                let contract = failure_contract.clone();
                let implementation = implementation.clone();
                let binding_ref = binding_ref.clone();
                let binding = Arc::clone(&binding);
                Box::pin(async move {
                    let evidence = super::native::read_evidence::<C, I>(
                        implementation,
                        binding_ref,
                        binding,
                        intent,
                        evidence,
                    )
                    .await
                    .map_err(ReadCompletionFailure::Bind)?;
                    let input = decode::<S::Input>(input)
                        .await
                        .map_err(ReadCompletionFailure::Interpret)?;
                    let proposed = execute(move || S::interpret(input, &evidence))
                        .await
                        .map_err(ReadCompletionFailure::Interpret)?;
                    outcome(proposed, position, &contract)
                        .await
                        .map_err(ReadCompletionFailure::Interpret)
                })
            }),
            classify: classify::<S::Failure>,
            classify_operational: classify::<I::OperationalError>,
        }
    }
}

impl EffectCallbacks {
    /// Captures exact native extraction and mandatory projection before settlement admission.
    pub fn new<S, C, I>(
        failure_contract: ContentRef,
        implementation: ContentRef,
        binding_ref: ContentRef,
        binding: Arc<I::Binding>,
    ) -> Self
    where
        S: EffectState<C>,
        C: EffectCapabilityContract,
        I: EffectImplementation<C>,
        I::OperationalError: ClassifyError,
    {
        let validate_implementation = implementation.clone();
        let validate_ref = binding_ref.clone();
        let validate_value = Arc::clone(&binding);
        let bind_implementation = implementation.clone();
        let bind_ref = binding_ref.clone();
        let bind_value = Arc::clone(&binding);
        Self {
            prepare: |input| {
                Box::pin(async move {
                    let input = decode::<S::Input>(input).await?;
                    encode(execute(move || S::prepare(&input)).await?).await
                })
            },
            validate_command: Arc::new(move |command| {
                let implementation = validate_implementation.clone();
                let binding_ref = validate_ref.clone();
                let binding = Arc::clone(&validate_value);
                Box::pin(async move {
                    super::native::effect_command::<C, I>(
                        implementation,
                        binding_ref,
                        binding,
                        command,
                    )
                    .await?;
                    Ok(())
                })
            }),
            bind: Arc::new(move |effect_id, command, evidence| {
                let implementation = bind_implementation.clone();
                let binding_ref = bind_ref.clone();
                let binding = Arc::clone(&bind_value);
                Box::pin(async move {
                    super::native::effect_evidence::<C, I>(
                        implementation,
                        binding_ref,
                        binding,
                        effect_id,
                        command,
                        evidence,
                    )
                    .await?;
                    Ok(())
                })
            }),
            interpret: Arc::new(move |input, effect_id, command, evidence, position| {
                let contract = failure_contract.clone();
                let implementation = implementation.clone();
                let binding_ref = binding_ref.clone();
                let binding = Arc::clone(&binding);
                Box::pin(async move {
                    let input = decode::<S::Input>(input).await?;
                    let evidence = super::native::effect_evidence::<C, I>(
                        implementation,
                        binding_ref,
                        binding,
                        effect_id,
                        command,
                        evidence,
                    )
                    .await?;
                    let proposed = execute(move || S::interpret(input, &evidence)).await?;
                    outcome(proposed, position, &contract).await
                })
            }),
            classify: classify::<S::Failure>,
            classify_operational: classify::<I::OperationalError>,
        }
    }
}
