use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use alloy_primitives::{keccak256, B256};
use k256::{ecdsa::SigningKey, elliptic_curve::rand_core::OsRng};
use mfm_adapters_evm_contracts::{
    register_contract_state_runners_with_factory, verify_contract_state_replay,
    EvmContractProvider, EvmContractReadProvider, EvmContractReadRuntime, EvmContractRuntime,
    EvmContractRuntimeFactory,
};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_events::v1 as events;
use mfm_evm_capabilities::{
    EvmBlockReadProvider, EvmBlockReadRequest, EvmBlockReadResponse, EvmBlockSelector,
    EvmCallReadProvider, EvmCallReadRequest, EvmCallReadResponse, EvmCapabilityError,
    EvmCapabilityFuture, EvmChainIdentityProvider, EvmChainIdentityRequest,
    EvmChainIdentityResponse, EvmCodeReadProvider, EvmCodeReadRequest, EvmCodeReadResponse,
    EvmFeeReadProvider, EvmFeeReadRequest, EvmFeeReadResponse, EvmGasEstimateProvider,
    EvmGasEstimateRequest, EvmGasEstimateResponse, EvmLogEntry, EvmLogsReadProvider,
    EvmLogsReadRequest, EvmLogsReadResponse, EvmNetworkBinding,
    EvmNetworkId as CapabilityEvmNetworkId, EvmNonceOccupancy, EvmNonceOccupancyReadProvider,
    EvmNonceOccupancyReadRequest, EvmNonceOccupancyReadResponse, EvmNonceReadProvider,
    EvmNonceReadRequest, EvmNonceReadResponse, EvmReceiptReadProvider, EvmReceiptReadRequest,
    EvmReceiptReadResponse, EvmSourcePolicyId, EvmSourceRef, EvmTransactionSubmitProvider,
    EvmTransactionSubmitRequest, EvmTransactionSubmitResponse, RedactedEvmSourceEvidence,
};
use mfm_evm_contract_model::{
    ConfiguredContractInstance, ContractArtifactConfig, ContractProfile, ContractProfileId,
    EvmCodeHash, EvmContractContext, EvmNetworkContext, EvmNetworkId, LifecycleArtifactEvidenceRef,
    LifecycleKey, ValidationCodeIdentityEvidence,
};
use mfm_ids::ArtifactId;
use mfm_program::{
    build_root_with_registries, PublicOutputKey, ScopeKey, SideEffectSagaPolicy,
    SideEffectVerificationSpec, StateKey, StateSpec,
};
use mfm_program_derive::PublicOutputs;
use mfm_replay::v1::{ReplayBroker, ReplayReadAuthority};
use mfm_runtime::{CertifiedRuntimeSpec, VerifiedRunHistoryView};
use mfm_signing::{
    PublicSigningIdentity, SignatureBytes, SigningError, SigningProvider, SigningRequest,
    SigningResult,
};
use mfm_spec::v1 as spec;
use mfm_state_evm_contracts::{
    account_nonce_resource_claim, ConfigureAction, ContextBoundConfigureContractState,
    ContextBoundDeployContractState, ContextBoundValidateContractState,
    ContextConfigureContractInputHandles, ContextValidateContractInputHandles, DeployAction,
    ValidateAction,
};
use mfm_store::v1::{self as store, RetainedArtifactReadProvider, RunEventStore as _};
use mfm_values::{MfmConfig, MfmValue};
use serde_json::json;
use tokio::sync::oneshot;

const RUNTIME_CODE: &[u8] = &[0x60, 0x00];
const DEPLOY_BLOCK_NUMBER: u64 = 42;
const LAST_CONFIGURE_BLOCK_NUMBER: u64 = DEPLOY_BLOCK_NUMBER + 2;
const LATEST_BLOCK_NUMBER: u64 = LAST_CONFIGURE_BLOCK_NUMBER + 1;
const CONFIGURED_GRAPH_CONFIGURE_CALLS: usize = 2;

fn direct_block_hash(block_number: u64) -> B256 {
    let offset = u8::try_from(
        block_number
            .checked_sub(DEPLOY_BLOCK_NUMBER)
            .expect("direct test block precedes deployment"),
    )
    .expect("direct test block range");
    B256::repeat_byte(
        0x42_u8
            .checked_add(offset)
            .expect("direct test block hash range"),
    )
}

fn direct_signer_address(signing_key: &SigningKey) -> String {
    let encoded = signing_key.verifying_key().to_encoded_point(false);
    let hash = keccak256(&encoded.as_bytes()[1..]);
    format!("0x{}", hex::encode(&hash.as_slice()[12..]))
}

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.evm.contract.test.direct_state_graph_outputs")]
struct DirectContractStateOutputs<'program, 'scope> {
    validation:
        mfm_program::Handle<'program, 'scope, mfm_evm_contract_model::ContextBoundValidationReport>,
}

