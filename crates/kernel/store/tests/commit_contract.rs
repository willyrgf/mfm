use mfm_events::v1::{self as events, side_effect, ArtifactRole, KernelEventPayload};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion, CellId,
    ContentDigest, DescriptorId, DigestAlgorithm, DigestBytes, LoweringVersion, NodeId, RunId,
    SchemaId, ScopeId, SeedId, SemanticTypeId, SpecHash, SpecVersion, StateKind, StateVersion,
};
use mfm_spec::v1::{
    CanonicalizerIdentity, CellProducer, MediaType, PublicFieldPath, ValueLineageRef,
};
use mfm_store::v1::{
    build_committed_batch, ArtifactEvidenceRef, CellTerminalProjection, CommitKey, CommitOutcome,
    CommitPreconditions, InMemoryTypedRunStore, ProjectionSnapshot, RequiredRunState, StoreError,
    StreamSeq, TypedCommitRequest, TypedProjectionRead, TypedRunEventStore,
    VerifiedRetentionProjection, VerifiedRetentionProjectionSet,
};

const SPEC_MEDIA_TYPE: &str = "application/vnd.mfm.typed-execution-spec+json;version=1";

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

fn run_started(run_id: RunId) -> KernelEventPayload {
    KernelEventPayload::RunStarted(events::RunStarted {
        run_id,
        spec_hash: spec_hash(1),
        spec_artifact_id: artifact_id(2),
        spec_media_type: media_type(SPEC_MEDIA_TYPE),
        spec_version: SpecVersion::new("mfm.typed.execution_spec.v1").expect("spec version"),
        lowering_version: LoweringVersion::new("mfm.typed.lowering.v1").expect("lowering version"),
        public_output_schema_id: schema_id("mfm.test.public_output", 3),
        descriptor_identities: Vec::new(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        canonicalizer_identity: CanonicalizerIdentity::new("mfm.jcs.v1").expect("canonicalizer"),
        framework_version: events::FrameworkVersion::new("mfm.test.1").expect("framework version"),
        source_revision: events::SourceRevision::new("test-revision").expect("source revision"),
        seed_cells: Vec::new(),
    })
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

fn intent_artifact_ref(artifact_id: ArtifactId, digest: ContentDigest) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id,
        digest,
        byte_len: 256,
        media_type: media_type("application/json"),
        schema_id: Some(schema_id("mfm.test.side_effect_intent", 70)),
        semantic_type_id: None,
        producer_node_id: Some(node_id(70)),
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

fn side_effect_attempt_started() -> KernelEventPayload {
    KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
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
    KernelEventPayload::SideEffectClaimed(side_effect::Claimed {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        claim_owner: events::RunnerInvocationId::new("owner-1").expect("claim owner"),
        invocation_epoch: 1,
        claim_generation: 1,
        claim_fencing_token: side_effect::ClaimFencingToken::new("token-1").expect("token"),
    })
}

fn side_effect_claim_taken_over() -> KernelEventPayload {
    KernelEventPayload::SideEffectClaimTakenOver(side_effect::ClaimTakenOver {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        previous_claim_owner: events::RunnerInvocationId::new("owner-1").expect("previous owner"),
        new_claim_owner: events::RunnerInvocationId::new("owner-2").expect("new owner"),
        invocation_epoch: 1,
        previous_claim_generation: 1,
        claim_generation: 2,
        claim_fencing_token: side_effect::ClaimFencingToken::new("token-2").expect("token"),
    })
}

fn side_effect_claim_taken_over_with_token(token: &str) -> KernelEventPayload {
    KernelEventPayload::SideEffectClaimTakenOver(side_effect::ClaimTakenOver {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        previous_claim_owner: events::RunnerInvocationId::new("owner-1").expect("previous owner"),
        new_claim_owner: events::RunnerInvocationId::new("owner-2").expect("new owner"),
        invocation_epoch: 1,
        previous_claim_generation: 1,
        claim_generation: 2,
        claim_fencing_token: side_effect::ClaimFencingToken::new(token).expect("token"),
    })
}

fn side_effect_prepared(claim_generation: u32, token: &str) -> KernelEventPayload {
    KernelEventPayload::SideEffectInvocationPrepared(side_effect::InvocationPrepared {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        invocation_epoch: 1,
        claim_generation,
        claim_fencing_token: side_effect::ClaimFencingToken::new(token).expect("token"),
        prepared_artifact_id: None,
        prepared_hash: None,
    })
}

fn side_effect_started(owner: &str, claim_generation: u32, token: &str) -> KernelEventPayload {
    KernelEventPayload::SideEffectInvocationStarted(side_effect::InvocationStarted {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
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

fn side_effect_submission_observed(
    artifact_id: ArtifactId,
    digest: ContentDigest,
) -> KernelEventPayload {
    KernelEventPayload::SideEffectSubmissionObserved(side_effect::SubmissionObserved {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        invocation_epoch: 1,
        submission_schema_id: submission_schema(),
        submission_hash: digest,
        submission_artifact_id: artifact_id,
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
        invocation_epoch: 1,
        receipt_schema_id: receipt_schema(),
        receipt_hash: digest,
        receipt_artifact_id: artifact_id,
        replay_verifier_id: events::ReplayVerifierId::new("verifier-1").expect("verifier"),
    })
}

fn side_effect_confirmation(artifact_id: ArtifactId, digest: ContentDigest) -> KernelEventPayload {
    KernelEventPayload::SideEffectConfirmationObserved(side_effect::ConfirmationObserved {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
        invocation_epoch: 1,
        confirmation_schema_id: confirmation_schema(),
        confirmation_hash: digest,
        confirmation_artifact_id: artifact_id,
        replay_verifier_id: events::ReplayVerifierId::new("verifier-1").expect("verifier"),
    })
}

fn side_effect_failed(retryable: bool) -> KernelEventPayload {
    KernelEventPayload::SideEffectFailed(side_effect::Failed {
        spec_hash: spec_hash(1),
        node_id: node_id(70),
        attempt_id: attempt_id(72),
        ledger_key: side_effect_ledger_key(),
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

fn record_run_start_artifact(store: &mut InMemoryTypedRunStore) {
    store
        .record_artifact_evidence(spec_artifact_ref())
        .expect("record spec artifact");
}

fn run_start_request(run_id: RunId, commit_key: &str) -> TypedCommitRequest {
    TypedCommitRequest {
        run_id: run_id.clone(),
        expected_next_seq: StreamSeq::FIRST,
        commit_key: CommitKey::new(commit_key).expect("commit key"),
        payloads: vec![run_started(run_id)],
        required_artifacts: vec![spec_artifact_ref()],
        preconditions: CommitPreconditions {
            required_run_state: RequiredRunState::Absent,
            ..CommitPreconditions::default()
        },
    }
}

fn append_side_effect_prepare(store: &mut InMemoryTypedRunStore, run_id: &RunId) {
    let artifact_id = artifact_id(81);
    let artifact_digest = content_digest(82);
    store
        .record_artifact_evidence(intent_artifact_ref(
            artifact_id.clone(),
            artifact_digest.clone(),
        ))
        .expect("record intent artifact");
    store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: CommitKey::new("sidefx-attempt-start").expect("commit key"),
            payloads: vec![side_effect_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append sidefx attempt start");
    store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: CommitKey::new("sidefx-prepare").expect("commit key"),
            payloads: vec![
                side_effect_intent(artifact_id, artifact_digest),
                side_effect_claim(),
                side_effect_prepared(1, "token-1"),
            ],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append sidefx prepare");
}

fn append_side_effect_started(store: &mut InMemoryTypedRunStore, run_id: &RunId) {
    store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(run_id),
            commit_key: CommitKey::new("sidefx-started").expect("commit key"),
            payloads: vec![side_effect_started("owner-1", 1, "token-1")],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append sidefx started");
}

fn record_side_effect_artifact(
    store: &mut InMemoryTypedRunStore,
    artifact_id: ArtifactId,
    digest: ContentDigest,
    schema_id: SchemaId,
    role: ArtifactRole,
) {
    store
        .record_artifact_evidence(side_effect_artifact_ref(
            artifact_id,
            digest,
            schema_id,
            role,
        ))
        .expect("record side-effect artifact");
}

#[test]
fn commit_key_idempotency_precedes_stale_expected_next_seq() {
    let run_id = run_id(40);
    let mut store = InMemoryTypedRunStore::new();
    record_run_start_artifact(&mut store);
    let request = run_start_request(run_id.clone(), "run-start");
    let appended = store
        .append_typed_run_commit(request.clone())
        .expect("append run start");
    assert!(matches!(appended, CommitOutcome::Appended(_)));

    let mut retry = request;
    retry.expected_next_seq = StreamSeq::new(99).expect("stale seq");
    let outcome = store
        .append_typed_run_commit(retry)
        .expect("idempotent retry");

    let CommitOutcome::Idempotent(batch) = outcome else {
        panic!("same commit key and fingerprint should be idempotent");
    };
    assert_eq!(batch.seq(), StreamSeq::FIRST);
    assert_eq!(store.expected_next_seq(&run_id), StreamSeq::new(2).unwrap());
}

#[test]
fn commit_key_conflict_is_rejected_before_stale_sequence() {
    let run_id = run_id(41);
    let mut store = InMemoryTypedRunStore::new();
    record_run_start_artifact(&mut store);
    let request = run_start_request(run_id.clone(), "run-start");
    store
        .append_typed_run_commit(request)
        .expect("append run start");

    let conflicting = TypedCommitRequest {
        run_id,
        expected_next_seq: StreamSeq::new(99).expect("stale seq"),
        commit_key: CommitKey::new("run-start").expect("commit key"),
        payloads: vec![state_attempt_started()],
        required_artifacts: Vec::new(),
        preconditions: CommitPreconditions::default(),
    };

    let error = store
        .append_typed_run_commit(conflicting)
        .expect_err("same key different fingerprint conflicts");
    assert!(matches!(error, StoreError::CommitConflict { .. }));
}

#[test]
fn store_owns_envelope_sequence_ordinal_and_event_id() {
    let run_id = run_id(42);
    let mut store = InMemoryTypedRunStore::new();
    record_run_start_artifact(&mut store);
    let first = store
        .append_typed_run_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");
    let first_event = &first.batch().events()[0];
    assert_eq!(first_event.seq(), StreamSeq::FIRST);
    assert_eq!(first_event.ordinal().as_u32(), 0);
    assert_eq!(first_event.commit_key().as_str(), "run-start");
    assert_eq!(first_event.logical_key().as_str(), "run:start");

    let second = store
        .append_typed_run_commit(TypedCommitRequest {
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
        &TypedCommitRequest {
            run_id,
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
fn required_artifact_precondition_is_atomic_with_append() {
    let artifact_id = artifact_id(50);
    let artifact_digest = content_digest(51);
    let mut store = InMemoryTypedRunStore::new();
    let run_id = run_id(52);
    let evidence = event_artifact_ref(artifact_id, artifact_digest);
    let request = TypedCommitRequest {
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

    let error = store
        .append_typed_run_commit(request)
        .expect_err("missing artifact rejects commit");
    assert!(matches!(error, StoreError::MissingArtifact { .. }));
    assert!(store.load_run_stream(&run_id).is_empty());
    assert_eq!(store.expected_next_seq(&run_id), StreamSeq::FIRST);
}

#[test]
fn payload_artifact_byte_len_media_type_and_producer_mismatches_are_rejected() {
    let first_run_id = run_id(53);
    let first_artifact_id = artifact_id(54);
    let first_artifact_digest = content_digest(55);
    let mut store = InMemoryTypedRunStore::new();
    let mut wrong_len =
        store_artifact_ref(first_artifact_id.clone(), first_artifact_digest.clone());
    wrong_len.byte_len = 129;
    store
        .record_artifact_evidence(wrong_len)
        .expect("record wrong byte len");
    let evidence = event_artifact_ref(first_artifact_id.clone(), first_artifact_digest.clone());
    let request = TypedCommitRequest {
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
        required_artifacts: Vec::new(),
        preconditions: CommitPreconditions::default(),
    };

    let error = store
        .append_typed_run_commit(request)
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
    store
        .record_artifact_evidence(wrong_media)
        .expect("record wrong media type");
    let evidence = event_artifact_ref(media_artifact_id.clone(), media_artifact_digest.clone());
    let request = TypedCommitRequest {
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
        required_artifacts: Vec::new(),
        preconditions: CommitPreconditions::default(),
    };
    let error = store
        .append_typed_run_commit(request)
        .expect_err("payload media type mismatch rejects commit");
    assert!(matches!(
        error,
        StoreError::ArtifactEvidenceMismatch {
            field: "media_type",
            ..
        }
    ));
    assert!(store.load_run_stream(&media_run_id).is_empty());

    let second_run_id = run_id(56);
    let second_artifact_id = artifact_id(57);
    let second_artifact_digest = content_digest(58);
    let mut store = InMemoryTypedRunStore::new();
    let mut wrong_producer =
        store_artifact_ref(second_artifact_id.clone(), second_artifact_digest.clone());
    wrong_producer.producer_node_id = Some(node_id(99));
    store
        .record_artifact_evidence(wrong_producer)
        .expect("record wrong producer");
    let request = TypedCommitRequest {
        run_id: second_run_id.clone(),
        expected_next_seq: StreamSeq::FIRST,
        commit_key: CommitKey::new("artifact-producer").expect("commit key"),
        payloads: terminal_cell_commit_payloads(second_artifact_id, second_artifact_digest),
        required_artifacts: Vec::new(),
        preconditions: CommitPreconditions::default(),
    };

    let error = store
        .append_typed_run_commit(request)
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
    store
        .record_artifact_evidence(store_artifact_ref(
            artifact_id.clone(),
            artifact_digest.clone(),
        ))
        .expect("record artifact");
    store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: StreamSeq::FIRST,
            commit_key: CommitKey::new("attempt-start").expect("commit key"),
            payloads: vec![state_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append attempt start");

    let duplicate_start = store
        .append_typed_run_commit(TypedCommitRequest {
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
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("attempt-complete").expect("commit key"),
            payloads: terminal_cell_commit_payloads(artifact_id.clone(), artifact_digest.clone()),
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append attempt complete");

    let duplicate_terminal = store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("attempt-complete-2").expect("commit key"),
            payloads: terminal_cell_commit_payloads(artifact_id, artifact_digest),
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("duplicate attempt terminal rejects");
    assert!(matches!(
        duplicate_terminal,
        StoreError::DuplicateLogicalKey { .. } | StoreError::ProjectionConflict { .. }
    ));
}

#[test]
fn attempt_terminal_and_cell_terminal_must_commit_together() {
    let run_id = run_id(62);
    let mut store = InMemoryTypedRunStore::new();
    store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: StreamSeq::FIRST,
            commit_key: CommitKey::new("attempt-start").expect("commit key"),
            payloads: vec![state_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append attempt start");

    let completion_only = store
        .append_typed_run_commit(TypedCommitRequest {
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
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("cell-only").expect("commit key"),
            payloads: vec![cell_produced(artifact_id(63), content_digest(64))],
            required_artifacts: Vec::new(),
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
    store
        .record_artifact_evidence(store_artifact_ref(
            artifact_id.clone(),
            artifact_digest.clone(),
        ))
        .expect("record terminal artifact");
    store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: StreamSeq::FIRST,
            commit_key: CommitKey::new("attempt-start").expect("commit key"),
            payloads: vec![state_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append attempt start");
    store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("receipt-terminal").expect("commit key"),
            payloads: terminal_cell_commit_payloads(artifact_id.clone(), artifact_digest.clone()),
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append receipt terminal");

    let split_public_output = store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("split-public-output").expect("commit key"),
            payloads: vec![public_output_produced(artifact_id, artifact_digest)],
            required_artifacts: Vec::new(),
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
    store
        .record_artifact_evidence(intent_artifact_ref(
            artifact_id.clone(),
            artifact_digest.clone(),
        ))
        .expect("record intent artifact");
    store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: StreamSeq::FIRST,
            commit_key: CommitKey::new("sidefx-attempt-start").expect("commit key"),
            payloads: vec![side_effect_attempt_started()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append sidefx attempt start");
    store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-intent").expect("commit key"),
            payloads: vec![side_effect_intent(artifact_id, artifact_digest)],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append intent");
    store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("sidefx-claim").expect("commit key"),
            payloads: vec![side_effect_claim()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append claim");

    let wrong_generation = store
        .append_typed_run_commit(TypedCommitRequest {
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
        .append_typed_run_commit(TypedCommitRequest {
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
fn side_effect_phase_order_and_fencing_are_enforced() {
    let mut stale_owner_store = InMemoryTypedRunStore::new();
    let stale_owner_run = run_id(100);
    append_side_effect_prepare(&mut stale_owner_store, &stale_owner_run);
    let stale_owner = stale_owner_store
        .append_typed_run_commit(TypedCommitRequest {
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
        .append_typed_run_commit(TypedCommitRequest {
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
        .append_typed_run_commit(TypedCommitRequest {
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
        .append_typed_run_commit(TypedCommitRequest {
            run_id: takeover_run.clone(),
            expected_next_seq: takeover_store.expected_next_seq(&takeover_run),
            commit_key: CommitKey::new("sidefx-takeover").expect("commit key"),
            payloads: vec![side_effect_claim_taken_over()],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("takeover before invocation started succeeds");
    takeover_store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: takeover_run.clone(),
            expected_next_seq: takeover_store.expected_next_seq(&takeover_run),
            commit_key: CommitKey::new("sidefx-prepared-after-takeover").expect("commit key"),
            payloads: vec![side_effect_prepared(2, "token-2")],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("prepare after takeover succeeds");
    takeover_store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: takeover_run.clone(),
            expected_next_seq: takeover_store.expected_next_seq(&takeover_run),
            commit_key: CommitKey::new("sidefx-started-after-takeover").expect("commit key"),
            payloads: vec![side_effect_started("owner-2", 2, "token-2")],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("start after takeover succeeds");
    let late_takeover = takeover_store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: takeover_run.clone(),
            expected_next_seq: takeover_store.expected_next_seq(&takeover_run),
            commit_key: CommitKey::new("sidefx-late-takeover").expect("commit key"),
            payloads: vec![KernelEventPayload::SideEffectClaimTakenOver(
                side_effect::ClaimTakenOver {
                    spec_hash: spec_hash(1),
                    node_id: node_id(70),
                    attempt_id: attempt_id(72),
                    ledger_key: side_effect_ledger_key(),
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
    record_side_effect_artifact(
        &mut receipt_store,
        receipt_artifact.clone(),
        receipt_digest.clone(),
        receipt_schema(),
        ArtifactRole::Receipt,
    );
    let receipt_without_submission = receipt_store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: receipt_run.clone(),
            expected_next_seq: receipt_store.expected_next_seq(&receipt_run),
            commit_key: CommitKey::new("sidefx-receipt-without-submission").expect("commit key"),
            payloads: vec![side_effect_receipt(receipt_artifact, receipt_digest)],
            required_artifacts: Vec::new(),
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
    record_side_effect_artifact(
        &mut confirmation_store,
        submission_artifact.clone(),
        submission_digest.clone(),
        submission_schema(),
        ArtifactRole::Submission,
    );
    confirmation_store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: confirmation_run.clone(),
            expected_next_seq: confirmation_store.expected_next_seq(&confirmation_run),
            commit_key: CommitKey::new("sidefx-submission").expect("commit key"),
            payloads: vec![side_effect_submission_observed(
                submission_artifact,
                submission_digest,
            )],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect("append submission");
    let confirmation_artifact = artifact_id(107);
    let confirmation_digest = content_digest(108);
    record_side_effect_artifact(
        &mut confirmation_store,
        confirmation_artifact.clone(),
        confirmation_digest.clone(),
        confirmation_schema(),
        ArtifactRole::Confirmation,
    );
    let confirmation_without_receipt = confirmation_store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: confirmation_run.clone(),
            expected_next_seq: confirmation_store.expected_next_seq(&confirmation_run),
            commit_key: CommitKey::new("sidefx-confirmation-without-receipt").expect("commit key"),
            payloads: vec![side_effect_confirmation(
                confirmation_artifact,
                confirmation_digest,
            )],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions::default(),
        })
        .expect_err("confirmation without receipt rejects");
    assert!(matches!(
        confirmation_without_receipt,
        StoreError::ProjectionConflict { .. }
    ));

    let unknown_artifact = artifact_id(109);
    let unknown_digest = content_digest(110);
    record_side_effect_artifact(
        &mut confirmation_store,
        unknown_artifact.clone(),
        unknown_digest.clone(),
        unknown_schema(),
        ArtifactRole::SubmissionUnknownEvidence,
    );
    let duplicate_submit = confirmation_store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: confirmation_run.clone(),
            expected_next_seq: confirmation_store.expected_next_seq(&confirmation_run),
            commit_key: CommitKey::new("sidefx-duplicate-submit").expect("commit key"),
            payloads: vec![side_effect_submission_unknown(
                unknown_artifact,
                unknown_digest,
            )],
            required_artifacts: Vec::new(),
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
        .append_typed_run_commit(TypedCommitRequest {
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
}

#[test]
fn fact_recorded_projects_reusable_fact_evidence() {
    let run_id = run_id(95);
    let artifact_id = artifact_id(96);
    let artifact_digest = content_digest(97);
    let mut store = InMemoryTypedRunStore::new();
    record_run_start_artifact(&mut store);
    store
        .record_artifact_evidence(fact_artifact_ref(
            artifact_id.clone(),
            artifact_digest.clone(),
        ))
        .expect("record fact artifact");
    store
        .append_typed_run_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");
    store
        .append_typed_run_commit(TypedCommitRequest {
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
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fact-recorded").expect("commit key"),
            payloads: vec![fact_recorded(artifact_id.clone(), artifact_digest.clone())],
            required_artifacts: Vec::new(),
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
    record_run_start_artifact(&mut store);
    store
        .record_artifact_evidence(fact_artifact_ref(
            artifact_id.clone(),
            artifact_digest.clone(),
        ))
        .expect("record fact artifact");
    store
        .append_typed_run_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");

    let error = store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("fact-before-attempt").expect("commit key"),
            payloads: vec![fact_recorded(artifact_id, artifact_digest)],
            required_artifacts: Vec::new(),
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
    record_run_start_artifact(&mut store);
    store
        .record_artifact_evidence(store_artifact_ref(artifact_id.clone(), digest.clone()))
        .expect("record retained artifact");
    store
        .append_typed_run_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");
    store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("retention-refs").expect("commit key"),
            payloads: vec![retention_refs_appended(
                artifact_id.clone(),
                digest.clone(),
                ArtifactRole::StateOutput,
            )],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::Started,
                ..CommitPreconditions::default()
            },
        })
        .expect("append retention refs");

    let stream = store.load_run_stream(&run_id);
    let verified = VerifiedRetentionProjection::from_run_stream(run_id.clone(), &stream)
        .expect("verified retention");
    let evidence = store_artifact_ref(artifact_id.clone(), digest.clone());
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
fn retention_manifest_projection_must_chain_append_only() {
    let run_id = run_id(120);
    let first_artifact = artifact_id(123);
    let first_digest = content_digest(124);
    let second_artifact = artifact_id(125);
    let second_digest = content_digest(126);
    let mut store = InMemoryTypedRunStore::new();
    record_run_start_artifact(&mut store);
    store
        .record_artifact_evidence(retention_manifest_artifact_ref(
            first_artifact.clone(),
            first_digest.clone(),
        ))
        .expect("record first manifest artifact");
    store
        .record_artifact_evidence(retention_manifest_artifact_ref(
            second_artifact.clone(),
            second_digest.clone(),
        ))
        .expect("record second manifest artifact");
    store
        .append_typed_run_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");

    let skipped_first = store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("retention-manifest-skipped-first").expect("commit key"),
            payloads: retention_manifest_commit_payloads(
                2,
                first_digest.clone(),
                None,
                first_artifact.clone(),
            ),
            required_artifacts: Vec::new(),
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
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("retention-manifest-1").expect("commit key"),
            payloads: retention_manifest_commit_payloads(
                1,
                first_digest.clone(),
                None,
                first_artifact,
            ),
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::Started,
                ..CommitPreconditions::default()
            },
        })
        .expect("append first manifest");

    let wrong_previous = store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("retention-manifest-wrong-prev").expect("commit key"),
            payloads: retention_manifest_commit_payloads(
                2,
                second_digest.clone(),
                Some(content_digest(128)),
                second_artifact.clone(),
            ),
            required_artifacts: Vec::new(),
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
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("retention-manifest-2").expect("commit key"),
            payloads: retention_manifest_commit_payloads(
                2,
                second_digest.clone(),
                Some(first_digest.clone()),
                second_artifact,
            ),
            required_artifacts: Vec::new(),
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
    let verified =
        VerifiedRetentionProjectionSet::from_run_streams(vec![(run_id.clone(), stream.as_slice())])
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
    record_run_start_artifact(&mut store);
    store
        .record_artifact_evidence(retention_manifest_artifact_ref(
            artifact_id.clone(),
            digest.clone(),
        ))
        .expect("record manifest artifact");
    store
        .append_typed_run_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");

    let missing_ref = store
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("retention-manifest-missing-ref").expect("commit key"),
            payloads: vec![retention_manifest_projected(1, digest, None, artifact_id)],
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::Started,
                ..CommitPreconditions::default()
            },
        })
        .expect_err("manifest without retention ref rejects");
    assert!(matches!(missing_ref, StoreError::ProjectionConflict { .. }));
}

#[test]
fn projections_rebuild_from_authoritative_run_stream() {
    let run_id = run_id(60);
    let artifact_id = artifact_id(61);
    let artifact_digest = content_digest(62);
    let mut store = InMemoryTypedRunStore::new();
    record_run_start_artifact(&mut store);
    store
        .record_artifact_evidence(store_artifact_ref(
            artifact_id.clone(),
            artifact_digest.clone(),
        ))
        .expect("record artifact");
    store
        .append_typed_run_commit(run_start_request(run_id.clone(), "run-start"))
        .expect("append run start");
    store
        .append_typed_run_commit(TypedCommitRequest {
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
        .append_typed_run_commit(TypedCommitRequest {
            run_id: run_id.clone(),
            expected_next_seq: store.expected_next_seq(&run_id),
            commit_key: CommitKey::new("cell-produced").expect("commit key"),
            payloads: terminal_cell_commit_payloads(artifact_id, artifact_digest),
            required_artifacts: Vec::new(),
            preconditions: CommitPreconditions {
                required_run_state: RequiredRunState::NotCompleted,
                ..CommitPreconditions::default()
            },
        })
        .expect("append cell produced");

    let stream = store.load_run_stream(&run_id);
    let rebuilt =
        ProjectionSnapshot::rebuild_from_run_stream(&stream).expect("rebuild projections");
    assert_eq!(store.projection_snapshot(), &rebuilt);
    assert!(matches!(
        rebuilt.cell_terminal(&cell_id(21)),
        Some(CellTerminalProjection::Produced { .. })
    ));
}
