use super::*;
use mfm_evm_capabilities::{
    EvmBlockReadResponse, EvmCallReadResponse, EvmCapabilityFuture, EvmChainIdentityResponse,
    EvmFeeReadResponse, EvmGasEstimateResponse, EvmLogsReadResponse, EvmNonceReadResponse,
    EvmReceiptReadResponse, RedactedEvmSourceEvidence,
};
use mfm_evm_signing::{
    primitive_signature_from_bytes, recover_signing_address,
    EvmTransactionStyle as SigningTransactionStyle,
};
use mfm_replay::v1::SideEffectReplayVerifier;
use mfm_signing::{
    PublicSigningIdentity, SignatureBytes, SigningError, SigningRequest, SigningResult,
};
use serde_json::json;
use std::sync::Mutex;

fn route() -> EvmContractRuntimeRoute {
    EvmContractRuntimeRoute::new(
        EvmSourceRef::new("local").expect("source"),
        EvmSourcePolicyId::new("test").expect("policy"),
    )
}

fn evidence() -> RedactedEvmSourceEvidence {
    RedactedEvmSourceEvidence {
        source_ref: route().source_ref().clone(),
        policy_id: route().policy_id().clone(),
        chain_id: 1,
    }
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
                    "name": "owner",
                    "inputs": [],
                    "outputs": [{ "name": "", "type": "bool" }],
                    "stateMutability": "view"
                }
            ]).to_string()
        },
        "bytecode": {
            "json_text": json!({"object": "0x6000"}).to_string()
        }
    })
}

fn deploy_config(style: &str) -> DeployPhaseConfig {
    let expected_signer_address = expected_test_signer_address(style);
    serde_json::from_value(json!({
        "artifact": artifact_json(),
        "network": {
            "network_id": "ethereum-mainnet",
            "expected_chain_id": 1
        },
        "signer": {
            "signer_ref": "deployer",
            "expected_signer_address": expected_signer_address
        },
        "transaction": match style {
            "legacy" => json!({"style": "legacy", "gas_price": "7"}),
            _ => json!({"style": "eip1559", "max_fee_per_gas": "11", "max_priority_fee_per_gas": "3"}),
        }
    }))
    .expect("deploy config")
}

fn validated_deploy_config(style: &str) -> ValidatedConfig<DeployPhaseConfig> {
    ValidatedConfig::new(deploy_config(style)).expect("valid deploy config")
}

fn deploy_intent(config: &ValidatedConfig<DeployPhaseConfig>) -> ContractDeployIntent {
    DeployContractState::new(config.clone())
        .expect("state")
        .prepare_intent(&())
        .expect("intent")
}

struct DeployPreparationFixture {
    route: EvmContractRuntimeRoute,
    providers: TestEvmProviders,
    config: ValidatedConfig<DeployPhaseConfig>,
    intent: ContractDeployIntent,
}

impl DeployPreparationFixture {
    fn new(style: &str) -> Self {
        let route = route();
        let providers = TestEvmProviders::preparation();
        let config = validated_deploy_config(style);
        let intent = deploy_intent(&config);
        Self {
            route,
            providers,
            config,
            intent,
        }
    }

    fn adapter(&self) -> EvmContractLifecycleAdapter<'_> {
        adapter(&self.route, &self.providers)
    }

    async fn prepare(&self) -> PreparedContractMutation {
        self.adapter()
            .prepare_deploy_invocation(&self.config, &self.intent)
            .await
            .expect("prepared")
    }
}

struct TestEvmProviders {
    mode: TestEvmProviderMode,
    submit_count: Arc<Mutex<u32>>,
    receipt_failure_reads: Arc<Mutex<u32>>,
}

#[derive(Clone, Copy)]
enum TestEvmProviderMode {
    Preparation,
    Recovery {
        receipt_mode: RecoveryReceiptMode,
        pending_nonce: u64,
        occupancy_mode: RecoveryOccupancyMode,
    },
    SubmitHashMismatch {
        returned_hash: B256,
    },
    ReceiptFailure,
    Finality,
}

impl TestEvmProviders {
    fn preparation() -> Self {
        Self::new(TestEvmProviderMode::Preparation)
    }

    fn recovery(
        receipt_mode: RecoveryReceiptMode,
        pending_nonce: u64,
        occupancy_mode: RecoveryOccupancyMode,
        submit_count: Arc<Mutex<u32>>,
    ) -> Self {
        Self {
            mode: TestEvmProviderMode::Recovery {
                receipt_mode,
                pending_nonce,
                occupancy_mode,
            },
            submit_count,
            receipt_failure_reads: Arc::new(Mutex::new(0)),
        }
    }

    fn receipt_failure(reads: Arc<Mutex<u32>>) -> Self {
        Self {
            mode: TestEvmProviderMode::ReceiptFailure,
            submit_count: Arc::new(Mutex::new(0)),
            receipt_failure_reads: reads,
        }
    }

