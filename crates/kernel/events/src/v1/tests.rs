use super::*;
use mfm_ids::DigestBytes;

#[path = "diagnostic_tests.rs"]
mod diagnostic_tests;
use self::diagnostic_tests::*;
#[path = "artifact_schema_tests.rs"]
mod artifact_schema_tests;
use self::artifact_schema_tests::*;
#[path = "event_schema_tests.rs"]
mod event_schema_tests;

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

fn side_effect_pair_id(byte: u8) -> SideEffectPairId {
    SideEffectPairId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
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

fn seed_id(byte: u8) -> SeedId {
    SeedId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
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

fn media_type(value: &str) -> MediaType {
    MediaType::new(value).expect("media type")
}

fn event_artifact_ref(
    artifact_id: ArtifactId,
    role: ArtifactRole,
    schema_id: SchemaId,
    content_digest: ContentDigest,
) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id,
        role,
        schema_id,
        semantic_type_id: None,
        content_digest: content_digest.clone(),
        evidence_hash: content_digest,
        byte_len: 64,
        media_type: media_type("application/json"),
    }
}

fn fact_claim(
    capability_kind_byte: u8,
    adapter_kind_byte: u8,
    request_schema_byte: u8,
    request_hash_byte: u8,
    response_schema_byte: u8,
    response_hash_byte: u8,
    artifact_id_byte: u8,
) -> mfm_facts::FactClaim {
    mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        visibility: mfm_facts::FactVisibility::indexed_default(mfm_facts::FactAudience::Platform),
        fact_kind: mfm_facts::FactKind::new("chain.head").expect("fact kind"),
        fact_descriptor_hash: content_digest(artifact_id_byte.wrapping_add(1)),
        subject: fact_subject_evidence(artifact_id_byte.wrapping_add(2)),
        observed_at: Some("2026-01-02T03:04:05Z".to_owned()),
        request: Some(mfm_facts::FactRequestEvidence::new(
            schema_id("mfm.test.fact_request", request_schema_byte),
            content_digest(request_hash_byte),
        )),
        response: mfm_facts::FactResponseEvidence::new(
            schema_id("mfm.test.fact_response", response_schema_byte),
            content_digest(response_hash_byte),
            artifact_id(artifact_id_byte),
            content_digest(artifact_id_byte.wrapping_add(5)),
        ),
        producer: mfm_facts::FactProducerProvenance::new(
            CapabilityKind::new(
                "mfm.test",
                "capability",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(capability_kind_byte),
            )
            .expect("capability kind"),
            CapabilityVersion::new("mfm.test.capability.v1").expect("capability version"),
            AdapterKind::new(
                "mfm.test",
                "adapter",
                DigestAlgorithm::Sha256JcsV1,
                digest_bytes(adapter_kind_byte),
            )
            .expect("adapter kind"),
            AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version"),
        ),
    })
    .expect("fact claim")
}

fn fact_subject_evidence(namespace_byte: u8) -> mfm_facts::FactSubjectEvidence {
    let material = mfm_facts::FactSubjectMaterialV2::new(
        mfm_canonical::CanonicalValue::object([(
            "chain",
            mfm_canonical::CanonicalValue::String(format!("chain_{namespace_byte}")),
        )])
        .expect("subject"),
    )
    .expect("subject material");
    mfm_facts::FactSubjectEvidence::from_material(content_digest(namespace_byte), &material)
        .expect("subject evidence")
}

fn run_artifact_ref(
    artifact_id: ArtifactId,
    role: ArtifactRole,
    schema_id: Option<SchemaId>,
    content_digest: ContentDigest,
) -> RunArtifactEvidenceRef {
    RunArtifactEvidenceRef {
        artifact_id,
        role,
        schema_id,
        semantic_type_id: None,
        content_digest: content_digest.clone(),
        evidence_hash: content_digest,
        byte_len: 64,
        media_type: media_type("application/json"),
    }
}
