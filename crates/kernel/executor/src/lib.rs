#![warn(missing_docs)]
//! Recoverable keyed-executor contracts and conformance substrate.
//!
//! An executor ledger is a separate authority from the MFM run journal. It
//! durably binds one effect key to one request, allocates any shared external
//! resource before target entry, and records a bounded append-only delivery
//! history. Stateless transports receive an affine target-entry authorization;
//! they do not own persistence, locks, or retry policy.
//!
//! The memory backend and reference queue in this crate exercise the complete
//! protocol without claiming production durability. A production backend must
//! preserve the exact ledger generation across recovery or rely on an
//! authoritative destination fence that permanently rejects old generations.
//! Verified ensure results carry one descriptor-bound, producer-free retained
//! closure. Runtime alone constructs the outer journal values and assigns
//! external-observation producer authority.

mod codec;
mod contract;
mod engine;
mod frontier;
mod ledger;
mod policy;
mod reference;
mod retained;

pub use contract::{
    AllocationStateRef, CanonicalExecutorRequest, CommittedEffectRequest, EffectIdentity,
    ExecutorBinding, ExecutorBindingRef, ExecutorContractDescriptor, ExecutorDeployment,
    ExecutorDeploymentRef, ExecutorFuture, ExecutorRetainedClosureContract, FencingRef,
    RequiredPlanExpansion, ResourceKeyRef, ResourceOwnership, ResourceOwnershipRef,
    SchemaQualifiedCanonicalValue, VerifiedExecutorBinding,
};
pub use engine::{
    ExecutorAppendOutcome, ExecutorEffectSnapshot, ExecutorLedgerAppend, ExecutorLedgerStore,
    ExecutorLedgerStoreIdentity, ExecutorResourceAppend, ExecutorResourceSnapshot,
    ExecutorStoreSnapshot, KeyedExecutorLedger,
};
pub use frontier::{
    derive_attempt_id, reference_safe_failure, verify_reference_safe_failure_tuple, AdmitFrontier,
    DeliveryAttemptOutcome, DeliveryAttemptView, DeliveryAudit, DeliveryAuditAccumulator,
    DeliveryAuditFrontier, DeliveryAuditFrontierRef, EvidenceBounds, ExecutorEvidenceRecord,
    ExecutorLedgerRecordRef, FrontierProof, ReferenceFailureCode, ReferenceSafeFailure,
    ReferenceTerminalProof, ResourceAllocatedRecord, ReturnedOutcome, TerminalTombstone,
    TerminalTombstoneRef,
};
pub use ledger::{
    AllocationOutcome, EffectEntryView, MemoryExecutorStore, MemoryLedgerCheckpoint,
    ResourceAllocationEvidence, ResourceLedgerRecord, ResourceLedgerRecordRef, ResourceStreamView,
    TargetEntryAuthority, TargetOperationReceipt,
};
pub use mfm_capabilities::{
    BoundaryStage, CoarseSizeClass, FailureClass, SafeFailure, SafeFailureCode, SafeFailureError,
};
pub use mfm_ids::{AttemptId, ContentRef, EffectKey, RequestDigest, SemanticDigest, TenantScopeId};
pub use mfm_values::RetainedValueContract;
pub use policy::{
    AccountSequenceAllocation, AccountSequencePolicy, AccountSequenceRequest,
    FiniteInventoryAllocation, FiniteInventoryPolicy, FiniteInventoryRequest, PolicyDecision,
    PolicyError, ResourcePolicyBinding, TypedResourcePolicy,
};
pub use reference::{
    MemoryConvergentDestination, MemoryDestinationCheckpoint, ReferenceContract,
    ReferenceCrashPoint, ReferenceDestination, ReferenceDestinationReturn, ReferenceDriveOutcome,
    ReferenceExecutor, ReferenceRequest, ReferenceTargetBehavior,
};
pub use retained::{
    verify_ensure_result, verify_retained_delivery_audit, verify_retained_terminal_evidence,
    EffectExecutorOutcome, EffectExecutorOutcomeView, Ensure, ExecutorEnsureResultClaim,
    ExecutorRetainedClosureClaim, ExecutorRetainedValue, ExecutorRetainedValueRelation,
    ExecutorTerminalEvidenceClaim, ProofBasis, VerifiedEnsureResult,
    VerifiedExecutorRetainedClosure, VerifiedTerminalEvidence,
};

/// Result type for executor contract and substrate operations.
pub type Result<T> = std::result::Result<T, ExecutorError>;

