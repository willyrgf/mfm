use mfm_artifact_capabilities::*;
use mfm_canonical::sha256_digest_bytes;
use mfm_events::v1::ArtifactRole;
use mfm_facts::{
    FactAudience, FactClaimId, FactKey, FactProducerProvenance, FactResponseEvidence,
    FactSubjectRef, FactVisibility, InternalFactRef, InternalFactRefParts,
};
use mfm_ids::{
    AdapterKind, AdapterVersion, ArtifactId, CapabilityKind, CapabilityVersion, ContentDigest,
    DigestAlgorithm, EventId, NodeId, RunId, SchemaId, SeedId, SemanticTypeId,
};
use mfm_spec::v1::MediaType;
use mfm_store::v1 as store;
use serde::Deserialize;

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

#[test]
fn verifies_artifact_bytes() {
    let bytes = br#"{"ok":true}"#.to_vec();
    let evidence = evidence(&bytes);
    let request = ArtifactReadRequest::from_replay_authorized_evidence(evidence.clone())
        .expect("replay authorized request");

    let verified =
        VerifiedArtifactBytes::new(bytes.clone(), evidence.clone(), &request).expect("verified");

    assert_eq!(verified.bytes(), bytes.as_slice());
    assert_eq!(verified.evidence(), &evidence);
}

