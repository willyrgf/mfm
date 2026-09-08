use mfm_ids::{ContentRef, EffectId, ExecutionPosition, StatePosition};
use mfm_runtime::{RunView, RunViewState, RunnableReason, ValueView};
use serde::{Serialize, Serializer};
use serde_json::value::RawValue;

/// Borrowed exact JSON serializer for one qualified durable run observation.
pub struct SerializableRunView<'a> {
    view: &'a RunView,
}
impl<'a> SerializableRunView<'a> {
    /// Wraps a view without changing its retained canonical values.
    pub const fn new(view: &'a RunView) -> Self {
        Self { view }
    }
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
pub(super) struct Object<'a> {
    contract_ref: &'a ContentRef,
    value_ref: &'a ContentRef,
    value: &'a RawValue,
}
impl<'a> Object<'a> {
    pub(super) fn new(value: &'a ValueView) -> Result<Self, serde_json::Error> {
        Ok(Self {
            contract_ref: value.contract_ref(),
            value_ref: value.value_ref(),
            value: serde_json::from_slice(value.canonical_bytes())?,
        })
    }
}
impl Serialize for SerializableRunView<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error;
        #[derive(Serialize)]
        struct View<'a> {
            run_id: &'a mfm_ids::RunId,
            head_sequence: u64,
            head_digest: &'a mfm_ids::ContentDigest,
            state: State<'a>,
        }
        let state = match self.view.state() {
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
            } => State::EffectPending {
                position,
                effect_id,
            },
            RunViewState::Succeeded(value) => State::Succeeded {
                value: Object::new(value).map_err(S::Error::custom)?,
            },
            RunViewState::Failed(report) => State::Failed {
                value_ref: report.value_ref(),
                report: serde_json::from_slice(report.canonical_bytes())
                    .map_err(S::Error::custom)?,
            },
        };
        View {
            run_id: self.view.run_id(),
            head_sequence: self.view.head_sequence(),
            head_digest: self.view.head_digest(),
            state,
        }
        .serialize(serializer)
    }
}

pub(super) struct Invocation<'a>(pub(super) &'a mfm_runtime::InvocationFailure);
impl Serialize for Invocation<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error;
        #[derive(Serialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        enum Wire<'a> {
            ExecutionStopped {
                run_id: &'a mfm_ids::RunId,
                last_observed: Option<SerializableRunView<'a>>,
            },
            RecoveryStopped {
                observed: SerializableRunView<'a>,
                reason: &'static str,
                error: Object<'a>,
                state_context: Object<'a>,
            },
        }
        match self.0 {
            mfm_runtime::InvocationFailure::Execution {
                run_id,
                last_observed,
                ..
            } => Wire::ExecutionStopped {
                run_id,
                last_observed: last_observed.as_ref().map(SerializableRunView::new),
            },
            mfm_runtime::InvocationFailure::RecoveryStopped {
                observed,
                incident,
                reason,
            } => Wire::RecoveryStopped {
                observed: SerializableRunView::new(observed),
                reason: stop_reason(*reason),
                error: Object::new(&incident.error).map_err(S::Error::custom)?,
                state_context: Object::new(&incident.state_context).map_err(S::Error::custom)?,
            },
        }
        .serialize(serializer)
    }
}

fn stop_reason(reason: mfm_program::StopReason) -> &'static str {
    use mfm_program::{RecoveryDenial, RecoveryLimit, StopReason};
    match reason {
        StopReason::Nonrecoverable => "nonrecoverable",
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
