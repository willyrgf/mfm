use super::*;

pub(super) fn deploy_action(chain_id: u64) -> ValidatedConfig<DeployAction> {
    deploy_action_for_style("eip1559", chain_id)
}

pub(super) fn deploy_action_for_style(style: &str, chain_id: u64) -> ValidatedConfig<DeployAction> {
    ValidatedConfig::new(
        serde_json::from_value(json!({
            "signer": {
                "signer_ref": "deployer",
                "expected_signer_address": expected_test_signer_address_for_chain(style, chain_id),
            },
            "transaction": match style {
                "legacy" => json!({"style": "legacy", "gas_price": "7"}),
                _ => json!({"style": "eip1559", "max_fee_per_gas": "11", "max_priority_fee_per_gas": "3"}),
            },
        }))
        .expect("deploy action"),
    )
    .expect("valid deploy action")
}

pub(super) fn configure_action(calls: serde_json::Value) -> ConfigureAction {
    serde_json::from_value(json!({
        "signer": {
            "signer_ref": "deployer",
            "expected_signer_address": "0x0000000000000000000000000000000000000000",
        },
        "calls": calls,
    }))
    .expect("configure action")
}

pub(super) fn contract_artifact_config() -> ContractArtifactConfig {
    serde_json::from_value(artifact_json()).expect("artifact config")
}

pub(super) struct DeployPreparationFixture {
    pub(super) providers: TestEvmProviders,
    pub(super) action: ValidatedConfig<DeployAction>,
    pub(super) context: mfm_program::CertifiedContext<EvmContractContext>,
    pub(super) artifact: ContractArtifactConfig,
    pub(super) intent: ContextContractDeployIntent,
}

pub(super) type PreparedDeployFixture = (DeployPreparationFixture, PreparedContractMutation);

impl DeployPreparationFixture {
    pub(super) fn new(style: &str) -> Self {
        let providers = TestEvmProviders::preparation();
        let action = deploy_action_for_style(style, 1);
        let context = certified_contract_context("ethereum-mainnet", 1);
        let artifact = contract_artifact_config();
        let intent = ContextBoundDeployContractState::new(action.clone())
            .expect("state")
            .prepare_intent(&(), &context)
            .expect("intent");
        Self {
            providers,
            action,
            context,
            artifact,
            intent,
        }
    }

    pub(super) fn adapter(&self) -> EvmContractLifecycleAdapter<'_> {
        adapter(&self.providers)
    }

    pub(super) async fn prepare(&self) -> PreparedContractMutation {
        self.adapter()
            .prepare_context_deploy_invocation(
                &self.action,
                &self.context,
                &self.artifact,
                &self.intent,
            )
            .await
            .expect("prepared")
    }

    pub(super) fn reconstruct_with_hash(
        &self,
        prepared: &PreparedContractMutation,
        expected_transaction_hash: &str,
    ) -> PreparedContractMutation {
        let mut evidence = prepared.evidence().clone();
        evidence.transactions[0].expected_transaction_hash = expected_transaction_hash.to_owned();
        self.adapter()
            .reconstruct_context_deploy_invocation(
                &self.action,
                &self.context,
                &self.artifact,
                &self.intent,
                &evidence,
            )
            .expect("reconstructed")
    }
}

pub(super) async fn prepared_deploy_fixture(style: &str) -> PreparedDeployFixture {
    let fixture = DeployPreparationFixture::new(style);
    let prepared = fixture.prepare().await;
    (fixture, prepared)
}

pub(super) struct TestEvmProviders {
    mode: TestEvmProviderMode,
    evidence: RedactedEvmSourceEvidence,
    submit_count: Arc<Mutex<u32>>,
    receipt_failure_reads: Arc<Mutex<u32>>,
}

#[derive(Clone, Copy)]
pub(super) enum TestEvmProviderMode {
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
    pub(super) fn preparation() -> Self {
        Self::new(TestEvmProviderMode::Preparation)
    }

    pub(super) fn preparation_for(network_id: &str, expected_chain_id: u64) -> Self {
        Self::preparation().with_source(network_id, expected_chain_id)
    }

    pub(super) fn recovery(
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
            evidence: test_evm_evidence(),
            submit_count,
            receipt_failure_reads: Arc::new(Mutex::new(0)),
        }
    }

    pub(super) fn receipt_failure(reads: Arc<Mutex<u32>>) -> Self {
        Self {
            mode: TestEvmProviderMode::ReceiptFailure,
            evidence: test_evm_evidence(),
            submit_count: Arc::new(Mutex::new(0)),
            receipt_failure_reads: reads,
        }
    }

    pub(super) fn finality() -> Self {
        Self::new(TestEvmProviderMode::Finality)
    }

    pub(super) fn submit_hash_mismatch(returned_hash: B256, submit_count: Arc<Mutex<u32>>) -> Self {
        Self {
            mode: TestEvmProviderMode::SubmitHashMismatch { returned_hash },
            evidence: test_evm_evidence(),
            submit_count,
            receipt_failure_reads: Arc::new(Mutex::new(0)),
        }
    }

    fn new(mode: TestEvmProviderMode) -> Self {
        Self {
            mode,
            evidence: test_evm_evidence(),
            submit_count: Arc::new(Mutex::new(0)),
            receipt_failure_reads: Arc::new(Mutex::new(0)),
        }
    }

    pub(super) fn with_source(mut self, network_id: &str, expected_chain_id: u64) -> Self {
        self.evidence = evidence_for_source(
            &EvmNetworkId::new(network_id).expect("test network"),
            expected_chain_id,
        );
        self
    }
}

