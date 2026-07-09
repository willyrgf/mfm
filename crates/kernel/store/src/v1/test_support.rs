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
    let subject_material = mfm_facts::extract_subject_material(descriptor, &input.subject)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let subject = mfm_facts::fact_subject_evidence_from_material(descriptor, &subject_material)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let response_canonical = CanonicalJsonBytes::from_value(&input.response);
    let response_bytes = response_canonical.as_bytes().to_vec();
    let response_hash = response_canonical.content_digest();
    let response_artifact_id = input.response_artifact_id.unwrap_or_else(|| {
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
        subject: subject.clone(),
        observed_at: input.observed_at.clone(),
        request: input.request.clone(),
        response: mfm_facts::FactResponseEvidence::new(
            input.response_schema_id.clone(),
            response_hash.clone(),
            response_artifact_id.clone(),
            artifact_evidence_hash.clone(),
        ),
        producer: input.producer.clone(),
    })
    .map_err(|error| StoreError::Identity(error.to_string()))?;
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

/// One Platform holding fact to seed into an in-memory store for report SelectHoldings tests.
///
/// Built through the same FactRecorded-shaped projection fixtures as production admission
/// (no SQL fact_index pokes). Callers supply real holding descriptors and subject/response
/// material that match portfolio SelectHoldings query predicates.
#[derive(Debug, Clone)]
pub struct PlatformHoldingFactSeedForTest {
    /// Holding fact descriptor (BTC address balance or EVM native balance at cutover).
    pub descriptor: mfm_facts::FactDescriptor,
    /// Fact-record projection input (subject/response/visibility/claim identity/producer).
    pub input: FactProjectionFixtureInputForTest,
}

