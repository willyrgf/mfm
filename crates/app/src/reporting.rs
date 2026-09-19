use crate::{ClientErrorDetail, RunRequestError, SerializableClientError};
use mfm_runtime::{InvocationFailure, RecordingFailure, RunView, RuntimeError};
use mfm_values::InvocationDiagnostic;
use serde::{ser::SerializeMap, Serialize};
use serde_json::value::RawValue;

/// Encodes prepared transport fields once, retaining the concrete JSON error on failure.
pub fn encode_response(value: &impl Serialize) -> Result<Box<RawValue>, mfm_canonical::JsonError> {
    serde_json::value::to_raw_value(value).map_err(mfm_canonical::JsonError::new)
}

impl<'a> SerializableClientError<'a> {
    /// Borrows known observation facts after its normal presentation failed.
    pub fn failed_view_report(
        view: &'a RunView,
        diagnostic: &'a InvocationDiagnostic,
        original_report: Option<&'a RawValue>,
    ) -> Self {
        Self {
            code: "report_render_failed",
            message: "run observation could not be rendered completely",
            detail: ClientErrorDetail::FailedView {
                view,
                original_report,
            },
            diagnostic: Some(diagnostic),
        }
    }

    /// Borrows a run error's primary disposition and known facts after reporting failed.
    pub fn failed_run_report(
        error: &'a RunRequestError,
        message: &'a str,
        diagnostic: &'a InvocationDiagnostic,
        original_report: Option<&'a RawValue>,
    ) -> Self {
        Self {
            code: error.code(),
            message,
            detail: ClientErrorDetail::FailedRun {
                error,
                original_report,
            },
            diagnostic: Some(diagnostic),
        }
    }

    /// Presents an ordinary error with its already supplied invocation diagnostic.
    pub fn with_diagnostic(mut self, diagnostic: &'a InvocationDiagnostic) -> Self {
        self.diagnostic = Some(diagnostic);
        self
    }
}

#[derive(Serialize)]
struct ObservedHead<'a> {
    run_id: &'a mfm_ids::RunId,
    head_sequence: u64,
    head_digest: &'a mfm_ids::ContentDigest,
}
impl<'a> From<&'a RunView> for ObservedHead<'a> {
    fn from(view: &'a RunView) -> Self {
        Self {
            run_id: view.run_id(),
            head_sequence: view.head_sequence(),
            head_digest: view.head_digest(),
        }
    }
}

pub(super) fn view_fields<S: SerializeMap>(state: &mut S, view: &RunView) -> Result<(), S::Error> {
    state.serialize_entry("run_id", view.run_id())?;
    state.serialize_entry("last_observed", &ObservedHead::from(view))
}

pub(super) fn run_fields<S: SerializeMap>(
    state: &mut S,
    error: &RunRequestError,
) -> Result<(), S::Error> {
    if let Some(recovery) = error.recovery() {
        state.serialize_entry("recovery", recovery)?;
    }
    let invocation = match error {
        RunRequestError::Construction { run_id, cause } => {
            state.serialize_entry("run_id", run_id)?;
            state.serialize_entry("diagnostic", cause)?;
            return Ok(());
        }
        RunRequestError::Request(_) => return Ok(()),
        RunRequestError::Invocation(invocation)
        | RunRequestError::AppendIndeterminate { invocation, .. } => invocation,
    };
    let runtime = match invocation {
        InvocationFailure::RecoveryStopped { observed } => return view_fields(state, observed),
        InvocationFailure::Execution {
            run_id,
            error,
            last_observed,
        } => {
            state.serialize_entry("run_id", run_id)?;
            if let Some(view) = last_observed {
                state.serialize_entry("last_observed", &ObservedHead::from(view))?;
            }
            error
        }
    };
    if let Some(size) = runtime.size_limit() {
        state.serialize_entry("size_limit", &size)?;
    }
    match runtime {
        RuntimeError::Projection { acknowledged, .. } => {
            state.serialize_entry("acknowledged", acknowledged)?
        }
        RuntimeError::Recording { failure, .. } => {
            let candidate = match failure.as_ref() {
                RecordingFailure::BeforeAppend { .. } => return Ok(()),
                RecordingFailure::Store { candidate, .. } => candidate,
                RecordingFailure::NotInserted {
                    candidate,
                    observation,
                    ..
                } => {
                    state.serialize_entry("observation", observation)?;
                    candidate
                }
            };
            #[derive(Serialize)]
            struct Candidate<'a> {
                run_id: &'a mfm_ids::RunId,
                sequence: u64,
                digest: &'a mfm_ids::ContentDigest,
            }
            state.serialize_entry(
                "candidate",
                &Candidate {
                    run_id: candidate.run_id(),
                    sequence: candidate.run_sequence(),
                    digest: candidate.head_digest(),
                },
            )?;
        }
        _ => {}
    }
    Ok(())
}
