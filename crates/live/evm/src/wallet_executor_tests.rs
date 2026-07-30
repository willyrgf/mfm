use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use alloy_primitives::{address, b256, keccak256, Address, PrimitiveSignature, B256, U256};
use k256::ecdsa::SigningKey;
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes, RecoverabilityContract};
use mfm_evm::{
    evm_submit_transaction_leaf_expansion, evm_submit_transaction_value_contracts,
    evm_wallet_assurance_policy_ref, evm_wallet_finality_policy_ref, evm_wallet_nonce_policy_ref,
    EvmSubmitTransactionRequest, EvmTransactionOutcome, EvmTransactionTarget,
    EvmWalletAccessListEntry, EvmWalletAttemptResult, EvmWalletBroadcastStatus,
    EvmWalletConvergencePlan, EvmWalletFeeCandidate, EvmWalletInitialNonceDescriptor,
    EvmWalletPolicy, EvmWalletReference, EvmWalletReplacementPolicy, EvmWalletTransactionAction,
    EvmWalletTransactionCandidate, EvmWalletTransactionTemplate,
    EVM_SUBMIT_TRANSACTION_OPERATION_ID, EVM_WALLET_BROADCAST_OPERATION_ID,
    EVM_WALLET_SIGNED_TRANSACTION_MAX_BYTES, EVM_WALLET_SUCCEEDED_TERMINAL_OUTCOME,
};
use mfm_executor::{
    AttemptId, CommittedEffectRequest, DeliveryAttemptOutcome, DeliveryAuditFrontierRef,
    EffectExecutorOutcome, EffectExecutorOutcomeParts, Ensure, EvidenceBounds,
    ExecuteTargetOutcome, ExecutorBinding, ExecutorContractDescriptor, ExecutorDeployment,
    ExecutorError, ExecutorRetainedClosureContract, KeyedExecutorLedger, MemoryExecutorStore,
    ReferenceFailureCode, ReferenceTerminalProof, ResourceOwnership, SchemaQualifiedCanonicalValue,
    TerminalTombstone, VerifiedEnsureResult, VerifiedExecutorBinding,
};
use mfm_ids::{
    ContentRef, DigestAlgorithm, NodeId, RunId, SchemaId, SemanticTypeId, StableId, StoreScopeId,
    TenantScopeId,
};
use mfm_program::{boundary_content_ref, decode_boundary, encode_boundary};
use mfm_signing::{
    GenerationGuardedDeterministicSigningProvider,
    GenerationGuardedDeterministicSigningProviderBinder, PublicSigningIdentity, SignatureBytes,
    SignerRef, SigningAlgorithmId, SigningError, SigningFuture, SigningProfileId,
    SigningProviderError, SigningResult, VerifiedGenerationGuardedSignerBinding,
    SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID, SECP256K1_RFC6979_LOW_S_PROFILE_ID,
};
use mfm_values::{MfmValue, RetainedValueContract};
use serde::Deserialize;
use serde_json::value::RawValue;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use zeroize::Zeroizing;

use crate::transport::{
    EvmJsonRpcTransport, EvmRoutingCatalogBuilder, EvmRpcAuthorization, EvmRpcEndpoint,
};
use crate::{
    evm_already_known_classifier_ref, evm_wallet_target_callback_surface_ref,
    wallet_rpc::{EvmWalletJsonRpcTarget, EvmWalletTargetEntryDescriptor},
    EvmWalletExecutor, EvmWalletRequestQualification,
};

const RECIPIENT: Address = address!("2222222222222222222222222222222222222222");
const BLOCK_A: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BLOCK_B: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const WRONG_BLOCK: &str = "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const FINALIZED_BLOCK: &str = "0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const WALLET_AUTHORIZATION_PREFIX: &str = "Bearer wallet-fixture-";
const MAX_TEST_HTTP_HEADER_BYTES: usize = 64 * 1024;
const MAX_TEST_HTTP_BODY_BYTES: usize = 74 + 2 * EVM_WALLET_SIGNED_TRANSACTION_MAX_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scenario {
    Succeeded,
    Reverted,
    ResponseLost,
    AlreadyKnown,
    Replacement,
    Reorganization,
}

#[derive(Clone)]
struct LoopbackRpc {
    inner: Arc<LoopbackRpcInner>,
}

struct LoopbackRpcInner {
    state: Arc<Mutex<LoopbackRpcState>>,
    shutdown: Mutex<Option<oneshot::Sender<()>>>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl Drop for LoopbackRpcInner {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.get_mut().expect("shutdown mutex").take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.get_mut().expect("server task mutex").take() {
            task.abort();
        }
    }
}

struct LoopbackRpcState {
    scenario: Scenario,
    calls: Vec<&'static str>,
    broadcast_hashes: Vec<String>,
    candidate_ordinals: BTreeMap<String, usize>,
    response_lost: bool,
    reorged: bool,
}

impl LoopbackRpc {
    fn start(
        listener: TcpListener,
        request: EvmSubmitTransactionRequest,
        scenario: Scenario,
        expected_authorization: Zeroizing<String>,
    ) -> Self {
        let state = Arc::new(Mutex::new(LoopbackRpcState {
            scenario,
            calls: Vec::new(),
            broadcast_hashes: Vec::new(),
            candidate_ordinals: BTreeMap::new(),
            response_lost: false,
            reorged: false,
        }));
        let captured = Arc::clone(&state);
        let (shutdown, mut shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    biased;
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => accepted,
                };
                let Ok((mut socket, _)) = accepted else {
                    break;
                };
                let capture = read_http_request(&mut socket).await;
                assert!(
                    header_value(capture.headers(), "authorization")
                        .is_some_and(|value| value == expected_authorization.as_str()),
                    "loopback authorization did not match"
                );
                match loopback_response(&request, &captured, capture.body()) {
                    LoopbackReply::Disconnect => {}
                    LoopbackReply::Envelope(envelope) => {
                        let body = serde_json::to_vec(&envelope).expect("loopback response JSON");
                        let head = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            body.len()
                        );
                        socket
                            .write_all(head.as_bytes())
                            .await
                            .expect("write loopback response head");
                        socket
                            .write_all(&body)
                            .await
                            .expect("write loopback response body");
                    }
                }
            }
        });
        Self {
            inner: Arc::new(LoopbackRpcInner {
                state,
                shutdown: Mutex::new(Some(shutdown)),
                task: Mutex::new(Some(task)),
            }),
        }
    }

    fn call_count(&self) -> usize {
        self.inner
            .state
            .lock()
            .expect("loopback RPC mutex")
            .calls
            .len()
    }

    fn operation_count(&self, method: &str) -> usize {
        self.inner
            .state
            .lock()
            .expect("loopback RPC mutex")
            .calls
            .iter()
            .filter(|candidate| **candidate == method)
            .count()
    }

    fn calls(&self) -> Vec<&'static str> {
        self.inner
            .state
            .lock()
            .expect("loopback RPC mutex")
            .calls
            .clone()
    }

    fn broadcast_hashes(&self) -> Vec<String> {
        self.inner
            .state
            .lock()
            .expect("loopback RPC mutex")
            .broadcast_hashes
            .clone()
    }
}

enum LoopbackReply {
    Envelope(Value),
    Disconnect,
}

fn loopback_response(
    request: &EvmSubmitTransactionRequest,
    state: &Mutex<LoopbackRpcState>,
    rpc_request: &[u8],
) -> LoopbackReply {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct BorrowedRequest<'a> {
        jsonrpc: &'a str,
        id: u64,
        method: &'a str,
        #[serde(borrow)]
        params: &'a RawValue,
    }

    let mut deserializer = serde_json::Deserializer::from_slice(rpc_request);
    let rpc_request =
        BorrowedRequest::deserialize(&mut deserializer).expect("borrowed loopback request");
    deserializer.end().expect("exact loopback request");
    assert_eq!(rpc_request.jsonrpc, "2.0");
    assert_eq!(rpc_request.id, 1);
    let method = match rpc_request.method {
        "eth_sendRawTransaction" => "eth_sendRawTransaction",
        "eth_getTransactionByHash" => "eth_getTransactionByHash",
        "eth_getTransactionReceipt" => "eth_getTransactionReceipt",
        "eth_getBlockByNumber" => "eth_getBlockByNumber",
        _ => return invalid_loopback_request(),
    };
    let Some((parameter, parameter_suffix)) = first_string_parameter(rpc_request.params) else {
        return invalid_loopback_request();
    };
    let mut state = state.lock().expect("loopback RPC mutex");
    state.calls.push(method);
    let result = match method {
        "eth_sendRawTransaction" => {
            let Some(signed_hex) = parameter.strip_prefix("0x") else {
                return invalid_loopback_request();
            };
            if parameter_suffix != b"]"
                || !signed_hex.len().is_multiple_of(2)
                || signed_hex.len() / 2 > EVM_WALLET_SIGNED_TRANSACTION_MAX_BYTES
            {
                return invalid_loopback_request();
            }
            let mut signed = Zeroizing::new(vec![0_u8; signed_hex.len() / 2]);
            if hex::decode_to_slice(signed_hex, &mut signed).is_err() {
                return invalid_loopback_request();
            }
            let transaction_hash = format!("{:#x}", keccak256(signed.as_slice()));
            drop(signed);
            let next_ordinal = state.candidate_ordinals.len();
            state
                .candidate_ordinals
                .entry(transaction_hash.clone())
                .or_insert(next_ordinal);
            state.broadcast_hashes.push(transaction_hash.clone());
            if state.scenario == Scenario::ResponseLost && !state.response_lost {
                state.response_lost = true;
                return LoopbackReply::Disconnect;
            }
            if state.scenario == Scenario::AlreadyKnown && state.broadcast_hashes.len() == 1 {
                return LoopbackReply::Envelope(json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "error": {
                        "code": -32_000,
                        "message": "already known",
                    },
                }));
            }
            Value::String(transaction_hash)
        }
        "eth_getTransactionByHash" => {
            if parameter_suffix != b"]" {
                return invalid_loopback_request();
            }
            let hash = parameter;
            let Some(ordinal) = candidate_ordinal(&state, hash) else {
                return invalid_loopback_request();
            };
            if state.scenario == Scenario::Replacement && ordinal == 0 {
                Value::Null
            } else {
                let (block_number, block_hash) = active_block(&state);
                let Ok(transaction) =
                    transaction_json(request, hash, ordinal, block_number, block_hash)
                else {
                    return invalid_loopback_request();
                };
                transaction
            }
        }
        "eth_getTransactionReceipt" => {
            if parameter_suffix != b"]" {
                return invalid_loopback_request();
            }
            let hash = parameter;
            let Some(ordinal) = candidate_ordinal(&state, hash) else {
                return invalid_loopback_request();
            };
            if state.scenario == Scenario::Replacement && ordinal == 0 {
                Value::Null
            } else {
                let (block_number, block_hash) = active_block(&state);
                let Ok(receipt) = receipt_json(
                    request,
                    hash,
                    block_number,
                    block_hash,
                    state.scenario == Scenario::Reverted,
                ) else {
                    return invalid_loopback_request();
                };
                receipt
            }
        }
        "eth_getBlockByNumber" => {
            if parameter_suffix != b",false]" {
                return invalid_loopback_request();
            }
            let selector = parameter;
            if selector == "finalized" {
                json!({
                    "hash": FINALIZED_BLOCK,
                    "number": "0x6e",
                })
            } else {
                let (number, hash) = active_block(&state);
                if selector != number {
                    return invalid_loopback_request();
                }
                if state.scenario == Scenario::Reorganization && !state.reorged {
                    state.reorged = true;
                    json!({
                        "hash": WRONG_BLOCK,
                        "number": number,
                    })
                } else {
                    json!({
                        "hash": hash,
                        "number": number,
                    })
                }
            }
        }
        _ => return invalid_loopback_request(),
    };
    LoopbackReply::Envelope(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": result,
    }))
}

