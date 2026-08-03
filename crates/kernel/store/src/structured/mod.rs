//! Sole callback-free fold and atomic persistence port for structured runs.

mod backend;
mod configuration;
mod fact_scan;
mod fold;
#[cfg(any(test, feature = "test-support"))]
mod memory;
mod mutation;
mod qualification;

pub use backend::{
    BackendAppendOutcome, RawRunHistory, StructuredBackendFuture, StructuredHistoryBackend,
    StructuredRunHistoryReader, StructuredRunHistoryWriter, StructuredRunStore,
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
    StructuredFrontier, StructuredProgramVerifier, StructuredStoreError, VerifiedProgramData,
    VerifiedStructuredRun,
};
#[cfg(any(test, feature = "test-support"))]
pub use memory::{open_structured_in_memory, StructuredMemoryBackend};
pub use mutation::{
    AccessAuthorizationProposal, AccessObservationProposal, NewlyAppendedAuthorization,
    ObservationCommit, ProposedCanonicalValue, ProposedObservationOutcome, ProposedTransitionValue,
    StateTransitionProposal, StructuredAdmissionMaterial, StructuredAdmissionRequest,
    StructuredAppendAttempt,
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
