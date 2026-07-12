use super::*;

#[test]
fn app_prepare_entry_point_run_launch_records_evidence_and_certifies() {
    let fixture = EntryPointPrepFixture::with_store_scope_hex("50505050505050505050505050505050");
    let registry_digest = fixture
        .entry_point_registry
        .registry_digest()
        .expect("digest");
    let prepared = fixture.prepare_sample_portfolio(None);

    assert_eq!(
        prepared.evidence.resolved_op_id.name.as_str(),
        mfm_op_portfolio_tracker::PORTFOLIO_SNAPSHOT_ENTRY_POINT.name
    );
    assert_eq!(
        prepared.evidence.entry_point_registry_digest,
        registry_digest
    );
    assert_eq!(
        prepared.request.run_id,
        prepared
            .request
            .identity_material
            .derive_run_id()
            .expect("run id")
    );
    assert!(!prepared.request.evidence.config_artifacts.is_empty());
}

#[test]
fn app_prepare_dual_mainnet_portfolio_run_launch_certifies() {
    let fixture = EntryPointPrepFixture::with_store_scope_hex("53535353535353535353535353535353");
    let authored_config = AuthoredConfig::new(
        AuthoredConfigFormat::Toml,
        include_str!("../../../../examples/configs/portfolio-dual-mainnet.toml"),
    )
    .expect("authored config");

    crate::prepare_entry_point_run_launch(crate::EntryPointRunLaunchInput {
        entry_point_registry: &fixture.entry_point_registry,
        public_op_name: portfolio_public_op_name(),
        op_version: None,
        authored_config,
        certification_registry: &fixture.certification_registry,
        store_scope_id: fixture.store_scope_id,
        invocation_key: None,
    })
    .expect("dual-mainnet portfolio launch should certify");
}

#[test]
fn app_prepare_entry_point_run_launch_derives_fresh_and_invocation_stable_run_ids() {
    let fixture = EntryPointPrepFixture::with_store_scope_hex("51515151515151515151515151515151");

    let first = fixture.prepare_sample_portfolio(None);
    let second = fixture.prepare_sample_portfolio(None);
    let explicit = crate::InvocationKey::new("alpha").expect("invocation key");
    let explicit_first = fixture.prepare_sample_portfolio(Some(explicit.clone()));
    let explicit_second = fixture.prepare_sample_portfolio(Some(explicit));
    let mut changed_config: serde_json::Value =
        serde_json::from_str(&sample_portfolio_config_json()).expect("portfolio json");
    changed_config["portfolio"]["portfolio_id"] = serde_json::json!("portfolio_other");
    let changed = fixture.prepare_portfolio_launch(changed_config.to_string(), None);

    assert_ne!(first.request.run_id, second.request.run_id);
    assert_eq!(
        first.request.certified_spec.spec_hash(),
        second.request.certified_spec.spec_hash()
    );
    assert_eq!(
        first.request.certified_spec.spec_hash(),
        explicit_first.request.certified_spec.spec_hash()
    );
    assert_eq!(
        explicit_first.request.run_id,
        explicit_second.request.run_id
    );
    assert_ne!(first.request.run_id, explicit_first.request.run_id);
    assert_eq!(
        explicit_first
            .request
            .identity_material
            .invocation_key_digest
            .as_str(),
        "content:sha256-jcs-v1:e8b2dba1a1730580547875fc9f27a3edf6a5c62660bdc3a6ac06518b487ecd43"
    );
    assert!(!format!("{:?}", explicit_first.request).contains("alpha"));
    assert_ne!(
        first.request.certified_spec.spec_hash(),
        changed.request.certified_spec.spec_hash()
    );
    assert_ne!(first.request.run_id, changed.request.run_id);
}

#[tokio::test]
async fn app_launch_duplicate_same_spec_attaches_without_second_admission() {
    let fixture = EntryPointRunFixture::in_memory().await;
    let invocation_key = crate::InvocationKey::new("mfm.test.app.duplicate-attach").expect("key");
    let first = fixture.prepare_sample_portfolio(Some(invocation_key.clone()));
    let second = fixture.prepare_sample_portfolio(Some(invocation_key));
    let run_id = first.request.run_id.clone();
    let services = fixture.services();

    let first_outcome = services
        .launch_run(first.request)
        .await
        .expect("first launch");
    let second_outcome = services
        .launch_run(second.request)
        .await
        .expect("duplicate launch");

    assert_eq!(
        first_outcome.status(),
        crate::RunLaunchOutcomeStatus::Admitted
    );
    assert_eq!(
        second_outcome.status(),
        crate::RunLaunchOutcomeStatus::Attached
    );
    assert_eq!(
        second_outcome
            .run()
            .expect("attached launch returns run")
            .scheduler_status,
        "observed"
    );
    assert_eq!(fixture.run_admitted_count(&run_id).await, 1);
}

