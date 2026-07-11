use super::*;

checked_string_type!(
    /// Store-owned opaque claim fencing token.
    ClaimFencingToken,
    "claim fencing token"
);

/// Side-effect intent persisted event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentPersisted {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Node id.
    pub node_id: NodeId,
    /// Scope id.
    pub scope_id: ScopeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Side-effect ledger key.
    pub ledger_key: SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: SideEffectLedgerPurpose,
    /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
    pub pair_id: SideEffectPairId,
    /// Pair phase authority, when this ledger is bound to a paired forward node.
    pub pair_role: SideEffectPairRole,
    /// Invocation epoch.
    pub invocation_epoch: u32,
    /// Intent schema id.
    pub intent_schema_id: SchemaId,
    /// Intent content hash.
    pub intent_hash: ContentDigest,
    /// Intent artifact id.
    pub intent_artifact_id: ArtifactId,
    /// Exact retained-artifact evidence identity for the intent artifact.
    pub intent_artifact_evidence_hash: ContentDigest,
    /// Idempotency input schema id.
    pub idempotency_input_schema_id: SchemaId,
    /// Idempotency input hash.
    pub idempotency_input_hash: ContentDigest,
    /// Idempotency key.
    pub idempotency_key: IdempotencyKeyRef,
    /// Capability kind.
    pub capability_kind: CapabilityKind,
    /// Capability version.
    pub capability_version: CapabilityVersion,
    /// Adapter kind.
    pub adapter_kind: AdapterKind,
    /// Adapter version.
    pub adapter_version: AdapterVersion,
}

/// Side-effect claim acquired event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claimed {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Node id.
    pub node_id: NodeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Side-effect ledger key.
    pub ledger_key: SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: SideEffectLedgerPurpose,
    /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
    pub pair_id: SideEffectPairId,
    /// Pair phase authority, when this ledger is bound to a paired forward node.
    pub pair_role: SideEffectPairRole,
    /// Claim owner.
    pub claim_owner: RunnerInvocationId,
    /// Invocation epoch.
    pub invocation_epoch: u32,
    /// Claim generation.
    pub claim_generation: u32,
    /// Claim fencing token.
    pub claim_fencing_token: ClaimFencingToken,
}

/// Side-effect claim takeover event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimTakenOver {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Node id.
    pub node_id: NodeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Side-effect ledger key.
    pub ledger_key: SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: SideEffectLedgerPurpose,
    /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
    pub pair_id: SideEffectPairId,
    /// Pair phase authority, when this ledger is bound to a paired forward node.
    pub pair_role: SideEffectPairRole,
    /// Previous claim owner.
    pub previous_claim_owner: RunnerInvocationId,
    /// New claim owner.
    pub new_claim_owner: RunnerInvocationId,
    /// Invocation epoch.
    pub invocation_epoch: u32,
    /// Previous claim generation.
    pub previous_claim_generation: u32,
    /// New claim generation.
    pub claim_generation: u32,
    /// Claim fencing token.
    pub claim_fencing_token: ClaimFencingToken,
}

/// Side-effect invocation prepared event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationPrepared {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Node id.
    pub node_id: NodeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Side-effect ledger key.
    pub ledger_key: SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: SideEffectLedgerPurpose,
    /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
    pub pair_id: SideEffectPairId,
    /// Pair phase authority, when this ledger is bound to a paired forward node.
    pub pair_role: SideEffectPairRole,
    /// Invocation epoch.
    pub invocation_epoch: u32,
    /// Claim generation.
    pub claim_generation: u32,
    /// Claim fencing token.
    pub claim_fencing_token: ClaimFencingToken,
    /// Optional exclusive resource lane key evidence echoed from the held lane.
    pub resource_key: Option<ResourceKeyEvidence>,
    /// Optional prepared artifact id.
    pub prepared_artifact_id: Option<ArtifactId>,
    /// Optional prepared artifact content hash.
    pub prepared_hash: Option<ContentDigest>,
    /// Exact retained-artifact evidence identity for the prepared artifact, when any.
    pub prepared_artifact_evidence_hash: Option<ContentDigest>,
}

/// Side-effect invocation started event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationStarted {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Node id.
    pub node_id: NodeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Side-effect ledger key.
    pub ledger_key: SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: SideEffectLedgerPurpose,
    /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
    pub pair_id: SideEffectPairId,
    /// Pair phase authority, when this ledger is bound to a paired forward node.
    pub pair_role: SideEffectPairRole,
    /// Invocation epoch.
    pub invocation_epoch: u32,
    /// Claim owner.
    pub claim_owner: RunnerInvocationId,
    /// Claim generation.
    pub claim_generation: u32,
    /// Claim fencing token.
    pub claim_fencing_token: ClaimFencingToken,
}

/// Side-effect not-submitted proof event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotSubmittedProven {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Node id.
    pub node_id: NodeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Side-effect ledger key.
    pub ledger_key: SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: SideEffectLedgerPurpose,
    /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
    pub pair_id: SideEffectPairId,
    /// Pair phase authority, when this ledger is bound to a paired forward node.
    pub pair_role: SideEffectPairRole,
    /// Invocation epoch.
    pub invocation_epoch: u32,
    /// Proof schema id.
    pub proof_schema_id: SchemaId,
    /// Proof content hash.
    pub proof_hash: ContentDigest,
    /// Proof artifact id.
    pub proof_artifact_id: ArtifactId,
    /// Exact retained-artifact evidence identity for the proof artifact.
    pub proof_artifact_evidence_hash: ContentDigest,
}

