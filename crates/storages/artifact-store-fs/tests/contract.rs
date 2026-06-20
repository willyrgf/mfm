use mfm_artifact_store_fs::{FsTypedArtifactError, FsTypedArtifactStore, TypedArtifactDescriptor};
use mfm_canonical::sha256_digest_bytes;
use mfm_events::v1::{
    ArtifactEvidenceRef as EventArtifactEvidenceRef, ArtifactProducerScope, ArtifactRole,
    ArtifactSchemaPolicy, ArtifactSemanticPolicy, FrameworkVersion, KernelEventPayload,
    RetentionReason, SeedCellRef, SourceRevision,
};
use mfm_ids::{
    ArtifactId, CellId, ContentDigest, DigestAlgorithm, DigestBytes, LoweringVersion, NodeId,
    RunId, SchemaId, ScopeId, SeedId, SemanticTypeId, SpecHash, SpecVersion,
};
use mfm_spec::v1::{CanonicalizerIdentity, MediaType, SagaPolicySpec};
use mfm_store::v1::{
    ArtifactEvidenceRef, AsyncInMemoryTypedRunStore, AsyncTypedRunEventStore,
    CommitArtifactEvidenceSet, CommitKey, CommitPreconditions, PreparedCommit, RequiredRunState,
    Retention, RunAdmission, StreamSeq, TypedCommitRequest, VerifiedRetentionProjectionSet,
};
use std::path::{Path, PathBuf};

