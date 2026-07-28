#![cfg(feature = "test-support")]

use std::collections::BTreeMap;
use std::ops::ControlFlow;
use std::sync::Mutex;

use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes};
use mfm_capabilities::{NoCaps, Pure};
use mfm_certify::CertifiedTypedSpec;
use mfm_events::v1::{self as events, ArtifactRole, KernelEventPayload};
use mfm_ids::{
    ArtifactId, AttemptId, ContentDigest, DigestAlgorithm, DigestBytes, NodeId, RunId, SchemaId,
    SpecHash, StateKind, StateVersion,
};
use mfm_program::{
    build_root_with_registries, CanonicalSeed, NoContext, PublicOutputKey, PureState, RootBuilder,
    ScopeKey, StateKey, StateRegistryBuilder, StateResult, StateSpec,
};
use mfm_program_derive::{MfmConfig, MfmValue, PublicOutputs};
use mfm_spec::v1::{self as spec, MediaType};
use mfm_store::v1::test_support::{
    fixed_content_digest_for_test, persisted_kernel_event_envelope_for_test,
    persisted_kernel_event_envelope_with_ordinal_for_test, poll_ready_store_future_for_test,
    run_artifact_ref_from_store_artifact_for_test, run_identity_material_for_test,
    StaticRunJournalBackendForTest,
};
use mfm_store::v1::{
    ArtifactByteAuthorityMap, ArtifactEvidenceRef, AsyncInMemoryRunStore, AsyncStoreFuture,
    CommitKey, CommitOutcome, CommittedRunJournal, EventArtifactReferenceSource,
    EventArtifactRequirement, JournalLoadVerifier, KernelEventEnvelope, PreparedCommitBundle,
    RunJournalBackend, RunJournalStore, StoreError, VerifiedRetainedArtifactBytes, VerifiedRunView,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmValue)]
#[mfm(
    namespace = "mfm.store.test",
    name = "journal_value",
    version = "1",
    schema = "mfm.store.test.journal_value"
)]
struct JournalValue {
    amount: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, MfmConfig)]
struct JournalConfig {
    multiplier: u64,
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.store.test.journal_outputs")]
struct JournalOutputs<'program, 'scope> {
    result: mfm_program::Handle<'program, 'scope, JournalValue>,
}

struct JournalState {
    config: JournalConfig,
}

impl StateSpec for JournalState {
    type Config = JournalConfig;
    type Context = NoContext;
    type Input = JournalValue;
    type Output = JournalValue;
    type Effect = Pure;
    type Caps = NoCaps;

    fn kind() -> mfm_program::Result<StateKind> {
        StateKind::new(
            "mfm.store.test",
            "journal_state",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x31; 32]),
        )
        .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn version() -> mfm_program::Result<StateVersion> {
        StateVersion::new("mfm.store.test.journal_state.v1")
            .map_err(|error| mfm_program::PlanError::Key(error.to_string()))
    }

    fn name() -> &'static str {
        "mfm.store.test.journal_state"
    }

    fn new(config: mfm_program::ValidatedConfig<Self::Config>) -> mfm_program::Result<Self> {
        Ok(Self {
            config: config.into_inner(),
        })
    }
}

impl PureState for JournalState {
    fn run(
        &self,
        input: Self::Input,
        _context: &mfm_program::CertifiedContext<Self::Context>,
    ) -> StateResult<Self::Output> {
        Ok(JournalValue {
            amount: input.amount * self.config.multiplier,
        })
    }
}

struct JournalFixture {
    certification_multiplier: u64,
    run_id: RunId,
    spec_hash: SpecHash,
    node: spec::NodeSpec,
    records: Vec<KernelEventEnvelope>,
    objects: ArtifactByteAuthorityMap,
}

fn certified_spec(multiplier: u64) -> CertifiedTypedSpec {
    let mut states = StateRegistryBuilder::new();
    states
        .register::<JournalState>()
        .expect("state registration");
    let draft = build_root_with_registries(
        ScopeKey::new("root").expect("root key"),
        states.snapshot(),
        mfm_program::OperationRegistryBuilder::new().snapshot(),
        |root: &mut RootBuilder<'_, '_>| {
            let seed = root.seed(
                mfm_program::SeedKey::new("initial")?,
                CanonicalSeed::from_value(&JournalValue { amount: 2 })?,
            )?;
            let result = root.scope().state::<JournalState, _>(
                StateKey::new("journal-state")?,
                NoContext,
                JournalConfig { multiplier },
                seed,
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("terminal")?,
                &JournalOutputs { result },
            )
        },
    )
    .expect("program draft");
    mfm_certify::certify_program_draft(&draft).expect("certified journal spec")
}

