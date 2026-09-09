#![warn(missing_docs)]
//! Immutable typed Runtime assembly and caller-driven run progression.
//!
//! Runtime is the sole semantic fold owner. Store supplies complete opaque
//! prefixes, Journal qualifies them, and the associated mode-specific
//! executable runs only the currently selected State.

mod assembly;
mod engine;
mod report;
pub use report::{AdapterIncidentView, FailureCauseView, FailureReport, InvocationFailure};

use std::sync::Arc;

use mfm_ids::{ContentDigest, ContentRef, EffectId, ExecutionPosition, RunId, StatePosition};
use mfm_program::Program;
use mfm_store::Store;
use mfm_values::MfmValue;

pub use assembly::{RuntimeAssembly, RuntimeAssemblyBuilder};

/// Result type for Runtime operations.
pub type Result<T> = std::result::Result<T, RuntimeError>;

/// Redaction-safe Runtime failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
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
    /// A measured size or declared admission bound exceeded its limit.
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
    /// A trusted local invariant failed.
    #[error("runtime internal failure")]
    Internal,
}

/// Resource measured by a Runtime size-limit failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeResource {
    /// One canonical typed value.
    CanonicalObject,
    /// One complete Journal frame.
    Frame,
    /// Non-payload frame metadata.
    FrameEnvelope,
    /// The complete run's accumulated frame bytes.
    HistoryBytes,
    /// The complete run's frame count.
    FrameCount,
    /// The executing declaration's admitted frame bound.
    DeclaredFrame,
    /// The derived inline terminal failure report.
    FailureReport,
}

impl std::fmt::Display for SizeResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::CanonicalObject => "canonical_object",
            Self::Frame => "frame",
            Self::FrameEnvelope => "frame_envelope",
            Self::HistoryBytes => "history_bytes",
            Self::FrameCount => "frame_count",
            Self::DeclaredFrame => "declared_frame",
            Self::FailureReport => "failure_report",
        })
    }
}

impl RuntimeError {
    /// Returns exact safe size diagnostics, including mechanical Store failures.
    pub const fn size_limit(self) -> Option<(SizeResource, mfm_values::SizeLimitExceeded)> {
        match self {
            Self::SizeLimit { resource, size } => Some((resource, size)),
            Self::Store(mfm_store::StoreError::FrameSize(size)) => {
                Some((SizeResource::Frame, size))
            }
            Self::Store(mfm_store::StoreError::HistorySize(size)) => {
                Some((SizeResource::HistoryBytes, size))
            }
            Self::Store(mfm_store::StoreError::FrameCount(size)) => {
                Some((SizeResource::FrameCount, size))
            }
            _ => None,
        }
    }
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
        /// Prepared execution occurrence.
        position: ExecutionPosition,
        /// Exact retained Effect identity.
        effect_id: EffectId,
    },
    /// The Program reached its declared root success.
    Succeeded(ValueView),
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

/// Qualified canonical value with an exact typed contract.
pub struct ValueView {
    contract_ref: ContentRef,
    value_ref: ContentRef,
    canonical: mfm_canonical::PlainCanonicalJsonBytes,
}

impl ValueView {
    /// Decodes the requested exact schema contract, rejecting structurally similar other types.
    pub fn decode<T: MfmValue>(&self) -> mfm_values::Result<T> {
        let expected = mfm_program::nominal_contract_ref::<T>()
            .map_err(|_| mfm_values::ValueError::InvalidSchemaIdentity)?;
        if expected != self.contract_ref {
            return Err(mfm_values::ValueError::SchemaShapeMismatch);
        }
        T::schema_descriptor()?
            .identity()
            .validate_canonical_value(self.canonical_bytes())?;
        serde_json::from_slice(self.canonical_bytes())
            .map_err(|_| mfm_values::ValueError::SchemaShapeMismatch)
    }

    /// Returns the nominal typed contract.
    pub const fn contract_ref(&self) -> &ContentRef {
        &self.contract_ref
    }

    /// Returns the exact retained instance reference.
    pub const fn value_ref(&self) -> &ContentRef {
        &self.value_ref
    }

    /// Returns exact canonical retained bytes.
    pub fn canonical_bytes(&self) -> &[u8] {
        self.canonical.as_bytes()
    }
}

/// Snapshot of one real qualified durable run head.
pub struct RunView {
    run_id: RunId,
    head_sequence: u64,
    head_digest: ContentDigest,
    state: RunViewState,
    admitted_context: Arc<ValueView>,
    entry_point: mfm_ids::EntryPointId,
}

impl RunView {
    /// Returns the exact entry point retained in the admitted Program.
    pub fn entry_point(&self) -> &mfm_ids::EntryPointId {
        &self.entry_point
    }

    /// Returns the exact genesis input already qualified by this view's Runtime fold.
    pub fn admitted_context(&self) -> &ValueView {
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

    /// Loads and folds an existing run without executing a State or adapter.
    pub async fn read(&self, run_id: &RunId) -> std::result::Result<RunView, InvocationFailure> {
        engine::read(
            self.assembly.handle(),
            Arc::clone(&self.store),
            run_id.clone(),
        )
        .await
    }
}