fn digest(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

fn content_digest(bytes: &[u8]) -> ContentDigest {
    ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
}

fn artifact_id(bytes: &[u8]) -> ArtifactId {
    ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
}

fn run_artifact_ref(artifact: &ArtifactEvidenceRef) -> mfm_events::v1::RunArtifactEvidenceRef {
    mfm_events::v1::RunArtifactEvidenceRef {
        artifact_id: artifact.artifact_id.clone(),
        role: artifact.artifact_role,
        schema_id: artifact.schema_id.clone(),
        semantic_type_id: artifact.semantic_type_id.clone(),
        content_digest: artifact.digest.clone(),
        byte_len: artifact.byte_len,
        media_type: artifact.media_type.clone(),
    }
}

fn schema_id(name: &str, byte: u8) -> SchemaId {
    SchemaId::new(name, "1", DigestAlgorithm::Sha256JcsV1, digest(byte)).expect("schema id")
}

fn semantic_id(name: &str, byte: u8) -> SemanticTypeId {
    SemanticTypeId::new(
        "mfm.test",
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        digest(byte),
    )
    .expect("semantic id")
}

fn node_id(byte: u8) -> NodeId {
    NodeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

fn seed_id(byte: u8) -> SeedId {
    SeedId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

fn run_id(byte: u8) -> RunId {
    RunId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

fn spec_hash(byte: u8) -> SpecHash {
    SpecHash::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

fn cell_id(byte: u8) -> CellId {
    CellId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

fn scope_id(byte: u8) -> ScopeId {
    ScopeId::from_digest(DigestAlgorithm::Sha256JcsV1, digest(byte))
}

fn json_media_type() -> MediaType {
    MediaType::new("application/json").expect("media type")
}

fn state_output_descriptor() -> TypedArtifactDescriptor {
    TypedArtifactDescriptor {
        media_type: json_media_type(),
        schema_id: Some(schema_id("mfm.test.position", 1)),
        semantic_type_id: Some(semantic_id("position", 2)),
        producer_node_id: Some(node_id(3)),
        producer_seed_id: None,
        artifact_role: ArtifactRole::StateOutput,
    }
}

fn seed_descriptor(seed_id: SeedId) -> TypedArtifactDescriptor {
    TypedArtifactDescriptor {
        media_type: json_media_type(),
        schema_id: Some(schema_id("mfm.test.seed", 4)),
        semantic_type_id: Some(semantic_id("seed", 5)),
        producer_node_id: None,
        producer_seed_id: Some(seed_id),
        artifact_role: ArtifactRole::SeedInput,
    }
}

fn typed_blob_path(root: &Path, artifact_id: &ArtifactId) -> PathBuf {
    let digest = artifact_id.digest().to_string();
    root.join("typed")
        .join("blobs")
        .join(&digest[0..2])
        .join(artifact_id.as_str())
}

fn typed_metadata_path(root: &Path, artifact_id: &ArtifactId) -> PathBuf {
    let digest = artifact_id.digest().to_string();
    root.join("typed")
        .join("metadata")
        .join(&digest[0..2])
        .join(format!("{}.json", artifact_id.as_str()))
}

async fn persisted_artifact_fixture() -> (
    tempfile::TempDir,
    FsTypedArtifactStore,
    ArtifactEvidenceRef,
    Vec<u8>,
) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FsTypedArtifactStore::new(dir.path());
    let bytes = br#"{"amount":42}"#.to_vec();
    let evidence = store
        .put_artifact(bytes.clone(), state_output_descriptor())
        .await
        .expect("put typed artifact");
    (dir, store, evidence, bytes)
}

async fn verified_retention_projection_for(
    evidence: &ArtifactEvidenceRef,
) -> VerifiedRetentionProjectionSet {
    let run_id = run_id(80);
    let spec_hash = spec_hash(81);
    let spec_artifact_root_id =
        ArtifactId::from_digest(DigestAlgorithm::Sha256JcsV1, *spec_hash.digest());
    let spec_evidence = ArtifactEvidenceRef {
        artifact_id: spec_artifact_root_id.clone(),
        digest: ContentDigest::from_digest(spec_hash.algorithm(), *spec_hash.digest()),
        byte_len: 64,
        media_type: MediaType::new("application/vnd.mfm.typed-execution-spec+json;version=1")
            .expect("spec media"),
        schema_id: None,
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: ArtifactRole::TypedExecutionSpec,
    };
    let certificate_evidence = ArtifactEvidenceRef {
        artifact_id: artifact_id(b"certificate"),
        digest: content_digest(b"certificate"),
        byte_len: 64,
        media_type: MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE).expect("certificate media"),
        schema_id: None,
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: ArtifactRole::TypedSpecCertificate,
    };
    let run_store = AsyncInMemoryTypedRunStore::new();
    let run_start_request = TypedCommitRequest::from_payloads(
        run_id.clone(),
        StreamSeq::FIRST,
        CommitKey::new("run-start").expect("commit key"),
        vec![KernelEventPayload::RunAdmitted(Box::new(
            mfm_events::v1::RunAdmitted {
                run_id: run_id.clone(),
                spec_hash: spec_hash.clone(),
                spec_artifact: run_artifact_ref(&spec_evidence),
                certificate_artifact: run_artifact_ref(&certificate_evidence),
                config_artifacts: Vec::new(),
                spec_version: SpecVersion::new("mfm.typed.execution_spec.v1")
                    .expect("spec version"),
                lowering_version: LoweringVersion::new("mfm.typed.lowering.v1")
                    .expect("lowering version"),
                public_output_schema_id: schema_id("mfm.test.public_output", 82),
                saga_policy_digest: SagaPolicySpec::NoSideEffects
                    .saga_policy_digest()
                    .expect("saga policy digest"),
                descriptor_identities: Vec::new(),
                runner_executables: Vec::new(),
                adapter_executables: Vec::new(),
                admitted_binding_digest: content_digest(b"binding"),
                canonicalizer_identity: CanonicalizerIdentity::new("mfm.jcs.v1")
                    .expect("canonicalizer"),
                framework_version: FrameworkVersion::new("mfm.test.1").expect("framework version"),
                source_revision: SourceRevision::new("test-revision").expect("source revision"),
                launched_at_unix_ms: 1_700_000_000_000,
                seed_cells: Vec::new(),
            },
        ))],
        vec![spec_evidence, certificate_evidence],
        CommitPreconditions {
            required_run_state: RequiredRunState::Absent,
            ..CommitPreconditions::default()
        },
    )
    .expect("run start request");
    let run_start_artifacts = run_start_request.required_artifacts().to_vec();
    let run_start_artifacts =
        CommitArtifactEvidenceSet::new(run_start_artifacts.clone(), run_start_artifacts)
            .expect("run start artifact evidence set");
    let run_start_commit =
        PreparedCommit::<RunAdmission>::new(run_start_request, run_start_artifacts)
            .expect("prepare run start");
    run_store
        .append_prepared_commit_plan(run_start_commit.into())
        .await
        .expect("append run start");
    let retention_request = TypedCommitRequest::from_payloads(
        run_id.clone(),
        run_store
            .expected_next_seq(&run_id)
            .await
            .expect("expected next seq"),
        CommitKey::new("retain-artifact").expect("commit key"),
        vec![KernelEventPayload::RetentionRefsAppended(
            mfm_events::v1::RetentionRefsAppended {
                run_id: run_id.clone(),
                spec_hash,
                refs: vec![mfm_events::v1::RetentionRef {
                    artifact_id: evidence.artifact_id.clone(),
                    role: evidence.artifact_role,
                    content_digest: evidence.digest.clone(),
                }],
                reason: RetentionReason::RuntimeEvidence,
            },
        )],
        vec![evidence.clone()],
        CommitPreconditions {
            required_run_state: RequiredRunState::Started,
            ..CommitPreconditions::default()
        },
    )
    .expect("retention request");
    let retention_artifacts =
        CommitArtifactEvidenceSet::new(vec![evidence.clone()], vec![evidence.clone()])
            .expect("retention artifact evidence set");
    let retention_commit = PreparedCommit::<Retention>::new(retention_request, retention_artifacts)
        .expect("prepare retention refs");
    run_store
        .append_prepared_commit_plan(retention_commit.into())
        .await
        .expect("append retention refs");
    let stream = run_store
        .load_run_stream(&run_id)
        .await
        .expect("load run stream");
    VerifiedRetentionProjectionSet::from_synthetic_run_streams(vec![(run_id, stream.as_slice())])
        .expect("verified retention projection")
}

#[tokio::test]
async fn typed_artifact_store_persists_and_verifies_full_evidence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FsTypedArtifactStore::new(dir.path());
    let bytes = br#"{"amount":1}"#.to_vec();

    let evidence = store
        .put_artifact(bytes.clone(), state_output_descriptor())
        .await
        .expect("put typed artifact");

    assert_eq!(evidence.artifact_id, artifact_id(&bytes));
    assert_eq!(evidence.digest, content_digest(&bytes));
    assert_eq!(evidence.byte_len, bytes.len() as u64);
    assert_eq!(evidence.artifact_role, ArtifactRole::StateOutput);
    assert_eq!(store.get_artifact(&evidence).await.expect("get"), bytes);
    assert!(store.has_artifact(&evidence).await.expect("exists"));
}

#[tokio::test]
async fn typed_artifact_store_rejects_mismatched_evidence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FsTypedArtifactStore::new(dir.path());
    let bytes = b"payload".to_vec();
    let evidence = store
        .put_artifact(bytes.clone(), state_output_descriptor())
        .await
        .expect("put typed artifact");

    let mut wrong_len = evidence.clone();
    wrong_len.byte_len += 1;
    let error = store
        .put_verified_artifact(bytes.clone(), wrong_len)
        .await
        .expect_err("byte length mismatch rejects");
    assert!(matches!(
        error,
        FsTypedArtifactError::EvidenceMismatch {
            field: "byte_len",
            ..
        }
    ));

    let mut wrong_schema = evidence.clone();
    wrong_schema.schema_id = Some(schema_id("mfm.test.other", 9));
    let error = store
        .get_artifact(&wrong_schema)
        .await
        .expect_err("schema mismatch rejects");
    assert!(matches!(
        error,
        FsTypedArtifactError::EvidenceMismatch {
            field: "schema_id",
            ..
        }
    ));

    let side_effect_bytes = b"side-effect-evidence".to_vec();
    let side_effect_evidence = ArtifactEvidenceRef {
        artifact_id: artifact_id(&side_effect_bytes),
        digest: content_digest(&side_effect_bytes),
        byte_len: side_effect_bytes.len() as u64,
        media_type: json_media_type(),
        schema_id: Some(schema_id("mfm.test.side_effect", 10)),
        semantic_type_id: None,
        producer_node_id: Some(node_id(11)),
        producer_seed_id: None,
        artifact_role: ArtifactRole::SideEffectIntent,
    };
    store
        .put_verified_artifact(side_effect_bytes.clone(), side_effect_evidence.clone())
        .await
        .expect("put side-effect evidence");
    let mut wrong_role = side_effect_evidence;
    wrong_role.artifact_role = ArtifactRole::Submission;
    let error = store
        .get_artifact(&wrong_role)
        .await
        .expect_err("role mismatch rejects");
    assert!(matches!(
        error,
        FsTypedArtifactError::EvidenceMismatch {
            field: "artifact_role",
            ..
        }
    ));
}

#[tokio::test]
async fn typed_artifact_store_detects_persisted_tampering() {
    let (dir, store, evidence, _) = persisted_artifact_fixture().await;
    tokio::fs::write(
        typed_blob_path(dir.path(), &evidence.artifact_id),
        b"tampered",
    )
    .await
    .expect("tamper bytes");
    let error = store
        .get_artifact(&evidence)
        .await
        .expect_err("corrupt bytes reject");
    assert!(matches!(
        error,
        FsTypedArtifactError::EvidenceMismatch {
            field: "content_digest",
            ..
        }
    ));

    let (dir, store, evidence, _) = persisted_artifact_fixture().await;
    tokio::fs::remove_file(typed_metadata_path(dir.path(), &evidence.artifact_id))
        .await
        .expect("remove metadata");
    let error = store
        .get_artifact(&evidence)
        .await
        .expect_err("missing metadata rejects");
    assert!(matches!(error, FsTypedArtifactError::NotFound { .. }));

    let (dir, store, evidence, _) = persisted_artifact_fixture().await;
    tokio::fs::remove_file(typed_blob_path(dir.path(), &evidence.artifact_id))
        .await
        .expect("remove bytes");
    let error = store
        .get_artifact(&evidence)
        .await
        .expect_err("missing bytes rejects");
    assert!(matches!(error, FsTypedArtifactError::NotFound { .. }));

    let (dir, store, evidence, _) = persisted_artifact_fixture().await;
    tokio::fs::write(
        typed_metadata_path(dir.path(), &evidence.artifact_id),
        b"{not-json",
    )
    .await
    .expect("tamper metadata");
    let error = store
        .get_artifact(&evidence)
        .await
        .expect_err("invalid metadata rejects");
    assert!(matches!(error, FsTypedArtifactError::Corruption { .. }));
}

#[tokio::test]
async fn typed_artifact_gc_refuses_verified_retained_artifacts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FsTypedArtifactStore::new(dir.path());
    let retained = store
        .put_artifact(br#"{"retained":true}"#.to_vec(), state_output_descriptor())
        .await
        .expect("put retained artifact");
    let unretained = store
        .put_artifact(br#"{"retained":false}"#.to_vec(), state_output_descriptor())
        .await
        .expect("put unretained artifact");
    let retention = verified_retention_projection_for(&retained).await;

    let error = store
        .remove_unretained_artifact(&retained, &retention)
        .await
        .expect_err("retained artifact must not be removed");
    assert!(matches!(
        error,
        FsTypedArtifactError::RetainedArtifactRefused { .. }
    ));
    assert!(store
        .has_artifact(&retained)
        .await
        .expect("retained exists"));

    store
        .remove_unretained_artifact(&unretained, &retention)
        .await
        .expect("remove unretained artifact");
    assert!(!store
        .has_artifact(&unretained)
        .await
        .expect("unretained removed"));
}

#[tokio::test]
async fn typed_artifact_store_enforces_seed_material_persistence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FsTypedArtifactStore::new(dir.path());
    let bytes = br#"{"seed":7}"#.to_vec();
    let seed_id = seed_id(10);
    let schema_id = schema_id("mfm.test.seed", 4);
    let semantic_type_id = semantic_id("seed", 5);
    let seed_ref = SeedCellRef {
        seed_id: seed_id.clone(),
        cell_id: cell_id(11),
        scope_id: scope_id(12),
        semantic_type_id: semantic_type_id.clone(),
        schema_id: schema_id.clone(),
        digest: content_digest(&bytes),
        seed_artifact: EventArtifactEvidenceRef {
            artifact_id: artifact_id(&bytes),
            role: ArtifactRole::SeedInput,
            schema_id,
            semantic_type_id: Some(semantic_type_id),
            content_digest: content_digest(&bytes),
            byte_len: bytes.len() as u64,
            media_type: json_media_type(),
        },
    };

    let missing = store
        .require_seed_material(&seed_ref)
        .await
        .expect_err("missing seed material rejects");
    assert!(matches!(missing, FsTypedArtifactError::NotFound { .. }));

    store
        .put_artifact(bytes.clone(), seed_descriptor(seed_id))
        .await
        .expect("put seed material");
    assert_eq!(
        store
            .require_seed_material(&seed_ref)
            .await
            .expect("seed material"),
        bytes
    );
}

#[tokio::test]
async fn typed_artifact_store_rejects_non_seed_evidence_for_seed_material() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FsTypedArtifactStore::new(dir.path());
    let bytes = br#"{"seed":9}"#.to_vec();
    let stored = store
        .put_artifact(bytes.clone(), state_output_descriptor())
        .await
        .expect("put non-seed artifact");
    let seed_ref = SeedCellRef {
        seed_id: seed_id(20),
        cell_id: cell_id(21),
        scope_id: scope_id(22),
        semantic_type_id: stored.semantic_type_id.clone().expect("semantic"),
        schema_id: stored.schema_id.clone().expect("schema"),
        digest: stored.digest.clone(),
        seed_artifact: EventArtifactEvidenceRef {
            artifact_id: stored.artifact_id.clone(),
            role: ArtifactRole::SeedInput,
            schema_id: stored.schema_id.clone().expect("schema"),
            semantic_type_id: stored.semantic_type_id.clone(),
            content_digest: stored.digest,
            byte_len: stored.byte_len,
            media_type: stored.media_type,
        },
    };

    let error = store
        .require_seed_material(&seed_ref)
        .await
        .expect_err("non-seed persisted evidence rejects seed material");
    assert!(matches!(
        error,
        FsTypedArtifactError::EvidenceMismatch {
            field: "artifact_role",
            ..
        }
    ));
}

#[tokio::test]
async fn typed_seed_artifacts_require_seed_producer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FsTypedArtifactStore::new(dir.path());
    let bytes = b"seed-without-producer".to_vec();
    let mut evidence = ArtifactEvidenceRef {
        artifact_id: artifact_id(&bytes),
        digest: content_digest(&bytes),
        byte_len: bytes.len() as u64,
        media_type: json_media_type(),
        schema_id: Some(schema_id("mfm.test.seed", 4)),
        semantic_type_id: Some(semantic_id("seed", 5)),
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: ArtifactRole::SeedInput,
    };

    let error = store
        .put_verified_artifact(bytes.clone(), evidence.clone())
        .await
        .expect_err("seed input without seed producer rejects");
    assert!(matches!(
        error,
        FsTypedArtifactError::InvalidEvidence { .. }
    ));

    evidence.producer_node_id = Some(node_id(99));
    evidence.producer_seed_id = Some(seed_id(99));
    let error = store
        .put_verified_artifact(bytes, evidence)
        .await
        .expect_err("seed input with node producer rejects");
    assert!(matches!(
        error,
        FsTypedArtifactError::InvalidEvidence { .. }
    ));
}