fn exact_artifact(
    bytes: &[u8],
    role: ArtifactRole,
    schema_id: SchemaId,
    media_type: MediaType,
) -> ArtifactEvidenceRef {
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes));
    ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.len() as u64,
        media_type,
        schema_id: Some(schema_id),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: role,
    }
}

fn object_key(evidence: &ArtifactEvidenceRef) -> (ArtifactId, ContentDigest) {
    (
        evidence.artifact_id.clone(),
        evidence.evidence_hash().expect("evidence hash"),
    )
}

fn fixture(multiplier: u64) -> JournalFixture {
    let certified_spec = certified_spec(multiplier);
    let spec = &certified_spec.envelope().spec;
    let node = certified_spec
        .envelope()
        .spec
        .nodes
        .iter()
        .find(|node| {
            node.framework.is_none()
                && node.state_kind == JournalState::kind().expect("journal state kind")
        })
        .cloned()
        .expect("certified journal state node");
    let config_json =
        serde_json::to_string(&JournalConfig { multiplier }).expect("journal config json");
    let journal_config_bytes =
        PlainCanonicalJsonBytes::from_json_str(&config_json).expect("canonical journal config");
    let config_objects = spec
        .config_refs
        .iter()
        .map(|config_ref| {
            let bytes = spec
                .nodes
                .iter()
                .find(|node| node.config_ref == *config_ref && node.framework.is_some())
                .map(|node| {
                    let framework = node.framework.as_ref().expect("framework node");
                    spec::framework_config_canonical_json(framework.config_kind(), &node.node_id)
                        .expect("framework config")
                        .to_vec()
                })
                .unwrap_or_else(|| journal_config_bytes.to_vec());
            let digest = ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(&bytes),
            );
            assert_eq!(
                digest, config_ref.digest,
                "fixture config bytes must match certified config"
            );
            assert_eq!(bytes.len() as u64, config_ref.byte_len);
            assert_eq!(
                ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
                config_ref.artifact_id
            );
            let evidence = ArtifactEvidenceRef {
                artifact_id: config_ref.artifact_id.clone(),
                digest: config_ref.digest.clone(),
                byte_len: config_ref.byte_len,
                media_type: config_ref.media_type.clone(),
                schema_id: Some(config_ref.schema_id.clone()),
                semantic_type_id: None,
                producer_node_id: None,
                producer_seed_id: None,
                artifact_role: ArtifactRole::TypedConfig,
            };
            (bytes, evidence)
        })
        .collect::<Vec<_>>();

    let seed_spec = spec.seeds.first().cloned().expect("certified journal seed");
    assert_eq!(spec.seeds.len(), 1, "one journal seed");
    let seed =
        CanonicalSeed::from_value(&JournalValue { amount: 2 }).expect("canonical journal seed");
    let seed_bytes = seed.canonical_json().as_bytes().to_vec();
    let seed_evidence = ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(
            seed.content_digest().algorithm(),
            *seed.content_digest().digest(),
        ),
        digest: seed.content_digest().clone(),
        byte_len: seed.byte_len() as u64,
        media_type: MediaType::new("application/json").expect("seed media type"),
        schema_id: Some(seed_spec.schema_id.clone()),
        semantic_type_id: Some(seed_spec.semantic_type_id.clone()),
        producer_node_id: None,
        producer_seed_id: Some(seed_spec.seed_id.clone()),
        artifact_role: ArtifactRole::SeedInput,
    };
    let persisted = certified_spec
        .to_persisted_parts()
        .expect("persisted certified authority");
    let spec_evidence = exact_artifact(
        persisted.spec_bytes(),
        ArtifactRole::TypedExecutionSpec,
        spec::typed_execution_spec_schema_id().expect("typed spec schema"),
        certified_spec.envelope().spec.media_type.clone(),
    );
    let certificate_evidence = exact_artifact(
        persisted.certificate_bytes(),
        ArtifactRole::TypedSpecCertificate,
        mfm_certify::typed_spec_certificate_schema_id().expect("certificate schema"),
        MediaType::new(mfm_certify::CERTIFICATE_MEDIA_TYPE).expect("certificate media type"),
    );
    let identity_material =
        run_identity_material_for_test(certified_spec.spec_hash().clone(), &"31".repeat(16));
    let run_id = identity_material.derive_run_id().expect("run id");
    let admitted = events::RunAdmitted {
        run_id: run_id.clone(),
        identity_material,
        entry_point: events::EntryPointLaunchEvidence::new("mfm.store.test/journal@1", Vec::new())
            .expect("entry point"),
        spec_hash: certified_spec.spec_hash().clone(),
        spec_artifact: run_artifact_ref_from_store_artifact_for_test(&spec_evidence),
        certificate_artifact: run_artifact_ref_from_store_artifact_for_test(&certificate_evidence),
        config_artifacts: config_objects
            .iter()
            .map(|(_, evidence)| run_artifact_ref_from_store_artifact_for_test(evidence))
            .collect(),
        fact_descriptor_artifacts: Vec::new(),
        spec_version: certified_spec.envelope().spec.spec_version.clone(),
        lowering_version: certified_spec.envelope().spec.lowering_version.clone(),
        public_output_schema_id: certified_spec
            .envelope()
            .spec
            .public_outputs
            .public_schema_id
            .clone(),
        saga_policy_digest: certified_spec
            .envelope()
            .spec
            .saga
            .saga_policy_digest()
            .expect("saga digest"),
        descriptor_identities: spec.descriptor_identities.clone(),
        runner_executables: Vec::new(),
        adapter_executables: Vec::new(),
        capability_implementations: Vec::new(),
        admitted_binding_digest: fixed_content_digest_for_test(0x32),
        canonicalizer_identity: spec
            .public_outputs
            .renderer_descriptor
            .canonicalizer_identity
            .clone(),
        seed_cells: vec![events::SeedCellRef {
            seed_id: seed_spec.seed_id.clone(),
            cell_id: seed_spec.cell_id.clone(),
            scope_id: seed_spec.scope_id.clone(),
            semantic_type_id: seed_spec.semantic_type_id.clone(),
            schema_id: seed_spec.schema_id.clone(),
            digest: seed_evidence.digest.clone(),
            seed_artifact: events::ArtifactEvidenceRef {
                artifact_id: seed_evidence.artifact_id.clone(),
                role: seed_evidence.artifact_role,
                schema_id: seed_spec.schema_id,
                semantic_type_id: seed_evidence.semantic_type_id.clone(),
                content_digest: seed_evidence.digest.clone(),
                evidence_hash: seed_evidence.evidence_hash().expect("seed evidence hash"),
                byte_len: seed_evidence.byte_len,
                media_type: seed_evidence.media_type.clone(),
            },
        }],
    };
    let records = vec![persisted_record(
        &run_id,
        1,
        1,
        "journal-admission",
        KernelEventPayload::RunAdmitted(Box::new(admitted)),
    )];
    let mut objects = BTreeMap::from([
        (
            object_key(&spec_evidence),
            (persisted.spec_bytes().to_vec(), spec_evidence),
        ),
        (
            object_key(&certificate_evidence),
            (persisted.certificate_bytes().to_vec(), certificate_evidence),
        ),
        (object_key(&seed_evidence), (seed_bytes, seed_evidence)),
    ]);
    for (bytes, evidence) in config_objects {
        objects.insert(object_key(&evidence), (bytes, evidence));
    }

    JournalFixture {
        certification_multiplier: multiplier,
        spec_hash: certified_spec.spec_hash().clone(),
        run_id,
        node,
        records,
        objects,
    }
}