/// Closed error taxonomy for executor contract and substrate failures.
///
/// Variants intentionally carry no arbitrary provider diagnostics, paths,
/// endpoints, or other unreviewed strings.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExecutorError {
    /// A frozen recoverability annex operation failed with this closed code.
    #[error("recoverability contract rejected executor data: {0:?}")]
    Recoverability(mfm_canonical::RecoverabilityErrorCode),
    /// A fixed canonical object could not be constructed.
    #[error("executor canonical encoding failed")]
    CanonicalEncoding,
    /// A supplied lightweight reference names the wrong frozen schema.
    #[error("executor schema reference mismatch")]
    SchemaReferenceMismatch,
    /// An executor binding names a different deployment object.
    #[error("executor deployment reference mismatch")]
    DeploymentReferenceMismatch,
    /// An executor binding names a different semantic contract descriptor.
    #[error("executor contract descriptor reference mismatch")]
    ExecutorContractReferenceMismatch,
    /// A deployment and resource-ownership object disagree.
    #[error("executor resource ownership reference mismatch")]
    ResourceOwnershipReferenceMismatch,
    /// Deployment and resource ownership use different durable generations.
    #[error("executor durable ledger generation mismatch")]
    LedgerGenerationMismatch,
    /// The resolved resource owner does not satisfy the contract's exact domain.
    #[error("executor resource domain requirement mismatch")]
    ResourceDomainMismatch,
    /// A deployment or request names another tenant partition.
    #[error("executor tenant scope mismatch")]
    TenantScopeMismatch,
    /// A caller used a binding other than the ledger's exact immutable binding.
    #[error("wrong executor binding")]
    WrongExecutorBinding,
    /// A supplied effect key does not match its immutable derivation inputs.
    #[error("effect key derivation mismatch")]
    EffectKeyMismatch,
    /// An existing effect key is bound to another request or binding.
    #[error("effect binding conflict")]
    EffectBindingConflict,
    /// The requested effect is not present in the executor ledger.
    #[error("effect is not bound")]
    EffectNotBound,
    /// A terminal effect cannot authorize a new target attempt.
    #[error("effect is already terminal")]
    EffectAlreadyTerminal,
    /// An effect already has an incompatible resource allocation.
    #[error("effect resource allocation conflict")]
    ResourceAllocationConflict,
    /// Resource coordination was requested without admitted ownership.
    #[error("executor binding has no resource ownership")]
    ResourceOwnershipRequired,
    /// A resource compare-and-swap predecessor is stale.
    #[error("resource stream compare-and-swap mismatch")]
    ResourceCasMismatch,
    /// Restored allocation state has not matched its exact policy pair.
    #[error("typed resource policy was not revalidated")]
    ResourcePolicyNotRevalidated,
    /// A resource policy rejected the allocation.
    #[error("typed resource policy rejected allocation: {0}")]
    ResourcePolicy(PolicyError),
    /// The append would exceed the contract's immutable evidence bounds.
    #[error("executor evidence bounds exhausted")]
    EvidenceBoundsExhausted,
    /// A restored checkpoint names different fixed evidence bounds.
    #[error("executor evidence bounds mismatch")]
    EvidenceBoundsMismatch,
    /// A delivery frontier is empty or structurally invalid.
    #[error("invalid delivery frontier structure")]
    InvalidFrontier,
    /// A delivery frontier proof or content reference is invalid.
    #[error("invalid delivery frontier proof")]
    InvalidFrontierProof,
    /// A referenced retained executor object was not supplied.
    #[error("referenced retained executor object is missing")]
    RetainedObjectMissing,
    /// Retained executor bytes do not match their schema-qualified identity.
    #[error("retained executor object identity mismatch")]
    RetainedObjectMismatch,
    /// Retained metadata does not equal the exact binding-selected contract.
    #[error("retained executor value contract mismatch")]
    RetainedValueContractMismatch,
    /// The retained closure repeats one exact relation/content member.
    #[error("retained executor closure contains a duplicate member")]
    RetainedClosureDuplicate,
    /// The retained closure omits a member reachable from its verified root.
    #[error("retained executor closure is incomplete")]
    RetainedClosureIncomplete,
    /// The retained closure contains an unreachable non-frontier member.
    #[error("retained executor closure contains an extra member")]
    RetainedClosureExtra,
    /// A frontier is not a prefix-relative ancestor or descendant.
    #[error("delivery frontier fork")]
    FrontierFork,
    /// A delivery observation does not name one unmatched authorization.
    #[error("delivery observation is unmatched or duplicated")]
    InvalidDeliveryObservation,
    /// An attempt identity does not match its deterministic derivation.
    #[error("delivery attempt identity mismatch")]
    AttemptIdentityMismatch,
    /// A reference safe-failure tuple violates its frozen relation.
    #[error("invalid executor safe failure")]
    InvalidSafeFailure,
    /// Terminal proof does not match its exact returned observation.
    #[error("terminal proof does not match returned observation")]
    TerminalProofMismatch,
    /// A terminal tombstone conflicts with the immutable existing tombstone.
    #[error("terminal tombstone conflict")]
    TerminalTombstoneConflict,
    /// Terminal evidence was requested without a returned target observation.
    #[error("terminal tombstone lacks returned target evidence")]
    TerminalEvidenceMissing,
    /// A destination authority was consumed or presented more than once.
    #[error("target-entry authority already consumed")]
    TargetAuthorityConsumed,
    /// The semantic destination already contains an incompatible operation.
    #[error("semantic destination operation conflict")]
    DestinationOperationConflict,
    /// The destination permanently rejects this ledger generation.
    #[error("executor ledger generation is fenced")]
    DestinationGenerationFenced,
    /// A destination-generation transition did not name its current generation.
    #[error("destination fence compare-and-swap mismatch")]
    DestinationFenceMismatch,
    /// A reference request used a target operation outside its contract.
    #[error("reference target operation mismatch")]
    TargetOperationMismatch,
    /// A reference effect identifier violates its retained 256-byte bound.
    #[error("reference effect identifier is invalid")]
    InvalidReferenceEffectIdentifier,
    /// A deterministic conformance crash was injected.
    #[error("reference executor conformance crash injected")]
    ReferenceCrashInjected,
    /// A memory synchronization primitive was poisoned.
    #[error("executor memory synchronization failed")]
    SynchronizationFailure,
    /// A durable backend could not complete its reviewed IO protocol.
    #[error("executor durable backend unavailable")]
    DurableBackendUnavailable,
    /// A durable append may have committed but no positive acknowledgement survived.
    #[error("executor durable append outcome is unknown")]
    DurableAppendOutcomeUnknown,
    /// Durable executor bytes are corrupt, truncated, oversized, or incompatible.
    #[error("executor durable snapshot is invalid")]
    InvalidDurableSnapshot,
}
