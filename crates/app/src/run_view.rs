use mfm_ids::{ContentRef, EffectId, ExecutionPosition, StatePosition};
use mfm_runtime::{RunView, RunViewState, RunnableReason};
use mfm_values::NativeCause;
use serde::Serialize;
use serde_json::value::RawValue;

/// Borrowed exact JSON serializer for one qualified durable run observation.
#[derive(Serialize)]
pub struct SerializableRunView<'a> {
    run_id: &'a mfm_ids::RunId,
    head_sequence: u64,
    head_digest: &'a mfm_ids::ContentDigest,
    state: State<'a>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Reason {
    Advance,
    Retry,
    Restart { checkpoint: StatePosition },
}
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum State<'a> {
    Runnable {
        position: &'a ExecutionPosition,
        reason: Reason,
    },
    EffectPending {
        position: &'a ExecutionPosition,
        effect_id: &'a EffectId,
        latest_failure: Option<PendingFailure<'a>>,
    },
    AwaitingRecovery {
        position: &'a ExecutionPosition,
        #[serde(flatten)]
        failure: Original<'a>,
    },
    AwaitingInterpretation {
        position: &'a ExecutionPosition,
        effect_id: &'a EffectId,
        input: Object<'a>,
        command: Object<'a>,
        evidence: Object<'a>,
    },
    Succeeded {
        #[serde(flatten)]
        value: Object<'a>,
    },
    Failed {
        value_ref: &'a ContentRef,
        report: &'a RawValue,
    },
}

#[derive(Serialize)]
struct PendingFailure<'a> {
    #[serde(flatten)]
    incident: Incident<'a>,
    decision: mfm_runtime::RecoveryDecision,
}

#[derive(Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
enum Incident<'a> {
    Read {
        error: Object<'a>,
        input: Object<'a>,
        intent: Object<'a>,
    },
    Effect {
        error: Object<'a>,
        input: Object<'a>,
        command: Object<'a>,
        effect_id: &'a EffectId,
    },
}
impl<'a> Incident<'a> {
    fn new(
        incident: &'a mfm_runtime::AdapterIncidentView,
    ) -> Result<Self, mfm_values::NativeCause> {
        use mfm_runtime::AdapterIncidentView;
        Ok(match incident {
            AdapterIncidentView::Read {
                error,
                input,
                intent,
            } => Self::Read {
                error: Object::new(error)?,
                input: Object::new(input)?,
                intent: Object::new(intent)?,
            },
            AdapterIncidentView::Effect {
                error,
                input,
                command,
                effect_id,
            } => Self::Effect {
                error: Object::new(error)?,
                input: Object::new(input)?,
                command: Object::new(command)?,
                effect_id,
            },
        })
    }
}

#[derive(Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
enum Original<'a> {
    Pure {
        original: Object<'a>,
        input: Object<'a>,
    },
    Read {
        original: Object<'a>,
        input: Object<'a>,
        intent: Object<'a>,
        #[serde(skip_serializing_if = "Option::is_none")]
        evidence: Option<Object<'a>>,
    },
    Effect {
        original: Object<'a>,
        input: Object<'a>,
        command: Object<'a>,
        effect_id: &'a EffectId,
        #[serde(skip_serializing_if = "Option::is_none")]
        evidence: Option<Object<'a>>,
    },
}
impl<'a> Original<'a> {
    fn new(failure: &'a mfm_runtime::Failure) -> Result<Self, mfm_values::NativeCause> {
        use mfm_runtime::{Failure, StateCall};
        let original = Object::new(failure.original())?;
        let input = Object::new(failure.call().input())?;
        Ok(match failure {
            Failure::Domain(failure) => match failure.call() {
                StateCall::Pure(_) => Self::Pure { original, input },
                StateCall::Read {
                    intent, evidence, ..
                } => Self::Read {
                    original,
                    input,
                    intent: Object::new(intent)?,
                    evidence: Some(Object::new(evidence)?),
                },
                StateCall::Effect(settlement) => Self::Effect {
                    original,
                    input,
                    command: Object::new(settlement.effect().command())?,
                    effect_id: settlement.effect().effect_id(),
                    evidence: Some(Object::new(settlement.evidence())?),
                },
            },
            Failure::Read(failure) => Self::Read {
                original,
                input,
                intent: Object::new(failure.intent())?,
                evidence: None,
            },
            Failure::PendingEffect { effect, .. } => Self::Effect {
                original,
                input,
                command: Object::new(effect.command())?,
                effect_id: effect.effect_id(),
                evidence: None,
            },
        })
    }
}

