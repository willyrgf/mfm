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
#[cfg(feature = "test-support")]
pub mod test_support;
#[cfg(all(test, not(feature = "test-support")))]
mod test_support;

pub use adapter::RegistryProgramVerifier;
#[doc(hidden)]
pub use adapter::StoreHistoryAdapter;
#[cfg(any(test, feature = "test-support"))]
pub use assembly::assemble_with_backend;
pub use assembly::{assemble_structured_runtime, AssembledStructuredRuntime};
pub use backend::{
    BackendAppendOutcome, PhysicalTargetIdentity, RawRunHistory, StructuredBackendFuture,
    StructuredHistoryBackend, StructuredRunSnapshot, StructuredStoreIdentity,
    TenantFactPublication,
};
pub use canonical_append::{
    validate_append_objects, validate_envelope_frame, validate_record_object_closure,
    CanonicalConfigurationAppend, CanonicalRunAppend, MAX_BATCH_OBJECTS, MAX_BATCH_RECORDS,
    MAX_STORED_FRAME_BYTES,
};
pub use configuration::{
    verify_configuration_history, ConfigurationAppendRequest, ConfigurationBackendAppendOutcome,
    ConfigurationBackendFuture, ConfigurationHistoryBackend, ConfigurationHistoryHead,
    ConfigurationHistoryReader, ConfigurationHistoryStore, ConfigurationHistoryWriter,
    ConfigurationRevision, ConfigurationStreamKey, MemoryConfigurationHistoryBackend,
    RawConfigurationHistory, VerifiedConfiguredValue,
};
#[doc(hidden)]
pub use fact_scan::PriorRunFactScanCompletion;
pub use fold::{
    verify_offline_recorded_history, ActionableState, LaneCursor, ObservationQualification,
    ProgramCursor, ProgramVerifier, StateLeaf, StructuredFrontier, StructuredStoreError,
    VerifiedProgramData,
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
pub use mfm_runtime::history::CommittedAccessAuthorization;
pub use purpose::{
    expand_export_source_closure, AuditAccessEntry, AuditObservation, AuditRunEvidence,
    AuditRunReader, ExportEncoderSource, ExportEncoderView, ExportFactRoute, ExportRunEvidence,
    ExportRunReader, ExportSourceClosureError, OfflineVerifiedRun, PublicRunEvidence,
    PublicRunReader, RecordedRunEvidence, ReplayRunReader, RunEvidenceStatus, TraceRunEvidence,
    TraceRunReader, TraceTransitionEntry, MAX_PORTABLE_SOURCE_RUNS,
};
pub use qualification::{
    PhysicalBindingAuthorization, PhysicalBindingSupersession, PhysicalBindingVerificationMode,
    PublicPhysicalBindingVerifier,
};

/// Result type for structured RunHistory operations.
pub type Result<T> = std::result::Result<T, StructuredStoreError>;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
