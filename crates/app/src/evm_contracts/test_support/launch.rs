use super::*;

pub(super) fn evm_forbidden_runtime_terms() -> Vec<String> {
    vec![
        ["raw", "_transaction"].concat(),
        ["raw", "_tx"].concat(),
        ["signed", "_payload"].concat(),
        "signature".to_owned(),
        ["key", "store"].concat(),
        ["key", "store", "_path"].concat(),
        ["private", "_key"].concat(),
        ["private", " key"].concat(),
        ["pass", "word"].concat(),
        ["mne", "monic"].concat(),
        ["seed", "_phrase"].concat(),
        ["rpc", "_url"].concat(),
        ["rpc", " url"].concat(),
        ["end", "point"].concat(),
        ["provider", "_kind"].concat(),
        ["provider", " kind"].concat(),
        ["author", "ization"].concat(),
        TEST_SIGNER_HEX.to_owned(),
    ]
}

pub(super) fn assert_no_evm_runtime_surface(label: &str, rendered: &str) {
    let rendered = rendered.to_ascii_lowercase();
    for forbidden in evm_forbidden_runtime_terms() {
        assert!(
            !rendered.contains(&forbidden),
            "{label} contains forbidden EVM runtime surface"
        );
    }
}

pub(super) fn assert_prepared_invocation_has_unsigned_provenance(
    prepared: &PreparedContractInvocation,
) {
    assert!(!prepared.signer_ref.is_empty());
    assert!(prepared.expected_signer_address.starts_with("0x"));
    assert!(
        !prepared.transactions.is_empty(),
        "prepared invocation must retain transaction provenance"
    );
    for transaction in &prepared.transactions {
        assert!(transaction
            .data_digest
            .starts_with("content:sha256-jcs-v1:"));
        assert!(transaction.signing_digest.starts_with("0x"));
        assert_eq!(transaction.signing_digest.len(), 66);
        assert!(transaction.gas_limit > 0);
    }
}

pub(super) async fn prepare_evm_entry_point_request<S, A>(
    services: &RunServices<S, A>,
    op: &'static str,
    config: serde_json::Value,
) -> RunLaunchRequest
where
    S: store::RunEventStore + store::StoreScopeStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    let entry_point_registry =
        crate::entry_points::production_entry_point_op_registry().expect("entry points");
    let public_op_name = crate::PublicOpName::new(op).expect("public op name");
    let authored_config =
        AuthoredConfig::from_json_transport_value(Some(AuthoredConfigFormat::Json), &config)
            .expect("authored config");

    prepare_entry_point_run_launch(EntryPointRunLaunchInput {
        entry_point_registry: &entry_point_registry,
        public_op_name,
        op_version: None,
        authored_config,
        certification_registry: services.certification_registry(),
        store_scope_id: services.load_store_scope_id().await.expect("store scope"),
        invocation_key: None,
    })
    .expect("entry-point launch request")
    .request
}

pub(super) fn public_schema_id(request: &RunLaunchRequest) -> mfm_ids::SchemaId {
    request
        .certified_spec
        .envelope()
        .spec
        .public_outputs
        .public_schema_id
        .clone()
}

pub(super) async fn launch_completed(
    services: &ContractRunServices,
    request: RunLaunchRequest,
    label: &str,
) -> (mfm_ids::RunId, mfm_ids::SchemaId) {
    let run_id = request.run_id.clone();
    let public_schema_id = public_schema_id(&request);
    let outcome = services.launch_run(request).await.expect(label);
    let (_, launched, _) = outcome.into_response_parts();
    let launched = launched.expect("completed launch returns run");
    assert_eq!(launched.run_mode, RunModeStatus::Completed);
    (run_id, public_schema_id)
}

pub(super) async fn assert_replay_completed(
    services: &ContractRunServices,
    run_id: &mfm_ids::RunId,
    label: &str,
) {
    let replay = services.verify_replay_for_run(run_id).await.expect(label);
    assert_eq!(replay.run_mode, RunModeStatus::Completed);
}

