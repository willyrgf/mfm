use std::sync::Arc;

use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_ids::{AppendRequestId, RunId, StableId, StoreEpoch, StoreScopeId};
use mfm_program::{QualifiedCandidateIdentity, QualifiedProgramRegistry};
use mfm_qualified_run_test_support::{
    CandidateCallbackCounts, PreparedQualifiedRun, QualifiedRunFixture,
};
use mfm_replay::trace_export::{
    export_portable_run, verify_portable_run_export, ExportKind, PortableRunExport,
    VerifiedPortableExport,
};
use mfm_replay::v1::{
    compare_current, required_export_source_run_ids, verify_recorded_history, ReplayErrorKind,
};
use mfm_runtime::{DriveOutcome, Runtime};
use mfm_store::{
    AppendOutcome, AsyncInMemoryRunStore, ExistingRunAppendMaterial, NewlyAppended,
    ObjectGraphProposal, ProducedObjectRoot, ProducedOutputSlot, RunAccessAuthorityIssuer,
    RunJournalStore, SettlementMaterial, StoreIdentity, TransitionMaterial,
};

struct ClosedQualifiedRun {
    store: AsyncInMemoryRunStore,
    issuer: RunAccessAuthorityIssuer,
    fixture: QualifiedRunFixture,
    registry: Arc<QualifiedProgramRegistry>,
    run_id: RunId,
}

impl ClosedQualifiedRun {
    async fn portable_history(&self) -> VerifiedPortableExport {
        let export_authority = self
            .issuer
            .authorize_export(self.fixture.tenant_scope_id().clone(), self.run_id.clone());
        assert!(
            required_export_source_run_ids(&self.store, &export_authority)
                .await
                .expect("discover source-free export closure")
                .is_empty()
        );
        let export = export_portable_run(&self.store, &export_authority, &[], ExportKind::Semantic)
            .await
            .expect("export genuine semantic history");
        bind_portable_history(self, &export).await
    }

    async fn candidate_registry(
        &self,
        fixture: &QualifiedRunFixture,
    ) -> Arc<QualifiedProgramRegistry> {
        prepare(&self.store, &self.issuer, fixture)
            .await
            .registry()
            .clone()
    }
}

fn store_identity(discriminator: u8) -> StoreIdentity {
    StoreIdentity::new(
        StoreScopeId::new(format!(
            "{}{}",
            StoreScopeId::PREFIX,
            format!("{discriminator:02x}").repeat(16)
        ))
        .expect("valid store scope"),
        StoreEpoch::new(1),
    )
}

fn provision(store: &AsyncInMemoryRunStore, fixture: &QualifiedRunFixture) {
    store
        .provision_configured_value(
            fixture.configured_binding().clone(),
            fixture.configured_bytes().to_vec(),
        )
        .expect("provision genuine configured value");
}

async fn prepare(
    store: &AsyncInMemoryRunStore,
    issuer: &RunAccessAuthorityIssuer,
    fixture: &QualifiedRunFixture,
) -> PreparedQualifiedRun {
    provision(store, fixture);
    fixture
        .prepare_on(store, issuer)
        .await
        .expect("prepare genuine qualified run")
}

async fn admit(
    store: &AsyncInMemoryRunStore,
    prepared: PreparedQualifiedRun,
) -> (Arc<QualifiedProgramRegistry>, RunId) {
    let (registry, authority, append) = prepared.into_parts();
    let outcome = store
        .append_admission(&authority, append)
        .await
        .expect("append genuine qualified admission");
    let AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) = outcome else {
        panic!("genuine admission was not newly committed");
    };
    (registry, admitted.run_id().clone())
}

async fn close_with_runtime(
    identity_discriminator: u8,
    fixture_discriminator: u8,
) -> ClosedQualifiedRun {
    let identity = store_identity(identity_discriminator);
    let (store, issuer) = AsyncInMemoryRunStore::new(identity.clone());
    let fixture =
        QualifiedRunFixture::for_store(identity, fixture_discriminator).expect("qualified fixture");
    close_fixture_with_runtime(store, issuer, fixture).await
}

