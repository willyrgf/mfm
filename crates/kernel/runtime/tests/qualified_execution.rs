use std::sync::Arc;

use mfm_ids::{StableId, StoreEpoch, StoreScopeId};
use mfm_program::CandidateCertificationErrorKind;
use mfm_qualified_run_test_support::{PreparedQualifiedRun, QualifiedRunFixture};
use mfm_runtime::{DriveOutcome, DriveWaitReason, Runtime, RuntimeError};
use mfm_store::{
    AppendOutcome, AsyncInMemoryRunStore, NewlyAppended, RunAccessAuthorityIssuer, RunJournalStore,
    StoreIdentity,
};

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
        .expect("provision qualified fixture configuration");
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
) -> (Arc<mfm_program::QualifiedProgramRegistry>, mfm_ids::RunId) {
    let (registry, authority, append) = prepared.into_parts();
    let outcome = store
        .append_admission(&authority, append)
        .await
        .expect("append genuine qualified run");
    let AppendOutcome::NewlyAppended(NewlyAppended::RunAdmitted(admitted)) = outcome else {
        panic!("qualified fixture admission was not newly committed");
    };
    (registry, admitted.run_id().clone())
}

#[tokio::test]
async fn genuine_qualified_run_advances_then_closes() {
    let identity = store_identity(1);
    let (store, issuer) = AsyncInMemoryRunStore::new(identity.clone());
    let fixture = QualifiedRunFixture::for_store(identity, 1).expect("qualified fixture");
    let prepared = prepare(&store, &issuer, &fixture).await;
    let (registry, run_id) = admit(&store, prepared).await;
    let runtime = Runtime::new(store, registry);

    let first = runtime
        .drive_once(issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id.clone()))
        .await
        .expect("first genuine drive");
    assert!(matches!(first, DriveOutcome::Advanced { .. }));

    let second = runtime
        .drive_once(issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id))
        .await
        .expect("second genuine drive");
    assert!(matches!(second, DriveOutcome::Closed { .. }));
}

#[tokio::test]
async fn unavailable_candidate_waits_on_an_operational_block() {
    let identity = store_identity(2);
    let (store, issuer) = AsyncInMemoryRunStore::new(identity.clone());
    let recorded = QualifiedRunFixture::for_store(identity.clone(), 2).expect("recorded fixture");
    let (recorded_registry, run_id) =
        admit(&store, prepare(&store, &issuer, &recorded).await).await;
    drop(recorded_registry);

    let unavailable = QualifiedRunFixture::for_store_with_operation(
        identity,
        3,
        StableId::new("mfm.fixture/unavailable-operation").expect("operation id"),
    )
    .expect("unavailable fixture");
    let current = prepare(&store, &issuer, &unavailable).await;
    let registry = Arc::clone(current.registry());
    let runtime = Runtime::new(store, registry);

    let outcome = runtime
        .drive_once(issuer.authorize_drive(recorded.tenant_scope_id().clone(), run_id))
        .await
        .expect("classify unavailable candidate");
    assert!(matches!(
        outcome,
        DriveOutcome::Waiting {
            reason: DriveWaitReason::OperationalBlock,
            ..
        }
    ));
}

#[tokio::test]
async fn incompatible_candidate_waits_on_an_integrity_block() {
    let identity = store_identity(3);
    let (store, issuer) = AsyncInMemoryRunStore::new(identity.clone());
    let recorded = QualifiedRunFixture::for_store(identity.clone(), 4).expect("recorded fixture");
    let (recorded_registry, run_id) =
        admit(&store, prepare(&store, &issuer, &recorded).await).await;
    drop(recorded_registry);

    let incompatible = QualifiedRunFixture::for_store(identity, 5)
        .expect("candidate fixture")
        .with_incompatible_planning_profile();
    let current = prepare(&store, &issuer, &incompatible).await;
    let registry = Arc::clone(current.registry());
    let runtime = Runtime::new(store, registry);

    let outcome = runtime
        .drive_once(issuer.authorize_drive(recorded.tenant_scope_id().clone(), run_id))
        .await
        .expect("classify incompatible candidate");
    assert!(matches!(
        outcome,
        DriveOutcome::Waiting {
            reason: DriveWaitReason::IntegrityBlock,
            ..
        }
    ));
}

#[tokio::test]
async fn same_identity_callback_decode_failure_is_an_integrity_block() {
    let identity = store_identity(5);
    let (store, issuer) = AsyncInMemoryRunStore::new(identity.clone());
    let recorded = QualifiedRunFixture::for_store(identity.clone(), 7).expect("recorded fixture");
    let (recorded_registry, run_id) =
        admit(&store, prepare(&store, &issuer, &recorded).await).await;
    drop(recorded_registry);

    let integrity_failing = QualifiedRunFixture::for_store(identity, 7)
        .expect("candidate fixture")
        .with_integrity_failing_state_callback();
    let current = prepare(&store, &issuer, &integrity_failing).await;
    let registry = Arc::clone(current.registry());
    let runtime = Runtime::new(store, registry);

    let outcome = runtime
        .drive_once(issuer.authorize_drive(recorded.tenant_scope_id().clone(), run_id))
        .await
        .expect("classify callback decode failure");
    assert!(matches!(
        outcome,
        DriveOutcome::Waiting {
            reason: DriveWaitReason::IntegrityBlock,
            ..
        }
    ));
}

#[tokio::test]
async fn genuine_two_node_chain_recomputes_both_frames_then_closes() {
    let identity = store_identity(6);
    let (store, issuer) = AsyncInMemoryRunStore::new(identity.clone());
    let fixture = QualifiedRunFixture::for_store(identity, 8)
        .expect("qualified fixture")
        .with_two_node_chain();
    fixture.reset_candidate_callback_counts();
    let (registry, run_id) = admit(&store, prepare(&store, &issuer, &fixture).await).await;
    let runtime = Runtime::new(store, registry);

    for expected_step in 1..=2 {
        let outcome = runtime
            .drive_once(issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id.clone()))
            .await
            .unwrap_or_else(|error| panic!("genuine drive {expected_step} failed: {error}"));
        assert!(
            matches!(outcome, DriveOutcome::Advanced { .. }),
            "genuine drive {expected_step} did not advance"
        );
    }
    let closed = runtime
        .drive_once(issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id))
        .await
        .expect("close genuine two-node run");
    assert!(matches!(closed, DriveOutcome::Closed { .. }));
    assert_eq!(fixture.candidate_callback_counts(), Default::default());
}

#[tokio::test]
async fn callback_panic_is_a_redaction_safe_execution_failure() {
    let identity = store_identity(7);
    let (store, issuer) = AsyncInMemoryRunStore::new(identity.clone());
    let fixture = QualifiedRunFixture::for_store(identity, 6)
        .expect("qualified fixture")
        .with_panicking_state_callback()
        .expect("panicking fixture");
    let (registry, run_id) = admit(&store, prepare(&store, &issuer, &fixture).await).await;
    let runtime = Runtime::new(store, registry);

    let error = runtime
        .drive_once(issuer.authorize_drive(fixture.tenant_scope_id().clone(), run_id))
        .await
        .expect_err("panicking callback must fail");
    let RuntimeError::CandidateCertification(candidate) = &error else {
        panic!("unexpected runtime error: {error}");
    };
    assert_eq!(
        candidate.kind(),
        CandidateCertificationErrorKind::ExecutionFailed
    );
    assert_eq!(
        error.to_string(),
        "the selected current candidate failed during execution"
    );
    assert!(!format!("{error:?}").contains("qualified fixture state callback"));
}
