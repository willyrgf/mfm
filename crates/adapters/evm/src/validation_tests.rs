use super::*;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use alloy_primitives::{keccak256, Address, Bytes, B256, U256};
use mfm_evm_capabilities::{
    EvmBlock, EvmBlockSelector, EvmCall, EvmCode, EvmSessionEvidence, EvmSessionFuture,
    ProviderDiagnosticCode,
};
use mfm_program::{StateSpec, ValidatedConfig};
use mfm_states_evm::{
    EvmContractCallCheck, EvmContractValidationConfig, EvmContractValidationTarget,
    EVM_CONTRACT_CODE_MAX_BYTES, EVM_CONTRACT_VALIDATION_MAX_TOTAL_RETURN_BYTES,
};

struct MissingArtifacts;

impl store::RetainedArtifactReadProvider for MissingArtifacts {
    fn read_retained_artifact<'a>(
        &'a self,
        requirement: &'a store::EventArtifactRequirement,
    ) -> store::RetainedArtifactReadFuture<'a> {
        let artifact_id = requirement.artifact_id.clone();
        Box::pin(async move { Err(store::StoreError::MissingArtifact { artifact_id }) })
    }
}

struct CountingReadSession {
    evidence: EvmSessionEvidence,
    uses: AtomicUsize,
}

impl CountingReadSession {
    fn with_chain_id(chain_id: u64) -> Self {
        let binding = EvmNetworkBinding::new(
            mfm_ids::LocalPublicId::new("ethereum-mainnet").expect("network id"),
            chain_id,
        )
        .expect("network binding");
        Self {
            evidence: EvmSessionEvidence::new(
                &binding,
                mfm_ids::LocalPublicId::new("primary").expect("source ref"),
                mfm_ids::LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID)
                    .expect("implementation id"),
            ),
            uses: AtomicUsize::new(0),
        }
    }

    fn used<'a, T: Send + 'a>(&'a self) -> EvmSessionFuture<'a, T> {
        self.uses.fetch_add(1, Ordering::SeqCst);
        Box::pin(std::future::ready(Err(
            EvmCapabilityError::provider_failure(mfm_evm_capabilities::evm_diagnostic(
                ProviderDiagnosticCode::ProviderConfigurationInvalid,
            )),
        )))
    }
}

impl EvmReadSession for CountingReadSession {
    fn evidence(&self) -> &EvmSessionEvidence {
        &self.evidence
    }

    fn read_block<'a>(&'a self, _selector: &'a EvmBlockSelector) -> EvmSessionFuture<'a, EvmBlock> {
        self.used()
    }

    fn read_balance<'a>(
        &'a self,
        _account: Address,
        _block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, U256> {
        self.used()
    }

    fn read_code<'a>(
        &'a self,
        _address: Address,
        _block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmCode> {
        self.used()
    }

    fn call<'a>(&'a self, _request: &'a EvmCall) -> EvmSessionFuture<'a, Bytes> {
        self.used()
    }
}

#[tokio::test]
async fn wrong_session_binding_fails_before_using_validation_authority() {
    let requested = EvmNetworkBinding::new(
        mfm_ids::LocalPublicId::new("ethereum-mainnet").expect("network id"),
        1,
    )
    .expect("requested binding");
    let session = Arc::new(CountingReadSession::with_chain_id(2));
    let bound_session = Arc::clone(&session);
    let capabilities = EvmValidationRunnerCapabilities::new(
        Arc::new(MissingArtifacts),
        |_| Ok(()),
        move |_| {
            let session = Arc::clone(&bound_session);
            Box::pin(async move { Ok(session as Arc<dyn EvmReadSession>) })
        },
    );

    let error = match capabilities.bind(requested).await {
        Ok(_) => panic!("wrong session binding must fail"),
        Err(error) => error,
    };

    assert_eq!(
        error,
        EvmCapabilityError::InvalidRequest {
            reason: mfm_evm_capabilities::EvmInvalidRequest::SessionAuthorityMismatch,
        }
    );
    assert_eq!(session.uses.load(Ordering::SeqCst), 0);
}

