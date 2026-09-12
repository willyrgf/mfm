use mfm_journal::EncodedRunFrame;
use mfm_store::{RunSummary, StoreError};
use mfm_values::NativeCause;
use serde::Serialize;
use serde_json::value::RawValue;

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
/// Reviewed task result; panic payloads are withheld.
#[derive(Debug, Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum TaskFailure {
    /// The operation did not return; its panic payload is withheld.
    #[error("task panicked; payload withheld")]
    Panicked,
    /// The operation did not return because the task was cancelled.
    #[error("task cancelled")]
    Cancelled,
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
/// Original outcome of this physical append attempt.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppendFailure {
    /// This attempt inserted nothing.
    NotInserted,
    /// The Store's definite or indeterminate error.
    Store(StoreError),
}
/// Invocation-local original/candidate custody after failed recording.
#[derive(Debug)]
pub enum RecordingFailure {
    /// No append was attempted.
    BeforeAppend {
        /// Returned declared original, when one exists.
        original: Option<NativeCause>,
        /// Exact candidate if sealing completed.
        candidate: Option<EncodedRunFrame>,
        /// Encoding, task, sealing or size failure.
        cause: NativeCause,
    },
    /// A physical append did not acknowledge insertion to this caller.
    Append {
        /// Returned declared original, when one exists.
        original: Option<NativeCause>,
        /// Exact sealed candidate retained only in native custody.
        candidate: EncodedRunFrame,
        /// This attempt's actual outcome.
        outcome: AppendFailure,
        /// Bound mechanical candidate finding, independently of payload projection.
        observation: Option<(Box<RunSummary>, CandidatePresence)>,
        /// Secondary reconciliation failure, without replacing the append outcome.
        reload_cause: Option<NativeCause>,
    },
}
impl RecordingFailure {
    fn project(&self) -> Result<Box<RawValue>, NativeCause> {
        #[derive(Serialize)]
        struct Candidate<'a> {
            run_id: &'a mfm_ids::RunId,
            sequence: u64,
            digest: &'a mfm_ids::ContentDigest,
        }
        fn candidate(frame: &EncodedRunFrame) -> Candidate<'_> {
            Candidate {
                run_id: frame.run_id(),
                sequence: frame.run_sequence(),
                digest: frame.head_digest(),
            }
        }
        #[derive(Serialize)]
        #[serde(rename_all = "snake_case")]
        enum Wire<'a> {
            BeforeAppend {
                original: Option<Box<RawValue>>,
                candidate: Option<Candidate<'a>>,
                cause: Box<RawValue>,
            },
            Append {
                original: Option<Box<RawValue>>,
                candidate: Candidate<'a>,
                outcome: &'a AppendFailure,
                observation: &'a Option<(Box<RunSummary>, CandidatePresence)>,
                reload_cause: Option<Box<RawValue>>,
            },
        }
        let wire = match self {
            Self::BeforeAppend {
                original,
                candidate: frame,
                cause,
            } => Wire::BeforeAppend {
                original: original.as_ref().map(NativeCause::project).transpose()?,
                candidate: frame.as_ref().map(candidate),
                cause: cause.project()?,
            },
            Self::Append {
                original,
                candidate: frame,
                outcome,
                observation,
                reload_cause,
            } => Wire::Append {
                original: original.as_ref().map(NativeCause::project).transpose()?,
                candidate: candidate(frame),
                outcome,
                observation,
                reload_cause: reload_cause
                    .as_ref()
                    .map(NativeCause::project)
                    .transpose()?,
            },
        };
        project(&wire)
    }
}
impl std::fmt::Display for RecordingFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("run recording failed")
    }
}
impl std::error::Error for RecordingFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BeforeAppend { cause, .. } => Some(cause),
            Self::Append {
                outcome: AppendFailure::Store(source),
                ..
            } => Some(source),
            Self::Append { .. } => None,
        }
    }
}

fn project<T: Serialize>(value: &T) -> Result<Box<RawValue>, NativeCause> {
    let json = mfm_canonical::to_json_bounded(value, mfm_values::MAX_RUN_OBJECT_CANONICAL_BYTES)
        .map_err(NativeCause::from_error)?;
    RawValue::from_string(json)
        .map_err(|source| NativeCause::from_error(mfm_canonical::JsonError::new(source)))
}
impl crate::RuntimeError {
    /// Retains this Runtime failure with its fallible reviewed projection in native custody.
    pub fn into_native(self) -> NativeCause {
        NativeCause::from_error_with(self, Self::project)
    }
    /// Prepares bounded causal detail without discarding a failing child's native cause.
    /// The borrowed invocation retains its original and any candidate if preparation fails.
    pub fn project(&self) -> Result<Box<RawValue>, NativeCause> {
        #[derive(Serialize)]
        #[serde(rename_all = "snake_case")]
        enum Wire<'a> {
            Absent,
            AdmissionConflict,
            Store(&'a StoreError),
            InvalidHistory,
            IncompatibleAssembly,
            SizeLimit {
                resource: &'a crate::SizeResource,
                size: &'a mfm_values::SizeLimitExceeded,
            },
            ArithmeticOverflow,
            Native {
                operation: &'a Operation,
                stage: &'a Stage,
                cause: Box<RawValue>,
            },
            Recording {
                operation: &'a Operation,
                failure: Box<RawValue>,
            },
            Projection {
                acknowledged: &'a RunSummary,
                cause: Box<RawValue>,
            },
        }
        let wire = match self {
            Self::Absent => Wire::Absent,
            Self::AdmissionConflict => Wire::AdmissionConflict,
            Self::Store(source) => Wire::Store(source),
            Self::InvalidHistory => Wire::InvalidHistory,
            Self::IncompatibleAssembly => Wire::IncompatibleAssembly,
            Self::SizeLimit { resource, size } => Wire::SizeLimit { resource, size },
            Self::ArithmeticOverflow => Wire::ArithmeticOverflow,
            Self::Native {
                operation,
                stage,
                cause,
            } => Wire::Native {
                operation,
                stage,
                cause: cause.project()?,
            },
            Self::Recording { operation, failure } => Wire::Recording {
                operation,
                failure: failure.project()?,
            },
            Self::Projection {
                acknowledged,
                cause,
            } => Wire::Projection {
                acknowledged,
                cause: cause.project()?,
            },
        };
        project(&wire)
    }
}