fn persisted_record(
    run_id: &RunId,
    sequence: u64,
    store_commit_order: u64,
    commit_key: &str,
    payload: KernelEventPayload,
) -> KernelEventEnvelope {
    persisted_kernel_event_envelope_for_test(
        run_id,
        sequence,
        store_commit_order,
        CommitKey::new(commit_key).expect("commit key"),
        payload,
    )
}

fn attempt_started(
    fixture: &JournalFixture,
    sequence: u64,
    store_commit_order: u64,
    discriminator: u8,
    attempt_no: u32,
) -> KernelEventEnvelope {
    persisted_record(
        &fixture.run_id,
        sequence,
        store_commit_order,
        &format!("attempt-start-{discriminator}"),
        KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
            spec_hash: fixture.spec_hash.clone(),
            node_id: fixture.node.node_id.clone(),
            attempt_id: attempt_id(discriminator),
            attempt_no,
            state_kind: fixture.node.state_kind.clone(),
            state_version: fixture.node.state_version.clone(),
        }),
    )
}

fn attempt_interrupted(
    fixture: &JournalFixture,
    sequence: u64,
    store_commit_order: u64,
    discriminator: u8,
) -> KernelEventEnvelope {
    persisted_record(
        &fixture.run_id,
        sequence,
        store_commit_order,
        &format!("attempt-interrupted-{discriminator}"),
        KernelEventPayload::StateAttemptInterrupted(events::StateAttemptInterrupted {
            spec_hash: fixture.spec_hash.clone(),
            node_id: fixture.node.node_id.clone(),
            attempt_id: attempt_id(discriminator),
        }),
    )
}

