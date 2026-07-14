use super::*;

fn test_evm_provider_failure() -> EvmCapabilityError {
    EvmCapabilityError::provider_failure(mfm_evm_capabilities::evm_diagnostic(
        mfm_capabilities::ProviderDiagnosticCode::TransportFailed,
    ))
}

#[derive(Clone)]
pub(super) struct TestRuntimeFactory {
    artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
    read_runtime: EvmContractReadRuntime,
    runtime: EvmContractRuntime,
}

impl TestRuntimeFactory {
    pub(super) fn new(artifacts: Arc<dyn store::RetainedArtifactReadProvider>) -> Self {
        Self::with_options(artifacts, false, None)
    }

    pub(super) fn with_chain_identity_delay(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        delay: Duration,
    ) -> Self {
        Self::with_options(artifacts, false, Some(delay))
    }

    fn with_options(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
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

    pub(super) fn with_signer(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
        signer: Arc<dyn SigningProvider>,
        fail_repeated_prepare_reads: bool,
    ) -> Self {
        Self::with_runtime(artifacts, signer, true, fail_repeated_prepare_reads, None)
    }

    fn with_runtime(
        artifacts: Arc<dyn store::RetainedArtifactReadProvider>,
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
    fn artifacts(&self) -> &dyn store::RetainedArtifactReadProvider {
        self.artifacts.as_ref()
    }

    fn validate_runtime_for(
        &self,
        binding: &EvmNetworkBinding,
        _signer_ref: Option<&SignerRef>,
    ) -> mfm_runtime::Result<()> {
        assert_eq!(binding.network_id().as_str(), "reth-dev");
        assert_eq!(binding.expected_chain_id(), 31337);
        Ok(())
    }

    fn read_runtime_for(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_runtime::Result<EvmContractReadRuntime> {
        assert_eq!(binding.network_id().as_str(), "reth-dev");
        assert_eq!(binding.expected_chain_id(), 31337);
        Ok(self.read_runtime.clone())
    }

    fn runtime_for(&self, binding: EvmNetworkBinding) -> mfm_runtime::Result<EvmContractRuntime> {
        assert_eq!(binding.network_id().as_str(), "reth-dev");
        assert_eq!(binding.expected_chain_id(), 31337);
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
    fn evidence(&self) -> RedactedEvmSourceEvidence {
        let network_id = EvmNetworkId::new("reth-dev").expect("test network");
        let expected_chain_id = 31337;
        RedactedEvmSourceEvidence {
            network_id,
            expected_chain_id,
            observed_chain_id: expected_chain_id,
            source_ref: self.source_ref.clone(),
            policy_id: self.policy_id.clone(),
        }
    }

    fn record_prepare_read(&self, kind: TestPrepareReadKind) -> Result<(), EvmCapabilityError> {
        if !self.fail_repeated_prepare_reads {
            return Ok(());
        }
        let mut reads = self.reads.lock().map_err(|_| test_evm_provider_failure())?;
        let count = match kind {
            TestPrepareReadKind::Nonce => &mut reads.nonce,
            TestPrepareReadKind::Fee => &mut reads.fee,
            TestPrepareReadKind::Gas => &mut reads.gas,
        };
        *count += 1;
        if *count > 1 {
            Err(test_evm_provider_failure())
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
        _request: &'a EvmChainIdentityRequest,
    ) -> EvmCapabilityFuture<'a, EvmChainIdentityResponse> {
        let evidence = self.evidence();
        let chain_id = evidence.expected_chain_id;
        Box::pin(async move {
            if let Some(delay) = self.chain_identity_delay {
                tokio::time::sleep(delay).await;
            }
            Ok(EvmChainIdentityResponse {
                evidence,
                chain_id,
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
        assert_eq!(request.block(), &EvmBlockSelector::Latest);
        let evidence = self.evidence();
        Box::pin(async move {
            Ok(EvmBlockReadResponse {
                evidence,
                block_number: 64,
                block_hash: Default::default(),
            })
        })
    }
}

impl EvmNonceReadProvider for TestEvmProvider {
    fn read_nonce<'a>(
        &'a self,
        _request: &'a EvmNonceReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmNonceReadResponse> {
        if !self.mutation {
            return failed_evm();
        }
        let evidence = self.evidence();
        Box::pin(async move {
            self.record_prepare_read(TestPrepareReadKind::Nonce)?;
            Ok(EvmNonceReadResponse { evidence, nonce: 7 })
        })
    }
}

impl EvmFeeReadProvider for TestEvmProvider {
    fn read_fee<'a>(
        &'a self,
        _request: &'a EvmFeeReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmFeeReadResponse> {
        if !self.mutation {
            return failed_evm();
        }
        let evidence = self.evidence();
        Box::pin(async move {
            self.record_prepare_read(TestPrepareReadKind::Fee)?;
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

impl EvmGasEstimateProvider for TestEvmProvider {
    fn estimate_gas<'a>(
        &'a self,
        _request: &'a EvmGasEstimateRequest,
    ) -> EvmCapabilityFuture<'a, EvmGasEstimateResponse> {
        if !self.mutation {
            return failed_evm();
        }
        let evidence = self.evidence();
        Box::pin(async move {
            self.record_prepare_read(TestPrepareReadKind::Gas)?;
            Ok(EvmGasEstimateResponse {
                evidence,
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
        let evidence = self.evidence();
        Box::pin(async move {
            Ok(EvmTransactionSubmitResponse {
                evidence,
                transaction_hash: request.signed_payload().transaction_hash(),
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
                let mut reads = reads.lock().map_err(|_| test_evm_provider_failure())?;
                reads.receipt += 1;
                Err(test_evm_provider_failure())
            });
        }
        let evidence = self.evidence();
        Box::pin(async move {
            let mut reads = self.reads.lock().map_err(|_| test_evm_provider_failure())?;
            reads.receipt += 1;
            Ok(EvmReceiptReadResponse {
                evidence,
                transaction_hash: request.transaction_hash(),
                block_number: 42,
                status: true,
            })
        })
    }
}

impl EvmNonceOccupancyReadProvider for TestEvmProvider {
    fn read_nonce_occupancy<'a>(
        &'a self,
        _request: &'a EvmNonceOccupancyReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmNonceOccupancyReadResponse> {
        let evidence = self.evidence();
        Box::pin(async move {
            Ok(EvmNonceOccupancyReadResponse {
                evidence,
                outcome: EvmNonceOccupancy::Unknown,
            })
        })
    }
}

impl EvmCallReadProvider for TestEvmProvider {
    fn read_call<'a>(
        &'a self,
        _request: &'a EvmCallReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmCallReadResponse> {
        let evidence = self.evidence();
        Box::pin(async move {
            let mut return_data = vec![0_u8; 32];
            return_data[31] = 1;
            Ok(EvmCallReadResponse {
                evidence,
                return_data,
            })
        })
    }
}

impl EvmCodeReadProvider for TestEvmProvider {
    fn read_code<'a>(
        &'a self,
        _request: &'a EvmCodeReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmCodeReadResponse> {
        let evidence = self.evidence();
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

impl EvmLogsReadProvider for TestEvmProvider {
    fn read_logs<'a>(
        &'a self,
        request: &'a EvmLogsReadRequest,
    ) -> EvmCapabilityFuture<'a, EvmLogsReadResponse> {
        let evidence = self.evidence();
        Box::pin(async move {
            Ok(EvmLogsReadResponse {
                evidence,
                logs: vec![EvmLogEntry {
                    address: request.address().expect("validation log address"),
                    topics: request.topics().to_vec(),
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
    Box::pin(async { Err(test_evm_provider_failure()) })
}

pub(super) async fn wait_for_live_execution_claim<S>(
    store: &S,
    execution_scope: &store::ExecutionClaimScope,
) -> store::AdmissionLease
where
    S: ExecutionClaimStore,
    S::Error: std::fmt::Debug,
{
    for _ in 0..100 {
        match store
            .execution_claim_status(execution_scope)
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

pub(super) async fn execution_scope_for_run(
    store: &ContractRunStore,
    run_id: &mfm_ids::RunId,
) -> store::ExecutionClaimScope {
    let stream = store.load_run_stream(run_id).await.expect("run stream");
    stream
        .iter()
        .find_map(|event| match event.payload() {
            events::KernelEventPayload::RunAdmitted(payload) => Some(
                store::ExecutionClaimScope::from_run_identity_material(&payload.identity_material),
            ),
            _ => None,
        })
        .expect("RunAdmitted")
}

pub(super) struct TestSignerRuntime {
    pub(super) provider: LocalTestSigner,
    pub(super) address: String,
}

pub(super) fn test_contract_signer() -> TestSignerRuntime {
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

pub(super) struct LocalTestSigner {
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
