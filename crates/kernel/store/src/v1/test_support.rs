//! Test-only helpers for typed store contract fixtures.

use std::collections::BTreeMap;
use std::future::Future;
use std::task::{Context, Poll, Waker};

use super::*;
use ed25519_dalek::{Signer, SigningKey};
use mfm_canonical::{sha256_digest_bytes, CanonicalJsonBytes, CanonicalValue};
use mfm_events::v1 as events;
use mfm_ids::{
    AdapterKind, ArtifactId, AttemptId, CapabilityKind, CellId, ContentDigest, DescriptorId,
    DigestAlgorithm, DigestBytes, EventId, NodeId, RunId, SchemaId, ScopeId, SemanticTypeId,
    SpecHash, StateKind, StoreScopeId,
};
use mfm_spec::v1 as spec;

/// Descriptor projection fixture built from canonical descriptor bytes.
#[derive(Debug, Clone)]
pub struct FactDescriptorProjectionFixtureForTest {
    /// Source descriptor used to build the projection.
    pub descriptor: mfm_facts::FactDescriptor,
    /// Canonical descriptor bytes retained as an artifact.
    pub descriptor_bytes: Vec<u8>,
    /// Descriptor content hash.
    pub descriptor_hash: ContentDigest,
    /// Descriptor artifact id derived from the descriptor hash.
    pub descriptor_artifact_id: ArtifactId,
    /// Descriptor artifact evidence.
    pub descriptor_evidence: ArtifactEvidenceRef,
    /// Verified descriptor artifact bytes.
    pub descriptor_artifact: VerifiedRunArtifactBytes,
    /// Descriptor projection row.
    pub projection: FactDescriptorProjection,
    /// Descriptor-derived subject namespace hash.
    pub subject_namespace_hash: ContentDigest,
}

/// Builds a descriptor projection fixture from a fact descriptor.
pub fn fact_descriptor_projection_fixture_for_test(
    descriptor: mfm_facts::FactDescriptor,
) -> Result<FactDescriptorProjectionFixtureForTest> {
    let descriptor_bytes = mfm_facts::canonical_fact_descriptor_bytes(&descriptor)
        .map_err(|error| StoreError::Identity(error.to_string()))?
        .to_vec();
    let descriptor_hash = mfm_facts::fact_descriptor_hash(&descriptor)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let descriptor_artifact_id =
        ArtifactId::from_digest(descriptor_hash.algorithm(), *descriptor_hash.digest());
    let media_type = spec::MediaType::new("application/json")
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let descriptor_schema_id = mfm_facts::fact_descriptor_schema_id()
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let descriptor_evidence = ArtifactEvidenceRef {
        artifact_id: descriptor_artifact_id.clone(),
        digest: descriptor_hash.clone(),
        byte_len: descriptor_bytes.len() as u64,
        media_type: media_type.clone(),
        schema_id: Some(descriptor_schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::FactDescriptor,
    };
    let descriptor_requirement = events::EventArtifactRequirement {
        source: events::EventArtifactReferenceSource::FactDescriptor,
        artifact_id: descriptor_artifact_id.clone(),
        evidence_hash: descriptor_evidence.evidence_hash()?,
        digest: Some(descriptor_hash.clone()),
        byte_len: Some(descriptor_bytes.len() as u64),
        media_type: Some(media_type),
        schema_id: Some(descriptor_schema_id),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::FactDescriptor),
    };
    let descriptor_artifact = VerifiedRunArtifactBytes::new(
        descriptor_bytes.clone(),
        descriptor_evidence.clone(),
        &descriptor_requirement,
    )?;
    let subject_namespace_hash = mfm_facts::fact_subject_namespace_hash(&descriptor)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let projection = FactDescriptorProjection {
        descriptor_hash: descriptor_hash.clone(),
        descriptor_artifact_id: descriptor_artifact_id.clone(),
        descriptor_artifact_evidence: descriptor_evidence.clone(),
        fact_kind: descriptor.fact_kind().clone(),
        descriptor_schema_id: descriptor.descriptor_schema_id().clone(),
        subject_schema_id: descriptor.subject_schema_id().clone(),
        response_schema_id: descriptor.response_schema_id().clone(),
        fact_subject_namespace_hash: subject_namespace_hash.clone(),
    };
    Ok(FactDescriptorProjectionFixtureForTest {
        descriptor,
        descriptor_bytes,
        descriptor_hash,
        descriptor_artifact_id,
        descriptor_evidence,
        descriptor_artifact,
        projection,
        subject_namespace_hash,
    })
}

/// Input for building a projected fact record fixture.
#[derive(Debug, Clone)]
pub struct FactProjectionFixtureInputForTest {
    /// Producing run id.
    pub run_id: RunId,
    /// Producing stream sequence.
    pub source_seq: u64,
    /// Producing event ordinal.
    pub source_ordinal: u32,
    /// Fact-record event id.
    pub source_event_id: EventId,
    /// Producing node id.
    pub node_id: NodeId,
    /// Producing attempt id.
    pub attempt_id: AttemptId,
    /// Commit idempotency key for index rows.
    pub commit_id: CommitKey,
    /// Store commit ordering coordinate.
    pub store_commit_order: u64,
    /// Store-recorded timestamp.
    pub recorded_at: String,
    /// Source observation timestamp.
    pub observed_at: Option<String>,
    /// Fact visibility.
    pub visibility: mfm_facts::FactVisibility,
    /// Canonical subject value used by descriptor subject extractions.
    pub subject: CanonicalValue,
    /// Canonical response value used by descriptor result extractions.
    pub response: CanonicalValue,
    /// Optional request evidence pinned in the fact claim.
    pub request: Option<mfm_facts::FactRequestEvidence>,
    /// Response schema id.
    pub response_schema_id: SchemaId,
    /// Optional response artifact id. Defaults to the response content digest.
    pub response_artifact_id: Option<ArtifactId>,
    /// Fact producer provenance.
    pub producer: mfm_facts::FactProducerProvenance,
}

/// Input for a fact fixture that is appended through the typed store.
#[derive(Debug, Clone)]
pub struct FactRecordFixtureInputForTest {
    /// Store scope used to derive the source run identity.
    pub source_scope: StoreScopeId,
    /// Producing node id.
    pub node_id: NodeId,
    /// Producing attempt id.
    pub attempt_id: AttemptId,
    /// Commit idempotency key for the source fact commit.
    pub commit_id: CommitKey,
    /// Source observation timestamp.
    pub observed_at: Option<String>,
    /// Fact visibility.
    pub visibility: mfm_facts::FactVisibility,
    /// Canonical subject value used by descriptor subject extractions.
    pub subject: CanonicalValue,
    /// Canonical response value used by descriptor result extractions.
    pub response: CanonicalValue,
    /// Optional request evidence pinned in the fact claim.
    pub request: Option<mfm_facts::FactRequestEvidence>,
    /// Response schema id.
    pub response_schema_id: SchemaId,
    /// Optional response artifact id. Defaults to the response content digest.
    pub response_artifact_id: Option<ArtifactId>,
    /// Fact producer provenance.
    pub producer: mfm_facts::FactProducerProvenance,
}

/// Projected fact fixture built from descriptor, subject, and response values.
#[derive(Debug, Clone)]
pub struct FactProjectionFixtureForTest {
    /// Store-owned fact record projection.
    pub record: FactRecordProjection,
    /// Store-owned index projection when the fact is indexed.
    pub index: Option<FactIndexProjection>,
    /// Extracted index term projections when the fact is indexed.
    pub terms: Vec<FactIndexTermProjection>,
    /// Verified response artifact evidence.
    pub response_artifact_evidence: ArtifactEvidenceRef,
    /// Canonical response JSON bytes retained as the FactResponse artifact.
    pub response_bytes: Vec<u8>,
}