#[tokio::test]
async fn report_only_portfolio_admits_without_live_evm_runtime_config() {
    // After collectors cutover, portfolio_snapshot never constructs live chain providers.
    // Missing runtime config must not block admission as LaunchRunnerUnavailable.
    let fixture = EntryPointRunFixture::in_memory().await;
    let prepared = fixture.prepare_sample_portfolio(None);
    let fact_index = crate::ProjectionFactIndexProvider::new(fixture.store.clone());
    let runners = crate::production_runner_registry(
        Arc::new(fixture.store.clone()),
        Arc::new(fact_index),
        None,
    )
    .expect("runners");
    let services = crate::make_run_services(
        runners,
        fixture.store.clone(),
        fixture.store.clone(),
        fixture.prep.certification_registry.clone(),
    );

    let outcome = services
        .launch_run(prepared.request)
        .await
        .expect("report-only portfolio admits without live EVM runtime config");

    assert_eq!(outcome.status(), crate::RunLaunchOutcomeStatus::Admitted);
}

#[tokio::test]
async fn report_only_portfolio_admits_without_live_btc_runtime_config() {
    // Bitcoin portfolio report is facts-only; BTC JSON-RPC is required for collectors, not report.
    let fixture = EntryPointRunFixture::in_memory().await;
    let prepared = fixture.prepare_bitcoin_portfolio(None);
    let services = fixture.services();

    let outcome = services
        .launch_run(prepared.request)
        .await
        .expect("report-only portfolio admits without live BTC runtime config");

    assert_eq!(outcome.status(), crate::RunLaunchOutcomeStatus::Admitted);
}

#[tokio::test]
async fn missing_platform_facts_follow_normal_report_failure_after_admission() {
    let fixture = EntryPointRunFixture::in_memory().await;
    let prepared = fixture.prepare_sample_portfolio(None);
    let runners = crate::production_runner_registry(
        Arc::new(fixture.store.clone()),
        crate::ProjectionFactIndexProvider::empty_arc(),
        None,
    )
    .expect("runners");
    let services = crate::make_run_services(
        runners,
        fixture.store.clone(),
        fixture.store.clone(),
        fixture.prep.certification_registry.clone(),
    );

    let run_id = prepared.request.run_id.clone();
    let outcome = services
        .launch_run(prepared.request)
        .await
        .expect("missing Platform facts must produce a failed admitted run");

    let run = outcome
        .run()
        .expect("report failure still returns the admitted run");
    assert_eq!(run.run_mode, crate::RunModeStatus::FailedWithoutAcdcClaim);
    assert!(run
        .attempt_dispositions
        .iter()
        .any(|attempt| attempt.error_code.as_deref() == Some("missing_fact")));
    assert!(
        fixture
            .store
            .load_run_stream(&run_id)
            .await
            .expect("run stream")
            .len()
            > 1
    );
}

#[tokio::test]
async fn app_launch_concurrent_duplicates_admit_once_and_dedupe_rest() {
    const LAUNCHERS: usize = 8;

    let fixture = EntryPointRunFixture::in_memory().await;
    let prepared = fixture.prepare_sample_portfolio(None);
    let run_id = prepared.request.run_id.clone();
    let services = fixture.services();
    let barrier = Arc::new(tokio::sync::Barrier::new(LAUNCHERS));
    let mut handles = Vec::new();

    for _ in 0..LAUNCHERS {
        let services = services.clone();
        let request = prepared.request.clone();
        let barrier = barrier.clone();
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            services.launch_run(request).await.expect("launch").status()
        }));
    }

    let mut statuses = Vec::new();
    for handle in handles {
        statuses.push(handle.await.expect("launcher task"));
    }

    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == crate::RunLaunchOutcomeStatus::Admitted)
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| {
                **status == crate::RunLaunchOutcomeStatus::Attached
                    || **status == crate::RunLaunchOutcomeStatus::AlreadyActive
            })
            .count(),
        LAUNCHERS - 1
    );
    assert_eq!(fixture.run_admitted_count(&run_id).await, 1);
}

