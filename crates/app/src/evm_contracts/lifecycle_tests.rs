use super::*;

#[tokio::test]
async fn app_runner_resumes_replays_and_renders_validate_only_lifecycle_run() {
    let (store, services) = contract_test_services(TestRuntimeFactory::new);
    let request = prepare_evm_entry_point_request(
        &services,
        "evm_contract_validate",
        validate_entry_config_json(),
    )
    .await;
    let (run_id, rendered) = launch_replay_and_render(&services, request, "validate").await;
    assert_execution_claim_unclaimed(&store, &run_id).await;
    assert_rendered_contains(&rendered, "\"valid\":true");
}

#[tokio::test]
async fn app_replay_rejects_missing_validation_report_retained_artifact() {
    let (store, services) = contract_test_services(TestRuntimeFactory::new);
    let request = prepare_evm_entry_point_request(
        &services,
        "evm_contract_validate",
        validate_entry_config_json(),
    )
    .await;
    let (run_id, _) = launch_completed(&services, request, "launch validate lifecycle").await;
    let missing = MissingValidationReportArtifactStore::new(store);
    let replay_services = crate::make_run_read_services(
        missing.clone(),
        missing,
        contract_lifecycle_certification_registry(),
    );

    let error = replay_services
        .verify_replay_for_run(&run_id)
        .await
        .expect_err("replay must reject missing retained validation report artifact");

    assert_eq!(error.code, "ArtifactNotFound");
}

#[tokio::test]
async fn app_resume_completed_run_is_evidence_only_without_live_runners() {
    let store = test_run_store();
    let artifacts = ContractArtifactOverlay::new(Arc::new(store.clone()));
    let certification = contract_lifecycle_certification_registry();
    let runners = contract_lifecycle_runners(TestRuntimeFactory::new(Arc::new(artifacts.clone())));
    let launch_services =
        contract_lifecycle_services(&store, artifacts.clone(), runners, certification.clone());
    let request = prepare_evm_entry_point_request(
        &launch_services,
        "evm_contract_validate",
        validate_entry_config_json(),
    )
    .await;
    let run_id = request.run_id.clone();

    let (_, launched, _) = launch_services
        .launch_run(request)
        .await
        .expect("launch validate lifecycle")
        .into_response_parts();
    let launched = launched.expect("completed launch returns run");
    assert_eq!(launched.run_mode, RunModeStatus::Completed);
    assert_execution_claim_unclaimed(&store, &run_id).await;
    let completed_stream = store
        .load_run_stream(&run_id)
        .await
        .expect("completed stream");

    let resume_services = contract_lifecycle_services(
        &store,
        artifacts,
        ErasedRunnerRegistry::new(),
        certification,
    );
    assert!(
        completed_stream
            .iter()
            .any(|event| matches!(event.payload(), events::KernelEventPayload::RunCompleted(_))),
        "launch must persist terminal completion before evidence-only resume"
    );
    assert_eq!(
        resume_services
            .run_status(&run_id)
            .await
            .expect("observed terminal status before resume")
            .run_mode,
        RunModeStatus::Completed
    );
    let resumed = resume_services
        .resume_stored_run(&run_id)
        .await
        .expect("terminal resume");

    assert_eq!(resumed.run_mode, RunModeStatus::Completed);
    assert_eq!(resumed.scheduler_status, "observed");
    assert_eq!(resumed.head_seq, launched.head_seq);
    assert_execution_claim_unclaimed(&store, &run_id).await;
    assert_eq!(
        store
            .load_run_stream(&run_id)
            .await
            .expect("stream after terminal resume"),
        completed_stream
    );
}