    fn finality() -> Self {
        Self::new(TestEvmProviderMode::Finality)
    }

    fn submit_hash_mismatch(returned_hash: B256, submit_count: Arc<Mutex<u32>>) -> Self {
        Self {
            mode: TestEvmProviderMode::SubmitHashMismatch { returned_hash },
            submit_count,
            receipt_failure_reads: Arc::new(Mutex::new(0)),
        }
    }

    fn new(mode: TestEvmProviderMode) -> Self {
        Self {
            mode,
            submit_count: Arc::new(Mutex::new(0)),
            receipt_failure_reads: Arc::new(Mutex::new(0)),
        }
    }
}

fn runtime_from_provider(
    route: EvmContractRuntimeRoute,
    provider: TestEvmProviders,
) -> EvmContractRuntime {
    let provider = Arc::new(provider);
    let evm: Arc<dyn EvmContractProvider> = provider.clone();
    let signer: Arc<dyn SigningProvider> = provider;
    EvmContractRuntime::new(route, evm, signer)
}

fn unexpected_evm_call<'a, T>(capability: &'static str) -> EvmCapabilityFuture<'a, T> {
    Box::pin(async move { panic!("unexpected test EVM capability call: {capability}") })
}

fn unexpected_signing_call<'a>() -> mfm_signing::SigningFuture<'a> {
    Box::pin(async { panic!("unexpected test signing provider call") })
}

impl EvmChainIdentityProvider for TestEvmProviders {
    fn chain_identity<'a>(
        &'a self,
        _request: &'a EvmChainIdentityRequest,
    ) -> EvmCapabilityFuture<'a, EvmChainIdentityResponse> {
        match self.mode {
            TestEvmProviderMode::Preparation => Box::pin(async {
                Ok(EvmChainIdentityResponse {
                    evidence: evidence(),
                    chain_id: 1,
                    client_version: Some("test-client".to_owned()),
                })
            }),
            _ => unexpected_evm_call("chain_identity"),
        }
    }
}

impl EvmBlockReadProvider for TestEvmProviders {
    fn read_block<'a>(
        &'a self,
        _request: &'a EvmBlockReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmBlockReadResponse> {
        match self.mode {
            TestEvmProviderMode::Finality => Box::pin(async {
                Ok(EvmBlockReadResponse {
                    evidence: evidence(),
                    block_number: 64,
                    block_hash: B256::from([0x64; 32]),
                })
            }),
            _ => unexpected_evm_call("read_block"),
        }
    }
}

impl EvmNonceReadProvider for TestEvmProviders {
    fn read_nonce<'a>(
        &'a self,
        request: &'a EvmNonceReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmNonceReadResponse> {
        match self.mode {
            TestEvmProviderMode::Preparation => Box::pin(async {
                Ok(EvmNonceReadResponse {
                    evidence: evidence(),
                    nonce: 7,
                })
            }),
            TestEvmProviderMode::Recovery { pending_nonce, .. } => {
                assert_eq!(request.block, EvmBlockSelector::Pending);
                Box::pin(async move {
                    Ok(EvmNonceReadResponse {
                        evidence: evidence(),
                        nonce: pending_nonce,
                    })
                })
            }
            _ => unexpected_evm_call("read_nonce"),
        }
    }
}

impl EvmFeeReadProvider for TestEvmProviders {
    fn read_fee<'a>(
        &'a self,
        _request: &'a EvmFeeReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmFeeReadResponse> {
        match self.mode {
            TestEvmProviderMode::Preparation => Box::pin(async {
                Ok(EvmFeeReadResponse {
                    evidence: evidence(),
                    base_fee_per_gas: Some(5),
                    priority_fee_per_gas: Some(3),
                    max_fee_per_gas: Some(11),
                    legacy_gas_price: Some(7),
                })
            }),
            _ => unexpected_evm_call("read_fee"),
        }
    }
}

impl EvmGasEstimateProvider for TestEvmProviders {
    fn estimate_gas<'a>(
        &'a self,
        _request: &'a EvmGasEstimateRequest,
    ) -> EvmCapabilityFuture<'a, EvmGasEstimateResponse> {
        match self.mode {
            TestEvmProviderMode::Preparation => Box::pin(async {
                Ok(EvmGasEstimateResponse {
                    evidence: evidence(),
                    gas_limit: 21_000,
                })
            }),
            _ => unexpected_evm_call("estimate_gas"),
        }
    }
}

