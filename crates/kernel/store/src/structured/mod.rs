//! Sole callback-free fold and atomic persistence port for structured runs.

mod adapter;
mod assembly;
mod backend;
mod canonical_append;
mod configuration;
mod fact_scan;
mod fold;
#[cfg(any(test, feature = "test-support"))]
mod memory;
mod mutation;
mod purpose;
mod qualification;

pub use adapter::RegistryProgramVerifier;
#[doc(hidden)]
pub use adapter::StoreHistoryAdapter;
#[cfg(any(test, feature = "test-support"))]
pub use assembly::assemble_with_backend;
pub use assembly::{assemble_structured_runtime, AssembledStructuredRuntime};
pub use backend::{
    BackendAppendOutcome, RawRunHistory, StructuredBackendFuture, StructuredHistoryBackend,
    StructuredRunSnapshot, StructuredStoreIdentity, TenantFactPublication, ValidatedBatch,
};
pub use canonical_append::{
    validate_append_objects, validate_envelope_frame, MAX_BATCH_OBJECTS, MAX_STORED_FRAME_BYTES,
};
pub use configuration::{
    verify_configuration_history, ConfigurationAppendRequest, ConfigurationBackendAppendOutcome,
    ConfigurationBackendFuture, ConfigurationHistoryBackend, ConfigurationHistoryHead,
    ConfigurationHistoryReader, ConfigurationHistoryStore, ConfigurationHistoryWriter,
    ConfigurationRevision, ConfigurationStreamKey, MemoryConfigurationHistoryBackend,
    RawConfigurationHistory, ValidatedConfigurationRevision, VerifiedConfiguredValue,
};
#[doc(hidden)]
pub use fact_scan::PriorRunFactScanCompletion;
pub use fold::{
    verify_offline_recorded_history, ActionableState, LaneCursor, ObservationQualification,
    ProgramCursor, ProgramVerifier, StateLeaf, StructuredFrontier, StructuredStoreError,
    VerifiedProgramData, VerifiedStructuredRun,
};
#[cfg(any(test, feature = "test-support"))]
pub use memory::{assemble_in_memory_runtime, StructuredMemoryBackend};
// Semantic command types are owned by mfm-runtime; re-export for store tests and fold.
pub use mfm_runtime::history::{
    AccessAuthorizationProposal, AccessObservationProposal, ProposedCanonicalValue,
    ProposedObservationOutcome, ProposedTransitionValue, StateTransitionProposal,
    StructuredAdmissionMaterial,
};
// Store-local append attempt and observation commit used by internal writer tests.
pub use mutation::{ObservationCommit, StructuredAdmissionRequest, StructuredAppendAttempt};
// Runtime-facing authorization proof type.
pub use mfm_certify::structured::NewlyAppendedAuthorization;
pub use purpose::{
    expand_export_source_closure, AuditRunEvidence, AuditRunReader, ExportRunEvidence,
    ExportRunReader, ExportSourceClosureError, PublicRunEvidence, PublicRunReader,
    RecordedRunEvidence, ReplayRunReader, TraceRunEvidence, TraceRunReader, MAX_EXPORT_SOURCE_RUNS,
};

/// Converts an offline-verified fold result into sealed recorded-replay evidence.
pub fn recorded_evidence_from_verified(verified: VerifiedStructuredRun) -> RecordedRunEvidence {
    RecordedRunEvidence::from_offline_verified(verified)
}
pub use qualification::{
    PhysicalBindingAuthorization, PhysicalBindingSupersession, PhysicalBindingVerificationMode,
    PublicPhysicalBindingVerifier,
};

/// Result type for structured RunHistory operations.
pub type Result<T> = std::result::Result<T, StructuredStoreError>;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