fn invalid_loopback_request() -> LoopbackReply {
    LoopbackReply::Envelope(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "error": {
            "code": -32602,
            "message": "invalid params",
        },
    }))
}

struct CapturedHttpRequest {
    bytes: Zeroizing<Vec<u8>>,
    header_end: usize,
    body_end: usize,
}

impl CapturedHttpRequest {
    fn headers(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.header_end]).expect("request headers")
    }

    fn body(&self) -> &[u8] {
        &self.bytes[self.header_end..self.body_end]
    }
}

async fn read_http_request(socket: &mut TcpStream) -> CapturedHttpRequest {
    let mut bytes = Zeroizing::new(Vec::with_capacity(
        MAX_TEST_HTTP_HEADER_BYTES + MAX_TEST_HTTP_BODY_BYTES,
    ));
    let header_end = loop {
        let mut chunk = Zeroizing::new([0_u8; 1024]);
        let count = socket.read(&mut *chunk).await.expect("read request");
        assert!(count > 0, "request ended before headers");
        bytes.extend_from_slice(&chunk[..count]);
        assert!(
            bytes.len() <= MAX_TEST_HTTP_HEADER_BYTES,
            "request headers exceeded fixture bound"
        );
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end]).expect("request headers");
    let length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().expect("content length"))
        })
        .expect("content length header");
    assert!(
        length <= MAX_TEST_HTTP_BODY_BYTES,
        "request body exceeded fixture bound"
    );
    let body_end = header_end.checked_add(length).expect("request body end");
    while bytes.len() < body_end {
        let mut chunk = Zeroizing::new([0_u8; 1024]);
        let count = socket.read(&mut *chunk).await.expect("read request body");
        assert!(count > 0, "request body ended early");
        bytes.extend_from_slice(&chunk[..count]);
        assert!(
            bytes.len() <= body_end,
            "request exceeded declared body length"
        );
    }
    CapturedHttpRequest {
        bytes,
        header_end,
        body_end,
    }
}

fn header_value<'a>(headers: &'a str, expected_name: &str) -> Option<&'a str> {
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case(expected_name)
            .then_some(value.trim())
    })
}

fn first_string_parameter(params: &RawValue) -> Option<(&str, &[u8])> {
    let remainder = params.get().as_bytes().strip_prefix(b"[\"")?;
    let string_end = remainder.iter().position(|byte| *byte == b'"')?;
    let encoded = &remainder[..string_end];
    if encoded.iter().any(|byte| *byte == b'\\' || *byte < 0x20) {
        return None;
    }
    let value = std::str::from_utf8(encoded).ok()?;
    Some((value, &remainder[string_end + 1..]))
}

fn candidate_ordinal(state: &LoopbackRpcState, transaction_hash: &str) -> Option<usize> {
    state.candidate_ordinals.get(transaction_hash).copied()
}

fn active_block(state: &LoopbackRpcState) -> (&'static str, &'static str) {
    if state.scenario == Scenario::Reorganization && state.reorged {
        ("0x65", BLOCK_B)
    } else {
        ("0x64", BLOCK_A)
    }
}

fn transaction_json(
    request: &EvmSubmitTransactionRequest,
    transaction_hash: &str,
    fee_ordinal: usize,
    block_number: &str,
    block_hash: &str,
) -> Result<Value, ()> {
    let fee = request
        .policy()
        .replacement()
        .fee_candidates()
        .get(fee_ordinal)
        .ok_or(())?;
    let to = request
        .template()
        .action()
        .call_destination()
        .map_err(|_| ())?
        .ok_or(())?;
    let access_list = request
        .template()
        .access_list()
        .iter()
        .map(|entry| {
            json!({
                "address": entry.address(),
                "storageKeys": entry.storage_keys(),
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "accessList": access_list,
        "blockHash": block_hash,
        "blockNumber": block_number,
        "chainId": quantity(U256::from(request.policy().chain_id())),
        "from": request.policy().sender(),
        "gas": quantity(request.template().gas_limit_quantity().map_err(|_| ())?),
        "hash": transaction_hash,
        "input": request.template().input(),
        "maxFeePerGas": quantity(fee.max_fee_quantity().map_err(|_| ())?),
        "maxPriorityFeePerGas": quantity(fee.max_priority_fee_quantity().map_err(|_| ())?),
        "nonce": quantity(U256::from(request.policy().initial_nonce().map_err(|_| ())?)),
        "to": format!("{to:#x}"),
        "transactionIndex": "0x0",
        "type": "0x2",
        "value": quantity(request.template().value_quantity().map_err(|_| ())?),
    }))
}

fn receipt_json(
    request: &EvmSubmitTransactionRequest,
    transaction_hash: &str,
    block_number: &str,
    block_hash: &str,
    reverted: bool,
) -> Result<Value, ()> {
    let to = request
        .template()
        .action()
        .call_destination()
        .map_err(|_| ())?
        .ok_or(())?;
    Ok(json!({
        "blockHash": block_hash,
        "blockNumber": block_number,
        "contractAddress": null,
        "cumulativeGasUsed": "0x5208",
        "from": request.policy().sender(),
        "gasUsed": "0x5208",
        "logs": [],
        "status": if reverted { "0x0" } else { "0x1" },
        "to": format!("{to:#x}"),
        "transactionHash": transaction_hash,
        "transactionIndex": "0x0",
        "type": "0x2",
    }))
}

fn quantity(value: U256) -> String {
    format!("0x{value:x}")
}

#[derive(Clone)]
struct TestSigner {
    binding: VerifiedGenerationGuardedSignerBinding,
    key: SigningKey,
    calls: Arc<AtomicUsize>,
    available: Arc<AtomicBool>,
}

impl GenerationGuardedDeterministicSigningProvider for TestSigner {
    fn binding(&self) -> &VerifiedGenerationGuardedSignerBinding {
        &self.binding
    }

    fn sign_guarded<'a>(
        &'a self,
        expected_generation_ref: &'a ContentRef,
        request: &'a mfm_signing::SigningRequest,
    ) -> SigningFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let result = if !self.available.load(Ordering::SeqCst) {
            Err(SigningError::Provider {
                reason: SigningProviderError::Failed,
            })
        } else if expected_generation_ref != self.binding.durable_generation_ref() {
            Err(SigningError::Provider {
                reason: SigningProviderError::GenerationMismatch,
            })
        } else {
            self.binding.verify_request(request).and_then(|()| {
                let (signature, recovery_id) = self
                    .key
                    .sign_prehash_recoverable(request.digest().as_ref())
                    .map_err(|_| SigningError::Provider {
                        reason: SigningProviderError::Failed,
                    })?;
                let signature = signature.to_bytes();
                let signature = PrimitiveSignature::from_scalars_and_parity(
                    B256::from_slice(&signature[..32]),
                    B256::from_slice(&signature[32..]),
                    recovery_id.is_y_odd(),
                )
                .normalized_s();
                SigningResult::for_request(
                    request,
                    self.binding.expected_public_identity().clone(),
                    SignatureBytes::new(signature.as_bytes().to_vec())?,
                )
            })
        };
        Box::pin(async move { result })
    }
}

#[derive(Clone)]
struct RequestInputs {
    tenant_scope_id: TenantScopeId,
    wallet_domain_ref: ContentRef,
    route_generation_ref: ContentRef,
    chain_id: u64,
    signer_binding_ref: ContentRef,
    nonce_policy_ref: ContentRef,
    initial_nonce: u64,
    initial_nonce_descriptor_ref: ContentRef,
    already_known_classifier_ref: ContentRef,
    finality_policy_ref: ContentRef,
    assurance_policy_ref: ContentRef,
    sender: Address,
    bounds: EvidenceBounds,
    max_attempt_result_bytes: u64,
}

struct Fixture {
    binding: VerifiedExecutorBinding,
    store: MemoryExecutorStore,
    executor: EvmWalletExecutor<MemoryExecutorStore>,
    rpc: LoopbackRpc,
    signer: GenerationGuardedDeterministicSigningProviderBinder,
    qualification: Arc<EvmWalletRequestQualification>,
    committed: CommittedEffectRequest<EvmSubmitTransactionRequest>,
    request_inputs: RequestInputs,
    signer_calls: Arc<AtomicUsize>,
    signer_available: Arc<AtomicBool>,
}