pub(super) fn runtime_from_provider(provider: TestEvmProviders) -> EvmContractRuntime {
    let provider = Arc::new(provider);
    let evm: Arc<dyn EvmContractProvider> = provider.clone();
    let signer: Arc<dyn SigningProvider> = provider;
    EvmContractRuntime::new(evm, signer)
}

pub(super) type ContractSubmissionDecision = SideEffectSubmissionDecision<
    ContractTransactionSubmissions,
    ContractTransactionSubmissions,
    ContractNotSubmittedProof,
    ContractTransactionSubmissions,
>;

pub(super) async fn recover_with_provider(
    receipt_mode: RecoveryReceiptMode,
    pending_nonce: u64,
    occupancy_mode: RecoveryOccupancyMode,
    prepared: &PreparedContractMutation,
) -> (ContractSubmissionDecision, Arc<Mutex<u32>>) {
    let submit_count = Arc::new(Mutex::new(0));
    let runtime = runtime_from_provider(TestEvmProviders::recovery(
        receipt_mode,
        pending_nonce,
        occupancy_mode,
        Arc::clone(&submit_count),
    ));
    (recover_submission(&runtime, prepared).await, submit_count)
}

pub(super) async fn recover_submission(
    runtime: &EvmContractRuntime,
    prepared: &PreparedContractMutation,
) -> ContractSubmissionDecision {
    submit_or_recover_contract_submission(
        runtime,
        prepared,
        SideEffectProtocolAction::SubmitOrRecoverSubmission {
            invocation_epoch: 1,
        },
    )
    .await
    .expect("recovered")
}

pub(super) async fn start_with_submit_hash_mismatch(
    mismatched_hash: B256,
    prepared: &PreparedContractMutation,
) -> (ContractSubmissionDecision, Arc<Mutex<u32>>) {
    let submit_count = Arc::new(Mutex::new(0));
    let runtime = runtime_from_provider(TestEvmProviders::submit_hash_mismatch(
        mismatched_hash,
        Arc::clone(&submit_count),
    ));
    let decision = submit_or_recover_contract_submission(
        &runtime,
        prepared,
        SideEffectProtocolAction::StartPreparedAndSubmitOrRecoverSubmission {
            invocation_epoch: 1,
        },
    )
    .await
    .expect("submission decision");

    (decision, submit_count)
}

pub(super) fn expected_transaction_hash(prepared: &PreparedContractMutation) -> &str {
    &prepared.evidence().transactions[0].expected_transaction_hash
}

pub(super) fn assert_submit_count(submit_count: &Arc<Mutex<u32>>, expected: u32) {
    assert_eq!(*submit_count.lock().expect("submit count"), expected);
}

pub(super) fn assert_observed_submission(
    decision: ContractSubmissionDecision,
    submit_count: &Arc<Mutex<u32>>,
    expected_count: u32,
    prepared: &PreparedContractMutation,
) {
    let SideEffectSubmissionDecision::Observed(submissions) = decision else {
        panic!("expected observed submission");
    };
    assert_submit_count(submit_count, expected_count);
    assert_eq!(
        submissions.transactions[0].transaction_hash,
        expected_transaction_hash(prepared)
    );
}

pub(super) fn assert_unknown_submission(
    decision: ContractSubmissionDecision,
    submit_count: &Arc<Mutex<u32>>,
    prepared: &PreparedContractMutation,
) {
    let SideEffectSubmissionDecision::Unknown(evidence) = decision else {
        panic!("expected unknown submission");
    };
    assert_submit_count(submit_count, 0);
    assert_eq!(
        evidence.transactions[0].transaction_hash,
        expected_transaction_hash(prepared)
    );
}

pub(super) fn assert_hash_mismatch_ambiguity(
    decision: ContractSubmissionDecision,
    submit_count: &Arc<Mutex<u32>>,
    expected_count: u32,
) -> ContractTransactionSubmissions {
    let SideEffectSubmissionDecision::Ambiguous {
        ambiguity_code,
        evidence,
    } = decision
    else {
        panic!("expected transaction-hash mismatch ambiguity");
    };
    assert_submit_count(submit_count, expected_count);
    assert_eq!(ambiguity_code.as_str(), "mfm.evm.transaction_hash_mismatch");
    evidence
}

pub(super) async fn assert_recovery_observed(
    receipt_mode: RecoveryReceiptMode,
    expected_count: u32,
) {
    let (_fixture, prepared) = prepared_deploy_fixture("eip1559").await;
    let (decision, submit_count) =
        recover_with_provider(receipt_mode, 7, RecoveryOccupancyMode::Unknown, &prepared).await;
    assert_observed_submission(decision, &submit_count, expected_count, &prepared);
}