#[derive(Debug, Clone, Copy)]
struct ProducerPolicyBaseline {
    role: ArtifactRole,
    accepts_no_producer: bool,
    accepts_node_producer: bool,
    accepts_seed_producer: bool,
}

fn artifact_store_producer_policy_baselines() -> Vec<ProducerPolicyBaseline> {
    ArtifactRole::ALL
        .iter()
        .copied()
        .map(|role| {
            let producer = role.contract().producer;
            ProducerPolicyBaseline {
                role,
                accepts_no_producer: matches!(
                    producer,
                    ArtifactProducerScope::LaunchOrGlobalNoSeed
                        | ArtifactProducerScope::GlobalNoSeed
                        | ArtifactProducerScope::DiagnosticOptionalNodeNoSeed
                        | ArtifactProducerScope::MiddlewareNoSeed
                ),
                accepts_node_producer: matches!(
                    producer,
                    ArtifactProducerScope::LaunchOrGlobalNoSeed
                        | ArtifactProducerScope::NodeRequired
                        | ArtifactProducerScope::DiagnosticOptionalNodeNoSeed
                ),
                accepts_seed_producer: matches!(producer, ArtifactProducerScope::SeedRequired),
            }
        })
        .collect()
}

fn producer_policy_evidence(
    role: ArtifactRole,
    bytes: &[u8],
    producer_node_id: Option<NodeId>,
    producer_seed_id: Option<SeedId>,
) -> ArtifactEvidenceRef {
    ArtifactEvidenceRef {
        artifact_id: artifact_id(bytes),
        digest: content_digest(bytes),
        byte_len: bytes.len() as u64,
        media_type: json_media_type(),
        schema_id: schema_id_for_role(role),
        semantic_type_id: semantic_type_id_for_role(role),
        producer_node_id,
        producer_seed_id,
        artifact_role: role,
    }
}