/// The contract states remain executable through a direct typed graph. This deliberately bypasses
/// app entry-point discovery: the app is used only as generic launch/scheduler test support.
#[tokio::test]
async fn direct_contract_configured_graph_reports_true_and_false_with_configured_anchor_replay() {
    assert_contract_entry_points_are_absent();

    for (label, configured_assertion_value) in [("true", true), ("false", false)] {
        let (stream, provider) = run_direct_contract_state_graph(
            Some(expected_runtime_code_hash()),
            &format!("direct-contract-state-graph-configured-{label}"),
            false,
            CONFIGURED_GRAPH_CONFIGURE_CALLS,
            configured_assertion_value,
        )
        .await;
        assert_eq!(
            provider.transaction_submission_hashes().len(),
            3,
            "{label}: the graph must submit deployment plus both configure calls"
        );
        assert_eq!(provider.code_read_requests().len(), 1, "{label}");
        assert_eq!(
            provider.code_read_requests()[0].block(),
            &EvmBlockSelector::Hash(direct_block_hash(LAST_CONFIGURE_BLOCK_NUMBER)),
            "{label}: validation must use the last configure receipt anchor"
        );
        assert_eq!(provider.call_read_requests().len(), 1, "{label}");
        assert_eq!(
            provider.log_read_requests(),
            1,
            "{label}: event assertion must execute"
        );
        assert!(
            provider
                .block_read_requests()
                .iter()
                .any(|request| request.block()
                    == &EvmBlockSelector::Number(LAST_CONFIGURE_BLOCK_NUMBER)),
            "{label}: the last configure receipt anchor must be proven canonical"
        );
        assert_eq!(
            stream
                .iter()
                .filter(|event| matches!(
                    event.payload(),
                    events::KernelEventPayload::ArtifactReferenced(reference)
                        if reference.artifact_ref.role == events::ArtifactRole::ExternalReadEvidence
                            && reference.artifact_ref.schema_id
                                == ValidationCodeIdentityEvidence::schema_id().expect("code evidence schema")
                ))
                .count(),
            1,
            "{label}: hash-bound runtime-code observation must be retained once"
        );
    }
}

#[tokio::test]
async fn direct_contract_validation_skips_code_reads_without_a_profile_hash() {
    let (stream, provider) = run_direct_contract_state_graph(
        None,
        "direct-contract-state-graph-without-code",
        false,
        CONFIGURED_GRAPH_CONFIGURE_CALLS,
        true,
    )
    .await;
    assert!(provider.code_read_requests().is_empty());
    assert!(stream.iter().all(|event| !matches!(
        event.payload(),
        events::KernelEventPayload::ArtifactReferenced(reference)
            if reference.artifact_ref.role == events::ArtifactRole::ExternalReadEvidence
                && reference.artifact_ref.schema_id
                    == ValidationCodeIdentityEvidence::schema_id().expect("code evidence schema")
    )));
}

#[tokio::test]
async fn direct_contract_replay_rejects_tampered_runtime_code_evidence() {
    let _ = run_direct_contract_state_graph(
        Some(expected_runtime_code_hash()),
        "direct-contract-state-graph-tampered-code",
        true,
        CONFIGURED_GRAPH_CONFIGURE_CALLS,
        true,
    )
    .await;
}

#[tokio::test]
async fn direct_contract_uses_deploy_anchor_without_configure_calls() {
    let (_, provider) = run_direct_contract_state_graph(
        Some(expected_runtime_code_hash()),
        "direct-contract-state-graph-deploy-anchor",
        false,
        0,
        true,
    )
    .await;
    assert_eq!(
        provider.transaction_submission_hashes().len(),
        1,
        "the deploy-only graph must not submit a configure transaction"
    );
    assert_eq!(
        provider.code_read_requests()[0].block(),
        &EvmBlockSelector::Hash(direct_block_hash(DEPLOY_BLOCK_NUMBER)),
        "validation must fall back to the deploy receipt anchor"
    );
    assert_eq!(
        provider.call_read_requests().len(),
        1,
        "the deploy-anchor validation still executes the configured read assertion"
    );
    assert_eq!(provider.log_read_requests(), 0);
}

#[tokio::test]
async fn direct_contract_replay_verifies_empty_configure_confirmation_before_output() {
    let harness = direct_contract_state_graph_harness(
        Some(expected_runtime_code_hash()),
        "direct-contract-state-graph-empty-configure-prefix",
        None,
        0,
        true,
    )
    .await;
    let report = harness
        .services
        .launch_run_and_render(harness.request.clone())
        .await
        .expect("run empty-configure direct state graph");
    assert_eq!(
        report.run.expect("run response").run_mode,
        mfm_app::RunModeStatus::Completed
    );

    let committed = harness
        .store
        .load_committed_run_stream(&harness.request.run_id)
        .await
        .expect("completed committed run stream");
    let configure_node_id = configure_node_id(committed.events());
    let configure_pair_id = committed
        .events()
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::SideEffectInvocationStarted(payload)
                if payload.node_id == configure_node_id =>
            {
                Some(payload.pair_id.clone())
            }
            _ => None,
        })
        .expect("empty configure side effect must start");
    let confirmation_seq = committed
        .events()
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::SideEffectConfirmationObserved(payload)
                if payload.pair_id == configure_pair_id =>
            {
                Some(event.seq())
            }
            _ => None,
        })
        .expect("empty configure confirmation must be durable");
    let prefix = store::CommittedRunStream::from_events(
        harness.request.run_id.clone(),
        committed
            .events()
            .iter()
            .take_while(|event| event.seq() <= confirmation_seq)
            .cloned()
            .collect(),
    )
    .expect("post-confirmation committed prefix");
    let configured_schema =
        ConfiguredContractInstance::schema_id().expect("configured contract schema");
    assert!(prefix.events().iter().all(|event| !matches!(
        event.payload(),
        events::KernelEventPayload::CellProduced(payload)
            if payload.schema_id == configured_schema
    )));

    verify_direct_contract_state_replay_from_committed(&harness, prefix).await;
}

