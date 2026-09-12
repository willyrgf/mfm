use crate::{RunRecovery, RunRequestError, StartRunResult};
use mfm_ids::{ContentDigest, RunId};
use mfm_runtime::{InvocationFailure, RunView, RuntimeError, SizeViolation};
use mfm_values::NativeCause;
use serde::Serialize;
use serde_json::value::RawValue;

/// Encodes prepared transport fields without adding a response-size quota.
/// Native causes must already have been projected by their bounded owning boundary.
pub fn encode_response(value: &impl Serialize) -> Result<Vec<u8>, NativeCause> {
    serde_json::to_vec(value)
        .map_err(|source| NativeCause::from_error(mfm_canonical::JsonError::new(source)))
}

/// Stage at which preparation of a transport report failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportStage {
    /// Preparing reviewed fields from the retained result.
    Prepare,
    /// Encoding already prepared fields as JSON.
    Encode,
    /// A transport observed failure while writing or flushing the prepared report.
    Deliver,
}

/// A failed report retaining its existing result and separate reporting causes.
///
/// This is invocation-local custody, not a durable audit or delivery acknowledgement.
/// Transports retain it until their observable write or response handoff ends.
pub struct ReportFailure<T> {
    original: T,
    stage: ReportStage,
    cause: NativeCause,
    projection_failure: Option<NativeCause>,
}
impl<T> ReportFailure<T> {
    /// Retains the exact existing result when its report cannot be prepared or encoded.
    pub fn new(original: T, stage: ReportStage, cause: NativeCause) -> Self {
        Self {
            original,
            stage,
            cause,
            projection_failure: None,
        }
    }
    /// Borrows the complete existing result without reconstructing it from public fields.
    pub fn original(&self) -> &T {
        &self.original
    }
    /// Returns the observed reporting stage.
    pub fn stage(&self) -> ReportStage {
        self.stage
    }
    /// Returns the initial reporting cause.
    pub fn cause(&self) -> &NativeCause {
        &self.cause
    }
    /// Returns a separate failure of the single secondary projection attempt, when one occurred.
    pub fn projection_failure(&self) -> Option<&NativeCause> {
        self.projection_failure.as_ref()
    }
    /// Returns actual size evidence for the reporting cause, when available.
    pub fn size_limit(&self) -> Option<SizeViolation> {
        size_limit(&self.cause)
    }

    fn detail(&mut self) -> ReportDetail {
        let reason = match self.stage {
            ReportStage::Prepare => "projection_failed",
            ReportStage::Encode => "encoding_failed",
            ReportStage::Deliver => "delivery_failed",
        };
        let mut omissions = vec![omission("original_detail", &self.cause, reason)];
        let cause = if self.projection_failure.is_some() {
            None
        } else {
            match self.cause.project() {
                Ok(projected) => Some(projected),
                Err(error) => {
                    self.projection_failure = Some(error);
                    None
                }
            }
        };
        if let Some(error) = &self.projection_failure {
            omissions.push(omission("report_failure.cause", error, "projection_failed"));
        }
        ReportDetail {
            stage: self.stage,
            cause,
            omissions,
        }
    }
}
impl<T> std::fmt::Debug for ReportFailure<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReportFailure")
            .field("stage", &self.stage)
            .finish_non_exhaustive()
    }
}
impl<T> std::fmt::Display for ReportFailure<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("reporting failed")
    }
}
impl<T> std::error::Error for ReportFailure<T> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

fn size_limit(cause: &NativeCause) -> Option<SizeViolation> {
    RuntimeError::Native {
        operation: mfm_runtime::Operation::Project,
        stage: mfm_runtime::Stage::Encode,
        cause: cause.clone(),
    }
    .size_limit()
}

#[derive(Serialize)]
struct Omission {
    field: &'static str,
    reason: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_at_least: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actual: Option<u64>,
}
fn omission(field: &'static str, cause: &NativeCause, reason: &'static str) -> Omission {
    let (limit, observed_at_least, actual) = match size_limit(cause) {
        Some(SizeViolation::SerializationBound {
            limit,
            observed_at_least,
            ..
        }) => (Some(limit), Some(observed_at_least), None),
        Some(SizeViolation::Measured { limit, actual, .. }) => (Some(limit), None, Some(actual)),
        None => (None, None, None),
    };
    Omission {
        field,
        reason: if limit.is_some() {
            "bound_reached"
        } else {
            reason
        },
        limit,
        observed_at_least,
        actual,
    }
}
#[derive(Serialize)]
struct ReportDetail {
    stage: ReportStage,
    #[serde(skip_serializing_if = "Option::is_none")]
    cause: Option<Box<RawValue>>,
    omissions: Vec<Omission>,
}
#[derive(Serialize)]
struct ObservedHead<'a> {
    run_id: &'a RunId,
    head_sequence: u64,
    head_digest: &'a ContentDigest,
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

/// Prepared incomplete report with explicit omissions and accurately labelled head evidence.
///
/// Encoding these prepared fields does not invoke a native projector. This response has no
/// separate size quota; retained Objects and native projections retain their existing bounds.
#[derive(Serialize)]
pub struct IncompleteReport<'a> {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_id: Option<&'a RunId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery: Option<&'a RunRecovery>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_observed: Option<ObservedHead<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    acknowledged: Option<&'a mfm_store::RunSummary>,
    report_failure: ReportDetail,
}
impl ReportFailure<RunRequestError> {
    /// Prepares an incomplete error report without retrying a failed secondary projector.
    pub fn incomplete(&mut self) -> IncompleteReport<'_> {
        let report_failure = self.detail();
        let invocation = match &self.original {
            RunRequestError::Request(_) => None,
            RunRequestError::Invocation(invocation)
            | RunRequestError::AppendIndeterminate { invocation, .. } => Some(invocation),
        };
        let (run_id, last_observed, acknowledged) = match invocation {
            Some(InvocationFailure::Execution {
                run_id,
                last_observed,
                error,
            }) => {
                let acknowledged = match error {
                    RuntimeError::Projection { acknowledged, .. } => Some(acknowledged.as_ref()),
                    _ => None,
                };
                (
                    Some(run_id),
                    last_observed.as_ref().map(ObservedHead::from),
                    acknowledged,
                )
            }
            Some(InvocationFailure::RecoveryStopped { observed, .. }) => (
                Some(observed.run_id()),
                Some(ObservedHead::from(observed)),
                None,
            ),
            None => (None, None, None),
        };
        IncompleteReport {
            code: self.original.code(),
            message: self.original.to_string(),
            run_id,
            recovery: self.original.recovery(),
            last_observed,
            acknowledged,
            report_failure,
        }
    }
}
fn observed_report(view: &RunView, report_failure: ReportDetail) -> IncompleteReport<'_> {
    IncompleteReport {
        code: "report_render_failed",
        message: "run observation could not be rendered completely".into(),
        run_id: Some(view.run_id()),
        recovery: None,
        last_observed: Some(view.into()),
        acknowledged: None,
        report_failure,
    }
}
impl ReportFailure<RunView> {
    /// Prepares compact observation evidence without claiming this invocation inserted it.
    pub fn incomplete(&mut self) -> IncompleteReport<'_> {
        let detail = self.detail();
        observed_report(&self.original, detail)
    }
}
impl ReportFailure<StartRunResult> {
    /// Prepares compact observation evidence while retaining the complete start result natively.
    pub fn incomplete(&mut self) -> IncompleteReport<'_> {
        let detail = self.detail();
        observed_report(self.original.run(), detail)
    }
}