fn attempt_id(discriminator: u8) -> AttemptId {
    AttemptId::from_digest(DigestBytes::from_array([discriminator.wrapping_add(1); 32]))
}

fn append_interrupted_attempt(
    records: &mut Vec<KernelEventEnvelope>,
    fixture: &JournalFixture,
    start_sequence: u64,
    discriminator: u8,
    attempt_no: u32,
) {
    records.push(attempt_started(
        fixture,
        start_sequence,
        start_sequence,
        discriminator,
        attempt_no,
    ));
    records.push(attempt_interrupted(
        fixture,
        start_sequence + 1,
        start_sequence + 1,
        discriminator,
    ));
}

fn uncertified_attempt_started(
    fixture: &JournalFixture,
    sequence: u64,
    discriminator: u8,
) -> KernelEventEnvelope {
    persisted_record(
        &fixture.run_id,
        sequence,
        sequence,
        &format!("uncertified-attempt-start-{discriminator}"),
        KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
            spec_hash: fixture.spec_hash.clone(),
            node_id: NodeId::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([discriminator; 32]),
            ),
            attempt_id: attempt_id(discriminator),
            attempt_no: 1,
            state_kind: fixture.node.state_kind.clone(),
            state_version: fixture.node.state_version.clone(),
        }),
    )
}

fn journal(
    fixture: &JournalFixture,
    records: Vec<KernelEventEnvelope>,
    objects: ArtifactByteAuthorityMap,
) -> Result<CommittedRunJournal, StoreError> {
    let backend = StaticRunJournalBackendForTest::new(fixture.run_id.clone(), records, objects);
    poll_ready_store_future_for_test(backend.load_committed_journal(&fixture.run_id))
}

fn initial_view(fixture: &JournalFixture) -> VerifiedRunView {
    journal(fixture, fixture.records.clone(), fixture.objects.clone())
        .expect("initial journal")
        .verify(certified_spec(fixture.certification_multiplier))
        .expect("exact certified binding")
}

#[test]
fn journal_binding_requires_the_exact_spec_and_certificate_pair() {
    let fixture = fixture(3);
    let wrong = certified_spec(5);
    let error = journal(&fixture, fixture.records.clone(), fixture.objects.clone())
        .expect("journal")
        .verify(wrong)
        .expect_err("different certified authority must reject");
    assert!(matches!(
        error,
        StoreError::PersistedEventMismatch {
            field: "certified_spec_object",
            ..
        }
    ));

    let view = initial_view(&fixture);
    assert_eq!(view.run_id(), &fixture.run_id);
    assert_eq!(view.spec_hash(), &fixture.spec_hash);
    assert_eq!(view.current_run_sequence(), Some(1));
}

#[test]
fn current_reader_resolves_only_the_exact_full_seed_requirement() {
    let mut fixture = fixture(3);
    let unrelated_bytes = br#"{"unreferenced":"object"}"#.to_vec();
    let unrelated_evidence = exact_artifact(
        &unrelated_bytes,
        ArtifactRole::RedactedDiagnostic,
        SchemaId::new(
            "mfm.store.test.unreferenced",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x45; 32]),
        )
        .expect("unreferenced schema"),
        MediaType::new("application/json").expect("unreferenced media type"),
    );
    fixture.objects.insert(
        object_key(&unrelated_evidence),
        (unrelated_bytes, unrelated_evidence.clone()),
    );
    let view = initial_view(&fixture);
    let reader = mfm_store::v1::current_lifecycle::read(&view);
    let mut requirements = Vec::new();
    let visited = reader.visit_records(|record| {
        let _ = record.visit_artifact_requirements(|requirement| {
            requirements.push(requirement.clone());
            ControlFlow::<()>::Continue(())
        });
        ControlFlow::<()>::Continue(())
    });
    assert!(matches!(visited, ControlFlow::Continue(())));

    let seed_requirement = requirements
        .into_iter()
        .find(|requirement| requirement.source == EventArtifactReferenceSource::SeedCell)
        .expect("admission seed requirement");
    assert!(seed_requirement.digest.is_some());
    assert!(seed_requirement.byte_len.is_some());
    assert!(seed_requirement.media_type.is_some());
    assert!(seed_requirement.schema_id.is_some());
    assert!(seed_requirement.semantic_type_id.is_some());
    assert!(seed_requirement.producer_node_id.is_none());
    assert!(seed_requirement.producer_seed_id.is_some());
    assert_eq!(
        seed_requirement.artifact_role,
        Some(ArtifactRole::SeedInput)
    );

    let seed_object = reader
        .object_for_requirement(&seed_requirement)
        .expect("unchanged event-derived seed requirement resolves");
    assert_eq!(
        seed_requirement.byte_len,
        Some(seed_object.bytes().len() as u64)
    );
    assert_eq!(
        seed_requirement.media_type.as_ref(),
        Some(&seed_object.evidence().media_type)
    );

    let mut wrong_hash = seed_requirement.clone();
    wrong_hash.evidence_hash = fixed_content_digest_for_test(0x46);
    assert!(reader.object_for_requirement(&wrong_hash).is_none());

    let mut wrong_optional_field = seed_requirement.clone();
    wrong_optional_field.byte_len = seed_requirement.byte_len.map(|length| length + 1);
    assert!(reader
        .object_for_requirement(&wrong_optional_field)
        .is_none());

    let mut wrong_source = seed_requirement.clone();
    wrong_source.source = EventArtifactReferenceSource::RunSpec;
    assert!(reader.object_for_requirement(&wrong_source).is_none());

    let unreferenced_requirement = EventArtifactRequirement {
        source: EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id: unrelated_evidence.artifact_id.clone(),
        evidence_hash: unrelated_evidence
            .evidence_hash()
            .expect("unreferenced evidence hash"),
        digest: Some(unrelated_evidence.digest.clone()),
        byte_len: Some(unrelated_evidence.byte_len),
        media_type: Some(unrelated_evidence.media_type.clone()),
        schema_id: unrelated_evidence.schema_id.clone(),
        semantic_type_id: unrelated_evidence.semantic_type_id.clone(),
        producer_node_id: unrelated_evidence.producer_node_id.clone(),
        producer_seed_id: unrelated_evidence.producer_seed_id.clone(),
        artifact_role: Some(unrelated_evidence.artifact_role),
    };
    assert!(reader
        .object_for_requirement(&unreferenced_requirement)
        .is_none());
}