/// Builds a fact record, optional index row, and optional term rows for projection fixtures.
pub fn fact_projection_fixture_for_test(
    descriptor: &mfm_facts::FactDescriptor,
    descriptor_hash: ContentDigest,
    input: FactProjectionFixtureInputForTest,
) -> Result<FactProjectionFixtureForTest> {
    let claim_fixture = fact_claim_fixture_for_test(
        descriptor,
        &descriptor_hash,
        FactClaimFixtureInput {
            node_id: &input.node_id,
            visibility: input.visibility.clone(),
            observed_at: input.observed_at.as_deref(),
            request: input.request.as_ref(),
            response_schema_id: &input.response_schema_id,
            response_artifact_id: input.response_artifact_id.as_ref(),
            producer: &input.producer,
            subject_value: &input.subject,
            response_value: &input.response,
        },
    )?;
    let FactClaimFixtureForTest {
        subject_material,
        claim,
        response_artifact_evidence,
        response_bytes,
    } = claim_fixture;
    let fact_claim_id =
        mfm_facts::FactClaimId::new(input.run_id.clone(), input.source_seq, input.source_ordinal)
            .map_err(|error| StoreError::Identity(error.to_string()))?;
    let record = FactRecordProjection {
        fact_claim_id: fact_claim_id.clone(),
        source_event_id: input.source_event_id.clone(),
        source_run_id: input.run_id.clone(),
        source_seq: input.source_seq,
        source_ordinal: input.source_ordinal,
        node_id: input.node_id.clone(),
        attempt_id: input.attempt_id,
        response_artifact_evidence: Some(response_artifact_evidence.clone()),
        claim,
    };
    let Some(index) = FactIndexProjection::from_record_projection(
        &record,
        input.commit_id.clone(),
        input.store_commit_order,
        input.recorded_at.clone(),
    )?
    else {
        return Ok(FactProjectionFixtureForTest {
            record,
            index: None,
            terms: Vec::new(),
            response_artifact_evidence,
            response_bytes,
        });
    };
    let metadata = mfm_facts::FactExtractionMetadata::new(
        input.recorded_at,
        input.observed_at,
        input.store_commit_order,
    )
    .map_err(|error| StoreError::Identity(error.to_string()))?;
    let terms = mfm_facts::extract_terms_from_material(
        descriptor,
        &subject_material,
        &input.response,
        &metadata,
    )
    .map_err(|error| StoreError::Identity(error.to_string()))?
    .into_iter()
    .map(|term| {
        FactIndexTermProjection::from_extracted_term(&fact_claim_id, &descriptor_hash, &term)
    })
    .collect();
    Ok(FactProjectionFixtureForTest {
        record,
        index: Some(index),
        terms,
        response_artifact_evidence,
        response_bytes,
    })
}

#[derive(Debug, Clone)]
struct FactClaimFixtureForTest {
    subject_material: mfm_facts::FactSubjectMaterialV1,
    claim: mfm_facts::FactClaim,
    response_artifact_evidence: ArtifactEvidenceRef,
    response_bytes: Vec<u8>,
}

struct FactClaimFixtureInput<'a> {
    node_id: &'a NodeId,
    visibility: mfm_facts::FactVisibility,
    observed_at: Option<&'a str>,
    request: Option<&'a mfm_facts::FactRequestEvidence>,
    response_schema_id: &'a SchemaId,
    response_artifact_id: Option<&'a ArtifactId>,
    producer: &'a mfm_facts::FactProducerProvenance,
    subject_value: &'a CanonicalValue,
    response_value: &'a CanonicalValue,
}

fn fact_claim_fixture_for_test(
    descriptor: &mfm_facts::FactDescriptor,
    descriptor_hash: &ContentDigest,
    input: FactClaimFixtureInput<'_>,
) -> Result<FactClaimFixtureForTest> {
    let subject_material = mfm_facts::extract_subject_material(descriptor, input.subject_value)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let subject = mfm_facts::fact_subject_evidence_from_material(descriptor, &subject_material)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let response_canonical = CanonicalJsonBytes::from_value(input.response_value);
    let response_bytes = response_canonical.as_bytes().to_vec();
    let response_hash = response_canonical.content_digest();
    let response_artifact_id = input.response_artifact_id.cloned().unwrap_or_else(|| {
        ArtifactId::from_digest(response_hash.algorithm(), *response_hash.digest())
    });
    let response_artifact_evidence = ArtifactEvidenceRef {
        artifact_id: response_artifact_id.clone(),
        digest: response_hash.clone(),
        byte_len: response_bytes.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("json media type"),
        schema_id: Some(input.response_schema_id.clone()),
        semantic_type_id: None,
        producer_node_id: Some(input.node_id.clone()),
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::FactResponse,
    };
    let artifact_evidence_hash = response_artifact_evidence.evidence_hash()?;
    let claim = mfm_facts::FactClaim::new(mfm_facts::FactClaimParts {
        visibility: input.visibility,
        fact_kind: descriptor.fact_kind().clone(),
        fact_descriptor_hash: descriptor_hash.clone(),
        subject,
        observed_at: input.observed_at.map(str::to_owned),
        request: input.request.cloned(),
        response: mfm_facts::FactResponseEvidence::new(
            input.response_schema_id.clone(),
            response_hash,
            response_artifact_id,
            artifact_evidence_hash,
        ),
        producer: input.producer.clone(),
    })
    .map_err(|error| StoreError::Identity(error.to_string()))?;
    Ok(FactClaimFixtureForTest {
        subject_material,
        claim,
        response_artifact_evidence,
        response_bytes,
    })
}

/// One Platform holding fact to append into an in-memory store for report SelectHoldings tests.
#[derive(Debug, Clone)]
pub struct PlatformHoldingFactSeedForTest {
    /// Holding fact descriptor (BTC address balance or EVM native balance at cutover).
    pub descriptor: mfm_facts::FactDescriptor,
    /// Fact-record input appended through the typed store.
    pub input: FactRecordFixtureInputForTest,
}