impl EvmTransactionSubmitProvider for TestEvmProviders {
    fn submit_transaction<'a>(
        &'a self,
        request: &'a EvmTransactionSubmitRequest,
    ) -> EvmCapabilityFuture<'a, mfm_evm_capabilities::EvmTransactionSubmitResponse> {
        match self.mode {
            TestEvmProviderMode::Recovery { .. } => {
                let submit_count = Arc::clone(&self.submit_count);
                let transaction_hash = request.signed_payload.transaction_hash();
                Box::pin(async move {
                    *submit_count.lock().expect("submit count") += 1;
                    Ok(mfm_evm_capabilities::EvmTransactionSubmitResponse {
                        evidence: evidence(),
                        transaction_hash,
                    })
                })
            }
            TestEvmProviderMode::SubmitHashMismatch { returned_hash } => {
                let submit_count = Arc::clone(&self.submit_count);
                Box::pin(async move {
                    *submit_count.lock().expect("submit count") += 1;
                    Ok(mfm_evm_capabilities::EvmTransactionSubmitResponse {
                        evidence: evidence(),
                        transaction_hash: returned_hash,
                    })
                })
            }
            _ => unexpected_evm_call("submit_transaction"),
        }
    }
}

impl EvmReceiptReadProvider for TestEvmProviders {
    fn read_receipt<'a>(
        &'a self,
        request: &'a EvmReceiptReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmReceiptReadResponse> {
        match self.mode {
            TestEvmProviderMode::Recovery { receipt_mode, .. } => {
                let transaction_hash = request.transaction_hash;
                Box::pin(async move {
                    match receipt_mode {
                        RecoveryReceiptMode::Landed => Ok(EvmReceiptReadResponse {
                            evidence: evidence(),
                            transaction_hash,
                            block_number: 42,
                            status: true,
                        }),
                        RecoveryReceiptMode::Pending => Err(EvmCapabilityError::ReceiptPending),
                        RecoveryReceiptMode::ProviderFailure => {
                            Err(EvmCapabilityError::redacted_provider_failure("test rpc"))
                        }
                    }
                })
            }
            TestEvmProviderMode::ReceiptFailure => {
                let reads = Arc::clone(&self.receipt_failure_reads);
                Box::pin(async move {
                    let mut reads = reads
                        .lock()
                        .map_err(|_| EvmCapabilityError::redacted_provider_failure("test evm"))?;
                    *reads += 1;
                    Err(EvmCapabilityError::redacted_provider_failure("test evm"))
                })
            }
            _ => unexpected_evm_call("read_receipt"),
        }
    }
}

impl EvmNonceOccupancyReadProvider for TestEvmProviders {
    fn read_nonce_occupancy<'a>(
        &'a self,
        request: &'a EvmNonceOccupancyReadRequest,
    ) -> EvmCapabilityFuture<'a, mfm_evm_capabilities::EvmNonceOccupancyReadResponse> {
        match self.mode {
            TestEvmProviderMode::Recovery { occupancy_mode, .. } => {
                let nonce = request.nonce;
                Box::pin(async move {
                    match occupancy_mode {
                        RecoveryOccupancyMode::Unknown => {
                            Ok(mfm_evm_capabilities::EvmNonceOccupancyReadResponse {
                                evidence: evidence(),
                                outcome: EvmNonceOccupancy::Unknown,
                            })
                        }
                        RecoveryOccupancyMode::Occupied { transaction_hash } => {
                            Ok(mfm_evm_capabilities::EvmNonceOccupancyReadResponse {
                                evidence: evidence(),
                                outcome: EvmNonceOccupancy::Occupied {
                                    transaction_hash,
                                    block_number: Some(43 + nonce),
                                },
                            })
                        }
                    }
                })
            }
            _ => unexpected_evm_call("read_nonce_occupancy"),
        }
    }
}

impl EvmCallReadProvider for TestEvmProviders {
    fn read_call<'a>(
        &'a self,
        _request: &'a EvmCallReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmCallReadResponse> {
        unexpected_evm_call("read_call")
    }
}

impl EvmLogsReadProvider for TestEvmProviders {
    fn read_logs<'a>(
        &'a self,
        _request: &'a EvmLogsReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmLogsReadResponse> {
        unexpected_evm_call("read_logs")
    }
}

impl SigningProvider for TestEvmProviders {
    fn sign<'a>(&'a self, request: &'a SigningRequest) -> mfm_signing::SigningFuture<'a> {
        match self.mode {
            TestEvmProviderMode::Preparation
            | TestEvmProviderMode::Recovery { .. }
            | TestEvmProviderMode::SubmitHashMismatch { .. } => {
                let result = test_signing_result(request);
                Box::pin(async move { result })
            }
            _ => unexpected_signing_call(),
        }
    }
}

fn test_signature_bytes() -> SignatureBytes {
    SignatureBytes::new(
        hex_to_bytes(
            "0x48b55bfa915ac795c431978d8a6a992b628d557da5ff759b307d495a36649353\
             efffd310ac743f371de3b9f7f9cb56c0b28ad43601b4ab949f53faa07bd2c8041b",
        )
        .expect("signature hex"),
    )
    .expect("signature bytes")
}

