//! Recoverability-v3 store-owned journal, authority, fold, and backend contracts.

mod admission_preparation;
mod admission_source;
mod append;
mod append_preparation;
mod authority;
mod backend;
mod comparison;
mod configured_value;
mod errors;
mod fact_scan;
mod frame_preparation;
mod history;
mod journal;
mod objects;
#[cfg(test)]
mod observation_verification_tests;
mod preparation;
mod public_read;
mod support;
mod trace;
mod transition_preparation;

#[cfg(any(test, feature = "test-support"))]
mod memory;
#[cfg(any(test, feature = "test-support"))]
pub use self::memory::{open_in_memory, InMemoryRunJournalBackend, MemoryCommitFailurePoint};
#[cfg(any(test, feature = "backend-conformance"))]
pub mod test_support;

pub use self::admission_source::{
    AdmissionSourceBackend, AdmissionSourceVerifier, ProposedAdmissionSourceRoot,
    ProposedAdmissionSources,
};
pub use self::append::{
    AdmitRun, AppendOutcome, AppendRejection, AssignedJournalAppend, AuthorizeExternalAccess,
    CommitTransition, CommittedAppend, ExactResolution, JournalAppendVerifier,
    LogicalObservationIdentity, NewlyAdmittedRun, NewlyAppended, NewlyAppendedAuthorization,
    ObserveExternalAccess, PhysicalAppendIdentity, PhysicalAppendResolution, PreparedAppendKind,
    PreparedJournalAppend, ResolvedLogicalObservation, SuccessorDisposition,
    FACT_SELECTION_OPERATION_ID,
};
pub use self::authority::{
    Admit, CommittedJournalLoadGrant, Drive, ExistingRunAccessGrant, Export, InspectAudit,
    InspectTrace, QualifiedDeploymentAuthority, ReadPublic, Replay, RunAccessAuthority,
    RunAccessAuthorityIssuer, RunAccessGrant, StoreAuthorityContext, StoreIdentity,
};
pub use self::backend::{AsyncStoreFuture, RunJournalBackend, VerifiedAccessAuditPage};
pub use self::comparison::{
    ComparisonEvidenceKind, ComparisonSettlementKind, ComparisonTransitionKind,
    RecordedEvidenceVerdict, VerifiedComparisonEvidence, VerifiedComparisonFact,
    VerifiedComparisonFrame, VerifiedComparisonFrameReader, VerifiedComparisonOutput,
    VerifiedComparisonReadOutcome, VerifiedComparisonSafeFailure, VerifiedComparisonSettlement,
    VerifiedComparisonStateFrame, VerifiedComparisonTerminalEffect, VerifiedComparisonValue,
};
pub use self::configured_value::{
    ConfiguredValueBackend, ConfiguredValueResolveVerifier, VerifiedConfiguredValue,
};
pub use self::errors::{FactScanFailureProvenance, StoreError, StoreErrorInspection};
pub use self::fact_scan::{
    CompletedFactScan, FactAttestationLoadVerifier, FactScanBackend, FactScanPage,
    FactScanPageVerifier, FactScanPermit, FactSelectionAuthorizationOutcome,
    PendingFactScanAttestation, PersistedFactScanAttestation, SealedFactSelectionObservation,
    VerifiedFactSelectionCompleteness, FACT_SCAN_STEP_FACTS, FACT_SCAN_STEP_PUBLICATIONS,
    FACT_SOURCE_CLOSURE_MAX_REFERENCES,
};
pub use self::history::{
    QualifiedRunStore, RunHistoryReader, RunHistoryReadinessBackend, RunHistoryWriter,
};
#[cfg(any(test, feature = "backend-conformance"))]
pub use self::history::{RunHistoryAdmissionLockTestBackend, RunHistoryCommitFailureBackend};
pub use self::journal::{
    derive_initial_run_state_digest, verify_offline_recorded_history,
    verify_offline_recorded_material, CommittedJournalCommit, CommittedJournalRecord,
    CommittedRunJournal, FoldedAccessAuditEntry, FoldedPendingEffect, FoldedTransitionEntry,
    JournalLoadVerifier, NodeTerminalOutcome, PendingEffectStatus, TransitionFrame,
    TransitionFrameReader, VerifiedAccessAttempt, VerifiedAccessAuditEntry, VerifiedAdmissionRoot,
    VerifiedAdmissionSourceRequirements, VerifiedCrossRunSourceRequirement,
    VerifiedNodeAccessHistory, VerifiedObservedAccess, VerifiedQualifiedSupportRoot,
    VerifiedRecordedConfiguredValue, VerifiedRunView,
};
pub use self::objects::{
    CommittedObject, ObjectAuthorityKey, PreparedObjectGraph, PreparedObjectPayload,
    UntrustedObjectPayload,
};
pub use self::preparation::{
    AdmissionMaterial, AuthorizationMaterial, ExistingRunAppendMaterial, ObjectGraphMember,
    ObjectGraphProposal, ObservationMaterial, PreparedFrame, PreparedValue, ProducedObjectRoot,
    ProducedOutputSlot, ProposedAdmissionInput, ReadObservationMaterial, SafeFailureMetadata,
    SettlementMaterial, TransitionMaterial, VerifiedAdmissionSources,
};
pub use self::public_read::VerifiedPublicRunView;
pub use self::support::{
    AdmittedSupportGraph, AdmittedSupportMember, PreparedSupportGraph, PreparedSupportMember,
    QualifiedSupportGraph, QualifiedSupportMember, SupportBackend, SupportGraphAdmissionVerifier,
};
pub use self::trace::{
    TransitionTracePageRequest, TransitionTraceSourceRequirements, VerifiedTransitionTrace,
    VerifiedTransitionTracePage,
};

/// Result type for recoverability-v3 store operations.
pub type Result<T> = std::result::Result<T, StoreError>;
