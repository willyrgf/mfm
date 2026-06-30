use mfm_artifact_capabilities::*;
use mfm_canonical::sha256_digest_bytes;
use mfm_events::v1::ArtifactRole;
use mfm_ids::{
    ArtifactId, ContentDigest, DigestAlgorithm, NodeId, SchemaId, SeedId, SemanticTypeId,
};
use mfm_spec::v1::MediaType;
use mfm_store::v1 as store;

fn digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
}

fn artifact_id(bytes: &[u8]) -> ArtifactId {
    let digest = digest(bytes);
    ArtifactId::from_digest(digest.algorithm(), *digest.digest())
}

fn schema_id(name: &str) -> SchemaId {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(name.as_bytes()),
    )
    .expect("schema id")
}

fn semantic_id(name: &str) -> SemanticTypeId {
    SemanticTypeId::new(
        "mfm.test",
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(name.as_bytes()),
    )
    .expect("semantic id")
}

fn node_id(name: &str) -> NodeId {
    NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(name.as_bytes()),
    )
}

fn seed_id(name: &str) -> SeedId {
    SeedId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(name.as_bytes()),
    )
}

fn evidence(bytes: &[u8]) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id: artifact_id(bytes),
        digest: digest(bytes),
        byte_len: bytes.len() as u64,
        media_type: MediaType::new("application/json").expect("media type"),
        schema_id: Some(schema_id("mfm.test.schema")),
        semantic_type_id: Some(semantic_id("value")),
        producer_node_id: Some(node_id("producer")),
        producer_seed_id: None,
        artifact_role: ArtifactRole::StateOutput,
    }
}

fn request(evidence: &ArtifactEvidenceRef) -> ArtifactReadRequest {
    ArtifactReadRequest::from_replay_authorized_evidence(evidence.clone())
}

#[test]
fn verifies_artifact_bytes() {
    let bytes = br#"{"ok":true}"#.to_vec();
    let evidence = evidence(&bytes);

    let verified = VerifiedArtifactBytes::new(bytes.clone(), evidence.clone(), &request(&evidence))
        .expect("verified");

    assert_eq!(verified.bytes(), bytes.as_slice());
    assert_eq!(verified.evidence(), &evidence);
}

