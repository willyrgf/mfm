use super::*;
use crate::{
    make_run_services_with_certification_registry, prepare_entry_point_run_launch,
    EntryPointRunLaunchInput, ErrorClass, RunLaunchRequest, RunModeStatus, RunServices,
};
use mfm_adapters_evm_contracts::{
    ensure_prepared_invocation_public, EvmContractReadRuntime, EvmContractRuntime,
    EvmContractRuntimeFactory, PreparedContractInvocation,
};
use mfm_authored_config::{AuthoredConfig, AuthoredConfigFormat};
use mfm_certify::CertificationRegistry;
use mfm_core::crypto::EthereumPrivateKey;
use mfm_events::v1 as events;
use mfm_evm_capabilities::{
    EvmBlockReadProvider, EvmBlockReadRequest, EvmBlockReadResponse, EvmBlockSelector,
    EvmCallReadProvider, EvmCallReadRequest, EvmCallReadResponse, EvmCapabilityError,
    EvmCapabilityFuture, EvmChainIdentityProvider, EvmChainIdentityRequest,
    EvmChainIdentityResponse, EvmFeeReadProvider, EvmFeeReadRequest, EvmFeeReadResponse,
    EvmGasEstimateProvider, EvmGasEstimateRequest, EvmGasEstimateResponse, EvmLogEntry,
    EvmLogsReadProvider, EvmLogsReadRequest, EvmLogsReadResponse, EvmNonceOccupancy,
    EvmNonceOccupancyReadProvider, EvmNonceOccupancyReadRequest, EvmNonceOccupancyReadResponse,
    EvmNonceReadProvider, EvmNonceReadRequest, EvmNonceReadResponse, EvmReceiptReadProvider,
    EvmReceiptReadRequest, EvmReceiptReadResponse, EvmSourcePolicyId, EvmSourceRef,
    EvmTransactionSubmitProvider, EvmTransactionSubmitRequest, EvmTransactionSubmitResponse,
    RedactedEvmSourceEvidence,
};
use mfm_evm_contract_config::{ConfigurePhaseConfig, DeployPhaseConfig, ValidatePhaseConfig};
use mfm_evm_contract_model::{ConfiguredContract, DeployedContract};
use mfm_op_evm_contract_lifecycle::ContractLifecycleConfig;
use mfm_runtime::{CertifiedRuntimeSpec, ErasedRunnerRegistry};
use mfm_signing::{
    PublicSigningIdentity, SignatureBytes, SignerRef, SigningError, SigningFuture, SigningProvider,
    SigningRequest, SigningResult,
};
use mfm_store::v1::{
    self as store, ExecutionClaimStatus, ExecutionClaimStore, RetainedArtifactReadProvider,
    RunEventStore,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

const TEST_SIGNER_HEX: &str = "4c0883a69102937d6231471b5dbb6204fe512961708279c2f802d6a8ebf2d3a4";

type ContractRunStore = store::AsyncInMemoryRunStore;
type ContractRunServices = RunServices<ContractRunStore, ContractRunStore>;

fn evm_forbidden_runtime_terms() -> Vec<String> {
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

fn assert_no_evm_runtime_surface(label: &str, rendered: &str) {
    let rendered = rendered.to_ascii_lowercase();
    for forbidden in evm_forbidden_runtime_terms() {
        assert!(
            !rendered.contains(&forbidden),
            "{label} contains forbidden EVM runtime surface"
        );
    }
}

fn assert_prepared_invocation_has_unsigned_provenance(prepared: &PreparedContractInvocation) {
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

async fn prepare_evm_entry_point_request<S, A>(
    services: &RunServices<S, A>,
    op: &'static str,
    config: serde_json::Value,
) -> RunLaunchRequest
where
    S: store::RunEventStore + store::TrustScopeStore + Send + Sync,
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
        trust_scope_id: services.load_trust_scope_id().await.expect("trust scope"),
        distinct_run_key: None,
    })
    .expect("entry-point launch request")
    .request
}

async fn prepare_validate_entry_point_request<S, A>(
    services: &RunServices<S, A>,
    config: ValidatePhaseConfig,
    configured: ConfiguredContract,
) -> RunLaunchRequest
where
    S: store::RunEventStore + store::TrustScopeStore + Send + Sync,
    A: store::RetainedArtifactReadProvider + Clone + Send + Sync + 'static,
{
    prepare_evm_entry_point_request(
        services,
        "evm_contract_validate",
        json!({
            "config": config,
            "configured": configured,
        }),
    )
    .await
}

