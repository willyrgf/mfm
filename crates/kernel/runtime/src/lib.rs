#![warn(missing_docs)]
//! Immutable typed Runtime assembly and caller-driven run progression.
//!
//! Runtime owns the current continuation and its local validation. Store supplies
//! admission/latest rows, Journal decodes their opaque envelopes, and the associated
//! executable runs only the currently selected State.

mod assembly;
mod engine;
mod error;
mod report;
mod state;
pub use error::{CandidatePresence, Operation, RecordingFailure, Stage};
pub use mfm_values::Object;
pub use report::{FailureReport, InvocationFailure};
pub use state::{Call, EffectCall, Failure, RecoveryOutcome, Settlement, StateCall};

use std::sync::Arc;

use mfm_ids::{ContentDigest, ExecutionPosition, RunId, StatePosition};
use mfm_program::Program;
use mfm_store::Store;
use mfm_values::MfmValue;

pub use assembly::{RuntimeAssembly, RuntimeAssemblyBuilder};

/// Result type for Runtime operations.
pub type Result<T> = std::result::Result<T, RuntimeError>;

/// Redaction-safe Runtime failure.
#[derive(Debug, serde::Serialize, thiserror::Error)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeError {
    /// The requested run does not exist.
    #[error("run is absent")]
    Absent,
    /// The requested admission differs from retained genesis.
    #[error("run admission conflicts with retained history")]
    AdmissionConflict,
    /// A mechanical Store operation failed; ambiguous acknowledgement remains distinguishable.
    #[error("store operation failed")]
    Store(#[from] mfm_store::StoreError),
    /// Retained physical, structural, or semantic history is invalid.
    #[error("retained run history is invalid")]
    InvalidHistory,
    /// Static assembly and Program associations are incomplete or inconsistent.
    #[error("runtime assembly is incompatible")]
    IncompatibleAssembly,
    /// A measured size exceeded its limit.
    #[error("{resource} {size}")]
    SizeLimit {
        /// Resource whose inclusive limit was exceeded.
        resource: SizeResource,
        /// Safe numeric evidence of the violation.
        size: mfm_values::SizeLimitExceeded,
    },
    /// Capacity arithmetic could not represent the result.
    #[error("capacity arithmetic overflow")]
    ArithmeticOverflow,
    /// A reviewed native failure at its actual operation/stage.
    #[error("runtime operation failed")]
    Native {
        /// Originating operation.
        operation: Operation,
        /// Actual stage within that operation.
        stage: Stage,
        /// Selected owner data, without native custody.
        cause: mfm_values::InvocationDiagnostic,
    },
    /// A returned original and candidate remain in invocation custody.
    #[error("runtime recording failed")]
    Recording {
        /// Originating operation.
        operation: Operation,
        /// Available recording facts.
        #[source]
        failure: Box<RecordingFailure>,
    },
    /// Insertion was acknowledged but the resulting view could not be projected.
    #[error("acknowledged state projection failed")]
    Projection {
        /// Acknowledged mechanical head, even if the last renderable view is older.
        acknowledged: Box<mfm_store::RunSummary>,
        /// Actual projection failure.
        #[source]
        cause: Box<RuntimeError>,
    },
}

impl RuntimeError {
    pub(crate) fn native(operation: Operation, cause: mfm_values::InvocationDiagnostic) -> Self {
        Self::Native {
            operation,
            stage: Stage::Execute,
            cause,
        }
    }
    pub(crate) fn at(
        operation: Operation,
        stage: Stage,
        cause: mfm_values::InvocationDiagnostic,
    ) -> Self {
        Self::Native {
            operation,
            stage,
            cause,
        }
    }
}
impl From<mfm_values::ValueError> for RuntimeError {
    fn from(error: mfm_values::ValueError) -> Self {
        Self::at(
            Operation::Record,
            Stage::Encode,
            error.into_diagnostic("from"),
        )
    }
}

use mfm_values::{SizeResource, SizeViolation};
impl RuntimeError {
    /// Projects size evidence from the primary failure without consuming its cause chain.
    /// A reconciliation error does not replace the original append disposition.
    pub fn size_limit(&self) -> Option<SizeViolation> {
        match self {
            Self::SizeLimit { resource, size } => Some(SizeViolation::measured(*resource, *size)),
            Self::Store(error) => store_size(error),
            Self::Native { cause, .. } => cause.size(),
            Self::Projection { cause, .. } => cause.size_limit(),
            Self::Recording { failure, .. } => match failure.as_ref() {
                RecordingFailure::BeforeAppend { cause, .. } => cause.size_limit(),
                RecordingFailure::Store { cause, .. } => store_size(cause),
                RecordingFailure::NotInserted { .. } => None,
            },
            _ => None,
        }
    }
}
fn store_size(error: &mfm_store::StoreError) -> Option<SizeViolation> {
    use mfm_store::StoreError;
    let (resource, size) = match error {
        StoreError::FrameSize(size) => (SizeResource::Frame, size),
        StoreError::HistorySize(size) => (SizeResource::HistoryBytes, size),
        StoreError::FrameCount(size) => (SizeResource::FrameCount, size),
        _ => return None,
    };
    Some(SizeViolation::measured(resource, *size))
}
pub(crate) fn check_size(resource: SizeResource, actual: u64, limit: u64) -> Result<()> {
    mfm_values::SizeLimitExceeded::check(actual, limit)
        .map_err(|size| RuntimeError::SizeLimit { resource, size })
}

