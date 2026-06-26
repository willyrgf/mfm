//! Test-only helpers for typed store contract fixtures.

use super::*;
use mfm_events::v1 as events;
use mfm_ids::{AttemptId, DigestAlgorithm, DigestBytes, RunId, SpecHash};
use mfm_spec::v1 as spec;

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

/// Returns a deterministic attempt id made from one repeated digest byte.
pub fn fixed_attempt_id_for_test(byte: u8) -> AttemptId {
    AttemptId::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([byte; 32]),
    )
}
