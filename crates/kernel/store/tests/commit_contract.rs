use mfm_events::v1::{self as events, side_effect, ArtifactRole, KernelEventPayload};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, AttemptId, CapabilityKind, CapabilityVersion, CellId,
    ContentDigest, DigestAlgorithm, DigestBytes, LoweringVersion, NodeId, RunId, SchemaId, ScopeId,
    SeedId, SemanticTypeId, SpecHash, SpecVersion, StateKind, StateVersion,
};
use mfm_spec::v1::{CanonicalizerIdentity, MediaType, ValueLineageRef};
use mfm_store::v1::{
    ArtifactEvidenceRef, CellTerminalProjection, CommitKey, CommitOutcome, CommitPreconditions,
    InMemoryTypedRunStore, ProjectionSnapshot, RequiredRunState, StoreError, StreamSeq,
    TypedCommitRequest, TypedProjectionRead, TypedRunEventStore,
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

fn side_effect_ledger_key() -> events::SideEffectLedgerKey {
    events::SideEffectLedgerKey::new("ledger-key-1").expect("ledger key")
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

fn run_start_request(run_id: RunId, commit_key: &str) -> TypedCommitRequest {
    TypedCommitRequest {
        run_id: run_id.clone(),
        expected_next_seq: StreamSeq::FIRST,
        commit_key: CommitKey::new(commit_key).expect("commit key"),
        payloads: vec![run_started(run_id)],
        required_artifacts: Vec::new(),
        preconditions: CommitPreconditions {
            required_run_state: RequiredRunState::Absent,
            ..CommitPreconditions::default()
        },
    }
}

#[test]
fn commit_key_idempotency_precedes_stale_expected_next_seq() {
    let run_id = run_id(40);
    let mut store = InMemoryTypedRunStore::new();
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
fn projections_rebuild_from_authoritative_run_stream() {
    let run_id = run_id(60);
    let artifact_id = artifact_id(61);
    let artifact_digest = content_digest(62);
    let mut store = InMemoryTypedRunStore::new();
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