#[test]
fn verified_view_accepts_two_multi_commit_successors_without_recertification() {
    let fixture = fixture(3);
    let view = initial_view(&fixture);
    let mut first_records = fixture.records.clone();
    append_interrupted_attempt(&mut first_records, &fixture, 2, 0x41, 1);
    let first_successor = journal(&fixture, first_records.clone(), fixture.objects.clone())
        .expect("first successor journal");

    let once_advanced = view
        .verify_successor(first_successor)
        .expect("first strict extension must retain certified authority");
    assert_eq!(once_advanced.current_run_sequence(), Some(3));

    let mut second_records = first_records;
    append_interrupted_attempt(&mut second_records, &fixture, 4, 0x42, 2);
    let second_successor = journal(&fixture, second_records, fixture.objects.clone())
        .expect("second successor journal");
    let twice_advanced = once_advanced
        .verify_successor(second_successor)
        .expect("second strict extension must retain certified authority");
    assert_eq!(twice_advanced.current_run_sequence(), Some(5));
    assert_eq!(twice_advanced.spec_hash(), &fixture.spec_hash);
}

#[test]
fn verified_view_rejects_equal_truncated_divergent_and_reordered_histories() {
    for case in ["equal", "truncated", "divergent", "reordered"] {
        let fixture = fixture(3);
        let mut baseline = fixture.records.clone();
        append_interrupted_attempt(&mut baseline, &fixture, 2, 0x51, 1);
        append_interrupted_attempt(&mut baseline, &fixture, 4, 0x52, 2);
        let view = journal(&fixture, baseline.clone(), fixture.objects.clone())
            .expect("baseline journal")
            .verify(certified_spec(fixture.certification_multiplier))
            .expect("baseline view");

        let candidate_records = match case {
            "equal" => baseline.clone(),
            "truncated" => {
                let mut records = fixture.records.clone();
                append_interrupted_attempt(&mut records, &fixture, 2, 0x51, 1);
                records
            }
            "divergent" => {
                let mut records = fixture.records.clone();
                append_interrupted_attempt(&mut records, &fixture, 2, 0x51, 1);
                append_interrupted_attempt(&mut records, &fixture, 4, 0x61, 2);
                append_interrupted_attempt(&mut records, &fixture, 6, 0x63, 3);
                records
            }
            "reordered" => {
                let mut records = fixture.records.clone();
                append_interrupted_attempt(&mut records, &fixture, 2, 0x52, 1);
                append_interrupted_attempt(&mut records, &fixture, 4, 0x51, 2);
                append_interrupted_attempt(&mut records, &fixture, 6, 0x53, 3);
                records
            }
            _ => unreachable!(),
        };
        let independently_verified =
            journal(&fixture, candidate_records.clone(), fixture.objects.clone())
                .expect(case)
                .verify(certified_spec(fixture.certification_multiplier))
                .unwrap_or_else(|error| {
                    panic!("{case} candidate must be certified-valid: {error}")
                });
        drop(independently_verified);
        let candidate = journal(&fixture, candidate_records, fixture.objects.clone()).expect(case);
        assert!(view.verify_successor(candidate).is_err(), "{case}");
    }
}

