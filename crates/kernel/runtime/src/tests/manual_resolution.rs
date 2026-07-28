use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::scheduler::{
    ManualResolutionCommitExpectationForTest, ManualResolutionExpectationMutationForTest,
};

#[tokio::test]
async fn runtime_resolves_manual_resolution_terminal() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let registry = side_effect_driver_registry_with_submission_decision(
        &fixture,
        TestSubmissionDecision::Ambiguous,
    );
    let mut store = TestTypedRunStore::new();
    let scheduler = test_scheduler(register_fixture_capabilities(registry.clone(), &fixture));
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");

    for _ in 0..3 {
        assert_drive!(scheduler, store, current, Advanced, "advance to ambiguity");
    }
    let (run_mode, manual_block_reason) = current_saga_state(&current);
    assert_eq!(run_mode, store::RunMode::ManualBlocked);
    assert_eq!(
        manual_block_reason,
        Some(store::ManualBlockReason::PolicyManualResolution)
    );

    current = append_manual_resolution(
        &scheduler,
        &store,
        current,
        events::ManualResolutionOutcome::ConfirmRemediated,
    )
    .await;
    assert_eq!(
        current_saga_state(&current).0,
        store::RunMode::ManuallyResolved
    );
    let terminal_proof = current
        .lifecycle()
        .saga_terminal_proof(
            current
                .lifecycle()
                .next_sequence()
                .expect("terminal prefix sequence"),
        )
        .expect("verified view mints terminal proof without caller proof material");
    assert_eq!(
        terminal_proof.outcome(),
        events::RunCompletionOutcome::ManuallyResolved
    );

    let fresh_scheduler =
        SerialTypedScheduler::new(register_fixture_capabilities(registry, &fixture));
    assert_drive!(
        fresh_scheduler,
        store,
        current,
        Advanced,
        "resolve manual terminal"
    );
    assert!(matches!(
        current
            .lifecycle()
            .completion()
            .expect("run completion")
            .outcome(),
        events::RunCompletionOutcome::ManuallyResolved
    ));
}

#[tokio::test]
async fn runtime_rejects_manual_resolution_prefix_with_open_attempt() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let registry = side_effect_driver_registry_with_submission_decision(
        &fixture,
        TestSubmissionDecision::Ambiguous,
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");

    for _ in 0..3 {
        assert_drive!(
            scheduler,
            store,
            current,
            Advanced,
            "advance to manual block"
        );
    }
    assert_eq!(
        current_saga_state(&current).0,
        store::RunMode::ManualBlocked
    );

    let resolve_node = fixture
        .runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
            )
        })
        .expect("resolve saga terminal node")
        .clone();
    append_attempt_start(&mut store, &fixture, &resolve_node, 1);
    current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("reload current run with open attempt");

    let error = current
        .lifecycle()
        .manual_resolution_prefix_authority()
        .expect_err("manual prefix rejects open attempt");
    assert!(
        error
            .to_string()
            .contains("contains an open semantic attempt"),
        "{error}"
    );
}