async fn close_two_node_run(
    identity_discriminator: u8,
    fixture_discriminator: u8,
) -> ClosedQualifiedRun {
    let identity = store_identity(identity_discriminator);
    let (store, issuer) = AsyncInMemoryRunStore::new(identity.clone());
    let fixture = QualifiedRunFixture::for_store(identity, fixture_discriminator)
        .expect("qualified fixture")
        .with_two_node_chain();
    close_fixture_with_runtime(store, issuer, fixture).await
}

async fn close_fixture_with_runtime(
    store: AsyncInMemoryRunStore,
    issuer: RunAccessAuthorityIssuer,
    fixture: QualifiedRunFixture,
) -> ClosedQualifiedRun {
    let (registry, run_id) = admit(&store, prepare(&store, &issuer, &fixture).await).await;
    let runtime = Runtime::new(store.clone(), Arc::clone(&registry));

    for _ in 0..8 {
        let outcome = runtime
            .drive_once(issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id.clone()))
            .await
            .expect("drive genuine qualified run");
        match outcome {
            DriveOutcome::Advanced { .. } => {}
            DriveOutcome::Closed { .. } => {
                return ClosedQualifiedRun {
                    store,
                    issuer,
                    fixture,
                    registry,
                    run_id,
                };
            }
            DriveOutcome::Waiting { reason, .. } => {
                panic!("source-free genuine run unexpectedly waited: {reason:?}");
            }
        }
    }
    panic!("source-free genuine run did not close within its certified node bound");
}

async fn close_panicking_history_manually(
    identity_discriminator: u8,
    fixture_discriminator: u8,
) -> ClosedQualifiedRun {
    let identity = store_identity(identity_discriminator);
    let (store, issuer) = AsyncInMemoryRunStore::new(identity.clone());
    let fixture = QualifiedRunFixture::for_store(identity, fixture_discriminator)
        .expect("qualified fixture")
        .with_panicking_state_callback()
        .expect("panicking fixture");
    let (registry, run_id) = admit(&store, prepare(&store, &issuer, &fixture).await).await;

    let drive_authority = issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id.clone());
    let view = store
        .load_committed_journal(&drive_authority)
        .await
        .expect("load genuine admitted history")
        .verify_recorded_history()
        .expect("verify genuine admitted history");
    let [node] = view.certified_spec().nodes() else {
        panic!("panicking fixture must certify exactly one node");
    };
    let [output_slot] = node.settlement_contract().output_slots() else {
        panic!("panicking fixture must certify exactly one output");
    };
    let frame = store
        .prepare_frame(&drive_authority, &view, node.node_id())
        .await
        .expect("prepare verified historical frame");
    let configured: serde_json::Value =
        serde_json::from_slice(fixture.configured_bytes()).expect("fixture configuration JSON");
    let output = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::json!({
            "value": configured
                .get("value")
                .and_then(serde_json::Value::as_u64)
                .expect("fixture numeric value")
        })
        .to_string(),
    )
    .expect("canonical historical output");
    let append = store
        .prepare_append(
            &drive_authority,
            &view,
            AppendRequestId::new(format!(
                "replay-fixture/manual-settlement/{fixture_discriminator:02x}"
            ))
            .expect("manual settlement append identity"),
            ExistingRunAppendMaterial::Transition(Box::new(TransitionMaterial::PureSettled {
                prepared_frame: Box::new(frame),
                settlement: SettlementMaterial::Succeeded {
                    output_roots: vec![ProducedOutputSlot::new(
                        output_slot.output_ordinal(),
                        output_slot.field_path().clone(),
                        ProducedObjectRoot::new(output_slot.value_contract().clone(), output),
                    )],
                    fact_roots: Vec::new(),
                },
                object_graph: ObjectGraphProposal::empty(),
            })),
        )
        .expect("prepare genuine recorded settlement");
    assert!(matches!(
        store
            .append(&drive_authority, append)
            .await
            .expect("append genuine recorded settlement"),
        AppendOutcome::NewlyAppended(NewlyAppended::Transition(_))
    ));

    ClosedQualifiedRun {
        store,
        issuer,
        fixture,
        registry,
        run_id,
    }
}

