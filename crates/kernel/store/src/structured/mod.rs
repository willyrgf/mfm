//! Sole callback-free fold and atomic persistence port for structured runs.

mod adapter;
mod assembly;
mod backend;
mod configuration;
mod fact_scan;
mod fold;
#[cfg(any(test, feature = "test-support"))]
mod memory;
mod mutation;
mod purpose;
mod qualification;

#[doc(hidden)]
pub use adapter::StoreHistoryAdapter;
pub use assembly::{assemble_structured_runtime, AssembledStructuredRuntime};
#[cfg(any(test, feature = "test-support"))]
pub use assembly::assemble_with_backend;
pub use backend::{
    BackendAppendOutcome, RawRunHistory, StructuredBackendFuture, StructuredHistoryBackend,
    StructuredStoreIdentity, TenantFactPublication, ValidatedBatch,
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
    ActionableState, LaneCursor, ObservationQualification, ProgramCursor, StateLeaf,
    StructuredFrontier, StructuredStoreError, VerifiedProgramData, VerifiedStructuredRun,
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
pub use mutation::{
    ObservationCommit, StructuredAdmissionRequest, StructuredAppendAttempt,
};
// Runtime-facing authorization proof type.
pub use mfm_certify::structured::NewlyAppendedAuthorization;
pub use purpose::{
    AuditRunReader, ExportRunReader, PublicRunReader, ReplayRunReader, TraceRunReader,
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
