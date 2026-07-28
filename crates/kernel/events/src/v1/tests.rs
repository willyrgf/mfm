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

fn artifact_id(byte: u8) -> ArtifactId {
    ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn attempt_id(byte: u8) -> AttemptId {
    AttemptId::from_digest(digest_bytes(byte))
}

fn node_id(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
}

fn side_effect_pair_id(byte: u8) -> SideEffectPairId {
    SideEffectPairId::from_digest(DigestAlgorithm::Sha256JcsV1, digest_bytes(byte))
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
