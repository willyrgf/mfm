use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_events::v1::{self as events, side_effect, ArtifactRole, KernelEventPayload};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion, CellId,
    ContentDigest, DescriptorId, DigestAlgorithm, DigestBytes, EventId, LoweringVersion, NodeId,
    RunId, SchemaId, ScopeId, SeedId, SemanticTypeId, SpecHash, SpecVersion, StateKind,
    StateVersion,
};
use mfm_manual_auth::{
    manual_authorization_proof_schema_id, ManualAuthorizationSignatureBytes,
    ManualResolutionAuthorizationClaim, ManualResolutionAuthorizationProof,
    ManualResolutionAuthorizationSignature, ManualResolutionBlockReason,
    ManualResolutionEvidenceRef, ManualResolutionPrefixAuthority, ManualResolutionProofAuthority,
    VerifiedManualResolutionForPrefix,
};
use mfm_spec::v1::{
    self as spec, CanonicalizerIdentity, CellProducer, ManualResolutionEvidenceSpec, MediaType,
    PublicFieldPath, RemediationUnresolvedSpec, ResourceNamespace, SagaPolicySpec, ValueLineageRef,
};
use mfm_store::v1::{
    build_committed_batch, event_artifact_requirements, payload_canonical_json,
    payload_from_json_value, ArtifactEvidenceRef, AsyncInMemoryTypedRunStore, AsyncStoreFuture,
    AsyncTypedRunEventStore, AttemptStatus, AttemptTerminal, CellTerminalProjection,
    CommitArtifactEvidenceSet, CommitKey, CommitOrdinal, CommitOutcome, CommitPreconditions,
    CommittedRunStream, EventArtifactReferenceSource, ForwardLedgerClassification,
    InMemoryTypedRunStore, KernelEventEnvelope, ManualBlockReason, ManualResolution,
    ManualResolutionProjection, NonEmptyPayloadBatch, PersistedKernelEventRecord, PreparedCommit,
    PreparedCommitPlan, ProjectionSnapshot, PublicOutputProjection, RequiredRunState,
    ResourceLaneKey, Retention, RunAdmission, RunCompletionProjection, RunMode, RunState,
    SagaAdmitToken, SagaEngagementProjection, SagaEngagementReason, SagaTerminal,
    SagaTerminalProof, SideEffectLedgerPhase, SideEffectPhase, SideEffectProgress,
    SideEffectTerminal, StateAttemptStarted, StoreError, StreamSeq, TypedCommitRequest,
    TypedProjectionRead, TypedRunEventStore, VerifiedRetentionProjection,
    VerifiedRetentionProjectionSet,
};

const SPEC_MEDIA_TYPE: &str = "application/vnd.mfm.typed-execution-spec+json;version=1";

macro_rules! typed_commit_request {
    (
        run_id: $run_id:expr,
        expected_next_seq: $expected_next_seq:expr,
        commit_key: $commit_key:expr,
        payloads: $payloads:expr,
        required_artifacts: $required_artifacts:expr,
        preconditions: $preconditions:expr $(,)?
    ) => {
        TypedCommitRequest::from_payloads(
            $run_id,
            $expected_next_seq,
            $commit_key,
            $payloads,
            $required_artifacts,
            $preconditions,
        )
        .expect("typed commit request")
    };
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_ready_store_future<T, E>(
    mut future: AsyncStoreFuture<'_, T, E>,
) -> std::result::Result<T, E> {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    match Future::poll(future.as_mut(), &mut context) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("async in-memory store future should be ready"),
    }
}

fn digest_bytes(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

fn content_digest(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn spec_hash(byte: u8) -> SpecHash {
    SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn run_id(byte: u8) -> RunId {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn artifact_id(byte: u8) -> ArtifactId {
    ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn attempt_id(byte: u8) -> AttemptId {
    AttemptId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn node_id(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn event_id(byte: u8) -> EventId {
    EventId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn descriptor_id(byte: u8) -> DescriptorId {
    DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn cell_id(byte: u8) -> CellId {
    CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn scope_id(byte: u8) -> ScopeId {
    ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn schema_id(name: &str, byte: u8) -> SchemaId {
    SchemaId::new(name, "1", DigestAlgorithm::Sha256JcsV1, digest_bytes(byte)).expect("schema id")
}

fn semantic_id(name: &str, byte: u8) -> SemanticTypeId {
    SemanticTypeId::new(
        "mfm.test",
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(byte),
    )
    .expect("semantic id")
}

fn state_kind(byte: u8) -> StateKind {
    StateKind::new(
        "mfm.test",
        "state",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(byte),
    )
    .expect("state kind")
}

fn capability_kind(byte: u8) -> CapabilityKind {
    CapabilityKind::new(
        "mfm.test",
        "capability",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(byte),
    )
    .expect("capability kind")
}

fn adapter_kind(byte: u8) -> AdapterKind {
    AdapterKind::new(
        "mfm.test",
        "adapter",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(byte),
    )
    .expect("adapter kind")
}

fn media_type(value: &str) -> MediaType {
    MediaType::new(value).expect("media type")
}

fn run_admitted(run_id: RunId) -> KernelEventPayload {
    run_admitted_with_saga_policy(run_id, &SagaPolicySpec::NoSideEffects)
}

fn run_admitted_with_saga_policy(
    run_id: RunId,
    saga_policy: &SagaPolicySpec,
) -> KernelEventPayload {
    let spec_artifact = spec_artifact_ref();
    let certificate_artifact = certificate_artifact_ref();
    KernelEventPayload::RunAdmitted(Box::new(events::RunAdmitted {
        run_id,
        spec_hash: spec_hash(1),
        spec_artifact: run_artifact_ref(&spec_artifact),
        certificate_artifact: run_artifact_ref(&certificate_artifact),
        config_artifacts: Vec::new(),
        spec_version: SpecVersion::new("mfm.typed.execution_spec.v1").expect("spec version"),
        lowering_version: LoweringVersion::new("mfm.typed.lowering.v1").expect("lowering version"),
        public_output_schema_id: schema_id("mfm.test.public_output", 3),
        saga_policy_digest: saga_policy
            .saga_policy_digest()
            .expect("saga policy digest"),
        descriptor_identities: Vec::new(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        admitted_binding_digest: content_digest(9),
        canonicalizer_identity: CanonicalizerIdentity::new("mfm.jcs.v1").expect("canonicalizer"),
        framework_version: events::FrameworkVersion::new("mfm.test.1").expect("framework version"),
        source_revision: events::SourceRevision::new("test-revision").expect("source revision"),
        launched_at_unix_ms: 1_700_000_000_000,
        seed_cells: Vec::new(),
    }))
}

fn state_attempt_started() -> KernelEventPayload {
    KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
        spec_hash: spec_hash(1),
        node_id: node_id(20),
        attempt_id: attempt_id(23),
        attempt_no: 1,
        state_kind: state_kind(12),
        state_version: StateVersion::new("mfm.test.state.v1").expect("state version"),
    })
}

fn state_attempt_completed() -> KernelEventPayload {
    KernelEventPayload::StateAttemptCompleted(events::StateAttemptCompleted {
        spec_hash: spec_hash(1),
        node_id: node_id(20),
        attempt_id: attempt_id(23),
        output_cell_id: cell_id(21),
    })
}

fn state_attempt_interrupted() -> KernelEventPayload {
    KernelEventPayload::StateAttemptInterrupted(events::StateAttemptInterrupted {
        spec_hash: spec_hash(1),
        node_id: node_id(20),
        attempt_id: attempt_id(23),
    })
}

fn cell_produced(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    KernelEventPayload::CellProduced(events::CellProduced {
        spec_hash: spec_hash(1),
        node_id: node_id(20),
        cell_id: cell_id(21),
        scope_id: scope_id(22),
        attempt_id: attempt_id(23),
        semantic_type_id: semantic_id("position", 24),
        schema_id: schema_id("mfm.test.position", 25),
        value_lineage: ValueLineageRef {
            lineage_digest: content_digest(26),
        },
        artifact_id,
        content_digest: digest,
        producer_state_kind: None,
        producer_state_version: None,
    })
}

fn terminal_cell_commit_payloads(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> Vec<KernelEventPayload> {
    vec![
        cell_produced(artifact_id, digest),
        state_attempt_completed(),
    ]
}

fn public_output_produced(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    KernelEventPayload::PublicOutputProduced(events::PublicOutputProduced {
        spec_hash: spec_hash(1),
        node_id: node_id(20),
        attempt_id: attempt_id(23),
        receipt_cell_id: cell_id(21),
        public_schema_id: schema_id("mfm.test.public_output", 3),
        output_spec_digest: content_digest(27),
        cells: vec![events::NamedTypedCellRef {
            public_field_path: PublicFieldPath::new("result").expect("field path"),
            cell_id: cell_id(21),
            producer: CellProducer::Node(node_id(20)),
            scope_id: scope_id(22),
            semantic_type_id: semantic_id("position", 24),
            schema_id: schema_id("mfm.test.position", 25),
            value_lineage: ValueLineageRef {
                lineage_digest: content_digest(26),
            },
            content_digest: digest,
            artifact_id,
        }],
        rendered_digest: content_digest(28),
        rendered_artifact_id: None,
        renderer_descriptor_id: descriptor_id(29),
    })
}

fn public_output_produced_with_rendered_artifact(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    rendered_artifact_id: ArtifactId,
    rendered_digest: ContentDigest,
) -> KernelEventPayload {
    let mut payload = public_output_produced(artifact_id, digest);
    let KernelEventPayload::PublicOutputProduced(public_output) = &mut payload else {
        unreachable!("helper returns public-output payload");
    };
    public_output.rendered_digest = rendered_digest;
    public_output.rendered_artifact_id = Some(rendered_artifact_id);
    payload
}

fn event_artifact_ref(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> events::ArtifactEvidenceRef {
    events::ArtifactEvidenceRef {
        artifact_id,
        role: ArtifactRole::StateOutput,
        schema_id: schema_id("mfm.test.position", 25),
        semantic_type_id: Some(semantic_id("position", 24)),
        content_digest: digest,
        byte_len: 128,
        media_type: media_type("application/json"),
    }
}

fn store_artifact_ref(artifact_id: ArtifactId, digest: ContentDigest) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 128,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id("mfm.test.position", 25)),
        semantic_type_id: Some(semantic_id("position", 24)),
        producer_node_id: Some(node_id(20)),
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::StateOutput,
    }
}

fn public_output_artifact_ref(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 256,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id("mfm.test.public_output", 3)),
        semantic_type_id: None,
        producer_node_id: Some(node_id(20)),
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::PublicOutput,
    }
}

fn intent_artifact_ref(artifact_id: ArtifactId, digest: ContentDigest) -> ArtifactEvidenceRef {
    intent_artifact_ref_for_node(artifact_id, digest, node_id(70))
}

fn intent_artifact_ref_for_node(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    producer_node_id: NodeId,
) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 256,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id("mfm.test.side_effect_intent", 70)),
        semantic_type_id: None,
        producer_node_id: Some(producer_node_id),
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::SideEffectIntent,
    }
}

fn side_effect_artifact_ref(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    schema_id: SchemaId,
    artifact_role: ArtifactRole,
) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 256,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id),
        semantic_type_id: None,
        producer_node_id: Some(node_id(70)),
        producer_seed_id: None::<SeedId>,
        artifact_role,
    }
}

fn side_effect_ledger_key() -> events::SideEffectLedgerKey {
    events::SideEffectLedgerKey::new("ledger-key-1").expect("ledger key")
}

fn side_effect_ledger_key_with_suffix(suffix: u8) -> events::SideEffectLedgerKey {
    events::SideEffectLedgerKey::new(format!("ledger-key-{suffix}")).expect("ledger key")
}

fn remediation_ledger_key(byte: u8) -> events::SideEffectLedgerKey {
    events::SideEffectLedgerKey::new(format!("remediation-ledger-{byte}"))
        .expect("remediation ledger key")
}

fn side_effect_ledger_purpose() -> events::SideEffectLedgerPurpose {
    events::SideEffectLedgerPurpose::Forward
}

fn remediation_ledger_purpose() -> events::SideEffectLedgerPurpose {
    events::SideEffectLedgerPurpose::Remediation {
        forward_ledger_key: side_effect_ledger_key(),
    }
}

fn resource_namespace() -> ResourceNamespace {
    ResourceNamespace::new("mfm.test.account_nonce").expect("resource namespace")
}

fn resource_key(value: &str, schema_byte: u8) -> events::ResourceKeyEvidence {
    events::ResourceKeyEvidence {
        namespace: resource_namespace(),
        key_schema_id: schema_id("mfm.test.resource_key", schema_byte),
        key: events::ResourceKey::new(value).expect("resource key"),
    }
}

fn resource_lane_key(value: &str) -> ResourceLaneKey {
    ResourceLaneKey::from_evidence(&resource_key(value, 200))
}

fn resource_touched_set(byte: u8) -> events::ResourceTouchedSetEvidence {
    events::ResourceTouchedSetEvidence {
        namespace: resource_namespace(),
        evidence_schema_id: schema_id("mfm.test.touched_set", byte),
        evidence_hash: content_digest(byte),
        evidence_artifact_id: artifact_id(byte),
    }
}

fn resource_touched_set_artifact_ref(
    evidence: &events::ResourceTouchedSetEvidence,
) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id: evidence.evidence_artifact_id.clone(),
        digest: evidence.evidence_hash.clone(),
        byte_len: 128,
        media_type: media_type("application/json"),
        schema_id: Some(evidence.evidence_schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::StateOutput,
    }
}

fn payload_json_value(payload: &KernelEventPayload) -> serde_json::Value {
    serde_json::from_str(
        payload_canonical_json(payload)
            .expect("payload canonical json")
            .as_str(),
    )
    .expect("payload json value")
}

fn assert_projection_conflict_contains(error: StoreError, expected: &str) {
    assert!(
        matches!(
            &error,
            StoreError::ProjectionConflict { message, .. } if message.contains(expected)
        ),
        "unexpected error: {error:?}"
    );
}

fn assert_invalid_prepared_commit_contains(error: StoreError, expected: &str) {
    assert!(
        matches!(
            &error,
            StoreError::InvalidPreparedCommitPurpose { message, .. } if message.contains(expected)
        ),
        "unexpected error: {error:?}"
    );
}

fn assert_event_error_contains(error: StoreError, expected: &str) {
    assert!(
        matches!(
            &error,
            StoreError::Event(message) if message.contains(expected)
        ),
        "unexpected error: {error:?}"
    );
}

fn assert_resource_lane_blocked(error: StoreError, expected_lane_key: &ResourceLaneKey) {
    assert!(
        matches!(
            &error,
            StoreError::ResourceLaneBlocked { lane_key, .. }
                if lane_key.as_ref() == expected_lane_key
        ),
        "unexpected error: {error:?}"
    );
}

fn side_effect_attempt_started() -> KernelEventPayload {
    side_effect_attempt_started_for(node_id(70), attempt_id(72))
}

fn side_effect_attempt_started_for(node_id: NodeId, attempt_id: AttemptId) -> KernelEventPayload {
    KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
        spec_hash: spec_hash(1),
        node_id,
        attempt_id,
        attempt_no: 1,
        state_kind: state_kind(70),
        state_version: StateVersion::new("mfm.test.side_effect_state.v1").expect("state version"),
    })
}

fn side_effect_intent(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    KernelEventPayload::SideEffectIntentPersisted(side_effect::IntentPersisted {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        scope_id: scope_id(71),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        invocation_epoch: 1,
        intent_schema_id: schema_id("mfm.test.side_effect_intent", 70),
        intent_hash: digest,
        intent_artifact_id: artifact_id,
        idempotency_input_schema_id: schema_id("mfm.test.idempotency_input", 73),
        idempotency_input_hash: content_digest(74),
        idempotency_key: events::IdempotencyKeyRef::new("idem-key-1").expect("idempotency key"),
        capability_kind: capability_kind(75),
        capability_version: CapabilityVersion::new("mfm.test.capability.v1")
            .expect("capability version"),
        adapter_kind: adapter_kind(76),
        adapter_version: AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version"),
    })
}

fn side_effect_claim() -> KernelEventPayload {
    side_effect_claim_for_epoch(1, 1, "token-1")
}

fn side_effect_claim_for_epoch(
    invocation_epoch: u32,
    claim_generation: u32,
    token: &str,
) -> KernelEventPayload {
    KernelEventPayload::SideEffectClaimed(side_effect::Claimed {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        claim_owner: events::RunnerInvocationId::new("owner-1").expect("claim owner"),
        invocation_epoch,
        claim_generation,
        claim_fencing_token: side_effect::ClaimFencingToken::new(token).expect("token"),
    })
}

fn side_effect_claim_taken_over() -> KernelEventPayload {
    KernelEventPayload::SideEffectClaimTakenOver(side_effect::ClaimTakenOver {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        previous_claim_owner: events::RunnerInvocationId::new("owner-1").expect("previous owner"),
        new_claim_owner: events::RunnerInvocationId::new("owner-2").expect("new owner"),
        invocation_epoch: 1,
        previous_claim_generation: 1,
        claim_generation: 2,
        claim_fencing_token: side_effect::ClaimFencingToken::new("token-2").expect("token"),
    })
}

fn side_effect_claim_taken_over_with_token(token: &str) -> KernelEventPayload {
    side_effect_claim_taken_over_generation(1, 2, token)
}

fn side_effect_claim_taken_over_generation(
    previous_claim_generation: u32,
    claim_generation: u32,
    token: &str,
) -> KernelEventPayload {
    KernelEventPayload::SideEffectClaimTakenOver(side_effect::ClaimTakenOver {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        previous_claim_owner: events::RunnerInvocationId::new("owner-1").expect("previous owner"),
        new_claim_owner: events::RunnerInvocationId::new("owner-2").expect("new owner"),
        invocation_epoch: 1,
        previous_claim_generation,
        claim_generation,
        claim_fencing_token: side_effect::ClaimFencingToken::new(token).expect("token"),
    })
}

fn side_effect_prepared(claim_generation: u32, token: &str) -> KernelEventPayload {
    side_effect_prepared_for_epoch(1, claim_generation, token)
}

fn side_effect_prepared_for_epoch(
    invocation_epoch: u32,
    claim_generation: u32,
    token: &str,
) -> KernelEventPayload {
    KernelEventPayload::SideEffectInvocationPrepared(side_effect::InvocationPrepared {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        invocation_epoch,
        claim_generation,
        claim_fencing_token: side_effect::ClaimFencingToken::new(token).expect("token"),
        prepared_artifact_id: None,
        prepared_hash: None,
        resource_key: None,
    })
}

fn side_effect_prepared_with_resource_key(
    claim_generation: u32,
    token: &str,
    resource_key: events::ResourceKeyEvidence,
) -> KernelEventPayload {
    side_effect_prepared_with_resource_key_for_epoch(1, claim_generation, token, resource_key)
}

fn side_effect_prepared_with_resource_key_for_epoch(
    invocation_epoch: u32,
    claim_generation: u32,
    token: &str,
    resource_key: events::ResourceKeyEvidence,
) -> KernelEventPayload {
    let mut prepared = side_effect_prepared_for_epoch(invocation_epoch, claim_generation, token);
    let KernelEventPayload::SideEffectInvocationPrepared(payload) = &mut prepared else {
        unreachable!("helper returns invocation-prepared payload");
    };
    payload.resource_key = Some(resource_key);
    prepared
}

fn side_effect_started(owner: &str, claim_generation: u32, token: &str) -> KernelEventPayload {
    KernelEventPayload::SideEffectInvocationStarted(side_effect::InvocationStarted {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        invocation_epoch: 1,
        claim_owner: events::RunnerInvocationId::new(owner).expect("claim owner"),
        claim_generation,
        claim_fencing_token: side_effect::ClaimFencingToken::new(token).expect("token"),
    })
}

fn submission_schema() -> SchemaId {
    schema_id("mfm.test.submission", 77)
}

fn receipt_schema() -> SchemaId {
    schema_id("mfm.test.receipt", 78)
}

fn confirmation_schema() -> SchemaId {
    schema_id("mfm.test.confirmation", 79)
}

fn unknown_schema() -> SchemaId {
    schema_id("mfm.test.submission_unknown", 83)
}

fn not_submitted_schema() -> SchemaId {
    schema_id("mfm.test.not_submitted", 80)
}

fn side_effect_not_submitted(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    KernelEventPayload::SideEffectNotSubmittedProven(side_effect::NotSubmittedProven {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        invocation_epoch: 1,
        proof_schema_id: not_submitted_schema(),
        proof_hash: digest,
        proof_artifact_id: artifact_id,
    })
}

fn side_effect_submission_observed(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> KernelEventPayload {
    KernelEventPayload::SideEffectSubmissionObserved(side_effect::SubmissionObserved {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        invocation_epoch: 1,
        submission_schema_id: submission_schema(),
        submission_hash: digest,
        submission_artifact_id: artifact_id,
    })
}

fn side_effect_ambiguous(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    KernelEventPayload::SideEffectAmbiguous(side_effect::Ambiguous {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        invocation_epoch: 1,
        ambiguity_code: events::AmbiguityCode::new("ambiguous").expect("ambiguity code"),
        evidence_schema_id: schema_id("mfm.test.ambiguity", 76),
        evidence_hash: digest,
        evidence_artifact_id: artifact_id,
    })
}

fn side_effect_submission_unknown(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> KernelEventPayload {
    KernelEventPayload::SideEffectSubmissionUnknown(side_effect::SubmissionUnknown {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        invocation_epoch: 1,
        evidence_schema_id: unknown_schema(),
        evidence_hash: digest,
        evidence_artifact_id: artifact_id,
    })
}

fn side_effect_receipt(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    KernelEventPayload::SideEffectReceiptObserved(side_effect::ReceiptObserved {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        invocation_epoch: 1,
        receipt_schema_id: receipt_schema(),
        receipt_hash: digest,
        receipt_artifact_id: artifact_id,
        replay_verifier_id: events::ReplayVerifierId::new("verifier-1").expect("verifier"),
        resource_touched_set: None,
    })
}

fn side_effect_confirmation(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    KernelEventPayload::SideEffectConfirmationObserved(side_effect::ConfirmationObserved {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        invocation_epoch: 1,
        confirmation_schema_id: confirmation_schema(),
        confirmation_hash: digest,
        confirmation_artifact_id: artifact_id,
        replay_verifier_id: events::ReplayVerifierId::new("verifier-1").expect("verifier"),
        resource_touched_set: None,
    })
}

fn side_effect_failed(retryable: bool) -> KernelEventPayload {
    KernelEventPayload::SideEffectFailed(side_effect::Failed {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        ledger_purpose: side_effect_ledger_purpose(),
        invocation_epoch: 1,
        failure_phase: side_effect::FailurePhase::BeforeInvocationStarted,
        retryable,
        error: events::MfmErrorInfo {
            code: events::ErrorCode::new("sidefx_failed").expect("error code"),
            category: events::ErrorCategory::SideEffect,
            retryable,
            safe_message: "side-effect failed".to_owned(),
            public_details: None,
            diagnostic_ref: None,
        },
    })
}

fn side_effect_attempt_failed(retryable: bool) -> KernelEventPayload {
    KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        retryable,
        error: events::MfmErrorInfo {
            code: events::ErrorCode::new("sidefx_failed").expect("error code"),
            category: events::ErrorCategory::SideEffect,
            retryable,
            safe_message: "side-effect failed".to_owned(),
            public_details: None,
            diagnostic_ref: None,
        },
    })
}

fn fact_attempt_failed(retryable: bool) -> KernelEventPayload {
    KernelEventPayload::StateAttemptFailed(events::StateAttemptFailed {
        spec_hash: spec_hash(1),
        node_id: node_id(90),
        attempt_id: attempt_id(91),
        retryable,
        error: events::MfmErrorInfo {
            code: events::ErrorCode::new("fact_failed").expect("error code"),
            category: events::ErrorCategory::Runtime,
            retryable,
            safe_message: "fact state failed".to_owned(),
            public_details: None,
            diagnostic_ref: None,
        },
    })
}

fn run_completed(outcome: events::RunCompletionOutcome) -> KernelEventPayload {
    run_completed_for_run(run_id(120), outcome)
}

fn run_completed_for_run(
    run_id: RunId,
    outcome: events::RunCompletionOutcome,
) -> KernelEventPayload {
    KernelEventPayload::RunCompleted(events::RunCompleted {
        run_id,
        spec_hash: spec_hash(1),
        outcome,
    })
}

fn completed_outcome(byte: u8) -> events::RunCompletionOutcome {
    events::RunCompletionOutcome::Completed(Box::new(events::PublicOutputCompletionEvidence {
        public_output_schema_id: schema_id("mfm.test.public_output", 3),
        public_output_event_id: event_id(byte),
    }))
}

fn manual_resolution_recorded_for_run(run_id: RunId, byte: u8) -> KernelEventPayload {
    KernelEventPayload::ManualResolutionRecorded(events::ManualResolutionRecorded {
        run_id,
        spec_hash: spec_hash(1),
        outcome: events::ManualResolutionOutcome::ConfirmRemediated,
        evidence_schema_id: schema_id("mfm.test.manual_evidence", byte + 1),
        evidence_hash: content_digest(byte + 1),
        evidence_artifact_id: artifact_id(byte + 1),
        authorization_schema_id: schema_id("mfm.test.manual_authorization", byte + 2),
        authorization_hash: content_digest(byte + 2),
        authorization_artifact_id: artifact_id(byte + 2),
        note: Some(events::ManualResolutionNote::new("reviewed evidence").expect("note")),
    })
}

fn manual_resolution_artifacts(byte: u8) -> Vec<ArtifactEvidenceRef> {
    vec![
        ArtifactEvidenceRef {
            artifact_id: artifact_id(byte + 1),
            digest: content_digest(byte + 1),
            byte_len: 128,
            media_type: media_type("application/json"),
            schema_id: Some(schema_id("mfm.test.manual_evidence", byte + 1)),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: ArtifactRole::ManualResolutionEvidence,
        },
        ArtifactEvidenceRef {
            artifact_id: artifact_id(byte + 2),
            digest: content_digest(byte + 2),
            byte_len: 128,
            media_type: media_type("application/json"),
            schema_id: Some(schema_id("mfm.test.manual_authorization", byte + 2)),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None,
            artifact_role: ArtifactRole::ManualResolutionAuthorization,
        },
    ]
}

fn manual_saga_policy(byte: u8) -> SagaPolicySpec {
    SagaPolicySpec::ManualResolution {
        manual: ManualResolutionEvidenceSpec {
            evidence_schema: schema_id("mfm.test.manual_evidence", byte + 1),
            authorization: manual_authorization(byte),
        },
    }
}

fn manual_authorization(byte: u8) -> spec::ManualResolutionAuthorizationSpec {
    spec::ManualResolutionAuthorizationSpec {
        verifier_id: spec::ManualAuthorizationVerifierId::new(format!(
            "mfm.test.manual.verifier.{byte}"
        ))
        .expect("verifier id"),
        signing_scheme: spec::ManualSigningSchemeSpec::new(
            "mfm.manual_resolution.digest_signature.v1",
        )
        .expect("signing scheme"),
        authority: spec::OperatorAuthoritySnapshotSpec {
            authority_id: spec::OperatorAuthorityId::new(format!(
                "mfm.test.manual.authority.{byte}"
            ))
            .expect("authority id"),
            operators: vec![spec::OperatorAuthorityMemberSpec {
                operator_id: spec::OperatorId::new(format!("operator.{byte}"))
                    .expect("operator id"),
                public_identity: spec::OperatorPublicIdentity::new(format!(
                    "operator-public-{byte}"
                ))
                .expect("operator public identity"),
            }],
        },
        quorum: spec::ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
    }
}

fn compensate_saga_policy() -> SagaPolicySpec {
    SagaPolicySpec::CompensateCompleted {
        on_remediation_unresolved: RemediationUnresolvedSpec::FailWithoutAcdcClaim,
    }
}

fn saga_preconditions(run_id: &RunId, policy: SagaPolicySpec) -> CommitPreconditions {
    CommitPreconditions {
        saga_admit_token: Some(
            SagaAdmitToken::new(run_id.clone(), spec_hash(1), policy).expect("saga admit token"),
        ),
        ..CommitPreconditions::default()
    }
}

fn manual_resolution_request(
    run_id: &RunId,
    expected_next_seq: StreamSeq,
    commit_key: &str,
    byte: u8,
    policy: SagaPolicySpec,
) -> TypedCommitRequest {
    TypedCommitRequest::from_payloads(
        run_id.clone(),
        expected_next_seq,
        CommitKey::new(commit_key).expect("commit key"),
        vec![manual_resolution_recorded_for_run(run_id.clone(), byte)],
        manual_resolution_artifacts(byte),
        saga_preconditions(run_id, policy),
    )
    .expect("manual resolution request")
}