fn public_schema_id(request: &RunLaunchRequest) -> mfm_ids::SchemaId {
    request
        .certified_spec
        .envelope()
        .spec
        .public_outputs
        .public_schema_id
        .clone()
}

async fn launch_completed(
    services: &ContractRunServices,
    request: RunLaunchRequest,
    label: &str,
) -> (mfm_ids::RunId, mfm_ids::SchemaId) {
    let run_id = request.run_id.clone();
    let public_schema_id = public_schema_id(&request);
    let (_, launched) = services
        .launch_run(request)
        .await
        .expect(label)
        .into_response_parts();
    assert_eq!(launched.run_mode, RunModeStatus::Completed);
    (run_id, public_schema_id)
}

async fn assert_replay_completed(
    services: &ContractRunServices,
    run_id: &mfm_ids::RunId,
    label: &str,
) {
    let replay = services.verify_replay_for_run(run_id).await.expect(label);
    assert_eq!(replay.run_mode, RunModeStatus::Completed);
}

async fn assert_execution_claim_unclaimed(store: &ContractRunStore, run_id: &mfm_ids::RunId) {
    assert!(matches!(
        store
            .execution_claim_status(run_id)
            .await
            .expect("execution claim status"),
        ExecutionClaimStatus::Unclaimed
    ));
}