/// Result of one successful Effect adapter invocation.
///
/// Nonterminal progress is represented without fabricating evidence or
/// concluding the durable Effect prepare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectAdapterOutcome<E> {
    /// The Effect remains pending and can be resumed by a later caller.
    Pending,
    /// The Effect produced terminal evidence that Runtime must bind and interpret.
    Settled(E),
}

/// Durable public state of a run.
// Keep the owned observation together; Object clones already share immutable payload bytes.
#[allow(clippy::large_enum_variant)]
pub enum RunViewState {
    /// The selected State is waiting for caller-driven progression.
    Runnable {
        /// Selected execution occurrence.
        position: ExecutionPosition,
        /// Committed transition that selected this occurrence.
        reason: RunnableReason,
    },
    /// Acknowledged command awaiting reconciliation with the same authority.
    EffectPending {
        /// Complete retained command authority and input.
        effect: EffectCall,
        /// Most recent acknowledged original and recovery outcome.
        latest_failure: Option<(Object, RecoveryOutcome)>,
    },
    /// A declared original is durable and awaits recovery evaluation.
    AwaitingRecovery {
        /// Complete original operation facts, without an uncommitted decision.
        failure: Failure,
    },
    /// Accepted settlement is durable and awaits deterministic interpretation.
    AwaitingInterpretation {
        /// Complete command authority and accepted evidence.
        settlement: Settlement,
    },
    /// The Program reached its declared root success.
    Succeeded(Object),
    /// The Program reached its declared root failure.
    Failed(FailureReport),
}

/// Committed transition that selected a runnable occurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnableReason {
    /// The preceding State succeeded, or the run was admitted.
    Advance,
    /// Retry with the same exact input.
    Retry,
    /// Restore the active retained checkpoint input.
    Restart {
        /// Selected checkpoint boundary.
        checkpoint: StatePosition,
    },
}

/// Snapshot of one real qualified durable run head.
pub struct RunView {
    run_id: RunId,
    head_sequence: u64,
    head_digest: ContentDigest,
    state: RunViewState,
    admitted_context: Arc<Object>,
    entry_point: mfm_ids::EntryPointId,
}

impl RunView {
    /// Returns the exact entry point retained in the admitted Program.
    pub fn entry_point(&self) -> &mfm_ids::EntryPointId {
        &self.entry_point
    }

    /// Returns the exact input qualified from this run's admission.
    pub fn admitted_context(&self) -> &Object {
        &self.admitted_context
    }

    /// Returns the run identity.
    pub const fn run_id(&self) -> &RunId {
        &self.run_id
    }

    /// Returns the durable head sequence.
    pub const fn head_sequence(&self) -> u64 {
        self.head_sequence
    }

    /// Returns the durable exact-byte frame head.
    pub const fn head_digest(&self) -> &ContentDigest {
        &self.head_digest
    }

    /// Returns the semantic state at this snapshot.
    pub const fn state(&self) -> &RunViewState {
        &self.state
    }
}

/// Runtime over one immutable assembly and one mechanical Store.
pub struct Runtime {
    assembly: RuntimeAssembly,
    store: Arc<dyn Store>,
}

impl Runtime {
    /// Constructs a Runtime over an already-open Store.
    pub fn new(assembly: RuntimeAssembly, store: Arc<dyn Store>) -> Self {
        Self { assembly, store }
    }

    /// Admits an exact checked Program and typed C0, then progresses it.
    pub async fn start<T: MfmValue>(
        &self,
        run_id: RunId,
        program: Program,
        c0: T,
    ) -> std::result::Result<RunView, InvocationFailure> {
        engine::start(
            self.assembly.handle(),
            Arc::clone(&self.store),
            run_id,
            program,
            c0,
        )
        .await
    }

    /// Loads and progresses an existing run.
    pub async fn resume(&self, run_id: &RunId) -> std::result::Result<RunView, InvocationFailure> {
        engine::resume(
            self.assembly.handle(),
            Arc::clone(&self.store),
            run_id.clone(),
        )
        .await
    }

    /// Loads and validates the current record without executing a State or adapter.
    pub async fn read(&self, run_id: &RunId) -> std::result::Result<RunView, InvocationFailure> {
        engine::read(
            self.assembly.handle(),
            Arc::clone(&self.store),
            run_id.clone(),
        )
        .await
    }
}