/// Appends Platform holding facts through the typed run-store commit path.
///
/// This deliberately uses a small certified source-run spec, a real run-admission commit, a real
/// attempt-start commit, and a real `FactRecorded` terminal commit. The store therefore owns the
/// source stream coordinates, event id, artifact authority, projection rows, and
/// `StoreCommitOrder` just as it does for a production fact recorder.
pub async fn append_platform_holding_facts_for_test(
    store: &AsyncInMemoryRunStore,
    seeds: impl IntoIterator<Item = PlatformHoldingFactSeedForTest>,
) -> Result<Vec<FactProjectionFixtureForTest>> {
    let mut fixtures = Vec::new();
    for seed in seeds {
        let descriptor_fixture =
            fact_descriptor_projection_fixture_for_test(seed.descriptor.clone())?;
        let descriptor_hash = descriptor_fixture.descriptor_hash.clone();
        let source_spec =
            fact_source_spec_for_test(seed.input.node_id.clone(), descriptor_hash.clone());
        let source_spec_hash = source_spec
            .spec_hash()
            .map_err(|error| StoreError::Identity(error.to_string()))?;
        let identity_material = events::RunIdentityMaterialV1 {
            certified_spec_hash: source_spec_hash.clone(),
            store_scope_id: seed.input.source_scope.clone(),
            invocation_key_digest: ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(seed.input.commit_id.as_str().as_bytes()),
            ),
        };
        let source_run_id = identity_material
            .derive_run_id()
            .map_err(|error| StoreError::Identity(error.to_string()))?;
        let claim_fixture = fact_claim_fixture_for_test(
            &seed.descriptor,
            &descriptor_hash,
            FactClaimFixtureInput {
                node_id: &seed.input.node_id,
                visibility: seed.input.visibility.clone(),
                observed_at: seed.input.observed_at.as_deref(),
                request: seed.input.request.as_ref(),
                response_schema_id: &seed.input.response_schema_id,
                response_artifact_id: seed.input.response_artifact_id.as_ref(),
                producer: &seed.input.producer,
                subject_value: &seed.input.subject,
                response_value: &seed.input.response,
            },
        )?;

        let (spec_bytes, spec_evidence) = source_spec_artifact_for_test(&source_spec);
        let (certificate_bytes, certificate_evidence) = source_certificate_artifact_for_test();
        let descriptor_run_artifact =
            run_artifact_ref_from_store_artifact_for_test(&descriptor_fixture.descriptor_evidence);
        let run_admitted = events::KernelEventPayload::RunAdmitted(Box::new(events::RunAdmitted {
            run_id: source_run_id.clone(),
            identity_material,
            entry_point: events::EntryPointLaunchEvidence {
                resolved_op_id: events::EntryPointOpId::new("mfm.test.fact_fixture.record.v1")
                    .map_err(|error| StoreError::Identity(error.to_string()))?,
                entry_point_registry_digest: ContentDigest::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    sha256_digest_bytes(b"mfm.test.fact_fixture.registry.v1"),
                ),
            },
            spec_hash: source_spec_hash.clone(),
            spec_artifact: run_artifact_ref_from_store_artifact_for_test(&spec_evidence),
            certificate_artifact: run_artifact_ref_from_store_artifact_for_test(
                &certificate_evidence,
            ),
            config_artifacts: Vec::new(),
            fact_descriptor_artifacts: vec![descriptor_run_artifact],
            spec_version: SpecVersion::new("mfm.typed.execution_spec.v1")
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            lowering_version: LoweringVersion::new("mfm.typed.lowering.v1")
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            public_output_schema_id: SchemaId::new(
                "mfm.test.fact_fixture.public_output",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                DigestBytes::from_array([0x71; 32]),
            )
            .map_err(|error| StoreError::Identity(error.to_string()))?,
            saga_policy_digest: SagaPolicySpec::NoSideEffects
                .saga_policy_digest()
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            descriptor_identities: Vec::new(),
            runner_executables: Vec::new(),
            adapter_executables: Vec::new(),
            admitted_binding_digest: ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(b"mfm.test.fact_fixture.bindings.v1"),
            ),
            canonicalizer_identity: CanonicalizerIdentity::new("mfm.jcs.v1")
                .map_err(|error| StoreError::Identity(error.to_string()))?,
            seed_cells: Vec::new(),
        }));
        let authority = CertifiedRunStoreAuthority::from_spec(source_run_id.clone(), &source_spec)?;
        let admission_request = CommitRequest::from_payloads(
            source_run_id.clone(),
            store.expected_next_seq(&source_run_id).await?,
            CommitKey::new(format!("{}-admission", seed.input.commit_id.as_str()))?,
            vec![run_admitted],
            vec![
                spec_evidence.clone(),
                certificate_evidence.clone(),
                descriptor_fixture.descriptor_evidence.clone(),
            ],
            CommitPreconditions {
                required_run_state: RequiredRunState::Absent,
                certified_run_authority: Some(authority.clone()),
                ..CommitPreconditions::default()
            },
        )?;
        let admission_plan = PreparedCommit::<RunAdmission>::new(
            admission_request,
            CommitArtifactEvidenceSet::new(
                vec![
                    spec_evidence.clone(),
                    certificate_evidence.clone(),
                    descriptor_fixture.descriptor_evidence.clone(),
                ],
                vec![
                    spec_evidence.clone(),
                    certificate_evidence.clone(),
                    descriptor_fixture.descriptor_evidence.clone(),
                ],
            )?,
        )?;
        let admission_bundle = PreparedCommitBundle::new(
            admission_plan.into(),
            vec![
                PreparedArtifactBytes::new(spec_bytes, spec_evidence.clone())?,
                PreparedArtifactBytes::new(certificate_bytes, certificate_evidence.clone())?,
                PreparedArtifactBytes::new(
                    descriptor_fixture.descriptor_bytes.clone(),
                    descriptor_fixture.descriptor_evidence.clone(),
                )?,
            ],
            Vec::new(),
        )?;
        append_batch_for_test(
            store
                .append_prepared_commit_bundle(admission_bundle)
                .await?,
        )?;

        let start_attempt_id = seed.input.attempt_id.clone();
        let node_id = seed.input.node_id.clone();
        let node = source_spec
            .nodes
            .first()
            .ok_or_else(|| StoreError::Event("source fact spec has no node".to_owned()))?;
        let start_request = CommitRequest::from_payloads(
            source_run_id.clone(),
            store.expected_next_seq(&source_run_id).await?,
            CommitKey::new(format!("{}-attempt-start", seed.input.commit_id.as_str()))?,
            vec![events::KernelEventPayload::StateAttemptStarted(
                events::StateAttemptStarted {
                    spec_hash: source_spec_hash.clone(),
                    node_id: node_id.clone(),
                    attempt_id: start_attempt_id.clone(),
                    attempt_no: 1,
                    state_kind: node.state_kind.clone(),
                    state_version: node.state_version.clone(),
                },
            )],
            Vec::new(),
            CommitPreconditions {
                required_run_state: RequiredRunState::NotCompleted,
                ..CommitPreconditions::default()
            },
        )?;
        let start_plan = PreparedCommit::<StateAttemptStarted>::new(
            start_request,
            CommitArtifactEvidenceSet::empty(),
        )?;
        store
            .append_prepared_commit_bundle(PreparedCommitBundle::without_artifacts(
                start_plan.into(),
            )?)
            .await?;

        let fact_payload = events::KernelEventPayload::FactRecorded(events::FactRecorded {
            spec_hash: source_spec_hash,
            node_id: node_id.clone(),
            attempt_id: start_attempt_id.clone(),
            claim: claim_fixture.claim.clone(),
        });
        let response_evidence = claim_fixture.response_artifact_evidence.clone();
        let fact_request = CommitRequest::from_payloads(
            source_run_id.clone(),
            store.expected_next_seq(&source_run_id).await?,
            seed.input.commit_id.clone(),
            vec![fact_payload],
            vec![response_evidence.clone()],
            CommitPreconditions {
                required_run_state: RequiredRunState::NotCompleted,
                certified_run_authority: Some(authority),
                ..CommitPreconditions::default()
            },
        )?;
        let fact_plan = PreparedCommit::<AttemptTerminal>::new(
            fact_request,
            CommitArtifactEvidenceSet::new(
                vec![response_evidence.clone()],
                vec![response_evidence.clone()],
            )?,
        )?;
        let fact_batch = append_batch_for_test(
            store
                .append_prepared_commit_bundle(PreparedCommitBundle::new(
                    fact_plan.into(),
                    vec![PreparedArtifactBytes::new(
                        claim_fixture.response_bytes.clone(),
                        response_evidence,
                    )?],
                    Vec::new(),
                )?)
                .await?,
        )?;
        let fact_event = fact_batch
            .events()
            .iter()
            .find(|event| matches!(event.payload(), events::KernelEventPayload::FactRecorded(_)))
            .ok_or_else(|| {
                StoreError::Event("fact append did not record FactRecorded".to_owned())
            })?;
        let fixture = fact_projection_fixture_for_test(
            &seed.descriptor,
            descriptor_hash,
            FactProjectionFixtureInputForTest {
                run_id: source_run_id,
                source_seq: fact_event.seq().as_u64(),
                source_ordinal: fact_event.ordinal().as_u32(),
                source_event_id: fact_event.event_id().clone(),
                node_id,
                attempt_id: start_attempt_id,
                commit_id: fact_batch.commit_key().clone(),
                store_commit_order: fact_batch.store_commit_order().as_u64(),
                recorded_at: "1970-01-01T00:00:00Z".to_owned(),
                observed_at: seed.input.observed_at,
                visibility: seed.input.visibility,
                subject: seed.input.subject,
                response: seed.input.response,
                request: seed.input.request,
                response_schema_id: seed.input.response_schema_id,
                response_artifact_id: seed.input.response_artifact_id,
                producer: seed.input.producer,
            },
        )?;
        fixtures.push(fixture);
    }
    Ok(fixtures)
}

