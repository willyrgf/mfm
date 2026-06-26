//! Test-only helpers for typed store contract fixtures.

use std::collections::BTreeMap;
use std::future::Future;
use std::task::{Context, Poll, Waker};

use super::*;
use mfm_canonical::sha256_digest_bytes;
use mfm_events::v1 as events;
use mfm_ids::{
    AdapterKind, ArtifactId, AttemptId, CapabilityKind, CellId, ContentDigest, DescriptorId,
    DigestAlgorithm, DigestBytes, EventId, NodeId, RunId, SchemaId, ScopeId, SemanticTypeId,
    SpecHash, StateKind, TrustScopeId,
};
use mfm_spec::v1 as spec;

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
        let admitted = store
            .acquire_execution_claim(run_id, holder.clone())
            .await?;
        let NowaitSkipAdmissionResult::Admitted(first_lease) = admitted else {
            panic!("first execution claim should be admitted");
        };
        assert_eq!(first_lease.token, holder);
        assert!(matches!(
            store.execution_claim_status(run_id).await?,
            ExecutionClaimStatus::Live(status) if status.token == first_lease.token
        ));

        let busy = store.acquire_execution_claim(run_id, other.clone()).await?;
        let NowaitSkipAdmissionResult::Busy(busy) = busy else {
            panic!("second execution claim should be busy");
        };
        assert_eq!(
            busy.holder.expect("busy holder lease").token,
            first_lease.token
        );

        assert!(store.renew_execution_claim(run_id, &other).await?.is_none());
        assert!(!store.release_execution_claim(run_id, &other).await?);

        let renewed = store
            .renew_execution_claim(run_id, &first_lease.token)
            .await?
            .expect("matching token returns lease");
        assert_eq!(renewed.token, first_lease.token);
        assert!(renewed.lease_expires_at_unix_ms >= first_lease.lease_expires_at_unix_ms);

        assert!(
            store
                .release_execution_claim(run_id, &renewed.token)
                .await?
        );
        assert!(matches!(
            store.execution_claim_status(run_id).await?,
            ExecutionClaimStatus::Unclaimed
        ));
        assert!(matches!(
            store.acquire_execution_claim(run_id, other).await?,
            NowaitSkipAdmissionResult::Admitted(_)
        ));
        Ok(())
    })
}

/// Returns deterministic digest bytes made from one repeated byte.
pub fn fixed_digest_bytes_for_test(byte: u8) -> DigestBytes {
    DigestBytes::from_array([byte; 32])
}

/// Returns a deterministic content digest made from one repeated digest byte.
pub fn fixed_content_digest_for_test(byte: u8) -> ContentDigest {
    ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        fixed_digest_bytes_for_test(byte),
    )
}

/// Returns a deterministic spec hash made from one repeated digest byte.
pub fn fixed_spec_hash_for_test(byte: u8) -> SpecHash {
    SpecHash::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        fixed_digest_bytes_for_test(byte),
    )
}

/// Returns a deterministic run id made from one repeated digest byte.
pub fn fixed_run_id_for_test(byte: u8) -> RunId {
    RunId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        fixed_digest_bytes_for_test(byte),
    )
}

/// Returns a deterministic artifact id made from one repeated digest byte.
pub fn fixed_artifact_id_for_test(byte: u8) -> ArtifactId {
    ArtifactId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        fixed_digest_bytes_for_test(byte),
    )
}

/// Returns a deterministic attempt id made from one repeated digest byte.
pub fn fixed_attempt_id_for_test(byte: u8) -> AttemptId {
    AttemptId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        fixed_digest_bytes_for_test(byte),
    )
}

/// Returns a deterministic node id made from one repeated digest byte.
pub fn fixed_node_id_for_test(byte: u8) -> NodeId {
    NodeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        fixed_digest_bytes_for_test(byte),
    )
}

/// Returns a deterministic cell id made from one repeated digest byte.
pub fn fixed_cell_id_for_test(byte: u8) -> CellId {
    CellId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        fixed_digest_bytes_for_test(byte),
    )
}

/// Returns a deterministic scope id made from one repeated digest byte.
pub fn fixed_scope_id_for_test(byte: u8) -> ScopeId {
    ScopeId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        fixed_digest_bytes_for_test(byte),
    )
}

/// Returns a deterministic event id made from one repeated digest byte.
pub fn fixed_event_id_for_test(byte: u8) -> EventId {
    EventId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        fixed_digest_bytes_for_test(byte),
    )
}

/// Returns a deterministic descriptor id made from one repeated digest byte.
pub fn fixed_descriptor_id_for_test(byte: u8) -> DescriptorId {
    DescriptorId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        fixed_digest_bytes_for_test(byte),
    )
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

/// Returns the artifact id for deterministic artifact bytes.
pub fn artifact_bytes_artifact_id_for_test(byte: u8) -> ArtifactId {
    let digest = artifact_content_digest_for_test(byte);
    ArtifactId::from_digest(digest.algorithm(), *digest.digest())
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

/// Builds verified prepared artifact bytes for deterministic artifact test fixtures.
pub fn prepared_artifact_bytes_for_test(
    evidence: &ArtifactEvidenceRef,
) -> Result<PreparedArtifactBytes> {
    let bytes = artifact_bytes_for_digest_for_test(&evidence.digest).ok_or_else(|| {
        StoreError::ArtifactEvidenceMismatch {
            artifact_id: evidence.artifact_id.clone(),
            field: "bytes",
        }
    })?;
    PreparedArtifactBytes::new(bytes, evidence.clone())
}

/// Builds run identity material for a deterministic test trust scope suffix.
pub fn run_identity_material_for_test(
    certified_spec_hash: SpecHash,
    trust_scope_hex: &str,
) -> events::RunIdentityMaterialV1 {
    events::RunIdentityMaterialV1 {
        certified_spec_hash,
        trust_scope_id: TrustScopeId::new(format!("{}{}", TrustScopeId::PREFIX, trust_scope_hex))
            .expect("test trust scope"),
        distinct_run_key_digest: None,
    }
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