#[tokio::test]
async fn app_resume_runtime_config_ingress_failures_before_claim_or_attempt() {
    assert_resume_runtime_config_ingress_failure(None).await;

    let runtime_config_dir = tempfile::tempdir().expect("runtime config tempdir");
    let runtime_config_path = runtime_config_dir.path().join("runtime.toml");
    std::fs::write(&runtime_config_path, "[evm.sources.bad\n").expect("write malformed config");
    assert_resume_runtime_config_ingress_failure(Some(&runtime_config_path)).await;
}

#[tokio::test]
async fn app_runner_reports_execution_claim_lost_after_renewal_failure() {
    let (store, mut services) = contract_test_services(|artifacts| {
        TestRuntimeFactory::with_chain_identity_delay(artifacts, Duration::from_millis(100))
    });
    services.execution_claim_heartbeat_interval = Duration::from_millis(10);
    let request = prepare_evm_entry_point_request(
        &services,
        "evm_contract_validate",
        validate_entry_config_json(),
    )
    .await;
    let run_id = request.run_id.clone();
    let execution_scope =
        store::ExecutionClaimScope::from_run_identity_material(&request.identity_material);
    let services_for_launch = services.clone();
    let launch = tokio::spawn(async move {
        services_for_launch
            .launch_run(request)
            .await
            .expect("launch response")
            .into_response_parts()
            .1
            .expect("launch returns run")
    });
    let lease = wait_for_live_execution_claim(&store, &execution_scope).await;
    assert!(
        store
            .release_execution_claim(&execution_scope, &run_id, &lease.token)
            .await
            .expect("release execution claim"),
        "test must remove the active claim before renewal"
    );

    let resumed = launch.await.expect("launch task");

    assert_eq!(resumed.scheduler_status, "execution_claim_lost");
}

async fn assert_resume_runtime_config_ingress_failure(
    runtime_config_path: Option<&std::path::Path>,
) {
    let store = test_run_store();
    let artifacts = ContractArtifactOverlay::new(Arc::new(store.clone()));
    let certification = contract_lifecycle_certification_registry();
    let launch_runners =
        contract_lifecycle_runners(TestRuntimeFactory::new(Arc::new(artifacts.clone())));
    let launch_services = contract_lifecycle_services(
        &store,
        artifacts.clone(),
        launch_runners,
        certification.clone(),
    );
    let request = prepare_evm_entry_point_request(
        &launch_services,
        "evm_contract_validate",
        validate_entry_config_json(),
    )
    .await;
    let run_id = request.run_id.clone();
    let runtime_spec =
        CertifiedRuntimeSpec::new(request.certified_spec.clone()).expect("runtime spec");
    let expected_next_seq = store
        .expected_next_seq(&run_id)
        .await
        .expect("expected next seq");
    let launch = launch_services
        .scheduler
        .prepare_run_launch(
            &runtime_spec,
            request.identity_material,
            request.evidence,
            expected_next_seq,
        )
        .expect("prepare admitted-only launch");
    launch_services
        .scheduler
        .start_run(&store, launch)
        .await
        .expect("admit run");
    let admitted_stream = store
        .load_run_stream(&run_id)
        .await
        .expect("admitted stream");
    assert_eq!(
        admitted_stream.len(),
        1,
        "test setup must admit only RunAdmitted"
    );

    let resume_runners = crate::production_runner_registry(
        Arc::new(artifacts.clone()),
        crate::ProjectionFactIndexProvider::empty_arc(),
        runtime_config_path,
    )
    .expect("production runners");
    let resume_services =
        contract_lifecycle_services(&store, artifacts, resume_runners, certification);
    let error = resume_services
        .resume_stored_run(&run_id)
        .await
        .expect_err("resume ingress must reject missing or malformed runtime config");

    assert_eq!(error.class, ErrorClass::BadRequest);
    assert_eq!(error.code, "LaunchRunnerUnavailable");
    assert_eq!(error.message, "A required typed runner is unavailable");
    assert_execution_claim_unclaimed(&store, &run_id).await;
    let stream = store.load_run_stream(&run_id).await.expect("run stream");
    assert_eq!(stream, admitted_stream);
    assert!(
        stream.iter().all(|event| !matches!(
            event.payload(),
            events::KernelEventPayload::StateAttemptStarted(_)
        )),
        "resume ingress failure must not append attempts"
    );
}