fn append_batch_for_test(outcome: CommitOutcome) -> Result<CommittedBatch> {
    match outcome {
        CommitOutcome::Appended(batch) | CommitOutcome::Idempotent(batch) => Ok(batch),
        CommitOutcome::ExecutionClaimBusy(_) => Err(StoreError::Event(
            "fixture append unexpectedly hit an execution claim".to_owned(),
        )),
        CommitOutcome::AdmissionBlocked(_) => Err(StoreError::Event(
            "fixture append unexpectedly blocked admission".to_owned(),
        )),
    }
}

fn fact_source_spec_for_test(
    node_id: NodeId,
    descriptor_hash: ContentDigest,
) -> TypedExecutionSpec {
    let planning_lineage = spec::PlanningLineage {
        active_operation_instances: Vec::new(),
        completed_operation_frames: Vec::new(),
        lineage_digest: ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.test.fact_fixture.lineage.v1"),
        ),
    };
    let state_kind = StateKind::new(
        "mfm.test",
        "fact_fixture_state",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.test.fact_fixture_state"),
    )
    .expect("fact fixture state kind");
    let state_version =
        StateVersion::new("mfm.test.fact_fixture_state.v1").expect("fact fixture state version");
    let descriptor_id = DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.test.fact_fixture_state_descriptor"),
    );
    let config_schema_id = SchemaId::new(
        "mfm.test.fact_fixture.config",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.test.fact_fixture.config"),
    )
    .expect("fact fixture config schema");
    let config_ref = spec::ConfigRef {
        schema_id: config_schema_id.clone(),
        artifact_id: ArtifactId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.test.fact_fixture.config.artifact"),
        ),
        digest: ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"{}"),
        ),
        byte_len: 2,
        media_type: spec::MediaType::new("application/json").expect("fact fixture media type"),
    };
    let input_schema_id = SchemaId::new(
        "mfm.test.fact_fixture.input",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.test.fact_fixture.input"),
    )
    .expect("fact fixture config schema");
    let input_bindings = spec::InputBindingSpec {
        input_schema_id: input_schema_id.clone(),
        input_descriptor_id: DescriptorId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.test.fact_fixture.input_descriptor"),
        ),
        root: spec::InputBindingNodeSpec::Unit,
        digest: ContentDigest::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.test.fact_fixture.input_binding"),
        ),
    };
    let output_schema_id = SchemaId::new(
        "mfm.test.fact_fixture.output",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.test.fact_fixture.output"),
    )
    .expect("fact fixture input schema");
    let output_semantic_type_id = SemanticTypeId::new(
        "mfm.test",
        "fact_fixture_output",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.test.fact_fixture_output"),
    )
    .expect("fact fixture output schema");
    let effect_kind = EffectKind::new(
        "mfm.test",
        "fact_fixture_read",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.test.fact_fixture_read"),
    )
    .expect("fact fixture effect kind");
    let capabilities = CapabilitySetDescriptor::new(Vec::new()).expect("empty capabilities");
    let public_schema_id = SchemaId::new(
        "mfm.test.fact_fixture.public_output",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"mfm.test.fact_fixture.public_output"),
    )
    .expect("fact fixture public schema");
    let renderer_descriptor = RendererDescriptorIdentity {
        descriptor_id: DescriptorId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.test.fact_fixture.renderer"),
        ),
        renderer_kind: RendererKind::new("public-output/json").expect("renderer kind"),
        renderer_version: RendererVersion::new("mfm.test.fact_fixture_renderer.v1")
            .expect("renderer version"),
        public_schema_id: public_schema_id.clone(),
        canonicalizer_identity: CanonicalizerIdentity::new("mfm.jcs.v1").expect("canonicalizer"),
    };
    let output_cell = CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(node_id.as_str().as_bytes()),
    );
    let node = spec::NodeSpec {
        node_id,
        stable_key: spec::StableAuthorKey::new("fact-fixture").expect("stable key"),
        scope_id: ScopeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"mfm.test.fact_fixture.scope"),
        ),
        state_kind: state_kind.clone(),
        state_version: state_version.clone(),
        descriptor_id: descriptor_id.clone(),
        context: spec::NodeContextSpec::no_context(),
        config_ref: config_ref.clone(),
        input_bindings: input_bindings.clone(),
        output_cell,
        effect_kind: effect_kind.clone(),
        capability_bindings: capabilities.clone(),
        adapter_bindings: Vec::new(),
        fact_descriptor_allowlist: vec![spec::FactDescriptorRef { descriptor_hash }],
        side_effect: None,
        framework: None,
        planning_lineage: planning_lineage.clone(),
        deterministic_predecessors: Vec::new(),
    };
    TypedExecutionSpec::new(spec::TypedExecutionSpecParts {
        authoring: spec::AuthoringProvenance::StateComposition {
            descriptor: spec::CompositionDescriptor {
                descriptor_id: DescriptorId::from_digest(
                    DigestAlgorithm::Sha256JcsV1,
                    sha256_digest_bytes(b"mfm.test.fact_fixture.composition"),
                ),
                name: "mfm.test.fact_fixture".to_owned(),
                version: "mfm.test.fact_fixture.v1".to_owned(),
            },
            config_hash: ContentDigest::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(b"mfm.test.fact_fixture.composition_config"),
            ),
        },
        saga: SagaPolicySpec::NoSideEffects,
        contexts: Vec::new(),
        scopes: Vec::new(),
        seeds: Vec::new(),
        descriptor_identities: vec![
            DescriptorIdentity::State(Box::new(StateDescriptorIdentity {
                descriptor_id,
                name: "mfm.test.fact_fixture_state".to_owned(),
                state_kind,
                state_version,
                context: spec::StateContextDescriptorSpec::no_context(),
                input_context: spec::StateInputContextContractSpec::no_context(),
                output_context: spec::StateOutputContextContractSpec::no_context(),
                config_schema_id,
                input_schema_id,
                output_schema_id,
                output_semantic_type_id,
                effect_kind,
                effect_class: "read".to_owned(),
                effect_name: "fact_fixture_read".to_owned(),
                effect_version: EffectVersion::new("mfm.test.fact_fixture_read.v1")
                    .expect("effect version"),
                capabilities,
                emitted_fact_descriptors: Vec::new(),
                runner: "mfm.test.fact_fixture_runner".to_owned(),
                side_effect_contract_digest: None,
            })),
            DescriptorIdentity::Renderer(Box::new(renderer_descriptor.clone())),
        ],
        config_refs: vec![config_ref],
        nodes: vec![node],
        remediations: BTreeMap::new(),
        cells: Vec::new(),
        value_lineages: Vec::new(),
        planning_lineage: Vec::new(),
        public_outputs: spec::PublicOutputSpec {
            public_schema_id,
            outputs: Vec::new(),
            renderer_descriptor,
        },
    })
    .expect("fact fixture source spec")
}

