use super::*;

#[tokio::test]
async fn execution_claim_acquire_busy_and_release_are_token_matched() {
    let (store, schema) = test_store().await;
    let run = run_id(6);
    let holder = admission_token("mfm.test.execution_claim.holder");
    let other = admission_token("mfm.test.execution_claim.other");

    mfm_store::v1::test_support::assert_execution_claim_token_lifecycle_for_test(
        &store, &run, holder, other,
    )
    .await
    .expect("execution claim lifecycle");

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn execution_claim_expiry_requires_explicit_reap() {
    let (store, schema) = test_store().await;
    let run = run_id(7);
    let scope = execution_claim_scope(7);
    let holder = admission_token("mfm.test.execution_claim.expired_holder");
    let other = admission_token("mfm.test.execution_claim.expired_other");

    let lease = acquire_execution_claim_lease(&store, &scope, &run, holder.clone()).await;
    expire_execution_claim_row(&store, &run).await;
    assert!(matches!(
        store
            .execution_claim_status(&scope)
            .await
            .expect("expired execution claim status"),
        ExecutionClaimStatus::Expired(status) if status.token == lease.token
    ));

    let busy = store
        .acquire_execution_claim(&scope, &run, other.clone())
        .await
        .expect("expired holder still blocks acquire");
    let NowaitSkipAdmissionResult::Busy(busy) = busy else {
        panic!("expired holder should remain busy before explicit reap");
    };
    assert_eq!(busy.holder.expect("expired holder").token, lease.token);

    let expired = store
        .expired_execution_claims()
        .await
        .expect("expired execution claims");
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].run_id, run);
    assert_eq!(expired[0].lease.token, holder);

    assert!(!store
        .reap_expired_execution_claim(&scope, &run, &other)
        .await
        .expect("wrong-token reap"));
    assert!(matches!(
        store
            .acquire_execution_claim(&scope, &run, other.clone())
            .await
            .expect("busy after wrong-token reap"),
        NowaitSkipAdmissionResult::Busy(_)
    ));

    assert!(store
        .reap_expired_execution_claim(&scope, &run, &holder)
        .await
        .expect("matching-token reap"));
    assert!(matches!(
        store
            .acquire_execution_claim(&scope, &run, other)
            .await
            .expect("acquire after explicit reap"),
        NowaitSkipAdmissionResult::Admitted(_)
    ));

    drop_schema(&store, &schema).await;
}

#[tokio::test]
async fn execution_claim_stale_token_cannot_reap_newer_holder() {
    let (store, schema) = test_store().await;
    let run = run_id(8);
    let scope = execution_claim_scope(8);
    let first = admission_token("mfm.test.execution_claim.first");
    let second = admission_token("mfm.test.execution_claim.second");
    let third = admission_token("mfm.test.execution_claim.third");

    acquire_execution_claim_lease(&store, &scope, &run, first.clone()).await;
    expire_execution_claim_row(&store, &run).await;
    assert!(store
        .reap_expired_execution_claim(&scope, &run, &first)
        .await
        .expect("first reap"));

    let second_lease = acquire_execution_claim_lease(&store, &scope, &run, second.clone()).await;
    expire_execution_claim_row(&store, &run).await;

    assert!(!store
        .reap_expired_execution_claim(&scope, &run, &first)
        .await
        .expect("stale-token reap"));
    let busy = store
        .acquire_execution_claim(&scope, &run, third)
        .await
        .expect("newer holder still blocks after stale reap");
    let NowaitSkipAdmissionResult::Busy(busy) = busy else {
        panic!("newer holder should remain busy");
    };
    assert_eq!(busy.holder.expect("newer holder").token, second_lease.token);

    assert!(store
        .reap_expired_execution_claim(&scope, &run, &second)
        .await
        .expect("newer holder reap"));

    drop_schema(&store, &schema).await;
}

async fn acquire_execution_claim_lease(
    store: &PostgresRunStore,
    scope: &mfm_store::v1::ExecutionClaimScope,
    run_id: &RunId,
    token: AdmissionToken,
) -> AdmissionLease {
    let admitted = store
        .acquire_execution_claim(scope, run_id, token)
        .await
        .expect("acquire execution claim");
    let NowaitSkipAdmissionResult::Admitted(lease) = admitted else {
        panic!("execution claim should be admitted");
    };
    lease
}
fn admission_token(value: &str) -> AdmissionToken {
    AdmissionToken::new(value).expect("admission token")
}

fn execution_claim_scope(byte: u8) -> mfm_store::v1::ExecutionClaimScope {
    let identity = run_identity_material_for_test(spec_hash(byte), &store_scope_hex(byte));
    mfm_store::v1::ExecutionClaimScope::from_run_identity_material(&identity)
}

async fn expire_execution_claim_row(store: &PostgresRunStore, run_id: &RunId) {
    sqlx::query(
        "UPDATE admission_lane \
         SET lease_expires_at = statement_timestamp() - make_interval(secs => 1), \
             updated_at = statement_timestamp() \
         WHERE class = 'execution_claim' AND execution_run_id = $1",
    )
    .bind(run_id.as_str())
    .execute(&store.pool)
    .await
    .expect("expire execution claim row");
}