/// The direct graph resumes both sides of the external side-effect boundary without an app
/// entry point: a broadcast whose response was lost and finality confirmation after a durable
/// receipt. The retained terminal evidence must still be sufficient for offline replay.
#[tokio::test]
async fn direct_contract_states_resume_submission_and_confirmation_boundaries() {
    assert_contract_entry_points_are_absent();

    for (boundary, invocation_key) in [
        (
            DirectContractInterruptionBoundary::AmbiguousConfigureSubmission,
            "direct-contract-state-graph-resume-configure-submission",
        ),
        (
            DirectContractInterruptionBoundary::ConfigureConfirmation,
            "direct-contract-state-graph-resume-configure-confirmation",
        ),
    ] {
        let (interruption, entered) = DirectContractInterruption::new(boundary);
        let harness = direct_contract_state_graph_harness(
            Some(expected_runtime_code_hash()),
            invocation_key,
            Some(interruption),
            CONFIGURED_GRAPH_CONFIGURE_CALLS,
            true,
        )
        .await;
        let run_id = harness.request.run_id.clone();
        let launch_services = harness.services.clone();
        let launch_request = harness.request.clone();
        let launch = tokio::spawn(async move { launch_services.launch_run(launch_request).await });

        tokio::time::timeout(Duration::from_secs(10), entered)
            .await
            .expect("direct contract graph timed out before the interruption boundary")
            .expect("direct contract graph reached the requested interruption boundary");
        let interrupted = harness
            .store
            .load_run_stream(&run_id)
            .await
            .expect("interrupted direct contract run stream");
        assert_interrupted_at_contract_boundary(&interrupted, boundary);

        launch.abort();
        let _ = launch.await;
        assert!(
            harness
                .store
                .expire_execution_claim_for_test(&run_id)
                .expect("expire interrupted direct-contract execution claim"),
            "the interrupted direct contract graph must hold its execution claim"
        );

        let resumed = harness
            .services
            .resume_stored_run(&run_id)
            .await
            .expect("resume interrupted direct contract graph");
        assert_eq!(resumed.run_mode, mfm_app::RunModeStatus::Completed);

        let stream = harness
            .store
            .load_run_stream(&run_id)
            .await
            .expect("resumed direct contract run stream");
        assert_contract_resume_preserves_single_submission_and_nonce_lane(
            &stream,
            harness.provider.as_ref(),
        );
        assert_contract_finality_anchor_reads(harness.provider.as_ref());
        verify_direct_contract_state_replay(&harness).await;
    }
}

fn assert_contract_entry_points_are_absent() {
    let entry_points = mfm_app::entry_point_ids();
    assert_eq!(
        entry_points,
        &["mfm.portfolio/snapshot@1"],
        "the public portfolio objective must not register contract state graphs"
    );
    for id in [
        "mfm.evm.contract/deploy@1",
        "mfm.evm.contract/configure@1",
        "mfm.evm.contract/validate@1",
        "mfm.evm.contract/lifecycle@1",
    ] {
        assert!(
            !entry_points.contains(&id),
            "a reusable contract state must not be public: {id}"
        );
    }
}

async fn run_direct_contract_state_graph(
    deployed_code_hash: Option<EvmCodeHash>,
    invocation_key: &str,
    tamper_code_evidence: bool,
    configure_call_count: usize,
    configured_assertion_value: bool,
) -> (
    Vec<store::KernelEventEnvelope>,
    Arc<DirectContractStateProvider>,
) {
    let harness = direct_contract_state_graph_harness(
        deployed_code_hash,
        invocation_key,
        None,
        configure_call_count,
        configured_assertion_value,
    )
    .await;
    let run_id = harness.request.run_id.clone();
    let report = harness
        .services
        .launch_run_and_render(harness.request.clone())
        .await
        .expect("run direct state graph");
    let run = report.run.expect("run response");
    let valid = report
        .public_output
        .and_then(|output| output.json)
        .and_then(|output| {
            output
                .pointer("/validation/valid")
                .and_then(serde_json::Value::as_bool)
        })
        .expect("terminal validation output");
    let stream = harness
        .store
        .load_run_stream(&run_id)
        .await
        .expect("run stream");
    assert_eq!(
        run.run_mode,
        mfm_app::RunModeStatus::Completed,
        "direct contract-state graph failed: {run:?}"
    );
    assert_eq!(
        valid, configured_assertion_value,
        "the terminal report must preserve configured assertion validity"
    );
    assert!(
        stream
            .iter()
            .any(|event| matches!(event.payload(), events::KernelEventPayload::RunCompleted(_))),
        "the direct graph must reach a terminal completed run"
    );
    if tamper_code_evidence {
        let committed = harness
            .store
            .load_committed_run_stream(&run_id)
            .await
            .expect("committed run stream");
        let tampered = TamperedRuntimeCodeEvidenceProvider::new(harness.artifacts.clone());
        assert!(
            store::VerifiedRunArtifactStore::from_committed_stream(&committed, &tampered)
                .await
                .is_err(),
            "replay artifact admission must reject tampered retained runtime bytecode"
        );
    } else {
        verify_direct_contract_state_replay(&harness).await;
    }
    (stream, harness.provider)
}

struct DirectContractStateGraphHarness {
    store: store::AsyncInMemoryRunStore,
    artifacts: ContractArtifactOverlay,
    services: mfm_app::RunServices<store::AsyncInMemoryRunStore, ContractArtifactOverlay>,
    provider: Arc<DirectContractStateProvider>,
    profile_replay_artifact: store::VerifiedRunArtifactBytes,
    runtime_spec: CertifiedRuntimeSpec,
    request: mfm_app::RunLaunchRequest,
}

async fn direct_contract_state_graph_harness(
    deployed_code_hash: Option<EvmCodeHash>,
    invocation_key: &str,
    interruption: Option<Arc<DirectContractInterruption>>,
    configure_call_count: usize,
    configured_assertion_value: bool,
) -> DirectContractStateGraphHarness {
    let (context, artifact) = direct_contract_context_and_artifact(deployed_code_hash);
    let profile_replay_artifact = replay_profile_artifact(&artifact);
    let store = store::AsyncInMemoryRunStore::new();
    let artifacts = ContractArtifactOverlay::new(store.clone(), artifact);
    let factory = Arc::new(DirectContractStateRuntimeFactory::new(
        artifacts.clone(),
        interruption,
        configured_assertion_value,
    ));
    let provider = Arc::clone(&factory.provider);
    let mut runners = mfm_runtime::ErasedRunnerRegistry::new();
    register_contract_state_runners_with_factory(&mut runners, factory)
        .expect("register reusable contract-state runners");

    let mut certification = mfm_certify::CertificationRegistry::new();
    certification
        .register_state::<ContextBoundDeployContractState>()
        .expect("register deploy descriptor");
    certification
        .register_state::<ContextBoundConfigureContractState>()
        .expect("register configure descriptor");
    certification
        .register_state::<ContextBoundValidateContractState>()
        .expect("register validate descriptor");

    let services =
        mfm_app::make_run_services(runners, store.clone(), artifacts.clone(), certification);
    let draft =
        direct_contract_state_graph(context, provider.expected_signer(), configure_call_count)
            .expect("direct state graph");
    let request = mfm_app::prepare_typed_program_run_launch_for_test(
        draft,
        Default::default(),
        services.certification_registry(),
        services.load_store_scope_id().await.expect("store scope"),
        Some(mfm_app::InvocationKey::new(invocation_key).expect("invocation key")),
    )
    .expect("prepare direct state graph launch");
    let runtime_spec =
        CertifiedRuntimeSpec::new(request.certified_spec.clone()).expect("certified runtime spec");
    DirectContractStateGraphHarness {
        store,
        artifacts,
        services,
        provider,
        profile_replay_artifact,
        runtime_spec,
        request,
    }
}