pub(super) async fn assert_recovery_unknown(receipt_mode: RecoveryReceiptMode, pending_nonce: u64) {
    let (_fixture, prepared) = prepared_deploy_fixture("eip1559").await;
    let (decision, submit_count) = recover_with_provider(
        receipt_mode,
        pending_nonce,
        RecoveryOccupancyMode::Unknown,
        &prepared,
    )
    .await;
    assert_unknown_submission(decision, &submit_count, &prepared);
}

pub(super) fn unexpected_evm_call<'a, T>(capability: &'static str) -> EvmCapabilityFuture<'a, T> {
    Box::pin(async move { panic!("unexpected test EVM capability call: {capability}") })
}

pub(super) fn unexpected_signing_call<'a>() -> mfm_signing::SigningFuture<'a> {
    Box::pin(async { panic!("unexpected test signing provider call") })
}

impl EvmChainIdentityProvider for TestEvmProviders {
    fn chain_identity<'a>(
        &'a self,
        _request: &'a EvmChainIdentityRequest,
    ) -> EvmCapabilityFuture<'a, EvmChainIdentityResponse> {
        let evidence = self.evidence.clone();
        let chain_id = evidence.expected_chain_id;
        match self.mode {
            TestEvmProviderMode::Preparation
            | TestEvmProviderMode::Recovery { .. }
            | TestEvmProviderMode::SubmitHashMismatch { .. } => Box::pin(async move {
                Ok(EvmChainIdentityResponse {
                    evidence,
                    chain_id,
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
            TestEvmProviderMode::Finality => {
                let evidence = self.evidence.clone();
                Box::pin(async move {
                    Ok(EvmBlockReadResponse {
                        evidence,
                        block_number: 64,
                        block_hash: B256::from([0x64; 32]),
                    })
                })
            }
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
            TestEvmProviderMode::Preparation => {
                let evidence = self.evidence.clone();
                Box::pin(async move { Ok(EvmNonceReadResponse { evidence, nonce: 7 }) })
            }
            TestEvmProviderMode::Recovery { pending_nonce, .. } => {
                assert_eq!(request.block(), &EvmBlockSelector::Pending);
                let evidence = self.evidence.clone();
                Box::pin(async move {
                    Ok(EvmNonceReadResponse {
                        evidence,
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
            TestEvmProviderMode::Preparation => {
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
            TestEvmProviderMode::Preparation => {
                let evidence = self.evidence.clone();
                Box::pin(async move {
                    Ok(EvmGasEstimateResponse {
                        evidence,
                        gas_limit: 21_000,
                    })
                })
            }
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
                let transaction_hash = request.signed_payload().transaction_hash();
                let evidence = self.evidence.clone();
                Box::pin(async move {
                    *submit_count.lock().expect("submit count") += 1;
                    Ok(mfm_evm_capabilities::EvmTransactionSubmitResponse {
                        evidence,
                        transaction_hash,
                    })
                })
            }
            TestEvmProviderMode::SubmitHashMismatch { returned_hash } => {
                let submit_count = Arc::clone(&self.submit_count);
                let evidence = self.evidence.clone();
                Box::pin(async move {
                    *submit_count.lock().expect("submit count") += 1;
                    Ok(mfm_evm_capabilities::EvmTransactionSubmitResponse {
                        evidence,
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
                let transaction_hash = request.transaction_hash();
                let evidence = self.evidence.clone();
                Box::pin(async move {
                    match receipt_mode {
                        RecoveryReceiptMode::Landed => Ok(EvmReceiptReadResponse {
                            evidence,
                            transaction_hash,
                            block_number: 42,
                            status: true,
                        }),
                        RecoveryReceiptMode::Pending => Err(EvmCapabilityError::ReceiptPending),
                        RecoveryReceiptMode::ProviderFailure => Err(test_evm_provider_failure()),
                    }
                })
            }
            TestEvmProviderMode::ReceiptFailure => {
                let reads = Arc::clone(&self.receipt_failure_reads);
                Box::pin(async move {
                    let mut reads = reads.lock().map_err(|_| test_evm_provider_failure())?;
                    *reads += 1;
                    Err(test_evm_provider_failure())
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
                let nonce = request.nonce();
                let evidence = self.evidence.clone();
                Box::pin(async move {
                    match occupancy_mode {
                        RecoveryOccupancyMode::Unknown => {
                            Ok(mfm_evm_capabilities::EvmNonceOccupancyReadResponse {
                                evidence,
                                outcome: EvmNonceOccupancy::Unknown,
                            })
                        }
                        RecoveryOccupancyMode::Occupied { transaction_hash } => {
                            Ok(mfm_evm_capabilities::EvmNonceOccupancyReadResponse {
                                evidence,
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

impl EvmCodeReadProvider for TestEvmProviders {
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

pub(super) fn adapter(providers: &TestEvmProviders) -> EvmContractLifecycleAdapter<'_> {
    EvmContractLifecycleAdapter::new(
        EvmContractMutationProviders::from_evm_and_signer(providers, providers),
        EvmContractReadProviders::from_provider(providers),
    )
}