#[tokio::test]
async fn runtime_rejects_changed_manual_authorization_object_in_successor() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let registry = side_effect_driver_registry_with_submission_decision(
        &fixture,
        TestSubmissionDecision::Ambiguous,
    );
    let mut store = TestTypedRunStore::new();
    let scheduler = test_scheduler(register_fixture_capabilities(registry, &fixture));
    start_fixture_run(
        &scheduler,
        &mut store,
        &fixture,
        vec![fixture.seed_ref.clone()],
    )
    .await
    .expect("start run");
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");

    for _ in 0..3 {
        assert_drive!(scheduler, store, current, Advanced, "advance to ambiguity");
    }
    current = append_manual_resolution(
        &scheduler,
        &store,
        current,
        events::ManualResolutionOutcome::ConfirmRemediated,
    )
    .await;
    let (authorization_requirement, authorization_bytes) = manual_authorization_material(&current);
    store.replace_committed_artifact_for_corruption(
        &authorization_requirement.artifact_id,
        &authorization_requirement.evidence_hash,
        br#"{"redacted":"corrupted"}"#.to_vec(),
    );
    let sequence_before = current
        .view()
        .current_run_sequence()
        .expect("current run sequence");

    let resolve_node = fixture
        .runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
            )
        })
        .expect("resolve saga terminal node");
    let error = drive_current_once_with_claim(&scheduler, &store, current)
        .await
        .expect_err("changed authorization object rejects successor");
    let public_error = error.to_string();
    assert!(
        matches!(
            error,
            RuntimeError::Store(_) | RuntimeError::InvalidRunStream(_)
        ),
        "{public_error}"
    );
    assert!(!public_error.contains("signature_hex"));
    assert!(!public_error.contains("operator_note"));
    assert!(!public_error.contains(r#""redacted":"corrupted""#));
    store.replace_committed_artifact_for_corruption(
        &authorization_requirement.artifact_id,
        &authorization_requirement.evidence_hash,
        authorization_bytes,
    );
    let current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("reload current after restoring authorization");
    assert_eq!(
        current.view().current_run_sequence(),
        Some(sequence_before + 1),
        "attempt start must remain the only append after successor verification fails"
    );
    let resolve_attempt = attempt_id(
        &fixture.run_id,
        fixture.runtime_spec.spec_hash(),
        &resolve_node.node_id,
        1,
    )
    .expect("resolve attempt id");
    assert!(matches!(
        current
            .lifecycle()
            .attempt(&resolve_node.node_id, &resolve_attempt)
            .expect("open resolve attempt")
            .status(),
        store::current_lifecycle::CurrentAttemptStatusRef::Started { .. }
    ));
    assert!(current.lifecycle().completion().is_none());
}

#[tokio::test]
async fn runtime_rejects_manual_resolution_before_manual_blocked() {
    let fixture = fixture_with_manual_resolution_side_effect_state();
    let registry = side_effect_driver_registry_with_submission_decision(
        &fixture,
        TestSubmissionDecision::Ambiguous,
    );
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;
    let current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");
    let evidence_bytes = br#"{"operator_note":"too_early"}"#.to_vec();

    let error = record_manual_resolution(
        &scheduler,
        &mut store,
        current,
        ManualResolutionRequest {
            outcome: events::ManualResolutionOutcome::ConfirmRemediated,
            evidence_artifact: ManualResolutionEvidenceArtifact {
                bytes: evidence_bytes,
                media_type: spec::MediaType::new("application/json").expect("media"),
            },
            proof_bytes: br#"{}"#.to_vec(),
            note: None,
        },
    )
    .await
    .expect_err("manual resolution before block rejects");

    assert!(
        matches!(error, RuntimeError::InvalidRunStream(_)),
        "{error}"
    );
}

#[tokio::test]
async fn runtime_manual_resolution_masked_commit_with_later_suffix_reconciles_exactly() {
    let (fixture, scheduler, store, current) = stale_manual_blocked_run().await;
    let expected_sequence = current
        .lifecycle()
        .next_sequence()
        .expect("manual expected sequence");
    let suffix_sequence =
        store::StreamSeq::new(expected_sequence.as_u64() + 1).expect("suffix sequence");
    let (suffix, resolve_attempt) = attempt_bundle(
        &fixture,
        resolve_saga_terminal_node(&fixture),
        suffix_sequence,
        AttemptEvent::Start { attempt_no: 1 },
    );
    store.inject_exact_with_later_suffix(suffix);
    let request = manual_resolution_request_for_current(
        &current,
        events::ManualResolutionOutcome::ConfirmRemediated,
    );

    let current = scheduler
        .record_manual_resolution(&store, current, request)
        .await
        .expect("masked exact commit reconciles across a later legal suffix");

    assert_eq!(
        current.view().current_run_sequence(),
        Some(suffix_sequence.as_u64())
    );
    assert_eq!(
        current_saga_state(&current).0,
        store::RunMode::ManuallyResolved
    );
    {
        let lifecycle = current.lifecycle();
        let resolve_attempt = lifecycle
            .attempt(
                &resolve_saga_terminal_node(&fixture).node_id,
                &resolve_attempt,
            )
            .expect("later resolve attempt");
        assert!(matches!(
            resolve_attempt.status(),
            store::current_lifecycle::CurrentAttemptStatusRef::Started { .. }
        ));
    }
    assert_eq!(
        store.manual_append_attempts(),
        1,
        "stale reconciliation must not reappend the sealed request"
    );
    store.assert_captured_expectation_matrix(&current);
}

#[tokio::test]
async fn runtime_manual_resolution_unrelated_successor_is_typed_stale_without_retry() {
    let (fixture, scheduler, store, current) = stale_manual_blocked_run().await;
    let expected_sequence = current
        .lifecycle()
        .next_sequence()
        .expect("manual expected sequence");
    let interrupted_sequence =
        store::StreamSeq::new(expected_sequence.as_u64() + 1).expect("interrupted sequence");
    let unrelated_node = node_by_output(&fixture, &fixture.cell_a);
    let (start, unrelated_attempt) = attempt_bundle(
        &fixture,
        unrelated_node,
        expected_sequence,
        AttemptEvent::Start { attempt_no: 2 },
    );
    let (interrupted, _) = attempt_bundle(
        &fixture,
        unrelated_node,
        interrupted_sequence,
        AttemptEvent::Interrupted(unrelated_attempt),
    );
    store.inject_unrelated_successor(vec![start, interrupted]);
    let request = manual_resolution_request_for_current(
        &current,
        events::ManualResolutionOutcome::ConfirmRemediated,
    );

    let error = scheduler
        .record_manual_resolution(&store, current, request)
        .await
        .expect_err("unrelated successor must not reconcile as the manual request");

    assert_eq!(
        error,
        RuntimeError::ManualResolutionRequestStale { expected_sequence }
    );
    assert_eq!(
        store.manual_append_attempts(),
        1,
        "a stale request must not be retried implicitly"
    );

    let current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load unrelated successor");
    assert_eq!(
        current.view().current_run_sequence(),
        Some(interrupted_sequence.as_u64())
    );
    assert_eq!(
        current_saga_state(&current).0,
        store::RunMode::ManualBlocked
    );
    let request = manual_resolution_request_for_current(
        &current,
        events::ManualResolutionOutcome::ConfirmRemediated,
    );
    let current = scheduler
        .record_manual_resolution(&store, current, request)
        .await
        .expect("fresh explicit request binds to the unrelated successor");
    assert_eq!(
        current_saga_state(&current).0,
        store::RunMode::ManuallyResolved
    );
    assert_eq!(
        store.manual_append_attempts(),
        2,
        "only the caller's fresh explicit request may append again"
    );
}

#[tokio::test]
async fn runtime_rejects_forward_node_emitting_remediation_ledger_purpose() {
    let fixture = fixture_with_first_side_effect_state();
    let mut registry = test_runner_registry();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            ForwardEmitsRemediationPurposeRunner,
        ))
        .expect("binding a");
    register_fixture_read_runner(&mut registry, &fixture, READ_EXTERNAL_RUNNER);
    let (scheduler, store) = started_fixture_run_with_registry(registry, &fixture).await;
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");

    assert_first_node_invalid_after_drive!(
        scheduler,
        store,
        current,
        fixture,
        "terminalize wrong forward ledger purpose"
    );
}