fn proof_manual_saga_policy() -> SagaPolicySpec {
    SagaPolicySpec::ManualResolution {
        manual: proof_manual_evidence_spec(),
    }
}

fn proof_manual_evidence_spec() -> ManualResolutionEvidenceSpec {
    ManualResolutionEvidenceSpec {
        evidence_schema: schema_id("mfm.test.manual_evidence", 201),
        authorization: spec::ManualResolutionAuthorizationSpec {
            verifier_id: spec::ManualAuthorizationVerifierId::new("mfm.test.manual.verifier.proof")
                .expect("verifier id"),
            signing_scheme: spec::ManualSigningSchemeSpec::new(
                "mfm.manual_resolution.digest_signature.v1",
            )
            .expect("signing scheme"),
            authority: spec::OperatorAuthoritySnapshotSpec {
                authority_id: spec::OperatorAuthorityId::new("mfm.test.manual.authority.proof")
                    .expect("authority id"),
                operators: vec![spec::OperatorAuthorityMemberSpec {
                    operator_id: spec::OperatorId::new("operator.proof").expect("operator id"),
                    public_identity: spec::OperatorPublicIdentity::new(
                        "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf",
                    )
                    .expect("operator public identity"),
                }],
            },
            quorum: spec::ManualAuthorizationQuorumSpec::new(1).expect("quorum"),
        },
    }
}

fn proof_manual_evidence_ref() -> ManualResolutionEvidenceRef {
    ManualResolutionEvidenceRef {
        schema_id: schema_id("mfm.test.manual_evidence", 201),
        content_hash: content_digest(201),
        artifact_id: artifact_id(201),
    }
}

fn verified_manual_resolution_for_seq(expected_next_seq: u64) -> VerifiedManualResolutionForPrefix {
    let prefix = ManualResolutionPrefixAuthority::new(
        run_id(220),
        spec_hash(1),
        expected_next_seq,
        content_digest(250),
        ManualResolutionBlockReason::PolicyManualResolution,
        content_digest(251),
        proof_manual_evidence_spec(),
    )
    .expect("manual prefix authority");
    let evidence = proof_manual_evidence_ref();
    let claim = prefix
        .authorization_claim(
            events::ManualResolutionOutcome::ConfirmRemediated,
            evidence.clone(),
        )
        .expect("manual authorization claim");
    for signature in manual_signature_candidates(expected_next_seq, &claim) {
        let authorization = manual_authorization_ref(signature.proof_bytes.as_bytes());
        if let Ok(verified) = ManualResolutionProofAuthority::new(
            prefix.clone(),
            claim.outcome,
            evidence.clone(),
            authorization,
            signature.proof_bytes.to_vec(),
        )
        .and_then(ManualResolutionProofAuthority::verify)
        {
            return verified;
        }
    }
    panic!("no manual signature fixture verified for sequence {expected_next_seq}");
}

struct ManualSignatureCandidate {
    proof_bytes: PlainCanonicalJsonBytes,
}

fn manual_signature_candidates(
    expected_next_seq: u64,
    claim: &ManualResolutionAuthorizationClaim,
) -> Vec<ManualSignatureCandidate> {
    let (r_hex, s_hex, normalized_s_hex) = match expected_next_seq {
        7 => (
            "a380063901f4c963898f2f997bd0ffa1b54cd07d1c8ec8800c395bcce745c54c",
            "e26f1b7d09fb4faf752e566187aa835583d36a747a488c34ab441491849f239d",
            "1d90e482f604b0508ad1a99e78557ca936db727235001407148e49fb4b971da4",
        ),
        8 => (
            "3e7e3a08374ccd3e3bc9d4b63ac4b7b167feceb2f9208f9367770906eadced20",
            "02260e1cc68113c6d29a5b2f7efe88e7f5bcc78c9855290a973545800c0ff060",
            "02260e1cc68113c6d29a5b2f7efe88e7f5bcc78c9855290a973545800c0ff060",
        ),
        9 => (
            "5b5d0a504ca7952abf53ba2eb4d1c5904e11882caac055d3cd95a88e1c37fff7",
            "22f2c34f3ccefe5ed956db06e43046024f6a1c371574db19a2a60e7c84f76a93",
            "22f2c34f3ccefe5ed956db06e43046024f6a1c371574db19a2a60e7c84f76a93",
        ),
        other => panic!("missing manual signature fixture for sequence {other}"),
    };
    let policy = proof_manual_evidence_spec().authorization;
    let operator = policy.authority.operators[0].clone();
    [s_hex, normalized_s_hex]
        .into_iter()
        .flat_map(|s| {
            [0_u8, 1]
                .into_iter()
                .map(move |recovery_id| (s, recovery_id))
        })
        .map(|(s, recovery_id)| {
            let mut signature = hex_to_bytes(r_hex);
            signature.extend(hex_to_bytes(s));
            signature.push(recovery_id);
            let proof = ManualResolutionAuthorizationProof {
                verifier_id: policy.verifier_id.clone(),
                signing_scheme: policy.signing_scheme.clone(),
                claim: claim.clone(),
                signatures: vec![ManualResolutionAuthorizationSignature {
                    operator_id: operator.operator_id.clone(),
                    public_identity: operator.public_identity.clone(),
                    signature: ManualAuthorizationSignatureBytes::new(signature)
                        .expect("manual signature bytes"),
                }],
            };
            ManualSignatureCandidate {
                proof_bytes: proof.canonical_json().expect("manual proof canonical json"),
            }
        })
        .collect()
}

fn hex_to_bytes(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0, "hex string length");
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).expect("hex byte"))
        .collect()
}

fn manual_authorization_ref(proof_bytes: &[u8]) -> ManualResolutionEvidenceRef {
    let content_hash = PlainCanonicalJsonBytes::from_canonical_json_slice(proof_bytes)
        .expect("canonical proof bytes")
        .content_digest();
    ManualResolutionEvidenceRef {
        schema_id: manual_authorization_proof_schema_id().expect("manual authorization schema"),
        artifact_id: ArtifactId::from_digest(content_hash.algorithm(), *content_hash.digest()),
        content_hash,
    }
}

fn manual_resolution_payload_from_verified(
    verified: &VerifiedManualResolutionForPrefix,
) -> KernelEventPayload {
    let claim = verified.claim();
    let authorization = verified.authorization();
    KernelEventPayload::ManualResolutionRecorded(events::ManualResolutionRecorded {
        run_id: claim.run_id.clone(),
        spec_hash: claim.spec_hash.clone(),
        outcome: claim.outcome,
        evidence_schema_id: claim.evidence.schema_id.clone(),
        evidence_hash: claim.evidence.content_hash.clone(),
        evidence_artifact_id: claim.evidence.artifact_id.clone(),
        authorization_schema_id: authorization.schema_id.clone(),
        authorization_hash: authorization.content_hash.clone(),
        authorization_artifact_id: authorization.artifact_id.clone(),
        note: None,
    })
}

fn manual_resolution_artifacts_from_verified(
    verified: &VerifiedManualResolutionForPrefix,
) -> Vec<ArtifactEvidenceRef> {
    let claim = verified.claim();
    let authorization = verified.authorization();
    vec![
        ArtifactEvidenceRef {
            artifact_id: claim.evidence.artifact_id.clone(),
            digest: claim.evidence.content_hash.clone(),
            byte_len: 128,
            media_type: media_type("application/json"),
            schema_id: Some(claim.evidence.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None::<SeedId>,
            artifact_role: ArtifactRole::ManualResolutionEvidence,
        },
        ArtifactEvidenceRef {
            artifact_id: authorization.artifact_id.clone(),
            digest: authorization.content_hash.clone(),
            byte_len: verified.proof_bytes().len() as u64,
            media_type: media_type("application/json"),
            schema_id: Some(authorization.schema_id.clone()),
            semantic_type_id: None,
            producer_node_id: None,
            producer_seed_id: None::<SeedId>,
            artifact_role: ArtifactRole::ManualResolutionAuthorization,
        },
    ]
}

fn manual_resolution_request_from_verified(
    verified: &VerifiedManualResolutionForPrefix,
    expected_next_seq: StreamSeq,
    commit_key: &str,
    policy: SagaPolicySpec,
) -> TypedCommitRequest {
    TypedCommitRequest::from_payloads(
        verified.claim().run_id.clone(),
        expected_next_seq,
        CommitKey::new(commit_key).expect("commit key"),
        vec![manual_resolution_payload_from_verified(verified)],
        manual_resolution_artifacts_from_verified(verified),
        CommitPreconditions {
            required_run_state: RequiredRunState::NotCompleted,
            ..saga_preconditions(&verified.claim().run_id, policy)
        },
    )
    .expect("manual resolution request")
}

fn prepared_manual_resolution_commit(
    verified: &VerifiedManualResolutionForPrefix,
    expected_next_seq: StreamSeq,
    commit_key: &str,
    policy: SagaPolicySpec,
) -> PreparedCommit<ManualResolution> {
    let request =
        manual_resolution_request_from_verified(verified, expected_next_seq, commit_key, policy);
    let artifacts = manual_resolution_artifacts_from_verified(verified);
    PreparedCommit::<ManualResolution>::new(
        request,
        CommitArtifactEvidenceSet::new(artifacts.clone(), artifacts)
            .expect("manual artifact evidence set"),
        verified,
    )
    .expect("proof-backed manual resolution prepared commit")
}

fn set_remediation_purpose(
    payload: &mut KernelEventPayload,
    ledger_key: events::SideEffectLedgerKey,
) {
    set_side_effect_ledger(payload, ledger_key, remediation_ledger_purpose());
}

macro_rules! with_side_effect_payload_mut {
    ($payload:expr, $inner:ident, $body:block) => {
        match $payload {
            KernelEventPayload::SideEffectIntentPersisted($inner) => $body,
            KernelEventPayload::SideEffectClaimed($inner) => $body,
            KernelEventPayload::SideEffectClaimTakenOver($inner) => $body,
            KernelEventPayload::SideEffectInvocationPrepared($inner) => $body,
            KernelEventPayload::SideEffectInvocationStarted($inner) => $body,
            KernelEventPayload::SideEffectNotSubmittedProven($inner) => $body,
            KernelEventPayload::SideEffectSubmissionObserved($inner) => $body,
            KernelEventPayload::SideEffectSubmissionUnknown($inner) => $body,
            KernelEventPayload::SideEffectReceiptObserved($inner) => $body,
            KernelEventPayload::SideEffectConfirmationObserved($inner) => $body,
            KernelEventPayload::SideEffectAmbiguous($inner) => $body,
            KernelEventPayload::SideEffectFailed($inner) => $body,
            _ => unreachable!("payload is not side-effect evidence"),
        }
    };
}

fn set_side_effect_ledger(
    payload: &mut KernelEventPayload,
    ledger_key: events::SideEffectLedgerKey,
    purpose: events::SideEffectLedgerPurpose,
) {
    with_side_effect_payload_mut!(payload, inner, {
        inner.ledger_key = ledger_key.clone();
        inner.ledger_purpose = purpose.clone();
    });
}

fn set_side_effect_node_attempt(
    payload: &mut KernelEventPayload,
    node_id: NodeId,
    attempt_id: AttemptId,
) {
    with_side_effect_payload_mut!(payload, inner, {
        inner.node_id = node_id.clone();
        inner.attempt_id = attempt_id.clone();
    });
}

fn set_attempt_failure_node_attempt(
    payload: &mut KernelEventPayload,
    node_id: NodeId,
    attempt_id: AttemptId,
) {
    let KernelEventPayload::StateAttemptFailed(payload) = payload else {
        unreachable!("payload is not attempt failure evidence");
    };
    payload.node_id = node_id;
    payload.attempt_id = attempt_id;
}

fn fact_recorded(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    KernelEventPayload::FactRecorded(events::FactRecorded {
        spec_hash: spec_hash(1),
        node_id: node_id(90),
        attempt_id: attempt_id(91),
        capability_kind: capability_kind(92),
        capability_version: CapabilityVersion::new("mfm.test.fact.v1").expect("capability version"),
        adapter_kind: adapter_kind(93),
        adapter_version: AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version"),
        request_schema_id: schema_id("mfm.test.fact_request", 94),
        request_hash: content_digest(95),
        response_schema_id: schema_id("mfm.test.fact_response", 96),
        response_hash: digest,
        fact_key: events::FactKey::new("fact-key-1").expect("fact key"),
        artifact_id,
    })
}

fn fact_attempt_started() -> KernelEventPayload {
    KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
        spec_hash: spec_hash(1),
        node_id: node_id(90),
        attempt_id: attempt_id(91),
        attempt_no: 1,
        state_kind: state_kind(90),
        state_version: StateVersion::new("mfm.test.fact_state.v1").expect("state version"),
    })
}

fn fact_artifact_ref(artifact_id: ArtifactId, digest: ContentDigest) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 64,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id("mfm.test.fact_response", 96)),
        semantic_type_id: None,
        producer_node_id: Some(node_id(90)),
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::FactResponse,
    }
}

fn retention_refs_appended(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    role: ArtifactRole,
) -> KernelEventPayload {
    KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
        run_id: run_id(120),
        spec_hash: spec_hash(1),
        refs: vec![events::RetentionRef {
            artifact_id,
            role,
            content_digest: digest,
        }],
        reason: events::RetentionReason::RuntimeEvidence,
    })
}

fn retention_manifest_artifact_ref(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 512,
        media_type: media_type("application/vnd.mfm.retention-manifest+json;version=1"),
        schema_id: None,
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::RetentionManifest,
    }
}

fn retention_manifest_projected(
    seq: u64,
    digest: ContentDigest,
    previous: Option<ContentDigest>,
    artifact_id: ArtifactId,
) -> KernelEventPayload {
    KernelEventPayload::RetentionManifestProjected(events::RetentionManifestProjected {
        run_id: run_id(120),
        spec_hash: spec_hash(1),
        manifest_seq: seq,
        manifest_digest: digest,
        previous_manifest_digest: previous,
        manifest_artifact_id: artifact_id,
    })
}

fn retention_manifest_commit_payloads(
    seq: u64,
    digest: ContentDigest,
    previous: Option<ContentDigest>,
    artifact_id: ArtifactId,
) -> Vec<KernelEventPayload> {
    vec![
        retention_manifest_projected(seq, digest.clone(), previous, artifact_id.clone()),
        KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
            run_id: run_id(120),
            spec_hash: spec_hash(1),
            refs: vec![events::RetentionRef {
                artifact_id,
                role: ArtifactRole::RetentionManifest,
                content_digest: digest,
            }],
            reason: events::RetentionReason::ManifestProjection,
        }),
    ]
}

fn projection_differential_summary(
    store: &InMemoryTypedRunStore,
    run_id: &RunId,
    stream: &[KernelEventEnvelope],
) -> String {
    let rebuilt = ProjectionSnapshot::rebuild_from_run_stream(stream).expect("rebuild projections");
    assert_eq!(store.projection_snapshot(), &rebuilt);

    let committed =
        CommittedRunStream::from_events(run_id.clone(), stream.to_vec()).expect("committed stream");
    assert_eq!(committed.projection(), &rebuilt);

    projection_snapshot_summary(&rebuilt, &committed)
}

fn projection_snapshot_summary(
    snapshot: &ProjectionSnapshot,
    committed: &CommittedRunStream,
) -> String {
    let run_id = committed.run_id();
    let mut rows = Vec::new();
    rows.push(format!(
        "committed run_state={:?} commits={} events={} next_seq={}",
        snapshot.run_state(run_id),
        committed.commits().len(),
        committed.events().len(),
        committed.next_seq().as_u64()
    ));

    rows.extend(snapshot.facts().map(|((_node, _attempt, fact_key), fact)| {
        format!(
            "fact key={} schema={} artifact={}",
            fact_key.as_str(),
            fact.response_schema_id.as_str(),
            fact.artifact_id.as_str()
        )
    }));

    rows.extend(snapshot.side_effects().map(|(ledger_ref, side_effect)| {
        format!(
            "side_effect ledger={} phase={} prepared={} resource_key={} touched_set={}",
            ledger_ref.ledger_key.as_str(),
            side_effect.phase.as_str(),
            side_effect.prepared_invocation.is_some(),
            side_effect.resource_key.is_some(),
            side_effect.resource_touched_set.is_some()
        )
    }));

    rows.extend(snapshot.resource_lanes().map(|(lane_key, lane)| {
        format!(
            "resource_lane {}:{} holder={} phase_epoch={}",
            lane_key.namespace.as_str(),
            lane_key.key.as_str(),
            lane.holder.ledger_key.as_str(),
            lane.invocation_epoch
        )
    }));

    rows.extend(snapshot.public_outputs().map(|(schema_id, projection)| {
        let PublicOutputProjection::Produced {
            rendered_artifact_id,
            ..
        } = projection
        else {
            return format!("public_output schema={} failed", schema_id.as_str());
        };
        format!(
            "public_output schema={} rendered_artifact={}",
            schema_id.as_str(),
            rendered_artifact_id.is_some()
        )
    }));

    rows.extend(snapshot.retentions().map(|(retention_run_id, retention)| {
        let latest = retention.manifest.as_ref().expect("latest manifest");
        format!(
            "retention run={} refs={} manifests={} latest_seq={}",
            retention_run_id.as_str(),
            retention.refs.len(),
            retention.manifests.len(),
            latest.manifest_seq
        )
    }));

    rows.join("\n")
}

fn artifact_role_tag_baselines() -> &'static [(ArtifactRole, &'static str)] {
    &[
        (ArtifactRole::TypedExecutionSpec, "typed_execution_spec"),
        (ArtifactRole::TypedSpecCertificate, "typed_spec_certificate"),
        (ArtifactRole::TypedConfig, "typed_config"),
        (ArtifactRole::SeedInput, "seed_input"),
        (ArtifactRole::StateOutput, "state_output"),
        (ArtifactRole::FactResponse, "fact_response"),
        (ArtifactRole::SideEffectIntent, "side_effect_intent"),
        (ArtifactRole::PreparedInvocation, "prepared_invocation"),
        (ArtifactRole::NotSubmittedProof, "not_submitted_proof"),
        (ArtifactRole::Submission, "submission"),
        (
            ArtifactRole::SubmissionUnknownEvidence,
            "submission_unknown_evidence",
        ),
        (ArtifactRole::Receipt, "receipt"),
        (ArtifactRole::Confirmation, "confirmation"),
        (ArtifactRole::AmbiguityEvidence, "ambiguity_evidence"),
        (
            ArtifactRole::ManualResolutionEvidence,
            "manual_resolution_evidence",
        ),
        (
            ArtifactRole::ManualResolutionAuthorization,
            "manual_resolution_authorization",
        ),
        (ArtifactRole::PublicOutput, "public_output"),
        (ArtifactRole::RedactedDiagnostic, "redacted_diagnostic"),
        (ArtifactRole::RetentionManifest, "retention_manifest"),
    ]
}

#[test]
fn artifact_role_contract_store_codec_roundtrips_current_tags() {
    let rows = artifact_role_tag_baselines()
        .iter()
        .map(|(role, tag)| {
            assert_eq!(mfm_store::v1::codec::artifact_role_str(*role), *tag);
            assert_eq!(
                mfm_store::v1::codec::parse_artifact_role(tag).expect("parse artifact role"),
                *role
            );
            format!("{tag} -> {role:?}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        rows,
        "typed_execution_spec -> TypedExecutionSpec\n\
typed_spec_certificate -> TypedSpecCertificate\n\
typed_config -> TypedConfig\n\
seed_input -> SeedInput\n\
state_output -> StateOutput\n\
fact_response -> FactResponse\n\
side_effect_intent -> SideEffectIntent\n\
prepared_invocation -> PreparedInvocation\n\
not_submitted_proof -> NotSubmittedProof\n\
submission -> Submission\n\
submission_unknown_evidence -> SubmissionUnknownEvidence\n\
receipt -> Receipt\n\
confirmation -> Confirmation\n\
ambiguity_evidence -> AmbiguityEvidence\n\
manual_resolution_evidence -> ManualResolutionEvidence\n\
manual_resolution_authorization -> ManualResolutionAuthorization\n\
public_output -> PublicOutput\n\
redacted_diagnostic -> RedactedDiagnostic\n\
retention_manifest -> RetentionManifest"
    );

    assert!(mfm_store::v1::codec::parse_artifact_role("resource_touched_set_evidence").is_err());
}

#[test]
fn event_artifact_requirements_mark_filterable_sources() {
    let cell_requirements =
        event_artifact_requirements(&cell_produced(artifact_id(31), content_digest(32)));
    assert_eq!(cell_requirements.len(), 1);
    assert_eq!(
        cell_requirements[0].source,
        EventArtifactReferenceSource::StateOutput
    );
    assert!(cell_requirements[0]
        .source
        .is_terminal_lifecycle_receipt_candidate());
    assert_eq!(
        cell_requirements[0].artifact_role,
        Some(ArtifactRole::StateOutput)
    );

    let public_requirements =
        event_artifact_requirements(&public_output_produced(artifact_id(33), content_digest(34)));
    assert_eq!(public_requirements.len(), 1);
    assert_eq!(
        public_requirements[0].source,
        EventArtifactReferenceSource::PublicOutputCell
    );
    assert!(!public_requirements[0]
        .source
        .is_terminal_lifecycle_receipt_candidate());

    let retention_requirements = event_artifact_requirements(&retention_refs_appended(
        artifact_id(35),
        content_digest(36),
        ArtifactRole::FactResponse,
    ));
    assert_eq!(retention_requirements.len(), 1);
    assert_eq!(
        retention_requirements[0].source,
        EventArtifactReferenceSource::RetentionRef
    );
    assert!(retention_requirements[0].source.is_retention());

    let mut failure = side_effect_failed(false);
    let KernelEventPayload::SideEffectFailed(payload) = &mut failure else {
        panic!("side-effect failure payload");
    };
    payload.error.diagnostic_ref = Some(event_artifact_ref(artifact_id(37), content_digest(38)));
    let failure_requirements = event_artifact_requirements(&failure);
    assert_eq!(failure_requirements.len(), 1);
    assert_eq!(
        failure_requirements[0].source,
        EventArtifactReferenceSource::SideEffectFailureDiagnostic
    );
    assert!(!failure_requirements[0].source.is_retention());
    assert!(!failure_requirements[0]
        .source
        .is_terminal_lifecycle_receipt_candidate());
}

#[test]
fn fact_recorded_protocol_baselines_cover_codec_requirements_and_projection() {
    let payload = fact_recorded(artifact_id(63), content_digest(64));
    let canonical = payload_canonical_json(&payload).expect("fact payload json");
    assert_eq!(
        canonical.as_str(),
        r#"{"adapter_kind":"adapter:mfm.test:adapter:sha256-jcs-v1:5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d","adapter_version":"mfm.test.adapter.v1","artifact_id":"artifact:sha256-jcs-v1:3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f","attempt_id":"attempt:sha256-jcs-v1:5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b5b","capability_kind":"capability:mfm.test:capability:sha256-jcs-v1:5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c","capability_version":"mfm.test.fact.v1","fact_key":"fact-key-1","node_id":"node:sha256-jcs-v1:5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a","request_hash":"content:sha256-jcs-v1:5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f5f","request_schema_id":"schema:mfm.test.fact_request:1:sha256-jcs-v1:5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e","response_hash":"content:sha256-jcs-v1:4040404040404040404040404040404040404040404040404040404040404040","response_schema_id":"schema:mfm.test.fact_response:1:sha256-jcs-v1:6060606060606060606060606060606060606060606060606060606060606060","spec_hash":"spec:sha256-jcs-v1:0101010101010101010101010101010101010101010101010101010101010101","variant":"FactRecorded"}"#
    );
    assert_eq!(
        mfm_store::v1::payload_hash(&payload)
            .expect("fact payload hash")
            .as_str(),
        "content:sha256-jcs-v1:327d0e04794116c46f8ab330b071cb1b10ae862177a1b1e3a04159d742aa986b"
    );
    let decoded_json: serde_json::Value =
        serde_json::from_str(canonical.as_str()).expect("payload json");
    assert_eq!(
        payload_from_json_value(&decoded_json).expect("payload roundtrip"),
        payload
    );

    let requirements = event_artifact_requirements(&payload);
    assert_eq!(requirements.len(), 1);
    let requirement = &requirements[0];
    assert_eq!(
        requirement.source,
        EventArtifactReferenceSource::FactResponse
    );
    assert_eq!(requirement.artifact_role, Some(ArtifactRole::FactResponse));
    assert_eq!(requirement.artifact_id, artifact_id(63));
    assert_eq!(requirement.digest, Some(content_digest(64)));
    assert_eq!(requirement.byte_len, None);
    assert_eq!(requirement.media_type, None);
    assert_eq!(
        requirement.schema_id,
        Some(schema_id("mfm.test.fact_response", 96))
    );
    assert_eq!(requirement.semantic_type_id, None);
    assert_eq!(requirement.producer_node_id, Some(node_id(90)));
    assert_eq!(requirement.producer_seed_id, None);

    let run_id = run_id(250);
    let fact_artifact_id = artifact_id(63);
    let fact_digest = content_digest(64);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "fact-baseline-run-start"))
        .expect("append run start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fact-baseline-attempt-start").expect("commit key"),
            payloads: vec![fact_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::NotCompleted,
                ..CommitPreconditions::default()
            },
        })
        .expect("append fact attempt start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fact-baseline-recorded").expect("commit key"),
            payloads: vec![payload.clone()],
            required_artifacts: vec![fact_artifact_ref(fact_artifact_id, fact_digest)],
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::NotCompleted,
                ..CommitPreconditions::default()
            },
        })
        .expect("append fact recorded");
    let stream = store.load_run_stream(&run_id);
    let snapshot = ProjectionSnapshot::rebuild_from_run_stream(&stream).expect("rebuild stream");
    let projection = snapshot
        .fact(
            &node_id(90),
            &attempt_id(91),
            &events::FactKey::new("fact-key-1").expect("fact key"),
        )
        .expect("fact projection");
    assert_eq!(projection.node_id, node_id(90));
    assert_eq!(projection.attempt_id, attempt_id(91));
    assert_eq!(
        projection.fact_key,
        events::FactKey::new("fact-key-1").expect("fact key")
    );
    assert_eq!(
        projection.request_schema_id,
        schema_id("mfm.test.fact_request", 94)
    );
    assert_eq!(projection.request_hash, content_digest(95));
    assert_eq!(
        projection.response_schema_id,
        schema_id("mfm.test.fact_response", 96)
    );
    assert_eq!(projection.response_hash, content_digest(64));
    assert_eq!(projection.artifact_id, artifact_id(63));
    assert_eq!(projection.capability_kind, capability_kind(92));
    assert_eq!(
        projection.capability_version,
        CapabilityVersion::new("mfm.test.fact.v1").expect("capability version")
    );
    assert_eq!(projection.adapter_kind, adapter_kind(93));
    assert_eq!(
        projection.adapter_version,
        AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version")
    );
}

#[test]
fn fact_recorded_codec_declaration_view_matches_handwritten_codec() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct EventCodecDeclaration {
        variant_tag: &'static str,
        fields: &'static [&'static str],
    }

    const FACT_RECORDED_CODEC_DECLARATION: EventCodecDeclaration = EventCodecDeclaration {
        variant_tag: "FactRecorded",
        fields: &[
            "adapter_kind",
            "adapter_version",
            "artifact_id",
            "attempt_id",
            "capability_kind",
            "capability_version",
            "fact_key",
            "node_id",
            "request_hash",
            "request_schema_id",
            "response_hash",
            "response_schema_id",
            "spec_hash",
        ],
    };

    fn declared_json(payload: &events::FactRecorded) -> serde_json::Value {
        assert_eq!(FACT_RECORDED_CODEC_DECLARATION.variant_tag, "FactRecorded");
        assert_eq!(
            FACT_RECORDED_CODEC_DECLARATION.fields,
            [
                "adapter_kind",
                "adapter_version",
                "artifact_id",
                "attempt_id",
                "capability_kind",
                "capability_version",
                "fact_key",
                "node_id",
                "request_hash",
                "request_schema_id",
                "response_hash",
                "response_schema_id",
                "spec_hash",
            ]
        );
        serde_json::json!({
            "adapter_kind": payload.adapter_kind.as_str(),
            "adapter_version": payload.adapter_version.as_str(),
            "artifact_id": payload.artifact_id.as_str(),
            "attempt_id": payload.attempt_id.as_str(),
            "capability_kind": payload.capability_kind.as_str(),
            "capability_version": payload.capability_version.as_str(),
            "fact_key": payload.fact_key.as_str(),
            "node_id": payload.node_id.as_str(),
            "request_hash": payload.request_hash.as_str(),
            "request_schema_id": payload.request_schema_id.as_str(),
            "response_hash": payload.response_hash.as_str(),
            "response_schema_id": payload.response_schema_id.as_str(),
            "spec_hash": payload.spec_hash.as_str(),
            "variant": FACT_RECORDED_CODEC_DECLARATION.variant_tag,
        })
    }

    let payload = fact_recorded(artifact_id(63), content_digest(64));
    let KernelEventPayload::FactRecorded(fact) = &payload else {
        unreachable!("helper returns fact payload");
    };
    let declared = declared_json(fact);
    let declared_canonical =
        PlainCanonicalJsonBytes::from_json_str(&declared.to_string()).expect("declared canonical");

    assert_eq!(declared, payload_json_value(&payload));
    assert_eq!(
        declared_canonical,
        payload_canonical_json(&payload).expect("handwritten canonical")
    );
    assert_eq!(
        declared_canonical.content_digest(),
        mfm_store::v1::payload_hash(&payload).expect("handwritten payload hash")
    );
    assert_eq!(
        payload_from_json_value(&declared).expect("declared payload decode"),
        payload
    );
}