fn test_signing_result(request: &SigningRequest) -> mfm_signing::Result<SigningResult> {
    let signature = test_signature_bytes();
    let primitive = primitive_signature_from_bytes(&signature)
        .map_err(|_| SigningError::redacted_provider_failure("test signer"))?;
    let recovered = recover_signing_address(B256::from(*request.digest().as_bytes()), primitive)
        .map_err(|_| SigningError::redacted_provider_failure("test signer"))?;
    let identity = PublicSigningIdentity::new(
        request.algorithm().clone(),
        None,
        Some(format!("{recovered:?}")),
    )?;
    SigningResult::for_request(request, identity, signature)
}

fn expected_test_signer_address(style: &str) -> String {
    let signer_ref = SignerRef::new("deployer").expect("signer ref");
    let expected_from = Address::from([0_u8; 20]);
    let data = vec![0x60, 0x00];
    let request = match style {
        "legacy" => EvmSigningRequest::legacy(
            signer_ref,
            LegacyTxToSign {
                to: None,
                value_wei: 0,
                chain_id: 1,
                nonce: 7,
                gas_price_wei: 7,
                gas_limit: 21_000,
                data,
            },
            expected_from,
        ),
        _ => EvmSigningRequest::eip1559(
            signer_ref,
            Eip1559TxToSign {
                to: None,
                value_wei: 0,
                chain_id: 1,
                nonce: 7,
                max_fee_per_gas: 11,
                max_priority_fee_per_gas: 3,
                gas_limit: 21_000,
                data,
            },
            expected_from,
        ),
    }
    .expect("signing request");
    let primitive = primitive_signature_from_bytes(&test_signature_bytes()).expect("signature");
    let recovered =
        recover_signing_address(request.signing_hash(), primitive).expect("recovered address");
    format!("{recovered:?}")
}

fn adapter<'a>(
    route: &'a EvmContractRuntimeRoute,
    providers: &'a TestEvmProviders,
) -> EvmContractLifecycleAdapter<'a> {
    EvmContractLifecycleAdapter::new(
        route,
        EvmContractMutationProviders {
            chain_identity: providers,
            block: providers,
            nonce: providers,
            fee: providers,
            gas: providers,
            signer: providers,
            submit: providers,
            receipt: providers,
            nonce_occupancy: providers,
        },
        EvmContractReadProviders {
            chain_identity: providers,
            call: providers,
            logs: providers,
        },
    )
}

#[test]
fn executable_identity_summary_matches_golden() {
    assert_eq!(
        executable_identity_summary([SIDE_EFFECT_FACTORY, READ_FACTORY]),
        [
            "factory=apply_side_effect;cargo_digest=content:sha256-jcs-v1:685edb3e7cd5decfb0f17613a76568a2c5c572024d6db20bf0803d61ee0657e4;binary_digest=content:sha256-jcs-v1:4a583ef5eb9bb2ce3f01767ddffc34e0744290d029bde3d69967b272a74f302f;nix_derivation=false;nix_output=false",
            "factory=read_external;cargo_digest=content:sha256-jcs-v1:685edb3e7cd5decfb0f17613a76568a2c5c572024d6db20bf0803d61ee0657e4;binary_digest=content:sha256-jcs-v1:4a583ef5eb9bb2ce3f01767ddffc34e0744290d029bde3d69967b272a74f302f;nix_derivation=false;nix_output=false",
        ]
    );
}

#[tokio::test]
async fn deploy_preparation_defaults_to_eip1559_contract_creation() {
    let fixture = DeployPreparationFixture::new("eip1559");
    let prepared = fixture.prepare().await;

    assert_eq!(prepared.evidence().transactions.len(), 1);
    assert_eq!(
        prepared.evidence().transactions[0].style,
        PreparedContractTransactionStyle::Eip1559
    );
    assert_eq!(prepared.evidence().transactions[0].to_address, None);
    assert_eq!(
        prepared.signing_requests()[0].style(),
        SigningTransactionStyle::Eip1559
    );
}

#[tokio::test]
async fn deploy_preparation_supports_legacy_contract_creation() {
    let fixture = DeployPreparationFixture::new("legacy");
    let prepared = fixture.prepare().await;

    assert_eq!(
        prepared.evidence().transactions[0].style,
        PreparedContractTransactionStyle::Legacy
    );
    assert_eq!(
        prepared.signing_requests()[0].style(),
        SigningTransactionStyle::Legacy
    );
}

