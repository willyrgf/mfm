use mfm_journal::EncodedRunFrame;
use mfm_store::{RunSummary, StoreError};
use serde::Serialize;

/// Runtime operation at which an invocation failed.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// Admission and executable association.
    Admission,
    /// Loading and validating the selected stored rows.
    Restore,
    /// Deterministic Pure evaluation.
    PureEvaluate,
    /// Observational intent preparation.
    ReadPrepare,
    /// Read adapter invocation.
    ReadAdapter,
    /// Binding accepted Read evidence.
    ReadBind,
    /// Read interpretation.
    ReadInterpret,
    /// Effect command preparation.
    EffectPrepare,
    /// Effect reconciliation.
    EffectAdapter,
    /// Effect evidence binding.
    EffectBind,
    /// Interpretation of durably accepted Effect evidence.
    EffectInterpret,
    /// Classification, handler invocation and authorization.
    Recovery,
    /// Root failure mapping.
    RootMap,
    /// Complete commit encoding and recording.
    Record,
    /// Public observation construction.
    Project,
}
/// Stage within the originating Runtime operation.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Native constructor materialization.
    Decode,
    /// Callback or pure validation.
    Execute,
    /// Canonical encoding.
    Encode,
    /// Frame sealing or actual metadata accounting.
    Seal,
    /// Atomic Store append.
    Append,
    /// Store snapshot load.
    Load,
    /// Public projection.
    Project,
}
/// Exact-candidate observation from a bound Store snapshot.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidatePresence {
    /// Exact bytes exist at the candidate sequence.
    Present,
    /// Different immutable bytes exclude this candidate.
    Excluded,
    /// No candidate row was visible; this does not resolve an in-flight append.
    Absent,
}
/// Original admitted outcome and physical recording facts after failed recording.
#[derive(Debug, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum RecordingFailure {
    /// Encoding or sealing failed before an append was attempted.
    #[error("run encoding or sealing failed")]
    BeforeAppend {
        /// Admitted declared failure whose recording did not reach append.
        original: crate::Failure,
        /// Original Runtime encoding, task, sealing or size failure.
        #[source]
        cause: Box<crate::RuntimeError>,
    },
    /// The Store returned a definite or indeterminate failure for the submitted candidate.
    #[error("run store append failed")]
    Store {
        /// Admitted declared failure, when one exists.
        original: Option<crate::Failure>,
        /// Exact submitted candidate; public serialization exposes its identity only.
        #[serde(serialize_with = "serialize_candidate")]
        candidate: EncodedRunFrame,
        /// Actual Store disposition and cause.
        #[source]
        cause: StoreError,
    },
    /// This append inserted nothing; a bound snapshot may independently locate the candidate.
    #[error("run candidate was not inserted")]
    NotInserted {
        /// Admitted declared failure, when one exists.
        original: Option<crate::Failure>,
        /// Exact submitted candidate; public serialization exposes its identity only.
        #[serde(serialize_with = "serialize_candidate")]
        candidate: EncodedRunFrame,
        /// Mechanical finding from one bound Store snapshot.
        observation: Option<(Box<RunSummary>, CandidatePresence)>,
        /// Independent failed reconciliation, without replacing the append outcome.
        reload_cause: Option<Box<crate::RuntimeError>>,
    },
}

fn serialize_candidate<S: serde::Serializer>(
    frame: &EncodedRunFrame,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeStruct;
    let mut candidate = serializer.serialize_struct("Candidate", 3)?;
    candidate.serialize_field("run_id", frame.run_id())?;
    candidate.serialize_field("sequence", &frame.run_sequence())?;
    candidate.serialize_field("digest", frame.head_digest())?;
    candidate.end()
}