fn source_spec_artifact_for_test(
    source_spec: &TypedExecutionSpec,
) -> (Vec<u8>, ArtifactEvidenceRef) {
    let bytes = source_spec
        .canonical_json()
        .expect("fact fixture source spec canonical json")
        .to_vec();
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&bytes));
    let evidence = ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("fact fixture media type"),
        schema_id: Some(
            spec::typed_execution_spec_schema_id().expect("typed execution spec schema"),
        ),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedExecutionSpec,
    };
    (bytes, evidence)
}

fn source_certificate_artifact_for_test() -> (Vec<u8>, ArtifactEvidenceRef) {
    let bytes = b"{}".to_vec();
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(&bytes));
    let evidence = ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest,
        byte_len: bytes.len() as u64,
        media_type: spec::MediaType::new("application/json").expect("fact fixture media type"),
        schema_id: Some(
            SchemaId::new(
                "mfm.test.fact_fixture.certificate",
                "1",
                DigestAlgorithm::Sha256JcsV1,
                sha256_digest_bytes(b"mfm.test.fact_fixture.certificate"),
            )
            .expect("fact fixture certificate schema"),
        ),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedSpecCertificate,
    };
    (bytes, evidence)
}

/// Polls an in-memory store future that is expected to complete immediately.
pub fn poll_ready_store_future_for_test<T, E>(
    mut future: AsyncStoreFuture<'_, T, E>,
) -> std::result::Result<T, E> {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    match Future::poll(future.as_mut(), &mut context) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("async in-memory store future should be ready"),
    }
}