#[tokio::test]
async fn prepared_invocation_evidence_excludes_live_and_secret_surfaces() {
    let fixture = DeployPreparationFixture::new("eip1559");
    let prepared = fixture.prepare().await;

    ensure_prepared_invocation_public(prepared.evidence()).expect("public evidence");
    let evidence = prepared.evidence();
    let rendered_value = serde_json::to_value(evidence).expect("json value");
    assert_eq!(
        sorted_json_keys(&rendered_value),
        vec![
            "expected_chain_id",
            "expected_signer_address",
            "max_receipt_polls",
            "network_id",
            "phase",
            "poll_interval_ms",
            "prepared_version",
            "signer_ref",
            "transactions",
        ]
    );
    assert_eq!(evidence.signer_ref, "deployer");
    assert_eq!(
        evidence.expected_signer_address,
        expected_test_signer_address("eip1559")
    );
    let transaction = evidence.transactions.first().expect("transaction");
    let rendered_transaction = rendered_value["transactions"][0].clone();
    assert_eq!(
        sorted_json_keys(&rendered_transaction),
        vec![
            "chain_id",
            "data_digest",
            "data_len",
            "expected_transaction_hash",
            "gas_limit",
            "gas_price",
            "index",
            "max_fee_per_gas",
            "max_priority_fee_per_gas",
            "nonce",
            "signing_digest",
            "style",
            "to_address",
            "value_wei",
        ]
    );
    assert_eq!(transaction.nonce, 7);
    assert_eq!(transaction.gas_limit, 21_000);
    assert_eq!(transaction.max_fee_per_gas.as_deref(), Some("11"));
    assert_eq!(transaction.max_priority_fee_per_gas.as_deref(), Some("3"));
    assert!(transaction
        .data_digest
        .starts_with("content:sha256-jcs-v1:"));
    assert_eq!(transaction.data_len, 2);
    assert!(transaction.signing_digest.starts_with("0x"));
    assert_eq!(transaction.signing_digest.len(), 66);
    assert!(transaction.expected_transaction_hash.starts_with("0x"));
    assert_eq!(transaction.expected_transaction_hash.len(), 66);

    let rendered = serde_json::to_string(prepared.evidence()).expect("json");
    let rendered = rendered.to_ascii_lowercase();
    for forbidden in forbidden_prepared_terms() {
        assert!(
            !rendered.contains(&forbidden),
            "prepared invocation evidence contains forbidden runtime surface"
        );
    }
}

#[tokio::test]
async fn prepared_invocation_reconstruction_preserves_submission_anchor() {
    let fixture = DeployPreparationFixture::new("eip1559");
    let prepared = fixture.prepare().await;
    let reconstructed = fixture
        .adapter()
        .reconstruct_deploy_invocation(&fixture.config, &fixture.intent, prepared.evidence())
        .expect("reconstructed");

    assert_eq!(reconstructed.evidence(), prepared.evidence());
    assert_eq!(
        reconstructed.evidence().transactions[0].expected_transaction_hash,
        prepared.evidence().transactions[0].expected_transaction_hash
    );
}

#[tokio::test]
async fn submit_rejects_resigned_hash_mismatch_before_broadcast() {
    let fixture = DeployPreparationFixture::new("eip1559");
    let prepared = fixture.prepare().await;
    let mut evidence = prepared.evidence().clone();
    evidence.transactions[0].expected_transaction_hash =
        "0x1111111111111111111111111111111111111111111111111111111111111111".to_owned();
    let adapter = fixture.adapter();
    let reconstructed = adapter
        .reconstruct_deploy_invocation(&fixture.config, &fixture.intent, &evidence)
        .expect("reconstructed");

    assert!(matches!(
        adapter.submit_prepared(&reconstructed).await,
        Err(EvmContractAdapterError::TransactionHashMismatch)
    ));
}

#[tokio::test]
async fn recovery_observes_landed_anchor_without_resubmitting() {
    let fixture = DeployPreparationFixture::new("eip1559");
    let prepared = fixture.prepare().await;
    let submit_count = Arc::new(Mutex::new(0_u32));
    let runtime = runtime_from_provider(
        fixture.route,
        TestEvmProviders::recovery(
            RecoveryReceiptMode::Landed,
            7,
            RecoveryOccupancyMode::Unknown,
            Arc::clone(&submit_count),
        ),
    );

    let decision = submit_or_recover_contract_submission(
        &runtime,
        &prepared,
        SideEffectProtocolAction::SubmitOrRecoverSubmission {
            invocation_epoch: 1,
        },
    )
    .await
    .expect("recovered");

    let SideEffectSubmissionDecision::Observed(submissions) = decision else {
        panic!("expected observed anchor recovery");
    };
    assert_eq!(*submit_count.lock().expect("submit count"), 0);
    assert_eq!(
        submissions.transactions[0].transaction_hash,
        prepared.evidence().transactions[0].expected_transaction_hash
    );
}