async fn verify_direct_contract_state_replay(harness: &DirectContractStateGraphHarness) {
    let committed = harness
        .store
        .load_committed_run_stream(&harness.request.run_id)
        .await
        .expect("committed run stream");
    verify_direct_contract_state_replay_from_committed(harness, committed).await;
}

async fn verify_direct_contract_state_replay_from_committed(
    harness: &DirectContractStateGraphHarness,
    committed: store::CommittedRunStream,
) {
    let retained =
        store::VerifiedRunArtifactStore::from_committed_stream(&committed, &harness.artifacts)
            .await
            .expect("verified retained artifacts");
    let view =
        VerifiedRunHistoryView::from_committed_stream(&harness.runtime_spec, committed, retained)
            .expect("verified run history");
    let broker = ReplayBroker::from_read_authority(
        ReplayReadAuthority::from_verified_run_history_view_with_source_facts_and_artifacts(
            &harness.runtime_spec,
            &view,
            Vec::new(),
            vec![harness.profile_replay_artifact.clone()],
        )
        .expect("replay authority"),
    )
    .expect("replay broker");
    verify_contract_state_replay(&broker).expect("evidence-only contract-state replay");
}

fn assert_interrupted_at_contract_boundary(
    stream: &[store::KernelEventEnvelope],
    boundary: DirectContractInterruptionBoundary,
) {
    let configure_node_id = configure_node_id(stream);
    let configure_pair_id = stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::SideEffectInvocationStarted(payload)
                if payload.node_id == configure_node_id =>
            {
                Some(payload.pair_id.clone())
            }
            _ => None,
        })
        .expect("configure side effect must start");
    match boundary {
        DirectContractInterruptionBoundary::AmbiguousConfigureSubmission => assert!(
            stream.iter().any(|event| matches!(
                event.payload(),
                events::KernelEventPayload::SideEffectInvocationStarted(payload)
                    if payload.node_id == configure_node_id
            )) && !stream.iter().any(|event| matches!(
                event.payload(),
                events::KernelEventPayload::SideEffectSubmissionObserved(payload)
                    if payload.node_id == configure_node_id
            )),
            "the configure submission interruption must occur after invocation preparation but before a durable submission result"
        ),
        DirectContractInterruptionBoundary::ConfigureConfirmation => assert!(
            stream.iter().any(|event| matches!(
                event.payload(),
                events::KernelEventPayload::SideEffectReceiptObserved(payload)
                    if payload.pair_id == configure_pair_id
            )) && !stream.iter().any(|event| matches!(
                event.payload(),
                events::KernelEventPayload::SideEffectConfirmationObserved(payload)
                    if payload.pair_id == configure_pair_id
            )),
            "the configure confirmation interruption must occur after durable receipt evidence but before confirmation"
        ),
    }
}

fn configure_node_id(stream: &[store::KernelEventEnvelope]) -> mfm_ids::NodeId {
    let configure_kind = ContextBoundConfigureContractState::kind().expect("configure kind");
    stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::StateAttemptStarted(payload)
                if payload.state_kind == configure_kind =>
            {
                Some(payload.node_id.clone())
            }
            _ => None,
        })
        .expect("direct graph must start the configure state")
}

fn assert_contract_resume_preserves_single_submission_and_nonce_lane(
    stream: &[store::KernelEventEnvelope],
    provider: &DirectContractStateProvider,
) {
    let configure_node_id = configure_node_id(stream);
    let submissions = stream
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::SideEffectSubmissionObserved(payload)
                    if payload.node_id == configure_node_id
            )
        })
        .count();
    assert_eq!(
        submissions, 1,
        "resume must retain exactly one durable submission for the configure side effect"
    );
    let transaction_hashes = provider.transaction_submission_hashes();
    assert_eq!(
        transaction_hashes.len(),
        3,
        "resume must not issue duplicate live deployment or configure transactions"
    );
    assert_ne!(
        transaction_hashes[1], transaction_hashes[2],
        "resume must not duplicate a nonce-lane transaction"
    );
    let claims = stream
        .iter()
        .filter(|event| {
            matches!(
                event.payload(),
                events::KernelEventPayload::ResourceLaneClaimed(payload)
                    if payload.node_id == configure_node_id
            )
        })
        .count();
    assert_eq!(
        claims, 1,
        "resume must not reacquire configure's exclusive signer/nonce lane"
    );
}

fn assert_contract_finality_anchor_reads(provider: &DirectContractStateProvider) {
    let reads = provider.block_read_requests();
    assert!(
        reads.iter().any(
            |request| request.block() == &EvmBlockSelector::Number(LAST_CONFIGURE_BLOCK_NUMBER)
        ),
        "resume must preserve canonical reads for the last configure receipt anchor"
    );
    assert!(
        reads
            .iter()
            .any(|request| request.block() == &EvmBlockSelector::Latest),
        "resume must preserve finality-depth reads after canonical anchor verification"
    );
}