/// Seeds Platform holding facts into an in-memory run store for fact-backed report tests.
///
/// This is the **single** merge-safe seed path for certified `portfolio_snapshot` complete
/// tests. For each seed it:
/// 1. Builds descriptor + FactRecorded-shaped record/index/term fixtures.
/// 2. Builds a matching source-run `FactRecorded` stream envelope and aligns
///    `source_event_id` to the envelope's derived event id (required for
///    `verify_fact_query_returned_ref` during report replay).
/// 3. Merges projection rows into the store's current projection (does not clobber run state).
/// 4. Retains descriptor and FactResponse artifact bytes.
/// 5. Admits those evidences into the store **artifact authority map** (required so
///    SelectHoldings fact-query retention staging does not fail with MissingArtifact /
///    redacted RunStoreRejected).
/// 6. Injects the source `FactRecorded` envelopes so `verify_replay` can load cross-run
///    source fact events for FactQueryEvidence.
///
/// Call this on the same store used for report launch/resume. Do not dual-store.
pub fn seed_platform_holding_facts_for_test(
    store: &AsyncInMemoryRunStore,
    seeds: impl IntoIterator<Item = PlatformHoldingFactSeedForTest>,
) -> Result<()> {
    let existing = store.projection_snapshot()?;
    let mut parts = ProjectionSnapshotParts::from_snapshot(&existing);
    let mut retained = Vec::new();
    let mut admitted_evidence = Vec::new();
    let mut source_envelopes = Vec::new();

    for seed in seeds {
        let descriptor_fixture =
            fact_descriptor_projection_fixture_for_test(seed.descriptor.clone())?;
        let descriptor_hash = descriptor_fixture.descriptor_hash.clone();
        let mut fact_fixture =
            fact_projection_fixture_for_test(&seed.descriptor, descriptor_hash.clone(), seed.input)?;

        // Source-run FactRecorded envelope: replay loads these by claim coordinates.
        // Use a dedicated source-run SpecHash (not the consumer program); retained
        // source facts are not re-checked against the consumer certified graph.
        let source_spec_hash = SpecHash::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            DigestBytes::from_array([0x5f; 32]),
        );
        let source_run_id = fact_fixture.record.source_run_id.clone();
        let source_seq = fact_fixture.record.source_seq;
        let source_ordinal = fact_fixture.record.source_ordinal;
        let commit_id = fact_fixture
            .index
            .as_ref()
            .map(|index| index.commit_id.clone())
            .unwrap_or_else(|| {
                CommitKey::new(format!("holding-source-{source_seq}")).expect("commit key")
            });
        let payload = events::KernelEventPayload::FactRecorded(events::FactRecorded {
            spec_hash: source_spec_hash,
            node_id: fact_fixture.record.node_id.clone(),
            attempt_id: fact_fixture.record.attempt_id.clone(),
            claim: fact_fixture.record.claim.clone(),
        });
        let envelope = persisted_kernel_event_envelope_with_ordinal_for_test(
            &source_run_id,
            source_seq,
            source_ordinal,
            commit_id.clone(),
            payload,
        );
        // Align projection source_event_id with the derived envelope event id.
        fact_fixture.record.source_event_id = envelope.event_id().clone();
        if let Some(index) = fact_fixture.index.as_mut() {
            index.source_event_id = envelope.event_id().clone();
        }
        source_envelopes.push(envelope);

        match parts.fact_descriptors.get(&descriptor_hash) {
            Some(existing)
                if existing.descriptor_artifact_id
                    == descriptor_fixture.projection.descriptor_artifact_id
                    && existing.descriptor_artifact_evidence
                        == descriptor_fixture.projection.descriptor_artifact_evidence => {}
            Some(_) => {
                return Err(StoreError::ProjectionConflict {
                    key: format!("fact_descriptor:{descriptor_hash}"),
                    message: "conflicting platform holding descriptor seed".to_owned(),
                });
            }
            None => {
                parts.fact_descriptors.insert(
                    descriptor_hash.clone(),
                    descriptor_fixture.projection.clone(),
                );
                retained.push(descriptor_fixture.descriptor_artifact);
                admitted_evidence.push(descriptor_fixture.descriptor_evidence);
            }
        }

        let claim_id = fact_fixture.record.fact_claim_id.clone();
        if parts.fact_records.contains_key(&claim_id) {
            return Err(StoreError::ProjectionConflict {
                key: format!("fact_record:{claim_id:?}"),
                message: "duplicate platform holding fact claim seed".to_owned(),
            });
        }
        parts
            .fact_records
            .insert(claim_id.clone(), fact_fixture.record.clone());
        if let Some(index) = fact_fixture.index.clone() {
            parts.fact_index_entries.insert(claim_id.clone(), index);
        }
        for term in fact_fixture.terms {
            parts
                .fact_term_entries
                .insert((term.fact_claim_id.clone(), term.field_id.clone()), term);
        }

        let response_evidence = fact_fixture.response_artifact_evidence.clone();
        let response_requirement = events::EventArtifactRequirement {
            source: events::EventArtifactReferenceSource::FactResponse,
            artifact_id: response_evidence.artifact_id.clone(),
            digest: Some(response_evidence.digest.clone()),
            byte_len: Some(response_evidence.byte_len),
            media_type: Some(response_evidence.media_type.clone()),
            schema_id: response_evidence.schema_id.clone(),
            semantic_type_id: None,
            producer_node_id: response_evidence.producer_node_id.clone(),
            producer_seed_id: None,
            artifact_role: Some(events::ArtifactRole::FactResponse),
        };
        let response_artifact = VerifiedRunArtifactBytes::new(
            fact_fixture.response_bytes,
            response_evidence.clone(),
            &response_requirement,
        )?;
        retained.push(response_artifact);
        admitted_evidence.push(response_evidence);
    }

    let projection = ProjectionSnapshot::from_parts(parts)?;
    store.seed_projection_snapshot_for_test(projection, retained)?;
    store.seed_artifact_evidence_for_test(&admitted_evidence)?;
    store.seed_run_stream_envelopes_for_test(source_envelopes)?;
    Ok(())
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
pub fn persisted_kernel_event_envelope_for_test(
    run_id: &RunId,
    seq: u64,
    commit_key: CommitKey,
    payload: events::KernelEventPayload,
) -> KernelEventEnvelope {
    persisted_kernel_event_envelope_with_ordinal_for_test(run_id, seq, 0, commit_key, payload)
}

/// Builds a validated persisted event envelope from a typed payload and explicit commit ordinal.
pub fn persisted_kernel_event_envelope_with_ordinal_for_test(
    run_id: &RunId,
    seq: u64,
    ordinal: u32,
    commit_key: CommitKey,
    payload: events::KernelEventPayload,
) -> KernelEventEnvelope {
    let seq = StreamSeq::new(seq).expect("stream seq");
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
        mfm_facts::FactProjectionGeneration::new(1),
        max_order,
        mfm_facts::StoreCommitWatermark::new(max_order),
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