#[tokio::test]
async fn recovery_rebroadcasts_unlanded_anchor_after_resign_match() {
    let fixture = DeployPreparationFixture::new("eip1559");
    let prepared = fixture.prepare().await;
    let submit_count = Arc::new(Mutex::new(0_u32));
    let runtime = runtime_from_provider(
        fixture.route,
        TestEvmProviders::recovery(
            RecoveryReceiptMode::Pending,
            7,
            RecoveryOccupancyMode::Unknown,
            Arc::clone(&submit_count),
        ),
    );

    let decision = submit_or_recover_contract_submission(
        &runtime,
        &prepared,
        SideEffectProtocolAction::SubmitOrRecoverSubmission {
            invocation_epoch: 1,
        },
    )
    .await
    .expect("recovered");

    let SideEffectSubmissionDecision::Observed(submissions) = decision else {
        panic!("expected observed rebroadcast");
    };
    assert_eq!(*submit_count.lock().expect("submit count"), 1);
    assert_eq!(
        submissions.transactions[0].transaction_hash,
        prepared.evidence().transactions[0].expected_transaction_hash
    );
}

#[tokio::test]
async fn recovery_does_not_rebroadcast_when_resign_hash_mismatches_anchor() {
    let fixture = DeployPreparationFixture::new("eip1559");
    let prepared = fixture.prepare().await;
    let mut evidence = prepared.evidence().clone();
    evidence.transactions[0].expected_transaction_hash =
        "0x1111111111111111111111111111111111111111111111111111111111111111".to_owned();
    let reconstructed = fixture
        .adapter()
        .reconstruct_deploy_invocation(&fixture.config, &fixture.intent, &evidence)
        .expect("reconstructed");
    let submit_count = Arc::new(Mutex::new(0_u32));
    let runtime = runtime_from_provider(
        fixture.route,
        TestEvmProviders::recovery(
            RecoveryReceiptMode::Pending,
            7,
            RecoveryOccupancyMode::Unknown,
            Arc::clone(&submit_count),
        ),
    );

    let decision = submit_or_recover_contract_submission(
        &runtime,
        &reconstructed,
        SideEffectProtocolAction::SubmitOrRecoverSubmission {
            invocation_epoch: 1,
        },
    )
    .await
    .expect("recovered");

    let SideEffectSubmissionDecision::Ambiguous {
        ambiguity_code,
        evidence,
    } = decision
    else {
        panic!("expected ambiguous mismatched anchor evidence");
    };
    assert_eq!(*submit_count.lock().expect("submit count"), 0);
    assert_eq!(ambiguity_code.as_str(), "mfm.evm.transaction_hash_mismatch");
    assert_eq!(
        evidence.transactions[0].transaction_hash,
        "0x1111111111111111111111111111111111111111111111111111111111111111"
    );
}

#[tokio::test]
async fn fresh_submit_records_ambiguity_when_provider_returns_mismatched_hash() {
    let fixture = DeployPreparationFixture::new("eip1559");
    let prepared = fixture.prepare().await;
    let submit_count = Arc::new(Mutex::new(0_u32));
    let mismatched_hash = B256::from([0x11; 32]);
    let runtime = runtime_from_provider(
        fixture.route,
        TestEvmProviders::submit_hash_mismatch(mismatched_hash, Arc::clone(&submit_count)),
    );

    let decision = submit_or_recover_contract_submission(
        &runtime,
        &prepared,
        SideEffectProtocolAction::StartPreparedAndSubmitOrRecoverSubmission {
            invocation_epoch: 1,
        },
    )
    .await
    .expect("fresh submit mismatch handled");

    let SideEffectSubmissionDecision::Ambiguous {
        ambiguity_code,
        evidence,
    } = decision
    else {
        panic!("expected ambiguous provider hash mismatch evidence");
    };
    assert_eq!(*submit_count.lock().expect("submit count"), 1);
    assert_eq!(ambiguity_code.as_str(), "mfm.evm.transaction_hash_mismatch");
    assert_eq!(
        evidence.transactions[0].transaction_hash,
        prepared.evidence().transactions[0].expected_transaction_hash
    );
    assert_ne!(
        evidence.transactions[0].transaction_hash,
        format!("{mismatched_hash:?}")
    );
}

#[tokio::test]
async fn recovery_keeps_advanced_nonce_unknown_without_occupancy_proof() {
    let fixture = DeployPreparationFixture::new("eip1559");
    let prepared = fixture.prepare().await;
    let submit_count = Arc::new(Mutex::new(0_u32));
    let runtime = runtime_from_provider(
        fixture.route,
        TestEvmProviders::recovery(
            RecoveryReceiptMode::Pending,
            8,
            RecoveryOccupancyMode::Unknown,
            Arc::clone(&submit_count),
        ),
    );

    let decision = submit_or_recover_contract_submission(
        &runtime,
        &prepared,
        SideEffectProtocolAction::SubmitOrRecoverSubmission {
            invocation_epoch: 1,
        },
    )
    .await
    .expect("recovered");

    let SideEffectSubmissionDecision::Unknown(evidence) = decision else {
        panic!("expected unknown occupied nonce evidence");
    };
    assert_eq!(*submit_count.lock().expect("submit count"), 0);
    assert_eq!(
        evidence.transactions[0].transaction_hash,
        prepared.evidence().transactions[0].expected_transaction_hash
    );
}