fn direct_contract_state_graph(
    context: EvmContractContext,
    expected_signer: &str,
    configure_call_count: usize,
) -> mfm_program::Result<mfm_program::TypedProgramDraft> {
    let mut states = mfm_program::StateRegistryBuilder::new();
    states.register::<ContextBoundDeployContractState>()?;
    states.register::<ContextBoundConfigureContractState>()?;
    states.register::<ContextBoundValidateContractState>()?;

    build_root_with_registries(
        ScopeKey::new("evm_contract_direct_state_graph")?,
        states.into_snapshot(),
        mfm_program::OperationRegistryBuilder::new().into_snapshot(),
        |root| {
            root.set_saga_policy(SideEffectSagaPolicy::FailWithoutAcdcClaim)?;
            let context = root.scope().declare_context(context)?;
            let deployed = root
                .scope()
                .side_effect::<ContextBoundDeployContractState, _>(
                    StateKey::new("deploy")?,
                    &context,
                    deploy_action(expected_signer),
                    (),
                    account_nonce_resource_claim()?,
                    SideEffectVerificationSpec::Finalized { depth: 1 },
                )?
                .into_handle();
            let configured = root
                .scope()
                .side_effect::<ContextBoundConfigureContractState, _>(
                    StateKey::new("configure")?,
                    &context,
                    configure_action(expected_signer, configure_call_count),
                    ContextConfigureContractInputHandles { deployed },
                    account_nonce_resource_claim()?,
                    SideEffectVerificationSpec::Finalized { depth: 1 },
                )?
                .into_handle();
            let validation = root.scope().state::<ContextBoundValidateContractState, _>(
                StateKey::new("validate")?,
                &context,
                validate_action(configure_call_count),
                ContextValidateContractInputHandles { configured },
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("validation")?,
                &DirectContractStateOutputs { validation },
            )
        },
    )
}

fn deploy_action(expected_signer: &str) -> DeployAction {
    serde_json::from_value(json!({
        "signer": {
            "signer_ref": "deployer",
            "expected_signer_address": expected_signer,
        },
        "receipt": {"poll_interval_ms": 1, "max_receipt_polls": 1},
    }))
    .expect("deploy action")
}

fn configure_action(expected_signer: &str, configure_call_count: usize) -> ConfigureAction {
    serde_json::from_value(json!({
        "signer": {
            "signer_ref": "deployer",
            "expected_signer_address": expected_signer,
        },
        "calls": (0..configure_call_count)
            .map(|_| json!({"function": "configure", "args": []}))
            .collect::<Vec<_>>(),
        "receipt": {"poll_interval_ms": 1, "max_receipt_polls": 1},
    }))
    .expect("configure action")
}

fn validate_action(configure_call_count: usize) -> ValidateAction {
    let mut action = json!({
        "read_assertions": [{
            "function": "configured",
            "args": [],
            "expected": {"json_text": "true"},
        }],
    });
    if configure_call_count > 0 {
        action["event_assertions"] = json!([{
            "event": "Configured",
            "min_count": 1,
            "from_block": {"kind": "number", "number": DEPLOY_BLOCK_NUMBER},
            "to_block": {
                "kind": "number",
                "number": DEPLOY_BLOCK_NUMBER + configure_call_count as u64,
            },
        }]);
    }
    serde_json::from_value(action).expect("validate action")
}

fn direct_contract_context_and_artifact(
    deployed_code_hash: Option<EvmCodeHash>,
) -> (EvmContractContext, ContractArtifact) {
    let profile: ContractArtifactConfig = serde_json::from_value(json!({
        "abi": {
            "json_text": serde_json::to_string(&json!([
                {"type": "constructor", "inputs": []},
                {
                    "type": "function",
                    "name": "configure",
                    "inputs": [],
                    "outputs": [],
                    "stateMutability": "nonpayable",
                },
                {
                    "type": "function",
                    "name": "configured",
                    "inputs": [],
                    "outputs": [{"name": "", "type": "bool"}],
                    "stateMutability": "view",
                },
                {
                    "type": "event",
                    "name": "Configured",
                    "inputs": [],
                    "anonymous": false,
                },
            ]))
            .expect("ABI JSON")
        },
        "bytecode": {"json_text": serde_json::to_string(&json!({"object": "0x6000"}))
            .expect("bytecode JSON")}
    }))
    .expect("contract artifact config");
    let canonical = PlainCanonicalJsonBytes::from_json_str(
        &serde_json::to_string(&profile).expect("artifact JSON"),
    )
    .expect("canonical artifact JSON");
    let digest = canonical.content_digest();
    let evidence = store::ArtifactEvidenceRef {
        artifact_id: ArtifactId::from_digest(digest.algorithm(), *digest.digest()),
        digest: digest.clone(),
        byte_len: canonical.as_bytes().len() as u64,
        media_type: spec::MediaType::new("application/json").expect("media type"),
        schema_id: Some(
            <ContractArtifactConfig as MfmConfig>::schema_id().expect("artifact schema"),
        ),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: events::ArtifactRole::TypedConfig,
    };
    let artifact_ref = LifecycleArtifactEvidenceRef::new(
        evidence.artifact_id.clone(),
        digest.clone(),
        evidence.evidence_hash().expect("artifact evidence hash"),
        evidence.byte_len,
        evidence.schema_id.clone(),
        None,
    );
    let context = EvmContractContext {
        lifecycle_key: LifecycleKey::new("direct-contract-state-graph").expect("lifecycle key"),
        network: EvmNetworkContext::new(
            EvmNetworkId::new("ethereum-mainnet").expect("network id"),
            1,
        )
        .expect("network context"),
        contract_profile: ContractProfile {
            profile_id: ContractProfileId::new("direct-contract-profile").expect("profile id"),
            artifact_digest: Some(digest.into()),
            artifact_ref: Some(artifact_ref),
            interface_digest: None,
            creation_bytecode_digest: None,
            deployed_code_hash,
            selector_event_policy_digest: None,
        },
    };
    (
        context,
        ContractArtifact {
            bytes: canonical.to_vec(),
            evidence,
        },
    )
}

