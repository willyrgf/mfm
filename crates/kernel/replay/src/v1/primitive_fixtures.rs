use super::*;

pub(super) fn persisted_envelope(
    run_id: &RunId,
    seq: u64,
    payload: KernelEventPayload,
) -> KernelEventEnvelope {
    // Explicit store order for single-run fixtures; multi-run LWW must use real appends.
    store_persisted_kernel_event_envelope_for_test(
        run_id,
        seq,
        seq,
        store::CommitKey::new(format!("replay-test:{seq}")).expect("commit key"),
        payload,
    )
}

pub(super) fn persisted_envelope_with_ordinal(
    run_id: &RunId,
    seq: u64,
    ordinal: u32,
    commit_key: store::CommitKey,
    payload: KernelEventPayload,
) -> KernelEventEnvelope {
    store_persisted_kernel_event_envelope_with_ordinal_for_test(
        run_id, seq, seq, ordinal, commit_key, payload,
    )
}

pub(super) fn persisted_commit(
    run_id: &RunId,
    seq: u64,
    payloads: Vec<KernelEventPayload>,
) -> Vec<KernelEventEnvelope> {
    let commit_key =
        store::CommitKey::new(format!("replay-test:{seq}")).expect("replay test commit key");
    payloads
        .into_iter()
        .enumerate()
        .map(|(ordinal, payload)| {
            persisted_envelope_with_ordinal(
                run_id,
                seq,
                u32::try_from(ordinal).expect("replay test event ordinal"),
                commit_key.clone(),
                payload,
            )
        })
        .collect()
}

pub(super) fn fact_response_artifact(byte: u8) -> StoredArtifactEvidenceRef {
    StoredArtifactEvidenceRef {
        artifact_id: artifact_id(byte),
        digest: content_digest(byte.wrapping_add(1)),
        byte_len: 2,
        media_type: spec::MediaType::new("application/json").expect("media type"),
        schema_id: Some(fact_response_schema_id()),
        semantic_type_id: None,
        producer_node_id: Some(fact_node_id()),
        producer_seed_id: None::<SeedId>,
        artifact_role: ArtifactRole::FactResponse,
    }
}

pub(super) fn fact_node_id() -> NodeId {
    node_id(0xce)
}

pub(super) fn fact_attempt_id() -> AttemptId {
    attempt_id(0xcf)
}

pub(super) fn fact_capability_kind() -> CapabilityKind {
    CapabilityKind::new(
        "mfm.replay.test",
        "fact_read",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(0xd0),
    )
    .expect("capability kind")
}

pub(super) fn fact_capability_version() -> CapabilityVersion {
    CapabilityVersion::new("mfm.replay.test.fact_read.v1").expect("capability version")
}

pub(super) fn fact_adapter_kind() -> AdapterKind {
    AdapterKind::new(
        "mfm.replay.test",
        "fact_adapter",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(0xd1),
    )
    .expect("adapter kind")
}

pub(super) fn fact_adapter_version() -> AdapterVersion {
    AdapterVersion::new("mfm.replay.test.fact_adapter.v1").expect("adapter version")
}

pub(super) fn fact_response_schema_id() -> SchemaId {
    schema_id("mfm.replay.test.fact_response", 0xd4)
}

pub(super) fn side_effect_terminal_policies(
    pair_id: SideEffectPairId,
    policy: store::SideEffectTerminalPolicy,
) -> store::SideEffectTerminalPolicies {
    store::SideEffectTerminalPolicies::new(BTreeMap::from([(pair_id, policy)]))
}

pub(super) fn spec_hash(byte: u8) -> SpecHash {
    SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

pub(super) fn run_id(byte: u8) -> RunId {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

pub(super) fn pair_id(byte: u8) -> SideEffectPairId {
    SideEffectPairId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

pub(super) fn node_id(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

pub(super) fn attempt_id(byte: u8) -> AttemptId {
    AttemptId::from_digest(digest_bytes(byte))
}

pub(super) fn descriptor_id(byte: u8) -> mfm_ids::DescriptorId {
    mfm_ids::DescriptorId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

pub(super) fn scope_id(byte: u8) -> mfm_ids::ScopeId {
    mfm_ids::ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

pub(super) fn state_kind(byte: u8) -> mfm_ids::StateKind {
    mfm_ids::StateKind::new(
        "mfm.replay.test",
        "state",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(byte),
    )
    .expect("state kind")
}

pub(super) fn effect_kind(byte: u8) -> mfm_ids::EffectKind {
    mfm_ids::EffectKind::new(
        "mfm.replay.test",
        "effect",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(byte),
    )
    .expect("effect kind")
}

pub(super) fn cell_id(byte: u8) -> mfm_ids::CellId {
    mfm_ids::CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

pub(super) fn semantic_type_id(byte: u8) -> SemanticTypeId {
    SemanticTypeId::new(
        "mfm.replay.test",
        "value",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest_bytes(byte),
    )
    .expect("semantic type id")
}

pub(super) fn artifact_id(byte: u8) -> ArtifactId {
    ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

pub(super) fn content_digest(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

pub(super) fn schema_id(name: &str, byte: u8) -> SchemaId {
    SchemaId::new(name, "1", DigestAlgorithm::Sha256JcsV1, digest_bytes(byte)).expect("schema id")
}

pub(super) fn digest_bytes(byte: u8) -> mfm_ids::DigestBytes {
    mfm_ids::DigestBytes::from_array([byte; 32])
}