#[tokio::test]
async fn recovery_proves_not_submitted_when_foreign_transaction_occupies_nonce() {
    let fixture = DeployPreparationFixture::new("eip1559");
    let prepared = fixture.prepare().await;
    let submit_count = Arc::new(Mutex::new(0_u32));
    let occupying_hash = "0x2222222222222222222222222222222222222222222222222222222222222222"
        .parse::<B256>()
        .expect("occupying hash");
    let runtime = runtime_from_provider(
        fixture.route,
        TestEvmProviders::recovery(
            RecoveryReceiptMode::Pending,
            8,
            RecoveryOccupancyMode::Occupied {
                transaction_hash: occupying_hash,
            },
            Arc::clone(&submit_count),
        ),
    );

    let decision = submit_or_recover_contract_submission(
        &runtime,
        &prepared,
        SideEffectProtocolAction::SubmitOrRecoverSubmission {
            invocation_epoch: 1,
        },
    )
    .await
    .expect("recovered");

    let SideEffectSubmissionDecision::NotSubmitted(proof) = decision else {
        panic!("expected not-submitted proof");
    };
    assert_eq!(*submit_count.lock().expect("submit count"), 0);
    assert_eq!(
        proof.expected_transaction_hash,
        prepared.evidence().transactions[0].expected_transaction_hash
    );
    assert_eq!(
        proof.occupying_transaction_hash,
        "0x2222222222222222222222222222222222222222222222222222222222222222"
    );
    assert_eq!(proof.nonce, prepared.evidence().transactions[0].nonce);
    assert_eq!(proof.evidence_chain_id, 1);
}

#[tokio::test]
async fn recovery_records_unknown_on_transient_anchor_read_failure() {
    let fixture = DeployPreparationFixture::new("eip1559");
    let prepared = fixture.prepare().await;
    let submit_count = Arc::new(Mutex::new(0_u32));
    let runtime = runtime_from_provider(
        fixture.route,
        TestEvmProviders::recovery(
            RecoveryReceiptMode::ProviderFailure,
            7,
            RecoveryOccupancyMode::Unknown,
            Arc::clone(&submit_count),
        ),
    );

    let decision = submit_or_recover_contract_submission(
        &runtime,
        &prepared,
        SideEffectProtocolAction::SubmitOrRecoverSubmission {
            invocation_epoch: 1,
        },
    )
    .await
    .expect("recovered");

    let SideEffectSubmissionDecision::Unknown(evidence) = decision else {
        panic!("expected unknown recovery evidence");
    };
    assert_eq!(*submit_count.lock().expect("submit count"), 0);
    assert_eq!(
        evidence.transactions[0].transaction_hash,
        prepared.evidence().transactions[0].expected_transaction_hash
    );
}

#[test]
fn prepared_invocation_guard_rejects_forbidden_runtime_terms() {
    for forbidden in forbidden_prepared_terms() {
        let mut prepared = prepared_invocation_fixture();
        prepared.signer_ref = format!("deployer-{forbidden}");

        assert!(
            matches!(
                ensure_prepared_invocation_public(&prepared),
                Err(EvmContractAdapterError::PreparedInvocationLeak)
            ),
            "expected forbidden prepared term {forbidden} to be rejected"
        );
    }
}

fn prepared_invocation_fixture() -> PreparedContractInvocation {
    PreparedContractInvocation {
        prepared_version: 1,
        phase: ContractMutationPhase::Deploy,
        network_id: "ethereum-mainnet".to_owned(),
        expected_chain_id: 1,
        signer_ref: "deployer".to_owned(),
        expected_signer_address: "0x0f65fe9276bc9a24ae7083ae28e2660ef72df99e".to_owned(),
        transactions: vec![PreparedContractTransactionEvidence {
            index: 0,
            style: PreparedContractTransactionStyle::Eip1559,
            chain_id: 1,
            nonce: 7,
            to_address: None,
            value_wei: "0".to_owned(),
            gas_limit: 21_000,
            max_fee_per_gas: Some("11".to_owned()),
            max_priority_fee_per_gas: Some("3".to_owned()),
            gas_price: None,
            data_digest:
                "content:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_owned(),
            data_len: 2,
            signing_digest:
                "0x0000000000000000000000000000000000000000000000000000000000000000"
                    .to_owned(),
            expected_transaction_hash:
                "0x1111111111111111111111111111111111111111111111111111111111111111"
                    .to_owned(),
        }],
        poll_interval_ms: 1_000,
        max_receipt_polls: 10,
    }
}