#[tokio::test]
async fn app_runner_resumes_replays_and_renders_deploy_lifecycle_run() {
    let signer = test_contract_signer();
    let (_store, services) = contract_test_services(|artifacts| {
        TestRuntimeFactory::with_signer(artifacts, Arc::new(signer.provider), true)
    });
    let request = prepare_evm_entry_point_request(
        &services,
        "evm_contract_deploy",
        deploy_entry_config_json(&signer.address),
    )
    .await;
    let (_, rendered) = launch_replay_and_render(&services, request, "deploy").await;
    assert_rendered_contains(&rendered, "\"address\"");
    assert_rendered_contains(&rendered, "deploy_evidence");
}

#[tokio::test]
async fn replay_diagnostic_accepts_side_effect_submit_node_public_details() {
    let signer = test_contract_signer();
    let (_store, services) = contract_test_services(|artifacts| {
        TestRuntimeFactory::with_signer(artifacts, Arc::new(signer.provider), true)
    });
    let request = prepare_evm_entry_point_request(
        &services,
        "evm_contract_deploy",
        deploy_entry_config_json(&signer.address),
    )
    .await;
    let runtime_spec =
        CertifiedRuntimeSpec::new(request.certified_spec.clone()).expect("runtime spec");
    let verify_node = runtime_spec
        .topological_order()
        .iter()
        .filter_map(|node_id| runtime_spec.node(node_id))
        .find(|node| {
            matches!(
                &node.framework,
                Some(mfm_spec::v1::FrameworkNodeSpec::SideEffectVerify(_))
            )
        })
        .expect("deploy spec has side-effect verifier");
    let Some(mfm_spec::v1::FrameworkNodeSpec::SideEffectVerify(verify)) = &verify_node.framework
    else {
        unreachable!("matched side-effect verifier");
    };
    let submit_node = runtime_spec
        .node(&verify.submit_node_id)
        .expect("submit node is certified");
    assert!(submit_node.side_effect.is_some());
    assert_ne!(
        verify_node.config_ref.schema_id, submit_node.config_ref.schema_id,
        "verifier framework config must not be used as EVM route authority"
    );

    let diagnostic = json!({
        "kind": "provider",
        "version": 1,
        "details": {
            "diagnostic_kind": "provider_source_mismatch",
            "provider_family": "evm",
            "code": "source_mismatch",
            "operation": null,
            "fields": {
                "network_id": "reth-dev",
                "expected_chain_id": 31337,
                "observed_chain_id": 31338,
                "source_ref": "reth-dev",
                "policy_id": "reth-dev",
            },
        },
    });
    let parsed =
        mfm_runtime::RuntimeDiagnostic::from_json(&diagnostic).expect("typed provider diagnostic");
    let expected = events::RedactedJson::new(
        crate::canonical_value_digest(&parsed.public_details_json()).expect("details digest"),
    );
    crate::verify_replay_diagnostic_json(Some(&expected), &diagnostic)
        .expect("verifier diagnostic details are valid public chain-mismatch details");
}