#[test]
fn rejects_digest_mismatch() {
    let bytes = br#"{"ok":true}"#.to_vec();
    let mut evidence = evidence(&bytes);
    evidence.digest = digest(br#"{"ok":false}"#);

    let err = VerifiedArtifactBytes::new(bytes, evidence.clone(), &request(&evidence))
        .expect_err("digest mismatch");

    assert!(matches!(
        err,
        ArtifactReadError::EvidenceMismatch {
            field: "content_digest",
            ..
        }
    ));
}

#[test]
fn rejects_role_mismatch() {
    let bytes = br#"{"ok":true}"#.to_vec();
    let evidence = evidence(&bytes);
    let mut expected = evidence.clone();
    expected.artifact_role = ArtifactRole::TypedConfig;
    let request = ArtifactReadRequest::from_replay_authorized_evidence(expected);

    let err = VerifiedArtifactBytes::new(bytes, evidence, &request).expect_err("role mismatch");

    assert!(matches!(
        err,
        ArtifactReadError::EvidenceMismatch {
            field: "artifact_role",
            ..
        }
    ));
}

#[test]
fn materialized_seed_cell_request_checks_seed_role_schema_and_semantic_identity() {
    let bytes = br#"{"seed":true}"#.to_vec();
    let seed_id = seed_id("configured");
    let request = ArtifactReadRequest::from_materialized_seed_cell(
        artifact_id(&bytes),
        digest(&bytes),
        schema_id("mfm.test.seed_schema"),
        semantic_id("seed_value"),
        seed_id.clone(),
    );
    let evidence = ArtifactEvidenceRef {
        artifact_id: artifact_id(&bytes),
        digest: digest(&bytes),
        byte_len: bytes.len() as u64,
        media_type: MediaType::new("application/json").expect("media type"),
        schema_id: Some(schema_id("mfm.test.seed_schema")),
        semantic_type_id: Some(semantic_id("seed_value")),
        producer_node_id: None,
        producer_seed_id: Some(seed_id),
        artifact_role: ArtifactRole::SeedInput,
    };

    VerifiedArtifactBytes::new(bytes, evidence, &request).expect("verified seed cell");
}

#[test]
fn materialized_produced_cell_request_checks_state_output_schema_and_semantic_identity() {
    let bytes = br#"{"produced":true}"#.to_vec();
    let request = ArtifactReadRequest::from_materialized_produced_cell(
        artifact_id(&bytes),
        digest(&bytes),
        schema_id("mfm.test.output_schema"),
        semantic_id("output_value"),
        node_id("producer"),
    );
    let evidence = ArtifactEvidenceRef {
        artifact_id: artifact_id(&bytes),
        digest: digest(&bytes),
        byte_len: bytes.len() as u64,
        media_type: MediaType::new("application/json").expect("media type"),
        schema_id: Some(schema_id("mfm.test.output_schema")),
        semantic_type_id: Some(semantic_id("output_value")),
        producer_node_id: Some(node_id("producer")),
        producer_seed_id: None,
        artifact_role: ArtifactRole::StateOutput,
    };

    VerifiedArtifactBytes::new(bytes, evidence, &request).expect("verified produced cell");
}

#[test]
fn side_effect_projection_request_accepts_unprojected_schema_and_semantic_identity() {
    let bytes = br#"{"prepared":true}"#.to_vec();
    let projection = store::SideEffectArtifactProjection {
        artifact_id: artifact_id(&bytes),
        content_digest: digest(&bytes),
        schema_id: None,
    };
    let request = ArtifactReadRequest::from_side_effect_projection(
        &projection,
        ArtifactRole::PreparedInvocation,
        node_id("producer"),
    );
    let evidence = ArtifactEvidenceRef {
        artifact_id: artifact_id(&bytes),
        digest: digest(&bytes),
        byte_len: bytes.len() as u64,
        media_type: MediaType::new("application/json").expect("media type"),
        schema_id: Some(schema_id("mfm.test.prepared_schema")),
        semantic_type_id: Some(semantic_id("prepared_value")),
        producer_node_id: Some(node_id("producer")),
        producer_seed_id: None,
        artifact_role: ArtifactRole::PreparedInvocation,
    };

    VerifiedArtifactBytes::new(bytes, evidence, &request)
        .expect("verified prepared side-effect artifact");
}

#[test]
fn rejects_schema_and_semantic_mismatch() {
    let bytes = br#"{"ok":true}"#.to_vec();
    let evidence = evidence(&bytes);
    let mut expected = evidence.clone();
    expected.schema_id = Some(schema_id("mfm.test.other_schema"));
    let request = ArtifactReadRequest::from_replay_authorized_evidence(expected);

    let err = VerifiedArtifactBytes::new(bytes.clone(), evidence.clone(), &request)
        .expect_err("schema mismatch");
    assert!(matches!(
        err,
        ArtifactReadError::EvidenceMismatch {
            field: "schema_id",
            ..
        }
    ));

    let mut expected = evidence.clone();
    expected.semantic_type_id = Some(semantic_id("other_value"));
    let request = ArtifactReadRequest::from_replay_authorized_evidence(expected);

    let err = VerifiedArtifactBytes::new(bytes, evidence, &request).expect_err("semantic mismatch");
    assert!(matches!(
        err,
        ArtifactReadError::EvidenceMismatch {
            field: "semantic_type_id",
            ..
        }
    ));
}

#[test]
fn rejects_producer_mismatch() {
    let bytes = br#"{"ok":true}"#.to_vec();
    let evidence = evidence(&bytes);
    let mut expected = evidence.clone();
    expected.producer_node_id = Some(node_id("other-producer"));
    let request = ArtifactReadRequest::from_replay_authorized_evidence(expected);

    let err = VerifiedArtifactBytes::new(bytes, evidence, &request).expect_err("producer mismatch");

    assert!(matches!(
        err,
        ArtifactReadError::EvidenceMismatch {
            field: "producer_node_id",
            ..
        }
    ));
}

#[test]
fn rejects_byte_length_mismatch() {
    let bytes = br#"{"ok":true}"#.to_vec();
    let mut evidence = evidence(&bytes);
    evidence.byte_len += 1;

    let err = VerifiedArtifactBytes::new(bytes, evidence.clone(), &request(&evidence))
        .expect_err("byte length mismatch");

    assert!(matches!(
        err,
        ArtifactReadError::EvidenceMismatch {
            field: "byte_len",
            ..
        }
    ));
}

#[test]
fn errors_are_redaction_safe() {
    let bytes = br#"{"ok":true}"#.to_vec();
    let evidence = evidence(&bytes);
    let err = ArtifactReadError::redacted_backend_failure("/tmp/secret/mfm-key.json");
    let rendered = err.to_string();

    assert!(matches!(
        err,
        ArtifactReadError::Backend {
            reason: ArtifactReadBackendError::Failed
        }
    ));
    assert!(!rendered.contains("/tmp/secret"));
    assert!(!rendered.contains("password"));
    assert!(!rendered.contains("private_key"));
    assert!(!rendered.contains(String::from_utf8_lossy(&bytes).as_ref()));
    assert!(!rendered.contains(evidence.digest.as_str()));
}