fn sorted_json_keys(value: &serde_json::Value) -> Vec<&str> {
    let mut keys = value
        .as_object()
        .expect("json object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys
}

#[derive(Clone, Copy)]
enum RecoveryReceiptMode {
    Landed,
    Pending,
    ProviderFailure,
}

#[derive(Clone, Copy)]
enum RecoveryOccupancyMode {
    Unknown,
    Occupied { transaction_hash: B256 },
}

#[test]
fn replay_verifier_uses_contract_namespace() {
    let verifier = EvmContractLifecycleReplayVerifier::new().expect("verifier");

    assert_eq!(verifier.verifier_id().as_str(), REPLAY_VERIFIER_ID);
    assert!(!verifier
        .verifier_id()
        .as_str()
        .contains(&["d", "cv"].concat()));
}

#[tokio::test]
async fn receipt_polling_does_not_retry_permanent_capability_failures() {
    let reads = Arc::new(Mutex::new(0_u32));
    let runtime = runtime_from_provider(
        route(),
        TestEvmProviders::receipt_failure(Arc::clone(&reads)),
    );
    let transaction_hash = "0x1111111111111111111111111111111111111111111111111111111111111111";
    let prepared = PreparedContractInvocation {
        prepared_version: 1,
        phase: ContractMutationPhase::Deploy,
        network_id: "reth-dev".to_owned(),
        expected_chain_id: 31337,
        signer_ref: "deployer".to_owned(),
        expected_signer_address: "0x0000000000000000000000000000000000000001".to_owned(),
        transactions: vec![PreparedContractTransactionEvidence {
            index: 0,
            style: PreparedContractTransactionStyle::Eip1559,
            chain_id: 31337,
            nonce: 7,
            to_address: None,
            value_wei: "0".to_owned(),
            gas_limit: 21_000,
            max_fee_per_gas: Some("11".to_owned()),
            max_priority_fee_per_gas: Some("3".to_owned()),
            gas_price: None,
            data_digest: "content:sha256-jcs-v1:0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
            data_len: 0,
            signing_digest: transaction_hash.to_owned(),
            expected_transaction_hash: transaction_hash.to_owned(),
        }],
        poll_interval_ms: 25,
        max_receipt_polls: 3,
    };
    let submissions = ContractTransactionSubmissions {
        submissions_version: 1,
        transactions: vec![ContractTransactionSubmission {
            submission_version: 1,
            transaction_hash: transaction_hash.to_owned(),
            signer_public_key: None,
        }],
    };

    let error = read_receipts_with_poll(&runtime, &prepared, &submissions)
        .await
        .expect_err("permanent capability failure");

    assert!(error.to_string().contains("EVM capability failed"));
    assert_eq!(*reads.lock().expect("reads"), 1);
}

#[tokio::test]
async fn finality_confirmation_requires_certified_depth() {
    let runtime = runtime_from_provider(route(), TestEvmProviders::finality());
    let receipt = ContractTransactionReceipt {
        receipt_version: 1,
        transaction_hash: "0x1111111111111111111111111111111111111111111111111111111111111111"
            .to_owned(),
        block_number: 63,
        status: true,
        receipt_evidence: None,
    };

    let confirmations =
        verified_finality_confirmations(&runtime, 1, std::slice::from_ref(&receipt), 2)
            .await
            .expect("sufficient confirmations");
    assert_eq!(confirmations, 2);

    let error = verified_finality_confirmations(&runtime, 1, std::slice::from_ref(&receipt), 3)
        .await
        .expect_err("insufficient confirmations");
    assert!(matches!(error, mfm_runtime::RuntimeError::Blocked(_)));
}

#[test]
fn replay_confirmation_depth_rejects_insufficient_certified_depth() {
    assert!(ensure_replay_confirmation_depth(3, 3).is_ok());
    let error = ensure_replay_confirmation_depth(2, 3).expect_err("insufficient replay depth");
    assert_eq!(error.kind, replay::ReplayErrorKind::SideEffectMismatch);
}

#[test]
fn adapter_source_has_no_concrete_store_or_signer_provider_coupling() {
    let source = include_str!("lib.rs");
    for forbidden in [
        ["artifact", "_store", "_fs"].concat(),
        ["Fs", "Typed", "Artifact", "Store"].concat(),
        ["Key", "store"].concat(),
        ["Runtime", "Secret", "Source"].concat(),
        ["Signer", "Provider", "Runtime", "Config"].concat(),
        ["MFM", "_EVM", "_RPC"].concat(),
    ] {
        assert!(
            !source.contains(&forbidden),
            "adapter source contains forbidden coupling {forbidden}"
        );
    }
}

fn executable_identity_summary(factories: [&str; 2]) -> Vec<String> {
    factories
        .into_iter()
        .map(|factory| {
            let identity = executable(events::RunnerFactoryId::new(factory).expect("factory id"))
                .expect("executable identity");
            format!(
                "factory={};cargo_digest={};binary_digest={};nix_derivation={};nix_output={}",
                identity.factory_id,
                identity.cargo_package_digest,
                identity.binary_digest,
                identity.nix_derivation_hash.is_some(),
                identity.nix_output_hash.is_some()
            )
        })
        .collect()
}