#[test]
fn fact_recorded_projection_transition_descriptor_matches_rebuilt_projection() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct FactRecordedProjectionTransitionDeclaration {
        requires_started_attempt: bool,
        unique_key: &'static [&'static str],
        projected_fields: &'static [&'static str],
    }

    const FACT_RECORDED_PROJECTION: FactRecordedProjectionTransitionDeclaration =
        FactRecordedProjectionTransitionDeclaration {
            requires_started_attempt: true,
            unique_key: &["node_id", "attempt_id", "fact_key"],
            projected_fields: &[
                "event_id",
                "node_id",
                "attempt_id",
                "fact_key",
                "request_schema_id",
                "request_hash",
                "response_schema_id",
                "response_hash",
                "artifact_id",
                "capability_kind",
                "capability_version",
                "adapter_kind",
                "adapter_version",
            ],
        };

    let run_id = run_id(251);
    let fact_artifact_id = artifact_id(63);
    let fact_digest = content_digest(64);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request(
            run_id.clone(),
            "fact-projection-run-start",
        ))
        .expect("append run start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fact-projection-attempt-start").expect("commit key"),
            payloads: vec![fact_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::NotCompleted,
                ..CommitPreconditions::default()
            },
        })
        .expect("append fact attempt start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fact-projection-recorded").expect("commit key"),
            payloads: vec![fact_recorded(fact_artifact_id.clone(), fact_digest.clone())],
            required_artifacts: vec![fact_artifact_ref(fact_artifact_id, fact_digest)],
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::NotCompleted,
                ..CommitPreconditions::default()
            },
        })
        .expect("append fact recorded");

    let stream = store.load_run_stream(&run_id);
    let snapshot = ProjectionSnapshot::rebuild_from_run_stream(&stream).expect("rebuild stream");
    let projection = snapshot
        .fact(
            &node_id(90),
            &attempt_id(91),
            &events::FactKey::new("fact-key-1").expect("fact key"),
        )
        .expect("fact projection");

    assert!(FACT_RECORDED_PROJECTION.requires_started_attempt);
    assert_eq!(
        FACT_RECORDED_PROJECTION.unique_key,
        ["node_id", "attempt_id", "fact_key"]
    );
    assert_eq!(
        FACT_RECORDED_PROJECTION.projected_fields,
        [
            "event_id",
            "node_id",
            "attempt_id",
            "fact_key",
            "request_schema_id",
            "request_hash",
            "response_schema_id",
            "response_hash",
            "artifact_id",
            "capability_kind",
            "capability_version",
            "adapter_kind",
            "adapter_version",
        ]
    );
    assert_eq!(projection.event_id, stream[2].event_id().clone());
    assert_eq!(projection.node_id, node_id(90));
    assert_eq!(projection.attempt_id, attempt_id(91));
    assert_eq!(
        projection.fact_key,
        events::FactKey::new("fact-key-1").expect("fact key")
    );
    assert_eq!(
        projection.request_schema_id,
        schema_id("mfm.test.fact_request", 94)
    );
    assert_eq!(projection.request_hash, content_digest(95));
    assert_eq!(
        projection.response_schema_id,
        schema_id("mfm.test.fact_response", 96)
    );
    assert_eq!(projection.response_hash, content_digest(64));
    assert_eq!(projection.artifact_id, artifact_id(63));
    assert_eq!(projection.capability_kind, capability_kind(92));
    assert_eq!(
        projection.capability_version,
        CapabilityVersion::new("mfm.test.fact.v1").expect("capability version")
    );
    assert_eq!(projection.adapter_kind, adapter_kind(93));
    assert_eq!(
        projection.adapter_version,
        AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version")
    );
}

#[test]
fn committed_run_stream_exposes_store_owned_authority() {
    let run_id = run_id(141);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request(
            run_id.clone(),
            "committed-stream-run-start",
        ))
        .expect("append run start");
    append_side_effect_prepare(&mut store, &run_id);

    let stream = store.load_run_stream(&run_id);
    let committed =
        CommittedRunStream::from_events(run_id.clone(), stream.clone()).expect("committed stream");

    assert_eq!(committed.run_id(), &run_id);
    assert_eq!(committed.events(), stream.as_slice());
    assert_eq!(committed.next_seq(), store.expected_next_seq(&run_id));
    assert_eq!(committed.commits().len(), 3);
    assert_eq!(committed.commits()[0].seq(), StreamSeq::FIRST);
    assert_eq!(committed.commits()[0].events().len(), 1);
    assert_eq!(committed.commits()[1].events().len(), 1);
    assert_eq!(committed.commits()[2].events().len(), 3);
    assert_eq!(
        committed.commits()[2].commit_key().as_str(),
        "sidefx-prepare"
    );
    assert_eq!(committed.projection().run_state(&run_id), RunState::Started);
    assert!(committed.saga_projection().is_none());
    assert_eq!(
        committed
            .side_effect_projection(&side_effect_ledger_key())
            .expect("side-effect projection")
            .phase,
        SideEffectPhase::InvocationPrepared {
            invocation_epoch: 1,
            claim_generation: 1,
            claim_fencing_token: side_effect::ClaimFencingToken::new("token-1").expect("token"),
        }
    );
    assert!(committed.artifact_requirements().iter().any(|requirement| {
        requirement.source == EventArtifactReferenceSource::RunSpec
            && requirement.artifact_role == Some(ArtifactRole::TypedExecutionSpec)
    }));
    assert!(committed.artifact_requirements().iter().any(|requirement| {
        requirement.source == EventArtifactReferenceSource::SideEffectIntent
            && requirement.artifact_role == Some(ArtifactRole::SideEffectIntent)
    }));
}

#[test]
fn committed_run_stream_rejects_events_for_a_different_run() {
    let requested_run_id = run_id(142);
    let other_run_id = run_id(143);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request(
            requested_run_id.clone(),
            "committed-stream-wrong-run-start",
        ))
        .expect("append run start");

    let error =
        CommittedRunStream::from_events(other_run_id, store.load_run_stream(&requested_run_id))
            .expect_err("wrong run id rejects");
    assert!(matches!(
        error,
        StoreError::PersistedEventMismatch {
            field: "run_id",
            ..
        }
    ));
}

#[test]
fn non_empty_payload_batch_rejects_empty_batches() {
    assert!(matches!(
        NonEmptyPayloadBatch::new(Vec::new()),
        Err(StoreError::EmptyCommit)
    ));
    let batch = NonEmptyPayloadBatch::new(vec![run_admitted(run_id(144))]).expect("payload batch");
    assert_eq!(batch.as_slice().len(), 1);
    assert_eq!(batch.into_vec().len(), 1);
}

#[test]
fn prepared_commit_plan_mints_valid_run_start_authority() {
    let run_id = run_id(145);
    let request = run_start_request(run_id.clone(), "purpose-run-start");
    let artifacts = CommitArtifactEvidenceSet::new(
        request.required_artifacts().to_vec(),
        request.required_artifacts().to_vec(),
    )
    .expect("artifact evidence set");
    let commit = PreparedCommit::<RunAdmission>::new(request.clone(), artifacts)
        .expect("run-start authority");
    let plan = PreparedCommitPlan::from(commit);

    assert_eq!(plan.request().run_id(), &run_id);
    assert_eq!(plan.request().payloads(), request.payloads());
    assert_eq!(
        plan.request().required_artifacts(),
        request.required_artifacts()
    );
}

#[test]
fn prepared_commit_authority_rejects_invalid_request_shapes() {
    let base_run_id = run_id(146);
    assert!(matches!(
        TypedCommitRequest::from_payloads(
            base_run_id.clone(),
            StreamSeq::FIRST,
            CommitKey::new("purpose-empty").expect("commit key"),
            Vec::new(),
            Vec::new(),
            CommitPreconditions::default(),
        ),
        Err(StoreError::EmptyCommit)
    ));

    assert!(matches!(
        TypedCommitRequest::from_payloads(
            base_run_id.clone(),
            StreamSeq::FIRST,
            CommitKey::new("purpose-mixed-run").expect("commit key"),
            vec![run_admitted(run_id(147))],
            Vec::new(),
            CommitPreconditions::default(),
        ),
        Err(StoreError::PayloadRunMismatch { .. })
    ));

    let mut foreign_spec_payload = side_effect_attempt_started();
    let KernelEventPayload::StateAttemptStarted(payload) = &mut foreign_spec_payload else {
        panic!("state-attempt-start payload")
    };
    payload.spec_hash = spec_hash(148);
    assert!(matches!(
        TypedCommitRequest::from_payloads(
            base_run_id.clone(),
            StreamSeq::FIRST,
            CommitKey::new("purpose-mixed-spec").expect("commit key"),
            vec![run_admitted(base_run_id.clone()), foreign_spec_payload],
            Vec::new(),
            CommitPreconditions::default(),
        ),
        Err(StoreError::PayloadSpecHashMismatch { .. })
    ));

    let missing_artifact = TypedCommitRequest::from_payloads(
        base_run_id.clone(),
        StreamSeq::FIRST,
        CommitKey::new("purpose-missing-artifact").expect("commit key"),
        vec![run_admitted(base_run_id.clone())],
        vec![spec_artifact_ref()],
        CommitPreconditions::default(),
    )
    .expect("missing artifact request");
    let missing_artifact_set = CommitArtifactEvidenceSet::new(
        missing_artifact.required_artifacts().to_vec(),
        missing_artifact.required_artifacts().to_vec(),
    )
    .expect("artifact evidence set");
    assert!(matches!(
        PreparedCommit::<RunAdmission>::new(missing_artifact, missing_artifact_set),
        Err(StoreError::InvalidPreparedCommitPurpose {
            purpose: "run_admission",
            ..
        })
    ));

    let wrong_purpose = run_start_request(base_run_id, "purpose-wrong-marker");
    let wrong_purpose_artifacts = CommitArtifactEvidenceSet::new(
        wrong_purpose.required_artifacts().to_vec(),
        wrong_purpose.required_artifacts().to_vec(),
    )
    .expect("artifact evidence set");
    assert!(matches!(
        PreparedCommit::<StateAttemptStarted>::new(wrong_purpose, wrong_purpose_artifacts),
        Err(StoreError::InvalidPreparedCommitPurpose {
            purpose: "state_attempt_started",
            ..
        })
    ));
}

#[test]
fn prepared_commit_plan_accepts_explicit_terminal_attempt_authority() {
    let artifact_id = artifact_id(149);
    let digest = content_digest(150);
    let payloads = terminal_cell_commit_payloads(artifact_id.clone(), digest.clone());
    let artifact = store_artifact_ref(artifact_id, digest);
    let request = typed_commit_request! {
        run_id: run_id(151),
        expected_next_seq: StreamSeq::FIRST,
        commit_key: CommitKey::new("purpose-attempt-terminal").expect("commit key"),
        payloads: payloads,
        required_artifacts: vec![artifact.clone()],
        preconditions: CommitPreconditions {
            required_run_state: RequiredRunState::NotCompleted,
            ..CommitPreconditions::default()
        },
    };
    let commit = PreparedCommit::<AttemptTerminal>::new(
        request,
        CommitArtifactEvidenceSet::new(vec![artifact.clone()], vec![artifact])
            .expect("artifact evidence set"),
    )
    .expect("attempt-terminal authority");
    let plan = PreparedCommitPlan::from(commit);

    assert!(matches!(plan, PreparedCommitPlan::AttemptTerminal(_)));
}

#[test]
fn commit_rejects_secret_shaped_persisted_error_message() {
    let mut store = InMemoryTypedRunStore::new();
    let run_id = run_id(152);
    ensure_test_run_admitted(&mut store, &run_id, "public-diagnostic-secret-run-start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("public-diagnostic-secret-attempt-start").expect("commit key"),
            payloads: vec![fact_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append attempt start");

    let mut failure = fact_attempt_failed(false);
    let KernelEventPayload::StateAttemptFailed(payload) = &mut failure else {
        unreachable!("helper returns state attempt failure");
    };
    payload.error.safe_message = "provider returned bearer token=super-secret-value".to_owned();
    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("public-diagnostic-secret-attempt-failed").expect("commit key"),
            payloads: vec![failure],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("secret-shaped diagnostic rejects before append");

    assert_event_error_contains(error.clone(), "message resembles secret material");
    assert!(
        !error.to_string().contains("super-secret-value"),
        "rejection must not echo secret-shaped diagnostic text"
    );
}

#[test]
fn commit_rejects_non_redacted_diagnostic_artifact_ref() {
    let mut store = InMemoryTypedRunStore::new();
    let run_id = run_id(153);
    ensure_test_run_admitted(&mut store, &run_id, "public-diagnostic-artifact-run-start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("public-diagnostic-artifact-attempt-start").expect("commit key"),
            payloads: vec![fact_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append attempt start");

    let artifact_id = artifact_id(154);
    let digest = content_digest(155);
    let schema_id = schema_id("mfm.test.diagnostic", 156);
    let diagnostic_ref = events::ArtifactEvidenceRef {
        artifact_id: artifact_id.clone(),
        role: ArtifactRole::SideEffectIntent,
        schema_id: schema_id.clone(),
        semantic_type_id: None,
        content_digest: digest.clone(),
        byte_len: 64,
        media_type: media_type("application/json"),
    };
    let required_artifact = ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 64,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id),
        semantic_type_id: None,
        producer_node_id: Some(node_id(90)),
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::SideEffectIntent,
    };
    let mut failure = fact_attempt_failed(false);
    let KernelEventPayload::StateAttemptFailed(payload) = &mut failure else {
        unreachable!("helper returns state attempt failure");
    };
    payload.error.diagnostic_ref = Some(diagnostic_ref);

    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("public-diagnostic-artifact-attempt-failed").expect("commit key"),
            payloads: vec![failure],
            required_artifacts: vec![required_artifact],
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("non-redacted diagnostic artifact rejects before append");

    assert_event_error_contains(
        error,
        "diagnostic artifact role must be redacted_diagnostic",
    );
}

fn spec_artifact_ref() -> ArtifactEvidenceRef {
    let hash = spec_hash(1);
    ArtifactEvidenceRef {
        artifact_id: artifact_id(2),
        digest: ContentDigest::from_digest(hash.algorithm(), *hash.digest()),
        byte_len: 128,
        media_type: media_type(SPEC_MEDIA_TYPE),
        schema_id: None,
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::TypedExecutionSpec,
    }
}

fn certificate_artifact_ref() -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id: artifact_id(4),
        digest: content_digest(4),
        byte_len: 64,
        media_type: media_type(mfm_certify::CERTIFICATE_MEDIA_TYPE),
        schema_id: None,
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::TypedSpecCertificate,
    }
}

fn run_artifact_ref(artifact: &ArtifactEvidenceRef) -> events::RunArtifactEvidenceRef {
    events::RunArtifactEvidenceRef {
        artifact_id: artifact.artifact_id.clone(),
        role: artifact.artifact_role,
        schema_id: artifact.schema_id.clone(),
        semantic_type_id: artifact.semantic_type_id.clone(),
        content_digest: artifact.digest.clone(),
        byte_len: artifact.byte_len,
        media_type: artifact.media_type.clone(),
    }
}

trait TestPreparedCommitExt {
    fn append_prepared_commit(
        &mut self,
        request: TypedCommitRequest,
    ) -> mfm_store::v1::Result<CommitOutcome>;

    fn append_prepared_commit_with_artifacts(
        &mut self,
        request: TypedCommitRequest,
        admitted_artifacts: Vec<ArtifactEvidenceRef>,
    ) -> mfm_store::v1::Result<CommitOutcome>;
}

impl TestPreparedCommitExt for InMemoryTypedRunStore {
    fn append_prepared_commit(
        &mut self,
        request: TypedCommitRequest,
    ) -> mfm_store::v1::Result<CommitOutcome> {
        let admitted_artifacts = request.required_artifacts().to_vec();
        let plan = test_prepared_commit_plan(request, admitted_artifacts)?;
        self.append_prepared_commit_plan(plan)
    }

    fn append_prepared_commit_with_artifacts(
        &mut self,
        request: TypedCommitRequest,
        admitted_artifacts: Vec<ArtifactEvidenceRef>,
    ) -> mfm_store::v1::Result<CommitOutcome> {
        let plan = test_prepared_commit_plan(request, admitted_artifacts)?;
        self.append_prepared_commit_plan(plan)
    }
}

fn test_prepared_commit_plan(
    request: TypedCommitRequest,
    admitted_artifacts: Vec<ArtifactEvidenceRef>,
) -> mfm_store::v1::Result<PreparedCommitPlan> {
    let artifacts =
        CommitArtifactEvidenceSet::new(request.required_artifacts().to_vec(), admitted_artifacts)?;
    if request
        .payloads()
        .iter()
        .all(|payload| matches!(payload, KernelEventPayload::RunAdmitted(_)))
    {
        return PreparedCommit::<RunAdmission>::new(request, artifacts)
            .map(PreparedCommitPlan::from);
    }
    if request
        .payloads()
        .iter()
        .all(|payload| matches!(payload, KernelEventPayload::StateAttemptStarted(_)))
    {
        let mut preconditions = request.preconditions().clone();
        preconditions.required_run_state = RequiredRunState::NotCompleted;
        let request = request.with_preconditions(preconditions);
        return PreparedCommit::<StateAttemptStarted>::new(request, artifacts)
            .map(PreparedCommitPlan::from);
    }
    if request
        .payloads()
        .iter()
        .any(|payload| matches!(payload, KernelEventPayload::ManualResolutionRecorded(_)))
    {
        return Err(StoreError::InvalidPreparedCommitPurpose {
            purpose: "manual_resolution",
            message: "manual resolution commits requires verified manual resolution proof".into(),
        });
    }
    if request.payloads().iter().any(test_is_saga_terminal_payload) {
        return Err(StoreError::InvalidPreparedCommitPurpose {
            purpose: "saga_terminal",
            message: "saga terminal commits requires SagaTerminalProof".into(),
        });
    }
    if request
        .payloads()
        .iter()
        .any(test_is_side_effect_terminal_payload)
    {
        return PreparedCommit::<SideEffectTerminal>::new(request, artifacts)
            .map(PreparedCommitPlan::from);
    }
    if request
        .payloads()
        .iter()
        .any(|payload| payload.side_effect_ref().is_some())
    {
        return PreparedCommit::<SideEffectProgress>::new(request, artifacts)
            .map(PreparedCommitPlan::from);
    }
    if request.payloads().iter().any(test_is_retention_payload) {
        return PreparedCommit::<Retention>::new(request, artifacts).map(PreparedCommitPlan::from);
    }
    PreparedCommit::<AttemptTerminal>::new(request, artifacts).map(PreparedCommitPlan::from)
}

fn test_is_retention_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::RetentionRefsAppended(_)
            | KernelEventPayload::RetentionManifestProjected(_)
    )
}

fn test_is_side_effect_terminal_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::SideEffectNotSubmittedProven(_)
            | KernelEventPayload::SideEffectSubmissionObserved(_)
            | KernelEventPayload::SideEffectSubmissionUnknown(_)
            | KernelEventPayload::SideEffectReceiptObserved(_)
            | KernelEventPayload::SideEffectConfirmationObserved(_)
            | KernelEventPayload::SideEffectAmbiguous(_)
            | KernelEventPayload::SideEffectFailed(_)
    )
}

fn test_is_saga_terminal_payload(payload: &KernelEventPayload) -> bool {
    matches!(
        payload,
        KernelEventPayload::RunCompleted(events::RunCompleted {
            outcome: events::RunCompletionOutcome::Compensated
                | events::RunCompletionOutcome::ManuallyResolved
                | events::RunCompletionOutcome::FailedWithoutAcdcClaim,
            ..
        })
    )
}

fn run_start_request(run_id: RunId, commit_key: &str) -> TypedCommitRequest {
    run_start_request_with_saga_policy(run_id, commit_key, &SagaPolicySpec::NoSideEffects)
}

fn run_start_request_with_saga_policy(
    run_id: RunId,
    commit_key: &str,
    saga_policy: &SagaPolicySpec,
) -> TypedCommitRequest {
    TypedCommitRequest::from_payloads(
        run_id.clone(),
        StreamSeq::FIRST,
        CommitKey::new(commit_key).expect("commit key"),
        vec![run_admitted_with_saga_policy(run_id, saga_policy)],
        vec![spec_artifact_ref(), certificate_artifact_ref()],
        CommitPreconditions {
            required_run_state: RequiredRunState::Absent,
            ..CommitPreconditions::default()
        },
    )
    .expect("run start request")
}

fn ensure_test_run_admitted(store: &mut InMemoryTypedRunStore, run_id: &RunId, commit_key: &str) {
    if store.expected_next_seq(run_id) == StreamSeq::FIRST {
        store
            .append_prepared_commit(run_start_request(run_id.clone(), commit_key))
            .expect("append fixture run admission");
    }
}

fn append_side_effect_prepare(store: &mut InMemoryTypedRunStore, run_id: &RunId) {
    ensure_test_run_admitted(store, run_id, "sidefx-fixture-run-start");
    let artifact_id = artifact_id(81);
    let artifact_digest = content_digest(82);
    let intent_evidence = intent_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: CommitKey::new("sidefx-attempt-start").expect("commit key"),
            payloads: vec![side_effect_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append sidefx attempt start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: CommitKey::new("sidefx-prepare").expect("commit key"),
            payloads: vec![
                side_effect_intent(artifact_id, artifact_digest),
                side_effect_claim(),
                side_effect_prepared(1, "token-1"),
            ],
            required_artifacts: vec![intent_evidence],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append sidefx prepare");
}

fn append_side_effect_prepare_for_ledger(
    store: &mut InMemoryTypedRunStore,
    run_id: &RunId,
    commit_key: &str,
    ledger_key: events::SideEffectLedgerKey,
    resource_key: events::ResourceKeyEvidence,
    artifact_byte: u8,
    start_attempt: bool,
) {
    append_side_effect_prepare_for_ledger_on_attempt(
        store,
        run_id,
        commit_key,
        ledger_key,
        resource_key,
        artifact_byte,
        start_attempt,
        node_id(70),
        attempt_id(72),
    );
}

#[allow(clippy::too_many_arguments)]
fn append_side_effect_prepare_for_ledger_on_attempt(
    store: &mut InMemoryTypedRunStore,
    run_id: &RunId,
    commit_key: &str,
    ledger_key: events::SideEffectLedgerKey,
    resource_key: events::ResourceKeyEvidence,
    artifact_byte: u8,
    start_attempt: bool,
    node_id: NodeId,
    attempt_id: AttemptId,
) {
    ensure_test_run_admitted(store, run_id, "sidefx-ledger-fixture-run-start");
    if start_attempt {
        store
            .append_prepared_commit(typed_commit_request! {
                run_id: run_id.clone(),
                expected_next_seq: store.expected_next_seq(run_id),
                commit_key: CommitKey::new(format!("{commit_key}-attempt-start"))
                    .expect("commit key"),
                payloads: vec![side_effect_attempt_started_for(
                    node_id.clone(),
                    attempt_id.clone(),
                )],
                required_artifacts: Vec::new(),
                preconditions: CommitPreconditions::default(),
            })
            .expect("append sidefx attempt start");
    }

    let artifact_id = artifact_id(artifact_byte);
    let artifact_digest = content_digest(artifact_byte + 1);
    let intent_evidence = intent_artifact_ref_for_node(
        artifact_id.clone(),
        artifact_digest.clone(),
        node_id.clone(),
    );
    let purpose = events::SideEffectLedgerPurpose::Forward;
    let mut intent = side_effect_intent(artifact_id, artifact_digest);
    let mut claim = side_effect_claim();
    let mut prepared = side_effect_prepared_with_resource_key(1, "token-1", resource_key);
    for payload in [&mut intent, &mut claim, &mut prepared] {
        set_side_effect_ledger(payload, ledger_key.clone(), purpose.clone());
        set_side_effect_node_attempt(payload, node_id.clone(), attempt_id.clone());
    }

    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: CommitKey::new(commit_key).expect("commit key"),
            payloads: vec![intent, claim, prepared],
            required_artifacts: vec![intent_evidence],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append sidefx prepare");
}

fn append_side_effect_started(store: &mut InMemoryTypedRunStore, run_id: &RunId) {
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: CommitKey::new("sidefx-started").expect("commit key"),
            payloads: vec![side_effect_started("owner-1", 1, "token-1")],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append sidefx started");
}

fn append_generic_nonretryable_failure(
    store: &mut InMemoryTypedRunStore,
    run_id: &RunId,
    key_prefix: &str,
) {
    ensure_test_run_admitted(store, run_id, &format!("{key_prefix}-fixture-run-start"));
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: CommitKey::new(format!("{key_prefix}-attempt-start")).expect("commit key"),
            payloads: vec![fact_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append generic attempt start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: CommitKey::new(format!("{key_prefix}-attempt-failed")).expect("commit key"),
            payloads: vec![fact_attempt_failed(false)],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append generic attempt failure");
}

fn append_forward_confirmation(store: &mut InMemoryTypedRunStore, run_id: &RunId) {
    append_side_effect_prepare(store, run_id);
    append_side_effect_started(store, run_id);
    let submission_artifact_id = artifact_id(84);
    let submission_digest = content_digest(85);
    let receipt_artifact_id = artifact_id(86);
    let receipt_digest = content_digest(87);
    let confirmation_artifact_id = artifact_id(88);
    let confirmation_digest = content_digest(89);
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: CommitKey::new("forward-confirmation").expect("commit key"),
            payloads: vec![
                side_effect_submission_observed(
                    submission_artifact_id.clone(),
                    submission_digest.clone(),
                ),
                side_effect_receipt(receipt_artifact_id.clone(), receipt_digest.clone()),
                side_effect_confirmation(
                    confirmation_artifact_id.clone(),
                    confirmation_digest.clone(),
                ),
            ],
            required_artifacts: vec![
                side_effect_evidence(
                    submission_artifact_id,
                    submission_digest,
                    submission_schema(),
                    ArtifactRole::Submission,
                ),
                side_effect_evidence(
                    receipt_artifact_id,
                    receipt_digest,
                    receipt_schema(),
                    ArtifactRole::Receipt,
                ),
                side_effect_evidence(
                    confirmation_artifact_id,
                    confirmation_digest,
                    confirmation_schema(),
                    ArtifactRole::Confirmation,
                ),
            ],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append forward confirmation");
}

fn append_remediation_confirmation(
    store: &mut InMemoryTypedRunStore,
    run_id: &RunId,
    ledger_key: events::SideEffectLedgerKey,
) {
    let intent_artifact_id = artifact_id(101);
    let intent_digest = content_digest(102);
    let submission_artifact_id = artifact_id(103);
    let submission_digest = content_digest(104);
    let receipt_artifact_id = artifact_id(105);
    let receipt_digest = content_digest(106);
    let confirmation_artifact_id = artifact_id(107);
    let confirmation_digest = content_digest(108);

    let mut intent = side_effect_intent(intent_artifact_id.clone(), intent_digest.clone());
    let mut claim = side_effect_claim();
    let mut prepared = side_effect_prepared(1, "token-1");
    let mut started = side_effect_started("owner-1", 1, "token-1");
    let mut submission =
        side_effect_submission_observed(submission_artifact_id.clone(), submission_digest.clone());
    let mut receipt = side_effect_receipt(receipt_artifact_id.clone(), receipt_digest.clone());
    let mut confirmation = side_effect_confirmation(
        confirmation_artifact_id.clone(),
        confirmation_digest.clone(),
    );
    for payload in [
        &mut intent,
        &mut claim,
        &mut prepared,
        &mut started,
        &mut submission,
        &mut receipt,
        &mut confirmation,
    ] {
        set_remediation_purpose(payload, ledger_key.clone());
    }

    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: CommitKey::new("remediation-confirmation").expect("commit key"),
            payloads: vec![
                intent,
                claim,
                prepared,
                started,
                submission,
                receipt,
                confirmation,
            ],
            required_artifacts: vec![
                intent_artifact_ref(intent_artifact_id, intent_digest),
                side_effect_evidence(
                    submission_artifact_id,
                    submission_digest,
                    submission_schema(),
                    ArtifactRole::Submission,
                ),
                side_effect_evidence(
                    receipt_artifact_id,
                    receipt_digest,
                    receipt_schema(),
                    ArtifactRole::Receipt,
                ),
                side_effect_evidence(
                    confirmation_artifact_id,
                    confirmation_digest,
                    confirmation_schema(),
                    ArtifactRole::Confirmation,
                ),
            ],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append remediation confirmation");
}

fn side_effect_evidence(
    artifact_id: ArtifactId,
    digest: ContentDigest,
    schema_id: SchemaId,
    role: ArtifactRole,
) -> ArtifactEvidenceRef {
    side_effect_artifact_ref(artifact_id, digest, schema_id, role)
}

#[test]
fn commit_key_idempotency_precedes_stale_expected_next_seq() {
    let run_id = run_id(40);
    let mut store = InMemoryTypedRunStore::new();
    let request = run_start_request(run_id.clone(), "run-start");
    let appended = store
        .append_prepared_commit(request.clone())
        .expect("append run start");
    assert!(matches!(appended, CommitOutcome::Appended(_)));

    let retry = request.with_expected_next_seq(StreamSeq::new(99).expect("stale seq"));
    let outcome = store
        .append_prepared_commit(retry)
        .expect("idempotent retry");

    let CommitOutcome::Idempotent(batch) = outcome else {
        panic!("same commit key and fingerprint should be idempotent");
    };
    assert_eq!(batch.seq(), StreamSeq::FIRST);
    assert_eq!(store.expected_next_seq(&run_id), StreamSeq::new(2).unwrap());
}

#[test]
fn async_in_memory_store_exposes_commit_stream_and_status_contract() {
    let store = AsyncInMemoryTypedRunStore::new();
    let resource_run = run_id(43);
    let status_run = run_id(44);
    let lane_key = resource_lane_key("async-wallet");

    let resource_request = run_start_request(resource_run.clone(), "async-resource-run-start");
    let resource_plan = test_prepared_commit_plan(
        resource_request.clone(),
        resource_request.required_artifacts().to_vec(),
    )
    .expect("prepare resource run start");
    let resource_outcome =
        poll_ready_store_future(store.append_prepared_commit_plan(resource_plan))
            .expect("append resource run start");
    assert!(matches!(resource_outcome, CommitOutcome::Appended(_)));

    store
        .with_inner_mut(|inner| {
            append_side_effect_prepare_for_ledger(
                inner,
                &resource_run,
                "async-resource-prepare",
                side_effect_ledger_key_with_suffix(30),
                resource_key("async-wallet", 230),
                170,
                true,
            );
        })
        .expect("append resource lane fixture");

    let status_request = run_start_request(status_run.clone(), "async-status-run-start");
    let status_plan = test_prepared_commit_plan(
        status_request.clone(),
        status_request.required_artifacts().to_vec(),
    )
    .expect("prepare status run start");
    poll_ready_store_future(store.append_prepared_commit_plan(status_plan))
        .expect("append status run start");

    let stream =
        poll_ready_store_future(store.load_run_stream(&status_run)).expect("load status stream");
    assert_eq!(stream.len(), 1);
    assert_eq!(stream[0].run_id(), &status_run);
    assert_eq!(
        poll_ready_store_future(store.expected_next_seq(&status_run)).expect("status next seq"),
        StreamSeq::new(2).expect("next seq")
    );

    let status_projection = poll_ready_store_future(store.status_projection_snapshot(&status_run))
        .expect("status projection");
    assert_eq!(status_projection.run_state(&status_run), RunState::Started);
    assert_eq!(status_projection.run_state(&resource_run), RunState::Absent);
    let lane = status_projection
        .resource_lane(&lane_key)
        .expect("cross-run resource lane");
    assert_eq!(lane.holder.run_id, resource_run);
    assert_eq!(
        lane.holder.ledger_key,
        side_effect_ledger_key_with_suffix(30)
    );
}

#[test]
fn commit_key_conflict_is_rejected_before_stale_sequence() {
    let run_id = run_id(41);
    let mut store = InMemoryTypedRunStore::new();
    let request = run_start_request(run_id.clone(), "run-start");
    store
        .append_prepared_commit(request)
        .expect("append run start");

    let conflicting = typed_commit_request! {
        run_id: run_id,
        expected_next_seq: StreamSeq::new(99).expect("stale seq"),
        commit_key: CommitKey::new("run-start").expect("commit key"),
        payloads: vec![state_attempt_started()],
        required_artifacts: Vec::new(),
        preconditions: CommitPreconditions::default(),
    };

    let error = store
        .append_prepared_commit(conflicting)
        .expect_err("same key different fingerprint conflicts");
    assert!(matches!(error, StoreError::CommitConflict { .. }));
}

#[test]
fn store_owns_envelope_sequence_ordinal_and_event_id() {
    let run_id = run_id(42);
    let mut store = InMemoryTypedRunStore::new();
    let first = store
        .append_prepared_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");
    let first_event = &first.batch().events()[0];
    assert_eq!(first_event.seq(), StreamSeq::FIRST);
    assert_eq!(first_event.ordinal().as_u32(), 0);
    assert_eq!(first_event.commit_key().as_str(), "run-start");
    assert_eq!(first_event.logical_key().as_str(), "run:admission");

    let second = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("attempt-start").expect("commit key"),
            payloads: vec![state_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::NotCompleted,
                ..CommitPreconditions::default()
            },
        })
        .expect("append attempt start");
    let second_event = &second.batch().events()[0];
    assert_eq!(second_event.seq(), StreamSeq::new(2).unwrap());
    assert_eq!(second_event.ordinal().as_u32(), 0);
    assert_ne!(first_event.event_id(), second_event.event_id());
}