#[tokio::test]
async fn runtime_rejects_remediation_node_emitting_forward_ledger_purpose() {
    let fixture = fixture_with_two_side_effects_and_failing_tail();
    let forward_a = node_by_output(&fixture, &fixture.cell_a).clone();
    let forward_b = node_by_output(&fixture, &fixture.cell_b).clone();
    let forward_a_output = effective_output_cell_for_node(&fixture, &forward_a);
    let forward_b_output = effective_output_cell_for_node(&fixture, &forward_b);
    let failure_node = node_by_output(
        &fixture,
        fixture.cell_c.as_ref().expect("failing output cell"),
    )
    .clone();
    let mut registry = test_runner_registry();
    registry
        .register(binding(
            fixture.descriptor_a.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            RemediationEmitsForwardPurposeRunner::new(&fixture),
        ))
        .expect("binding a");
    registry
        .register(binding(
            fixture.descriptor_b.clone(),
            APPLY_SIDE_EFFECT_RUNNER,
            RemediationEmitsForwardPurposeRunner::new(&fixture),
        ))
        .expect("binding b");
    registry
        .register(binding(
            fixture
                .descriptor_c
                .clone()
                .expect("failing node descriptor"),
            "pure",
            BlockingRunner,
        ))
        .expect("binding failure node");
    let (scheduler, mut store) = started_fixture_run_with_registry(registry, &fixture).await;

    drive_until_cells_terminal(
        &scheduler,
        &mut store,
        &fixture,
        &[forward_a_output, forward_b_output],
        "drive forward side-effect phase",
    )
    .await;
    let failure_attempt = append_or_get_started_attempt(&mut store, &fixture, &failure_node, 1);
    append_attempt_failure(&mut store, &fixture, &failure_node, &failure_attempt, false);
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");

    assert_drive!(
        scheduler,
        store,
        current,
        Advanced,
        "terminalize wrong remediation ledger purpose"
    );
    assert_failure_code_count(&current, "runner_output_invalid", 1);
}