impl Fixture {
    async fn new(scenario: Scenario) -> Self {
        let bounds = EvidenceBounds::new(20, 64, 8 * 1024 * 1024, 256 * 1024, 2, 256 * 1024)
            .expect("bounds");
        Self::new_with_configuration(scenario, bounds, 128 * 1024).await
    }

    async fn new_with_bounds(scenario: Scenario, bounds: EvidenceBounds) -> Self {
        Self::new_with_configuration(scenario, bounds, 128 * 1024).await
    }

    async fn new_with_configuration(
        scenario: Scenario,
        bounds: EvidenceBounds,
        max_attempt_result_bytes: u64,
    ) -> Self {
        let key = SigningKey::from_slice(&[7_u8; 32]).expect("test signing key");
        let sender = signing_address(&key);
        let tenant_scope_id =
            TenantScopeId::new("mfm.tenant_scope.v1:00000000000000000000000000000041")
                .expect("tenant");
        let generation_ref = reviewed_ref("wallet-generation");
        let fence_ref = reviewed_ref("wallet-generation-fence");
        let wallet_domain_ref = reviewed_ref("wallet-domain");
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback RPC");
        let listener_address = listener.local_addr().expect("loopback address");
        let authorization = Zeroizing::new(format!(
            "{WALLET_AUTHORIZATION_PREFIX}{}",
            listener_address.port()
        ));
        let expected_authorization = authorization.clone();
        let endpoint = format!("http://{}", listener_address);
        let mut routing = EvmRoutingCatalogBuilder::new();
        let route_generation = routing
            .insert(
                "primary",
                "mfm.test.evm-wallet-source",
                1,
                StableId::new("mfm.test.evm-wallet-route-generation").expect("generation id"),
                EvmRpcEndpoint::new(endpoint).expect("endpoint"),
                Some(EvmRpcAuthorization::new(authorization).expect("wallet authorization")),
            )
            .expect("route generation");
        let qualification_transport =
            EvmJsonRpcTransport::new(routing.build().expect("routing catalog"))
                .expect("qualification transport");
        let object_evidence_ref = reviewed_ref("object-evidence");
        let value_contracts = evm_submit_transaction_value_contracts(object_evidence_ref.clone())
            .expect("wallet value contracts");
        let retained = ExecutorRetainedClosureContract::new(
            retained_contract(
                "ensure-result",
                "mfm.executor-ensure-result.v1",
                &object_evidence_ref,
            ),
            retained_contract(
                "delivery-audit",
                "mfm.executor-delivery-frontier.v2",
                &object_evidence_ref,
            ),
            retained_contract(
                "executor-frontier",
                "mfm.executor-delivery-frontier.v2",
                &object_evidence_ref,
            ),
            retained_contract(
                "terminal-evidence",
                "mfm.terminal-effect-evidence.v1",
                &object_evidence_ref,
            ),
            retained_contract(
                "terminal-tombstone",
                "mfm.executor-terminal-tombstone.v2",
                &object_evidence_ref,
            ),
            retained_contract(
                "terminal-proof",
                "mfm.executor-reference-terminal-proof.v2",
                &object_evidence_ref,
            ),
            value_contracts.attempt_result().clone(),
        )
        .expect("retained closure");
        let target_surface =
            evm_wallet_target_callback_surface_ref().expect("wallet target surface");
        let contract = ExecutorContractDescriptor::new(
            reviewed_ref("ensure-contract"),
            value_contracts.request().clone(),
            retained_contract("safe-failure", "mfm.safe-failure.v1", &object_evidence_ref),
            retained,
            mfm_evm::evm_safe_failure_contract_ref().expect("safe failure contract"),
            target_surface.clone(),
            bounds.clone(),
            Some(wallet_domain_ref.clone()),
            vec![evm_submit_transaction_leaf_expansion(target_surface)
                .expect("wallet leaf expansion")],
        )
        .expect("executor contract");
        let ownership = ResourceOwnership::new(
            reviewed_ref("wallet-coordination"),
            wallet_domain_ref.clone(),
            generation_ref.clone(),
            Some(fence_ref.clone()),
        )
        .expect("resource ownership");
        let deployment = ExecutorDeployment::new(
            reviewed_ref("executor-namespace"),
            generation_ref.clone(),
            tenant_scope_id.clone(),
            reviewed_ref("evidence-authority"),
            Some(ownership.reference().expect("ownership ref")),
        )
        .expect("deployment");
        let binding = ExecutorBinding::new(
            contract.reference().expect("contract ref"),
            reviewed_ref("wallet-implementation"),
            deployment.reference().expect("deployment ref"),
        )
        .expect("binding");
        let binding = VerifiedExecutorBinding::verify(
            binding,
            contract,
            deployment,
            Some(ownership),
            &tenant_scope_id,
        )
        .expect("verified binding");

        let signer_binding = VerifiedGenerationGuardedSignerBinding::verify(
            SignerRef::new("wallet").expect("signer ref"),
            "mfm.test.wallet-signer",
            SigningAlgorithmId::new(SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID)
                .expect("algorithm"),
            SigningProfileId::new(SECP256K1_RFC6979_LOW_S_PROFILE_ID).expect("profile"),
            PublicSigningIdentity::new(
                SigningAlgorithmId::new(SECP256K1_KECCAK256_RECOVERABLE_ALGORITHM_ID)
                    .expect("algorithm"),
                None,
                Some(format!("{sender:#x}")),
            )
            .expect("public identity"),
            generation_ref,
            fence_ref,
            reviewed_ref("direct-sign-exclusion"),
        )
        .expect("guarded signer binding");
        let signer_descriptor = signer_binding
            .public_descriptor()
            .expect("signer descriptor");
        let initial_nonce_descriptor = EvmWalletInitialNonceDescriptor::new(
            7,
            EvmWalletReference::from_content_ref(reviewed_ref("nonce-source-attestation")),
            EvmWalletReference::from_content_ref(wallet_domain_ref.clone()),
            1,
            sender,
            EvmWalletReference::from_content_ref(
                binding.deployment().durable_ledger_generation_ref().clone(),
            ),
        )
        .expect("initial nonce descriptor");
        let qualification = Arc::new(
            EvmWalletRequestQualification::qualify(
                &qualification_transport,
                route_generation.clone(),
                binding.clone(),
                &signer_binding,
                initial_nonce_descriptor.clone(),
                object_evidence_ref,
            )
            .expect("wallet request qualification"),
        );
        let request_inputs = RequestInputs {
            tenant_scope_id: tenant_scope_id.clone(),
            wallet_domain_ref,
            route_generation_ref: route_generation
                .to_content_ref()
                .expect("route generation reference"),
            chain_id: 1,
            signer_binding_ref: signer_descriptor.reference().clone(),
            nonce_policy_ref: evm_wallet_nonce_policy_ref()
                .and_then(|reference| reference.to_content_ref())
                .expect("nonce policy reference"),
            initial_nonce: 7,
            initial_nonce_descriptor_ref: initial_nonce_descriptor
                .reference()
                .and_then(|reference| reference.to_content_ref())
                .expect("initial nonce reference"),
            already_known_classifier_ref: evm_already_known_classifier_ref()
                .expect("already-known classifier")
                .to_content_ref()
                .expect("already-known classifier reference"),
            finality_policy_ref: evm_wallet_finality_policy_ref()
                .and_then(|reference| reference.to_content_ref())
                .expect("finality policy reference"),
            assurance_policy_ref: evm_wallet_assurance_policy_ref()
                .and_then(|reference| reference.to_content_ref())
                .expect("assurance policy reference"),
            sender,
            bounds,
            max_attempt_result_bytes,
        };
        let request = make_request(&request_inputs, U256::from(5));
        let signer_calls = Arc::new(AtomicUsize::new(0));
        let signer_available = Arc::new(AtomicBool::new(true));
        let provider = TestSigner {
            binding: signer_binding.clone(),
            key,
            calls: Arc::clone(&signer_calls),
            available: Arc::clone(&signer_available),
        };
        let binder =
            GenerationGuardedDeterministicSigningProviderBinder::new(signer_binding, move || {
                let provider: Arc<dyn GenerationGuardedDeterministicSigningProvider> =
                    Arc::new(provider.clone());
                Box::pin(async move { Ok(provider) })
            });
        let rpc = LoopbackRpc::start(listener, request.clone(), scenario, expected_authorization);
        let store = MemoryExecutorStore::new(&binding);
        let executor = wallet_executor(
            store.clone(),
            binding.clone(),
            binder.clone(),
            Arc::clone(&qualification),
        );
        let committed = committed_request(&binding, &tenant_scope_id, request);
        Self {
            binding,
            store,
            executor,
            rpc,
            signer: binder,
            qualification,
            committed,
            request_inputs,
            signer_calls,
            signer_available,
        }
    }

    fn restart(&self, store: MemoryExecutorStore) -> EvmWalletExecutor<MemoryExecutorStore> {
        wallet_executor(
            store,
            self.binding.clone(),
            self.signer.clone(),
            Arc::clone(&self.qualification),
        )
    }
}

fn wallet_executor(
    store: MemoryExecutorStore,
    binding: VerifiedExecutorBinding,
    signer: GenerationGuardedDeterministicSigningProviderBinder,
    qualification: Arc<EvmWalletRequestQualification>,
) -> EvmWalletExecutor<MemoryExecutorStore> {
    let ledger = KeyedExecutorLedger::new(store, binding).expect("keyed ledger");
    EvmWalletExecutor::new(ledger, signer, qualification).expect("wallet executor")
}