#[test]
fn rejects_artifact_evidence_mismatches() {
    enum Mismatch {
        Digest,
        Role,
        Schema,
        Semantic,
        Producer,
        ByteLen,
    }

    // Identity-bearing fields are folded into evidence_hash; exact retained-artifact
    // reads fail closed on evidence_hash before the individual field is reported.
    for (name, mismatch, expected_field) in [
        ("digest", Mismatch::Digest, "content_digest"),
        ("role", Mismatch::Role, "evidence_hash"),
        ("schema", Mismatch::Schema, "evidence_hash"),
        ("semantic", Mismatch::Semantic, "evidence_hash"),
        ("producer", Mismatch::Producer, "evidence_hash"),
        ("byte length", Mismatch::ByteLen, "byte_len"),
    ] {
        let bytes = br#"{"ok":true}"#.to_vec();
        let mut evidence = evidence(&bytes);
        let mut expected = evidence.clone();
        match mismatch {
            Mismatch::Digest => {
                evidence.digest = digest(br#"{"ok":false}"#);
                expected = evidence.clone();
            }
            Mismatch::Role => {
                expected.artifact_role = ArtifactRole::TypedConfig;
            }
            Mismatch::Schema => {
                expected.schema_id = Some(schema_id("mfm.test.other_schema"));
            }
            Mismatch::Semantic => {
                expected.semantic_type_id = Some(semantic_id("other_value"));
            }
            Mismatch::Producer => {
                expected.producer_node_id = Some(node_id("other-producer"));
            }
            Mismatch::ByteLen => {
                evidence.byte_len += 1;
                expected = evidence.clone();
            }
        }
        let request = ArtifactReadRequest::from_replay_authorized_evidence(expected)
            .expect("replay authorized request");

        let err = VerifiedArtifactBytes::new(bytes, evidence, &request).expect_err(name);

        assert!(
            matches!(
                err,
                ArtifactReadError::EvidenceMismatch { field, .. } if field == expected_field
            ),
            "{name}: {err:?}"
        );
    }
}

#[test]
fn materialized_seed_cell_request_checks_seed_role_schema_and_semantic_identity() {
    let bytes = br#"{"seed":true}"#.to_vec();
    let seed_id = seed_id("configured");
    let evidence = ArtifactEvidenceRef {
        artifact_id: artifact_id(&bytes),
        digest: digest(&bytes),
        byte_len: bytes.len() as u64,
        media_type: MediaType::new("application/json").expect("media type"),
        schema_id: Some(schema_id("mfm.test.seed_schema")),
        semantic_type_id: Some(semantic_id("seed_value")),
        producer_node_id: None,
        producer_seed_id: Some(seed_id.clone()),
        artifact_role: ArtifactRole::SeedInput,
    };
    let request = ArtifactReadRequest::from_materialized_seed_cell(
        artifact_id(&bytes),
        digest(&bytes),
        evidence
            .clone()
            .into_store()
            .evidence_hash()
            .expect("seed evidence hash"),
        schema_id("mfm.test.seed_schema"),
        semantic_id("seed_value"),
        seed_id,
    );

    VerifiedArtifactBytes::new(bytes, evidence, &request).expect("verified seed cell");
}

#[test]
fn materialized_produced_cell_request_checks_state_output_schema_and_semantic_identity() {
    let bytes = br#"{"produced":true}"#.to_vec();
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
    let request = ArtifactReadRequest::from_materialized_produced_cell(
        artifact_id(&bytes),
        digest(&bytes),
        evidence
            .clone()
            .into_store()
            .evidence_hash()
            .expect("produced evidence hash"),
        schema_id("mfm.test.output_schema"),
        semantic_id("output_value"),
        node_id("producer"),
    );

    VerifiedArtifactBytes::new(bytes, evidence, &request).expect("verified produced cell");
}

#[test]
fn side_effect_projection_request_accepts_unprojected_schema_and_semantic_identity() {
    let bytes = br#"{"prepared":true}"#.to_vec();
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
    let projection = store::SideEffectArtifactProjection {
        artifact_id: artifact_id(&bytes),
        content_digest: digest(&bytes),
        evidence_hash: evidence
            .clone()
            .into_store()
            .evidence_hash()
            .expect("prepared evidence hash"),
        schema_id: None,
    };
    let request = ArtifactReadRequest::from_side_effect_projection(
        &projection,
        ArtifactRole::PreparedInvocation,
        node_id("producer"),
    );

    VerifiedArtifactBytes::new(bytes, evidence, &request)
        .expect("verified prepared side-effect artifact");
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

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct SampleFactResponse {
    height: u64,
    hash: String,
}

#[test]
fn fact_response_artifact_requirement_pins_response_identity() {
    let bytes = br#"{"height":100,"hash":"0xabc"}"#;
    let fact_ref = internal_fact_ref(bytes);

    let requirement = fact_response_artifact_requirement(&fact_ref);

    assert_eq!(
        requirement.source,
        store::EventArtifactReferenceSource::FactResponse
    );
    assert_eq!(&requirement.artifact_id, fact_ref.artifact_id());
    assert_eq!(requirement.digest.as_ref(), Some(fact_ref.response_hash()));
    assert_eq!(
        requirement.schema_id.as_ref(),
        Some(fact_ref.response_schema_id())
    );
    assert_eq!(
        requirement.producer_node_id.as_ref(),
        Some(fact_ref.producer_node_id())
    );
    assert_eq!(requirement.artifact_role, Some(ArtifactRole::FactResponse));
    assert!(requirement.byte_len.is_none());
    assert!(requirement.media_type.is_none());
    assert!(requirement.semantic_type_id.is_none());
    assert!(requirement.producer_seed_id.is_none());
}

#[test]
fn hydrate_fact_response_json_decodes_payload_and_rejects_invalid_json() {
    let ok_bytes = br#"{"height":100,"hash":"0xabc"}"#;
    let fact_ref = internal_fact_ref(ok_bytes);

    let decoded: SampleFactResponse =
        hydrate_fact_response_json(&fact_ref, ok_bytes).expect("hydrate");
    assert_eq!(
        decoded,
        SampleFactResponse {
            height: 100,
            hash: "0xabc".to_owned(),
        }
    );

    let err = hydrate_fact_response_json::<SampleFactResponse>(&fact_ref, br#"not-json"#)
        .expect_err("invalid json");
    assert!(matches!(
        err,
        ArtifactReadError::Decode {
            format: ArtifactReadDecodeFormat::Json,
            ..
        }
    ));
    assert!(!err.to_string().contains("password"));
}

fn internal_fact_ref(bytes: &[u8]) -> InternalFactRef {
    let response_digest = digest(bytes);
    let artifact_id =
        ArtifactId::from_digest(response_digest.algorithm(), *response_digest.digest());
    let producer = node_id("fact-producer");
    let schema = schema_id("mfm.test.fact_response");
    InternalFactRef::new(InternalFactRefParts {
        fact_claim_id: FactClaimId::new(
            RunId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(b"fact-run"),
            ),
            1,
            0,
        )
        .expect("claim id"),
        source_event_id: EventId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"fact-event"),
        ),
        recorded_at: "2026-07-02T00:00:00Z".to_owned(),
        producer_node_id: producer.clone(),
        observed_at: Some("2026-07-02T00:00:00Z".to_owned()),
        visibility: FactVisibility::indexed_default(FactAudience::Control),
        fact_kind: mfm_facts::FactKind::new("test.fact").expect("kind"),
        fact_descriptor_hash: digest(b"descriptor"),
        subject: FactSubjectRef::new(
            digest(b"subject-ns"),
            FactKey::from_digest(digest(b"subject-key")),
            digest(b"subject-material"),
        ),
        request: None,
        response: FactResponseEvidence::new(
            schema,
            response_digest,
            artifact_id,
            digest(b"artifact-evidence"),
        ),
        producer: FactProducerProvenance::new(
            CapabilityKind::new(
                "mfm.test",
                "read",
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(b"cap-kind"),
            )
            .expect("capability kind"),
            CapabilityVersion::new("mfm.test.read.v1").expect("capability version"),
            AdapterKind::new(
                "mfm.test",
                "adapter",
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(b"adapter-kind"),
            )
            .expect("adapter kind"),
            AdapterVersion::new("mfm.test.adapter.v1").expect("adapter version"),
        ),
    })
    .expect("fact ref")
}