#[test]
fn verified_view_rejects_an_illegal_certified_lifecycle_suffix() {
    let fixture = fixture(3);
    let view = initial_view(&fixture);
    let mut records = fixture.records.clone();
    records.push(uncertified_attempt_started(&fixture, 2, 0x70));
    let candidate =
        journal(&fixture, records, fixture.objects.clone()).expect("physical successor journal");

    let error = view
        .verify_successor(candidate)
        .expect_err("uncertified node in suffix must reject");
    assert!(matches!(
        error,
        StoreError::PersistedEventMismatch {
            field: "certified_history",
            ..
        }
    ));
}

#[test]
fn verified_view_rejects_an_appended_ordinal_in_the_old_atomic_batch() {
    const DEBUG_SENTINEL: &str = "same-batch-retained-diagnostic";

    let fixture = fixture(3);
    let diagnostic_bytes = DEBUG_SENTINEL.as_bytes().to_vec();
    let mut diagnostic_evidence = exact_artifact(
        &diagnostic_bytes,
        ArtifactRole::ExternalReadEvidence,
        SchemaId::new(
            "mfm.store.test.successor_diagnostic",
            "1",
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x73; 32]),
        )
        .expect("diagnostic schema"),
        MediaType::new("application/json").expect("diagnostic media type"),
    );
    diagnostic_evidence.producer_node_id = Some(fixture.node.node_id.clone());
    let event_evidence = events::ArtifactEvidenceRef {
        artifact_id: diagnostic_evidence.artifact_id.clone(),
        role: diagnostic_evidence.artifact_role,
        schema_id: diagnostic_evidence
            .schema_id
            .clone()
            .expect("diagnostic schema"),
        semantic_type_id: None,
        content_digest: diagnostic_evidence.digest.clone(),
        evidence_hash: diagnostic_evidence
            .evidence_hash()
            .expect("diagnostic evidence hash"),
        byte_len: diagnostic_evidence.byte_len,
        media_type: diagnostic_evidence.media_type.clone(),
    };
    let retained = diagnostic_evidence
        .retention_ref()
        .expect("diagnostic retention ref");
    let mut objects = fixture.objects.clone();
    objects.insert(
        object_key(&diagnostic_evidence),
        (diagnostic_bytes, diagnostic_evidence.clone()),
    );
    let batch_key = CommitKey::new("diagnostic-retention").expect("commit key");
    let mut baseline = fixture.records.clone();
    baseline.push(attempt_started(&fixture, 2, 2, 0x71, 1));
    baseline.push(persisted_kernel_event_envelope_with_ordinal_for_test(
        &fixture.run_id,
        3,
        3,
        0,
        batch_key.clone(),
        KernelEventPayload::ArtifactReferenced(events::ArtifactReferenced {
            spec_hash: fixture.spec_hash.clone(),
            node_id: Some(fixture.node.node_id.clone()),
            attempt_id: Some(attempt_id(0x71)),
            artifact_ref: event_evidence,
        }),
    ));
    baseline.push(persisted_kernel_event_envelope_with_ordinal_for_test(
        &fixture.run_id,
        3,
        3,
        1,
        batch_key.clone(),
        KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
            run_id: fixture.run_id.clone(),
            spec_hash: fixture.spec_hash.clone(),
            refs: vec![retained.clone()],
            reason: events::RetentionReason::RuntimeEvidence,
        }),
    ));
    let baseline_journal =
        journal(&fixture, baseline.clone(), objects.clone()).expect("baseline journal");
    assert!(
        !format!("{baseline_journal:?}").contains(DEBUG_SENTINEL),
        "journal Debug must not expose retained bytes"
    );
    let view = baseline_journal
        .verify(certified_spec(fixture.certification_multiplier))
        .expect("baseline view");
    assert!(
        !format!("{view:?}").contains(DEBUG_SENTINEL),
        "verified-view Debug must not expose retained bytes"
    );
    {
        let reader = mfm_store::v1::current_lifecycle::read(&view);
        let reference = reader
            .artifact_reference(
                &diagnostic_evidence.artifact_id,
                &diagnostic_evidence
                    .evidence_hash()
                    .expect("diagnostic evidence hash"),
            )
            .expect("explicit diagnostic reference");
        assert!(
            !format!("{reference:?}").contains(DEBUG_SENTINEL),
            "artifact-reference Debug must not expose retained bytes"
        );
        let record = reference.record();
        assert!(
            !format!("{record:?}").contains(DEBUG_SENTINEL),
            "record Debug must not expose retained bytes"
        );
        let mut requirement = None;
        let visited = record.visit_artifact_requirements(|candidate| {
            requirement = Some(candidate.clone());
            ControlFlow::<()>::Break(())
        });
        assert!(matches!(visited, ControlFlow::Break(())));
        let requirement = requirement.expect("explicit artifact requirement");
        let object = reader
            .object_for_requirement(&requirement)
            .expect("exact diagnostic object");
        assert_eq!(object.bytes(), DEBUG_SENTINEL.as_bytes());
        assert!(
            !format!("{object:?}").contains(DEBUG_SENTINEL),
            "object Debug must not expose retained bytes"
        );
        let verified_bytes = VerifiedRetainedArtifactBytes::new(
            object.bytes().to_vec(),
            object.evidence().clone(),
            &requirement,
        )
        .expect("verified retained diagnostic");
        assert!(
            !format!("{verified_bytes:?}").contains(DEBUG_SENTINEL),
            "verified retained bytes Debug must not expose retained bytes"
        );
    }

    let mut candidate_records = baseline;
    candidate_records.push(persisted_kernel_event_envelope_with_ordinal_for_test(
        &fixture.run_id,
        3,
        3,
        2,
        batch_key,
        KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
            run_id: fixture.run_id.clone(),
            spec_hash: fixture.spec_hash.clone(),
            refs: vec![retained],
            reason: events::RetentionReason::RuntimeEvidence,
        }),
    ));
    let independently_verified = journal(&fixture, candidate_records.clone(), objects.clone())
        .expect("same-batch ordinal is a physically valid full journal");
    let independently_verified = independently_verified
        .verify(certified_spec(fixture.certification_multiplier))
        .expect("same-batch candidate is independently certified-valid");
    drop(independently_verified);
    let candidate = journal(&fixture, candidate_records, objects)
        .expect("same-batch ordinal is a physically valid full journal");

    let error = view
        .verify_successor(candidate)
        .expect_err("successor must begin with a new atomic commit");
    assert!(matches!(
        error,
        StoreError::PersistedEventMismatch {
            field: "journal_successor",
            ..
        }
    ));
}