fn make_request(inputs: &RequestInputs, value: U256) -> EvmSubmitTransactionRequest {
    let template = EvmWalletTransactionTemplate::new(
        EvmTransactionTarget::new("primary").expect("target"),
        EvmWalletTransactionAction::call(RECIPIENT),
        value,
        [0xde, 0xad],
        vec![EvmWalletAccessListEntry::new(
            RECIPIENT,
            vec![b256!(
                "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
            )],
        )
        .expect("access list")],
        U256::from(75_000),
    )
    .expect("template");
    let replacement = EvmWalletReplacementPolicy::new(vec![
        EvmWalletFeeCandidate::new(U256::from(20), U256::from(2)).expect("fee"),
        EvmWalletFeeCandidate::new(U256::from(30), U256::from(3)).expect("fee"),
    ])
    .expect("replacement");
    let policy = EvmWalletPolicy::new(
        EvmWalletReference::from_content_ref(inputs.wallet_domain_ref.clone()),
        inputs.tenant_scope_id.clone(),
        EvmWalletReference::from_content_ref(inputs.route_generation_ref.clone()),
        inputs.chain_id,
        inputs.sender,
        EvmWalletReference::from_content_ref(inputs.signer_binding_ref.clone()),
        EvmWalletReference::from_content_ref(inputs.nonce_policy_ref.clone()),
        inputs.initial_nonce,
        EvmWalletReference::from_content_ref(inputs.initial_nonce_descriptor_ref.clone()),
        replacement,
        EvmWalletReference::from_content_ref(inputs.already_known_classifier_ref.clone()),
        EvmWalletReference::from_content_ref(inputs.finality_policy_ref.clone()),
        EvmWalletReference::from_content_ref(inputs.assurance_policy_ref.clone()),
        EvmWalletConvergencePlan::new(2, 2, 2, 2, 2, inputs.max_attempt_result_bytes)
            .expect("convergence"),
        inputs.bounds.clone(),
    )
    .expect("policy");
    let template_ref = wallet_value_ref(&template);
    let policy_ref = wallet_value_ref(&policy);
    EvmSubmitTransactionRequest::new(template_ref, template, policy_ref, policy).expect("request")
}

fn wallet_value_ref<ValueType>(value: &ValueType) -> EvmWalletReference
where
    ValueType: MfmValue + serde::Serialize,
{
    let canonical = encode_boundary(value).expect("canonical wallet value");
    EvmWalletReference::from_content_ref(
        SchemaQualifiedCanonicalValue::new(
            ValueType::schema_id().expect("wallet schema"),
            canonical.as_bytes(),
        )
        .expect("schema-qualified wallet value")
        .reference()
        .expect("wallet value ref"),
    )
}

fn committed_request(
    binding: &VerifiedExecutorBinding,
    tenant_scope_id: &TenantScopeId,
    request: EvmSubmitTransactionRequest,
) -> CommittedEffectRequest<EvmSubmitTransactionRequest> {
    CommittedEffectRequest::new(
        binding.binding_ref().clone(),
        tenant_scope_id.clone(),
        &StoreScopeId::new("mfm.store_scope.v1:00000000000000000000000000000051")
            .expect("store scope"),
        &RunId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"wallet test run"),
        ),
        &NodeId::from_digest(
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(b"wallet test node"),
        ),
        request,
    )
    .expect("committed effect request")
}

fn retained_contract(
    label: &str,
    schema_contract: &str,
    evidence_contract_ref: &ContentRef,
) -> RetainedValueContract {
    RetainedValueContract::new(
        RecoverabilityContract::embedded()
            .expect("recoverability contract")
            .schema_id(schema_contract)
            .expect("retained schema")
            .clone(),
        SemanticTypeId::new(
            "mfm.test.evm-wallet",
            label,
            "1",
            DigestAlgorithm::Sha256JcsV1,
            sha256_digest_bytes(format!("semantic:{label}").as_bytes()),
        )
        .expect("semantic type"),
        StableId::new(format!("mfm.test.evm-wallet.{label}")).expect("role"),
        "application/json",
        evidence_contract_ref.clone(),
    )
    .expect("retained contract")
}

fn reviewed_ref(label: &str) -> ContentRef {
    let schema = SchemaId::new(
        "mfm.test.evm-wallet-reference",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(b"schema:mfm.test.evm-wallet-reference:1"),
    )
    .expect("schema");
    let canonical = PlainCanonicalJsonBytes::from_json_str(&json!({"label": label}).to_string())
        .expect("canonical");
    boundary_content_ref(schema, &canonical).expect("content ref")
}

fn signing_address(key: &SigningKey) -> Address {
    let encoded = key.verifying_key().to_encoded_point(false);
    let digest = keccak256(&encoded.as_bytes()[1..]);
    Address::from_slice(&digest[12..])
}

async fn drive_to_terminal(
    executor: &EvmWalletExecutor<MemoryExecutorStore>,
    committed: &CommittedEffectRequest<EvmSubmitTransactionRequest>,
    rpc: &LoopbackRpc,
) -> VerifiedEnsureResult {
    for _ in 0..40 {
        let result = expect_returned(executor.drive(committed).await, rpc);
        if matches!(result.outcome(), Ensure::Terminal { .. }) {
            return result;
        }
    }
    panic!("finite wallet script did not reach terminal evidence")
}

fn expect_returned(outcome: EffectExecutorOutcome, rpc: &LoopbackRpc) -> VerifiedEnsureResult {
    match outcome.into_parts() {
        EffectExecutorOutcomeParts::Returned(result) => result,
        failure => panic!(
            "unexpected wallet outcome {failure:?}; calls={:?}",
            rpc.calls()
        ),
    }
}

fn terminal_attempt(result: &VerifiedEnsureResult) -> EvmWalletAttemptResult {
    let Ensure::Terminal { evidence } = result.outcome() else {
        panic!("expected terminal result")
    };
    let canonical =
        PlainCanonicalJsonBytes::from_canonical_json_slice(evidence.domain_evidence().as_bytes())
            .expect("terminal canonical bytes");
    decode_boundary(&canonical).expect("terminal attempt result")
}

async fn drive_from_preterminal_checkpoint(fixture: &Fixture) -> (Vec<u8>, EvmWalletAttemptResult) {
    for _ in 0..40 {
        let before = fixture
            .store
            .checkpoint()
            .expect("checkpoint")
            .to_durable_bytes()
            .expect("checkpoint bytes");
        let result = expect_returned(
            fixture.executor.drive(&fixture.committed).await,
            &fixture.rpc,
        );
        if matches!(result.outcome(), Ensure::Terminal { .. }) {
            return (before, terminal_attempt(&result));
        }
    }
    panic!("finite wallet script did not reach terminal evidence")
}

fn restore_checkpoint(fixture: &Fixture, durable: &[u8]) -> MemoryExecutorStore {
    MemoryExecutorStore::restore(
        &fixture.binding,
        mfm_executor::MemoryLedgerCheckpoint::from_durable_bytes(durable)
            .expect("decode checkpoint"),
    )
    .expect("restore")
}

fn returned_attempt_outcome(result: &EvmWalletAttemptResult) -> DeliveryAttemptOutcome {
    let canonical = encode_boundary(result).expect("canonical attempt result");
    let safe_result = SchemaQualifiedCanonicalValue::new(
        EvmWalletAttemptResult::schema_id().expect("attempt-result schema"),
        canonical.as_bytes(),
    )
    .expect("schema-qualified attempt result");
    DeliveryAttemptOutcome::returned(safe_result).expect("returned attempt outcome")
}

fn prepared_lookup_for_classification(
    fixture: &Fixture,
) -> crate::wallet_rpc::PreparedEvmWalletTarget {
    let candidate = EvmWalletTransactionCandidate::new(
        fixture.committed.request().clone(),
        fixture.request_inputs.initial_nonce,
        0,
        b256!("abababababababababababababababababababababababababababababababab"),
    )
    .expect("classification candidate");
    EvmWalletJsonRpcTarget::new(fixture.signer.clone(), Arc::clone(&fixture.qualification))
        .expect("wallet target")
        .prepare_transaction_lookup(fixture.committed.request(), candidate)
        .expect("prepared lookup")
}

fn valid_lookup_result(
    prepared: &crate::wallet_rpc::PreparedEvmWalletTarget,
) -> EvmWalletAttemptResult {
    EvmWalletAttemptResult::TransactionLookup {
        transaction_hash: prepared.candidate().transaction_hash().to_owned(),
        transaction: None,
    }
}

fn executor_failure_code(outcome: EffectExecutorOutcome) -> mfm_executor::NonDomainFailureCode {
    let EffectExecutorOutcomeParts::NonDomainFailure(failure) = outcome.into_parts() else {
        panic!("executor error classification must produce a non-domain failure")
    };
    failure.fields().code
}

#[tokio::test]
async fn executor_error_classification_distinguishes_fresh_material_from_retained_history() {
    let fixture = Fixture::new(Scenario::Succeeded).await;

    assert_eq!(
        executor_failure_code(
            fixture
                .executor
                .classify_executor_error_for_test(&ExecutorError::TargetOperationMismatch, false,),
        ),
        mfm_journal::NonDomainFailureCode::AdapterContractViolation
    );
    assert_eq!(
        executor_failure_code(
            fixture
                .executor
                .classify_executor_error_for_test(&ExecutorError::TargetOperationMismatch, true,),
        ),
        mfm_journal::NonDomainFailureCode::ExecutorHistoryInvalid
    );
    assert_eq!(
        executor_failure_code(
            fixture
                .executor
                .classify_executor_error_for_test(&ExecutorError::CanonicalEncoding, false),
        ),
        mfm_journal::NonDomainFailureCode::ResultEncodingFailure
    );
    assert_eq!(
        executor_failure_code(
            fixture
                .executor
                .classify_executor_error_for_test(&ExecutorError::CanonicalEncoding, true),
        ),
        mfm_journal::NonDomainFailureCode::ExecutorHistoryInvalid
    );
    assert_eq!(
        executor_failure_code(
            fixture
                .executor
                .classify_executor_error_for_test(&ExecutorError::LocalContention, false),
        ),
        mfm_executor::NonDomainFailureCode::ExecutorContention
    );
}

