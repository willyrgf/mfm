//! Production assembly that returns Runtime and purpose readers only.

use std::sync::Arc;

use mfm_certify::structured::QualifiedProgramRegistry;
use mfm_runtime::structured::Runtime;

use super::adapter::{build_program_verifier, StoreHistoryAdapter};
use super::backend::{StructuredHistoryBackend, StructuredRunStore};
use super::purpose::{
    AuditRunReader, ExportRunReader, PublicRunReader, ReplayRunReader, TraceRunReader,
};
use super::qualification::PublicPhysicalBindingVerifier;

/// Complete production assembly result. Never includes store, writer, backend, or port.
pub struct AssembledStructuredRuntime<B: StructuredHistoryBackend> {
    /// Sole Runtime mutation authority.
    pub runtime: Runtime<StoreHistoryAdapter<B>>,
    /// Public-read purpose reader.
    pub public_reader: PublicRunReader<B>,
    /// Trace purpose reader.
    pub trace_reader: TraceRunReader<B>,
    /// Audit purpose reader.
    pub audit_reader: AuditRunReader<B>,
    /// Replay purpose reader.
    pub replay_reader: ReplayRunReader<B>,
    /// Export purpose reader.
    pub export_reader: ExportRunReader<B>,
}

/// Consumes one complete qualified registry and backend into Runtime plus purpose readers.
///
/// The registry is consumed exactly once: admission verification is installed in the
/// private store adapter and the process registry is installed in Runtime. Callers
/// cannot split, omit, substitute, or reuse either half independently.
pub fn assemble_structured_runtime<B: StructuredHistoryBackend>(
    backend: B,
    registry: QualifiedProgramRegistry,
    physical_binding_verifier: Arc<dyn PublicPhysicalBindingVerifier>,
) -> AssembledStructuredRuntime<B> {
    let (admission, processes) = registry.into_runtime_parts();
    let program_verifier = build_program_verifier(admission);
    let store = StructuredRunStore::new(backend, program_verifier, physical_binding_verifier);
    let (writer, reader) = store.split();
    let history = StoreHistoryAdapter::from_writer(writer);
    let runtime = Runtime::new(history, processes);
    AssembledStructuredRuntime {
        runtime,
        public_reader: PublicRunReader::new(reader.clone()),
        trace_reader: TraceRunReader::new(reader.clone()),
        audit_reader: AuditRunReader::new(reader.clone()),
        replay_reader: ReplayRunReader::new(reader.clone()),
        export_reader: ExportRunReader::new(reader),
    }
}


/// Test-support assembly over an arbitrary backend.
#[cfg(any(test, feature = "test-support"))]
pub fn assemble_with_backend<B: StructuredHistoryBackend>(
    backend: B,
    registry: QualifiedProgramRegistry,
    physical_binding_verifier: Arc<dyn PublicPhysicalBindingVerifier>,
) -> AssembledStructuredRuntime<B> {
    assemble_structured_runtime(backend, registry, physical_binding_verifier)
}