async fn public_output_string(
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

async fn launch_replay_and_render(
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

fn assert_rendered_contains(rendered: &str, needle: &str) {
    assert!(
        rendered.contains(needle),
        "rendered output must contain {needle}: {rendered}"
    );
}

fn contract_lifecycle_certification_registry() -> CertificationRegistry {
    let mut certification = CertificationRegistry::new();
    mfm_op_evm_contract_lifecycle::register_contract_lifecycle_certification_descriptors(
        &mut certification,
    )
    .expect("contract certification descriptors");
    certification
}

fn contract_lifecycle_runners(
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

fn contract_lifecycle_services(
    store: &ContractRunStore,
    runners: ErasedRunnerRegistry,
    certification: CertificationRegistry,
) -> ContractRunServices {
    make_run_services_with_certification_registry(
        runners,
        store.clone(),
        store.clone(),
        certification,
    )
}

fn contract_services(
    store: &ContractRunStore,
    factory: impl EvmContractRuntimeFactory + 'static,
) -> ContractRunServices {
    contract_lifecycle_services(
        store,
        contract_lifecycle_runners(factory),
        contract_lifecycle_certification_registry(),
    )
}

fn contract_test_services(
    make_factory: impl FnOnce(Arc<dyn ArtifactReadProvider>) -> TestRuntimeFactory,
) -> (ContractRunStore, ContractRunServices) {
    let store = test_run_store();
    let artifacts = crate::artifact_read_provider_from_retained(store.clone());
    let services = contract_services(&store, make_factory(artifacts));
    (store, services)
}

#[tokio::test]
async fn app_runner_resumes_replays_and_renders_validate_only_lifecycle_run() {
    let (store, services) = contract_test_services(TestRuntimeFactory::new);
    let request =
        prepare_validate_entry_point_request(&services, validate_config(), configured_contract())
            .await;
    let (run_id, rendered) = launch_replay_and_render(&services, request, "validate").await;
    assert_execution_claim_unclaimed(&store, &run_id).await;
    assert_rendered_contains(&rendered, "\"valid\":true");
}

#[tokio::test]
async fn app_resume_completed_run_is_evidence_only_without_live_runners() {
    let store = test_run_store();
    let artifacts = crate::artifact_read_provider_from_retained(store.clone());
    let certification = contract_lifecycle_certification_registry();
    let runners = contract_lifecycle_runners(TestRuntimeFactory::new(artifacts));
    let launch_services = contract_lifecycle_services(&store, runners, certification.clone());
    let request = prepare_validate_entry_point_request(
        &launch_services,
        validate_config(),
        configured_contract(),
    )
    .await;
    let run_id = request.run_id.clone();

    let (_, launched) = launch_services
        .launch_run(request)
        .await
        .expect("launch validate lifecycle")
        .into_response_parts();
    assert_eq!(launched.run_mode, RunModeStatus::Completed);
    assert_execution_claim_unclaimed(&store, &run_id).await;
    let completed_stream = store
        .load_run_stream(&run_id)
        .await
        .expect("completed stream");

    let resume_services =
        contract_lifecycle_services(&store, ErasedRunnerRegistry::new(), certification);
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
async fn app_resume_missing_runtime_config_fails_before_claim_or_attempt() {
    assert_resume_runtime_config_ingress_failure(None).await;
}

#[tokio::test]
async fn app_resume_malformed_runtime_config_fails_before_claim_or_attempt() {
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
    let request =
        prepare_validate_entry_point_request(&services, validate_config(), configured_contract())
            .await;
    let run_id = request.run_id.clone();
    let services_for_launch = services.clone();
    let launch = tokio::spawn(async move {
        services_for_launch
            .launch_run(request)
            .await
            .expect("launch response")
            .into_response_parts()
            .1
    });
    let lease = wait_for_live_execution_claim(&store, &run_id).await;
    assert!(
        store
            .release_execution_claim(&run_id, &lease.token)
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
    let artifacts = crate::artifact_read_provider_from_retained(store.clone());
    let certification = contract_lifecycle_certification_registry();
    let launch_runners = contract_lifecycle_runners(TestRuntimeFactory::new(artifacts.clone()));
    let launch_services =
        contract_lifecycle_services(&store, launch_runners, certification.clone());
    let request = prepare_validate_entry_point_request(
        &launch_services,
        validate_config(),
        configured_contract(),
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

    let resume_runners = crate::production_runner_registry(artifacts, runtime_config_path)
        .expect("production runners");
    let resume_services = contract_lifecycle_services(&store, resume_runners, certification);
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
    let config = deploy_config(&signer.address);
    let (_store, services) = contract_test_services(|artifacts| {
        TestRuntimeFactory::with_signer(artifacts, Arc::new(signer.provider), true)
    });
    let request = prepare_evm_entry_point_request(
        &services,
        "evm_contract_deploy",
        serde_json::to_value(config).expect("deploy config json"),
    )
    .await;
    let (_, rendered) = launch_replay_and_render(&services, request, "deploy").await;
    assert_rendered_contains(&rendered, "contract_address");
    assert_rendered_contains(&rendered, "deploy_receipt_evidence");
}

#[tokio::test]
async fn replay_diagnostic_resolves_evm_guard_from_side_effect_submit_node() {
    let signer = test_contract_signer();
    let config = deploy_config(&signer.address);
    let (_store, services) = contract_test_services(|artifacts| {
        TestRuntimeFactory::with_signer(artifacts, Arc::new(signer.provider), true)
    });
    let request = prepare_evm_entry_point_request(
        &services,
        "evm_contract_deploy",
        serde_json::to_value(config).expect("deploy config json"),
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
        "verifier framework config must not be used as EVM guard authority"
    );

    let guards = crate::certified_evm_chain_guards_for_failed_node(
        &runtime_spec,
        &request.evidence,
        &verify_node.node_id,
    )
    .expect("verifier diagnostics derive submit-node guards");
    assert!(
        guards.iter().any(|guard| {
            guard.network_id().as_str() == "reth-dev" && guard.expected_chain_id() == 31337
        }),
        "derived guards must include the submit node's certified EVM guard"
    );
    let details = json!({
        "network_id": "reth-dev",
        "expected_chain_id": 31337,
        "observed_chain_id": 31338,
        "source_ref": "reth-dev",
        "policy_id": "reth-dev",
    });
    let expected =
        events::RedactedJson::new(crate::canonical_value_digest(&details).expect("details digest"));
    crate::verify_replay_public_details(Some(&expected), &details, &guards)
        .expect("verifier diagnostic details match submit-node guard");
}

#[tokio::test]
async fn app_runner_resumes_replays_and_renders_full_lifecycle_run() {
    let signer = test_contract_signer();
    let config = ContractLifecycleConfig::new(
        deploy_config(&signer.address),
        configure_config(&signer.address),
        validate_config(),
    );
    let (_store, services) = contract_test_services(|artifacts| {
        TestRuntimeFactory::with_signer(artifacts, Arc::new(signer.provider), false)
    });
    let request = prepare_evm_entry_point_request(
        &services,
        "evm_contract_lifecycle",
        serde_json::to_value(config).expect("lifecycle config json"),
    )
    .await;
    let (run_id, rendered) = launch_replay_and_render(&services, request, "full").await;
    assert_rendered_contains(&rendered, "\"valid\":true");
    assert_rendered_contains(&rendered, "configure_tx_hashes");
    assert_rendered_contains(&rendered, "configure_receipt_evidence");
    assert_no_evm_runtime_surface("contract public output", &rendered);

    let stream = services
        .store()
        .load_run_stream(&run_id)
        .await
        .expect("load run stream");
    assert_eq!(
        contract_lifecycle_runner_output_summary(&stream),
        [
            "attempt-output:mfm.evm.contract/deploy:side_effect.intent_persisted+side_effect.claimed+resource_lane.claimed+retention_refs_appended[roles=side_effect_intent]",
            "attempt-output:mfm.evm.contract/deploy:side_effect.invocation_prepared+side_effect.invocation_started+retention_refs_appended[roles=prepared_invocation]",
            "attempt-output:mfm.evm.contract/deploy:side_effect.submission_observed+retention_refs_appended[roles=submission]",
            "attempt-output:mfm.evm.contract/deploy:cell_skipped+state_attempt_completed",
            "attempt-output:mfm.evm.contract/configure:side_effect.intent_persisted+side_effect.claimed+resource_lane.claimed+retention_refs_appended[roles=side_effect_intent]",
            "attempt-output:mfm.evm.contract/configure:side_effect.invocation_prepared+side_effect.invocation_started+retention_refs_appended[roles=prepared_invocation]",
            "attempt-output:mfm.evm.contract/configure:side_effect.submission_observed+retention_refs_appended[roles=submission]",
            "attempt-output:mfm.evm.contract/configure:cell_skipped+state_attempt_completed",
            "attempt-output:mfm.evm.contract/validate:cell_produced+state_attempt_completed+artifact_referenced[role=state_output]+retention_refs_appended[roles=state_output]",
        ]
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

#[derive(Clone)]
struct TestRuntimeFactory {
    artifacts: Arc<dyn ArtifactReadProvider>,
    read_runtime: EvmContractReadRuntime,
    runtime: EvmContractRuntime,
}

impl TestRuntimeFactory {
    fn new(artifacts: Arc<dyn ArtifactReadProvider>) -> Self {
        Self::with_options(artifacts, false, None)
    }

    fn with_chain_identity_delay(
        artifacts: Arc<dyn ArtifactReadProvider>,
        delay: Duration,
    ) -> Self {
        Self::with_options(artifacts, false, Some(delay))
    }

    fn with_options(
        artifacts: Arc<dyn ArtifactReadProvider>,
        mutation: bool,
        chain_identity_delay: Option<Duration>,
    ) -> Self {
        Self::with_runtime(
            artifacts,
            Arc::new(TestSigner),
            mutation,
            false,
            chain_identity_delay,
        )
    }

    fn with_signer(
        artifacts: Arc<dyn ArtifactReadProvider>,
        signer: Arc<dyn SigningProvider>,
        fail_repeated_prepare_reads: bool,
    ) -> Self {
        Self::with_runtime(artifacts, signer, true, fail_repeated_prepare_reads, None)
    }

    fn with_runtime(
        artifacts: Arc<dyn ArtifactReadProvider>,
        signer: Arc<dyn SigningProvider>,
        mutation: bool,
        fail_repeated_prepare_reads: bool,
        chain_identity_delay: Option<Duration>,
    ) -> Self {
        let evm = Arc::new(TestEvmProvider {
            source_ref: EvmSourceRef::new("reth-dev").expect("source ref"),
            policy_id: EvmSourcePolicyId::new("reth-dev").expect("policy id"),
            mutation,
            fail_repeated_prepare_reads,
            chain_identity_delay,
            reads: Arc::new(Mutex::new(TestEvmReads::default())),
        });
        Self {
            artifacts,
            read_runtime: EvmContractReadRuntime::new(evm.clone()),
            runtime: EvmContractRuntime::new(evm, signer),
        }
    }
}

impl EvmContractRuntimeFactory for TestRuntimeFactory {
    fn artifacts(&self) -> &dyn ArtifactReadProvider {
        self.artifacts.as_ref()
    }

    fn validate_runtime_for(
        &self,
        _network_id: &str,
        _signer_ref: Option<&SignerRef>,
    ) -> mfm_runtime::Result<()> {
        Ok(())
    }

    fn read_runtime_for(&self, _network_id: &str) -> mfm_runtime::Result<EvmContractReadRuntime> {
        Ok(self.read_runtime.clone())
    }

    fn runtime_for(&self, _network_id: &str) -> mfm_runtime::Result<EvmContractRuntime> {
        Ok(self.runtime.clone())
    }
}

struct TestSigner;

impl SigningProvider for TestSigner {
    fn sign<'a>(&'a self, _request: &'a SigningRequest) -> SigningFuture<'a> {
        Box::pin(async { Err(SigningError::redacted_provider_failure("test signer")) })
    }
}

#[derive(Clone)]
struct TestEvmProvider {
    source_ref: EvmSourceRef,
    policy_id: EvmSourcePolicyId,
    mutation: bool,
    fail_repeated_prepare_reads: bool,
    chain_identity_delay: Option<Duration>,
    reads: Arc<Mutex<TestEvmReads>>,
}

#[derive(Default)]
struct TestEvmReads {
    nonce: u32,
    fee: u32,
    gas: u32,
    receipt: u32,
}

impl TestEvmProvider {
    fn evidence(&self, guard: &EvmChainGuard) -> RedactedEvmSourceEvidence {
        RedactedEvmSourceEvidence {
            network_id: guard.network_id().clone(),
            expected_chain_id: guard.expected_chain_id(),
            observed_chain_id: guard.expected_chain_id(),
            source_ref: self.source_ref.clone(),
            policy_id: self.policy_id.clone(),
        }
    }

    fn record_prepare_read(&self, kind: TestPrepareReadKind) -> Result<(), EvmCapabilityError> {
        if !self.fail_repeated_prepare_reads {
            return Ok(());
        }
        let mut reads = self
            .reads
            .lock()
            .map_err(|_| EvmCapabilityError::redacted_provider_failure("test evm"))?;
        let count = match kind {
            TestPrepareReadKind::Nonce => &mut reads.nonce,
            TestPrepareReadKind::Fee => &mut reads.fee,
            TestPrepareReadKind::Gas => &mut reads.gas,
        };
        *count += 1;
        if *count > 1 {
            Err(EvmCapabilityError::redacted_provider_failure("test evm"))
        } else {
            Ok(())
        }
    }
}

enum TestPrepareReadKind {
    Nonce,
    Fee,
    Gas,
}

impl EvmChainIdentityProvider for TestEvmProvider {
    fn chain_identity<'a>(
        &'a self,
        request: &'a EvmChainIdentityRequest,
    ) -> EvmCapabilityFuture<'a, EvmChainIdentityResponse> {
        Box::pin(async move {
            if let Some(delay) = self.chain_identity_delay {
                tokio::time::sleep(delay).await;
            }
            Ok(EvmChainIdentityResponse {
                evidence: self.evidence(&request.guard),
                chain_id: request.guard.expected_chain_id(),
                client_version: Some("mfm-test-evm".to_owned()),
            })
        })
    }
}

impl EvmBlockReadProvider for TestEvmProvider {
    fn read_block<'a>(
        &'a self,
        request: &'a EvmBlockReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBlockReadResponse> {
        if !self.mutation {
            return failed_evm();
        }
        assert_eq!(request.block, EvmBlockSelector::Latest);
        Box::pin(async move {
            Ok(EvmBlockReadResponse {
                evidence: self.evidence(&request.guard),
                block_number: 64,
                block_hash: Default::default(),
            })
        })
    }
}

impl EvmNonceReadProvider for TestEvmProvider {
    fn read_nonce<'a>(
        &'a self,
        request: &'a EvmNonceReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmNonceReadResponse> {
        if !self.mutation {
            return failed_evm();
        }
        Box::pin(async move {
            self.record_prepare_read(TestPrepareReadKind::Nonce)?;
            Ok(EvmNonceReadResponse {
                evidence: self.evidence(&request.guard),
                nonce: 7,
            })
        })
    }
}

impl EvmFeeReadProvider for TestEvmProvider {
    fn read_fee<'a>(
        &'a self,
        request: &'a EvmFeeReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmFeeReadResponse> {
        if !self.mutation {
            return failed_evm();
        }
        Box::pin(async move {
            self.record_prepare_read(TestPrepareReadKind::Fee)?;
            Ok(EvmFeeReadResponse {
                evidence: self.evidence(&request.guard),
                base_fee_per_gas: Some(5),
                priority_fee_per_gas: Some(3),
                max_fee_per_gas: Some(11),
                legacy_gas_price: Some(7),
            })
        })
    }
}

impl EvmGasEstimateProvider for TestEvmProvider {
    fn estimate_gas<'a>(
        &'a self,
        request: &'a EvmGasEstimateRequest,
    ) -> EvmCapabilityFuture<'a, EvmGasEstimateResponse> {
        if !self.mutation {
            return failed_evm();
        }
        Box::pin(async move {
            self.record_prepare_read(TestPrepareReadKind::Gas)?;
            Ok(EvmGasEstimateResponse {
                evidence: self.evidence(&request.guard),
                gas_limit: 21_000,
            })
        })
    }
}

impl EvmTransactionSubmitProvider for TestEvmProvider {
    fn submit_transaction<'a>(
        &'a self,
        request: &'a EvmTransactionSubmitRequest,
    ) -> EvmCapabilityFuture<'a, EvmTransactionSubmitResponse> {
        if !self.mutation {
            return failed_evm();
        }
        Box::pin(async move {
            Ok(EvmTransactionSubmitResponse {
                evidence: self.evidence(&request.guard),
                transaction_hash: request.signed_payload.transaction_hash(),
            })
        })
    }
}