async fn bind_portable_history(
    run: &ClosedQualifiedRun,
    export: &PortableRunExport,
) -> VerifiedPortableExport {
    let offline = verify_portable_run_export(export.as_bytes(), export.digest())
        .expect("independently verify generated portable export");
    assert_eq!(offline.run_id(), &run.run_id);
    assert_eq!(offline.export_kind(), ExportKind::Semantic);
    let replay_authority = run
        .issuer
        .authorize_replay(run.fixture.tenant_scope_id().clone(), run.run_id.clone());
    let verified = verify_recorded_history(&run.store, &replay_authority)
        .await
        .expect("verify callback-free recorded history");
    verified
        .verify_portable_export(
            export.as_bytes(),
            &export.content_ref().expect("portable export content ref"),
        )
        .expect("bind semantic export to verified history")
}

fn parse_result(result: &mfm_replay::v1::CanonicalReplayResult) -> serde_json::Value {
    serde_json::from_slice(result.as_bytes()).expect("canonical candidate result JSON")
}

fn assert_content_ref(value: &serde_json::Value, expected: &mfm_ids::ContentRef) {
    assert_eq!(
        value.get("schema_id").and_then(serde_json::Value::as_str),
        Some(expected.schema_id().as_str())
    );
    assert_eq!(
        value
            .get("content_digest")
            .and_then(serde_json::Value::as_str),
        Some(expected.content_digest().as_str())
    );
}

fn assert_candidate_identities(
    result: &serde_json::Value,
    admitted: &QualifiedProgramRegistry,
    candidate: &QualifiedCandidateIdentity,
) {
    let report = result.get("report").expect("candidate report");
    assert_content_ref(
        report
            .get("admitted_executable_identity_ref")
            .expect("admitted executable ref"),
        admitted.executable_identity_ref(),
    );
    for (field, expected) in [
        (
            "candidate_executable_identity_ref",
            candidate.candidate_executable_identity_ref(),
        ),
        (
            "candidate_planning_profile_ref",
            candidate.planning_profile_ref(),
        ),
        (
            "candidate_planner_contract_ref",
            candidate.planner_contract_ref(),
        ),
        (
            "candidate_planner_implementation_ref",
            candidate.planner_implementation_ref(),
        ),
        (
            "candidate_state_implementation_manifest_ref",
            candidate.state_implementation_manifest_ref(),
        ),
        (
            "candidate_capability_binding_manifest_ref",
            candidate.capability_binding_manifest_ref(),
        ),
    ] {
        assert_content_ref(
            report.get(field).expect("candidate identity field"),
            expected,
        );
    }
    assert_content_ref(
        result
            .get("candidate_executable_identity_ref")
            .expect("top-level candidate executable ref"),
        candidate.candidate_executable_identity_ref(),
    );
}

#[tokio::test]
async fn exact_current_candidate_agrees_and_binds_every_identity() {
    let run = close_with_runtime(0x31, 0x41).await;
    let historical = run.portable_history().await;
    let candidate = run
        .registry
        .select_current_candidate(run.fixture.entry_point_operation_id())
        .expect("select exact current candidate");

    let result = compare_current(&historical, &run.registry).expect("compare exact candidate");
    let result = parse_result(&result);
    assert_eq!(
        result.get("kind").and_then(serde_json::Value::as_str),
        Some("candidate_comparison")
    );
    assert_candidate_identities(&result, &run.registry, &candidate);
    let report = result.get("report").expect("candidate report");
    assert_eq!(
        report.get("plan").and_then(serde_json::Value::as_str),
        Some("agrees")
    );
    let transitions = report
        .get("transitions")
        .and_then(serde_json::Value::as_array)
        .expect("candidate transition results");
    assert_eq!(transitions.len(), 1);
    assert_eq!(
        transitions[0]
            .get("result")
            .and_then(serde_json::Value::as_str),
        Some("agrees")
    );
}

#[tokio::test]
async fn absent_current_operation_is_candidate_unavailable() {
    let run = close_with_runtime(0x32, 0x42).await;
    let historical = run.portable_history().await;
    let unavailable = QualifiedRunFixture::for_store_with_operation(
        run.fixture.store_identity().clone(),
        0x42,
        StableId::new("mfm.fixture/unavailable-replay-operation").expect("operation"),
    )
    .expect("unavailable candidate fixture");
    let registry = run.candidate_registry(&unavailable).await;

    let error =
        compare_current(&historical, &registry).expect_err("candidate operation must be absent");
    assert_eq!(error.kind(), ReplayErrorKind::CandidateUnavailable);
    assert_eq!(error.code(), "MFM_REPLAY_CANDIDATE_UNAVAILABLE");
}

