use super::*;

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
    /// Holding fact descriptor admitted by the source-run fixture.
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
            entry_point: events::EntryPointLaunchEvidence::new(
                "mfm.test/fact_fixture_record@1",
                Vec::new(),
            )
            .map_err(|error| StoreError::Identity(error.to_string()))?,
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
            capability_implementations: Vec::new(),
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
                effect_contract_digest: None,
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