#[test]
fn validation_provider_availability_blocks_while_contract_failures_terminalize() {
    let unavailable = EvmCapabilityError::provider_failure(mfm_evm_capabilities::evm_diagnostic(
        ProviderDiagnosticCode::TransportFailed,
    ));
    assert!(matches!(
        evm_read_runtime_error(unavailable),
        mfm_runtime::RuntimeError::Blocked(_)
    ));

    let malformed = EvmCapabilityError::provider_failure(mfm_evm_capabilities::evm_diagnostic(
        ProviderDiagnosticCode::ResponseInvalid,
    ));
    assert!(matches!(
        evm_read_runtime_error(malformed),
        mfm_runtime::RuntimeError::Failure(_)
    ));
}

struct ScriptedValidationSession {
    evidence: EvmSessionEvidence,
    code: EvmCode,
    responses: Mutex<VecDeque<Bytes>>,
    canonical_block: EvmBlock,
    code_reads: AtomicUsize,
    calls: AtomicUsize,
    block_reads: AtomicUsize,
}

impl ScriptedValidationSession {
    fn new(code: Bytes, responses: Vec<Bytes>) -> Self {
        let binding = EvmNetworkBinding::new(
            mfm_ids::LocalPublicId::new("ethereum-mainnet").expect("network id"),
            1,
        )
        .expect("network binding");
        Self {
            evidence: EvmSessionEvidence::new(
                &binding,
                mfm_ids::LocalPublicId::new("primary").expect("source ref"),
                mfm_ids::LocalPublicId::new(EVM_JSONRPC_SESSION_IMPLEMENTATION_ID)
                    .expect("implementation id"),
            ),
            code: EvmCode {
                hash: keccak256(&code),
                bytes: code,
            },
            responses: Mutex::new(responses.into()),
            canonical_block: EvmBlock {
                number: U256::from(100),
                hash: B256::from([0xaa; 32]),
            },
            code_reads: AtomicUsize::new(0),
            calls: AtomicUsize::new(0),
            block_reads: AtomicUsize::new(0),
        }
    }
}

impl EvmReadSession for ScriptedValidationSession {
    fn evidence(&self) -> &EvmSessionEvidence {
        &self.evidence
    }

    fn read_block<'a>(&'a self, _selector: &'a EvmBlockSelector) -> EvmSessionFuture<'a, EvmBlock> {
        self.block_reads.fetch_add(1, Ordering::SeqCst);
        Box::pin(std::future::ready(Ok(self.canonical_block.clone())))
    }

    fn read_balance<'a>(
        &'a self,
        _account: Address,
        _block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, U256> {
        Box::pin(async { panic!("validation must not read a balance") })
    }

    fn read_code<'a>(
        &'a self,
        _address: Address,
        _block: &'a EvmBlockSelector,
    ) -> EvmSessionFuture<'a, EvmCode> {
        self.code_reads.fetch_add(1, Ordering::SeqCst);
        Box::pin(std::future::ready(Ok(self.code.clone())))
    }

    fn call<'a>(&'a self, _request: &'a EvmCall) -> EvmSessionFuture<'a, Bytes> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let response = self
            .responses
            .lock()
            .expect("responses")
            .pop_front()
            .expect("scripted response");
        Box::pin(std::future::ready(Ok(response)))
    }
}

fn validation_plan(code: &[u8], expected_returns: &[&[u8]]) -> EvmContractValidationPlan {
    let calls = expected_returns
        .iter()
        .map(|expected_return| {
            EvmContractCallCheck::new(
                Address::from([0x22; 20]),
                Address::from([0x33; 20]),
                U256::ZERO,
                [],
                U256::from(50_000),
                Vec::new(),
                expected_return,
            )
            .expect("call check")
        })
        .collect();
    let config = EvmContractValidationConfig::new("ethereum-mainnet", 1, keccak256(code), calls)
        .expect("validation config");
    let state = <ValidateEvmContractState as StateSpec>::new(
        ValidatedConfig::new(config).expect("validated config"),
    )
    .expect("validation state");
    let target: EvmContractValidationTarget = serde_json::from_value(serde_json::json!({
        "address": format!("{:#x}", Address::from([0x11; 20])),
        "anchor": {
            "number": "100",
            "hash": format!("{:#x}", B256::from([0xaa; 32])),
        },
    }))
    .expect("validation target");
    state.plan(&target).expect("validation plan")
}