#[tokio::test]
async fn panicking_candidate_is_redaction_safe_execution_failure() {
    let run = close_panicking_history_manually(0x33, 0x43).await;
    let historical = run.portable_history().await;

    let error =
        compare_current(&historical, &run.registry).expect_err("candidate callback must panic");
    assert_eq!(error.kind(), ReplayErrorKind::CandidateExecutionFailed);
    assert_eq!(error.code(), "MFM_REPLAY_CANDIDATE_EXECUTION_FAILED");
    assert!(!error
        .to_string()
        .contains("qualified fixture state callback"));
    assert!(!format!("{error:?}").contains("qualified fixture state callback"));
}

#[tokio::test]
async fn candidate_frame_decode_integrity_is_comparison_integrity_failure() {
    let run = close_with_runtime(0x34, 0x44).await;
    let historical = run.portable_history().await;
    let integrity_failing =
        QualifiedRunFixture::for_store(run.fixture.store_identity().clone(), 0x44)
            .expect("candidate fixture")
            .with_integrity_failing_state_callback();
    let registry = run.candidate_registry(&integrity_failing).await;

    let error = compare_current(&historical, &registry)
        .expect_err("candidate typed frame decode must fail integrity");
    assert_eq!(error.kind(), ReplayErrorKind::ComparisonIntegrityFailed);
    assert_eq!(error.code(), "MFM_REPLAY_COMPARISON_INTEGRITY_FAILED");
}

#[tokio::test]
async fn global_not_comparable_skips_every_candidate_callback() {
    let run = close_two_node_run(0x35, 0x45).await;
    let historical = run.portable_history().await;
    let incompatible = QualifiedRunFixture::for_store(run.fixture.store_identity().clone(), 0x45)
        .expect("candidate fixture")
        .with_two_node_divergent_candidate()
        .with_incompatible_planning_profile();
    incompatible.reset_candidate_callback_counts();
    let registry = run.candidate_registry(&incompatible).await;

    let result =
        compare_current(&historical, &registry).expect("incompatible candidate must not execute");
    let result = parse_result(&result);
    let report = result.get("report").expect("candidate report");
    assert_eq!(
        report.get("plan").and_then(serde_json::Value::as_str),
        Some("not_comparable")
    );
    let transitions = report
        .get("transitions")
        .and_then(serde_json::Value::as_array)
        .expect("candidate transition results");
    assert_eq!(transitions.len(), 2);
    assert!(transitions.iter().all(|transition| {
        transition.get("result").and_then(serde_json::Value::as_str) == Some("not_comparable")
    }));
    assert_eq!(
        incompatible.candidate_callback_counts(),
        CandidateCallbackCounts {
            first_state: 0,
            second_state: 0,
        }
    );
}

#[tokio::test]
async fn transition_comparisons_use_recorded_frames_not_prior_candidate_outputs() {
    let run = close_two_node_run(0x36, 0x46).await;
    let historical = run.portable_history().await;
    let divergent = QualifiedRunFixture::for_store(run.fixture.store_identity().clone(), 0x46)
        .expect("candidate fixture")
        .with_two_node_divergent_candidate();
    divergent.reset_candidate_callback_counts();
    let registry = run.candidate_registry(&divergent).await;

    let result = compare_current(&historical, &registry).expect("compare divergent candidate");
    let result = parse_result(&result);
    let report = result.get("report").expect("candidate report");
    assert_eq!(
        report.get("plan").and_then(serde_json::Value::as_str),
        Some("agrees")
    );
    let transition_results = report
        .get("transitions")
        .and_then(serde_json::Value::as_array)
        .expect("candidate transition results")
        .iter()
        .map(|transition| {
            transition
                .get("result")
                .and_then(serde_json::Value::as_str)
                .expect("transition verdict")
        })
        .collect::<Vec<_>>();
    assert_eq!(transition_results, ["differs", "agrees"]);
    assert_eq!(
        divergent.candidate_callback_counts(),
        CandidateCallbackCounts {
            first_state: 1,
            second_state: 1,
        }
    );
}
