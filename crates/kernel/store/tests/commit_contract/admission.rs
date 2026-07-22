use super::*;

#[test]
fn admission_lane_constructors_bind_class_to_mode() {
    let resource_lane =
        ResourceAdmissionLane::from_resource_key_evidence(&resource_key("admission-wallet", 240))
            .expect("resource admission lane");
    assert_eq!(resource_lane.class(), AdmissionLaneClass::ResourceLane);
    assert_eq!(resource_lane.mode(), AdmissionLaneMode::WaitFifo);

    let execution_lane = ExecutionClaimAdmissionLane::from_scope(&execution_claim_scope(240))
        .expect("execution admission lane");
    assert_eq!(execution_lane.class(), AdmissionLaneClass::ExecutionClaim);
    assert_eq!(execution_lane.mode(), AdmissionLaneMode::NowaitSkip);
    assert_ne!(
        resource_lane.id().as_bytes(),
        execution_lane.id().as_bytes()
    );
}

#[test]
fn admission_lane_helpers_are_stable_and_domain_separated() {
    let run_id = run_id(241);
    let resource_key = resource_key("admission-wallet-token", 241);
    let lane =
        ResourceAdmissionLane::from_resource_key_evidence(&resource_key).expect("resource lane");
    let ledger_key = side_effect_ledger_key_with_suffix(241);
    let mut intent = events::ResourceLaneClaimIntent {
        spec_hash: spec_hash(241),
        node_id: node_id(241),
        attempt_id: attempt_id(241),
        ledger_key: ledger_key.clone(),
        ledger_purpose: events::SideEffectLedgerPurpose::Forward,
        pair_id: side_effect_pair_id(),
        pair_role: events::SideEffectPairRole::Submit,
        invocation_epoch: 1,
        resource_key,
        requirement_digest: content_digest(241),
        resolved_by_capability_impl: events::RunnerFactoryId::new("mfm.test.runner")
            .expect("runner id"),
    };

    let first = resource_wait_fifo_admission_token(&run_id, &lane, &intent).expect("first token");
    let retry = resource_wait_fifo_admission_token(&run_id, &lane, &intent).expect("retry token");
    intent.invocation_epoch = 2;
    let different_epoch =
        resource_wait_fifo_admission_token(&run_id, &lane, &intent).expect("different token");

    assert_eq!(first, retry);
    assert_ne!(first, different_epoch);
    assert_eq!(
        admission_waiter_id(&first)
            .expect("first waiter id")
            .as_str(),
        admission_waiter_id(&retry)
            .expect("retry waiter id")
            .as_str()
    );
    assert!(admission_waiter_id(&first)
        .expect("waiter id")
        .as_str()
        .starts_with("admission_waiter:"));

    let resource_lock =
        admission_advisory_lock_key(&lane.erased_key()).expect("resource advisory lock");
    let resource_retry_lock =
        admission_advisory_lock_key(&lane.erased_key()).expect("resource advisory lock retry");
    let execution_lock = admission_advisory_lock_key(
        &ExecutionClaimAdmissionLane::from_scope(&execution_claim_scope(241))
            .expect("execution lane")
            .erased_key(),
    )
    .expect("execution advisory lock");

    assert_eq!(resource_lock, resource_retry_lock);
    assert_ne!(resource_lock, execution_lock);
}

#[test]
fn execution_claim_contract_defaults_are_explicit() {
    assert_eq!(EXECUTION_CLAIM_LEASE_TTL_SECS, 60);
    assert_eq!(EXECUTION_CLAIM_HEARTBEAT_INTERVAL_SECS, 20);
}

#[test]
fn store_scope_id_contract_is_store_owned_shape() {
    let store_scope = StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef0123456789abcdef")
        .expect("store scope id");
    assert_eq!(
        store_scope.as_str(),
        "mfm.store_scope.v1:0123456789abcdef0123456789abcdef"
    );
    assert!(
        StoreScopeId::new("mfm.store_scope.unsupported:0123456789abcdef0123456789abcdef").is_err()
    );
    assert!(StoreScopeId::new("mfm.store_scope.v1:0123456789ABCDEF0123456789abcdef").is_err());
    assert!(StoreScopeId::new("mfm.store_scope.v1:0123456789abcdef").is_err());
}

#[test]
fn in_memory_store_exposes_store_owned_store_scope() {
    let store = AsyncInMemoryRunStore::default();
    let store_scope =
        poll_ready_store_future(store.load_store_scope_id()).expect("load in-memory store scope");
    assert_eq!(
        store_scope.as_str(),
        "mfm.store_scope.v1:00000000000000000000000000000000"
    );
}

#[test]
fn in_memory_execution_claims_are_token_matched() {
    let store = AsyncInMemoryRunStore::default();
    let run_id = run_id(242);
    let holder = AdmissionToken::new("mfm.test.execution_claim.holder").expect("holder token");
    let other = AdmissionToken::new("mfm.test.execution_claim.other").expect("other token");

    poll_ready_store_future(
        mfm_store::v1::test_support::assert_execution_claim_token_lifecycle_for_test(
            &store, &run_id, holder, other,
        ),
    )
    .expect("execution claim lifecycle");
}