/// Builds a prepared commit bundle that treats all admitted artifact evidence as pre-existing.
pub fn prepared_commit_bundle_from_plan(plan: PreparedCommitPlan) -> Result<PreparedCommitBundle> {
    let existing = plan
        .admitted_artifacts()
        .iter()
        .map(|evidence| {
            Ok(ExistingArtifactAdmission::new(
                evidence.artifact_id.clone(),
                evidence.evidence_hash()?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    PreparedCommitBundle::new(plan, Vec::new(), existing)
}

/// Builds a prepared commit plan for test fixtures from typed payload purpose.
///
/// This helper exists for tests that construct synthetic event streams across several commit
/// purposes. It keeps production commit constructors authoritative and rejects fixture attempts
/// that need manual-resolution or saga-terminal proof authority.
pub fn prepared_commit_plan_for_test(
    request: CommitRequest,
    admitted_artifacts: Vec<ArtifactEvidenceRef>,
) -> Result<PreparedCommitPlan> {
    let artifacts =
        CommitArtifactEvidenceSet::new(request.required_artifacts().to_vec(), admitted_artifacts)?;
    if request
        .payloads()
        .iter()
        .all(|payload| matches!(payload, events::KernelEventPayload::RunAdmitted(_)))
    {
        let mut preconditions = request.preconditions().clone();
        preconditions.required_run_state = RequiredRunState::Absent;
        let request = request.with_preconditions(preconditions);
        return PreparedCommit::<RunAdmission>::new(request, artifacts)
            .map(PreparedCommitPlan::from);
    }
    if request
        .payloads()
        .iter()
        .all(|payload| matches!(payload, events::KernelEventPayload::StateAttemptStarted(_)))
    {
        let mut preconditions = request.preconditions().clone();
        preconditions.required_run_state = RequiredRunState::NotCompleted;
        let request = request.with_preconditions(preconditions);
        return PreparedCommit::<StateAttemptStarted>::new(request, artifacts)
            .map(PreparedCommitPlan::from);
    }
    if request.payloads().iter().any(is_manual_resolution_payload) {
        return Err(StoreError::InvalidPreparedCommitPurpose {
            purpose: "manual_resolution",
            message: "manual resolution commits requires verified manual resolution proof".into(),
        });
    }
    if request.payloads().iter().any(is_saga_terminal_payload) {
        return Err(StoreError::InvalidPreparedCommitPurpose {
            purpose: "saga_terminal",
            message: "saga terminal commits requires SagaTerminalProof".into(),
        });
    }
    if request.payloads().iter().any(is_run_completed_payload) {
        return PreparedCommit::<AttemptTerminal>::new(request, artifacts)
            .map(PreparedCommitPlan::from);
    }
    if request
        .payloads()
        .iter()
        .any(is_side_effect_terminal_payload)
    {
        return PreparedCommit::<SideEffectTerminal>::new(request, artifacts)
            .map(PreparedCommitPlan::from);
    }
    if request
        .payloads()
        .iter()
        .any(|payload| payload.side_effect_ref().is_some())
    {
        return PreparedCommit::<SideEffectProgress>::new(request, artifacts)
            .map(PreparedCommitPlan::from);
    }
    if request.payloads().iter().any(is_retention_payload) {
        return PreparedCommit::<Retention>::new(request, artifacts).map(PreparedCommitPlan::from);
    }
    PreparedCommit::<AttemptTerminal>::new(request, artifacts).map(PreparedCommitPlan::from)
}

fn is_manual_resolution_payload(payload: &events::KernelEventPayload) -> bool {
    matches!(
        payload,
        events::KernelEventPayload::ManualResolutionRecorded(_)
    )
}

fn is_saga_terminal_payload(payload: &events::KernelEventPayload) -> bool {
    matches!(
        payload,
        events::KernelEventPayload::RunCompleted(events::RunCompleted {
            outcome: events::RunCompletionOutcome::Compensated
                | events::RunCompletionOutcome::ManuallyResolved
                | events::RunCompletionOutcome::FailedWithoutAcdcClaim,
            ..
        })
    )
}

fn is_run_completed_payload(payload: &events::KernelEventPayload) -> bool {
    matches!(payload, events::KernelEventPayload::RunCompleted(_))
}

fn is_side_effect_terminal_payload(payload: &events::KernelEventPayload) -> bool {
    matches!(
        payload,
        events::KernelEventPayload::SideEffectNotSubmittedProven(_)
            | events::KernelEventPayload::SideEffectSubmissionObserved(_)
            | events::KernelEventPayload::SideEffectSubmissionUnknown(_)
            | events::KernelEventPayload::SideEffectReceiptObserved(_)
            | events::KernelEventPayload::SideEffectConfirmationObserved(_)
            | events::KernelEventPayload::SideEffectAmbiguous(_)
            | events::KernelEventPayload::SideEffectFailed(_)
            | events::KernelEventPayload::ResourceLaneReleaseIntent(_)
            | events::KernelEventPayload::ResourceLaneReleased(_)
    )
}

fn is_retention_payload(payload: &events::KernelEventPayload) -> bool {
    matches!(
        payload,
        events::KernelEventPayload::RetentionRefsAppended(_)
            | events::KernelEventPayload::RetentionManifestProjected(_)
    )
}

/// Appends a started or terminal test commit through the typed prepared-commit surface.
///
/// This panics on fixture construction or store errors, matching normal integration-test helper
/// behavior.
pub async fn append_started_or_terminal_commit_for_test<S>(
    store: &S,
    request: CommitRequest,
) -> CommitOutcome
where
    S: RunEventStore + ?Sized,
{
    let admitted_artifacts = request.required_artifacts().to_vec();
    let artifacts =
        CommitArtifactEvidenceSet::new(request.required_artifacts().to_vec(), admitted_artifacts)
            .expect("artifact evidence set");
    let plan = if request
        .payloads()
        .iter()
        .all(|payload| matches!(payload, events::KernelEventPayload::StateAttemptStarted(_)))
    {
        PreparedCommit::<StateAttemptStarted>::new(request, artifacts)
            .expect("prepared attempt-start commit")
            .into()
    } else {
        PreparedCommit::<AttemptTerminal>::new(request, artifacts)
            .expect("prepared attempt-terminal commit")
            .into()
    };
    let bundle = prepared_commit_bundle_from_plan(plan).expect("prepared commit bundle");
    store
        .append_prepared_commit_bundle(bundle)
        .await
        .unwrap_or_else(|error| panic!("append typed commit: {error}"))
}

/// Appends a started then interrupted attempt for status/history integration fixtures.
pub async fn append_interrupted_attempt_for_test<S>(
    store: &S,
    run_id: &RunId,
    spec_hash: &SpecHash,
    node: &spec::NodeSpec,
    attempt_id: &AttemptId,
) where
    S: RunEventStore + ?Sized,
{
    let start = CommitRequest::from_payloads(
        run_id.clone(),
        store
            .expected_next_seq(run_id)
            .await
            .unwrap_or_else(|error| panic!("expected next seq: {error}")),
        CommitKey::new(format!(
            "mfm-test-interrupted-attempt-start-{}",
            attempt_id.as_str()
        ))
        .expect("commit key"),
        vec![events::KernelEventPayload::StateAttemptStarted(
            events::StateAttemptStarted {
                spec_hash: spec_hash.clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                attempt_no: 1,
                state_kind: node.state_kind.clone(),
                state_version: node.state_version.clone(),
            },
        )],
        Vec::new(),
        CommitPreconditions {
            required_run_state: RequiredRunState::NotCompleted,
            required_cell_states: vec![CellStatePrecondition {
                cell_id: node.output_cell.clone(),
                required: RequiredCellState::Absent,
            }],
            ..CommitPreconditions::default()
        },
    )
    .expect("attempt start request");
    append_started_or_terminal_commit_for_test(store, start).await;

    let interrupted = CommitRequest::from_payloads(
        run_id.clone(),
        store
            .expected_next_seq(run_id)
            .await
            .unwrap_or_else(|error| panic!("expected next seq: {error}")),
        CommitKey::new(format!(
            "mfm-test-interrupted-attempt-terminal-{}",
            attempt_id.as_str()
        ))
        .expect("commit key"),
        vec![events::KernelEventPayload::StateAttemptInterrupted(
            events::StateAttemptInterrupted {
                spec_hash: spec_hash.clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
            },
        )],
        Vec::new(),
        CommitPreconditions {
            required_run_state: RequiredRunState::NotCompleted,
            required_cell_states: vec![CellStatePrecondition {
                cell_id: node.output_cell.clone(),
                required: RequiredCellState::Absent,
            }],
            ..CommitPreconditions::default()
        },
    )
    .expect("attempt interrupted request");
    append_started_or_terminal_commit_for_test(store, interrupted).await;
}

/// Asserts the shared execution-claim token lifecycle for an [`ExecutionClaimStore`].
pub fn assert_execution_claim_token_lifecycle_for_test<'a, S>(
    store: &'a S,
    run_id: &'a RunId,
    holder: AdmissionToken,
    other: AdmissionToken,
) -> AsyncStoreFuture<'a, (), S::Error>
where
    S: ExecutionClaimStore + Sync + 'a,
{
    Box::pin(async move {
        let identity = run_identity_material_for_test(
            SpecHash::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                fixed_digest_bytes_for_test(0x51),
            ),
            "51515151515151515151515151515151",
        );
        let scope = ExecutionClaimScope::from_run_identity_material(&identity);
        let admitted = store
            .acquire_execution_claim(&scope, run_id, holder.clone())
            .await?;
        let NowaitSkipAdmissionResult::Admitted(first_lease) = admitted else {
            panic!("first execution claim should be admitted");
        };
        assert_eq!(first_lease.token, holder);
        assert!(matches!(
            store.execution_claim_status(&scope).await?,
            ExecutionClaimStatus::Live(status)
                if status.token == first_lease.token && status.holder_run_id == *run_id
        ));

        let busy = store
            .acquire_execution_claim(&scope, run_id, other.clone())
            .await?;
        let NowaitSkipAdmissionResult::Busy(busy) = busy else {
            panic!("second execution claim should be busy");
        };
        assert_eq!(
            busy.holder.expect("busy holder lease").token,
            first_lease.token
        );

        assert!(store
            .renew_execution_claim(&scope, run_id, &other)
            .await?
            .is_none());
        assert!(
            !store
                .release_execution_claim(&scope, run_id, &other)
                .await?
        );

        let renewed = store
            .renew_execution_claim(&scope, run_id, &first_lease.token)
            .await?
            .expect("matching token returns lease");
        assert_eq!(renewed.token, first_lease.token);
        assert!(renewed.lease_expires_at_unix_ms >= first_lease.lease_expires_at_unix_ms);

        assert!(
            store
                .release_execution_claim(&scope, run_id, &renewed.token)
                .await?
        );
        assert!(matches!(
            store.execution_claim_status(&scope).await?,
            ExecutionClaimStatus::Unclaimed
        ));
        assert!(matches!(
            store.acquire_execution_claim(&scope, run_id, other).await?,
            NowaitSkipAdmissionResult::Admitted(_)
        ));
        Ok(())
    })
}

/// Returns deterministic digest bytes made from one repeated byte.
pub fn fixed_digest_bytes_for_test(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

macro_rules! fixed_digest_id_for_test {
    ($(#[$meta:meta])* $name:ident -> $ty:ty) => {
        $(#[$meta])*
        pub fn $name(byte: u8) -> $ty {
            <$ty>::from_digest(
                DigestAlgorithm::Sha256JcsV1,
                fixed_digest_bytes_for_test(byte),
            )
        }
    };
}

fixed_digest_id_for_test! {
    /// Returns a deterministic content digest made from one repeated digest byte.
    fixed_content_digest_for_test -> ContentDigest
}

fixed_digest_id_for_test! {
    /// Returns a deterministic spec hash made from one repeated digest byte.
    fixed_spec_hash_for_test -> SpecHash
}

fixed_digest_id_for_test! {
    /// Returns a deterministic artifact id made from one repeated digest byte.
    fixed_artifact_id_for_test -> ArtifactId
}

fixed_digest_id_for_test! {
    /// Returns a deterministic attempt id made from one repeated digest byte.
    fixed_attempt_id_for_test -> AttemptId
}

fixed_digest_id_for_test! {
    /// Returns a deterministic node id made from one repeated digest byte.
    fixed_node_id_for_test -> NodeId
}

fixed_digest_id_for_test! {
    /// Returns a deterministic cell id made from one repeated digest byte.
    fixed_cell_id_for_test -> CellId
}

fixed_digest_id_for_test! {
    /// Returns a deterministic scope id made from one repeated digest byte.
    fixed_scope_id_for_test -> ScopeId
}

fixed_digest_id_for_test! {
    /// Returns a deterministic event id made from one repeated digest byte.
    fixed_event_id_for_test -> EventId
}

fixed_digest_id_for_test! {
    /// Returns a deterministic descriptor id made from one repeated digest byte.
    fixed_descriptor_id_for_test -> DescriptorId
}

/// Returns the persisted event id derived from store envelope inputs.
pub fn event_id_for_envelope_inputs_for_test(
    run_id: &RunId,
    seq: StreamSeq,
    ordinal: CommitOrdinal,
    event_schema_id: &SchemaId,
    payload_hash: &ContentDigest,
) -> EventId {
    derive_event_id(run_id, seq, ordinal, event_schema_id, payload_hash).expect("event id")
}

/// Builds a validated persisted event envelope from a typed payload.
///
/// Callers must supply the store-owned commit order explicitly. Do not derive it from run-local
/// `seq`; multi-run LWW tests must use real appends or intentional distinct orders.
pub fn persisted_kernel_event_envelope_for_test(
    run_id: &RunId,
    seq: u64,
    store_commit_order: u64,
    commit_key: CommitKey,
    payload: events::KernelEventPayload,
) -> KernelEventEnvelope {
    persisted_kernel_event_envelope_with_ordinal_for_test(
        run_id,
        seq,
        store_commit_order,
        0,
        commit_key,
        payload,
    )
}

/// Builds a validated persisted event envelope from a typed payload and explicit commit ordinal.
pub fn persisted_kernel_event_envelope_with_ordinal_for_test(
    run_id: &RunId,
    seq: u64,
    store_commit_order: u64,
    ordinal: u32,
    commit_key: CommitKey,
    payload: events::KernelEventPayload,
) -> KernelEventEnvelope {
    let seq = StreamSeq::new(seq).expect("stream seq");
    let store_commit_order = StoreCommitOrder::new(store_commit_order);
    let ordinal = CommitOrdinal::new(ordinal);
    let payload_hash = payload_canonical_json(&payload)
        .expect("payload canonical")
        .content_digest();
    let event_schema_id = payload.event_schema_id().expect("event schema");
    let event_id = event_id_for_envelope_inputs_for_test(
        run_id,
        seq,
        ordinal,
        &event_schema_id,
        &payload_hash,
    );
    let logical_key =
        derive_logical_key(run_id, seq, ordinal, &payload, &payload_hash).expect("logical key");
    KernelEventEnvelope::from_persisted_record(PersistedKernelEventRecord {
        event_id,
        event_schema_id,
        run_id: run_id.clone(),
        seq,
        store_commit_order,
        ordinal,
        spec_hash: payload_spec_hash(&payload),
        commit_key,
        logical_key,
        payload_hash,
        payload,
    })
    .expect("persisted envelope")
}

/// Returns a deterministic schema id in version 1.
pub fn fixed_schema_id_for_test(name: &str, byte: u8) -> SchemaId {
    SchemaId::new(
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        fixed_digest_bytes_for_test(byte),
    )
    .expect("schema id")
}

/// Returns a deterministic semantic type id in the `mfm.test` namespace.
pub fn fixed_semantic_type_id_for_test(name: &str, byte: u8) -> SemanticTypeId {
    SemanticTypeId::new(
        "mfm.test",
        name,
        "1",
        DigestAlgorithm::Sha256JcsV1,
        fixed_digest_bytes_for_test(byte),
    )
    .expect("semantic id")
}

/// Returns a deterministic state kind in the `mfm.test` namespace.
pub fn fixed_state_kind_for_test(byte: u8) -> StateKind {
    StateKind::new(
        "mfm.test",
        "state",
        DigestAlgorithm::Sha256JcsV1,
        fixed_digest_bytes_for_test(byte),
    )
    .expect("state kind")
}

/// Returns a deterministic capability kind in the `mfm.test` namespace.
pub fn fixed_capability_kind_for_test(byte: u8) -> CapabilityKind {
    CapabilityKind::new(
        "mfm.test",
        "capability",
        DigestAlgorithm::Sha256JcsV1,
        fixed_digest_bytes_for_test(byte),
    )
    .expect("capability kind")
}

/// Returns a deterministic adapter kind in the `mfm.test` namespace.
pub fn fixed_adapter_kind_for_test(byte: u8) -> AdapterKind {
    AdapterKind::new(
        "mfm.test",
        "adapter",
        DigestAlgorithm::Sha256JcsV1,
        fixed_digest_bytes_for_test(byte),
    )
    .expect("adapter kind")
}

/// Returns a parsed test media type.
pub fn media_type_for_test(value: &str) -> spec::MediaType {
    spec::MediaType::new(value).expect("media type")
}

/// Returns deterministic artifact bytes for tests that persist real blobs.
pub fn artifact_bytes_for_test(byte: u8) -> Vec<u8> {
    vec![byte; 128]
}

/// Returns the content digest for deterministic artifact bytes.
pub fn artifact_content_digest_for_test(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(&artifact_bytes_for_test(byte)),
    )
}

/// Finds deterministic artifact bytes by content digest.
pub fn artifact_bytes_for_digest_for_test(digest: &ContentDigest) -> Option<Vec<u8>> {
    (u8::MIN..=u8::MAX)
        .map(artifact_bytes_for_test)
        .find(|bytes| {
            ContentDigest::from_digest(DigestAlgorithm::Sha256JcsV1, sha256_digest_bytes(bytes))
                == *digest
        })
}

/// Builds run-admission artifact evidence from stored artifact evidence.
pub fn run_artifact_ref_from_store_artifact_for_test(
    artifact: &ArtifactEvidenceRef,
) -> events::RunArtifactEvidenceRef {
    events::RunArtifactEvidenceRef {
        artifact_id: artifact.artifact_id.clone(),
        role: artifact.artifact_role,
        schema_id: artifact.schema_id.clone(),
        semantic_type_id: artifact.semantic_type_id.clone(),
        content_digest: artifact.digest.clone(),
        evidence_hash: artifact
            .evidence_hash()
            .expect("test store artifact evidence hash"),
        byte_len: artifact.byte_len,
        media_type: artifact.media_type.clone(),
    }
}

/// Builds run identity material for a deterministic test store scope suffix.
pub fn run_identity_material_for_test(
    certified_spec_hash: SpecHash,
    store_scope_hex: &str,
) -> events::RunIdentityMaterialV1 {
    events::RunIdentityMaterialV1 {
        certified_spec_hash,
        store_scope_id: StoreScopeId::new(format!("{}{}", StoreScopeId::PREFIX, store_scope_hex))
            .expect("test store scope"),
        invocation_key_digest: invocation_key_digest_for_test(store_scope_hex),
    }
}

fn invocation_key_digest_for_test(store_scope_hex: &str) -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("mfm.store.test.invocation:{store_scope_hex}").as_bytes()),
    )
}

/// Builds side-effect terminal policies for projected side-effect ledgers in one run.
pub fn terminal_policies_for_projection_for_test(
    projection: &ProjectionSnapshot,
    run_id: &RunId,
    terminal_policy: SideEffectTerminalPolicy,
) -> SideEffectTerminalPolicies {
    SideEffectTerminalPolicies::new(
        projection
            .side_effects()
            .filter(|(_, side_effect)| side_effect.run_id == *run_id)
            .map(|(_, side_effect)| (side_effect.pair_id.clone(), terminal_policy))
            .collect::<BTreeMap<_, _>>(),
    )
}

/// Builds confirmation terminal policies for projected side-effect ledgers in one run.
pub fn confirmation_terminal_policies_for_projection_for_test(
    projection: &ProjectionSnapshot,
    run_id: &RunId,
) -> SideEffectTerminalPolicies {
    terminal_policies_for_projection_for_test(
        projection,
        run_id,
        SideEffectTerminalPolicy::Confirmation,
    )
}

/// Builds receipt terminal policies for projected side-effect ledgers in one run.
pub fn receipt_terminal_policies_for_projection_for_test(
    projection: &ProjectionSnapshot,
    run_id: &RunId,
) -> SideEffectTerminalPolicies {
    terminal_policies_for_projection_for_test(projection, run_id, SideEffectTerminalPolicy::Receipt)
}

/// Returns empty side-effect terminal policies.
pub fn empty_terminal_policies_for_test() -> SideEffectTerminalPolicies {
    SideEffectTerminalPolicies::new(BTreeMap::new())
}

/// One row returned by a projection-backed fact query fixture.
pub type FactQueryProjectionRowForTest = mfm_facts::FactQueryResultRow;

/// Executes a canonical fact query against a projection snapshot for tests.
pub fn execute_fact_query_projection_for_test(
    projection: &ProjectionSnapshot,
    plan: &mfm_facts::CanonicalFactQueryPlan,
) -> Result<Vec<FactQueryProjectionRowForTest>> {
    let shape = mfm_facts::parse_canonical_fact_query_shape(plan)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let mut rows = projection
        .fact_index_entries()
        .filter(|(_claim_id, entry)| {
            entry.fact_descriptor_hash == *plan.resolved_descriptor()
                && entry.audience == plan.query_scope().audience()
                && entry.visibility_scope == plan.query_scope().scope()
        })
        .filter(|(_claim_id, entry)| fact_entry_matches_predicates(projection, entry, &shape))
        .map(|(_claim_id, entry)| {
            Ok(FactQueryProjectionRowForTest::new(
                entry.internal_ref()?,
                returned_fields_from_projection(projection, entry, &shape)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    rows.sort_by(|left, right| compare_fact_projection_rows(projection, plan, left, right));
    if let Some(limit) = plan.limit() {
        rows.truncate(limit as usize);
    }
    Ok(rows)
}

/// Builds a signed fact-query receipt for projection-backed query rows.
pub fn signed_fact_query_receipt_for_projection_for_test(
    plan: &mfm_facts::CanonicalFactQueryPlan,
    projection: &ProjectionSnapshot,
    key: &SigningKey,
    store_identity: mfm_facts::StoreIdentity,
    key_id: mfm_facts::StoreKeyId,
    rows: &[mfm_facts::FactQueryResultRow],
) -> mfm_facts::FactQueryReceipt {
    let shape = mfm_facts::parse_canonical_fact_query_shape(plan).expect("query shape");
    let max_order = projection
        .fact_index_entries()
        .filter(|(_claim_id, entry)| {
            entry.audience == plan.query_scope().audience()
                && entry.visibility_scope == plan.query_scope().scope()
        })
        .map(|(_claim_id, entry)| entry.store_commit_order)
        .max()
        .unwrap_or_default();
    let read_frontier = mfm_facts::StoreReadFrontier::new(
        plan.store_scope().clone(),
        plan.query_scope().clone(),
        mfm_facts::DescriptorCatalogWatermark::new(projection.fact_descriptors().count() as u64),
        mfm_facts::StoreCommitOrder::new(max_order),
    );
    let plan_hash = mfm_facts::fact_query_plan_hash(plan).expect("fact query plan hash");
    let material = mfm_facts::FactQueryReceiptMaterial::from_rows(
        &plan_hash,
        read_frontier,
        mfm_facts::StoreReadFrontierType::Snapshot,
        rows,
        !shape.return_fields().is_empty(),
        plan.limit(),
    )
    .expect("fact query receipt material");
    signed_fact_query_receipt_material_for_test(material, key, store_identity, key_id)
}

fn fact_entry_matches_predicates(
    projection: &ProjectionSnapshot,
    entry: &FactIndexProjection,
    shape: &mfm_facts::CompiledFactQueryShape,
) -> bool {
    shape.predicates().iter().all(|predicate| {
        projection
            .fact_term(&entry.fact_claim_id, predicate.field_id())
            .is_some_and(|term| predicate.matches_scalar(&term.value))
    })
}

fn returned_fields_from_projection(
    projection: &ProjectionSnapshot,
    entry: &FactIndexProjection,
    shape: &mfm_facts::CompiledFactQueryShape,
) -> Result<Vec<mfm_facts::FactFieldValue>> {
    shape
        .return_fields()
        .iter()
        .filter_map(|return_field| {
            projection
                .fact_term(&entry.fact_claim_id, return_field)
                .map(|term| {
                    mfm_facts::FactFieldValue::new(
                        term.field_id.clone(),
                        term.value_type,
                        term.value.clone(),
                    )
                    .map_err(|error| StoreError::Identity(error.to_string()))
                })
        })
        .collect()
}

fn compare_fact_projection_rows(
    projection: &ProjectionSnapshot,
    plan: &mfm_facts::CanonicalFactQueryPlan,
    left: &FactQueryProjectionRowForTest,
    right: &FactQueryProjectionRowForTest,
) -> std::cmp::Ordering {
    for term in plan.ordering().terms() {
        let left_value =
            fact_ordering_value(projection, left.fact_ref().fact_claim_id(), term.field_id());
        let right_value = fact_ordering_value(
            projection,
            right.fact_ref().fact_claim_id(),
            term.field_id(),
        );
        let ordering = term
            .compare_values(left_value, right_value)
            .unwrap_or(std::cmp::Ordering::Equal);
        if ordering != std::cmp::Ordering::Equal {
            return ordering;
        }
    }
    left.fact_ref()
        .fact_claim_id()
        .cmp(right.fact_ref().fact_claim_id())
}

fn fact_ordering_value<'a>(
    projection: &'a ProjectionSnapshot,
    claim_id: &mfm_facts::FactClaimId,
    field_id: &mfm_facts::FactFieldId,
) -> Option<&'a mfm_facts::FactCanonicalScalar> {
    projection
        .fact_term(claim_id, field_id)
        .map(|term| &term.value)
}

/// Builds a fact-query receipt trust root for a deterministic test signing key.
pub fn fact_query_receipt_trust_root_for_test(
    key: &SigningKey,
    store_identity: mfm_facts::StoreIdentity,
    key_id: mfm_facts::StoreKeyId,
) -> FactQueryReceiptTrustRoot {
    FactQueryReceiptTrustRoot::new(
        store_identity,
        mfm_facts::StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
        key_id,
        key.verifying_key().to_bytes(),
    )
    .expect("fact query receipt trust root")
}

/// Input for signing a fact-query receipt fixture.
pub struct SignedFactQueryReceiptFixtureInputForTest<'a> {
    /// Canonical query plan hash.
    pub plan_hash: &'a ContentDigest,
    /// Signing key used for the local receipt authentication signature.
    pub key: &'a SigningKey,
    /// Store identity to bind into the signed receipt.
    pub store_identity: mfm_facts::StoreIdentity,
    /// Store key id to bind into the signed receipt.
    pub key_id: mfm_facts::StoreKeyId,
    /// Read frontier reported by the receipt.
    pub read_frontier: mfm_facts::StoreReadFrontier,
    /// Returned fact rows covered by the receipt.
    pub rows: &'a [mfm_facts::FactQueryResultRow],
    /// Whether returned field summaries are included in the receipt material.
    pub include_returned_field_summaries: bool,
    /// Query limit covered by the receipt material.
    pub limit: Option<u64>,
}

/// Builds a signed fact-query receipt fixture using the production receipt authentication message.
pub fn signed_fact_query_receipt_for_test(
    input: SignedFactQueryReceiptFixtureInputForTest<'_>,
) -> mfm_facts::FactQueryReceipt {
    let material = mfm_facts::FactQueryReceiptMaterial::from_rows(
        input.plan_hash,
        input.read_frontier,
        mfm_facts::StoreReadFrontierType::Snapshot,
        input.rows,
        input.include_returned_field_summaries,
        input.limit,
    )
    .expect("fact query receipt material");
    signed_fact_query_receipt_material_for_test(
        material,
        input.key,
        input.store_identity,
        input.key_id,
    )
}

fn signed_fact_query_receipt_material_for_test(
    material: mfm_facts::FactQueryReceiptMaterial,
    key: &SigningKey,
    store_identity: mfm_facts::StoreIdentity,
    key_id: mfm_facts::StoreKeyId,
) -> mfm_facts::FactQueryReceipt {
    let message = fact_query_receipt_authentication_message(
        &store_identity,
        mfm_facts::StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
        &key_id,
        material.store_receipt_hash(),
    )
    .expect("fact query receipt authentication message");
    let auth = mfm_facts::StoreReceiptAuthentication::new(
        store_identity,
        mfm_facts::StoreReceiptAuthenticationScheme::LocalEd25519Sha256JcsV1,
        Some(key_id),
        key.sign(message.as_bytes()).to_bytes().to_vec(),
    )
    .expect("fact query receipt authentication");
    material.into_receipt(auth)
}