fn current_saga_state(
    current: &VerifiedCurrentRun,
) -> (store::RunMode, Option<store::ManualBlockReason>) {
    let runtime_spec = current.runtime_spec();
    let lifecycle = current.lifecycle();
    let terminal_policies = store::SideEffectTerminalPolicies::from_spec(runtime_spec.spec())
        .expect("certified terminal policies");
    lifecycle
        .with_saga(&runtime_spec.spec().saga, &terminal_policies, |saga| {
            (saga.run_mode(), saga.manual_block_reason())
        })
        .expect("current saga")
}

fn manual_authorization_material(
    current: &VerifiedCurrentRun,
) -> (store::EventArtifactRequirement, Vec<u8>) {
    let lifecycle = current.lifecycle();
    let mut authorization_requirement = None;
    let _ = lifecycle.visit_records(|record| {
        let store::current_lifecycle::CurrentRecordKindRef::ManualResolutionRecorded(payload) =
            record.kind()
        else {
            return std::ops::ControlFlow::Continue(());
        };
        let _ = record.visit_artifact_requirements(|requirement| {
            if requirement.artifact_id == payload.authorization_artifact_id {
                authorization_requirement = Some(requirement.clone());
                return std::ops::ControlFlow::Break(());
            }
            std::ops::ControlFlow::Continue(())
        });
        std::ops::ControlFlow::Break(())
    });
    let requirement = authorization_requirement.expect("manual authorization requirement");
    let bytes = lifecycle
        .object_for_requirement(&requirement)
        .expect("manual authorization object")
        .bytes()
        .to_vec();
    (requirement, bytes)
}

