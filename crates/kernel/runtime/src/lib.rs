#![warn(missing_docs)]
//! Immutable typed Runtime assembly and caller-driven run progression.
//!
//! Runtime is the sole semantic fold owner. Store supplies complete opaque
//! prefixes, Journal qualifies them, and registered typed drivers execute only
//! the currently selected State.

mod assembly;
mod engine;

use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{ContentDigest, ContentRef, RunId};
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
    /// A Store append may have committed.
    #[error("append outcome is indeterminate")]
    Indeterminate,
    /// Retained physical, structural, or semantic history is invalid.
    #[error("retained run history is invalid")]
    InvalidHistory,
    /// Static assembly and Program associations are incomplete or inconsistent.
    #[error("runtime assembly is incompatible")]
    IncompatibleAssembly,
    /// A local fixed capacity was exceeded.
    #[error("runtime capacity exceeded")]
    Capacity,
    /// A required Store or capability dependency is unavailable.
    #[error("runtime dependency is unavailable")]
    Unavailable,
    /// A trusted local invariant failed.
    #[error("runtime internal failure")]
    Internal,
}

/// Redaction-safe error available to Read and Effect adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AdapterError {
    /// No trusted observation or settlement evidence was produced.
    #[error("adapter is unavailable")]
    Unavailable,
    /// A trusted adapter invariant failed.
    #[error("adapter failed")]
    Internal,
}

/// Durable public state of a run.
pub enum RunViewState {
    /// The selected State is waiting for caller-driven progression.
    Runnable,
    /// The Program reached its declared root success.
    Succeeded(RetainedValueView),
    /// The Program reached its declared root failure.
    Failed(RetainedValueView),
}

/// Qualified retained terminal value.
pub struct RetainedValueView {
    contract_ref: ContentRef,
    value_ref: ContentRef,
    canonical: PlainCanonicalJsonBytes,
}

impl RetainedValueView {
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
}

impl RunView {
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
    ) -> Result<RunView> {
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
    pub async fn resume(&self, run_id: &RunId) -> Result<RunView> {
        engine::resume(
            self.assembly.handle(),
            Arc::clone(&self.store),
            run_id.clone(),
        )
        .await
    }

    /// Loads and folds an existing run without executing a State or adapter.
    pub async fn read(&self, run_id: &RunId) -> Result<RunView> {
        engine::read(
            self.assembly.handle(),
            Arc::clone(&self.store),
            run_id.clone(),
        )
        .await
    }
}