fn expected_runtime_code_hash() -> EvmCodeHash {
    EvmCodeHash::new(format!("{:?}", keccak256(RUNTIME_CODE))).expect("runtime code hash")
}

#[derive(Clone)]
struct ContractArtifact {
    bytes: Vec<u8>,
    evidence: store::ArtifactEvidenceRef,
}

fn replay_profile_artifact(artifact: &ContractArtifact) -> store::VerifiedRunArtifactBytes {
    let requirement = store::EventArtifactRequirement {
        source: store::EventArtifactReferenceSource::ArtifactReferenced,
        artifact_id: artifact.evidence.artifact_id.clone(),
        evidence_hash: artifact
            .evidence
            .evidence_hash()
            .expect("artifact evidence hash"),
        digest: Some(artifact.evidence.digest.clone()),
        byte_len: Some(artifact.evidence.byte_len),
        media_type: Some(artifact.evidence.media_type.clone()),
        schema_id: artifact.evidence.schema_id.clone(),
        semantic_type_id: None,
        producer_node_id: None,
        producer_seed_id: None,
        artifact_role: Some(events::ArtifactRole::TypedConfig),
    };
    store::VerifiedRunArtifactBytes::new(
        artifact.bytes.clone(),
        artifact.evidence.clone(),
        &requirement,
    )
    .expect("verified profile artifact")
}

#[derive(Clone)]
struct ContractArtifactOverlay {
    store: store::AsyncInMemoryRunStore,
    artifact: Arc<ContractArtifact>,
}

impl ContractArtifactOverlay {
    fn new(store: store::AsyncInMemoryRunStore, artifact: ContractArtifact) -> Self {
        Self {
            store,
            artifact: Arc::new(artifact),
        }
    }
}

impl RetainedArtifactReadProvider for ContractArtifactOverlay {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        if requirement.artifact_id == self.artifact.evidence.artifact_id
            && requirement.evidence_hash
                == self
                    .artifact
                    .evidence
                    .evidence_hash()
                    .expect("artifact evidence hash")
        {
            let artifact = Arc::clone(&self.artifact);
            return Box::pin(async move {
                store::VerifiedRunArtifactBytes::new(
                    artifact.bytes.clone(),
                    artifact.evidence.clone(),
                    requirement,
                )
            });
        }
        self.store.read_retained_artifact(requirement)
    }
}

#[derive(Clone)]
struct TamperedRuntimeCodeEvidenceProvider {
    inner: ContractArtifactOverlay,
}

impl TamperedRuntimeCodeEvidenceProvider {
    fn new(inner: ContractArtifactOverlay) -> Self {
        Self { inner }
    }
}

impl RetainedArtifactReadProvider for TamperedRuntimeCodeEvidenceProvider {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        Box::pin(async move {
            let artifact = self.inner.read_retained_artifact(requirement).await?;
            let code_schema =
                ValidationCodeIdentityEvidence::schema_id().expect("code identity evidence schema");
            if artifact.evidence().artifact_role != events::ArtifactRole::ExternalReadEvidence
                || artifact.evidence().schema_id.as_ref() != Some(&code_schema)
            {
                return Ok(artifact);
            }

            let mut tampered_bytes = artifact.bytes().to_vec();
            tampered_bytes.push(b'\n');
            store::VerifiedRunArtifactBytes::new(
                tampered_bytes,
                artifact.evidence().clone(),
                requirement,
            )
        })
    }
}

struct DirectContractStateRuntimeFactory {
    artifacts: ContractArtifactOverlay,
    provider: Arc<DirectContractStateProvider>,
}

impl DirectContractStateRuntimeFactory {
    fn new(
        artifacts: ContractArtifactOverlay,
        interruption: Option<Arc<DirectContractInterruption>>,
        configured_assertion_value: bool,
    ) -> Self {
        Self {
            artifacts,
            provider: Arc::new(DirectContractStateProvider::new(
                interruption,
                configured_assertion_value,
            )),
        }
    }
}

impl EvmContractRuntimeFactory for DirectContractStateRuntimeFactory {
    fn artifacts(&self) -> &dyn RetainedArtifactReadProvider {
        &self.artifacts
    }

    fn validate_runtime_for(
        &self,
        binding: &EvmNetworkBinding,
        _signer_ref: Option<&mfm_signing::SignerRef>,
    ) -> mfm_runtime::Result<()> {
        if binding.network_id().as_str() == "ethereum-mainnet" && binding.expected_chain_id() == 1 {
            Ok(())
        } else {
            Err(mfm_runtime::RuntimeError::RunnerBinding(
                "unexpected direct contract-state test network".to_owned(),
            ))
        }
    }

    fn read_runtime_for(
        &self,
        _binding: EvmNetworkBinding,
    ) -> mfm_runtime::Result<EvmContractReadRuntime> {
        let provider: Arc<dyn EvmContractReadProvider> = self.provider.clone();
        Ok(EvmContractReadRuntime::new(provider))
    }

