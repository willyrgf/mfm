//! Test-only helpers for typed store contract fixtures.

use std::collections::BTreeMap;
use std::future::Future;
use std::task::{Context, Poll, Waker};

use super::validation::{is_retention_payload, is_run_completed_payload};
use super::*;
use mfm_canonical::{sha256_digest_bytes, CanonicalJsonBytes, CanonicalValue};
use mfm_events::v1 as events;
use mfm_ids::{
    AdapterKind, ArtifactId, AttemptId, CapabilityKind, CellId, ContentDigest, DescriptorId,
    DigestAlgorithm, DigestBytes, EventId, NodeId, RunId, SchemaId, ScopeId, SemanticTypeId,
    SpecHash, StateKind, StoreScopeId,
};
use mfm_spec::v1 as spec;

#[path = "fact_fixtures.rs"]
mod fact_fixtures;
pub use self::fact_fixtures::*;

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
        .any(events::KernelEventPayload::is_side_effect_terminal)
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
    super::fact_query::execute_fact_query_projection(projection, plan)
}

/// Builds deterministic fact-query evidence for projection-backed query rows.
pub fn fact_query_receipt_for_projection_for_test(
    plan: &mfm_facts::CanonicalFactQueryPlan,
    projection: &ProjectionSnapshot,
    rows: &[mfm_facts::FactQueryResultRow],
) -> mfm_facts::FactQueryReceipt {
    let shape = mfm_facts::parse_canonical_fact_query_shape(plan).expect("query shape");
    let max_order = projection
        .fact_query_entries()
        .map(|(_claim_id, entry)| entry.store_commit_order())
        .max()
        .unwrap_or_default();
    let read_frontier = mfm_facts::StoreReadFrontier::new(
        StoreScopeId::new("mfm.store_scope.v1:01010101010101010101010101010101")
            .expect("store scope"),
        mfm_facts::StoreCommitOrder::new(max_order),
    );
    mfm_facts::FactQueryReceipt::from_rows(
        read_frontier,
        rows,
        !shape.return_fields().is_empty(),
        plan.limit(),
    )
    .expect("fact query receipt")
}

/// Input for a deterministic fact-query receipt fixture.
pub struct FactQueryReceiptFixtureInputForTest<'a> {
    /// Read frontier reported by the receipt.
    pub read_frontier: mfm_facts::StoreReadFrontier,
    /// Returned fact rows covered by the receipt.
    pub rows: &'a [mfm_facts::FactQueryResultRow],
    /// Whether returned field summaries are included in the receipt material.
    pub include_returned_field_summaries: bool,
    /// Query limit covered by the receipt material.
    pub limit: Option<u64>,
}

/// Builds a deterministic fact-query receipt fixture.
pub fn fact_query_receipt_for_test(
    input: FactQueryReceiptFixtureInputForTest<'_>,
) -> mfm_facts::FactQueryReceipt {
    mfm_facts::FactQueryReceipt::from_rows(
        input.read_frontier,
        input.rows,
        input.include_returned_field_summaries,
        input.limit,
    )
    .expect("fact query receipt")
}