#[test]
fn projection_rebuild_rejects_gapped_persisted_run_stream() {
    let run_id = run_id(43);
    let first = build_committed_batch(
        &run_start_request(run_id.clone(), "run-start"),
        StreamSeq::FIRST,
    )
    .expect("first batch");
    let second = build_committed_batch(
        &typed_commit_request! {
            run_id: run_id,
            expected_next_seq: StreamSeq::new(3).expect("gapped seq"),
            commit_key: CommitKey::new("attempt-start").expect("commit key"),
            payloads: vec![state_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        },
        StreamSeq::new(3).expect("gapped seq"),
    )
    .expect("second batch");
    let mut events = first.events().to_vec();
    events.extend(second.events().iter().cloned());

    let error = ProjectionSnapshot::rebuild_from_run_stream(&events)
        .expect_err("gapped persisted stream rejects");
    assert!(matches!(
        error,
        StoreError::PersistedEventMismatch { field: "seq", .. }
    ));
}

#[test]
fn projection_rebuild_rejects_mixed_commit_keys_for_one_sequence() {
    let run_id = run_id(44);
    let first = build_committed_batch(
        &run_start_request(run_id.clone(), "run-start"),
        StreamSeq::FIRST,
    )
    .expect("first batch");
    let sidecar = build_committed_batch(
        &typed_commit_request! {
            run_id: run_id,
            expected_next_seq: StreamSeq::FIRST,
            commit_key: CommitKey::new("same-seq-sidecar").expect("commit key"),
            payloads: vec![state_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        },
        StreamSeq::FIRST,
    )
    .expect("sidecar batch");
    let mut events = first.events().to_vec();
    events.push(rewrite_envelope(
        &sidecar.events()[0],
        StreamSeq::FIRST,
        CommitOrdinal::new(1),
        CommitKey::new("same-seq-sidecar").expect("commit key"),
    ));

    let error = ProjectionSnapshot::rebuild_from_run_stream(&events)
        .expect_err("mixed commit keys for one sequence reject");
    assert!(matches!(
        error,
        StoreError::PersistedEventMismatch {
            field: "commit_key",
            ..
        }
    ));
}

#[test]
fn projection_rebuild_rejects_old_model_terminal_attempt_without_start() {
    let run_id = run_id(45);
    let first = build_committed_batch(
        &run_start_request(run_id.clone(), "run-start"),
        StreamSeq::FIRST,
    )
    .expect("first batch");
    let old_terminal = build_committed_batch(
        &typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: StreamSeq::new(2).expect("next seq"),
            commit_key: CommitKey::new("old-terminal-without-start").expect("commit key"),
            payloads: terminal_cell_commit_payloads(artifact_id(46), content_digest(47)),
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        },
        StreamSeq::new(2).expect("terminal seq"),
    )
    .expect("old terminal batch");
    let mut events = first.events().to_vec();
    events.extend(old_terminal.events().iter().cloned());

    let error = ProjectionSnapshot::rebuild_from_run_stream(&events)
        .expect_err("old terminal attempt model rejects");
    assert_projection_conflict_contains(
        error,
        "unsupported old stream model: attempt-bound payload is not preceded by a StateAttemptStarted commit",
    );
}

#[test]
fn projection_rebuild_rejects_old_model_start_and_terminal_in_same_commit() {
    let run_id = run_id(48);
    let first = build_committed_batch(
        &run_start_request(run_id.clone(), "run-start"),
        StreamSeq::FIRST,
    )
    .expect("first batch");
    let old_combined = build_committed_batch(
        &typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: StreamSeq::new(2).expect("next seq"),
            commit_key: CommitKey::new("old-combined-framework").expect("commit key"),
            payloads: {
                let mut payloads = vec![state_attempt_started()];
                payloads.extend(terminal_cell_commit_payloads(artifact_id(49), content_digest(50)));
                payloads
            },
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        },
        StreamSeq::new(2).expect("combined seq"),
    )
    .expect("old combined batch");
    let mut events = first.events().to_vec();
    events.extend(old_combined.events().iter().cloned());

    let error = CommittedRunStream::from_events(run_id, events)
        .expect_err("old combined framework model rejects");
    assert_projection_conflict_contains(
        error,
        "unsupported old stream model: StateAttemptStarted must be committed before attempt-bound terminal payloads",
    );
}

#[test]
fn required_artifact_precondition_is_atomic_with_append() {
    let artifact_id = artifact_id(50);
    let artifact_digest = content_digest(51);
    let mut store = InMemoryTypedRunStore::new();
    let run_id = run_id(52);
    let evidence = event_artifact_ref(artifact_id, artifact_digest);
    let request = typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: StreamSeq::FIRST,
        commit_key: CommitKey::new("artifact-ref").expect("commit key"),
        payloads: vec![KernelEventPayload::ArtifactReferenced(
            events::ArtifactReferenced {
                spec_hash: spec_hash(1),
                node_id: Some(node_id(20)),
                attempt_id: Some(attempt_id(23)),
                artifact_ref: evidence.clone(),
            },
        )],
        required_artifacts: vec![store_artifact_ref(
            evidence.artifact_id.clone(),
            evidence.content_digest.clone(),
        )],
        preconditions: CommitPreconditions::default(),
    };

    let artifacts =
        CommitArtifactEvidenceSet::new(request.required_artifacts().to_vec(), Vec::new())
            .expect("missing artifact evidence set");
    let commit = PreparedCommit::<AttemptTerminal>::new(request, artifacts)
        .expect("prepare missing artifact");
    let error = store
        .append_prepared_commit_plan(commit.into())
        .expect_err("missing artifact rejects commit");
    assert!(matches!(error, StoreError::MissingArtifact { .. }));
    assert!(store.load_run_stream(&run_id).is_empty());
    assert_eq!(store.expected_next_seq(&run_id), StreamSeq::FIRST);
}

fn rewrite_envelope(
    event: &KernelEventEnvelope,
    seq: StreamSeq,
    ordinal: CommitOrdinal,
    commit_key: CommitKey,
) -> KernelEventEnvelope {
    KernelEventEnvelope::from_persisted_record(PersistedKernelEventRecord {
        event_id: event_id_for(event, seq, ordinal),
        event_schema_id: event.event_schema_id().clone(),
        run_id: event.run_id().clone(),
        seq,
        ordinal,
        spec_hash: event.spec_hash().clone(),
        commit_key,
        logical_key: event.logical_key().clone(),
        payload_hash: event.payload_hash().clone(),
        payload: event.payload().clone(),
        payload_canonical_byte_len: event.audit().payload_canonical_byte_len(),
    })
    .expect("rewritten envelope")
}

fn event_id_for(event: &KernelEventEnvelope, seq: StreamSeq, ordinal: CommitOrdinal) -> EventId {
    let canonical = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::json!({
            "event_schema_id": event.event_schema_id().as_str(),
            "ordinal": ordinal.as_u32(),
            "payload_hash": event.payload_hash().as_str(),
            "run_id": event.run_id().as_str(),
            "seq": seq.as_u64(),
        })
        .to_string(),
    )
    .expect("event id canonical");
    EventId::from_digest(DigestAlgorithm::Sha256JcsV1, canonical.digest_bytes())
}

#[test]
fn prepared_commit_rejects_unreferenced_admitted_artifact_without_persisting_it() {
    let run_id = run_id(53);
    let artifact_id = artifact_id(54);
    let artifact_digest = content_digest(55);
    let mut store = InMemoryTypedRunStore::new();
    let admitted = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());

    let error = store
        .append_prepared_commit_with_artifacts(
            typed_commit_request! {
                run_id: run_id.clone(),
                expected_next_seq: StreamSeq::FIRST,
                commit_key: CommitKey::new("unreferenced-artifact").expect("commit key"),
                payloads: vec![state_attempt_started()],
                required_artifacts: Vec::new(),
                preconditions: CommitPreconditions::default(),
            },
            vec![admitted.clone()],
        )
        .expect_err("unreferenced admitted artifact rejects before append");
    assert!(matches!(
        error,
        StoreError::UnreferencedArtifactEvidence { .. }
    ));
    assert!(store.load_run_stream(&run_id).is_empty());

    let missing_evidence = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    let error = store
        .append_prepared_commit_with_artifacts(
            typed_commit_request! {
                run_id: run_id.clone(),
                expected_next_seq: StreamSeq::FIRST,
                commit_key: CommitKey::new("artifact-still-missing").expect("commit key"),
                payloads: vec![KernelEventPayload::ArtifactReferenced(
                    events::ArtifactReferenced {
                        spec_hash: spec_hash(1),
                        node_id: Some(node_id(20)),
                        attempt_id: Some(attempt_id(23)),
                        artifact_ref: event_artifact_ref(artifact_id, artifact_digest),
                    },
                )],
                required_artifacts: vec![missing_evidence],
                preconditions: CommitPreconditions::default(),
            },
            Vec::new(),
        )
        .expect_err("rejected unreferenced evidence must not leak into store");
    assert!(matches!(error, StoreError::MissingArtifact { .. }));
}

#[test]
fn prepared_commit_idempotency_fingerprint_includes_admitted_artifacts() {
    let run_id = run_id(120);
    let artifact_id = artifact_id(131);
    let artifact_digest = content_digest(132);
    let mut store = InMemoryTypedRunStore::new();
    let evidence = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    let mut conflicting_evidence = evidence.clone();
    conflicting_evidence.byte_len += 1;
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");
    let request = typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(&run_id),
        commit_key: CommitKey::new("prepared-fingerprint").expect("commit key"),
        payloads: vec![retention_refs_appended(
            artifact_id,
            artifact_digest,
            ArtifactRole::StateOutput,
        )],
        required_artifacts: vec![evidence.clone()],
        preconditions: CommitPreconditions {
            required_run_state: RequiredRunState::Started,
            ..CommitPreconditions::default()
        },
    };

    store
        .append_prepared_commit_with_artifacts(request.clone(), vec![evidence])
        .expect("append initial prepared commit");
    let retry = request.with_expected_next_seq(StreamSeq::new(99).expect("stale seq"));
    let error = store
        .append_prepared_commit_with_artifacts(retry, vec![conflicting_evidence])
        .expect_err("same request with different admitted evidence is not idempotent");
    assert!(matches!(error, StoreError::CommitConflict { .. }));
}

#[test]
fn admitted_artifacts_are_rolled_back_when_commit_validation_fails() {
    let run_id = run_id(56);
    let artifact_id = artifact_id(57);
    let artifact_digest = content_digest(58);
    let mut store = InMemoryTypedRunStore::new();
    let evidence = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());

    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: StreamSeq::FIRST,
            commit_key: CommitKey::new("cell-without-attempt-complete").expect("commit key"),
            payloads: vec![cell_produced(artifact_id.clone(), artifact_digest.clone())],
            required_artifacts: vec![evidence],
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("projection failure rejects commit after artifact validation");
    assert!(matches!(error, StoreError::ProjectionConflict { .. }));
    assert!(store.load_run_stream(&run_id).is_empty());

    let missing_evidence = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    let error = store
        .append_prepared_commit_with_artifacts(
            typed_commit_request! {
                run_id: run_id,
                expected_next_seq: StreamSeq::FIRST,
                commit_key: CommitKey::new("artifact-not-leaked").expect("commit key"),
                payloads: vec![KernelEventPayload::ArtifactReferenced(
                    events::ArtifactReferenced {
                        spec_hash: spec_hash(1),
                        node_id: Some(node_id(20)),
                        attempt_id: Some(attempt_id(23)),
                        artifact_ref: event_artifact_ref(artifact_id, artifact_digest),
                    },
                )],
                required_artifacts: vec![missing_evidence],
                preconditions: CommitPreconditions::default(),
            },
            Vec::new(),
        )
        .expect_err("failed commit must not persist admitted artifact evidence");
    assert!(matches!(error, StoreError::MissingArtifact { .. }));
}

#[test]
fn payload_artifact_byte_len_media_type_and_producer_mismatches_are_rejected() {
    let first_run_id = run_id(59);
    let first_artifact_id = artifact_id(60);
    let first_artifact_digest = content_digest(61);
    let mut store = InMemoryTypedRunStore::new();
    let mut wrong_len =
        store_artifact_ref(first_artifact_id.clone(), first_artifact_digest.clone());
    wrong_len.byte_len = 129;
    let evidence = event_artifact_ref(first_artifact_id.clone(), first_artifact_digest.clone());
    let request = typed_commit_request! {
        run_id: first_run_id.clone(),
        expected_next_seq: StreamSeq::FIRST,
        commit_key: CommitKey::new("artifact-byte-len").expect("commit key"),
        payloads: vec![KernelEventPayload::ArtifactReferenced(
            events::ArtifactReferenced {
                spec_hash: spec_hash(1),
                node_id: Some(node_id(20)),
                attempt_id: Some(attempt_id(23)),
                artifact_ref: evidence,
            },
        )],
        required_artifacts: vec![store_artifact_ref(
            first_artifact_id.clone(),
            first_artifact_digest.clone(),
        )],
        preconditions: CommitPreconditions::default(),
    };

    let error = store
        .append_prepared_commit_with_artifacts(request, vec![wrong_len])
        .expect_err("payload byte length mismatch rejects commit");
    assert!(matches!(
        error,
        StoreError::ArtifactEvidenceMismatch {
            field: "byte_len",
            ..
        }
    ));
    assert!(store.load_run_stream(&first_run_id).is_empty());

    let media_run_id = run_id(86);
    let media_artifact_id = artifact_id(87);
    let media_artifact_digest = content_digest(88);
    let mut store = InMemoryTypedRunStore::new();
    let mut wrong_media =
        store_artifact_ref(media_artifact_id.clone(), media_artifact_digest.clone());
    wrong_media.media_type = media_type("application/octet-stream");
    let evidence = event_artifact_ref(media_artifact_id.clone(), media_artifact_digest.clone());
    let request = typed_commit_request! {
        run_id: media_run_id.clone(),
        expected_next_seq: StreamSeq::FIRST,
        commit_key: CommitKey::new("artifact-media-type").expect("commit key"),
        payloads: vec![KernelEventPayload::ArtifactReferenced(
            events::ArtifactReferenced {
                spec_hash: spec_hash(1),
                node_id: Some(node_id(20)),
                attempt_id: Some(attempt_id(23)),
                artifact_ref: evidence,
            },
        )],
        required_artifacts: vec![store_artifact_ref(
            media_artifact_id.clone(),
            media_artifact_digest.clone(),
        )],
        preconditions: CommitPreconditions::default(),
    };
    let error = store
        .append_prepared_commit_with_artifacts(request, vec![wrong_media])
        .expect_err("payload media type mismatch rejects commit");
    assert!(matches!(
        error,
        StoreError::ArtifactEvidenceMismatch {
            field: "media_type",
            ..
        }
    ));
    assert!(store.load_run_stream(&media_run_id).is_empty());

    let second_run_id = run_id(62);
    let second_artifact_id = artifact_id(63);
    let second_artifact_digest = content_digest(64);
    let mut store = InMemoryTypedRunStore::new();
    let mut wrong_producer =
        store_artifact_ref(second_artifact_id.clone(), second_artifact_digest.clone());
    wrong_producer.producer_node_id = Some(node_id(99));
    let request = typed_commit_request! {
        run_id: second_run_id.clone(),
        expected_next_seq: StreamSeq::FIRST,
        commit_key: CommitKey::new("artifact-producer").expect("commit key"),
        payloads: terminal_cell_commit_payloads(
            second_artifact_id.clone(),
            second_artifact_digest.clone(),
        ),
        required_artifacts: vec![store_artifact_ref(
            second_artifact_id,
            second_artifact_digest,
        )],
        preconditions: CommitPreconditions::default(),
    };

    let error = store
        .append_prepared_commit_with_artifacts(request, vec![wrong_producer])
        .expect_err("payload producer mismatch rejects commit");
    assert!(matches!(
        error,
        StoreError::ArtifactEvidenceMismatch {
            field: "producer_node_id",
            ..
        }
    ));
    assert!(store.load_run_stream(&second_run_id).is_empty());
}