#[derive(Serialize)]
pub(super) struct Object<'a> {
    contract_ref: ContentRef,
    value_ref: &'a ContentRef,
    value: &'a RawValue,
}
impl<'a> Object<'a> {
    pub(super) fn new(value: &'a mfm_runtime::Object) -> Result<Self, mfm_values::NativeCause> {
        Ok(Self {
            contract_ref: value
                .contract_ref()
                .map_err(mfm_values::NativeCause::from_error)?,
            value_ref: value.value_ref(),
            value: serde_json::from_slice(value.canonical_bytes()).map_err(|source| {
                mfm_values::NativeCause::from_error(mfm_canonical::JsonError::new(source))
            })?,
        })
    }
}
impl<'a> SerializableRunView<'a> {
    /// Prepares exact retained fields, preserving native projection failures for the caller.
    pub fn new(view: &'a RunView) -> Result<Self, NativeCause> {
        let state = match view.state() {
            RunViewState::Runnable { position, reason } => State::Runnable {
                position,
                reason: match reason {
                    RunnableReason::Advance => Reason::Advance,
                    RunnableReason::Retry => Reason::Retry,
                    RunnableReason::Restart { checkpoint } => Reason::Restart {
                        checkpoint: *checkpoint,
                    },
                },
            },
            RunViewState::EffectPending {
                position,
                effect_id,
                latest_failure,
            } => State::EffectPending {
                position,
                effect_id,
                latest_failure: latest_failure
                    .as_ref()
                    .map(|failure| {
                        Ok::<_, mfm_values::NativeCause>(PendingFailure {
                            incident: Incident::new(&failure.incident)?,
                            decision: failure.decision,
                        })
                    })
                    .transpose()?,
            },
            RunViewState::AwaitingRecovery { failure } => State::AwaitingRecovery {
                position: failure.call().position(),
                failure: Original::new(failure)?,
            },
            RunViewState::AwaitingInterpretation { settlement } => State::AwaitingInterpretation {
                position: settlement.effect().call().position(),
                effect_id: settlement.effect().effect_id(),
                input: Object::new(settlement.effect().call().input())?,
                command: Object::new(settlement.effect().command())?,
                evidence: Object::new(settlement.evidence())?,
            },
            RunViewState::Succeeded(value) => State::Succeeded {
                value: Object::new(value)?,
            },
            RunViewState::Failed(report) => State::Failed {
                value_ref: report.value_ref(),
                report: serde_json::from_slice(report.canonical_bytes()).map_err(|source| {
                    NativeCause::from_error(mfm_canonical::JsonError::new(source))
                })?,
            },
        };
        Ok(Self {
            run_id: view.run_id(),
            head_sequence: view.head_sequence(),
            head_digest: view.head_digest(),
            state,
        })
    }
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum InvocationWire<'a> {
    ExecutionStopped {
        run_id: &'a mfm_ids::RunId,
        last_observed: Option<SerializableRunView<'a>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        size_limit: Option<mfm_runtime::SizeViolation>,
        cause: Box<RawValue>,
    },
    RecoveryStopped {
        observed: SerializableRunView<'a>,
        reason: &'static str,
        #[serde(flatten)]
        incident: Box<Incident<'a>>,
    },
}
#[derive(Serialize)]
#[serde(transparent)]
pub(super) struct Invocation<'a>(InvocationWire<'a>);
impl<'a> Invocation<'a> {
    pub(super) fn new(failure: &'a mfm_runtime::InvocationFailure) -> Result<Self, NativeCause> {
        Ok(Self(match failure {
            mfm_runtime::InvocationFailure::Execution {
                run_id,
                last_observed,
                error,
            } => InvocationWire::ExecutionStopped {
                run_id,
                last_observed: last_observed
                    .as_ref()
                    .map(SerializableRunView::new)
                    .transpose()?,
                size_limit: error.size_limit(),
                cause: error.project()?,
            },
            mfm_runtime::InvocationFailure::RecoveryStopped {
                observed,
                incident,
                reason,
            } => InvocationWire::RecoveryStopped {
                observed: SerializableRunView::new(observed)?,
                reason: stop_reason(*reason),
                incident: Box::new(Incident::new(incident)?),
            },
        }))
    }
}

fn stop_reason(reason: mfm_program::StopReason) -> &'static str {
    use mfm_program::{RecoveryDenial, RecoveryLimit, StopReason};
    match reason {
        StopReason::Requested => "requested",
        StopReason::Exhausted(RecoveryLimit::StateRetry) => "state_retry_exhausted",
        StopReason::Exhausted(RecoveryLimit::StateRestart) => "state_restart_exhausted",
        StopReason::Exhausted(RecoveryLimit::Run) => "run_exhausted",
        StopReason::Disallowed(RecoveryDenial::PureRetry) => "pure_retry",
        StopReason::Disallowed(RecoveryDenial::CheckpointUnavailable) => "checkpoint_unavailable",
        StopReason::Disallowed(RecoveryDenial::EffectBarrier) => "effect_barrier",
        StopReason::Disallowed(RecoveryDenial::EffectSettled) => "effect_settled",
    }
}