#[tokio::test]
async fn app_runner_resumes_replays_and_renders_full_lifecycle_run() {
    let signer = test_contract_signer();
    let (_store, services) = contract_test_services(|artifacts| {
        TestRuntimeFactory::with_signer(artifacts, Arc::new(signer.provider), false)
    });
    let request = prepare_evm_entry_point_request(
        &services,
        "evm_contract_lifecycle",
        lifecycle_entry_config_json(&signer.address),
    )
    .await;
    let (run_id, rendered) = launch_replay_and_render(&services, request, "full").await;
    assert_rendered_contains(&rendered, "\"valid\":true");
    assert_rendered_contains(&rendered, "configuration_claim");
    assert_rendered_contains(&rendered, "configure_or_import_evidence");
    assert_no_evm_runtime_surface("contract public output", &rendered);

    let stream = services
        .store()
        .load_run_stream(&run_id)
        .await
        .expect("load run stream");
    let reports = retained_validation_reports(&services, &stream).await;
    assert_eq!(
        contract_lifecycle_runner_output_summary(&stream),
        [
            "attempt-output:mfm.evm.contract/context_deploy:side_effect.intent_persisted+side_effect.claimed+resource_lane.claimed+retention_refs_appended[roles=side_effect_intent]",
            "attempt-output:mfm.evm.contract/context_deploy:side_effect.invocation_prepared+side_effect.invocation_started+retention_refs_appended[roles=prepared_invocation]",
            "attempt-output:mfm.evm.contract/context_deploy:side_effect.submission_observed+retention_refs_appended[roles=submission]",
            "attempt-output:mfm.evm.contract/context_deploy:cell_skipped+state_attempt_completed",
            "attempt-output:mfm.evm.contract/context_configure:side_effect.intent_persisted+side_effect.claimed+resource_lane.claimed+retention_refs_appended[roles=side_effect_intent]",
            "attempt-output:mfm.evm.contract/context_configure:side_effect.invocation_prepared+side_effect.invocation_started+retention_refs_appended[roles=prepared_invocation]",
            "attempt-output:mfm.evm.contract/context_configure:side_effect.submission_observed+retention_refs_appended[roles=submission]",
            "attempt-output:mfm.evm.contract/context_configure:cell_skipped+state_attempt_completed",
            "attempt-output:mfm.evm.contract/context_validate:cell_produced+state_attempt_completed+artifact_referenced[role=state_output]+retention_refs_appended[roles=state_output]",
        ]
    );
    assert_eq!(
        reports.len(),
        1,
        "full lifecycle should retain one validation report"
    );
    assert!(
        !reports[0].validation_read_evidence.is_empty(),
        "validation report must retain validation read evidence"
    );
    let mut prepared_artifact_requirements = Vec::new();
    for event in &stream {
        let payload_debug = format!("{:?}", event.payload());
        assert_no_evm_runtime_surface("contract event payload", &payload_debug);
        if let events::KernelEventPayload::SideEffectInvocationPrepared(payload) = event.payload() {
            let artifact_id = payload
                .prepared_artifact_id
                .clone()
                .expect("prepared invocation event has artifact id");
            prepared_artifact_requirements.push(
                event
                    .payload()
                    .artifact_requirements()
                    .into_iter()
                    .find(|requirement| requirement.artifact_id == artifact_id)
                    .expect("prepared invocation event has artifact requirement"),
            );
        }
    }
    assert_eq!(
        prepared_artifact_requirements.len(),
        2,
        "full lifecycle should prepare deploy and configure side effects"
    );

    for requirement in prepared_artifact_requirements {
        let artifact = services
            .artifacts()
            .read_retained_artifact(&requirement)
            .await
            .expect("prepared invocation artifact");
        let evidence = artifact.evidence().clone();
        assert_eq!(
            evidence.artifact_role,
            events::ArtifactRole::PreparedInvocation
        );
        assert_eq!(evidence.schema_id, None);
        assert_eq!(evidence.semantic_type_id, None);
        let bytes = artifact.into_bytes();
        let rendered = std::str::from_utf8(&bytes).expect("prepared invocation artifact is UTF-8");
        assert_no_evm_runtime_surface("prepared invocation artifact", rendered);
        let prepared = serde_json::from_slice::<PreparedContractInvocation>(&bytes)
            .expect("prepared invocation json");
        ensure_prepared_invocation_public(&prepared).expect("prepared invocation is public");
        assert_prepared_invocation_has_unsigned_provenance(&prepared);
    }
}