#[test]
fn a_replaced_old_object_cannot_become_successor_authority() {
    let fixture = fixture(3);
    let spec_key = fixture
        .objects
        .iter()
        .find(|(_, (_, evidence))| evidence.artifact_role == ArtifactRole::TypedExecutionSpec)
        .map(|(key, _)| key.clone())
        .expect("spec object key");
    let mut replacement_objects = fixture.objects.clone();
    replacement_objects
        .get_mut(&spec_key)
        .expect("spec object")
        .0
        .push(b'!');
    let mut extended = fixture.records.clone();
    extended.push(attempt_started(&fixture, 2, 2, 0x72, 1));

    let error = journal(&fixture, extended, replacement_objects)
        .expect_err("changed old object must fail before successor authority is minted");
    assert!(matches!(
        error,
        StoreError::ArtifactEvidenceMismatch { field: "bytes", .. }
    ));
}

struct DelegatingBackendForTest {
    journal: Mutex<Option<CommittedRunJournal>>,
    accept_with_verifier: bool,
}

impl DelegatingBackendForTest {
    fn new(journal: CommittedRunJournal, accept_with_verifier: bool) -> Self {
        Self {
            journal: Mutex::new(Some(journal)),
            accept_with_verifier,
        }
    }
}

impl RunJournalBackend for DelegatingBackendForTest {
    type Error = StoreError;

    fn backend_append<'a>(
        &'a self,
        _bundle: PreparedCommitBundle,
    ) -> AsyncStoreFuture<'a, CommitOutcome, Self::Error> {
        Box::pin(std::future::ready(Err(StoreError::Event(
            "delegating journal fixture does not append".to_owned(),
        ))))
    }

    fn backend_load<'a>(
        &'a self,
        verifier: JournalLoadVerifier,
    ) -> AsyncStoreFuture<'a, CommittedRunJournal, Self::Error> {
        let journal = self
            .journal
            .lock()
            .expect("delegating fixture lock")
            .take()
            .expect("one delegated load");
        let result = if self.accept_with_verifier {
            verifier.accept_verified(journal)
        } else {
            drop(verifier);
            Ok(journal)
        };
        Box::pin(std::future::ready(result))
    }
}

#[test]
fn blanket_journal_contract_accepts_same_run_delegation() {
    let fixture = fixture(3);
    let delegated = journal(&fixture, fixture.records.clone(), fixture.objects.clone())
        .expect("delegated journal");
    let backend = DelegatingBackendForTest::new(delegated, true);

    let loaded = poll_ready_store_future_for_test(backend.load_committed_journal(&fixture.run_id))
        .expect("same-run delegation");
    assert_eq!(loaded.run_id(), &fixture.run_id);
}