#[tokio::test]
async fn exact_qualified_completion_maximum_admits_observations_and_the_largest_tombstone() {
    let baseline = Fixture::new(Scenario::Succeeded).await;
    let (max_observation, max_tombstone) =
        crate::wallet_qualification::calculated_completion_maxima_for_test(
            &baseline.binding,
            baseline.committed.request(),
        )
        .expect("calculated completion maxima");
    let exact_maximum = max_observation.max(max_tombstone);
    let exact_maximum_u64 = u64::try_from(exact_maximum).expect("completion maximum");
    let exact_bounds = EvidenceBounds::new(
        20,
        64,
        8 * 1024 * 1024,
        exact_maximum_u64,
        2,
        exact_maximum_u64,
    )
    .expect("exact EVM completion bounds");
    drop(baseline);

    let fixture = Fixture::new_with_bounds(Scenario::Succeeded, exact_bounds).await;
    let recalculated = crate::wallet_qualification::calculated_completion_maxima_for_test(
        &fixture.binding,
        fixture.committed.request(),
    )
    .expect("recalculated maxima");
    assert_eq!(recalculated, (max_observation, max_tombstone));
    assert_eq!(
        fixture
            .binding
            .contract()
            .evidence_bounds()
            .max_completion_record_bytes(),
        exact_maximum
    );
    fixture
        .qualification
        .verify_request(fixture.committed.request())
        .expect("exact completion maximum is qualified");
    let terminal = drive_to_terminal(&fixture.executor, &fixture.committed, &fixture.rpc).await;
    assert!(matches!(terminal.outcome(), Ensure::Terminal { .. }));
}

#[tokio::test]
async fn qualification_rejects_one_byte_less_than_the_calculated_generic_maximum() {
    let baseline = Fixture::new(Scenario::Succeeded).await;
    let (max_observation, max_tombstone) =
        crate::wallet_qualification::calculated_completion_maxima_for_test(
            &baseline.binding,
            baseline.committed.request(),
        )
        .expect("calculated completion maxima");
    let required = max_observation.max(max_tombstone);
    drop(baseline);
    let reserve = u64::try_from(required).expect("completion reserve");
    let one_short = u64::try_from(required - 1).expect("one-short maximum");
    let fixture = Fixture::new_with_bounds(
        Scenario::Succeeded,
        EvidenceBounds::new(20, 64, 8 * 1024 * 1024, one_short, 2, reserve)
            .expect("one-short bounds"),
    )
    .await;
    assert_eq!(
        fixture
            .qualification
            .verify_request(fixture.committed.request()),
        Err(crate::EvmWalletLiveError::InvalidContract)
    );
}

#[tokio::test]
async fn one_byte_oversized_valid_rpc_result_is_persisted_as_result_unrepresentable() {
    let sample = EvmWalletAttemptResult::TransactionLookup {
        transaction_hash: "0xabababababababababababababababababababababababababababababababab"
            .to_owned(),
        transaction: None,
    };
    let sample_bytes = encode_boundary(&sample).expect("sample lookup result");
    let result_limit =
        u64::try_from(sample_bytes.as_bytes().len() - 1).expect("one-short result limit");
    let bounds =
        EvidenceBounds::new(20, 64, 8 * 1024 * 1024, 256 * 1024, 2, 256 * 1024).expect("bounds");
    let fixture = Fixture::new_with_configuration(Scenario::Succeeded, bounds, result_limit).await;
    fixture
        .qualification
        .verify_request(fixture.committed.request())
        .expect("one-short result contract remains exactly qualified");
    let candidate = EvmWalletTransactionCandidate::new(
        fixture.committed.request().clone(),
        fixture.request_inputs.initial_nonce,
        0,
        b256!("abababababababababababababababababababababababababababababababab"),
    )
    .expect("lookup candidate");
    let target =
        EvmWalletJsonRpcTarget::new(fixture.signer.clone(), Arc::clone(&fixture.qualification))
            .expect("wallet target");
    let prepared = target
        .prepare_transaction_lookup(fixture.committed.request(), candidate)
        .expect("prepared lookup");
    let target_operation = prepared.descriptor().canonical().clone();
    let ledger = KeyedExecutorLedger::new(fixture.store.clone(), fixture.binding.clone())
        .expect("keyed ledger");
    let bound = ledger
        .bind_effect(fixture.committed.identity())
        .await
        .expect("bind effect");
    let observed = ledger
        .execute_target_once(
            fixture.committed.identity(),
            &bound.delivery_audit().head_ref().expect("bound head"),
            target_operation,
            None,
            move |_authority| async move {
                let valid_result = valid_lookup_result(&prepared);
                assert_eq!(
                    encode_boundary(&valid_result)
                        .expect("valid lookup result")
                        .as_bytes()
                        .len(),
                    usize::try_from(result_limit).expect("result limit") + 1
                );
                prepared.classify_with_result_bound_for_test(
                    valid_result,
                    usize::try_from(result_limit).expect("result limit"),
                )
            },
        )
        .await
        .expect("persist totalized observation");
    let ExecuteTargetOutcome::Observed(view) = observed else {
        panic!("fresh lookup authorization must be observed")
    };
    let attempts = view.delivery_audit().attempts().expect("attempts");
    let failure = attempts[0]
        .outcome()
        .expect("persisted outcome")
        .indeterminate_failure()
        .expect("one-byte oversized valid result must be totalized");
    assert_eq!(
        failure.stable_code(),
        &ReferenceFailureCode::ResultUnrepresentable
    );
}

#[tokio::test]
async fn valid_oversized_postexchange_result_is_result_unrepresentable() {
    let fixture = Fixture::new(Scenario::Succeeded).await;
    let prepared = prepared_lookup_for_classification(&fixture);
    let outcome = prepared.classify_with_result_bound_for_test(valid_lookup_result(&prepared), 1);
    let failure = outcome
        .indeterminate_failure()
        .expect("oversized valid result must be a domain-safe indeterminate outcome");
    assert_eq!(
        failure.stable_code(),
        &ReferenceFailureCode::ResultUnrepresentable
    );
    assert_eq!(
        failure.failure_class(),
        mfm_executor::FailureClass::UnrepresentableResponse
    );
    assert_eq!(
        failure.boundary_stage(),
        mfm_executor::BoundaryStage::BoundaryObservation
    );
}

#[tokio::test]
async fn invalid_typed_postexchange_result_is_adapter_contract_violation() {
    let fixture = Fixture::new(Scenario::Succeeded).await;
    let prepared = prepared_lookup_for_classification(&fixture);
    let outcome = prepared.classify_with_result_bound_for_test(
        EvmWalletAttemptResult::TransactionLookup {
            transaction_hash: "not-a-transaction-hash".to_owned(),
            transaction: None,
        },
        usize::MAX,
    );
    let failure = outcome
        .non_domain_failure_value()
        .expect("invalid typed result must be an audit-only target failure");
    let fields = failure.fields();
    assert_eq!(
        fields.entry_status,
        mfm_journal::NonDomainEntryStatus::MayHaveEntered
    );
    assert_eq!(
        fields.disposition,
        mfm_journal::NonDomainDisposition::IntegrityBlocked
    );
    assert_eq!(
        fields.code,
        mfm_journal::NonDomainFailureCode::AdapterContractViolation
    );
}

#[tokio::test]
async fn postexchange_canonical_encoding_failure_is_result_encoding_failure() {
    let fixture = Fixture::new(Scenario::Succeeded).await;
    let prepared = prepared_lookup_for_classification(&fixture);
    let outcome = prepared.classify_encoding_failure_for_test(valid_lookup_result(&prepared));
    let failure = outcome
        .non_domain_failure_value()
        .expect("canonical encoding failure must be an audit-only target failure");
    let fields = failure.fields();
    assert_eq!(
        fields.entry_status,
        mfm_journal::NonDomainEntryStatus::MayHaveEntered
    );
    assert_eq!(
        fields.disposition,
        mfm_journal::NonDomainDisposition::IntegrityBlocked
    );
    assert_eq!(
        fields.code,
        mfm_journal::NonDomainFailureCode::ResultEncodingFailure
    );
}

async fn append_terminal_attempt_result(
    fixture: &Fixture,
    store: &MemoryExecutorStore,
    result: EvmWalletAttemptResult,
) {
    let target_operation = terminal_target_operation(fixture, &result);
    let ledger =
        KeyedExecutorLedger::new(store.clone(), fixture.binding.clone()).expect("keyed ledger");
    let view = ledger
        .effect_view(fixture.committed.identity())
        .await
        .expect("effect view")
        .expect("bound effect");
    let outcome = ledger
        .execute_target_once(
            fixture.committed.identity(),
            &view.delivery_audit().head_ref().expect("audit head"),
            target_operation,
            Some(fixture.qualification.resource_policy_binding()),
            |_authority| async move { returned_attempt_outcome(&result) },
        )
        .await
        .expect("append hostile terminal result");
    assert!(matches!(outcome, ExecuteTargetOutcome::Observed(_)));
}

fn terminal_target_operation(
    fixture: &Fixture,
    result: &EvmWalletAttemptResult,
) -> SchemaQualifiedCanonicalValue {
    let EvmWalletAttemptResult::CanonicalInclusion {
        block: Some(block),
        terminal: Some(evidence),
    } = result
    else {
        panic!("expected terminal canonical-inclusion result")
    };
    EvmWalletTargetEntryDescriptor::canonical_inclusion(
        fixture.committed.request(),
        evidence.candidate(),
        block.number_quantity().expect("inclusion number"),
    )
    .expect("canonical-inclusion descriptor")
    .canonical()
    .clone()
}

struct HeldTerminalAttempt {
    attempt_id: AttemptId,
    release: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<mfm_executor::Result<ExecuteTargetOutcome>>,
}

impl HeldTerminalAttempt {
    async fn release_and_observe(&mut self) {
        self.release
            .take()
            .expect("held attempt release")
            .send(())
            .expect("held attempt receiver");
        let outcome = (&mut self.task)
            .await
            .expect("held attempt task")
            .expect("observe held attempt");
        assert!(matches!(outcome, ExecuteTargetOutcome::Observed(_)));
    }
}