impl EvmReceiptReadProvider for TestEvmProvider {
    fn read_receipt<'a>(
        &'a self,
        request: &'a EvmReceiptReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmReceiptReadResponse> {
        let reads = Arc::clone(&self.reads);
        if !self.mutation {
            return Box::pin(async move {
                let mut reads = reads
                    .lock()
                    .map_err(|_| EvmCapabilityError::redacted_provider_failure("test evm"))?;
                reads.receipt += 1;
                Err(EvmCapabilityError::redacted_provider_failure("test evm"))
            });
        }
        Box::pin(async move {
            let mut reads = self
                .reads
                .lock()
                .map_err(|_| EvmCapabilityError::redacted_provider_failure("test evm"))?;
            reads.receipt += 1;
            Ok(EvmReceiptReadResponse {
                evidence: self.evidence(&request.guard),
                transaction_hash: request.transaction_hash,
                block_number: 42,
                status: true,
            })
        })
    }
}

impl EvmNonceOccupancyReadProvider for TestEvmProvider {
    fn read_nonce_occupancy<'a>(
        &'a self,
        request: &'a EvmNonceOccupancyReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmNonceOccupancyReadResponse> {
        Box::pin(async move {
            Ok(EvmNonceOccupancyReadResponse {
                evidence: self.evidence(&request.guard),
                outcome: EvmNonceOccupancy::Unknown,
            })
        })
    }
}