/// Side-effect submission observed event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionObserved {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Node id.
    pub node_id: NodeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Side-effect ledger key.
    pub ledger_key: SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: SideEffectLedgerPurpose,
    /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
    pub pair_id: SideEffectPairId,
    /// Pair phase authority, when this ledger is bound to a paired forward node.
    pub pair_role: SideEffectPairRole,
    /// Invocation epoch.
    pub invocation_epoch: u32,
    /// Submission schema id.
    pub submission_schema_id: SchemaId,
    /// Submission content hash.
    pub submission_hash: ContentDigest,
    /// Submission artifact id.
    pub submission_artifact_id: ArtifactId,
    /// Exact retained-artifact evidence identity for the submission artifact.
    pub submission_artifact_evidence_hash: ContentDigest,
}

/// Side-effect submission unknown event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionUnknown {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Node id.
    pub node_id: NodeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Side-effect ledger key.
    pub ledger_key: SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: SideEffectLedgerPurpose,
    /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
    pub pair_id: SideEffectPairId,
    /// Pair phase authority, when this ledger is bound to a paired forward node.
    pub pair_role: SideEffectPairRole,
    /// Invocation epoch.
    pub invocation_epoch: u32,
    /// Evidence schema id.
    pub evidence_schema_id: SchemaId,
    /// Evidence content hash.
    pub evidence_hash: ContentDigest,
    /// Evidence artifact id.
    pub evidence_artifact_id: ArtifactId,
    /// Exact retained-artifact evidence identity for the evidence artifact.
    pub evidence_artifact_evidence_hash: ContentDigest,
}

/// Side-effect receipt observed event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptObserved {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Node id.
    pub node_id: NodeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Side-effect ledger key.
    pub ledger_key: SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: SideEffectLedgerPurpose,
    /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
    pub pair_id: SideEffectPairId,
    /// Pair phase authority, when this ledger is bound to a paired forward node.
    pub pair_role: SideEffectPairRole,
    /// Invocation epoch.
    pub invocation_epoch: u32,
    /// Receipt schema id.
    pub receipt_schema_id: SchemaId,
    /// Receipt content hash.
    pub receipt_hash: ContentDigest,
    /// Receipt artifact id.
    pub receipt_artifact_id: ArtifactId,
    /// Exact retained-artifact evidence identity for the receipt artifact.
    pub receipt_artifact_evidence_hash: ContentDigest,
    /// Replay verifier id.
    pub replay_verifier_id: ReplayVerifierId,
    /// Optional exact touched-set evidence.
    pub resource_touched_set: Option<ResourceTouchedSetEvidence>,
}

/// Side-effect confirmation observed event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationObserved {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Node id.
    pub node_id: NodeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Side-effect ledger key.
    pub ledger_key: SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: SideEffectLedgerPurpose,
    /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
    pub pair_id: SideEffectPairId,
    /// Pair phase authority, when this ledger is bound to a paired forward node.
    pub pair_role: SideEffectPairRole,
    /// Invocation epoch.
    pub invocation_epoch: u32,
    /// Confirmation schema id.
    pub confirmation_schema_id: SchemaId,
    /// Confirmation content hash.
    pub confirmation_hash: ContentDigest,
    /// Confirmation artifact id.
    pub confirmation_artifact_id: ArtifactId,
    /// Exact retained-artifact evidence identity for the confirmation artifact.
    pub confirmation_artifact_evidence_hash: ContentDigest,
    /// Replay verifier id.
    pub replay_verifier_id: ReplayVerifierId,
    /// Optional exact touched-set evidence.
    pub resource_touched_set: Option<ResourceTouchedSetEvidence>,
}

/// Side-effect ambiguous event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ambiguous {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Node id.
    pub node_id: NodeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Side-effect ledger key.
    pub ledger_key: SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: SideEffectLedgerPurpose,
    /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
    pub pair_id: SideEffectPairId,
    /// Pair phase authority, when this ledger is bound to a paired forward node.
    pub pair_role: SideEffectPairRole,
    /// Invocation epoch.
    pub invocation_epoch: u32,
    /// Ambiguity code.
    pub ambiguity_code: AmbiguityCode,
    /// Evidence schema id.
    pub evidence_schema_id: SchemaId,
    /// Evidence content hash.
    pub evidence_hash: ContentDigest,
    /// Evidence artifact id.
    pub evidence_artifact_id: ArtifactId,
    /// Exact retained-artifact evidence identity for the evidence artifact.
    pub evidence_artifact_evidence_hash: ContentDigest,
}

/// Side-effect failed event payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failed {
    /// Certified typed spec hash.
    pub spec_hash: SpecHash,
    /// Node id.
    pub node_id: NodeId,
    /// Attempt id.
    pub attempt_id: AttemptId,
    /// Side-effect ledger key.
    pub ledger_key: SideEffectLedgerKey,
    /// Side-effect ledger purpose.
    pub ledger_purpose: SideEffectLedgerPurpose,
    /// Certified side-effect pair id, when this ledger is bound to a paired forward node.
    pub pair_id: SideEffectPairId,
    /// Pair phase authority, when this ledger is bound to a paired forward node.
    pub pair_role: SideEffectPairRole,
    /// Invocation epoch.
    pub invocation_epoch: u32,
    /// Failure phase.
    pub failure_phase: FailurePhase,
    /// Whether retry is allowed.
    pub retryable: bool,
    /// Redaction-safe error information.
    pub error: MfmErrorInfo,
}

/// Legal side-effect failure phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FailurePhase {
    /// Failure before invocation started.
    BeforeInvocationStarted,
    /// Failure after not-submitted was proven.
    AfterNotSubmittedProven,
}