async fn hold_terminal_attempt(
    fixture: &Fixture,
    ledger: KeyedExecutorLedger<MemoryExecutorStore>,
    expected_head: DeliveryAuditFrontierRef,
    target_operation: SchemaQualifiedCanonicalValue,
    result: EvmWalletAttemptResult,
) -> HeldTerminalAttempt {
    let identity = fixture.committed.identity().clone();
    let policy_binding = fixture.qualification.resource_policy_binding().clone();
    let (authorized_tx, authorized_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        ledger
            .execute_target_once(
                &identity,
                &expected_head,
                target_operation,
                Some(&policy_binding),
                move |authority| {
                    authorized_tx
                        .send(authority.attempt_id().clone())
                        .expect("authorized attempt receiver");
                    async move {
                        release_rx.await.expect("held attempt release");
                        returned_attempt_outcome(&result)
                    }
                },
            )
            .await
    });
    let attempt_id = authorized_rx
        .await
        .expect("target authorization must precede held observation");
    HeldTerminalAttempt {
        attempt_id,
        release: Some(release_tx),
        task,
    }
}

async fn assert_restored_drive_is_read_only_failure(fixture: &Fixture, store: MemoryExecutorStore) {
    let durable = store
        .checkpoint()
        .expect("hostile checkpoint")
        .to_durable_bytes()
        .expect("hostile checkpoint bytes");
    let restored = restore_checkpoint(fixture, &durable);
    let rpc_calls = fixture.rpc.call_count();
    let signer_calls = fixture.signer_calls.load(Ordering::SeqCst);
    let executor = fixture.restart(restored.clone());
    let EffectExecutorOutcomeParts::NonDomainFailure(failure) =
        executor.drive(&fixture.committed).await.into_parts()
    else {
        panic!("hostile restored history must produce a non-domain failure")
    };
    assert_eq!(
        failure.fields().code,
        mfm_journal::NonDomainFailureCode::ExecutorHistoryInvalid
    );
    assert_eq!(fixture.rpc.call_count(), rpc_calls);
    assert_eq!(fixture.signer_calls.load(Ordering::SeqCst), signer_calls);
    assert_eq!(
        restored
            .checkpoint()
            .expect("checkpoint after rejection")
            .to_durable_bytes()
            .expect("checkpoint bytes after rejection"),
        durable
    );
}

#[tokio::test]
async fn qualification_equality_binds_the_same_private_live_transport_instance() {
    let fixture = Fixture::new(Scenario::Succeeded).await;
    let qualification = fixture.qualification.as_ref();
    let cloned = qualification.clone();
    assert_eq!(qualification, &cloned);
    assert_eq!(qualification.canonical(), cloned.canonical());
    assert_eq!(qualification.reference(), cloned.reference());

    let rendered = format!("{qualification:?}");
    assert!(rendered.contains("<private-live-transport>"));
    for forbidden in [
        "127.0.0.1",
        WALLET_AUTHORIZATION_PREFIX,
        "authorization",
        "endpoint",
    ] {
        assert!(!rendered.contains(forbidden), "{forbidden}");
        assert!(
            !qualification.canonical().as_str().contains(forbidden),
            "{forbidden}"
        );
    }

    let mut routes = EvmRoutingCatalogBuilder::new();
    let independent_route = routes
        .insert(
            "primary",
            "mfm.test.evm-wallet-source",
            1,
            StableId::new("mfm.test.evm-wallet-route-generation").expect("generation id"),
            EvmRpcEndpoint::new("http://127.0.0.1:9").expect("independent endpoint"),
            None,
        )
        .expect("independent route");
    assert_eq!(
        independent_route
            .to_content_ref()
            .expect("independent route reference"),
        fixture.request_inputs.route_generation_ref
    );
    let independent_transport =
        EvmJsonRpcTransport::new(routes.build().expect("independent catalog"))
            .expect("independent transport");
    let independent = EvmWalletRequestQualification::qualify(
        &independent_transport,
        independent_route,
        fixture.binding.clone(),
        fixture.signer.binding(),
        qualification.initial_nonce_descriptor().clone(),
        qualification.object_evidence_contract_ref().clone(),
    )
    .expect("independent qualification");
    assert_eq!(qualification.canonical(), independent.canonical());
    assert_eq!(qualification.reference(), independent.reference());
    assert_ne!(qualification, &independent);
}

#[tokio::test]
async fn qualification_owned_transport_executes_after_original_handle_is_dropped() {
    let fixture = Fixture::new(Scenario::Succeeded).await;
    let terminal = drive_to_terminal(&fixture.executor, &fixture.committed, &fixture.rpc).await;
    assert!(matches!(terminal.outcome(), Ensure::Terminal { .. }));
    assert!(fixture.rpc.call_count() > 0);
}

#[tokio::test]
async fn concurrent_ensure_does_not_duplicate_one_plan_and_rejects_request_substitution() {
    let fixture = Fixture::new(Scenario::Succeeded).await;
    let (left, right) = tokio::join!(
        fixture.executor.drive(&fixture.committed),
        fixture.executor.drive(&fixture.committed)
    );
    let left = expect_returned(left, &fixture.rpc);
    let right = expect_returned(right, &fixture.rpc);
    assert_eq!(fixture.rpc.operation_count("eth_sendRawTransaction"), 1);
    assert!(
        left.delivery_audit().attempt_count() <= 2 && right.delivery_audit().attempt_count() <= 2
    );

    let substituted = committed_request(
        &fixture.binding,
        &fixture.request_inputs.tenant_scope_id,
        make_request(&fixture.request_inputs, U256::from(6)),
    );
    let EffectExecutorOutcomeParts::NonDomainFailure(failure) =
        fixture.executor.drive(&substituted).await.into_parts()
    else {
        panic!("request substitution must be audit-only")
    };
    assert_eq!(
        failure.fields().code,
        mfm_journal::NonDomainFailureCode::ExecutorHistoryInvalid
    );
}

#[tokio::test]
async fn every_qualified_policy_mismatch_fails_before_effect_or_nonce_allocation() {
    let fixture = Fixture::new(Scenario::Succeeded).await;
    let before = fixture
        .store
        .checkpoint()
        .expect("checkpoint")
        .to_durable_bytes()
        .expect("checkpoint bytes");
    let mut hostile = Vec::new();

    let mut changed = fixture.request_inputs.clone();
    changed.tenant_scope_id =
        TenantScopeId::new("mfm.tenant_scope.v1:00000000000000000000000000000042").expect("tenant");
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.wallet_domain_ref = reviewed_ref("other-wallet-domain");
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.route_generation_ref = reviewed_ref("other-route-generation");
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.chain_id = 2;
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.sender = RECIPIENT;
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.signer_binding_ref = reviewed_ref("other-signer-binding");
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.nonce_policy_ref = reviewed_ref("other-nonce-policy");
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.initial_nonce = 8;
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.initial_nonce_descriptor_ref = reviewed_ref("other-initial-nonce");
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.already_known_classifier_ref = reviewed_ref("other-already-known-classifier");
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.finality_policy_ref = reviewed_ref("other-finality-policy");
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.assurance_policy_ref = reviewed_ref("other-assurance-policy");
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.bounds = EvidenceBounds::new(21, 64, 8 * 1024 * 1024, 256 * 1024, 2, 256 * 1024)
        .expect("changed max attempts");
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.bounds = EvidenceBounds::new(20, 65, 8 * 1024 * 1024, 256 * 1024, 2, 256 * 1024)
        .expect("changed max records");
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.bounds = EvidenceBounds::new(20, 64, 8 * 1024 * 1024 + 1, 256 * 1024, 2, 256 * 1024)
        .expect("changed retained bytes");
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.bounds = EvidenceBounds::new(20, 64, 8 * 1024 * 1024, 256 * 1024 - 1, 2, 256 * 1024)
        .expect("changed completion maximum");
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.bounds = EvidenceBounds::new(20, 64, 8 * 1024 * 1024, 256 * 1024, 2, 256 * 1024 + 1)
        .expect("changed completion bytes");
    hostile.push(changed);

    for inputs in hostile {
        let committed = committed_request(
            &fixture.binding,
            &inputs.tenant_scope_id,
            make_request(&inputs, U256::from(5)),
        );
        let EffectExecutorOutcomeParts::NonDomainFailure(failure) =
            fixture.executor.drive(&committed).await.into_parts()
        else {
            panic!("qualification mismatch must be audit-only")
        };
        assert_eq!(
            failure.fields().code,
            mfm_journal::NonDomainFailureCode::AdapterContractViolation
        );
        assert_eq!(fixture.rpc.call_count(), 0);
        assert_eq!(fixture.signer_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            fixture
                .store
                .checkpoint()
                .expect("checkpoint")
                .to_durable_bytes()
                .expect("checkpoint bytes"),
            before
        );
    }
}