async fn stale_manual_blocked_run() -> (
    Fixture,
    SerialTypedScheduler,
    StaleManualResolutionStore,
    VerifiedCurrentRun,
) {
    let fixture = fixture_with_manual_resolution_exclusive_side_effect_state();
    let registry = side_effect_driver_registry_with_submission_decision(
        &fixture,
        TestSubmissionDecision::Ambiguous,
    );
    let scheduler = fixture_scheduler(registry, &fixture);
    let store = StaleManualResolutionStore::new();
    start_fixture_run_async_store(&scheduler, &store, &fixture, vec![fixture.seed_ref.clone()])
        .await
        .expect("start run");
    let mut current = load_fixture_current(&scheduler, &store, &fixture)
        .await
        .expect("load current run");
    for _ in 0..8 {
        if current_saga_state(&current).0 == store::RunMode::ManualBlocked {
            break;
        }
        let result = drive_current_once_with_claim(&scheduler, &store, current)
            .await
            .expect("advance exclusive run to manual block");
        current = result.into_current_run();
    }
    assert_eq!(
        current_saga_state(&current).0,
        store::RunMode::ManualBlocked
    );
    (fixture, scheduler, store, current)
}

fn fixture_with_manual_resolution_exclusive_side_effect_state() -> Fixture {
    let mut fixture = fixture_with_first_exclusive_side_effect_state();
    let mut typed_spec = fixture.runtime_spec.spec().clone();
    let evidence_schema = fixture.seed_ref.schema_id.clone();
    let authorization = manual_authorization(0xe1);
    typed_spec.saga = spec::SagaPolicySpec::ManualResolution {
        manual: Box::new(spec::ManualResolutionEvidenceSpec {
            evidence_schema: evidence_schema.clone(),
            authorization: authorization.clone(),
        }),
    };
    let certified = certify_fixture_spec(&fixture.runtime_spec, typed_spec, |registry| {
        registry.register_schema_role(
            evidence_schema,
            mfm_certify::CertifiedSchemaRole::ManualResolutionEvidence,
        )?;
        registry.register_manual_authorization_verifier(authorization.verifier_id.clone())?;
        registry.register_operator_authority_snapshot(authorization.authority.clone())
    })
    .expect("certified exclusive manual-resolution runtime spec");
    fixture.runtime_spec = CertifiedRuntimeSpec::new(certified).expect("runtime spec");
    refresh_fixture_run_id(&mut fixture);
    fixture
}

fn resolve_saga_terminal_node(fixture: &Fixture) -> &spec::NodeSpec {
    fixture
        .runtime_spec
        .spec()
        .nodes
        .iter()
        .find(|node| {
            matches!(
                node.framework,
                Some(spec::FrameworkNodeSpec::ResolveSagaTerminal(_))
            )
        })
        .expect("resolve saga terminal node")
}

enum AttemptEvent {
    Start { attempt_no: u32 },
    Interrupted(AttemptId),
}

fn attempt_bundle(
    fixture: &Fixture,
    node: &spec::NodeSpec,
    expected_sequence: store::StreamSeq,
    event: AttemptEvent,
) -> (store::PreparedCommitBundle, AttemptId) {
    let attempt_id = match &event {
        AttemptEvent::Start { attempt_no } => attempt_id(
            &fixture.run_id,
            fixture.runtime_spec.spec_hash(),
            &node.node_id,
            *attempt_no,
        )
        .expect("stale successor attempt id"),
        AttemptEvent::Interrupted(attempt_id) => attempt_id.clone(),
    };
    let (kind, payload) = match event {
        AttemptEvent::Start { attempt_no } => (
            "start",
            events::KernelEventPayload::StateAttemptStarted(events::StateAttemptStarted {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
                attempt_no,
                state_kind: node.state_kind.clone(),
                state_version: node.state_version.clone(),
            }),
        ),
        AttemptEvent::Interrupted(_) => (
            "interrupted",
            events::KernelEventPayload::StateAttemptInterrupted(events::StateAttemptInterrupted {
                spec_hash: fixture.runtime_spec.spec_hash().clone(),
                node_id: node.node_id.clone(),
                attempt_id: attempt_id.clone(),
            }),
        ),
    };
    let request = store_typed_commit_request! {
        run_id: fixture.run_id.clone(),
        expected_next_seq: expected_sequence,
        commit_key: store::CommitKey::new(format!(
            "manual-stale-attempt-{kind}:{}:{expected_sequence}",
            attempt_id
        ))
        .expect("stale successor attempt commit key"),
        payloads: vec![payload],
        required_artifacts: Vec::new(),
        preconditions: store::CommitPreconditions {
            required_run_state: store::RequiredRunState::NotCompleted,
            required_cell_states: vec![store::CellStatePrecondition {
                cell_id: node.output_cell.clone(),
                required: store::RequiredCellState::Absent,
            }],
            ..store::CommitPreconditions::default()
        },
    };
    let plan =
        test_prepared_commit_plan(request, Vec::new()).expect("stale successor attempt plan");
    (
        test_bundle_from_plan(plan).expect("stale successor attempt bundle"),
        attempt_id,
    )
}

