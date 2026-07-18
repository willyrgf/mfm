use std::collections::BTreeMap;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_events::v1::{self as events, side_effect, ArtifactRole, KernelEventPayload};
use mfm_ids::{
    AdapterVersion, ArtifactId, AttemptId, CapabilityVersion, CellId, ContentDigest,
    DigestAlgorithm, EffectKind, EffectVersion, LoweringVersion, NodeId, RunId, RuntimeBindingId,
    SchemaId, SeedId, SideEffectPairId, SpecHash, SpecVersion, StableAuthorKey, StateVersion,
};
use mfm_manual_auth::{
    manual_authorization_proof_schema_id, ManualAuthorizationSignatureBytes,
    ManualResolutionAuthorizationClaim, ManualResolutionAuthorizationProof,
    ManualResolutionAuthorizationSignature, ManualResolutionBlockReason,
    ManualResolutionEvidenceRef, ManualResolutionPrefixAuthority, ManualResolutionProofAuthority,
    VerifiedManualResolutionForPrefix,
};
use mfm_spec::v1::{
    self as spec, CanonicalizerIdentity, CellProducer, ManualResolutionEvidenceSpec,
    PublicFieldPath, RemediationUnresolvedSpec, ResourceNamespace, SagaPolicySpec, ValueLineageRef,
};
use mfm_store::v1::test_support::{
    confirmation_terminal_policies_for_projection_for_test as confirmation_terminal_policies_for_projection,
    empty_terminal_policies_for_test as empty_terminal_policies,
    fact_descriptor_projection_fixture_for_test, fixed_adapter_kind_for_test as adapter_kind,
    fixed_artifact_id_for_test as artifact_id, fixed_attempt_id_for_test as attempt_id,
    fixed_capability_kind_for_test as capability_kind, fixed_cell_id_for_test as cell_id,
    fixed_content_digest_for_test as content_digest, fixed_descriptor_id_for_test as descriptor_id,
    fixed_digest_bytes_for_test as digest_bytes, fixed_event_id_for_test as event_id,
    fixed_node_id_for_test as node_id, fixed_schema_id_for_test as schema_id,
    fixed_scope_id_for_test as scope_id, fixed_semantic_type_id_for_test as semantic_id,
    fixed_spec_hash_for_test as spec_hash, fixed_state_kind_for_test as state_kind,
    media_type_for_test as media_type, poll_ready_store_future_for_test as poll_ready_store_future,
    prepared_commit_bundle_from_plan as test_bundle_from_plan,
    prepared_commit_plan_for_test as test_prepared_commit_plan,
    receipt_terminal_policies_for_projection_for_test as receipt_terminal_policies_for_projection,
    run_identity_material_for_test,
};
use mfm_store::v1::{
    admission_advisory_lock_key, admission_waiter_id, committed_run_stream_canonical_json,
    committed_run_stream_from_canonical_json_slice, payload_canonical_json,
    payload_from_json_value, resource_wait_fifo_admission_token, AdmissionLaneClass,
    AdmissionLaneMode, AdmissionToken, ArtifactByteAuthorityMap, ArtifactEvidenceRef,
    AsyncInMemoryRunStore, AttemptStatus, AttemptTerminal, CellTerminalProjection,
    CertifiedRunStoreAuthority, CommitArtifactEvidenceSet, CommitKey, CommitOutcome,
    CommitPreconditions, CommitRequest, CommittedRunStream, EventArtifactReferenceSource,
    ExecutionClaimAdmissionLane, ExecutionClaimScope, ExistingArtifactAdmission,
    ForwardLedgerClassification, KernelEventEnvelope, ManualBlockReason, ManualResolution,
    ManualResolutionProjection, PreparedArtifactBytes, PreparedCommit, PreparedCommitBundle,
    PreparedCommitPlan, ProjectionSnapshot, PublicOutputProjection, RequiredRunState,
    ResourceAdmissionLane, ResourceLaneKey, RunAdmission, RunCompletionProjection, RunEventStore,
    RunMode, RunState, SagaEngagementProjection, SagaEngagementReason, SagaTerminal,
    SagaTerminalProof, SideEffectLedgerPhase, SideEffectPairLedgerRef, SideEffectPhase,
    SideEffectTerminal, StateAttemptStarted, StoreError, StoreScopeId, StoreScopeStore, StreamSeq,
    EXECUTION_CLAIM_HEARTBEAT_INTERVAL_SECS, EXECUTION_CLAIM_LEASE_TTL_SECS,
};

#[path = "commit_contract/support.rs"]
mod support;