#[test]
fn duplicate_attempt_lifecycle_events_are_rejected() {
    let run_id = run_id(59);
    let artifact_id = artifact_id(60);
    let artifact_digest = content_digest(61);
    let mut store = InMemoryTypedRunStore::new();
    let artifact = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "duplicate-run-start"))
        .expect("append run start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("attempt-start").expect("commit key"),
            payloads: vec![state_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append attempt start");

    let duplicate_start = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("attempt-start-2").expect("commit key"),
            payloads: vec![state_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("duplicate attempt start rejects");
    assert!(matches!(
        duplicate_start,
        StoreError::ProjectionConflict { .. }
    ));

    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("attempt-complete").expect("commit key"),
            payloads: terminal_cell_commit_payloads(artifact_id.clone(), artifact_digest.clone()),
            required_artifacts: vec![artifact.clone()],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append attempt complete");

    let duplicate_terminal = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("attempt-complete-2").expect("commit key"),
            payloads: terminal_cell_commit_payloads(artifact_id, artifact_digest),
            required_artifacts: vec![artifact],
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("duplicate attempt terminal rejects");
    assert!(matches!(
        duplicate_terminal,
        StoreError::DuplicateLogicalKey { .. } | StoreError::ProjectionConflict { .. }
    ));
}

#[test]
fn state_attempt_interrupted_projects_retryable_terminal_without_saga_engagement() {
    let run_id = run_id(60);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("attempt-start").expect("commit key"),
            payloads: vec![state_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append attempt start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("attempt-interrupt").expect("commit key"),
            payloads: vec![state_attempt_interrupted()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append attempt interruption");

    let projection = store.projection_snapshot();
    let attempt = projection
        .attempts()
        .find_map(|((projected_node_id, projected_attempt_id), projection)| {
            if *projected_node_id == node_id(20) && *projected_attempt_id == attempt_id(23) {
                Some(projection)
            } else {
                None
            }
        })
        .expect("attempt projection");
    assert!(matches!(&attempt.status, AttemptStatus::Interrupted));
    assert!(projection.saga_engagement(&run_id).is_none());
    assert_projection_codecs_round_trip(projection);
}

#[test]
fn state_attempt_interrupted_rejects_duplicate_terminal_event() {
    let run_id = run_id(61);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("attempt-start").expect("commit key"),
            payloads: vec![state_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append attempt start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("attempt-interrupt").expect("commit key"),
            payloads: vec![state_attempt_interrupted()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append attempt interruption");

    let duplicate = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("attempt-interrupt-2").expect("commit key"),
            payloads: vec![state_attempt_interrupted()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("duplicate interruption rejects");
    assert!(matches!(duplicate, StoreError::ProjectionConflict { .. }));
}

#[test]
fn state_attempt_interrupted_allows_side_effect_intent_before_invocation_prepared() {
    let run_id = run_id(62);
    let artifact_id = artifact_id(81);
    let artifact_digest = content_digest(82);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-attempt-start").expect("commit key"),
            payloads: vec![side_effect_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append sidefx attempt start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-intent").expect("commit key"),
            payloads: vec![side_effect_intent(
                artifact_id.clone(),
                artifact_digest.clone(),
            )],
            required_artifacts: vec![intent_artifact_ref(artifact_id, artifact_digest)],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append sidefx intent");

    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-attempt-interrupt").expect("commit key"),
            payloads: vec![KernelEventPayload::StateAttemptInterrupted(
                events::StateAttemptInterrupted {
                    spec_hash: spec_hash(1),
                    node_id: node_id(70),
                    attempt_id: attempt_id(72),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("interruption before sidefx prepare admits");

    let projection = store.projection_snapshot();
    let attempt = projection
        .attempt(&node_id(70), &attempt_id(72))
        .expect("attempt projection");
    assert!(matches!(attempt.status, AttemptStatus::Interrupted));
    assert!(matches!(
        projection
            .side_effect(&side_effect_ledger_key())
            .expect("side-effect projection")
            .ledger_state()
            .expect("ledger state")
            .phase(),
        SideEffectLedgerPhase::IntentPersisted { .. }
    ));
}

#[test]
fn state_attempt_interrupted_allows_side_effect_claim_before_invocation_prepared() {
    let run_id = run_id(63);
    let artifact_id = artifact_id(83);
    let artifact_digest = content_digest(84);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-attempt-start").expect("commit key"),
            payloads: vec![side_effect_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append sidefx attempt start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-claim").expect("commit key"),
            payloads: vec![
                side_effect_intent(artifact_id.clone(), artifact_digest.clone()),
                side_effect_claim(),
            ],
            required_artifacts: vec![intent_artifact_ref(artifact_id, artifact_digest)],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append sidefx claim");

    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-attempt-interrupt").expect("commit key"),
            payloads: vec![KernelEventPayload::StateAttemptInterrupted(
                events::StateAttemptInterrupted {
                    spec_hash: spec_hash(1),
                    node_id: node_id(70),
                    attempt_id: attempt_id(72),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("interruption before sidefx prepare admits");

    let projection = store.projection_snapshot();
    let attempt = projection
        .attempt(&node_id(70), &attempt_id(72))
        .expect("attempt projection");
    assert!(matches!(attempt.status, AttemptStatus::Interrupted));
    assert!(matches!(
        projection
            .side_effect(&side_effect_ledger_key())
            .expect("side-effect projection")
            .ledger_state()
            .expect("ledger state")
            .phase(),
        SideEffectLedgerPhase::Claimed { .. }
    ));
}

#[test]
fn state_attempt_interrupted_rejects_after_side_effect_invocation_prepared() {
    let run_id = run_id(64);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");
    append_side_effect_prepare(&mut store, &run_id);

    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-attempt-interrupt").expect("commit key"),
            payloads: vec![KernelEventPayload::StateAttemptInterrupted(
                events::StateAttemptInterrupted {
                    spec_hash: spec_hash(1),
                    node_id: node_id(70),
                    attempt_id: attempt_id(72),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("interruption after side-effect prepare rejects");
    assert!(matches!(error, StoreError::ProjectionConflict { .. }));
}

#[test]
fn state_attempt_failed_rejects_after_side_effect_authority_without_terminal_evidence() {
    let run_id = run_id(65);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");
    append_side_effect_prepare(&mut store, &run_id);

    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-attempt-failed-alone").expect("commit key"),
            payloads: vec![side_effect_attempt_failed(true)],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("failure after side-effect authority rejects without terminal evidence");
    assert_projection_conflict_contains(
        error,
        "side-effect attempt failure requires terminal side-effect evidence",
    );

    let request = typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(&run_id),
        commit_key: CommitKey::new("forged-sidefx-attempt-failed-alone").expect("commit key"),
        payloads: vec![side_effect_attempt_failed(true)],
        required_artifacts: Vec::new(),
        preconditions: CommitPreconditions::default(),
    };
    let batch =
        build_committed_batch(&request, store.expected_next_seq(&run_id)).expect("forged batch");
    let mut stream = store.load_run_stream(&run_id);
    stream.extend(batch.events().iter().cloned());
    let rebuild = ProjectionSnapshot::rebuild_from_run_stream(&stream)
        .expect_err("projection rebuild rejects forged generic failure");
    assert_projection_conflict_contains(
        rebuild,
        "side-effect attempt failure requires terminal side-effect evidence",
    );
}

#[test]
fn attempt_terminal_and_cell_terminal_must_commit_together() {
    let run_id = run_id(62);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "terminal-pair-run-start"))
        .expect("append run start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("attempt-start").expect("commit key"),
            payloads: vec![state_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append attempt start");

    let completion_only = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("completion-only").expect("commit key"),
            payloads: vec![state_attempt_completed()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("completion without terminal cell rejects");
    assert!(matches!(
        completion_only,
        StoreError::ProjectionConflict { .. }
    ));

    let cell_only = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("cell-only").expect("commit key"),
            payloads: vec![cell_produced(artifact_id(63), content_digest(64))],
            required_artifacts: vec![store_artifact_ref(artifact_id(63), content_digest(64))],
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("terminal cell without completion rejects");
    assert!(matches!(cell_only, StoreError::ProjectionConflict { .. }));
}

#[test]
fn public_output_must_commit_with_render_receipt_terminal() {
    let run_id = run_id(67);
    let artifact_id = artifact_id(68);
    let artifact_digest = content_digest(69);
    let mut store = InMemoryTypedRunStore::new();
    let artifact = store_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "public-output-run-start"))
        .expect("append run start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("attempt-start").expect("commit key"),
            payloads: vec![state_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append attempt start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("receipt-terminal").expect("commit key"),
            payloads: terminal_cell_commit_payloads(artifact_id.clone(), artifact_digest.clone()),
            required_artifacts: vec![artifact],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append receipt terminal");

    let split_public_output = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("split-public-output").expect("commit key"),
            payloads: vec![public_output_produced(
                artifact_id.clone(),
                artifact_digest.clone(),
            )],
            required_artifacts: vec![store_artifact_ref(artifact_id, artifact_digest)],
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("public output split from terminal commit rejects");
    assert!(matches!(
        split_public_output,
        StoreError::ProjectionConflict { .. }
    ));
}

#[test]
fn side_effect_transition_mismatches_are_rejected() {
    let run_id = run_id(80);
    let artifact_id = artifact_id(81);
    let artifact_digest = content_digest(82);
    let mut store = InMemoryTypedRunStore::new();
    let intent_evidence = intent_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    store
        .append_prepared_commit(run_start_request(
            run_id.clone(),
            "sidefx-transition-run-start",
        ))
        .expect("append run start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-attempt-start").expect("commit key"),
            payloads: vec![side_effect_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append sidefx attempt start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-intent").expect("commit key"),
            payloads: vec![side_effect_intent(artifact_id, artifact_digest)],
            required_artifacts: vec![intent_evidence],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append intent");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-claim").expect("commit key"),
            payloads: vec![side_effect_claim()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append claim");

    let wrong_generation = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-prepared-wrong-generation").expect("commit key"),
            payloads: vec![side_effect_prepared(2, "token-1")],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("prepared generation mismatch rejects");
    assert!(matches!(
        wrong_generation,
        StoreError::ProjectionConflict { .. }
    ));

    let wrong_token = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-prepared-wrong-token").expect("commit key"),
            payloads: vec![side_effect_prepared(1, "token-2")],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("prepared token mismatch rejects");
    assert!(matches!(wrong_token, StoreError::ProjectionConflict { .. }));
}

#[test]
fn run_mode_strings_and_terminal_outcome_mapping_are_canonical() {
    let modes = [
        (RunMode::Forward, "forward", None),
        (RunMode::Remediating, "remediating", None),
        (RunMode::ManualBlocked, "manual_blocked", None),
        (RunMode::Completed, "completed", None),
        (
            RunMode::Compensated,
            "compensated",
            Some(events::RunCompletionOutcome::Compensated),
        ),
        (
            RunMode::ManuallyResolved,
            "manually_resolved",
            Some(events::RunCompletionOutcome::ManuallyResolved),
        ),
        (
            RunMode::FailedWithoutAcdcClaim,
            "failed_without_acdc_claim",
            Some(events::RunCompletionOutcome::FailedWithoutAcdcClaim),
        ),
    ];
    for (mode, tag, terminal) in modes {
        assert_eq!(mode.as_str(), tag);
        assert_eq!(mode.saga_terminal_outcome(), terminal);
    }

    let outcomes = [
        (completed_outcome(21), RunMode::Completed, "completed"),
        (
            events::RunCompletionOutcome::Compensated,
            RunMode::Compensated,
            "compensated",
        ),
        (
            events::RunCompletionOutcome::ManuallyResolved,
            RunMode::ManuallyResolved,
            "manually_resolved",
        ),
        (
            events::RunCompletionOutcome::FailedWithoutAcdcClaim,
            RunMode::FailedWithoutAcdcClaim,
            "failed_without_acdc_claim",
        ),
    ];
    for (outcome, mode, tag) in outcomes {
        assert_eq!(RunMode::from_completion_outcome(&outcome), mode);
        assert_eq!(
            mfm_store::v1::codec::run_completion_outcome_str(&outcome),
            tag
        );
    }

    let owner = events::RunnerInvocationId::new("owner-1").expect("owner");
    let token = side_effect::ClaimFencingToken::new("token-1").expect("token");
    let phases = [
        (
            SideEffectPhase::IntentPersisted {
                invocation_epoch: 1,
            },
            "intent_persisted",
        ),
        (
            SideEffectPhase::Claimed {
                claim_owner: owner.clone(),
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: token.clone(),
            },
            "claimed",
        ),
        (
            SideEffectPhase::InvocationPrepared {
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: token.clone(),
            },
            "invocation_prepared",
        ),
        (
            SideEffectPhase::InvocationStarted {
                claim_owner: owner,
                invocation_epoch: 1,
                claim_generation: 1,
                claim_fencing_token: token,
            },
            "invocation_started",
        ),
        (
            SideEffectPhase::SubmissionObserved {
                invocation_epoch: 1,
            },
            "submission_observed",
        ),
        (
            SideEffectPhase::NotSubmittedProven {
                invocation_epoch: 1,
            },
            "not_submitted_proven",
        ),
        (
            SideEffectPhase::SubmissionUnknown {
                invocation_epoch: 1,
            },
            "submission_unknown",
        ),
        (
            SideEffectPhase::ReceiptObserved {
                invocation_epoch: 1,
            },
            "receipt_observed",
        ),
        (
            SideEffectPhase::ConfirmationObserved {
                invocation_epoch: 1,
            },
            "confirmation_observed",
        ),
        (
            SideEffectPhase::Ambiguous {
                invocation_epoch: 1,
            },
            "ambiguous",
        ),
        (
            SideEffectPhase::Failed {
                invocation_epoch: 1,
                failure_phase: side_effect::FailurePhase::BeforeInvocationStarted,
            },
            "failed",
        ),
    ];
    for (phase, tag) in phases {
        assert_eq!(phase.as_str(), tag);
    }
}

#[test]
fn resource_lane_rejects_same_key_for_other_ledgers_same_run_and_cross_run() {
    let mut store = InMemoryTypedRunStore::new();
    let run_a = run_id(201);
    let run_b = run_id(202);
    store
        .append_prepared_commit(run_start_request(run_a.clone(), "resource-run-a-start"))
        .expect("append run a");
    store
        .append_prepared_commit(run_start_request(run_b.clone(), "resource-run-b-start"))
        .expect("append run b");

    append_side_effect_prepare_for_ledger(
        &mut store,
        &run_a,
        "resource-a-prepare",
        side_effect_ledger_key_with_suffix(1),
        resource_key("wallet-1", 201),
        20,
        true,
    );

    let same_run_error = {
        let branch_node = node_id(73);
        let branch_attempt = attempt_id(74);
        let mut intent = side_effect_intent(artifact_id(22), content_digest(23));
        let mut claim = side_effect_claim();
        let mut prepared =
            side_effect_prepared_with_resource_key(1, "token-1", resource_key("wallet-1", 201));
        for payload in [&mut intent, &mut claim, &mut prepared] {
            set_side_effect_ledger(
                payload,
                side_effect_ledger_key_with_suffix(2),
                events::SideEffectLedgerPurpose::Forward,
            );
            set_side_effect_node_attempt(payload, branch_node.clone(), branch_attempt.clone());
        }
        store
            .append_prepared_commit(typed_commit_request! {
                run_id: run_a.clone(),
                expected_next_seq: store.expected_next_seq(&run_a),
                commit_key: CommitKey::new("resource-a-conflict-attempt-start")
                    .expect("commit key"),
                payloads: vec![side_effect_attempt_started_for(
                    branch_node.clone(),
                    branch_attempt.clone(),
                )],
                required_artifacts: Vec::new(),
                preconditions: CommitPreconditions::default(),
            })
            .expect("append branch attempt start");
        store
            .append_prepared_commit(typed_commit_request! {
                run_id: run_a.clone(),
                expected_next_seq: store.expected_next_seq(&run_a),
                commit_key: CommitKey::new("resource-a-conflict").expect("commit key"),
                payloads: vec![intent, claim, prepared],
                required_artifacts: vec![intent_artifact_ref_for_node(
                    artifact_id(22),
                    content_digest(23),
                    branch_node,
                )],
                preconditions: CommitPreconditions::default(),
            })
            .expect_err("same-run lane conflict rejects")
    };
    assert_resource_lane_blocked(same_run_error, &resource_lane_key("wallet-1"));

    let cross_run_error = {
        let branch_node = node_id(75);
        let branch_attempt = attempt_id(76);
        let mut intent = side_effect_intent(artifact_id(24), content_digest(25));
        let mut claim = side_effect_claim();
        let mut prepared =
            side_effect_prepared_with_resource_key(1, "token-1", resource_key("wallet-1", 201));
        for payload in [&mut intent, &mut claim, &mut prepared] {
            set_side_effect_ledger(
                payload,
                side_effect_ledger_key_with_suffix(3),
                events::SideEffectLedgerPurpose::Forward,
            );
            set_side_effect_node_attempt(payload, branch_node.clone(), branch_attempt.clone());
        }
        store
            .append_prepared_commit(typed_commit_request! {
                run_id: run_b.clone(),
                expected_next_seq: store.expected_next_seq(&run_b),
                commit_key: CommitKey::new("resource-b-conflict-attempt-start")
                    .expect("commit key"),
                payloads: vec![side_effect_attempt_started_for(
                    branch_node.clone(),
                    branch_attempt.clone(),
                )],
                required_artifacts: Vec::new(),
                preconditions: CommitPreconditions::default(),
            })
            .expect("append branch attempt start");
        store
            .append_prepared_commit(typed_commit_request! {
                run_id: run_b.clone(),
                expected_next_seq: store.expected_next_seq(&run_b),
                commit_key: CommitKey::new("resource-b-conflict").expect("commit key"),
                payloads: vec![intent, claim, prepared],
                required_artifacts: vec![intent_artifact_ref_for_node(
                    artifact_id(24),
                    content_digest(25),
                    branch_node,
                )],
                preconditions: CommitPreconditions::default(),
            })
            .expect_err("cross-run lane conflict rejects")
    };
    assert_resource_lane_blocked(cross_run_error, &resource_lane_key("wallet-1"));
}

#[test]
fn resource_lane_holder_identity_includes_run_for_same_ledger_key_across_runs() {
    let mut store = InMemoryTypedRunStore::new();
    let run_a = run_id(213);
    let run_b = run_id(214);
    let shared_ledger = side_effect_ledger_key_with_suffix(11);
    let lane_a = resource_lane_key("wallet-6");
    let lane_b = resource_lane_key("wallet-7");
    store
        .append_prepared_commit(run_start_request(
            run_a.clone(),
            "resource-same-ledger-a-start",
        ))
        .expect("append run a");
    store
        .append_prepared_commit(run_start_request(
            run_b.clone(),
            "resource-same-ledger-b-start",
        ))
        .expect("append run b");

    append_side_effect_prepare_for_ledger(
        &mut store,
        &run_a,
        "resource-same-ledger-a-prepare",
        shared_ledger.clone(),
        resource_key("wallet-6", 213),
        28,
        true,
    );

    let artifact_id = artifact_id(30);
    let artifact_digest = content_digest(31);
    let branch_node = node_id(78);
    let branch_attempt = attempt_id(79);
    let mut intent = side_effect_intent(artifact_id.clone(), artifact_digest.clone());
    let mut claim = side_effect_claim();
    let mut prepared =
        side_effect_prepared_with_resource_key(1, "token-1", resource_key("wallet-6", 213));
    for payload in [&mut intent, &mut claim, &mut prepared] {
        set_side_effect_ledger(
            payload,
            shared_ledger.clone(),
            events::SideEffectLedgerPurpose::Forward,
        );
        set_side_effect_node_attempt(payload, branch_node.clone(), branch_attempt.clone());
    }
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_b.clone(),
            expected_next_seq: store.expected_next_seq(&run_b),
            commit_key: CommitKey::new("resource-same-ledger-b-conflict-attempt-start")
                .expect("commit key"),
            payloads: vec![side_effect_attempt_started_for(
                branch_node.clone(),
                branch_attempt.clone(),
            )],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append branch attempt start");
    let cross_run_same_resource_error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_b.clone(),
            expected_next_seq: store.expected_next_seq(&run_b),
            commit_key: CommitKey::new("resource-same-ledger-b-conflict").expect("commit key"),
            payloads: vec![intent, claim, prepared],
            required_artifacts: vec![intent_artifact_ref_for_node(
                artifact_id,
                artifact_digest,
                branch_node,
            )],
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("same resource remains exclusive across runs");
    assert_resource_lane_blocked(cross_run_same_resource_error, &lane_a);

    append_side_effect_prepare_for_ledger_on_attempt(
        &mut store,
        &run_b,
        "resource-same-ledger-b-prepare",
        shared_ledger.clone(),
        resource_key("wallet-7", 214),
        32,
        true,
        node_id(80),
        attempt_id(81),
    );

    let projection = store.projection_snapshot();
    let lane_a = projection.resource_lane(&lane_a).expect("run a lane");
    assert_eq!(lane_a.holder.run_id, run_a);
    assert_eq!(lane_a.holder.ledger_key, shared_ledger);
    let lane_b = projection.resource_lane(&lane_b).expect("run b lane");
    assert_eq!(lane_b.holder.run_id, run_b);
    assert_eq!(lane_b.holder.ledger_key, shared_ledger);
    assert!(projection
        .side_effect_for_run(&run_a, &shared_ledger)
        .is_some());
    assert!(projection
        .side_effect_for_run(&run_b, &shared_ledger)
        .is_some());
}

#[test]
fn resource_lane_allows_same_ledger_refresh_with_same_key_only() {
    let mut store = InMemoryTypedRunStore::new();
    let run = run_id(203);
    let ledger = side_effect_ledger_key_with_suffix(4);
    store
        .append_prepared_commit(run_start_request(run.clone(), "resource-refresh-run-start"))
        .expect("append run");
    append_side_effect_prepare_for_ledger(
        &mut store,
        &run,
        "resource-refresh-prepare",
        ledger.clone(),
        resource_key("wallet-2", 202),
        26,
        true,
    );

    let mut takeover = side_effect_claim_taken_over_with_token("token-2");
    set_side_effect_ledger(
        &mut takeover,
        ledger.clone(),
        events::SideEffectLedgerPurpose::Forward,
    );
    let mut prepared =
        side_effect_prepared_with_resource_key(2, "token-2", resource_key("wallet-2", 202));
    set_side_effect_ledger(
        &mut prepared,
        ledger.clone(),
        events::SideEffectLedgerPurpose::Forward,
    );
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run.clone(),
            expected_next_seq: store.expected_next_seq(&run),
            commit_key: CommitKey::new("resource-refresh-same-key").expect("commit key"),
            payloads: vec![takeover, prepared],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("same ledger may refresh the same resource key");

    let mut takeover = side_effect_claim_taken_over_generation(2, 3, "token-3");
    let KernelEventPayload::SideEffectClaimTakenOver(payload) = &mut takeover else {
        unreachable!("helper returns takeover payload");
    };
    payload.previous_claim_owner = events::RunnerInvocationId::new("owner-2").expect("owner");
    payload.new_claim_owner = events::RunnerInvocationId::new("owner-3").expect("owner");
    set_side_effect_ledger(
        &mut takeover,
        ledger.clone(),
        events::SideEffectLedgerPurpose::Forward,
    );
    let mut changed =
        side_effect_prepared_with_resource_key(3, "token-3", resource_key("wallet-3", 203));
    set_side_effect_ledger(
        &mut changed,
        ledger,
        events::SideEffectLedgerPurpose::Forward,
    );
    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run.clone(),
            expected_next_seq: store.expected_next_seq(&run),
            commit_key: CommitKey::new("resource-refresh-changed-key").expect("commit key"),
            payloads: vec![takeover, changed],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("same ledger cannot change resource key");
    assert_projection_conflict_contains(error, "resource key evidence changed");
}

#[test]
fn resource_lane_releases_on_ledger_terminals_and_run_terminal() {
    let run = run_id(204);
    let lane_key = resource_lane_key("wallet-4");

    let mut not_submitted_store = InMemoryTypedRunStore::new();
    not_submitted_store
        .append_prepared_commit(run_start_request(
            run.clone(),
            "resource-not-submitted-run-start",
        ))
        .expect("append run");
    append_side_effect_prepare_for_ledger(
        &mut not_submitted_store,
        &run,
        "resource-not-submitted-prepare",
        side_effect_ledger_key_with_suffix(5),
        resource_key("wallet-4", 204),
        28,
        true,
    );
    let mut started = side_effect_started("owner-1", 1, "token-1");
    set_side_effect_ledger(
        &mut started,
        side_effect_ledger_key_with_suffix(5),
        events::SideEffectLedgerPurpose::Forward,
    );
    let mut not_submitted = side_effect_not_submitted(artifact_id(30), content_digest(31));
    set_side_effect_ledger(
        &mut not_submitted,
        side_effect_ledger_key_with_suffix(5),
        events::SideEffectLedgerPurpose::Forward,
    );
    not_submitted_store
        .append_prepared_commit(typed_commit_request! {
            run_id: run.clone(),
            expected_next_seq: not_submitted_store.expected_next_seq(&run),
            commit_key: CommitKey::new("resource-not-submitted-terminal").expect("commit key"),
            payloads: vec![started, not_submitted],
            required_artifacts: vec![side_effect_evidence(
                artifact_id(30),
                content_digest(31),
                not_submitted_schema(),
                ArtifactRole::NotSubmittedProof,
            )],
            preconditions: CommitPreconditions::default(),
        })
        .expect("not-submitted releases lane");
    assert!(not_submitted_store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_none());

    let run = run_id(205);
    let mut confirmation_store = InMemoryTypedRunStore::new();
    confirmation_store
        .append_prepared_commit(run_start_request(
            run.clone(),
            "resource-confirmation-run-start",
        ))
        .expect("append run");
    append_side_effect_prepare_for_ledger(
        &mut confirmation_store,
        &run,
        "resource-confirmation-prepare",
        side_effect_ledger_key_with_suffix(6),
        resource_key("wallet-4", 204),
        32,
        true,
    );
    for (commit_key, mut payload, artifact) in [
        (
            "resource-confirmation-start",
            side_effect_started("owner-1", 1, "token-1"),
            None,
        ),
        (
            "resource-confirmation-submission",
            side_effect_submission_observed(artifact_id(34), content_digest(35)),
            Some((
                artifact_id(34),
                content_digest(35),
                submission_schema(),
                ArtifactRole::Submission,
            )),
        ),
        (
            "resource-confirmation-receipt",
            side_effect_receipt(artifact_id(36), content_digest(37)),
            Some((
                artifact_id(36),
                content_digest(37),
                receipt_schema(),
                ArtifactRole::Receipt,
            )),
        ),
        (
            "resource-confirmation-confirmed",
            side_effect_confirmation(artifact_id(38), content_digest(39)),
            Some((
                artifact_id(38),
                content_digest(39),
                confirmation_schema(),
                ArtifactRole::Confirmation,
            )),
        ),
    ] {
        set_side_effect_ledger(
            &mut payload,
            side_effect_ledger_key_with_suffix(6),
            events::SideEffectLedgerPurpose::Forward,
        );
        let required_artifacts = artifact
            .map(|(artifact_id, digest, schema_id, role)| {
                vec![side_effect_evidence(artifact_id, digest, schema_id, role)]
            })
            .unwrap_or_default();
        confirmation_store
            .append_prepared_commit(typed_commit_request! {
                run_id: run.clone(),
                expected_next_seq: confirmation_store.expected_next_seq(&run),
                commit_key: CommitKey::new(commit_key).expect("commit key"),
                payloads: vec![payload],
                required_artifacts: required_artifacts,
                preconditions: CommitPreconditions::default(),
            })
            .expect("append confirmation path event");
    }
    assert!(confirmation_store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_none());

    let run = run_id(206);
    let mut failure_store = InMemoryTypedRunStore::new();
    failure_store
        .append_prepared_commit(run_start_request(run.clone(), "resource-failed-run-start"))
        .expect("append run");
    append_side_effect_prepare_for_ledger(
        &mut failure_store,
        &run,
        "resource-failed-prepare",
        side_effect_ledger_key_with_suffix(7),
        resource_key("wallet-4", 204),
        40,
        true,
    );
    let mut failed = side_effect_failed(true);
    set_side_effect_ledger(
        &mut failed,
        side_effect_ledger_key_with_suffix(7),
        events::SideEffectLedgerPurpose::Forward,
    );
    failure_store
        .append_prepared_commit(typed_commit_request! {
            run_id: run.clone(),
            expected_next_seq: failure_store.expected_next_seq(&run),
            commit_key: CommitKey::new("resource-failed-terminal").expect("commit key"),
            payloads: vec![failed, side_effect_attempt_failed(true)],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("failed releases lane");
    assert!(failure_store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_none());

    let run = run_id(207);
    let mut manual_store = InMemoryTypedRunStore::new();
    manual_store
        .append_prepared_commit(run_start_request_with_saga_policy(
            run.clone(),
            "resource-manual-run-start",
            &manual_saga_policy(46),
        ))
        .expect("append run");
    append_side_effect_prepare_for_ledger(
        &mut manual_store,
        &run,
        "resource-manual-prepare",
        side_effect_ledger_key_with_suffix(8),
        resource_key("wallet-4", 204),
        42,
        true,
    );
    let mut started = side_effect_started("owner-1", 1, "token-1");
    let mut ambiguous = side_effect_ambiguous(artifact_id(44), content_digest(45));
    for payload in [&mut started, &mut ambiguous] {
        set_side_effect_ledger(
            payload,
            side_effect_ledger_key_with_suffix(8),
            events::SideEffectLedgerPurpose::Forward,
        );
    }
    manual_store
        .append_prepared_commit(typed_commit_request! {
            run_id: run.clone(),
            expected_next_seq: manual_store.expected_next_seq(&run),
            commit_key: CommitKey::new("resource-manual-ambiguous").expect("commit key"),
            payloads: vec![started, ambiguous, side_effect_attempt_failed(false)],
            required_artifacts: vec![side_effect_evidence(
                artifact_id(44),
                content_digest(45),
                schema_id("mfm.test.ambiguity", 76),
                ArtifactRole::AmbiguityEvidence,
            )],
            preconditions: CommitPreconditions::default(),
        })
        .expect("ambiguous terminal evidence releases lane");
    assert!(manual_store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_none());
    let rebuilt = ProjectionSnapshot::rebuild_from_run_stream(&manual_store.load_run_stream(&run))
        .expect("rebuild ambiguous terminal stream");
    assert!(rebuilt.resource_lane(&lane_key).is_none());

    let second_run = run_id(209);
    manual_store
        .append_prepared_commit(run_start_request(
            second_run.clone(),
            "resource-second-run-start",
        ))
        .expect("append second run");
    append_side_effect_prepare_for_ledger_on_attempt(
        &mut manual_store,
        &second_run,
        "resource-second-run-prepare",
        side_effect_ledger_key_with_suffix(10),
        resource_key("wallet-4", 204),
        50,
        true,
        node_id(82),
        attempt_id(83),
    );
    let lane = manual_store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .expect("second run acquired released lane");
    assert_eq!(lane.holder.run_id, second_run);

    let run = run_id(208);
    let mut terminal_store = InMemoryTypedRunStore::new();
    terminal_store
        .append_prepared_commit(run_start_request(
            run.clone(),
            "resource-run-terminal-start",
        ))
        .expect("append run");
    append_side_effect_prepare_for_ledger(
        &mut terminal_store,
        &run,
        "resource-run-terminal-prepare",
        side_effect_ledger_key_with_suffix(9),
        resource_key("wallet-4", 204),
        48,
        true,
    );
    assert!(terminal_store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_some());
    terminal_store
        .append_prepared_commit(typed_commit_request! {
            run_id: run.clone(),
            expected_next_seq: terminal_store.expected_next_seq(&run),
            commit_key: CommitKey::new("resource-run-terminal-release").expect("commit key"),
            payloads: vec![run_completed_for_run(run.clone(), completed_outcome(209))],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("sealed run terminal releases lane");
    assert!(terminal_store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_none());
}

#[test]
fn resource_lane_reprepare_after_release_requires_stable_resource_key_evidence() {
    let run = run_id(212);
    let ledger = side_effect_ledger_key_with_suffix(12);
    let lane_key = resource_lane_key("wallet-stable");
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request(
            run.clone(),
            "resource-stability-run-start",
        ))
        .expect("append run");
    append_side_effect_prepare_for_ledger(
        &mut store,
        &run,
        "resource-stability-prepare",
        ledger.clone(),
        resource_key("wallet-stable", 212),
        52,
        true,
    );
    assert!(store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_some());

    let mut started = side_effect_started("owner-1", 1, "token-1");
    let mut not_submitted = side_effect_not_submitted(artifact_id(54), content_digest(55));
    for payload in [&mut started, &mut not_submitted] {
        set_side_effect_ledger(
            payload,
            ledger.clone(),
            events::SideEffectLedgerPurpose::Forward,
        );
    }
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run.clone(),
            expected_next_seq: store.expected_next_seq(&run),
            commit_key: CommitKey::new("resource-stability-release").expect("commit key"),
            payloads: vec![started, not_submitted],
            required_artifacts: vec![side_effect_evidence(
                artifact_id(54),
                content_digest(55),
                not_submitted_schema(),
                ArtifactRole::NotSubmittedProof,
            )],
            preconditions: CommitPreconditions::default(),
        })
        .expect("not-submitted releases lane");
    assert!(store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_none());

    let mut retry_claim = side_effect_claim_for_epoch(2, 2, "token-2");
    let mut retry_without_key = side_effect_prepared_for_epoch(2, 2, "token-2");
    for payload in [&mut retry_claim, &mut retry_without_key] {
        set_side_effect_ledger(
            payload,
            ledger.clone(),
            events::SideEffectLedgerPurpose::Forward,
        );
    }
    let missing_key = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run.clone(),
            expected_next_seq: store.expected_next_seq(&run),
            commit_key: CommitKey::new("resource-stability-missing-key").expect("commit key"),
            payloads: vec![retry_claim, retry_without_key],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("reprepare without prior resource key evidence rejects");
    assert_projection_conflict_contains(missing_key, "resource key evidence is required");

    let mut retry_claim = side_effect_claim_for_epoch(2, 2, "token-2");
    let mut retry_changed_key = side_effect_prepared_with_resource_key_for_epoch(
        2,
        2,
        "token-2",
        resource_key("wallet-changed", 213),
    );
    for payload in [&mut retry_claim, &mut retry_changed_key] {
        set_side_effect_ledger(
            payload,
            ledger.clone(),
            events::SideEffectLedgerPurpose::Forward,
        );
    }
    let changed_key = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run.clone(),
            expected_next_seq: store.expected_next_seq(&run),
            commit_key: CommitKey::new("resource-stability-changed-key").expect("commit key"),
            payloads: vec![retry_claim, retry_changed_key],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("reprepare with changed resource key evidence rejects");
    assert_projection_conflict_contains(changed_key, "resource key evidence changed");
}

#[test]
fn resource_lane_projection_rebuilds_from_non_terminal_run_stream() {
    let mut store = InMemoryTypedRunStore::new();
    let run = run_id(209);
    let lane_key = resource_lane_key("wallet-5");
    store
        .append_prepared_commit(run_start_request(run.clone(), "resource-rebuild-run-start"))
        .expect("append run");
    append_side_effect_prepare_for_ledger(
        &mut store,
        &run,
        "resource-rebuild-prepare",
        side_effect_ledger_key_with_suffix(10),
        resource_key("wallet-5", 205),
        50,
        true,
    );

    let stream = store.load_run_stream(&run);
    let rebuilt = ProjectionSnapshot::rebuild_from_run_stream(&stream).expect("rebuild projection");
    let lane = rebuilt
        .resource_lane(&lane_key)
        .expect("rebuilt non-terminal lane");
    assert_eq!(lane.holder.run_id, run);
    assert_eq!(
        lane.holder.ledger_key,
        side_effect_ledger_key_with_suffix(10)
    );
}

#[test]
fn resource_touched_set_evidence_is_schema_checked_at_admission() {
    let run = run_id(210);
    let touched = resource_touched_set(52);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request(run.clone(), "resource-touched-run-start"))
        .expect("append run");
    append_side_effect_prepare(&mut store, &run);
    append_side_effect_started(&mut store, &run);
    let submission_artifact_id = artifact_id(54);
    let submission_digest = content_digest(55);
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run.clone(),
            expected_next_seq: store.expected_next_seq(&run),
            commit_key: CommitKey::new("resource-touched-submission").expect("commit key"),
            payloads: vec![side_effect_submission_observed(
                submission_artifact_id.clone(),
                submission_digest.clone(),
            )],
            required_artifacts: vec![side_effect_evidence(
                submission_artifact_id,
                submission_digest,
                submission_schema(),
                ArtifactRole::Submission,
            )],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append submission");

    let mut receipt = side_effect_receipt(artifact_id(56), content_digest(57));
    let KernelEventPayload::SideEffectReceiptObserved(payload) = &mut receipt else {
        unreachable!("helper returns receipt");
    };
    payload.resource_touched_set = Some(touched.clone());
    let mut wrong_touched_artifact = resource_touched_set_artifact_ref(&touched);
    wrong_touched_artifact.schema_id = Some(schema_id("mfm.test.wrong_touched_set", 53));
    let error = store
        .append_prepared_commit_with_artifacts(
            typed_commit_request! {
                run_id: run.clone(),
                expected_next_seq: store.expected_next_seq(&run),
                commit_key: CommitKey::new("resource-touched-wrong-schema").expect("commit key"),
                payloads: vec![receipt.clone()],
                required_artifacts: vec![
                    side_effect_evidence(
                        artifact_id(56),
                        content_digest(57),
                        receipt_schema(),
                        ArtifactRole::Receipt,
                    ),
                    resource_touched_set_artifact_ref(&touched),
                ],
                preconditions: CommitPreconditions::default(),
            },
            vec![
                side_effect_evidence(
                    artifact_id(56),
                    content_digest(57),
                    receipt_schema(),
                    ArtifactRole::Receipt,
                ),
                wrong_touched_artifact,
            ],
        )
        .expect_err("touched-set schema mismatch rejects");
    assert!(matches!(
        error,
        StoreError::ArtifactEvidenceMismatch {
            field: "schema_id",
            ..
        }
    ));

    store
        .append_prepared_commit_with_artifacts(
            typed_commit_request! {
                run_id: run.clone(),
                expected_next_seq: store.expected_next_seq(&run),
                commit_key: CommitKey::new("resource-touched-valid").expect("commit key"),
                payloads: vec![receipt],
                required_artifacts: vec![
                    side_effect_evidence(
                        artifact_id(56),
                        content_digest(57),
                        receipt_schema(),
                        ArtifactRole::Receipt,
                    ),
                    resource_touched_set_artifact_ref(&touched),
                ],
                preconditions: CommitPreconditions::default(),
            },
            vec![
                side_effect_evidence(
                    artifact_id(56),
                    content_digest(57),
                    receipt_schema(),
                    ArtifactRole::Receipt,
                ),
                resource_touched_set_artifact_ref(&touched),
            ],
        )
        .expect("valid touched-set evidence admits");
    let projection = store
        .projection_snapshot()
        .side_effect(&side_effect_ledger_key())
        .expect("side-effect projection");
    assert_eq!(projection.resource_touched_set, Some(touched));
}

#[test]
fn side_effect_ledger_purpose_is_required_and_closed() {
    let payload = side_effect_claim();
    let mut missing = payload_json_value(&payload);
    missing
        .as_object_mut()
        .expect("payload object")
        .remove("ledger_purpose");
    assert!(matches!(
        payload_from_json_value(&missing),
        Err(StoreError::Event(message)) if message.contains("ledger_purpose")
    ));

    let mut unknown = payload_json_value(&payload);
    unknown
        .get_mut("ledger_purpose")
        .and_then(serde_json::Value::as_object_mut)
        .expect("ledger purpose object")
        .insert(
            "kind".to_owned(),
            serde_json::Value::String("other".to_owned()),
        );
    assert!(matches!(
        payload_from_json_value(&unknown),
        Err(StoreError::Identity(message))
            if message.contains("unknown side-effect ledger purpose")
    ));
}

#[test]
fn side_effect_ledger_purpose_cannot_change_after_intent() {
    let run_id = run_id(92);
    let artifact_id = artifact_id(93);
    let artifact_digest = content_digest(94);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "purpose-run-start"))
        .expect("append run start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("purpose-attempt-start").expect("commit key"),
            payloads: vec![side_effect_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append sidefx attempt start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("purpose-intent").expect("commit key"),
            payloads: vec![side_effect_intent(
                artifact_id.clone(),
                artifact_digest.clone(),
            )],
            required_artifacts: vec![intent_artifact_ref(artifact_id, artifact_digest)],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append intent");

    let mut changed = side_effect_claim();
    let KernelEventPayload::SideEffectClaimed(payload) = &mut changed else {
        unreachable!("helper returns claimed payload");
    };
    payload.ledger_purpose = events::SideEffectLedgerPurpose::Remediation {
        forward_ledger_key: side_effect_ledger_key(),
    };
    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("purpose-changed-claim").expect("commit key"),
            payloads: vec![changed],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("ledger purpose change rejects");
    assert!(matches!(
        error,
        StoreError::ProjectionConflict { message, .. }
            if message.contains("ledger purpose changed")
    ));
}

#[test]
fn removed_run_completion_outcome_tags_are_rejected() {
    let payload = KernelEventPayload::RunCompleted(events::RunCompleted {
        run_id: run_id(95),
        spec_hash: spec_hash(1),
        outcome: events::RunCompletionOutcome::FailedWithoutAcdcClaim,
    });
    for removed in ["failed", "cancelled"] {
        let mut json = payload_json_value(&payload);
        json.get_mut("outcome")
            .and_then(serde_json::Value::as_object_mut)
            .expect("outcome object")
            .insert(
                "kind".to_owned(),
                serde_json::Value::String(removed.to_owned()),
            );
        assert!(matches!(
            payload_from_json_value(&json),
            Err(StoreError::Identity(message))
                if message.contains(&format!("unknown run completion outcome {removed}"))
        ));
    }
}

#[test]
fn manual_resolution_outcome_is_closed() {
    let payload = KernelEventPayload::ManualResolutionRecorded(events::ManualResolutionRecorded {
        run_id: run_id(96),
        spec_hash: spec_hash(1),
        outcome: events::ManualResolutionOutcome::ConfirmRemediated,
        evidence_schema_id: schema_id("mfm.test.manual_evidence", 97),
        evidence_hash: content_digest(97),
        evidence_artifact_id: artifact_id(97),
        authorization_schema_id: schema_id("mfm.test.manual_authorization", 98),
        authorization_hash: content_digest(98),
        authorization_artifact_id: artifact_id(98),
        note: Some(events::ManualResolutionNote::new("reviewed evidence").expect("note")),
    });
    assert!(matches!(
        payload_from_json_value(&payload_json_value(&payload)),
        Ok(KernelEventPayload::ManualResolutionRecorded(_))
    ));

    let mut json = payload_json_value(&payload);
    json.as_object_mut().expect("payload object").insert(
        "outcome".to_owned(),
        serde_json::Value::String("invented".to_owned()),
    );
    assert!(matches!(
        payload_from_json_value(&json),
        Err(StoreError::Identity(message))
            if message.contains("unknown manual resolution outcome invented")
    ));

    let mut old = payload_json_value(&payload);
    let old_object = old.as_object_mut().expect("payload object");
    old_object.remove("authorization_schema_id");
    old_object.remove("authorization_hash");
    old_object.remove("authorization_artifact_id");
    old_object.insert(
        "operator_identity_ref_schema_id".to_owned(),
        serde_json::Value::String(schema_id("mfm.test.operator_identity", 99).to_string()),
    );
    old_object.insert(
        "operator_identity_ref_hash".to_owned(),
        serde_json::Value::String(content_digest(99).to_string()),
    );
    old_object.insert(
        "operator_identity_ref_artifact_id".to_owned(),
        serde_json::Value::String(artifact_id(99).to_string()),
    );
    assert!(matches!(
        payload_from_json_value(&old),
        Err(StoreError::Event(message)) if message.contains("authorization_schema_id")
    ));
}

#[test]
fn forward_fence_rejects_boundary_events_after_engagement() {
    fn started_store() -> (RunId, InMemoryTypedRunStore) {
        let run_id = run_id(120);
        let mut store = InMemoryTypedRunStore::new();
        store
            .append_prepared_commit(run_start_request(run_id.clone(), "fence-run-start"))
            .expect("append run start");
        store
            .append_prepared_commit(typed_commit_request! {
                run_id: run_id.clone(),
                expected_next_seq: store.expected_next_seq(&run_id),
                commit_key: CommitKey::new("fence-sidefx-attempt-start").expect("commit key"),
                payloads: vec![side_effect_attempt_started()],
                required_artifacts: Vec::new(),
                preconditions: CommitPreconditions::default(),
            })
            .expect("append sidefx attempt start");
        (run_id, store)
    }

    let (run_id, mut store) = started_store();
    append_generic_nonretryable_failure(&mut store, &run_id, "fence-intent");
    let intent_artifact_id = artifact_id(130);
    let intent_digest = content_digest(131);
    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fence-intent-reject").expect("commit key"),
            payloads: vec![side_effect_intent(
                intent_artifact_id.clone(),
                intent_digest.clone(),
            )],
            required_artifacts: vec![intent_artifact_ref(intent_artifact_id, intent_digest)],
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("intent rejects after engagement");
    assert_projection_conflict_contains(error, "forward side-effect boundary");

    let (run_id, mut store) = started_store();
    let intent_artifact_id = artifact_id(132);
    let intent_digest = content_digest(133);
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fence-claim-intent").expect("commit key"),
            payloads: vec![side_effect_intent(
                intent_artifact_id.clone(),
                intent_digest.clone(),
            )],
            required_artifacts: vec![intent_artifact_ref(intent_artifact_id, intent_digest)],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append intent before engagement");
    append_generic_nonretryable_failure(&mut store, &run_id, "fence-claim");
    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fence-claim-reject").expect("commit key"),
            payloads: vec![side_effect_claim()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("claim rejects after engagement");
    assert_projection_conflict_contains(error, "forward side-effect boundary");

    let (run_id, mut store) = started_store();
    let intent_artifact_id = artifact_id(134);
    let intent_digest = content_digest(135);
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fence-takeover-intent-claim").expect("commit key"),
            payloads: vec![
                side_effect_intent(intent_artifact_id.clone(), intent_digest.clone()),
                side_effect_claim(),
            ],
            required_artifacts: vec![intent_artifact_ref(intent_artifact_id, intent_digest)],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append claim before engagement");
    append_generic_nonretryable_failure(&mut store, &run_id, "fence-takeover");
    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fence-takeover-reject").expect("commit key"),
            payloads: vec![side_effect_claim_taken_over()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("claim takeover rejects after engagement");
    assert_projection_conflict_contains(error, "forward side-effect boundary");

    let (run_id, mut store) = started_store();
    let intent_artifact_id = artifact_id(136);
    let intent_digest = content_digest(137);
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fence-prepared-intent-claim").expect("commit key"),
            payloads: vec![
                side_effect_intent(intent_artifact_id.clone(), intent_digest.clone()),
                side_effect_claim(),
            ],
            required_artifacts: vec![intent_artifact_ref(intent_artifact_id, intent_digest)],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append claim before engagement");
    append_generic_nonretryable_failure(&mut store, &run_id, "fence-prepared");
    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fence-prepared-reject").expect("commit key"),
            payloads: vec![side_effect_prepared(1, "token-1")],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("prepared rejects after engagement");
    assert_projection_conflict_contains(error, "forward side-effect boundary");

    let (run_id, mut store) = started_store();
    let intent_artifact_id = artifact_id(138);
    let intent_digest = content_digest(139);
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fence-started-prepare").expect("commit key"),
            payloads: vec![
                side_effect_intent(intent_artifact_id.clone(), intent_digest.clone()),
                side_effect_claim(),
                side_effect_prepared(1, "token-1"),
            ],
            required_artifacts: vec![intent_artifact_ref(intent_artifact_id, intent_digest)],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append prepared before engagement");
    append_generic_nonretryable_failure(&mut store, &run_id, "fence-started");
    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fence-started-reject").expect("commit key"),
            payloads: vec![side_effect_started("owner-1", 1, "token-1")],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("started rejects after engagement");
    assert_projection_conflict_contains(error, "forward side-effect boundary");
}

#[test]
fn remediation_intent_requires_engaged_confirmed_forward_and_unique_link() {
    let run_id = run_id(120);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "remediation-run-start"))
        .expect("append run start");
    append_forward_confirmation(&mut store, &run_id);

    let mut remediation_intent = side_effect_intent(artifact_id(140), content_digest(141));
    set_remediation_purpose(&mut remediation_intent, remediation_ledger_key(1));
    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("remediation-before-engagement").expect("commit key"),
            payloads: vec![remediation_intent],
            required_artifacts: vec![intent_artifact_ref(artifact_id(140), content_digest(141))],
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("remediation before engagement rejects");
    assert_projection_conflict_contains(error, "prior saga engagement");

    append_generic_nonretryable_failure(&mut store, &run_id, "remediation-engagement");
    let mut remediation_intent = side_effect_intent(artifact_id(142), content_digest(143));
    set_remediation_purpose(&mut remediation_intent, remediation_ledger_key(1));
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("remediation-admitted").expect("commit key"),
            payloads: vec![remediation_intent],
            required_artifacts: vec![intent_artifact_ref(artifact_id(142), content_digest(143))],
            preconditions: CommitPreconditions::default(),
        })
        .expect("confirmed forward remediation is admitted after engagement");

    let mut duplicate = side_effect_intent(artifact_id(144), content_digest(145));
    set_remediation_purpose(&mut duplicate, remediation_ledger_key(2));
    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("remediation-duplicate").expect("commit key"),
            payloads: vec![duplicate],
            required_artifacts: vec![intent_artifact_ref(artifact_id(144), content_digest(145))],
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("duplicate remediation rejects");
    assert_projection_conflict_contains(error, "already exists for forward ledger");

    let unconfirmed_run_id = RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(121));
    let mut unconfirmed = InMemoryTypedRunStore::new();
    unconfirmed
        .append_prepared_commit(run_start_request(
            unconfirmed_run_id.clone(),
            "unconfirmed-run-start",
        ))
        .expect("append run start");
    append_side_effect_prepare(&mut unconfirmed, &unconfirmed_run_id);
    append_generic_nonretryable_failure(
        &mut unconfirmed,
        &unconfirmed_run_id,
        "unconfirmed-engagement",
    );
    let mut remediation_intent = side_effect_intent(artifact_id(146), content_digest(147));
    set_remediation_purpose(&mut remediation_intent, remediation_ledger_key(3));
    let error = unconfirmed
        .append_prepared_commit(typed_commit_request! {
            run_id: unconfirmed_run_id.clone(),
            expected_next_seq: unconfirmed.expected_next_seq(&unconfirmed_run_id),
            commit_key: CommitKey::new("remediation-unconfirmed").expect("commit key"),
            payloads: vec![remediation_intent],
            required_artifacts: vec![intent_artifact_ref(artifact_id(146), content_digest(147))],
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("unconfirmed forward remediation rejects");
    assert_projection_conflict_contains(error, "requires confirmed forward ledger");
}

#[test]
fn manual_resolution_requires_manual_blocked_prefix_and_is_unique() {
    let run_id = run_id(220);
    let mut non_quiescent = InMemoryTypedRunStore::new();
    non_quiescent
        .append_prepared_commit(run_start_request_with_saga_policy(
            run_id.clone(),
            "manual-quiescence-run-start",
            &proof_manual_saga_policy(),
        ))
        .expect("append run start");
    append_side_effect_prepare(&mut non_quiescent, &run_id);
    append_side_effect_started(&mut non_quiescent, &run_id);
    append_generic_nonretryable_failure(&mut non_quiescent, &run_id, "manual-quiescence");
    let verified =
        verified_manual_resolution_for_seq(non_quiescent.expected_next_seq(&run_id).as_u64());
    let prepared = prepared_manual_resolution_commit(
        &verified,
        non_quiescent.expected_next_seq(&run_id),
        "manual-non-quiescent",
        proof_manual_saga_policy(),
    );
    let error = non_quiescent
        .append_prepared_commit_plan(prepared.into())
        .expect_err("manual resolution rejects with open non-quiescent attempt");
    assert_projection_conflict_contains(error, "requires no open semantic attempts");

    let mut remediating = InMemoryTypedRunStore::new();
    remediating
        .append_prepared_commit(run_start_request_with_saga_policy(
            run_id.clone(),
            "manual-remediating-run-start",
            &compensate_saga_policy(),
        ))
        .expect("append run start");
    append_forward_confirmation(&mut remediating, &run_id);
    append_generic_nonretryable_failure(&mut remediating, &run_id, "manual-remediating");
    let error = remediating
        .append_prepared_commit(manual_resolution_request(
            &run_id,
            remediating.expected_next_seq(&run_id),
            "manual-remediating-reject",
            151,
            compensate_saga_policy(),
        ))
        .expect_err("manual resolution rejects while remediating");
    assert_invalid_prepared_commit_contains(error, "requires verified manual resolution proof");

    let raw_error = remediating
        .append_prepared_commit(manual_resolution_request(
            &run_id,
            remediating.expected_next_seq(&run_id),
            "manual-raw-without-proof",
            151,
            compensate_saga_policy(),
        ))
        .expect_err("raw manual resolution rejects without proof");
    assert_invalid_prepared_commit_contains(raw_error, "requires verified manual resolution proof");

    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request_with_saga_policy(
            run_id.clone(),
            "manual-run-start",
            &proof_manual_saga_policy(),
        ))
        .expect("append run start");
    append_forward_confirmation(&mut store, &run_id);
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("manual-clean-sidefx-failure").expect("commit key"),
            payloads: vec![side_effect_attempt_failed(false)],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append confirmed side-effect attempt failure");
    let verified = verified_manual_resolution_for_seq(store.expected_next_seq(&run_id).as_u64());
    let prepared = prepared_manual_resolution_commit(
        &verified,
        store.expected_next_seq(&run_id),
        "manual-recorded",
        proof_manual_saga_policy(),
    );
    store
        .append_prepared_commit_plan(prepared.into())
        .expect("manual resolution admitted in manual-blocked mode");
    let verified = verified_manual_resolution_for_seq(store.expected_next_seq(&run_id).as_u64());
    let prepared = prepared_manual_resolution_commit(
        &verified,
        store.expected_next_seq(&run_id),
        "manual-duplicate",
        proof_manual_saga_policy(),
    );
    let error = store
        .append_prepared_commit_plan(prepared.into())
        .expect_err("duplicate manual resolution rejects");
    match error {
        StoreError::LogicalKeyConflict { .. } | StoreError::DuplicateLogicalKey { .. } => {}
        StoreError::ProjectionConflict { message, .. }
            if message.contains("manual resolution already recorded") => {}
        other => panic!("unexpected duplicate manual resolution error: {other:?}"),
    }
}

#[test]
fn manual_resolution_prepared_commit_requires_matching_verified_proof() {
    let expected_next_seq = StreamSeq::new(7).expect("stream seq");
    let verified = verified_manual_resolution_for_seq(expected_next_seq.as_u64());
    let request = manual_resolution_request_from_verified(
        &verified,
        expected_next_seq,
        "manual-without-proof",
        proof_manual_saga_policy(),
    );
    let artifacts = manual_resolution_artifacts_from_verified(&verified);
    let error = test_prepared_commit_plan(request.clone(), artifacts.clone())
        .expect_err("manual resolution rejects without proof authority");
    assert_invalid_prepared_commit_contains(error, "requires verified manual resolution proof");

    let stale_request = manual_resolution_request_from_verified(
        &verified,
        StreamSeq::new(8).expect("stream seq"),
        "manual-stale-proof",
        proof_manual_saga_policy(),
    );
    let error = PreparedCommit::<ManualResolution>::new(
        stale_request,
        CommitArtifactEvidenceSet::new(artifacts.clone(), artifacts.clone())
            .expect("manual artifact evidence set"),
        &verified,
    )
    .expect_err("stale proof rejects");
    assert_invalid_prepared_commit_contains(error, "expected_next_seq");

    let mut mismatched_payload = manual_resolution_payload_from_verified(&verified);
    let KernelEventPayload::ManualResolutionRecorded(payload) = &mut mismatched_payload else {
        unreachable!("helper returns manual resolution payload");
    };
    payload.evidence_hash = content_digest(202);
    let mut mismatched_artifacts = manual_resolution_artifacts_from_verified(&verified);
    mismatched_artifacts[0].digest = content_digest(202);
    let mismatched_request = TypedCommitRequest::from_payloads(
        verified.claim().run_id.clone(),
        expected_next_seq,
        CommitKey::new("manual-mismatched-proof").expect("commit key"),
        vec![mismatched_payload],
        mismatched_artifacts.clone(),
        CommitPreconditions {
            required_run_state: RequiredRunState::NotCompleted,
            ..saga_preconditions(&verified.claim().run_id, proof_manual_saga_policy())
        },
    )
    .expect("manual resolution request");
    let error = PreparedCommit::<ManualResolution>::new(
        mismatched_request,
        CommitArtifactEvidenceSet::new(mismatched_artifacts.clone(), mismatched_artifacts)
            .expect("manual artifact evidence set"),
        &verified,
    )
    .expect_err("mismatched proof rejects");
    assert_invalid_prepared_commit_contains(error, "artifact refs do not match manual proof");
}

#[test]
fn manual_resolution_rejects_with_same_run_open_attempt() {
    let run_id = run_id(220);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request_with_saga_policy(
            run_id.clone(),
            "manual-open-attempt-run-start",
            &proof_manual_saga_policy(),
        ))
        .expect("append run start");
    append_forward_confirmation(&mut store, &run_id);
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("manual-open-attempt-sidefx-failure").expect("commit key"),
            payloads: vec![side_effect_attempt_failed(false)],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append confirmed side-effect attempt failure");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("manual-unrelated-open-attempt").expect("commit key"),
            payloads: vec![fact_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append unrelated open attempt");

    let verified = verified_manual_resolution_for_seq(store.expected_next_seq(&run_id).as_u64());
    let prepared = prepared_manual_resolution_commit(
        &verified,
        store.expected_next_seq(&run_id),
        "manual-open-attempt-reject",
        proof_manual_saga_policy(),
    );
    let error = store
        .append_prepared_commit_plan(prepared.into())
        .expect_err("manual resolution rejects open attempt");
    assert_projection_conflict_contains(error, "requires no open semantic attempts");
}

#[test]
fn manual_resolution_admits_with_unrelated_run_open_attempt() {
    let manual_run_id = run_id(220);
    let other_run_id = run_id(221);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request_with_saga_policy(
            manual_run_id.clone(),
            "manual-cross-run-start",
            &proof_manual_saga_policy(),
        ))
        .expect("append manual run start");
    append_forward_confirmation(&mut store, &manual_run_id);
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: manual_run_id.clone(),
            expected_next_seq: store.expected_next_seq(&manual_run_id),
            commit_key: CommitKey::new("manual-cross-run-sidefx-failure").expect("commit key"),
            payloads: vec![side_effect_attempt_failed(false)],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append confirmed side-effect attempt failure");

    store
        .append_prepared_commit(run_start_request_with_saga_policy(
            other_run_id.clone(),
            "manual-cross-run-other-start",
            &proof_manual_saga_policy(),
        ))
        .expect("append other run start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: other_run_id.clone(),
            expected_next_seq: store.expected_next_seq(&other_run_id),
            commit_key: CommitKey::new("manual-cross-run-other-open-attempt").expect("commit key"),
            payloads: vec![fact_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append other run open attempt");
    assert!(store
        .projection_snapshot()
        .open_attempt_for_run(&other_run_id)
        .is_some());
    assert!(store
        .projection_snapshot()
        .open_attempt_for_run(&manual_run_id)
        .is_none());

    let verified =
        verified_manual_resolution_for_seq(store.expected_next_seq(&manual_run_id).as_u64());
    let prepared = prepared_manual_resolution_commit(
        &verified,
        store.expected_next_seq(&manual_run_id),
        "manual-cross-run-admit",
        proof_manual_saga_policy(),
    );
    store
        .append_prepared_commit_plan(prepared.into())
        .expect("manual resolution ignores unrelated run open attempt");
    assert!(store
        .projection_snapshot()
        .manual_resolution(&manual_run_id)
        .is_some());
    assert!(store
        .projection_snapshot()
        .open_attempt_for_run(&other_run_id)
        .is_some());
}

#[test]
fn manual_resolution_rejects_open_side_effect_lane_without_releasing_it() {
    let run_id = run_id(220);
    let lane_key = resource_lane_key("wallet-open");
    let open_ledger = side_effect_ledger_key_with_suffix(31);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request_with_saga_policy(
            run_id.clone(),
            "manual-open-lane-run-start",
            &proof_manual_saga_policy(),
        ))
        .expect("append run start");
    append_forward_confirmation(&mut store, &run_id);
    append_side_effect_prepare_for_ledger_on_attempt(
        &mut store,
        &run_id,
        "manual-open-lane-prepare",
        open_ledger.clone(),
        resource_key("wallet-open", 220),
        70,
        true,
        node_id(80),
        attempt_id(81),
    );
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("manual-open-lane-sidefx-failure").expect("commit key"),
            payloads: vec![side_effect_attempt_failed(false)],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append confirmed side-effect attempt failure");
    assert!(store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .is_some());

    let verified = verified_manual_resolution_for_seq(store.expected_next_seq(&run_id).as_u64());
    let prepared = prepared_manual_resolution_commit(
        &verified,
        store.expected_next_seq(&run_id),
        "manual-open-lane-reject",
        proof_manual_saga_policy(),
    );
    let error = store
        .append_prepared_commit_plan(prepared.into())
        .expect_err("manual resolution rejects open side-effect lane attempt");
    assert_projection_conflict_contains(error, "requires no open semantic attempts");
    let lane = store
        .projection_snapshot()
        .resource_lane(&lane_key)
        .expect("open lane remains held after rejected manual resolution");
    assert_eq!(lane.holder.ledger_key, open_ledger);
}

#[test]
fn saga_admit_token_must_match_run_start_policy_digest() {
    let run_id = run_id(220);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request_with_saga_policy(
            run_id.clone(),
            "saga-token-run-start",
            &proof_manual_saga_policy(),
        ))
        .expect("append run start");
    append_forward_confirmation(&mut store, &run_id);
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("saga-token-sidefx-failure").expect("commit key"),
            payloads: vec![side_effect_attempt_failed(false)],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append confirmed side-effect attempt failure");

    let verified = verified_manual_resolution_for_seq(store.expected_next_seq(&run_id).as_u64());
    let request = manual_resolution_request_from_verified(
        &verified,
        store.expected_next_seq(&run_id),
        "saga-token-mismatch",
        manual_saga_policy(171),
    );
    let artifacts = manual_resolution_artifacts_from_verified(&verified);
    let error = PreparedCommit::<ManualResolution>::new(
        request,
        CommitArtifactEvidenceSet::new(artifacts.clone(), artifacts)
            .expect("manual artifact evidence set"),
        &verified,
    )
    .expect_err("mismatched saga token rejects before admission");
    assert_invalid_prepared_commit_contains(error, "policy does not match saga admit token");
}

#[test]
fn manual_resolution_artifacts_require_dedicated_roles() {
    fn reject_with(
        commit_key: &str,
        mut mutate: impl FnMut(&mut Vec<ArtifactEvidenceRef>),
        expected_field: &'static str,
    ) {
        let run_id = run_id(220);
        let mut store = InMemoryTypedRunStore::new();
        store
            .append_prepared_commit(run_start_request_with_saga_policy(
                run_id.clone(),
                "manual-artifact-role-run-start",
                &proof_manual_saga_policy(),
            ))
            .expect("append run start");
        append_forward_confirmation(&mut store, &run_id);
        store
            .append_prepared_commit(typed_commit_request! {
                run_id: run_id.clone(),
                expected_next_seq: store.expected_next_seq(&run_id),
                commit_key: CommitKey::new(format!("{commit_key}-sidefx-failure"))
                    .expect("commit key"),
                payloads: vec![side_effect_attempt_failed(false)],
                required_artifacts: Vec::new(),
                preconditions: CommitPreconditions::default(),
            })
            .expect("append confirmed side-effect attempt failure");
        let verified =
            verified_manual_resolution_for_seq(store.expected_next_seq(&run_id).as_u64());
        let request = manual_resolution_request_from_verified(
            &verified,
            store.expected_next_seq(&run_id),
            commit_key,
            proof_manual_saga_policy(),
        );
        let required_artifacts = manual_resolution_artifacts_from_verified(&verified);
        let mut admitted_artifacts = required_artifacts.clone();
        mutate(&mut admitted_artifacts);
        let prepared = PreparedCommit::<ManualResolution>::new(
            request,
            CommitArtifactEvidenceSet::new(required_artifacts, admitted_artifacts)
                .expect("manual artifact evidence set"),
            &verified,
        )
        .expect("proof-backed manual resolution prepared commit");
        let error = store
            .append_prepared_commit_plan(prepared.into())
            .expect_err("manual artifact mismatch rejects");
        assert!(matches!(
            error,
            StoreError::ArtifactEvidenceMismatch { field, .. } if field == expected_field
        ));
    }

    reject_with(
        "manual-wrong-evidence-role",
        |artifacts| artifacts[0].artifact_role = ArtifactRole::StateOutput,
        "artifact_role",
    );
    reject_with(
        "manual-wrong-authorization-role",
        |artifacts| artifacts[1].artifact_role = ArtifactRole::StateOutput,
        "artifact_role",
    );
    reject_with(
        "manual-wrong-authorization-schema",
        |artifacts| artifacts[1].schema_id = Some(schema_id("mfm.test.wrong_authorization", 200)),
        "schema_id",
    );
    reject_with(
        "manual-wrong-evidence-digest",
        |artifacts| artifacts[0].digest = content_digest(202),
        "digest",
    );
}

#[test]
fn saga_run_completed_requires_terminal_proof() {
    let terminal_run = run_id(120);
    let mut store = InMemoryTypedRunStore::new();
    let policy = manual_saga_policy(160);
    store
        .append_prepared_commit(run_start_request_with_saga_policy(
            terminal_run.clone(),
            "terminal-quiescence-run-start",
            &policy,
        ))
        .expect("append run start");
    append_side_effect_prepare(&mut store, &terminal_run);
    append_side_effect_started(&mut store, &terminal_run);
    append_generic_nonretryable_failure(&mut store, &terminal_run, "terminal-quiescence");
    let saga = store
        .projection_snapshot()
        .derive_saga_projection(&terminal_run, &policy);
    let proof_error =
        SagaTerminalProof::new(&policy, &saga, store.expected_next_seq(&terminal_run), None)
            .expect_err("proof rejects before terminal saga mode");
    assert_projection_conflict_contains(proof_error, "requires terminal saga mode");
    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: terminal_run.clone(),
            expected_next_seq: store.expected_next_seq(&terminal_run),
            commit_key: CommitKey::new("terminal-non-quiescent").expect("commit key"),
            payloads: vec![run_completed(
                events::RunCompletionOutcome::FailedWithoutAcdcClaim,
            )],
            required_artifacts: Vec::new(),
            preconditions: saga_preconditions(&terminal_run, policy.clone()),
        })
        .expect_err("raw terminal completion rejects without proof");
    assert_invalid_prepared_commit_contains(error, "requires SagaTerminalProof");

    let mut forged = InMemoryTypedRunStore::new();
    let forged_run = run_id(220);
    let forged_policy = proof_manual_saga_policy();
    forged
        .append_prepared_commit(run_start_request_with_saga_policy(
            forged_run.clone(),
            "terminal-forged-run-start",
            &forged_policy,
        ))
        .expect("append run start");
    append_forward_confirmation(&mut forged, &forged_run);
    forged
        .append_prepared_commit(typed_commit_request! {
            run_id: forged_run.clone(),
            expected_next_seq: forged.expected_next_seq(&forged_run),
            commit_key: CommitKey::new("terminal-forged-sidefx-failure").expect("commit key"),
            payloads: vec![side_effect_attempt_failed(false)],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append confirmed side-effect attempt failure");
    let saga = forged
        .projection_snapshot()
        .derive_saga_projection(&forged_run, &forged_policy);
    let proof_error = SagaTerminalProof::new(
        &forged_policy,
        &saga,
        forged.expected_next_seq(&forged_run),
        None,
    )
    .expect_err("manual terminal rejects before manual resolution");
    assert_projection_conflict_contains(proof_error, "requires terminal saga mode");
    let error = forged
        .append_prepared_commit(typed_commit_request! {
            run_id: forged_run.clone(),
            expected_next_seq: forged.expected_next_seq(&forged_run),
            commit_key: CommitKey::new("terminal-forged-manual").expect("commit key"),
            payloads: vec![run_completed_for_run(
                forged_run.clone(),
                events::RunCompletionOutcome::ManuallyResolved,
            )],
            required_artifacts: Vec::new(),
            preconditions: saga_preconditions(&forged_run, forged_policy.clone()),
        })
        .expect_err("raw forged manual terminal rejects without proof");
    assert_invalid_prepared_commit_contains(error, "requires SagaTerminalProof");

    let verified =
        verified_manual_resolution_for_seq(forged.expected_next_seq(&forged_run).as_u64());
    let prepared = prepared_manual_resolution_commit(
        &verified,
        forged.expected_next_seq(&forged_run),
        "terminal-manual-recorded",
        forged_policy.clone(),
    );
    forged
        .append_prepared_commit_plan(prepared.into())
        .expect("manual resolution admitted");
    let saga = forged
        .projection_snapshot()
        .derive_saga_projection(&forged_run, &forged_policy);
    let proof_error = SagaTerminalProof::new(
        &forged_policy,
        &saga,
        forged.expected_next_seq(&forged_run),
        None,
    )
    .expect_err("manual terminal requires verified proof authority");
    assert_projection_conflict_contains(proof_error, "requires verified manual resolution proof");
}

#[test]
fn saga_terminal_prepared_commit_requires_matching_proof() {
    let run_id = run_id(122);
    let policy = SagaPolicySpec::FailWithoutAcdcClaim;
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request_with_saga_policy(
            run_id.clone(),
            "terminal-proof-run-start",
            &policy,
        ))
        .expect("append run start");
    append_generic_nonretryable_failure(&mut store, &run_id, "terminal-proof-failure");
    let saga = store
        .projection_snapshot()
        .derive_saga_projection(&run_id, &policy);
    let proof = SagaTerminalProof::new(&policy, &saga, store.expected_next_seq(&run_id), None)
        .expect("failed terminal proof authority");
    let request = typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(&run_id),
        commit_key: CommitKey::new("terminal-proof").expect("commit key"),
        payloads: vec![run_completed_for_run(
            run_id.clone(),
            events::RunCompletionOutcome::FailedWithoutAcdcClaim,
        )],
        required_artifacts: Vec::new(),
        preconditions: saga_preconditions(&run_id, policy),
    };
    let prepared =
        PreparedCommit::<SagaTerminal>::new(request, CommitArtifactEvidenceSet::empty(), &proof)
            .expect("proof-backed saga terminal commit");
    store
        .append_prepared_commit_plan(prepared.into())
        .expect("append proof-backed saga terminal");
    assert!(matches!(
        store
            .projection_snapshot()
            .run_completion(&run_id)
            .expect("run completion")
            .outcome,
        events::RunCompletionOutcome::FailedWithoutAcdcClaim
    ));
}

#[test]
fn saga_terminal_prepared_commit_rejects_cross_run_proof() {
    let proof_run = run_id(123);
    let request_run = run_id(124);
    let policy = SagaPolicySpec::FailWithoutAcdcClaim;
    let mut proof_store = InMemoryTypedRunStore::new();
    proof_store
        .append_prepared_commit(run_start_request_with_saga_policy(
            proof_run.clone(),
            "terminal-cross-run-proof-start",
            &policy,
        ))
        .expect("append proof run start");
    append_generic_nonretryable_failure(
        &mut proof_store,
        &proof_run,
        "terminal-cross-run-proof-failure",
    );
    let proof_saga = proof_store
        .projection_snapshot()
        .derive_saga_projection(&proof_run, &policy);
    let proof = SagaTerminalProof::new(
        &policy,
        &proof_saga,
        proof_store.expected_next_seq(&proof_run),
        None,
    )
    .expect("terminal proof authority");

    let request = typed_commit_request! {
        run_id: request_run.clone(),
        expected_next_seq: StreamSeq::new(1).expect("stream seq"),
        commit_key: CommitKey::new("terminal-cross-run-request").expect("commit key"),
        payloads: vec![run_completed_for_run(
            request_run.clone(),
            events::RunCompletionOutcome::FailedWithoutAcdcClaim,
        )],
        required_artifacts: Vec::new(),
        preconditions: saga_preconditions(&request_run, policy),
    };
    let error =
        PreparedCommit::<SagaTerminal>::new(request, CommitArtifactEvidenceSet::empty(), &proof)
            .expect_err("cross-run proof rejects");
    assert_invalid_prepared_commit_contains(error, "run id does not match");
}

#[test]
fn saga_terminal_prepared_commit_rejects_policy_digest_mismatch() {
    let run_id = run_id(125);
    let policy = SagaPolicySpec::FailWithoutAcdcClaim;
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request_with_saga_policy(
            run_id.clone(),
            "terminal-policy-proof-start",
            &policy,
        ))
        .expect("append run start");
    append_generic_nonretryable_failure(&mut store, &run_id, "terminal-policy-proof-failure");
    let saga = store
        .projection_snapshot()
        .derive_saga_projection(&run_id, &policy);
    let proof = SagaTerminalProof::new(&policy, &saga, store.expected_next_seq(&run_id), None)
        .expect("terminal proof authority");

    let request = typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(&run_id),
        commit_key: CommitKey::new("terminal-policy-mismatch").expect("commit key"),
        payloads: vec![run_completed_for_run(
            run_id.clone(),
            events::RunCompletionOutcome::FailedWithoutAcdcClaim,
        )],
        required_artifacts: Vec::new(),
        preconditions: saga_preconditions(&run_id, compensate_saga_policy()),
    };
    let error =
        PreparedCommit::<SagaTerminal>::new(request, CommitArtifactEvidenceSet::empty(), &proof)
            .expect_err("policy digest mismatch rejects");
    assert_invalid_prepared_commit_contains(error, "policy digest");
}

#[test]
fn saga_terminal_prepared_commit_rejects_stale_prefix_proof() {
    let run_id = run_id(126);
    let policy = SagaPolicySpec::FailWithoutAcdcClaim;
    let mut proof_store = InMemoryTypedRunStore::new();
    proof_store
        .append_prepared_commit(run_start_request_with_saga_policy(
            run_id.clone(),
            "terminal-current-proof-start",
            &policy,
        ))
        .expect("append proof run start");
    append_generic_nonretryable_failure(
        &mut proof_store,
        &run_id,
        "terminal-current-proof-failure",
    );
    let proof_saga = proof_store
        .projection_snapshot()
        .derive_saga_projection(&run_id, &policy);
    let proof = SagaTerminalProof::new(
        &policy,
        &proof_saga,
        proof_store.expected_next_seq(&run_id),
        None,
    )
    .expect("terminal proof authority");

    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request_with_saga_policy(
            run_id.clone(),
            "terminal-current-run-start",
            &policy,
        ))
        .expect("append current run start");
    let request = typed_commit_request! {
        run_id: run_id.clone(),
        expected_next_seq: store.expected_next_seq(&run_id),
        commit_key: CommitKey::new("terminal-current-nonterminal").expect("commit key"),
        payloads: vec![run_completed_for_run(
            run_id.clone(),
            events::RunCompletionOutcome::FailedWithoutAcdcClaim,
        )],
        required_artifacts: Vec::new(),
        preconditions: saga_preconditions(&run_id, policy),
    };
    let error =
        PreparedCommit::<SagaTerminal>::new(request, CommitArtifactEvidenceSet::empty(), &proof)
            .expect_err("stale proof rejects");
    assert_invalid_prepared_commit_contains(error, "prefix");
}

#[test]
fn saga_projection_derives_obligations_and_run_mode_from_policy_and_stream() {
    let run_id = run_id(120);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_prepared_commit(run_start_request(
            run_id.clone(),
            "saga-projection-run-start",
        ))
        .expect("append run start");
    append_forward_confirmation(&mut store, &run_id);
    append_generic_nonretryable_failure(&mut store, &run_id, "saga-projection-engagement");

    let policy = SagaPolicySpec::CompensateCompleted {
        on_remediation_unresolved: RemediationUnresolvedSpec::FailWithoutAcdcClaim,
    };
    let projection = store
        .projection_snapshot()
        .derive_saga_projection(&run_id, &policy);
    assert_eq!(projection.run_mode, RunMode::Remediating);
    assert!(projection.engagement.is_some());
    assert!(projection.forward_quiescent);
    let obligation = projection
        .obligations
        .get(&side_effect_ledger_key())
        .expect("forward obligation");
    assert_eq!(obligation.classification, ForwardLedgerClassification::Owed);
    assert!(obligation.remediation.is_none());

    let manual_policy = SagaPolicySpec::ManualResolution {
        manual: ManualResolutionEvidenceSpec {
            evidence_schema: schema_id("mfm.test.manual_evidence", 180),
            authorization: manual_authorization(181),
        },
    };
    let manual_projection = store
        .projection_snapshot()
        .derive_saga_projection(&run_id, &manual_policy);
    assert_eq!(manual_projection.run_mode, RunMode::ManualBlocked);
    assert_eq!(
        manual_projection.manual_block_reason,
        Some(ManualBlockReason::PolicyManualResolution)
    );

    let remediation_key = remediation_ledger_key(10);
    append_remediation_confirmation(&mut store, &run_id, remediation_key.clone());
    let projection = store
        .projection_snapshot()
        .derive_saga_projection(&run_id, &policy);
    assert_eq!(projection.run_mode, RunMode::Compensated);
    let obligation = projection
        .obligations
        .get(&side_effect_ledger_key())
        .expect("forward obligation");
    let remediation = obligation.remediation.as_ref().expect("remediation");
    assert_eq!(remediation.ledger_key, remediation_key);
    assert!(remediation.closed);
    assert_eq!(remediation.unresolved, None);
}

#[test]
fn side_effect_ledger_state_exposes_valid_prepared_view() {
    let run_id = run_id(120);
    let mut store = InMemoryTypedRunStore::new();
    append_side_effect_prepare(&mut store, &run_id);

    let projection = store
        .projection_snapshot()
        .side_effect_for_run(&run_id, &side_effect_ledger_key())
        .expect("side-effect projection");
    let state = projection.ledger_state().expect("typed ledger state");
    assert!(state.is_forward_completion_candidate());
    let SideEffectLedgerPhase::Prepared {
        claim,
        prepared_invocation,
        resource_key,
    } = state.phase()
    else {
        panic!("expected prepared state");
    };
    assert_eq!(claim.claim_generation, 1);
    assert!(prepared_invocation.is_none());
    assert!(resource_key.is_none());
}

#[test]
fn side_effect_ledger_state_rejects_claim_phase_without_active_claim() {
    let run_id = run_id(120);
    let mut store = InMemoryTypedRunStore::new();
    append_side_effect_prepare(&mut store, &run_id);
    let mut projection = store
        .projection_snapshot()
        .side_effect_for_run(&run_id, &side_effect_ledger_key())
        .expect("side-effect projection")
        .clone();
    projection.claim = None;

    let error = projection
        .ledger_state()
        .expect_err("malformed claim projection rejects");
    assert_projection_conflict_contains(error, "phase requires active claim");
}

#[test]
fn side_effect_ledger_state_rejects_confirmed_phase_without_confirmation_evidence() {
    let run_id = run_id(120);
    let mut store = InMemoryTypedRunStore::new();
    append_forward_confirmation(&mut store, &run_id);
    let mut projection = store
        .projection_snapshot()
        .side_effect_for_run(&run_id, &side_effect_ledger_key())
        .expect("side-effect projection")
        .clone();
    projection.confirmation = None;

    let error = projection
        .ledger_state()
        .expect_err("malformed confirmation projection rejects");
    assert_projection_conflict_contains(error, "confirmation evidence is missing");
}

#[test]
fn side_effect_ledger_state_classifies_ambiguity_as_terminal_not_frontier() {
    let run_id = run_id(120);
    let mut store = InMemoryTypedRunStore::new();
    append_side_effect_prepare(&mut store, &run_id);
    append_side_effect_started(&mut store, &run_id);
    let ambiguity_artifact = artifact_id(121);
    let ambiguity_digest = content_digest(122);
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-ambiguity-typestate").expect("commit key"),
            payloads: vec![
                side_effect_ambiguous(ambiguity_artifact.clone(), ambiguity_digest.clone()),
                side_effect_attempt_failed(false),
            ],
            required_artifacts: vec![side_effect_evidence(
                ambiguity_artifact,
                ambiguity_digest,
                schema_id("mfm.test.ambiguity", 76),
                ArtifactRole::AmbiguityEvidence,
            )],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append ambiguity terminal pair");

    let projection = store
        .projection_snapshot()
        .side_effect_for_run(&run_id, &side_effect_ledger_key())
        .expect("side-effect projection");
    let state = projection.ledger_state().expect("typed ledger state");
    assert!(!state.is_forward_completion_candidate());
    assert!(matches!(
        state.phase(),
        SideEffectLedgerPhase::Ambiguous { .. }
    ));
}

#[test]
fn side_effect_phase_order_and_fencing_are_enforced() {
    let mut stale_owner_store = InMemoryTypedRunStore::new();
    let stale_owner_run = run_id(100);
    append_side_effect_prepare(&mut stale_owner_store, &stale_owner_run);
    let stale_owner = stale_owner_store
        .append_prepared_commit(typed_commit_request! {
            run_id: stale_owner_run.clone(),
            expected_next_seq: stale_owner_store.expected_next_seq(&stale_owner_run),
            commit_key: CommitKey::new("sidefx-started-stale-owner").expect("commit key"),
            payloads: vec![side_effect_started("owner-2", 1, "token-1")],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("stale owner rejects");
    assert!(matches!(stale_owner, StoreError::ProjectionConflict { .. }));

    let mut stale_token_store = InMemoryTypedRunStore::new();
    let stale_token_run = run_id(101);
    append_side_effect_prepare(&mut stale_token_store, &stale_token_run);
    let stale_token = stale_token_store
        .append_prepared_commit(typed_commit_request! {
            run_id: stale_token_run.clone(),
            expected_next_seq: stale_token_store.expected_next_seq(&stale_token_run),
            commit_key: CommitKey::new("sidefx-started-stale-token").expect("commit key"),
            payloads: vec![side_effect_started("owner-1", 1, "token-2")],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("stale token rejects");
    assert!(matches!(stale_token, StoreError::ProjectionConflict { .. }));

    let mut takeover_store = InMemoryTypedRunStore::new();
    let takeover_run = run_id(102);
    append_side_effect_prepare(&mut takeover_store, &takeover_run);
    let reused_token_takeover = takeover_store
        .append_prepared_commit(typed_commit_request! {
            run_id: takeover_run.clone(),
            expected_next_seq: takeover_store.expected_next_seq(&takeover_run),
            commit_key: CommitKey::new("sidefx-takeover-reused-token").expect("commit key"),
            payloads: vec![side_effect_claim_taken_over_with_token("token-1")],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("takeover reusing fencing token rejects");
    assert!(matches!(
        reused_token_takeover,
        StoreError::ProjectionConflict { .. }
    ));
    takeover_store
        .append_prepared_commit(typed_commit_request! {
            run_id: takeover_run.clone(),
            expected_next_seq: takeover_store.expected_next_seq(&takeover_run),
            commit_key: CommitKey::new("sidefx-takeover").expect("commit key"),
            payloads: vec![side_effect_claim_taken_over()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("takeover before invocation started succeeds");
    takeover_store
        .append_prepared_commit(typed_commit_request! {
            run_id: takeover_run.clone(),
            expected_next_seq: takeover_store.expected_next_seq(&takeover_run),
            commit_key: CommitKey::new("sidefx-prepared-after-takeover").expect("commit key"),
            payloads: vec![side_effect_prepared(2, "token-2")],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("prepare after takeover succeeds");
    takeover_store
        .append_prepared_commit(typed_commit_request! {
            run_id: takeover_run.clone(),
            expected_next_seq: takeover_store.expected_next_seq(&takeover_run),
            commit_key: CommitKey::new("sidefx-started-after-takeover").expect("commit key"),
            payloads: vec![side_effect_started("owner-2", 2, "token-2")],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("start after takeover succeeds");
    let late_takeover = takeover_store
        .append_prepared_commit(typed_commit_request! {
            run_id: takeover_run.clone(),
            expected_next_seq: takeover_store.expected_next_seq(&takeover_run),
            commit_key: CommitKey::new("sidefx-late-takeover").expect("commit key"),
            payloads: vec![KernelEventPayload::SideEffectClaimTakenOver(
                side_effect::ClaimTakenOver {
                    spec_hash: spec_hash(1),
                    node_id: node_id(70),
                    attempt_id: attempt_id(72),
                    ledger_key: side_effect_ledger_key(),
                    ledger_purpose: side_effect_ledger_purpose(),
                    previous_claim_owner: events::RunnerInvocationId::new("owner-2")
                        .expect("previous owner"),
                    new_claim_owner: events::RunnerInvocationId::new("owner-3").expect("new owner"),
                    invocation_epoch: 1,
                    previous_claim_generation: 2,
                    claim_generation: 3,
                    claim_fencing_token: side_effect::ClaimFencingToken::new("token-3")
                        .expect("token"),
                },
            )],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("takeover after invocation started rejects");
    assert!(matches!(
        late_takeover,
        StoreError::ProjectionConflict { .. }
    ));

    let mut receipt_store = InMemoryTypedRunStore::new();
    let receipt_run = run_id(103);
    append_side_effect_prepare(&mut receipt_store, &receipt_run);
    append_side_effect_started(&mut receipt_store, &receipt_run);
    let receipt_artifact = artifact_id(103);
    let receipt_digest = content_digest(104);
    let receipt_evidence = side_effect_evidence(
        receipt_artifact.clone(),
        receipt_digest.clone(),
        receipt_schema(),
        ArtifactRole::Receipt,
    );
    let receipt_without_submission = receipt_store
        .append_prepared_commit(typed_commit_request! {
            run_id: receipt_run.clone(),
            expected_next_seq: receipt_store.expected_next_seq(&receipt_run),
            commit_key: CommitKey::new("sidefx-receipt-without-submission").expect("commit key"),
            payloads: vec![side_effect_receipt(receipt_artifact, receipt_digest)],
            required_artifacts: vec![receipt_evidence],
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("receipt without submission rejects");
    assert!(matches!(
        receipt_without_submission,
        StoreError::ProjectionConflict { .. }
    ));

    let mut confirmation_store = InMemoryTypedRunStore::new();
    let confirmation_run = run_id(104);
    append_side_effect_prepare(&mut confirmation_store, &confirmation_run);
    append_side_effect_started(&mut confirmation_store, &confirmation_run);
    let submission_artifact = artifact_id(105);
    let submission_digest = content_digest(106);
    let submission_evidence = side_effect_evidence(
        submission_artifact.clone(),
        submission_digest.clone(),
        submission_schema(),
        ArtifactRole::Submission,
    );
    confirmation_store
        .append_prepared_commit(typed_commit_request! {
            run_id: confirmation_run.clone(),
            expected_next_seq: confirmation_store.expected_next_seq(&confirmation_run),
            commit_key: CommitKey::new("sidefx-submission").expect("commit key"),
            payloads: vec![side_effect_submission_observed(
                submission_artifact,
                submission_digest,
            )],
            required_artifacts: vec![submission_evidence],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append submission");
    let confirmation_artifact = artifact_id(107);
    let confirmation_digest = content_digest(108);
    let confirmation_evidence = side_effect_evidence(
        confirmation_artifact.clone(),
        confirmation_digest.clone(),
        confirmation_schema(),
        ArtifactRole::Confirmation,
    );
    let confirmation_without_receipt = confirmation_store
        .append_prepared_commit(typed_commit_request! {
            run_id: confirmation_run.clone(),
            expected_next_seq: confirmation_store.expected_next_seq(&confirmation_run),
            commit_key: CommitKey::new("sidefx-confirmation-without-receipt").expect("commit key"),
            payloads: vec![side_effect_confirmation(
                confirmation_artifact,
                confirmation_digest,
            )],
            required_artifacts: vec![confirmation_evidence],
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("confirmation without receipt rejects");
    assert!(matches!(
        confirmation_without_receipt,
        StoreError::ProjectionConflict { .. }
    ));

    let unknown_artifact = artifact_id(109);
    let unknown_digest = content_digest(110);
    let unknown_evidence = side_effect_evidence(
        unknown_artifact.clone(),
        unknown_digest.clone(),
        unknown_schema(),
        ArtifactRole::SubmissionUnknownEvidence,
    );
    let duplicate_submit = confirmation_store
        .append_prepared_commit(typed_commit_request! {
            run_id: confirmation_run.clone(),
            expected_next_seq: confirmation_store.expected_next_seq(&confirmation_run),
            commit_key: CommitKey::new("sidefx-duplicate-submit").expect("commit key"),
            payloads: vec![side_effect_submission_unknown(
                unknown_artifact,
                unknown_digest,
            )],
            required_artifacts: vec![unknown_evidence],
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("duplicate submit rejects");
    assert!(matches!(
        duplicate_submit,
        StoreError::DuplicateLogicalKey { .. }
            | StoreError::LogicalKeyConflict { .. }
            | StoreError::ProjectionConflict { .. }
    ));

    let mut failure_store = InMemoryTypedRunStore::new();
    let failure_run = run_id(105);
    append_side_effect_prepare(&mut failure_store, &failure_run);
    let mismatched_retryability = failure_store
        .append_prepared_commit(typed_commit_request! {
            run_id: failure_run.clone(),
            expected_next_seq: failure_store.expected_next_seq(&failure_run),
            commit_key: CommitKey::new("sidefx-failure-mismatched-retryability")
                .expect("commit key"),
            payloads: vec![side_effect_failed(false), side_effect_attempt_failed(true)],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("mismatched retryability rejects");
    assert!(matches!(
        mismatched_retryability,
        StoreError::ProjectionConflict { .. }
    ));

    let mut failed_without_attempt_store = InMemoryTypedRunStore::new();
    let failed_without_attempt_run = run_id(106);
    append_side_effect_prepare(
        &mut failed_without_attempt_store,
        &failed_without_attempt_run,
    );
    let failed_without_attempt = failed_without_attempt_store
        .append_prepared_commit(typed_commit_request! {
            run_id: failed_without_attempt_run.clone(),
            expected_next_seq: failed_without_attempt_store
                .expected_next_seq(&failed_without_attempt_run),
            commit_key: CommitKey::new("sidefx-failed-without-attempt").expect("commit key"),
            payloads: vec![side_effect_failed(false)],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("side-effect failure without attempt failure rejects");
    assert_projection_conflict_contains(
        failed_without_attempt,
        "side-effect failure requires matching StateAttemptFailed",
    );

    let mut failed_with_attempt_store = InMemoryTypedRunStore::new();
    let failed_with_attempt_run = run_id(107);
    append_side_effect_prepare(&mut failed_with_attempt_store, &failed_with_attempt_run);
    failed_with_attempt_store
        .append_prepared_commit(typed_commit_request! {
            run_id: failed_with_attempt_run.clone(),
            expected_next_seq: failed_with_attempt_store
                .expected_next_seq(&failed_with_attempt_run),
            commit_key: CommitKey::new("sidefx-failed-with-attempt").expect("commit key"),
            payloads: vec![side_effect_failed(false), side_effect_attempt_failed(false)],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("side-effect failure with matching attempt failure accepts");

    let mut failed_mismatch_store = InMemoryTypedRunStore::new();
    let failed_mismatch_run = run_id(108);
    append_side_effect_prepare(&mut failed_mismatch_store, &failed_mismatch_run);
    let mut mismatched_attempt = side_effect_attempt_failed(false);
    set_attempt_failure_node_attempt(&mut mismatched_attempt, node_id(170), attempt_id(172));
    let failed_mismatched_attempt = failed_mismatch_store
        .append_prepared_commit(typed_commit_request! {
            run_id: failed_mismatch_run.clone(),
            expected_next_seq: failed_mismatch_store.expected_next_seq(&failed_mismatch_run),
            commit_key: CommitKey::new("sidefx-failed-mismatched-attempt").expect("commit key"),
            payloads: vec![side_effect_failed(false), mismatched_attempt],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("side-effect failure with mismatched attempt rejects");
    assert_projection_conflict_contains(
        failed_mismatched_attempt,
        "side-effect failure requires matching StateAttemptFailed",
    );

    let mut ambiguity_without_attempt_store = InMemoryTypedRunStore::new();
    let ambiguity_without_attempt_run = run_id(109);
    append_side_effect_prepare(
        &mut ambiguity_without_attempt_store,
        &ambiguity_without_attempt_run,
    );
    append_side_effect_started(
        &mut ambiguity_without_attempt_store,
        &ambiguity_without_attempt_run,
    );
    let ambiguity_artifact = artifact_id(113);
    let ambiguity_digest = content_digest(114);
    let ambiguity_evidence = side_effect_evidence(
        ambiguity_artifact.clone(),
        ambiguity_digest.clone(),
        schema_id("mfm.test.ambiguity", 76),
        ArtifactRole::AmbiguityEvidence,
    );
    let ambiguity_without_attempt = ambiguity_without_attempt_store
        .append_prepared_commit(typed_commit_request! {
            run_id: ambiguity_without_attempt_run.clone(),
            expected_next_seq: ambiguity_without_attempt_store
                .expected_next_seq(&ambiguity_without_attempt_run),
            commit_key: CommitKey::new("sidefx-ambiguity-without-attempt").expect("commit key"),
            payloads: vec![side_effect_ambiguous(ambiguity_artifact, ambiguity_digest)],
            required_artifacts: vec![ambiguity_evidence],
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("side-effect ambiguity without attempt failure rejects");
    assert_projection_conflict_contains(
        ambiguity_without_attempt,
        "side-effect ambiguity requires matching StateAttemptFailed",
    );

    let mut ambiguity_with_attempt_store = InMemoryTypedRunStore::new();
    let ambiguity_with_attempt_run = run_id(110);
    append_side_effect_prepare(
        &mut ambiguity_with_attempt_store,
        &ambiguity_with_attempt_run,
    );
    append_side_effect_started(
        &mut ambiguity_with_attempt_store,
        &ambiguity_with_attempt_run,
    );
    let ambiguity_artifact = artifact_id(115);
    let ambiguity_digest = content_digest(116);
    let ambiguity_evidence = side_effect_evidence(
        ambiguity_artifact.clone(),
        ambiguity_digest.clone(),
        schema_id("mfm.test.ambiguity", 76),
        ArtifactRole::AmbiguityEvidence,
    );
    ambiguity_with_attempt_store
        .append_prepared_commit(typed_commit_request! {
            run_id: ambiguity_with_attempt_run.clone(),
            expected_next_seq: ambiguity_with_attempt_store
                .expected_next_seq(&ambiguity_with_attempt_run),
            commit_key: CommitKey::new("sidefx-ambiguity-with-attempt").expect("commit key"),
            payloads: vec![
                side_effect_ambiguous(ambiguity_artifact, ambiguity_digest),
                side_effect_attempt_failed(false),
            ],
            required_artifacts: vec![ambiguity_evidence],
            preconditions: CommitPreconditions::default(),
        })
        .expect("side-effect ambiguity with matching non-retryable attempt failure accepts");

    let mut ambiguity_retryable_attempt_store = InMemoryTypedRunStore::new();
    let ambiguity_retryable_attempt_run = run_id(111);
    append_side_effect_prepare(
        &mut ambiguity_retryable_attempt_store,
        &ambiguity_retryable_attempt_run,
    );
    append_side_effect_started(
        &mut ambiguity_retryable_attempt_store,
        &ambiguity_retryable_attempt_run,
    );
    let ambiguity_artifact = artifact_id(117);
    let ambiguity_digest = content_digest(118);
    let ambiguity_evidence = side_effect_evidence(
        ambiguity_artifact.clone(),
        ambiguity_digest.clone(),
        schema_id("mfm.test.ambiguity", 76),
        ArtifactRole::AmbiguityEvidence,
    );
    let retryable_attempt = ambiguity_retryable_attempt_store
        .append_prepared_commit(typed_commit_request! {
            run_id: ambiguity_retryable_attempt_run.clone(),
            expected_next_seq: ambiguity_retryable_attempt_store
                .expected_next_seq(&ambiguity_retryable_attempt_run),
            commit_key: CommitKey::new("sidefx-ambiguity-retryable-attempt").expect("commit key"),
            payloads: vec![
                side_effect_ambiguous(ambiguity_artifact, ambiguity_digest),
                side_effect_attempt_failed(true),
            ],
            required_artifacts: vec![ambiguity_evidence],
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("side-effect ambiguity with retryable attempt failure rejects");
    assert_projection_conflict_contains(
        retryable_attempt,
        "side-effect ambiguity retryability must match attempt failure",
    );

    let mut ambiguity_mismatch_store = InMemoryTypedRunStore::new();
    let ambiguity_mismatch_run = run_id(112);
    append_side_effect_prepare(&mut ambiguity_mismatch_store, &ambiguity_mismatch_run);
    append_side_effect_started(&mut ambiguity_mismatch_store, &ambiguity_mismatch_run);
    let ambiguity_artifact = artifact_id(119);
    let ambiguity_digest = content_digest(120);
    let ambiguity_evidence = side_effect_evidence(
        ambiguity_artifact.clone(),
        ambiguity_digest.clone(),
        schema_id("mfm.test.ambiguity", 76),
        ArtifactRole::AmbiguityEvidence,
    );
    let mut mismatched_attempt = side_effect_attempt_failed(false);
    set_attempt_failure_node_attempt(&mut mismatched_attempt, node_id(230), attempt_id(232));
    let ambiguity_mismatched_attempt = ambiguity_mismatch_store
        .append_prepared_commit(typed_commit_request! {
            run_id: ambiguity_mismatch_run.clone(),
            expected_next_seq: ambiguity_mismatch_store.expected_next_seq(&ambiguity_mismatch_run),
            commit_key: CommitKey::new("sidefx-ambiguity-mismatched-attempt").expect("commit key"),
            payloads: vec![
                side_effect_ambiguous(ambiguity_artifact, ambiguity_digest),
                mismatched_attempt,
            ],
            required_artifacts: vec![ambiguity_evidence],
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("side-effect ambiguity with mismatched attempt rejects");
    assert_projection_conflict_contains(
        ambiguity_mismatched_attempt,
        "side-effect ambiguity requires matching StateAttemptFailed",
    );
}

#[test]
fn side_effect_submission_unknown_recovery_uses_one_submission_result_key() {
    let mut store = InMemoryTypedRunStore::new();
    let run_id = run_id(113);
    append_side_effect_prepare(&mut store, &run_id);
    append_side_effect_started(&mut store, &run_id);

    let unknown_artifact = artifact_id(111);
    let unknown_digest = content_digest(112);
    let unknown_evidence = side_effect_evidence(
        unknown_artifact.clone(),
        unknown_digest.clone(),
        unknown_schema(),
        ArtifactRole::SubmissionUnknownEvidence,
    );
    let unknown = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-submission-unknown").expect("commit key"),
            payloads: vec![side_effect_submission_unknown(
                unknown_artifact,
                unknown_digest,
            )],
            required_artifacts: vec![unknown_evidence],
            preconditions: CommitPreconditions::default(),
        })
        .expect("append submission unknown")
        .batch()
        .clone();
    let submission_result_key = format!(
        "sidefx:forward:{}:invocation:1:submission_result",
        side_effect_ledger_key()
    );
    assert_eq!(
        unknown.events()[0].logical_key().as_str(),
        submission_result_key
    );

    let refreshed_unknown_artifact = artifact_id(117);
    let refreshed_unknown_digest = content_digest(118);
    let refreshed_unknown_evidence = side_effect_evidence(
        refreshed_unknown_artifact.clone(),
        refreshed_unknown_digest.clone(),
        unknown_schema(),
        ArtifactRole::SubmissionUnknownEvidence,
    );
    let refreshed_unknown = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-submission-unknown-refresh").expect("commit key"),
            payloads: vec![side_effect_submission_unknown(
                refreshed_unknown_artifact,
                refreshed_unknown_digest,
            )],
            required_artifacts: vec![refreshed_unknown_evidence],
            preconditions: CommitPreconditions::default(),
        })
        .expect("refresh submission unknown")
        .batch()
        .clone();
    assert_eq!(
        refreshed_unknown.events()[0].logical_key().as_str(),
        submission_result_key
    );
    let projection = store
        .projection_snapshot()
        .side_effect(&side_effect_ledger_key())
        .expect("side-effect projection");
    assert!(matches!(
        projection.phase,
        SideEffectPhase::SubmissionUnknown {
            invocation_epoch: 1
        }
    ));

    let submission_artifact = artifact_id(113);
    let submission_digest = content_digest(114);
    let submission_evidence = side_effect_evidence(
        submission_artifact.clone(),
        submission_digest.clone(),
        submission_schema(),
        ArtifactRole::Submission,
    );
    let observed = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-submission-observed-after-unknown")
                .expect("commit key"),
            payloads: vec![side_effect_submission_observed(
                submission_artifact,
                submission_digest,
            )],
            required_artifacts: vec![submission_evidence],
            preconditions: CommitPreconditions::default(),
        })
        .expect("recover observed submission")
        .batch()
        .clone();
    assert_eq!(
        observed.events()[0].logical_key().as_str(),
        submission_result_key
    );
    let projection = store
        .projection_snapshot()
        .side_effect(&side_effect_ledger_key())
        .expect("side-effect projection");
    assert!(matches!(
        projection.phase,
        SideEffectPhase::SubmissionObserved {
            invocation_epoch: 1
        }
    ));

    let duplicate_artifact = artifact_id(115);
    let duplicate_digest = content_digest(116);
    let duplicate_evidence = side_effect_evidence(
        duplicate_artifact.clone(),
        duplicate_digest.clone(),
        submission_schema(),
        ArtifactRole::Submission,
    );
    let duplicate = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-duplicate-observed-after-recovery")
                .expect("commit key"),
            payloads: vec![side_effect_submission_observed(
                duplicate_artifact,
                duplicate_digest,
            )],
            required_artifacts: vec![duplicate_evidence],
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("duplicate observed submission rejects after recovery");
    assert!(matches!(
        duplicate,
        StoreError::LogicalKeyConflict { .. } | StoreError::ProjectionConflict { .. }
    ));
}

#[test]
fn fact_recorded_projects_reusable_fact_evidence() {
    let run_id = run_id(95);
    let artifact_id = artifact_id(96);
    let artifact_digest = content_digest(97);
    let mut store = InMemoryTypedRunStore::new();
    let fact_evidence = fact_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fact-attempt-start").expect("commit key"),
            payloads: vec![fact_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::Started,
                ..CommitPreconditions::default()
            },
        })
        .expect("append fact attempt start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fact-recorded").expect("commit key"),
            payloads: vec![fact_recorded(artifact_id.clone(), artifact_digest.clone())],
            required_artifacts: vec![fact_evidence],
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::Started,
                ..CommitPreconditions::default()
            },
        })
        .expect("append fact");

    let fact_key = events::FactKey::new("fact-key-1").expect("fact key");
    let projection = store
        .projection_snapshot()
        .fact(&node_id(90), &attempt_id(91), &fact_key)
        .expect("fact projection");
    assert_eq!(projection.artifact_id, artifact_id);
    assert_eq!(projection.response_hash, artifact_digest);
}

#[test]
fn fact_recorded_requires_started_attempt_projection() {
    let run_id = run_id(96);
    let artifact_id = artifact_id(97);
    let artifact_digest = content_digest(98);
    let mut store = InMemoryTypedRunStore::new();
    let fact_evidence = fact_artifact_ref(artifact_id.clone(), artifact_digest.clone());
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");

    let error = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fact-before-attempt").expect("commit key"),
            payloads: vec![fact_recorded(artifact_id, artifact_digest)],
            required_artifacts: vec![fact_evidence],
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::Started,
                ..CommitPreconditions::default()
            },
        })
        .expect_err("fact before attempt must reject");
    assert!(matches!(error, StoreError::ProjectionConflict { .. }));
}

#[test]
fn retention_refs_are_projected_from_authoritative_stream() {
    let run_id = run_id(120);
    let artifact_id = artifact_id(121);
    let digest = content_digest(122);
    let mut store = InMemoryTypedRunStore::new();
    let evidence = store_artifact_ref(artifact_id.clone(), digest.clone());
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("retention-refs").expect("commit key"),
            payloads: vec![retention_refs_appended(
                artifact_id.clone(),
                digest.clone(),
                ArtifactRole::StateOutput,
            )],
            required_artifacts: vec![evidence.clone()],
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::Started,
                ..CommitPreconditions::default()
            },
        })
        .expect("append retention refs");

    let stream = store.load_run_stream(&run_id);
    let verified = VerifiedRetentionProjection::from_synthetic_run_stream(run_id.clone(), &stream)
        .expect("verified retention");
    assert!(verified.retains_artifact(&evidence));
    assert_eq!(
        verified
            .projection()
            .refs
            .get(&artifact_id)
            .expect("retention ref")
            .content_digest,
        digest
    );
}

#[test]
fn retention_refs_validate_role_contract_shape_without_repeating_exact_fields() {
    fn reject_state_output_shape(
        commit_key: &str,
        mut mutate: impl FnMut(&mut ArtifactEvidenceRef),
        expected_field: &'static str,
    ) {
        let run_id = run_id(120);
        let artifact_id = artifact_id(222);
        let digest = content_digest(223);
        let mut evidence = store_artifact_ref(artifact_id.clone(), digest.clone());
        mutate(&mut evidence);
        let mut store = InMemoryTypedRunStore::new();
        store
            .append_prepared_commit(run_start_request(
                run_id.clone(),
                &format!("{commit_key}-run-start"),
            ))
            .expect("append run start");

        let error = store
            .append_prepared_commit(typed_commit_request! {
                run_id: run_id.clone(),
                expected_next_seq: store.expected_next_seq(&run_id),
                commit_key: CommitKey::new(commit_key).expect("commit key"),
                payloads: vec![retention_refs_appended(
                    artifact_id,
                    digest,
                    ArtifactRole::StateOutput,
                )],
                required_artifacts: vec![evidence],
                preconditions: CommitPreconditions {
                    required_run_state: RequiredRunState::Started,
                    ..CommitPreconditions::default()
                },
            })
            .expect_err("invalid retention evidence shape rejects");
        assert_invalid_prepared_commit_contains(error, expected_field);
    }

    reject_state_output_shape(
        "retention-state-output-missing-schema",
        |evidence| evidence.schema_id = None,
        "schema_id",
    );
    reject_state_output_shape(
        "retention-state-output-missing-semantic",
        |evidence| evidence.semantic_type_id = None,
        "semantic_type_id",
    );
    reject_state_output_shape(
        "retention-state-output-missing-node",
        |evidence| evidence.producer_node_id = None,
        "producer_node_id",
    );

    fn reject_manifest_shape(
        commit_key: &str,
        mut mutate: impl FnMut(&mut ArtifactEvidenceRef),
        expected_field: &'static str,
    ) {
        let run_id = run_id(120);
        let artifact_id = artifact_id(224);
        let digest = content_digest(225);
        let mut evidence = retention_manifest_artifact_ref(artifact_id.clone(), digest.clone());
        mutate(&mut evidence);
        let mut store = InMemoryTypedRunStore::new();
        store
            .append_prepared_commit(run_start_request(
                run_id.clone(),
                &format!("{commit_key}-run-start"),
            ))
            .expect("append run start");

        let error = store
            .append_prepared_commit(typed_commit_request! {
                run_id: run_id.clone(),
                expected_next_seq: store.expected_next_seq(&run_id),
                commit_key: CommitKey::new(commit_key).expect("commit key"),
                payloads: retention_manifest_commit_payloads(
                    1,
                    digest,
                    None,
                    artifact_id,
                ),
                required_artifacts: vec![evidence],
                preconditions: CommitPreconditions {
                    required_run_state: RequiredRunState::Started,
                    ..CommitPreconditions::default()
                },
            })
            .expect_err("invalid retention manifest evidence shape rejects");
        assert_invalid_prepared_commit_contains(error, expected_field);
    }

    reject_manifest_shape(
        "retention-manifest-schema-present",
        |evidence| evidence.schema_id = Some(schema_id("mfm.test.retention_manifest", 224)),
        "schema_id",
    );
    reject_manifest_shape(
        "retention-manifest-producer-present",
        |evidence| evidence.producer_node_id = Some(node_id(224)),
        "producer_node_id",
    );
}

#[test]
fn retention_manifest_projection_must_chain_append_only() {
    let run_id = run_id(120);
    let first_artifact = artifact_id(123);
    let first_digest = content_digest(124);
    let second_artifact = artifact_id(125);
    let second_digest = content_digest(126);
    let mut store = InMemoryTypedRunStore::new();
    let first_evidence =
        retention_manifest_artifact_ref(first_artifact.clone(), first_digest.clone());
    let second_evidence =
        retention_manifest_artifact_ref(second_artifact.clone(), second_digest.clone());
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");

    let skipped_first = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("retention-manifest-skipped-first").expect("commit key"),
            payloads: retention_manifest_commit_payloads(
                2,
                first_digest.clone(),
                None,
                first_artifact.clone(),
            ),
            required_artifacts: vec![first_evidence.clone()],
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::Started,
                ..CommitPreconditions::default()
            },
        })
        .expect_err("first manifest must be seq 1");
    assert!(matches!(
        skipped_first,
        StoreError::ProjectionConflict { .. }
    ));

    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("retention-manifest-1").expect("commit key"),
            payloads: retention_manifest_commit_payloads(
                1,
                first_digest.clone(),
                None,
                first_artifact,
            ),
            required_artifacts: vec![first_evidence],
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::Started,
                ..CommitPreconditions::default()
            },
        })
        .expect("append first manifest");

    let wrong_previous = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("retention-manifest-wrong-prev").expect("commit key"),
            payloads: retention_manifest_commit_payloads(
                2,
                second_digest.clone(),
                Some(content_digest(128)),
                second_artifact.clone(),
            ),
            required_artifacts: vec![second_evidence.clone()],
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::Started,
                ..CommitPreconditions::default()
            },
        })
        .expect_err("wrong previous digest rejects");
    assert!(matches!(
        wrong_previous,
        StoreError::ProjectionConflict { .. }
    ));

    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("retention-manifest-2").expect("commit key"),
            payloads: retention_manifest_commit_payloads(
                2,
                second_digest.clone(),
                Some(first_digest.clone()),
                second_artifact,
            ),
            required_artifacts: vec![second_evidence],
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::Started,
                ..CommitPreconditions::default()
            },
        })
        .expect("append second manifest");

    let retention = store
        .projection_snapshot()
        .retention(&run_id)
        .expect("retention projection");
    assert_eq!(retention.manifests.len(), 2);
    assert_eq!(
        retention.manifest.as_ref().expect("latest").manifest_digest,
        second_digest
    );
    assert_eq!(
        retention
            .manifest
            .as_ref()
            .expect("latest")
            .previous_manifest_digest,
        Some(first_digest)
    );
    let stream = store.load_run_stream(&run_id);
    let verified = VerifiedRetentionProjectionSet::from_synthetic_run_streams(vec![(
        run_id.clone(),
        stream.as_slice(),
    )])
    .expect("verified retention set");
    assert!(verified.retains_artifact(&retention_manifest_artifact_ref(
        artifact_id(125),
        second_digest,
    )));
}

#[test]
fn retention_manifest_projection_requires_same_commit_retention_ref() {
    let run_id = run_id(120);
    let artifact_id = artifact_id(129);
    let digest = content_digest(130);
    let mut store = InMemoryTypedRunStore::new();
    let evidence = retention_manifest_artifact_ref(artifact_id.clone(), digest.clone());
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");

    let missing_ref = store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("retention-manifest-missing-ref").expect("commit key"),
            payloads: vec![retention_manifest_projected(1, digest, None, artifact_id)],
            required_artifacts: vec![evidence],
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::Started,
                ..CommitPreconditions::default()
            },
        })
        .expect_err("manifest without retention ref rejects");
    assert!(matches!(missing_ref, StoreError::ProjectionConflict { .. }));
}

fn assert_projection_codecs_round_trip(snapshot: &ProjectionSnapshot) {
    use mfm_store::v1::codec;

    for (_run_id, state) in snapshot.run_states() {
        assert_eq!(
            codec::parse_run_state(codec::run_state_str(*state)).expect("parse run state"),
            *state
        );
    }
    for (run_id, projection) in snapshot.run_completions() {
        let json = codec::run_completion_projection_json(run_id, projection);
        assert_eq!(
            codec::parse_run_completion_projection(&json).expect("parse run completion"),
            (run_id.clone(), projection.clone())
        );
    }
    for (run_id, projection) in snapshot.saga_engagements() {
        let json = codec::saga_engagement_projection_json(run_id, projection);
        assert_eq!(
            codec::parse_saga_engagement_projection(&json).expect("parse saga engagement"),
            (run_id.clone(), projection.clone())
        );
    }
    for (run_id, projection) in snapshot.manual_resolutions() {
        let json = codec::manual_resolution_projection_json(run_id, projection);
        assert_eq!(
            codec::parse_manual_resolution_projection(&json).expect("parse manual resolution"),
            (run_id.clone(), projection.clone())
        );
    }
    for (_key, projection) in snapshot.attempts() {
        let json = codec::attempt_projection_json(projection);
        assert_eq!(
            codec::parse_attempt_projection(&json).expect("parse attempt"),
            projection.clone()
        );
    }
    for (cell_id, projection) in snapshot.cells() {
        let json = codec::cell_projection_json(cell_id, projection);
        assert_eq!(
            codec::parse_cell_projection(&json).expect("parse cell"),
            (cell_id.clone(), projection.clone())
        );
    }
    for (_key, projection) in snapshot.facts() {
        let json = codec::fact_projection_json(projection);
        assert_eq!(
            codec::parse_fact_projection(&json).expect("parse fact"),
            projection.clone()
        );
    }
    for (_ledger_ref, projection) in snapshot.side_effects() {
        let json = codec::side_effect_projection_json(projection);
        assert_eq!(
            codec::parse_side_effect_projection(&json).expect("parse side effect"),
            projection.clone()
        );
    }
    for (lane_key, projection) in snapshot.resource_lanes() {
        let json = codec::resource_lane_projection_json(lane_key, projection);
        assert_eq!(
            codec::parse_resource_lane_projection(&json).expect("parse resource lane"),
            (lane_key.clone(), projection.clone())
        );
    }
    for (schema_id, projection) in snapshot.public_outputs() {
        let json = codec::public_output_projection_json(schema_id, projection);
        assert_eq!(
            codec::parse_public_output_projection(&json).expect("parse public output"),
            (schema_id.clone(), projection.clone())
        );
    }
    for (_run_id, projection) in snapshot.retentions() {
        for manifest in projection.manifests.values() {
            let json = codec::retention_manifest_projection_json(manifest);
            assert_eq!(
                codec::parse_retention_manifest_projection(&json)
                    .expect("parse retention manifest"),
                manifest.clone()
            );
        }
    }

    let synthetic_run_id = run_id(210);
    let run_completion = RunCompletionProjection {
        event_id: event_id(211),
        outcome: completed_outcome(212),
    };
    let json = codec::run_completion_projection_json(&synthetic_run_id, &run_completion);
    assert_eq!(
        codec::parse_run_completion_projection(&json).expect("parse synthetic run completion"),
        (synthetic_run_id.clone(), run_completion)
    );

    let saga_engagement = SagaEngagementProjection {
        event_id: event_id(213),
        reason: SagaEngagementReason::ForwardAmbiguous {
            ledger_key: side_effect_ledger_key_with_suffix(214),
        },
    };
    let json = codec::saga_engagement_projection_json(&synthetic_run_id, &saga_engagement);
    assert_eq!(
        codec::parse_saga_engagement_projection(&json).expect("parse synthetic saga engagement"),
        (synthetic_run_id.clone(), saga_engagement)
    );

    let manual_resolution = ManualResolutionProjection {
        event_id: event_id(215),
        outcome: events::ManualResolutionOutcome::ConfirmRemediated,
        evidence_schema_id: schema_id("mfm.test.manual_evidence", 219),
        evidence_hash: content_digest(220),
        evidence_artifact_id: artifact_id(221),
        authorization_schema_id: schema_id("mfm.test.manual_authorization", 222),
        authorization_hash: content_digest(223),
        authorization_artifact_id: artifact_id(224),
        note: Some(events::ManualResolutionNote::new("operator reviewed").expect("note")),
    };
    let json = codec::manual_resolution_projection_json(&synthetic_run_id, &manual_resolution);
    assert_eq!(
        codec::parse_manual_resolution_projection(&json)
            .expect("parse synthetic manual resolution"),
        (synthetic_run_id, manual_resolution)
    );
}

#[test]
fn projections_rebuild_from_authoritative_run_stream() {
    let run_id = run_id(120);
    let output_artifact_id = artifact_id(61);
    let output_digest = content_digest(62);
    let fact_artifact_id = artifact_id(63);
    let fact_digest = content_digest(64);
    let manifest_artifact_id = artifact_id(65);
    let manifest_digest = content_digest(66);
    let rendered_artifact_id = artifact_id(67);
    let rendered_digest = content_digest(68);
    let mut store = InMemoryTypedRunStore::new();
    let evidence = store_artifact_ref(output_artifact_id.clone(), output_digest.clone());
    let fact_evidence = fact_artifact_ref(fact_artifact_id.clone(), fact_digest.clone());
    let manifest_evidence =
        retention_manifest_artifact_ref(manifest_artifact_id.clone(), manifest_digest.clone());
    let rendered_evidence =
        public_output_artifact_ref(rendered_artifact_id.clone(), rendered_digest.clone());
    store
        .append_prepared_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("attempt-start").expect("commit key"),
            payloads: vec![state_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::NotCompleted,
                ..CommitPreconditions::default()
            },
        })
        .expect("append attempt start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fact-attempt-start").expect("commit key"),
            payloads: vec![fact_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::NotCompleted,
                ..CommitPreconditions::default()
            },
        })
        .expect("append fact attempt start");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fact-recorded").expect("commit key"),
            payloads: vec![fact_recorded(fact_artifact_id, fact_digest)],
            required_artifacts: vec![fact_evidence],
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::NotCompleted,
                ..CommitPreconditions::default()
            },
        })
        .expect("append fact recorded");
    append_side_effect_prepare_for_ledger(
        &mut store,
        &run_id,
        "projection-sidefx",
        side_effect_ledger_key_with_suffix(67),
        resource_key("account-1", 68),
        69,
        true,
    );
    let mut terminal_payloads =
        terminal_cell_commit_payloads(output_artifact_id.clone(), output_digest.clone());
    terminal_payloads.push(public_output_produced_with_rendered_artifact(
        output_artifact_id.clone(),
        output_digest.clone(),
        rendered_artifact_id,
        rendered_digest,
    ));
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("cell-produced").expect("commit key"),
            payloads: terminal_payloads,
            required_artifacts: vec![evidence, rendered_evidence],
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::NotCompleted,
                ..CommitPreconditions::default()
            },
        })
        .expect("append cell produced");
    store
        .append_prepared_commit(typed_commit_request! {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("retention-manifest").expect("commit key"),
            payloads: retention_manifest_commit_payloads(
                1,
                manifest_digest,
                None,
                manifest_artifact_id,
            ),
            required_artifacts: vec![manifest_evidence],
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::NotCompleted,
                ..CommitPreconditions::default()
            },
        })
        .expect("append retention manifest");

    let stream = store.load_run_stream(&run_id);
    let summary = projection_differential_summary(&store, &run_id, &stream);
    assert_eq!(
        summary,
        "committed run_state=Started commits=8 events=13 next_seq=9\n\
fact key=fact-key-1 schema=schema:mfm.test.fact_response:1:sha256-jcs-v1:6060606060606060606060606060606060606060606060606060606060606060 artifact=artifact:sha256-jcs-v1:3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f3f\n\
side_effect ledger=ledger-key-67 phase=invocation_prepared prepared=false resource_key=true touched_set=false\n\
resource_lane mfm.test.account_nonce:account-1 holder=ledger-key-67 phase_epoch=1\n\
public_output schema=schema:mfm.test.public_output:1:sha256-jcs-v1:0303030303030303030303030303030303030303030303030303030303030303 rendered_artifact=true\n\
retention run=run:sha256-jcs-v1:7878787878787878787878787878787878787878787878787878787878787878 refs=3 manifests=1 latest_seq=1"
    );

    let rebuilt =
        ProjectionSnapshot::rebuild_from_run_stream(&stream).expect("rebuild projections");
    assert_eq!(rebuilt.facts().count(), 1);
    assert_eq!(rebuilt.side_effects().count(), 1);
    assert_eq!(rebuilt.resource_lanes().count(), 1);
    assert_eq!(rebuilt.public_outputs().count(), 1);
    assert_eq!(rebuilt.retentions().count(), 1);
    assert!(matches!(
        rebuilt.cell_terminal(&cell_id(21)),
        Some(CellTerminalProjection::Produced { .. })
    ));
    assert_projection_codecs_round_trip(&rebuilt);
}