#[tokio::test]
async fn oversized_code_prevents_contract_calls_and_canonicality_read() {
    let code = Bytes::from(vec![0x60; EVM_CONTRACT_CODE_MAX_BYTES + 1]);
    let plan = validation_plan(&code, &[&[0x01], &[0x02]]);
    let session = Arc::new(ScriptedValidationSession::new(
        code,
        vec![Bytes::from_static(&[0x01]), Bytes::from_static(&[0x02])],
    ));

    collect_contract_validation_evidence(&plan, session.clone() as Arc<dyn EvmReadSession>)
        .await
        .expect_err("oversized code");

    assert_eq!(session.code_reads.load(Ordering::SeqCst), 1);
    assert_eq!(session.calls.load(Ordering::SeqCst), 0);
    assert_eq!(session.block_reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn hostile_call_result_is_consumed_once_before_execution_stops() {
    let code = Bytes::from_static(&[0x60, 0x00]);
    let plan = validation_plan(&code, &[&[0x01], &[0x02]]);
    let near_transport_limit = Bytes::from(vec![0x7f; (1024 * 1024 - 128) / 2]);
    let session = Arc::new(ScriptedValidationSession::new(
        code,
        vec![near_transport_limit, Bytes::from_static(&[0x02])],
    ));

    collect_contract_validation_evidence(&plan, session.clone() as Arc<dyn EvmReadSession>)
        .await
        .expect_err("oversized call result");

    assert_eq!(session.code_reads.load(Ordering::SeqCst), 1);
    assert_eq!(session.calls.load(Ordering::SeqCst), 1);
    assert_eq!(session.block_reads.load(Ordering::SeqCst), 0);
    assert_eq!(
        session.responses.lock().expect("responses").len(),
        1,
        "the later response must never be read"
    );
}

#[tokio::test]
async fn forged_aggregate_crossing_plan_is_rejected_before_rpc() {
    let code = Bytes::from_static(&[0x60, 0x00]);
    let valid_plan = validation_plan(&code, &[]);
    let maximum = vec![0x5a; mfm_evm_capabilities::EVM_CALL_MAX_RESPONSE_BYTES];
    let mut calls = (0..(EVM_CONTRACT_VALIDATION_MAX_TOTAL_RETURN_BYTES
        / mfm_evm_capabilities::EVM_CALL_MAX_RESPONSE_BYTES))
        .map(|_| {
            EvmContractCallCheck::new(
                Address::from([0x22; 20]),
                Address::from([0x33; 20]),
                U256::ZERO,
                [],
                U256::from(50_000),
                Vec::new(),
                &maximum,
            )
            .expect("call check")
        })
        .collect::<Vec<_>>();
    calls.push(
        EvmContractCallCheck::new(
            Address::from([0x22; 20]),
            Address::from([0x33; 20]),
            U256::ZERO,
            [],
            U256::from(50_000),
            Vec::new(),
            [0x01],
        )
        .expect("plus-one call"),
    );
    let mut plan_value = serde_json::to_value(valid_plan).expect("plan JSON");
    plan_value["calls"] = serde_json::to_value(calls).expect("calls JSON");
    let forged_plan: EvmContractValidationPlan =
        serde_json::from_value(plan_value).expect("forged plan");
    let session = Arc::new(ScriptedValidationSession::new(code, Vec::new()));

    collect_contract_validation_evidence(&forged_plan, session.clone() as Arc<dyn EvmReadSession>)
        .await
        .expect_err("aggregate-crossing plan");

    assert_eq!(session.code_reads.load(Ordering::SeqCst), 0);
    assert_eq!(session.calls.load(Ordering::SeqCst), 0);
    assert_eq!(session.block_reads.load(Ordering::SeqCst), 0);
}