enum ManualStaleInjection {
    ExactWithLaterSuffix(Box<store::PreparedCommitBundle>),
    UnrelatedSuccessor(Vec<store::PreparedCommitBundle>),
}

struct CapturedManualExpectationMatrix {
    exact: ManualResolutionCommitExpectationForTest,
    mismatches: Vec<(&'static str, ManualResolutionCommitExpectationForTest)>,
}

impl CapturedManualExpectationMatrix {
    fn from_bundle(bundle: &store::PreparedCommitBundle) -> Result<Self> {
        let exact = ManualResolutionCommitExpectationForTest::from_bundle(bundle)?;
        let mutations = [
            (
                "different run id",
                ManualResolutionExpectationMutationForTest::RunId,
            ),
            (
                "exact manual payload at a later sequence",
                ManualResolutionExpectationMutationForTest::Sequence,
            ),
            (
                "different commit key",
                ManualResolutionExpectationMutationForTest::CommitKey,
            ),
            (
                "extra resource-lane release",
                ManualResolutionExpectationMutationForTest::RecordCount,
            ),
            (
                "different record position",
                ManualResolutionExpectationMutationForTest::RecordPosition,
            ),
            (
                "different manual-resolution outcome",
                ManualResolutionExpectationMutationForTest::ManualOutcome,
            ),
            (
                "different manual-resolution note",
                ManualResolutionExpectationMutationForTest::ManualNote,
            ),
            (
                "different manual-resolution evidence",
                ManualResolutionExpectationMutationForTest::ManualEvidence,
            ),
            (
                "different manual-resolution authorization",
                ManualResolutionExpectationMutationForTest::ManualAuthorization,
            ),
            (
                "different resource-lane release fields",
                ManualResolutionExpectationMutationForTest::ReleaseFields,
            ),
            (
                "different admitted artifact authority",
                ManualResolutionExpectationMutationForTest::ArtifactKey,
            ),
            (
                "different retained artifact object",
                ManualResolutionExpectationMutationForTest::ArtifactObject,
            ),
        ];
        let mismatches = mutations
            .into_iter()
            .map(|(name, mutation)| {
                ManualResolutionCommitExpectationForTest::from_bundle(bundle)?
                    .mutated(mutation)
                    .map(|expectation| (name, expectation))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { exact, mismatches })
    }

    fn assert_matches_only_exact(self, current: &VerifiedCurrentRun) {
        assert!(
            self.exact.matches(current),
            "the exact sealed manual commit must match despite a later legal suffix"
        );
        for (name, mismatch) in self.mismatches {
            assert!(
                !mismatch.matches(current),
                "{name} must not reconcile as the sealed manual commit"
            );
        }
    }
}

struct StaleManualResolutionStore {
    inner: store::AsyncInMemoryRunStore,
    injection: Mutex<Option<ManualStaleInjection>>,
    captured: Mutex<Option<CapturedManualExpectationMatrix>>,
    manual_append_attempts: AtomicUsize,
}

impl StaleManualResolutionStore {
    fn new() -> Self {
        Self {
            inner: store::AsyncInMemoryRunStore::new(),
            injection: Mutex::new(None),
            captured: Mutex::new(None),
            manual_append_attempts: AtomicUsize::new(0),
        }
    }