#[tokio::test]
async fn app_launch_duplicate_reports_already_active_for_live_claim() {
    let fixture = EntryPointRunFixture::in_memory().await;
    let invocation_key =
        crate::InvocationKey::new("mfm.test.app.duplicate-live-claim").expect("key");
    let first = fixture.prepare_sample_portfolio(Some(invocation_key.clone()));
    let second = fixture.prepare_sample_portfolio(Some(invocation_key));
    let run_id = first.request.run_id.clone();
    let execution_scope =
        store::ExecutionClaimScope::from_run_identity_material(&first.request.identity_material);
    let services = fixture.services();
    services
        .launch_run(first.request)
        .await
        .expect("first launch");
    let token = AdmissionToken::new("mfm.test.app.execution_claim").expect("token");
    assert!(matches!(
        fixture
            .store
            .acquire_execution_claim(&execution_scope, &run_id, token)
            .await
            .expect("claim execution"),
        NowaitSkipAdmissionResult::Admitted(_)
    ));

    let outcome = services
        .launch_run(second.request)
        .await
        .expect("duplicate launch");

    assert_eq!(
        outcome.status(),
        crate::RunLaunchOutcomeStatus::AlreadyActive
    );
    assert_eq!(fixture.run_admitted_count(&run_id).await, 1);
}

#[tokio::test]
async fn app_resume_reports_busy_for_live_execution_claim_without_driving() {
    let fixture = EntryPointRunFixture::in_memory().await;
    let prepared = fixture.prepare_sample_portfolio(None);
    let execution_scope =
        store::ExecutionClaimScope::from_run_identity_material(&prepared.request.identity_material);
    let services = fixture.services();
    let run_id = admit_entry_point_run(&services, prepared).await;
    let head_seq = fixture
        .store
        .load_run_stream(&run_id)
        .await
        .expect("run stream")
        .last()
        .expect("admitted event")
        .seq()
        .as_u64();
    let token = AdmissionToken::new("mfm.test.app.execution_claim.resume_busy")
        .expect("execution claim token");
    assert!(matches!(
        fixture
            .store
            .acquire_execution_claim(&execution_scope, &run_id, token)
            .await
            .expect("claim execution"),
        NowaitSkipAdmissionResult::Admitted(_)
    ));

    let resumed = services
        .resume_stored_run(&run_id)
        .await
        .expect("resume response");

    assert_eq!(resumed.scheduler_status, "execution_claim_busy");
    assert_eq!(resumed.head_seq, head_seq);
    assert_eq!(fixture.run_admitted_count(&run_id).await, 1);
}

#[tokio::test]
async fn app_resume_rejects_unavailable_runner_binding_without_claiming() {
    let fixture = EntryPointRunFixture::in_memory().await;
    let prepared = fixture.prepare_sample_portfolio(None);
    let execution_scope =
        store::ExecutionClaimScope::from_run_identity_material(&prepared.request.identity_material);
    let services = fixture.services();
    let run_id = admit_entry_point_run(&services, prepared).await;
    let incompatible_services = crate::make_run_services(
        crate::ErasedRunnerRegistry::new(),
        fixture.store.clone(),
        fixture.store.clone(),
        fixture.prep.certification_registry.clone(),
    );

    let error = incompatible_services
        .resume_stored_run(&run_id)
        .await
        .expect_err("resume should reject unavailable runner bindings");

    assert_eq!(error.code, "LaunchRunnerUnavailable");
    assert!(matches!(
        fixture
            .store
            .execution_claim_status(&execution_scope)
            .await
            .expect("execution claim status"),
        ExecutionClaimStatus::Unclaimed
    ));
    assert_eq!(fixture.run_admitted_count(&run_id).await, 1);
}

#[tokio::test]
async fn app_launch_fails_before_run_admitted_when_runner_binding_is_unavailable() {
    let fixture = EntryPointRunFixture::in_memory().await;
    let prepared = fixture.prepare_sample_portfolio(None);
    let run_id = prepared.request.run_id.clone();
    let services = crate::make_run_services(
        crate::ErasedRunnerRegistry::new(),
        fixture.store.clone(),
        fixture.store.clone(),
        fixture.prep.certification_registry.clone(),
    );

    let err = services
        .launch_run(prepared.request)
        .await
        .expect_err("missing runner binding rejects at ingress");

    assert_eq!(err.code, "LaunchRunnerUnavailable");
    assert_eq!(
        fixture
            .store
            .expected_next_seq(&run_id)
            .await
            .expect("expected next seq")
            .as_u64(),
        1
    );
    assert!(fixture
        .store
        .load_run_stream(&run_id)
        .await
        .expect("run stream")
        .is_empty());
}
