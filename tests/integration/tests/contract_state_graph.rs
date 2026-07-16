use std::sync::Arc;

use alloy_primitives::{keccak256, B256};
use mfm_adapters_evm_contracts::{
    register_contract_state_runners_with_factory, EvmContractProvider, EvmContractReadProvider,
    EvmContractReadRuntime, EvmContractRuntime, EvmContractRuntimeFactory,
};
use mfm_canonical::PlainCanonicalJsonBytes;
use mfm_events::v1 as events;
use mfm_evm_capabilities::{
    EvmBlockReadProvider, EvmBlockReadRequest, EvmBlockReadResponse, EvmCallReadProvider,
    EvmCallReadRequest, EvmCallReadResponse, EvmCapabilityFuture, EvmChainIdentityProvider,
    EvmChainIdentityRequest, EvmChainIdentityResponse, EvmCodeReadProvider, EvmCodeReadRequest,
    EvmCodeReadResponse, EvmFeeReadProvider, EvmFeeReadRequest, EvmFeeReadResponse,
    EvmGasEstimateProvider, EvmGasEstimateRequest, EvmGasEstimateResponse, EvmLogsReadProvider,
    EvmLogsReadRequest, EvmLogsReadResponse, EvmNetworkBinding,
    EvmNetworkId as CapabilityEvmNetworkId, EvmNonceOccupancy, EvmNonceOccupancyReadProvider,
    EvmNonceOccupancyReadRequest, EvmNonceOccupancyReadResponse, EvmNonceReadProvider,
    EvmNonceReadRequest, EvmNonceReadResponse, EvmReceiptReadProvider, EvmReceiptReadRequest,
    EvmReceiptReadResponse, EvmSourcePolicyId, EvmSourceRef, EvmTransactionSubmitProvider,
    EvmTransactionSubmitRequest, EvmTransactionSubmitResponse, RedactedEvmSourceEvidence,
};
use mfm_evm_contract_model::{
    ContractArtifactConfig, ContractProfile, ContractProfileId, EvmContractContext,
    EvmNetworkContext, EvmNetworkId, LifecycleArtifactEvidenceRef, LifecycleKey,
};
use mfm_evm_signing::{primitive_signature_from_bytes, recover_signing_address};
use mfm_ids::ArtifactId;
use mfm_program::{
    build_root_with_registries, PublicOutputKey, ScopeKey, SideEffectSagaPolicy,
    SideEffectVerificationSpec, StateKey,
};
use mfm_program_derive::PublicOutputs;
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
use mfm_values::MfmConfig;
use serde_json::json;

const EXPECTED_SIGNER: &str = "0x5bb2b0ecce0dc85c2e21cf3747e6880074f738ec";

#[derive(PublicOutputs)]
#[mfm(schema = "mfm.evm.contract.test.direct_state_graph_outputs")]
struct DirectContractStateOutputs<'program, 'scope> {
    validation:
        mfm_program::Handle<'program, 'scope, mfm_evm_contract_model::ContextBoundValidationReport>,
}

/// The contract states remain executable through a direct typed graph. This deliberately bypasses
/// app entry-point discovery: the app is used only as generic launch/scheduler test support.
#[tokio::test]
async fn direct_contract_states_certify_and_run_without_app_entry_point_registration() {
    assert!(
        mfm_app::entry_point_summaries()
            .expect("entry-point discovery")
            .is_empty(),
        "the direct state graph must not depend on a public app entry point"
    );

    let (context, artifact) = direct_contract_context_and_artifact();
    let store = store::AsyncInMemoryRunStore::new();
    let artifacts = ContractArtifactOverlay::new(store.clone(), artifact);
    let factory = Arc::new(DirectContractStateRuntimeFactory::new(artifacts.clone()));
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

    let services = mfm_app::make_run_services(runners, store.clone(), artifacts, certification);
    let draft = direct_contract_state_graph(context).expect("direct state graph");
    let request = mfm_app::prepare_typed_program_run_launch_for_test(
        draft,
        Default::default(),
        services.certification_registry(),
        services.load_store_scope_id().await.expect("store scope"),
        Some(mfm_app::InvocationKey::new("direct-contract-state-graph").expect("invocation key")),
    )
    .expect("prepare direct state graph launch");
    let run_id = request.run_id.clone();

    let outcome = services
        .launch_run(request)
        .await
        .expect("run direct state graph");
    let (_, run, _) = outcome.into_response_parts();
    let run = run.expect("run response");
    let stream = store.load_run_stream(&run_id).await.expect("run stream");
    assert_eq!(
        run.run_mode,
        mfm_app::RunModeStatus::Completed,
        "direct contract-state graph failed: {run:?}"
    );
    assert!(
        stream
            .iter()
            .any(|event| matches!(event.payload(), events::KernelEventPayload::RunCompleted(_))),
        "the direct graph must reach a terminal completed run"
    );
}