    fn runtime_for(&self, _binding: EvmNetworkBinding) -> mfm_runtime::Result<EvmContractRuntime> {
        let evm: Arc<dyn EvmContractProvider> = self.provider.clone();
        let signer: Arc<dyn SigningProvider> = self.provider.clone();
        Ok(EvmContractRuntime::new(evm, signer))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectContractInterruptionBoundary {
    AmbiguousConfigureSubmission,
    ConfigureConfirmation,
}

struct DirectContractInterruption {
    boundary: DirectContractInterruptionBoundary,
    consumed: AtomicBool,
    entered: Mutex<Option<oneshot::Sender<()>>>,
}

impl DirectContractInterruption {
    fn new(boundary: DirectContractInterruptionBoundary) -> (Arc<Self>, oneshot::Receiver<()>) {
        let (entered_tx, entered_rx) = oneshot::channel();
        (
            Arc::new(Self {
                boundary,
                consumed: AtomicBool::new(false),
                entered: Mutex::new(Some(entered_tx)),
            }),
            entered_rx,
        )
    }

    async fn pause_if_selected(&self, boundary: DirectContractInterruptionBoundary) {
        if self.boundary != boundary || self.consumed.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(entered) = self.entered.lock().expect("interruption signal").take() {
            let _ = entered.send(());
        }
        std::future::pending::<()>().await;
    }
}

struct DirectContractStateProvider {
    evidence: RedactedEvmSourceEvidence,
    signing_key: SigningKey,
    expected_signer: String,
    block_read_requests: Mutex<Vec<EvmBlockReadRequest>>,
    code_read_requests: Mutex<Vec<EvmCodeReadRequest>>,
    call_read_requests: Mutex<Vec<EvmCallReadRequest>>,
    log_read_count: AtomicUsize,
    transaction_submission_hashes: Mutex<Vec<B256>>,
    interruption: Option<Arc<DirectContractInterruption>>,
    configured_assertion_value: bool,
}

impl DirectContractStateProvider {
    fn new(
        interruption: Option<Arc<DirectContractInterruption>>,
        configured_assertion_value: bool,
    ) -> Self {
        let signing_key = SigningKey::random(&mut OsRng);
        let expected_signer = direct_signer_address(&signing_key);
        Self {
            evidence: RedactedEvmSourceEvidence {
                network_id: CapabilityEvmNetworkId::new("ethereum-mainnet").expect("network id"),
                expected_chain_id: 1,
                observed_chain_id: 1,
                source_ref: EvmSourceRef::new("direct-test").expect("source ref"),
                policy_id: EvmSourcePolicyId::new("direct-test").expect("policy id"),
            },
            signing_key,
            expected_signer,
            block_read_requests: Mutex::new(Vec::new()),
            code_read_requests: Mutex::new(Vec::new()),
            call_read_requests: Mutex::new(Vec::new()),
            log_read_count: AtomicUsize::new(0),
            transaction_submission_hashes: Mutex::new(Vec::new()),
            interruption,
            configured_assertion_value,
        }
    }

    fn expected_signer(&self) -> &str {
        &self.expected_signer
    }

    fn block_read_requests(&self) -> Vec<EvmBlockReadRequest> {
        self.block_read_requests
            .lock()
            .expect("block read requests")
            .clone()
    }

    fn code_read_requests(&self) -> Vec<EvmCodeReadRequest> {
        self.code_read_requests
            .lock()
            .expect("code read requests")
            .clone()
    }

    fn call_read_requests(&self) -> Vec<EvmCallReadRequest> {
        self.call_read_requests
            .lock()
            .expect("call read requests")
            .clone()
    }

    fn log_read_requests(&self) -> usize {
        self.log_read_count.load(Ordering::Relaxed)
    }

    fn transaction_submission_hashes(&self) -> Vec<B256> {
        self.transaction_submission_hashes
            .lock()
            .expect("transaction submission hashes")
            .clone()
    }
}

impl EvmChainIdentityProvider for DirectContractStateProvider {
    fn chain_identity<'a>(
        &'a self,
        _request: &'a EvmChainIdentityRequest,
    ) -> EvmCapabilityFuture<'a, EvmChainIdentityResponse> {
        let evidence = self.evidence.clone();
        Box::pin(async move {
            Ok(EvmChainIdentityResponse {
                evidence,
                chain_id: 1,
                client_version: Some("direct-contract-state-test".to_owned()),
            })
        })
    }
}

impl EvmBlockReadProvider for DirectContractStateProvider {
    fn read_block<'a>(
        &'a self,
        request: &'a EvmBlockReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBlockReadResponse> {
        self.block_read_requests
            .lock()
            .expect("block read requests")
            .push(request.clone());
        let evidence = self.evidence.clone();
        let configure_confirmation_read =
            request.block() == &EvmBlockSelector::Number(LAST_CONFIGURE_BLOCK_NUMBER);
        let (block_number, block_hash) = match request.block() {
            EvmBlockSelector::Number(number) => (*number, direct_block_hash(*number)),
            EvmBlockSelector::Latest | EvmBlockSelector::Pending => {
                (LATEST_BLOCK_NUMBER, direct_block_hash(LATEST_BLOCK_NUMBER))
            }
            EvmBlockSelector::Hash(_) => {
                (LATEST_BLOCK_NUMBER, direct_block_hash(LATEST_BLOCK_NUMBER))
            }
        };
        let interruption = self.interruption.clone();
        Box::pin(async move {
            if configure_confirmation_read {
                if let Some(interruption) = interruption {
                    interruption
                        .pause_if_selected(
                            DirectContractInterruptionBoundary::ConfigureConfirmation,
                        )
                        .await;
                }
            }
            Ok(EvmBlockReadResponse {
                evidence,
                block_number,
                block_hash,
            })
        })
    }
}

impl EvmNonceReadProvider for DirectContractStateProvider {
    fn read_nonce<'a>(
        &'a self,
        _request: &'a EvmNonceReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmNonceReadResponse> {
        let evidence = self.evidence.clone();
        Box::pin(async move { Ok(EvmNonceReadResponse { evidence, nonce: 7 }) })
    }
}

impl EvmFeeReadProvider for DirectContractStateProvider {
    fn read_fee<'a>(
        &'a self,
        _request: &'a EvmFeeReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmFeeReadResponse> {
        let evidence = self.evidence.clone();
        Box::pin(async move {
            Ok(EvmFeeReadResponse {
                evidence,
                base_fee_per_gas: Some(5),
                priority_fee_per_gas: Some(3),
                max_fee_per_gas: Some(11),
                legacy_gas_price: Some(7),
            })
        })
    }
}