    fn inject_exact_with_later_suffix(&self, suffix: store::PreparedCommitBundle) {
        let mut injection = self.injection.lock().expect("manual stale-injection lock");
        assert!(injection.is_none(), "manual stale injection already armed");
        *injection = Some(ManualStaleInjection::ExactWithLaterSuffix(Box::new(suffix)));
    }

    fn inject_unrelated_successor(&self, commits: Vec<store::PreparedCommitBundle>) {
        assert!(!commits.is_empty(), "unrelated successor commits");
        let mut injection = self.injection.lock().expect("manual stale-injection lock");
        assert!(injection.is_none(), "manual stale injection already armed");
        *injection = Some(ManualStaleInjection::UnrelatedSuccessor(commits));
    }

    fn manual_append_attempts(&self) -> usize {
        self.manual_append_attempts.load(Ordering::SeqCst)
    }

    fn assert_captured_expectation_matrix(&self, current: &VerifiedCurrentRun) {
        self.captured
            .lock()
            .expect("captured manual expectation lock")
            .take()
            .expect("captured manual expectation")
            .assert_matches_only_exact(current);
    }
}

impl store::RunJournalBackend for StaleManualResolutionStore {
    type Error = store::StoreError;

    fn backend_append<'a>(
        &'a self,
        bundle: store::PreparedCommitBundle,
    ) -> store::AsyncStoreFuture<'a, store::CommitOutcome, Self::Error> {
        Box::pin(async move {
            let is_manual = bundle.request().payloads().iter().any(|payload| {
                matches!(
                    payload,
                    events::KernelEventPayload::ManualResolutionRecorded(_)
                )
            });
            if !is_manual {
                return self.inner.append_prepared_commit_bundle(bundle).await;
            }
            self.manual_append_attempts.fetch_add(1, Ordering::SeqCst);
            let injection = self
                .injection
                .lock()
                .map_err(|_| {
                    store::StoreError::Event("manual stale-injection test lock poisoned".to_owned())
                })?
                .take();
            let Some(injection) = injection else {
                return self.inner.append_prepared_commit_bundle(bundle).await;
            };
            let expected = bundle.request().expected_next_seq();
            let run_id = bundle.request().run_id().clone();
            let captured = CapturedManualExpectationMatrix::from_bundle(&bundle)
                .map_err(|error| store::StoreError::Event(error.to_string()))?;
            *self.captured.lock().map_err(|_| {
                store::StoreError::Event("captured manual expectation lock poisoned".to_owned())
            })? = Some(captured);

            match injection {
                ManualStaleInjection::ExactWithLaterSuffix(suffix) => {
                    self.inner.append_prepared_commit_bundle(bundle).await?;
                    self.inner.append_prepared_commit_bundle(*suffix).await?;
                }
                ManualStaleInjection::UnrelatedSuccessor(commits) => {
                    for commit in commits {
                        self.inner.append_prepared_commit_bundle(commit).await?;
                    }
                }
            }
            Err(store::StoreError::StaleExpectedNextSeq {
                expected,
                actual: self.inner.expected_next_sequence_for_test(&run_id)?,
            })
        })
    }

    fn backend_load<'a>(
        &'a self,
        verifier: store::JournalLoadVerifier,
    ) -> store::AsyncStoreFuture<'a, store::CommittedRunJournal, Self::Error> {
        Box::pin(async move {
            let run_id = verifier.run_id().clone();
            let journal = self.inner.load_committed_journal(&run_id).await?;
            verifier.accept_verified(journal)
        })
    }
}

delegate_execution_claim_store!(StaleManualResolutionStore, delegate_execution_claim_direct);
