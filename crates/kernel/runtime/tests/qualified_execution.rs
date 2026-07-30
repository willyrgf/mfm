use std::sync::Arc;

use mfm_ids::{StableId, StoreEpoch, StoreScopeId};
use mfm_program::CandidateCertificationErrorKind;
use mfm_qualified_run_test_support::{PreparedQualifiedRun, QualifiedRunFixture};
use mfm_runtime::{
    AdmissionDisposition, AuthorizedAdmissionPlan, DriveOutcome, Runtime, RuntimeError,
};
use mfm_store::{
    open_in_memory, InMemoryRunJournalBackend, QualifiedRunStore, RunAccessAuthorityIssuer,
    RunHistoryReader, StoreError, StoreIdentity,
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

fn provision(store: &QualifiedRunStore<InMemoryRunJournalBackend>, fixture: &QualifiedRunFixture) {
    store
        .provision_configured_value(
            fixture.configured_binding().clone(),
            fixture.configured_bytes().to_vec(),
        )
        .expect("provision qualified fixture configuration");
}

async fn prepare(
    reader: &RunHistoryReader<InMemoryRunJournalBackend>,
    issuer: &RunAccessAuthorityIssuer,
    fixture: &QualifiedRunFixture,
    registry: Arc<mfm_program::QualifiedProgramRegistry>,
) -> PreparedQualifiedRun {
    fixture
        .prepare_on(reader, issuer, registry)
        .await
        .expect("prepare genuine qualified run")
}

fn plan(
    fixture: &QualifiedRunFixture,
    prepared: PreparedQualifiedRun,
) -> (
    Arc<mfm_program::QualifiedProgramRegistry>,
    AuthorizedAdmissionPlan,
) {
    let (registry, authority, append_request_id, artifacts, input, configured, sources) =
        prepared.into_parts();
    let plan = AuthorizedAdmissionPlan::new(
        authority,
        append_request_id,
        fixture.entry_point_id().clone(),
        fixture.entry_point_operation_id().clone(),
        fixture.invocation_identity().clone(),
        artifacts,
        input,
        configured,
        sources,
    );
    (registry, plan)
}

async fn one_fixture_runtime(
    fixture: &QualifiedRunFixture,
) -> (
    Runtime<InMemoryRunJournalBackend>,
    RunHistoryReader<InMemoryRunJournalBackend>,
    RunAccessAuthorityIssuer,
    AuthorizedAdmissionPlan,
) {
    let (store, issuer) = open_in_memory(fixture.store_identity().clone());
    provision(&store, fixture);
    let registry = fixture
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify fixture");
    let (writer, reader) = store.split();
    let prepared = prepare(&reader, &issuer, fixture, Arc::clone(&registry)).await;
    let (_, plan) = plan(fixture, prepared);
    (Runtime::new(writer, registry), reader, issuer, plan)
}

#[tokio::test]
async fn runtime_admission_advances_then_closes() {
    let fixture = QualifiedRunFixture::for_store(store_identity(1), 1).expect("qualified fixture");
    let (runtime, _reader, issuer, plan) = one_fixture_runtime(&fixture).await;
    let admission = runtime.admit(plan).await.expect("runtime admission");
    assert_eq!(admission.disposition(), AdmissionDisposition::NewlyAdmitted);
    assert!(admission.committed().is_some());
    let run_id = admission.run_id().clone();

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
async fn independently_authored_identical_plans_attach_to_one_admission() {
    let fixture = QualifiedRunFixture::for_store(store_identity(2), 2).expect("qualified fixture");
    let (store, issuer) = open_in_memory(fixture.store_identity().clone());
    provision(&store, &fixture);
    let registry = fixture
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify fixture");
    let (writer, reader) = store.split();
    let (_, first_plan) = plan(
        &fixture,
        prepare(&reader, &issuer, &fixture, Arc::clone(&registry)).await,
    );
    let (_, second_plan) = plan(
        &fixture,
        prepare(&reader, &issuer, &fixture, Arc::clone(&registry)).await,
    );
    let runtime = Runtime::new(writer, registry);

    let first = runtime.admit(first_plan).await.expect("first admission");
    let second = runtime
        .admit(second_plan)
        .await
        .expect("idempotent admission");
    assert_eq!(first.disposition(), AdmissionDisposition::NewlyAdmitted);
    assert_eq!(second.disposition(), AdmissionDisposition::Attached);
    assert_eq!(first.run_id(), second.run_id());
    assert_eq!(
        first.committed().expect("known first head").journal_head(),
        second
            .committed()
            .expect("known attached head")
            .journal_head()
    );

    let replay = issuer.authorize_replay(fixture.tenant_scope_id().clone(), first.run_id().clone());
    let journal = reader
        .load_for_replay(&replay)
        .await
        .expect("load admitted run");
    assert_eq!(journal.commits().len(), 1);
}

#[tokio::test]
async fn foreign_registry_plan_is_rejected_before_append() {
    let identity = store_identity(3);
    let current = QualifiedRunFixture::for_store(identity.clone(), 3).expect("current fixture");
    let foreign = QualifiedRunFixture::for_store_with_operation(
        identity,
        4,
        StableId::new("mfm.fixture/foreign-operation").expect("operation id"),
    )
    .expect("foreign fixture");
    let (store, issuer) = open_in_memory(current.store_identity().clone());
    provision(&store, &current);
    provision(&store, &foreign);
    let current_registry = current
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify current fixture");
    let foreign_registry = foreign
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify foreign fixture");
    let (writer, reader) = store.split();
    let (_, foreign_plan) = plan(
        &foreign,
        prepare(&reader, &issuer, &foreign, foreign_registry).await,
    );
    let runtime = Runtime::new(writer, Arc::clone(&current_registry));

    assert!(matches!(
        runtime.admit(foreign_plan).await,
        Err(RuntimeError::CatalogSelection)
    ));

    let (_, current_plan) = plan(
        &current,
        prepare(&reader, &issuer, &current, current_registry).await,
    );
    assert_eq!(
        runtime
            .admit(current_plan)
            .await
            .expect("current admission")
            .disposition(),
        AdmissionDisposition::NewlyAdmitted
    );
}

#[tokio::test]
async fn mismatched_operation_tuple_is_rejected_before_append() {
    let fixture = QualifiedRunFixture::for_store(store_identity(4), 5).expect("qualified fixture");
    let (store, issuer) = open_in_memory(fixture.store_identity().clone());
    provision(&store, &fixture);
    let registry = fixture
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify fixture");
    let (writer, reader) = store.split();
    let prepared = prepare(&reader, &issuer, &fixture, Arc::clone(&registry)).await;
    let (_, authority, append_request_id, artifacts, input, configured, sources) =
        prepared.into_parts();
    let mismatched = AuthorizedAdmissionPlan::new(
        authority,
        append_request_id,
        fixture.entry_point_id().clone(),
        StableId::new("mfm.fixture/substituted-operation").expect("operation id"),
        fixture.invocation_identity().clone(),
        artifacts,
        input,
        configured,
        sources,
    );
    let runtime = Runtime::new(writer, Arc::clone(&registry));
    assert!(matches!(
        runtime.admit(mismatched).await,
        Err(RuntimeError::CatalogSelection)
    ));

    let (_, valid) = plan(
        &fixture,
        prepare(&reader, &issuer, &fixture, registry).await,
    );
    assert_eq!(
        runtime
            .admit(valid)
            .await
            .expect("valid admission")
            .disposition(),
        AdmissionDisposition::NewlyAdmitted
    );
}

#[tokio::test]
async fn foreign_source_proof_is_rejected_by_the_writer_seal() {
    let fixture = QualifiedRunFixture::for_store(store_identity(5), 6).expect("qualified fixture");
    let (store, issuer) = open_in_memory(fixture.store_identity().clone());
    provision(&store, &fixture);
    let registry = fixture
        .qualify_on(&store, &issuer)
        .await
        .expect("qualify fixture");
    let (writer, reader) = store.split();
    let prepared = prepare(&reader, &issuer, &fixture, Arc::clone(&registry)).await;
    let (_, authority, append_request_id, artifacts, input, configured, _sources) =
        prepared.into_parts();

    let (other_store, other_issuer) = open_in_memory(fixture.store_identity().clone());
    provision(&other_store, &fixture);
    fixture
        .qualify_on(&other_store, &other_issuer)
        .await
        .expect("qualify other fixture");
    let (_other_writer, other_reader) = other_store.split();
    let other_authority = other_issuer.authorize_admit(
        fixture.tenant_scope_id().clone(),
        fixture.entry_point_id().clone(),
        fixture.entry_point_operation_id().clone(),
        fixture.invocation_identity().clone(),
    );
    let foreign_sources = other_reader
        .verify_no_admission_sources(&other_authority)
        .await
        .expect("foreign source proof");
    let plan = AuthorizedAdmissionPlan::new(
        authority,
        append_request_id,
        fixture.entry_point_id().clone(),
        fixture.entry_point_operation_id().clone(),
        fixture.invocation_identity().clone(),
        artifacts,
        input,
        configured,
        foreign_sources,
    );
    let runtime = Runtime::new(writer, registry);
    assert!(matches!(
        runtime.admit(plan).await,
        Err(RuntimeError::Store(StoreError::AdmissionAuthorityMismatch))
    ));
}

#[tokio::test]
async fn genuine_two_node_chain_recomputes_both_frames_then_closes() {
    let fixture = QualifiedRunFixture::for_store(store_identity(6), 8)
        .expect("qualified fixture")
        .with_two_node_chain();
    fixture.reset_candidate_callback_counts();
    let (runtime, _reader, issuer, plan) = one_fixture_runtime(&fixture).await;
    let run_id = runtime
        .admit(plan)
        .await
        .expect("admit two-node run")
        .run_id()
        .clone();

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
    let fixture = QualifiedRunFixture::for_store(store_identity(7), 7)
        .expect("qualified fixture")
        .with_panicking_state_callback()
        .expect("panicking fixture");
    let (runtime, _reader, issuer, plan) = one_fixture_runtime(&fixture).await;
    let run_id = runtime
        .admit(plan)
        .await
        .expect("admit panicking run")
        .run_id()
        .clone();

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