impl EvmGasEstimateProvider for DirectContractStateProvider {
    fn estimate_gas<'a>(
        &'a self,
        _request: &'a EvmGasEstimateRequest,
    ) -> EvmCapabilityFuture<'a, EvmGasEstimateResponse> {
        let evidence = self.evidence.clone();
        Box::pin(async move {
            Ok(EvmGasEstimateResponse {
                evidence,
                gas_limit: 21_000,
            })
        })
    }
}

impl EvmTransactionSubmitProvider for DirectContractStateProvider {
    fn submit_transaction<'a>(
        &'a self,
        request: &'a EvmTransactionSubmitRequest,
    ) -> EvmCapabilityFuture<'a, EvmTransactionSubmitResponse> {
        let evidence = self.evidence.clone();
        let transaction_hash = request.signed_payload().transaction_hash();
        let submission_index = {
            let mut submissions = self
                .transaction_submission_hashes
                .lock()
                .expect("transaction submission hashes");
            let index = submissions.len();
            submissions.push(transaction_hash);
            index
        };
        let interruption = self.interruption.clone();
        Box::pin(async move {
            // Pause after the second configure submission reaches the provider but before the
            // batch response is persisted. Both configure receipts can then reconcile on resume.
            if submission_index == 2 {
                if let Some(interruption) = interruption {
                    interruption
                        .pause_if_selected(
                            DirectContractInterruptionBoundary::AmbiguousConfigureSubmission,
                        )
                        .await;
                }
            }
            Ok(EvmTransactionSubmitResponse {
                evidence,
                transaction_hash,
            })
        })
    }
}

impl EvmReceiptReadProvider for DirectContractStateProvider {
    fn read_receipt<'a>(
        &'a self,
        request: &'a EvmReceiptReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmReceiptReadResponse> {
        let evidence = self.evidence.clone();
        let transaction_hash = request.transaction_hash();
        let receipt = self
            .transaction_submission_hashes
            .lock()
            .expect("transaction submission hashes")
            .iter()
            .position(|submitted| submitted == &transaction_hash)
            .map(|index| {
                let block_number = DEPLOY_BLOCK_NUMBER
                    + u64::try_from(index).expect("direct test transaction index");
                (block_number, direct_block_hash(block_number))
            });
        Box::pin(async move {
            if let Some((block_number, block_hash)) = receipt {
                Ok(EvmReceiptReadResponse {
                    evidence,
                    transaction_hash,
                    block_number,
                    block_hash,
                    status: true,
                })
            } else {
                Err(EvmCapabilityError::ReceiptPending)
            }
        })
    }
}

impl EvmNonceOccupancyReadProvider for DirectContractStateProvider {
    fn read_nonce_occupancy<'a>(
        &'a self,
        _request: &'a EvmNonceOccupancyReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmNonceOccupancyReadResponse> {
        let evidence = self.evidence.clone();
        Box::pin(async move {
            Ok(EvmNonceOccupancyReadResponse {
                evidence,
                outcome: EvmNonceOccupancy::Unknown,
            })
        })
    }
}

impl EvmCodeReadProvider for DirectContractStateProvider {
    fn read_code<'a>(
        &'a self,
        request: &'a EvmCodeReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmCodeReadResponse> {
        self.code_read_requests
            .lock()
            .expect("code read requests")
            .push(request.clone());
        let evidence = self.evidence.clone();
        Box::pin(async move {
            let code = RUNTIME_CODE.to_vec();
            Ok(EvmCodeReadResponse {
                evidence,
                code_hash: keccak256(&code),
                code,
            })
        })
    }
}

impl EvmCallReadProvider for DirectContractStateProvider {
    fn read_call<'a>(
        &'a self,
        request: &'a EvmCallReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmCallReadResponse> {
        self.call_read_requests
            .lock()
            .expect("call read requests")
            .push(request.clone());
        let evidence = self.evidence.clone();
        let configured_assertion_value = self.configured_assertion_value;
        Box::pin(async move {
            let mut return_data = vec![0_u8; 32];
            return_data[31] = u8::from(configured_assertion_value);
            Ok(EvmCallReadResponse {
                evidence,
                return_data,
            })
        })
    }
}

impl EvmLogsReadProvider for DirectContractStateProvider {
    fn read_logs<'a>(
        &'a self,
        request: &'a EvmLogsReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmLogsReadResponse> {
        self.log_read_count.fetch_add(1, Ordering::Relaxed);
        let evidence = self.evidence.clone();
        let address = request
            .address()
            .expect("direct event assertion must bind the deployed address");
        let topics = request.topics().to_vec();
        Box::pin(async move {
            Ok(EvmLogsReadResponse {
                evidence,
                logs: vec![EvmLogEntry {
                    address,
                    topics,
                    data: Vec::new(),
                    block_number: Some(LAST_CONFIGURE_BLOCK_NUMBER),
                    transaction_hash: None,
                    log_index: Some(0),
                }],
            })
        })
    }
}

impl SigningProvider for DirectContractStateProvider {
    fn sign<'a>(&'a self, request: &'a SigningRequest) -> mfm_signing::SigningFuture<'a> {
        let result = (|| {
            let (signature, recovery_id) = self
                .signing_key
                .sign_prehash_recoverable(request.digest().as_bytes())
                .map_err(|_| SigningError::redacted_provider_failure("direct state signer"))?;
            let mut signature_bytes = signature.to_bytes().to_vec();
            signature_bytes.push(u8::from(recovery_id.is_y_odd()));
            let signature = SignatureBytes::new(signature_bytes)?;
            let identity = PublicSigningIdentity::new(
                request.algorithm().clone(),
                None,
                Some(self.expected_signer.clone()),
            )?;
            SigningResult::for_request(request, identity, signature)
        })();
        Box::pin(async move { result })
    }
}