#[test]
fn journal_load_verifier_rejects_wrong_run_delegation() {
    let returned_fixture = fixture(3);
    let requested_fixture = fixture(5);
    let delegated = journal(
        &returned_fixture,
        returned_fixture.records.clone(),
        returned_fixture.objects.clone(),
    )
    .expect("delegated journal");
    let backend = DelegatingBackendForTest::new(delegated, true);

    let error =
        poll_ready_store_future_for_test(backend.load_committed_journal(&requested_fixture.run_id))
            .expect_err("verifier must reject delegated authority for another run");
    assert!(matches!(
        error,
        StoreError::PersistedEventMismatch {
            field: "run_id",
            ..
        }
    ));
}

#[test]
fn blanket_journal_contract_rejects_backend_that_drops_the_verifier() {
    let returned_fixture = fixture(3);
    let requested_fixture = fixture(5);
    let substituted = journal(
        &returned_fixture,
        returned_fixture.records.clone(),
        returned_fixture.objects.clone(),
    )
    .expect("substituted journal");
    let backend = DelegatingBackendForTest::new(substituted, false);

    let error =
        poll_ready_store_future_for_test(backend.load_committed_journal(&requested_fixture.run_id))
            .expect_err("blanket contract must reject requested-run substitution");
    assert!(matches!(
        error,
        StoreError::PersistedEventMismatch {
            field: "run_id",
            ..
        }
    ));
}

#[test]
fn memory_backend_receives_only_the_blanket_journal_contract() {
    fn require_backend_and_consumer_contract<T>()
    where
        T: RunJournalBackend + RunJournalStore,
    {
    }

    require_backend_and_consumer_contract::<AsyncInMemoryRunStore>();
}

#[test]
fn physical_journal_allows_one_atomic_commit_key_but_rejects_later_reuse() {
    let fixture = fixture(3);
    let atomic_key = CommitKey::new("one-atomic-key").expect("atomic key");
    let retained_evidence = fixture
        .objects
        .values()
        .filter_map(|(_, evidence)| {
            matches!(
                evidence.artifact_role,
                ArtifactRole::TypedExecutionSpec | ArtifactRole::TypedSpecCertificate
            )
            .then_some(evidence.clone())
        })
        .collect::<Vec<_>>();
    assert_eq!(retained_evidence.len(), 2, "spec and certificate evidence");
    let first_retained_ref = retained_evidence[0]
        .retention_ref()
        .expect("first retention ref");
    let second_retained_ref = retained_evidence[1]
        .retention_ref()
        .expect("second retention ref");
    let mut same_batch = fixture.records.clone();
    same_batch.push(persisted_kernel_event_envelope_with_ordinal_for_test(
        &fixture.run_id,
        2,
        2,
        0,
        atomic_key.clone(),
        KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
            run_id: fixture.run_id.clone(),
            spec_hash: fixture.spec_hash.clone(),
            refs: vec![first_retained_ref],
            reason: events::RetentionReason::RuntimeEvidence,
        }),
    ));
    same_batch.push(persisted_kernel_event_envelope_with_ordinal_for_test(
        &fixture.run_id,
        2,
        2,
        1,
        atomic_key,
        KernelEventPayload::RetentionRefsAppended(events::RetentionRefsAppended {
            run_id: fixture.run_id.clone(),
            spec_hash: fixture.spec_hash.clone(),
            refs: vec![second_retained_ref],
            reason: events::RetentionReason::RuntimeEvidence,
        }),
    ));
    journal(&fixture, same_batch, fixture.objects.clone())
        .expect("one commit key is shared by every ordinal in its atomic sequence");

    let mut reused = fixture.records.clone();
    reused.push(attempt_started(&fixture, 2, 2, 0x7b, 1));
    reused.push(persisted_record(
        &fixture.run_id,
        3,
        3,
        "journal-admission",
        KernelEventPayload::StateAttemptInterrupted(events::StateAttemptInterrupted {
            spec_hash: fixture.spec_hash.clone(),
            node_id: fixture.node.node_id.clone(),
            attempt_id: attempt_id(0x7b),
        }),
    ));
    let error = journal(&fixture, reused, fixture.objects.clone())
        .expect_err("a committed key cannot identify a later sequence");
    assert!(matches!(
        error,
        StoreError::PersistedEventMismatch {
            field: "commit_key",
            message,
        } if message == "persisted run journal reuses a commit key at a later sequence"
    ));
}