fn direct_contract_state_graph(
    context: EvmContractContext,
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
                    deploy_action(),
                    (),
                    account_nonce_resource_claim()?,
                    SideEffectVerificationSpec::Receipt,
                )?
                .into_handle();
            let configured = root
                .scope()
                .side_effect::<ContextBoundConfigureContractState, _>(
                    StateKey::new("configure")?,
                    &context,
                    configure_action(),
                    ContextConfigureContractInputHandles { deployed },
                    account_nonce_resource_claim()?,
                    SideEffectVerificationSpec::Receipt,
                )?
                .into_handle();
            let validation = root.scope().state::<ContextBoundValidateContractState, _>(
                StateKey::new("validate")?,
                &context,
                ValidateAction::default(),
                ContextValidateContractInputHandles { configured },
            )?;
            root.bind_public_outputs(
                PublicOutputKey::new("validation")?,
                &DirectContractStateOutputs { validation },
            )
        },
    )
}

fn deploy_action() -> DeployAction {
    serde_json::from_value(json!({
        "signer": {
            "signer_ref": "deployer",
            "expected_signer_address": EXPECTED_SIGNER,
        },
        "receipt": {"poll_interval_ms": 1, "max_receipt_polls": 1},
    }))
    .expect("deploy action")
}

fn configure_action() -> ConfigureAction {
    serde_json::from_value(json!({
        "signer": {
            "signer_ref": "deployer",
            "expected_signer_address": EXPECTED_SIGNER,
        },
        "calls": [],
        "receipt": {"poll_interval_ms": 1, "max_receipt_polls": 1},
    }))
    .expect("configure action")
}

fn direct_contract_context_and_artifact() -> (EvmContractContext, ContractArtifact) {
    let profile: ContractArtifactConfig = serde_json::from_value(json!({
        "abi": {
            "json_text": serde_json::to_string(&json!([
                {"type": "constructor", "inputs": []}
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
        schema_id: Some(ContractArtifactConfig::schema_id().expect("artifact schema")),
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
            deployed_code_hash: None,
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

#[derive(Clone)]
struct ContractArtifact {
    bytes: Vec<u8>,
    evidence: store::ArtifactEvidenceRef,
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

struct DirectContractStateRuntimeFactory {
    artifacts: ContractArtifactOverlay,
    provider: Arc<DirectContractStateProvider>,
}

impl DirectContractStateRuntimeFactory {
    fn new(artifacts: ContractArtifactOverlay) -> Self {
        Self {
            artifacts,
            provider: Arc::new(DirectContractStateProvider::new()),
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

struct DirectContractStateProvider {
    evidence: RedactedEvmSourceEvidence,
}

impl DirectContractStateProvider {
    fn new() -> Self {
        Self {
            evidence: RedactedEvmSourceEvidence {
                network_id: CapabilityEvmNetworkId::new("ethereum-mainnet").expect("network id"),
                expected_chain_id: 1,
                observed_chain_id: 1,
                source_ref: EvmSourceRef::new("direct-test").expect("source ref"),
                policy_id: EvmSourcePolicyId::new("direct-test").expect("policy id"),
            },
        }
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
        _request: &'a EvmBlockReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBlockReadResponse> {
        let evidence = self.evidence.clone();
        Box::pin(async move {
            Ok(EvmBlockReadResponse {
                evidence,
                block_number: 42,
                block_hash: B256::repeat_byte(0x42),
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
        Box::pin(async move {
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
        Box::pin(async move {
            Ok(EvmReceiptReadResponse {
                evidence,
                transaction_hash,
                block_number: 42,
                status: true,
            })
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
        _request: &'a EvmCodeReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmCodeReadResponse> {
        let evidence = self.evidence.clone();
        Box::pin(async move {
            let code = vec![0x60, 0x00];
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
        _request: &'a EvmCallReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmCallReadResponse> {
        let evidence = self.evidence.clone();
        Box::pin(async move {
            Ok(EvmCallReadResponse {
                evidence,
                return_data: Vec::new(),
            })
        })
    }
}

impl EvmLogsReadProvider for DirectContractStateProvider {
    fn read_logs<'a>(
        &'a self,
        _request: &'a EvmLogsReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmLogsReadResponse> {
        let evidence = self.evidence.clone();
        Box::pin(async move {
            Ok(EvmLogsReadResponse {
                evidence,
                logs: Vec::new(),
            })
        })
    }
}

impl SigningProvider for DirectContractStateProvider {
    fn sign<'a>(&'a self, request: &'a SigningRequest) -> mfm_signing::SigningFuture<'a> {
        let signature = SignatureBytes::new(
            hex::decode(
                "48b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353\
                 efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c8041b",
            )
            .expect("signature bytes"),
        )
        .expect("valid signature");
        let result = (|| {
            let primitive = primitive_signature_from_bytes(&signature)
                .map_err(|_| SigningError::redacted_provider_failure("direct state signer"))?;
            let recovered =
                recover_signing_address(B256::from(*request.digest().as_bytes()), primitive)
                    .map_err(|_| SigningError::redacted_provider_failure("direct state signer"))?;
            let identity = PublicSigningIdentity::new(
                request.algorithm().clone(),
                None,
                Some(format!("{recovered:?}")),
            )?;
            SigningResult::for_request(request, identity, signature)
        })();
        Box::pin(async move { result })
    }
}