fn schema_id_for_role(role: ArtifactRole) -> Option<SchemaId> {
    match role.contract().schema {
        ArtifactSchemaPolicy::OptionalLaunchSchema | ArtifactSchemaPolicy::Absent => None,
        ArtifactSchemaPolicy::ExactSeedSchema
        | ArtifactSchemaPolicy::ExactValueSchema
        | ArtifactSchemaPolicy::ExactEvidenceSchema
        | ArtifactSchemaPolicy::ExactPublicSchema
        | ArtifactSchemaPolicy::ExactDiagnosticSchema => Some(schema_id("mfm.test.artifact", 34)),
    }
}

fn semantic_type_id_for_role(role: ArtifactRole) -> Option<SemanticTypeId> {
    match role.contract().semantic {
        ArtifactSemanticPolicy::OptionalLaunchSemantic | ArtifactSemanticPolicy::Absent => None,
        ArtifactSemanticPolicy::ExactSeedSemantic | ArtifactSemanticPolicy::ExactValueSemantic => {
            Some(semantic_id("mfm.test.artifact", 35))
        }
    }
}

#[tokio::test]
async fn typed_artifact_store_producer_policy_matrix_matches_current_roles() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FsTypedArtifactStore::new(dir.path());

    for (role_index, baseline) in artifact_store_producer_policy_baselines()
        .iter()
        .enumerate()
    {
        let cases = [
            ("no_producer", baseline.accepts_no_producer, None, None),
            (
                "node_producer",
                baseline.accepts_node_producer,
                Some(node_id(30)),
                None,
            ),
            (
                "seed_producer",
                baseline.accepts_seed_producer,
                None,
                Some(seed_id(31)),
            ),
            (
                "node_and_seed_producer",
                false,
                Some(node_id(32)),
                Some(seed_id(33)),
            ),
        ];

        for (case, should_accept, producer_node_id, producer_seed_id) in cases {
            let bytes = format!("producer-policy-{role_index}-{case}").into_bytes();
            let evidence =
                producer_policy_evidence(baseline.role, &bytes, producer_node_id, producer_seed_id);
            let result = store.put_verified_artifact(bytes, evidence).await;
            assert_eq!(
                result.is_ok(),
                should_accept,
                "unexpected producer policy result for {:?} {case}: {result:?}",
                baseline.role
            );
        }
    }
}

#[tokio::test]
async fn typed_non_seed_value_artifacts_require_node_producer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = FsTypedArtifactStore::new(dir.path());
    let bytes = b"state-output-without-producer".to_vec();
    let evidence = ArtifactEvidenceRef {
        artifact_id: artifact_id(&bytes),
        digest: content_digest(&bytes),
        byte_len: bytes.len() as u64,
        media_type: json_media_type(),
        schema_id: Some(schema_id("mfm.test.position", 1)),
        semantic_type_id: Some(semantic_id("position", 2)),
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: ArtifactRole::StateOutput,
    };

    let error = store
        .put_verified_artifact(bytes, evidence)
        .await
        .expect_err("state output without node producer rejects");
    assert!(matches!(
        error,
        FsTypedArtifactError::InvalidEvidence { .. }
    ));
}