pub(super) async fn assert_execution_claim_unclaimed(
    store: &ContractRunStore,
    run_id: &mfm_ids::RunId,
) {
    let scope = execution_scope_for_run(store, run_id).await;
    assert!(matches!(
        store
            .execution_claim_status(&scope)
            .await
            .expect("execution claim status"),
        ExecutionClaimStatus::Unclaimed
    ));
}

pub(super) async fn public_output_string(
    services: &ContractRunServices,
    run_id: &mfm_ids::RunId,
    public_schema_id: &mfm_ids::SchemaId,
) -> String {
    services
        .public_output(run_id, public_schema_id)
        .await
        .expect("public output")
        .json
        .expect("json")
        .to_string()
}

pub(super) async fn retained_validation_reports(
    services: &ContractRunServices,
    stream: &[store::KernelEventEnvelope],
) -> Vec<ContextBoundValidationReport> {
    let schema = ContextBoundValidationReport::schema_id().expect("validation report schema");
    let mut reports = Vec::new();
    for event in stream {
        let events::KernelEventPayload::CellProduced(payload) = event.payload() else {
            continue;
        };
        if payload.schema_id != schema {
            continue;
        }
        let requirement = event
            .payload()
            .artifact_requirements()
            .into_iter()
            .find(|requirement| requirement.artifact_id == payload.artifact_id)
            .expect("validation report artifact requirement");
        let artifact = services
            .artifacts()
            .read_retained_artifact(&requirement)
            .await
            .expect("validation report artifact");
        reports.push(
            serde_json::from_slice(artifact.bytes()).expect("validation report artifact json"),
        );
    }
    reports
}

pub(super) async fn launch_replay_and_render(
    services: &ContractRunServices,
    request: RunLaunchRequest,
    lifecycle: &str,
) -> (mfm_ids::RunId, String) {
    let launch_label = format!("launch {lifecycle} lifecycle");
    let (run_id, public_schema_id) = launch_completed(services, request, &launch_label).await;
    let replay_label = format!("replay {lifecycle} lifecycle");
    assert_replay_completed(services, &run_id, &replay_label).await;
    let rendered = public_output_string(services, &run_id, &public_schema_id).await;
    (run_id, rendered)
}

pub(super) fn assert_rendered_contains(rendered: &str, needle: &str) {
    assert!(
        rendered.contains(needle),
        "rendered output must contain {needle}: {rendered}"
    );
}

pub(super) fn contract_lifecycle_certification_registry() -> CertificationRegistry {
    let mut certification = CertificationRegistry::new();
    mfm_op_evm_contract_lifecycle::register_contract_lifecycle_certification_descriptors(
        &mut certification,
    )
    .expect("contract certification descriptors");
    certification
}

pub(super) fn contract_lifecycle_runners(
    factory: impl EvmContractRuntimeFactory + 'static,
) -> ErasedRunnerRegistry {
    let mut runners = ErasedRunnerRegistry::new();
    mfm_adapters_evm_contracts::register_contract_lifecycle_runners_with_factory(
        &mut runners,
        Arc::new(factory),
    )
    .expect("contract runners");
    runners
}

pub(super) fn contract_lifecycle_services(
    store: &ContractRunStore,
    artifacts: ContractArtifactOverlay,
    runners: ErasedRunnerRegistry,
    certification: CertificationRegistry,
) -> ContractRunServices {
    make_run_services(runners, store.clone(), artifacts, certification)
}

pub(super) fn contract_services(
    store: &ContractRunStore,
    artifacts: ContractArtifactOverlay,
    factory: impl EvmContractRuntimeFactory + 'static,
) -> ContractRunServices {
    contract_lifecycle_services(
        store,
        artifacts,
        contract_lifecycle_runners(factory),
        contract_lifecycle_certification_registry(),
    )
}

pub(super) fn contract_test_services(
    make_factory: impl FnOnce(Arc<dyn store::RetainedArtifactReadProvider>) -> TestRuntimeFactory,
) -> (ContractRunStore, ContractRunServices) {
    let store = test_run_store();
    let artifacts = ContractArtifactOverlay::new(Arc::new(store.clone()));
    let services = contract_services(&store, artifacts.clone(), make_factory(Arc::new(artifacts)));
    (store, services)
}