impl EvmCallReadProvider for TestEvmProvider {
    fn read_call<'a>(
        &'a self,
        request: &'a EvmCallReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmCallReadResponse> {
        Box::pin(async move {
            let mut return_data = vec![0_u8; 32];
            return_data[31] = 1;
            Ok(EvmCallReadResponse {
                evidence: self.evidence(&request.guard),
                return_data,
            })
        })
    }
}

impl EvmLogsReadProvider for TestEvmProvider {
    fn read_logs<'a>(
        &'a self,
        request: &'a EvmLogsReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmLogsReadResponse> {
        Box::pin(async move {
            Ok(EvmLogsReadResponse {
                evidence: self.evidence(&request.guard),
                logs: vec![EvmLogEntry {
                    address: request.address.expect("validation log address"),
                    topics: request.topics.clone(),
                    data: Vec::new(),
                    block_number: Some(42),
                    transaction_hash: None,
                    log_index: Some(0),
                }],
            })
        })
    }
}

fn failed_evm<'a, T>() -> EvmCapabilityFuture<'a, T> {
    Box::pin(async { Err(EvmCapabilityError::redacted_provider_failure("test evm")) })
}

async fn wait_for_live_execution_claim<S>(
    store: &S,
    run_id: &mfm_ids::RunId,
) -> store::AdmissionLease
where
    S: ExecutionClaimStore,
    S::Error: std::fmt::Debug,
{
    for _ in 0..100 {
        match store
            .execution_claim_status(run_id)
            .await
            .expect("execution claim status")
        {
            ExecutionClaimStatus::Live(lease) => return lease,
            ExecutionClaimStatus::Unclaimed | ExecutionClaimStatus::Expired(_) => {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
    }
    panic!("execution claim was not acquired");
}

struct TestSignerRuntime {
    provider: LocalTestSigner,
    address: String,
}

fn test_contract_signer() -> TestSignerRuntime {
    let signer_ref = SignerRef::new("deployer").expect("signer ref");
    let material = EthereumPrivateKey::from_hex_secret(TEST_SIGNER_HEX).expect("signer");
    let address = format!("{:?}", material.address().expect("address"));
    let provider = LocalTestSigner {
        signer_ref,
        address: address.clone(),
        material,
    };
    TestSignerRuntime { provider, address }
}

struct LocalTestSigner {
    signer_ref: SignerRef,
    address: String,
    material: EthereumPrivateKey,
}

impl SigningProvider for LocalTestSigner {
    fn sign<'a>(&'a self, request: &'a SigningRequest) -> SigningFuture<'a> {
        let result = self.sign_request(request);
        Box::pin(async move { result })
    }
}

impl LocalTestSigner {
    fn sign_request(&self, request: &SigningRequest) -> mfm_signing::Result<SigningResult> {
        if request.signer_ref() != &self.signer_ref {
            return Err(SigningError::redacted_provider_failure("test signer"));
        }
        let digest = request.digest().as_bytes();
        let signature = self
            .material
            .sign_hash_recoverable(digest)
            .map_err(|_| SigningError::redacted_provider_failure("test signer"))?;
        let identity = PublicSigningIdentity::new(
            request.algorithm().clone(),
            None,
            Some(self.address.clone()),
        )?;
        let signature = SignatureBytes::new(signature.as_bytes().to_vec())?;
        SigningResult::for_request(request, identity, signature)
    }
}

fn reth_network_json() -> serde_json::Value {
    json!({
        "network_id": "reth-dev",
        "expected_chain_id": 31337
    })
}

fn deployer_signer_json(expected_signer_address: &str) -> serde_json::Value {
    json!({
        "signer_ref": "deployer",
        "expected_signer_address": expected_signer_address
    })
}

fn validate_config() -> ValidatePhaseConfig {
    serde_json::from_value(json!({
        "network": reth_network_json()
    }))
    .expect("validate config")
}

fn deploy_config(expected_signer_address: &str) -> DeployPhaseConfig {
    serde_json::from_value(json!({
        "artifact": artifact_json(),
        "network": reth_network_json(),
        "signer": deployer_signer_json(expected_signer_address),
        "transaction": {
            "style": "eip1559",
            "max_fee_per_gas": "11",
            "max_priority_fee_per_gas": "3"
        }
    }))
    .expect("deploy config")
}

fn configure_config(expected_signer_address: &str) -> ConfigurePhaseConfig {
    serde_json::from_value(json!({
        "artifact": artifact_json(),
        "network": reth_network_json(),
        "signer": deployer_signer_json(expected_signer_address),
        "calls": [
            {
                "function": "configure",
                "args": []
            },
            {
                "function": "configure",
                "args": []
            }
        ]
    }))
    .expect("configure config")
}

fn artifact_json() -> serde_json::Value {
    json!({
        "abi": {
            "json_text": json!([
                {
                    "type": "constructor",
                    "inputs": []
                },
                {
                    "type": "function",
                    "name": "configure",
                    "inputs": [],
                    "outputs": [],
                    "stateMutability": "nonpayable"
                },
                {
                    "type": "function",
                    "name": "ready",
                    "inputs": [],
                    "outputs": [
                        {
                            "name": "",
                            "type": "bool"
                        }
                    ],
                    "stateMutability": "view"
                },
                {
                    "type": "event",
                    "name": "Configured",
                    "inputs": [],
                    "anonymous": false
                }
            ]).to_string()
        },
        "bytecode": {
            "json_text": json!({"object": "0x6000"}).to_string()
        }
    })
}

fn configured_contract() -> ConfiguredContract {
    ConfiguredContract {
        lifecycle_version: 1,
        deployed: DeployedContract {
            lifecycle_version: 1,
            network_id: "reth-dev".to_owned(),
            expected_chain_id: 31337,
            contract_address: "0x000000000000000000000000000000000000dead".to_owned(),
            deploy_tx_hash: "0x01".to_owned(),
            deploy_receipt_evidence: None,
            deployed_block_number: Some(1),
        },
        configure_calls: Vec::new(),
        confirmation_read_assertions: Vec::new(),
        confirmation_event_assertions: Vec::new(),
        configure_tx_hashes: Vec::new(),
        configure_receipt_evidence: Vec::new(),
        configured_block_number: Some(1),
    }
}

fn contract_lifecycle_runner_output_summary(stream: &[store::KernelEventEnvelope]) -> Vec<String> {
    attempt_output_commit_summaries(stream, "mfm.evm.contract/")
}

fn attempt_output_commit_summaries(
    stream: &[store::KernelEventEnvelope],
    state_kind_prefix: &str,
) -> Vec<String> {
    let state_kinds_by_node = state_kinds_by_node(stream);
    let mut summaries = Vec::new();
    let mut index = 0;
    while index < stream.len() {
        let first = &stream[index];
        let seq = first.seq();
        let commit_key = first.commit_key();
        let mut end = index + 1;
        while end < stream.len()
            && stream[end].seq() == seq
            && stream[end].commit_key() == commit_key
        {
            end += 1;
        }
        if commit_key.as_str().starts_with("attempt-output:") {
            let state_kind = runner_output_node_id(&stream[index..end])
                .and_then(|node_id| state_kinds_by_node.get(&node_id))
                .map(String::as_str)
                .unwrap_or("unknown");
            if state_kind.starts_with(state_kind_prefix) {
                let payloads = stream[index..end]
                    .iter()
                    .map(|event| runner_payload_summary(event.payload()))
                    .collect::<Vec<_>>()
                    .join("+");
                summaries.push(format!(
                    "{}:{state_kind}:{payloads}",
                    commit_key_class(commit_key.as_str())
                ));
            }
        }
        index = end;
    }
    summaries
}

fn state_kinds_by_node(stream: &[store::KernelEventEnvelope]) -> BTreeMap<String, String> {
    stream
        .iter()
        .filter_map(|event| match event.payload() {
            events::KernelEventPayload::StateAttemptStarted(payload) => Some((
                payload.node_id.as_str().to_owned(),
                payload
                    .state_kind
                    .canonical_name()
                    .unwrap_or_else(|| payload.state_kind.as_str())
                    .to_owned(),
            )),
            _ => None,
        })
        .collect()
}

macro_rules! node_id_from_payload {
    ($payload:expr, $($variant:ident),+ $(,)?) => {
        match $payload {
            $(
                events::KernelEventPayload::$variant(payload) => {
                    Some(payload.node_id.as_str().to_owned())
                }
            )+
            _ => None,
        }
    };
}

fn runner_output_node_id(events: &[store::KernelEventEnvelope]) -> Option<String> {
    events.iter().find_map(|event| {
        node_id_from_payload!(
            event.payload(),
            FactRecorded,
            CellProduced,
            CellSkipped,
            SideEffectIntentPersisted,
            SideEffectClaimed,
            SideEffectClaimTakenOver,
            SideEffectInvocationPrepared,
            SideEffectInvocationStarted,
            SideEffectNotSubmittedProven,
            SideEffectSubmissionObserved,
            SideEffectSubmissionUnknown,
            SideEffectReceiptObserved,
            SideEffectConfirmationObserved,
            SideEffectAmbiguous,
            SideEffectFailed,
            StateAttemptCompleted,
        )
    })
}

fn runner_payload_summary(payload: &events::KernelEventPayload) -> String {
    match payload {
        events::KernelEventPayload::ArtifactReferenced(payload) => {
            format!(
                "artifact_referenced[role={}]",
                payload.artifact_ref.role.as_str()
            )
        }
        events::KernelEventPayload::RetentionRefsAppended(payload) => {
            let roles = payload
                .refs
                .iter()
                .map(|retention| retention.role.as_str())
                .collect::<Vec<_>>()
                .join(",");
            format!("retention_refs_appended[roles={roles}]")
        }
        _ => payload_schema_name(payload).to_owned(),
    }
}

fn payload_schema_name(payload: &events::KernelEventPayload) -> &str {
    payload
        .schema_descriptor()
        .schema_name
        .strip_prefix("mfm.events.v1.")
        .unwrap_or_else(|| payload.schema_descriptor().schema_name)
}

fn commit_key_class(commit_key: &str) -> &str {
    if commit_key.starts_with("attempt-output:") {
        "attempt-output"
    } else {
        commit_key
    }
}