#[tokio::test]
async fn response_loss_restart_recovers_by_hash_without_persisting_bearer_material() {
    let fixture = Fixture::new(Scenario::ResponseLost).await;
    let pending = expect_returned(
        fixture.executor.drive(&fixture.committed).await,
        &fixture.rpc,
    );
    assert!(matches!(pending.outcome(), Ensure::Pending { .. }));
    assert_eq!(pending.delivery_audit().attempt_count(), 1);

    let checkpoint = fixture.store.checkpoint().expect("checkpoint");
    let durable = checkpoint.to_durable_bytes().expect("durable checkpoint");
    let retained = String::from_utf8_lossy(&durable);
    for forbidden in [
        "signed_bytes",
        "raw_transaction_bytes",
        "signature",
        "private_key",
        "unlock_file",
        "authorization_header",
        WALLET_AUTHORIZATION_PREFIX,
    ] {
        assert!(!retained.contains(forbidden), "{forbidden}");
    }
    let restored = MemoryExecutorStore::restore(
        &fixture.binding,
        mfm_executor::MemoryLedgerCheckpoint::from_durable_bytes(&durable)
            .expect("decode checkpoint"),
    )
    .expect("restore");
    let restarted = fixture.restart(restored);
    fixture.signer_available.store(false, Ordering::SeqCst);
    let terminal = drive_to_terminal(&restarted, &fixture.committed, &fixture.rpc).await;
    assert!(matches!(terminal.outcome(), Ensure::Terminal { .. }));
    assert_eq!(fixture.rpc.operation_count("eth_sendRawTransaction"), 1);
    assert_eq!(fixture.signer_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn exact_already_known_response_converges_through_public_hash_observation() {
    let fixture = Fixture::new(Scenario::AlreadyKnown).await;
    let terminal = drive_to_terminal(&fixture.executor, &fixture.committed, &fixture.rpc).await;
    let attempts = terminal.delivery_audit().attempts().expect("attempts");
    let already_known = attempts.into_iter().any(|attempt| {
        attempt
            .outcome()
            .and_then(mfm_executor::DeliveryAttemptOutcome::returned_outcome)
            .and_then(|returned| {
                let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(
                    returned.safe_result().as_bytes(),
                )
                .ok()?;
                decode_boundary::<EvmWalletAttemptResult>(&canonical).ok()
            })
            .is_some_and(|result| {
                matches!(
                    result,
                    EvmWalletAttemptResult::Broadcast {
                        status: EvmWalletBroadcastStatus::AlreadyKnown,
                        ..
                    }
                )
            })
    });
    assert!(already_known);
}

#[tokio::test]
async fn finalized_success_and_revert_produce_closed_terminal_evidence() {
    for (scenario, expected_revert) in [(Scenario::Succeeded, false), (Scenario::Reverted, true)] {
        let fixture = Fixture::new(scenario).await;
        let result = drive_to_terminal(&fixture.executor, &fixture.committed, &fixture.rpc).await;
        let attempt = terminal_attempt(&result);
        let evidence = attempt
            .terminal_evidence()
            .expect("valid terminal attempt")
            .expect("terminal evidence");
        assert_eq!(evidence.request(), fixture.committed.request());
        assert_eq!(
            matches!(
                evidence.outcome().expect("terminal outcome"),
                EvmTransactionOutcome::Reverted { .. }
            ),
            expected_revert
        );
    }
}

#[tokio::test]
async fn two_terminal_inclusions_select_first_observed_across_tombstone_and_restart() {
    let fixture = Fixture::new(Scenario::Succeeded).await;
    let (preterminal, valid_result) = drive_from_preterminal_checkpoint(&fixture).await;
    let target_operation = terminal_target_operation(&fixture, &valid_result);

    for first_observed in 0..2 {
        for tombstone_before_late_observation in [false, true] {
            let store = restore_checkpoint(&fixture, &preterminal);
            let ledger = KeyedExecutorLedger::new(store.clone(), fixture.binding.clone())
                .expect("keyed ledger");
            let mut held = Vec::new();
            for _ in 0..2 {
                let view = ledger
                    .effect_view(fixture.committed.identity())
                    .await
                    .expect("effect view")
                    .expect("bound effect");
                held.push(
                    hold_terminal_attempt(
                        &fixture,
                        ledger.clone(),
                        view.delivery_audit().head_ref().expect("audit head"),
                        target_operation.clone(),
                        valid_result.clone(),
                    )
                    .await,
                );
            }
            assert_ne!(held[0].attempt_id, held[1].attempt_id);

            let late_observed = 1 - first_observed;
            let selected_attempt_id = held[first_observed].attempt_id.clone();
            held[first_observed].release_and_observe().await;

            if tombstone_before_late_observation {
                let rpc_calls = fixture.rpc.call_count();
                let signer_calls = fixture.signer_calls.load(Ordering::SeqCst);
                let result = expect_returned(
                    fixture
                        .restart(store.clone())
                        .drive(&fixture.committed)
                        .await,
                    &fixture.rpc,
                );
                assert!(matches!(result.outcome(), Ensure::Terminal { .. }));
                assert_eq!(fixture.rpc.call_count(), rpc_calls);
                assert_eq!(fixture.signer_calls.load(Ordering::SeqCst), signer_calls);
            }

            held[late_observed].release_and_observe().await;

            if !tombstone_before_late_observation {
                let rpc_calls = fixture.rpc.call_count();
                let signer_calls = fixture.signer_calls.load(Ordering::SeqCst);
                let result = expect_returned(
                    fixture
                        .restart(store.clone())
                        .drive(&fixture.committed)
                        .await,
                    &fixture.rpc,
                );
                assert!(matches!(result.outcome(), Ensure::Terminal { .. }));
                assert_eq!(fixture.rpc.call_count(), rpc_calls);
                assert_eq!(fixture.signer_calls.load(Ordering::SeqCst), signer_calls);
            }

            let view = ledger
                .effect_view(fixture.committed.identity())
                .await
                .expect("effect view")
                .expect("bound effect");
            let (_, tombstone) = view.terminal_tombstone().expect("terminal tombstone");
            assert_eq!(
                tombstone.terminal_proof().attempt_id(),
                &selected_attempt_id
            );
            let attempts = view.delivery_audit().attempts().expect("delivery attempts");
            for held_attempt in &held {
                let attempt = attempts
                    .iter()
                    .find(|attempt| attempt.attempt_id() == &held_attempt.attempt_id)
                    .expect("held attempt");
                assert!(attempt.returned_observation_ref().is_some());
            }
            let selected = attempts
                .iter()
                .find(|attempt| attempt.attempt_id() == &selected_attempt_id)
                .expect("selected attempt");
            assert_eq!(
                selected
                    .outcome()
                    .and_then(DeliveryAttemptOutcome::returned_outcome),
                Some(tombstone.terminal_proof().returned_outcome())
            );
            assert_eq!(
                selected.returned_observation_ref(),
                Some(tombstone.terminal_proof().returned_observation_ref())
            );

            let durable = store
                .checkpoint()
                .expect("terminal checkpoint")
                .to_durable_bytes()
                .expect("terminal checkpoint bytes");
            let restored = restore_checkpoint(&fixture, &durable);
            let rpc_calls = fixture.rpc.call_count();
            let signer_calls = fixture.signer_calls.load(Ordering::SeqCst);
            let result = expect_returned(
                fixture
                    .restart(restored.clone())
                    .drive(&fixture.committed)
                    .await,
                &fixture.rpc,
            );
            assert!(matches!(result.outcome(), Ensure::Terminal { .. }));
            assert_eq!(fixture.rpc.call_count(), rpc_calls);
            assert_eq!(fixture.signer_calls.load(Ordering::SeqCst), signer_calls);
            assert_eq!(
                restored
                    .checkpoint()
                    .expect("checkpoint after terminal replay")
                    .to_durable_bytes()
                    .expect("checkpoint bytes after terminal replay"),
                durable
            );
            let restored_ledger =
                KeyedExecutorLedger::new(restored, fixture.binding.clone()).expect("keyed ledger");
            let restored_view = restored_ledger
                .effect_view(fixture.committed.identity())
                .await
                .expect("restored effect view")
                .expect("restored bound effect");
            assert_eq!(
                restored_view
                    .terminal_tombstone()
                    .expect("restored terminal tombstone")
                    .1
                    .terminal_proof()
                    .attempt_id(),
                &selected_attempt_id
            );
        }
    }
}

#[tokio::test]
async fn hostile_restored_terminal_descriptor_fails_before_target_entry() {
    let fixture = Fixture::new(Scenario::Succeeded).await;
    let (preterminal, valid_result) = drive_from_preterminal_checkpoint(&fixture).await;
    let evidence = valid_result
        .terminal_evidence()
        .expect("valid terminal result")
        .expect("terminal evidence");
    let descriptor = EvmWalletTargetEntryDescriptor::finalized_head(
        fixture.committed.request(),
        evidence.candidate(),
    )
    .expect("domain-valid but unexpected descriptor");
    let store = restore_checkpoint(&fixture, &preterminal);
    let ledger =
        KeyedExecutorLedger::new(store.clone(), fixture.binding.clone()).expect("keyed ledger");
    let view = ledger
        .effect_view(fixture.committed.identity())
        .await
        .expect("effect view")
        .expect("bound effect");
    let outcome = ledger
        .execute_target_once(
            fixture.committed.identity(),
            &view.delivery_audit().head_ref().expect("audit head"),
            descriptor.canonical().clone(),
            Some(fixture.qualification.resource_policy_binding()),
            |_authority| async move { returned_attempt_outcome(&valid_result) },
        )
        .await
        .expect("persist hostile descriptor");
    assert!(matches!(outcome, ExecuteTargetOutcome::Observed(_)));
    assert_restored_drive_is_read_only_failure(&fixture, store).await;
}

#[tokio::test]
async fn hostile_restored_terminal_history_fails_before_signing_rpc_or_append() {
    let fixture = Fixture::new(Scenario::Succeeded).await;
    let (preterminal, valid_result) = drive_from_preterminal_checkpoint(&fixture).await;
    let evidence = valid_result
        .terminal_evidence()
        .expect("valid terminal result")
        .expect("terminal evidence")
        .clone();

    for field in [
        "request",
        "attempt_result_refs",
        "candidate_lineage",
        "transaction",
        "receipt",
        "finalized_head",
        "inclusion_block",
        "outer_inclusion_block",
        "executor_generation_ref",
        "generation_fence_ref",
        "assurance_policy_ref",
    ] {
        let mut wire = serde_json::to_value(&evidence).expect("terminal JSON");
        match field {
            "request" => {
                wire[field] =
                    serde_json::to_value(make_request(&fixture.request_inputs, U256::from(6)))
                        .expect("other request JSON");
            }
            "attempt_result_refs" => {
                let references = wire[field].as_array_mut().expect("attempt-result array");
                assert!(references.len() > 1);
                references.swap(0, 1);
            }
            "candidate_lineage" => {
                let first = wire[field][0].clone();
                wire[field]
                    .as_array_mut()
                    .expect("candidate-lineage array")
                    .push(first);
            }
            "transaction" => {
                wire[field]["gas_limit"] = Value::String("75001".to_owned());
            }
            "receipt" => {
                wire[field]["status"] = Value::String("reverted".to_owned());
            }
            "finalized_head" => {
                wire[field]["hash"] = Value::String(WRONG_BLOCK.to_owned());
            }
            "inclusion_block" => {
                wire[field]["hash"] = Value::String(WRONG_BLOCK.to_owned());
            }
            "outer_inclusion_block" => {}
            "executor_generation_ref" | "generation_fence_ref" | "assurance_policy_ref" => {
                wire[field] = serde_json::to_value(EvmWalletReference::from_content_ref(
                    reviewed_ref(&format!("wrong-{field}")),
                ))
                .expect("wrong terminal reference JSON");
            }
            _ => unreachable!(),
        }
        let hostile_evidence =
            serde_json::from_value(wire).expect("structurally decodable terminal evidence");
        let EvmWalletAttemptResult::CanonicalInclusion { block, .. } = &valid_result else {
            panic!("expected canonical-inclusion result")
        };
        let block = if field == "outer_inclusion_block" {
            let mut block_wire =
                serde_json::to_value(block.as_ref().expect("canonical-inclusion block"))
                    .expect("inclusion block JSON");
            block_wire["hash"] = Value::String(WRONG_BLOCK.to_owned());
            Some(serde_json::from_value(block_wire).expect("hostile outer inclusion block"))
        } else {
            block.clone()
        };
        let hostile_result = EvmWalletAttemptResult::CanonicalInclusion {
            block,
            terminal: Some(hostile_evidence),
        };
        let store = restore_checkpoint(&fixture, &preterminal);
        append_terminal_attempt_result(&fixture, &store, hostile_result).await;
        assert_restored_drive_is_read_only_failure(&fixture, store).await;
    }
}

#[tokio::test]
async fn hostile_restored_tombstone_relation_fails_without_mutation() {
    let fixture = Fixture::new(Scenario::Succeeded).await;
    let (preterminal, valid_result) = drive_from_preterminal_checkpoint(&fixture).await;

    for (operation, outcome) in [
        (
            EVM_WALLET_BROADCAST_OPERATION_ID,
            EVM_WALLET_SUCCEEDED_TERMINAL_OUTCOME,
        ),
        (
            EVM_SUBMIT_TRANSACTION_OPERATION_ID,
            "mfm.evm.wallet-terminal-outcome.wrong",
        ),
    ] {
        let store = restore_checkpoint(&fixture, &preterminal);
        append_terminal_attempt_result(&fixture, &store, valid_result.clone()).await;
        let ledger =
            KeyedExecutorLedger::new(store.clone(), fixture.binding.clone()).expect("keyed ledger");
        let view = ledger
            .effect_view(fixture.committed.identity())
            .await
            .expect("effect view")
            .expect("bound effect");
        let attempt = view
            .delivery_audit()
            .attempts()
            .expect("attempts")
            .into_iter()
            .next_back()
            .expect("terminal attempt");
        let returned = attempt
            .outcome()
            .and_then(DeliveryAttemptOutcome::returned_outcome)
            .cloned()
            .expect("terminal returned outcome");
        let proof = ReferenceTerminalProof::new(
            attempt.attempt_id().clone(),
            returned,
            attempt
                .returned_observation_ref()
                .cloned()
                .expect("returned observation"),
        )
        .expect("terminal proof");
        let tombstone =
            TerminalTombstone::new(operation, outcome, proof).expect("hostile tombstone");
        ledger
            .append_terminal_tombstone(fixture.committed.identity(), tombstone)
            .await
            .expect("generic ledger accepts domain-hostile tombstone");
        assert_restored_drive_is_read_only_failure(&fixture, store).await;
    }

    let store = restore_checkpoint(&fixture, &preterminal);
    append_terminal_attempt_result(&fixture, &store, valid_result).await;
    let ledger =
        KeyedExecutorLedger::new(store.clone(), fixture.binding.clone()).expect("keyed ledger");
    let view = ledger
        .effect_view(fixture.committed.identity())
        .await
        .expect("effect view")
        .expect("bound effect");
    // Kernel frontier tests reject internally mismatched proof tuples. Keep this
    // tuple exact so restore reaches the wallet's stronger terminal relation.
    let prior = view
        .delivery_audit()
        .attempts()
        .expect("attempts")
        .into_iter()
        .find(|attempt| {
            let Some(returned) = attempt
                .outcome()
                .and_then(DeliveryAttemptOutcome::returned_outcome)
            else {
                return false;
            };
            let canonical = PlainCanonicalJsonBytes::from_canonical_json_slice(
                returned.safe_result().as_bytes(),
            )
            .expect("canonical prior result");
            !matches!(
                decode_boundary::<EvmWalletAttemptResult>(&canonical).expect("typed prior result"),
                EvmWalletAttemptResult::CanonicalInclusion {
                    terminal: Some(_),
                    ..
                }
            )
        })
        .expect("prior returned attempt");
    let proof = ReferenceTerminalProof::new(
        prior.attempt_id().clone(),
        prior
            .outcome()
            .and_then(DeliveryAttemptOutcome::returned_outcome)
            .cloned()
            .expect("prior returned outcome"),
        prior
            .returned_observation_ref()
            .cloned()
            .expect("prior returned observation"),
    )
    .expect("prior-attempt proof");
    let tombstone = TerminalTombstone::new(
        EVM_SUBMIT_TRANSACTION_OPERATION_ID,
        EVM_WALLET_SUCCEEDED_TERMINAL_OUTCOME,
        proof,
    )
    .expect("prior-attempt tombstone");
    ledger
        .append_terminal_tombstone(fixture.committed.identity(), tombstone)
        .await
        .expect("generic ledger accepts exact prior-attempt proof tuple");
    assert_restored_drive_is_read_only_failure(&fixture, store).await;
}

#[tokio::test]
async fn restored_valid_tombstone_is_signer_rpc_and_write_free() {
    let fixture = Fixture::new(Scenario::Succeeded).await;
    let result = drive_to_terminal(&fixture.executor, &fixture.committed, &fixture.rpc).await;
    assert!(matches!(result.outcome(), Ensure::Terminal { .. }));
    let durable = fixture
        .store
        .checkpoint()
        .expect("terminal checkpoint")
        .to_durable_bytes()
        .expect("terminal checkpoint bytes");
    let restored = restore_checkpoint(&fixture, &durable);
    fixture.signer_available.store(false, Ordering::SeqCst);
    let rpc_calls = fixture.rpc.call_count();
    let signer_calls = fixture.signer_calls.load(Ordering::SeqCst);
    let result = expect_returned(
        fixture
            .restart(restored.clone())
            .drive(&fixture.committed)
            .await,
        &fixture.rpc,
    );
    assert!(matches!(result.outcome(), Ensure::Terminal { .. }));
    assert_eq!(fixture.rpc.call_count(), rpc_calls);
    assert_eq!(fixture.signer_calls.load(Ordering::SeqCst), signer_calls);
    assert_eq!(
        restored
            .checkpoint()
            .expect("checkpoint after terminal replay")
            .to_durable_bytes()
            .expect("checkpoint bytes after terminal replay"),
        durable
    );
}

#[tokio::test]
async fn pre_resolution_reorganization_requires_fresh_transaction_receipt_and_inclusion() {
    let fixture = Fixture::new(Scenario::Reorganization).await;
    let result = drive_to_terminal(&fixture.executor, &fixture.committed, &fixture.rpc).await;
    let attempt = terminal_attempt(&result);
    let evidence = attempt
        .terminal_evidence()
        .expect("valid terminal attempt")
        .expect("terminal evidence");
    assert_eq!(evidence.inclusion_block().hash(), BLOCK_B);
    assert_eq!(fixture.rpc.operation_count("eth_getTransactionByHash"), 2);
    assert_eq!(fixture.rpc.operation_count("eth_getTransactionReceipt"), 2);
    assert_eq!(fixture.rpc.operation_count("eth_sendRawTransaction"), 2);
}

#[tokio::test]
async fn bounded_rebroadcast_then_replacement_preserves_nonce_and_semantic_request() {
    let fixture = Fixture::new(Scenario::Replacement).await;
    let result = drive_to_terminal(&fixture.executor, &fixture.committed, &fixture.rpc).await;
    let attempt = terminal_attempt(&result);
    let evidence = attempt
        .terminal_evidence()
        .expect("valid terminal attempt")
        .expect("terminal evidence");
    assert_eq!(evidence.candidate().fee_ordinal(), 1);
    assert_eq!(evidence.candidate().allocated_nonce(), Ok(7));
    assert_eq!(evidence.request(), fixture.committed.request());
    let hashes = fixture.rpc.broadcast_hashes();
    assert_eq!(hashes.len(), 3);
    assert_eq!(hashes[0], hashes[1]);
    assert_ne!(hashes[1], hashes[2]);
}

#[tokio::test]
async fn signer_unavailability_never_creates_a_delivery_authorization() {
    let fixture = Fixture::new(Scenario::Succeeded).await;
    fixture.signer_available.store(false, Ordering::SeqCst);
    let EffectExecutorOutcomeParts::DidNotEnter(failure) = fixture
        .executor
        .drive(&fixture.committed)
        .await
        .into_parts()
    else {
        panic!("signer failure must be did-not-enter")
    };
    assert_eq!(
        failure.stable_code(),
        &ReferenceFailureCode::DestinationUnavailable
    );
    assert_eq!(fixture.rpc.call_count(), 0);
    assert_eq!(fixture.signer_calls.load(Ordering::SeqCst), 1);
    let snapshot = fixture
        .store
        .load_effect_snapshot(fixture.committed.identity().effect_key())
        .expect("effect snapshot")
        .expect("bound effect");
    assert_eq!(snapshot.frontiers().len(), 1);
    let ledger =
        KeyedExecutorLedger::new(fixture.store.clone(), fixture.binding.clone()).expect("ledger");
    let view = ledger
        .effect_view(fixture.committed.identity())
        .await
        .expect("effect view")
        .expect("bound view");
    assert_eq!(view.delivery_audit().attempt_count(), 0);
}
