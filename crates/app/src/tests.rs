use super::*;
use std::sync::Mutex;

#[derive(Clone)]
struct ExpectingRetainedArtifactProvider {
    expected: store::EventArtifactRequirement,
    artifact: store::VerifiedRunArtifactBytes,
    seen: Arc<Mutex<Vec<store::EventArtifactRequirement>>>,
}

impl store::RetainedArtifactReadProvider for ExpectingRetainedArtifactProvider {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        let result = if requirement == &self.expected {
            self.seen
                .lock()
                .expect("seen lock")
                .push(requirement.clone());
            Ok(self.artifact.clone())
        } else {
            Err(store::StoreError::ArtifactEvidenceMismatch {
                artifact_id: requirement.artifact_id.clone(),
                field: "requirement",
            })
        };
        Box::pin(std::future::ready(result))
    }
}

#[tokio::test]
async fn retained_artifact_adapter_preserves_read_request_expectations() {
    let bytes = br#"{"answer":42}"#.to_vec();
    let digest = content_digest_for_bytes(&bytes);
    let schema_id = SchemaId::new(
        "mfm.test.config",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.test.config"),
    )
    .expect("schema id");
    let config_ref = spec::ConfigRef {
        schema_id: schema_id.clone(),
        artifact_id: artifact_id_for_digest(&digest),
        digest: digest.clone(),
        byte_len: bytes.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("media"),
    };
    let expected = store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id: config_ref.artifact_id.clone(),
        digest: Some(config_ref.digest.clone()),
        byte_len: Some(config_ref.byte_len),
        media_type: Some(config_ref.media_type.clone()),
        schema_id: Some(config_ref.schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::TypedConfig),
    };
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: config_ref.artifact_id.clone(),
        digest,
        byte_len: bytes.len() as u64,
        media_type: config_ref.media_type.clone(),
        schema_id: Some(schema_id),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedConfig,
    };
    let artifact = store::VerifiedRunArtifactBytes::new(bytes, evidence, &expected)
        .expect("verified artifact");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider = artifact_read_provider_from_retained(ExpectingRetainedArtifactProvider {
        expected: expected.clone(),
        artifact,
        seen: Arc::clone(&seen),
    });

    provider
        .read_artifact(
            &mfm_artifact_capabilities::ArtifactReadRequest::from_certified_config_ref(&config_ref),
        )
        .await
        .expect("adapter preserves exact config expectation");

    assert_eq!(*seen.lock().expect("seen lock"), vec![expected]);
}

#[test]
fn resource_key_status_redacts_raw_key() {
    let raw_key = "0x000000000000000000000000000000000000dead";
    let evidence = events::ResourceKeyEvidence {
        namespace: spec::ResourceNamespace::new("mfm.test.account_nonce")
            .expect("resource namespace"),
        key_schema_id: SchemaId::new(
            "mfm.test.account_nonce.resource_key",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.test.account_nonce.resource_key"),
        )
        .expect("schema id"),
        key: events::ResourceKey::new(raw_key).expect("resource key"),
    };

    let status = resource_key_status(&evidence);
    let rendered = serde_json::to_string(&status).expect("status JSON");

    assert_eq!(status.namespace, "mfm.test.account_nonce");
    assert_ne!(status.key_digest, raw_key);
    assert!(rendered.contains("key_digest"));
    assert!(!rendered.contains(raw_key));
    assert!(!rendered.contains("\"key\""));
}

#[tokio::test]
async fn run_read_services_are_evidence_only() {
    let source = include_str!("lib.rs");
    let production_read_constructor = source
        .split("pub async fn connect_production_run_read_services")
        .nth(1)
        .expect("production read constructor is present")
        .split("/// Builds the production typed runner registry")
        .next()
        .expect("production read constructor is bounded");
    assert!(!production_read_constructor.contains("production_runner_registry"));
    assert!(!production_read_constructor.contains("std::env"));

    let read_services_impl = source
        .split("pub struct RunReadServices")
        .nth(1)
        .expect("read services are present")
        .split("/// Application facade for certified typed runtime dispatch.")
        .next()
        .expect("read services implementation is bounded");
    assert!(!read_services_impl.contains("production_runner_registry"));
    assert!(!read_services_impl.contains("std::env"));

    let store = store::AsyncInMemoryRunStore::default();
    let services = make_run_read_services_with_certification_registry(
        store.clone(),
        store,
        production_certification_registry().expect("cert registry"),
    );
    let run_id = RunId::parse(
        "run:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000001",
    )
    .expect("run id");

    let status = services
        .run_status(&run_id)
        .await
        .expect_err("missing run should come from store evidence");
    assert_eq!(status.code, "RunNotFound");

    let observations = services
        .read_run_observations(store::RunObservationQuery::new(None, 50, 0))
        .await
        .expect("list observations does not construct live drivers");
    assert!(observations.runs.is_empty());
}
