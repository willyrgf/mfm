use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use alloy_primitives::{address, b256, keccak256, Address, PrimitiveSignature, B256, U256};
use k256::ecdsa::SigningKey;
use mfm_canonical::{sha256_digest_bytes, PlainCanonicalJsonBytes, RecoverabilityContractV1};
use mfm_evm::{
    evm_submit_transaction_leaf_expansion, evm_submit_transaction_value_contracts,
    evm_wallet_assurance_policy_ref, evm_wallet_finality_policy_ref, evm_wallet_nonce_policy_ref,
    EvmSubmitTransactionRequest, EvmTransactionOutcome, EvmTransactionTarget,
    EvmWalletAccessListEntry, EvmWalletAttemptResult, EvmWalletBroadcastStatus,
    EvmWalletConvergencePlan, EvmWalletFeeCandidate, EvmWalletInitialNonceDescriptor,
    EvmWalletPolicy, EvmWalletReference, EvmWalletReplacementPolicy, EvmWalletTransactionAction,
    EvmWalletTransactionTemplate,
};
use mfm_executor::{
    CommittedEffectRequest, Ensure, EvidenceBounds, ExecutorBinding, ExecutorContractDescriptor,
    ExecutorDeployment, ExecutorError, ExecutorRetainedClosureContract, KeyedExecutorLedger,
    MemoryExecutorStore, ReferenceFailureCode, ResourceOwnership, SchemaQualifiedCanonicalValue,
    VerifiedEnsureResult, VerifiedExecutorBinding,
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
use serde_json::{json, Value};

use crate::transport::{EvmJsonRpcTransport, EvmRoutingCatalogBuilder, EvmRpcEndpoint};
use crate::{
    evm_already_known_classifier_ref, evm_wallet_target_callback_surface_ref, EvmWalletExecutor,
    EvmWalletRequestQualification, EvmWalletRpcClient, EvmWalletRpcFailure, EvmWalletRpcFuture,
    EvmWalletRpcResponse,
};

const RECIPIENT: Address = address!("2222222222222222222222222222222222222222");
const BLOCK_A: &str = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BLOCK_B: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const WRONG_BLOCK: &str = "0xcccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const FINALIZED_BLOCK: &str = "0xdddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";

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
struct MockRpc {
    request: EvmSubmitTransactionRequest,
    state: Arc<Mutex<MockRpcState>>,
}

struct MockRpcState {
    scenario: Scenario,
    calls: Vec<&'static str>,
    broadcast_hashes: Vec<String>,
    candidate_ordinals: BTreeMap<String, usize>,
    response_lost: bool,
    reorged: bool,
}

impl MockRpc {
    fn new(request: EvmSubmitTransactionRequest, scenario: Scenario) -> Self {
        Self {
            request,
            state: Arc::new(Mutex::new(MockRpcState {
                scenario,
                calls: Vec::new(),
                broadcast_hashes: Vec::new(),
                candidate_ordinals: BTreeMap::new(),
                response_lost: false,
                reorged: false,
            })),
        }
    }

    fn call_count(&self) -> usize {
        self.state.lock().expect("mock RPC mutex").calls.len()
    }

    fn operation_count(&self, method: &'static str) -> usize {
        self.state
            .lock()
            .expect("mock RPC mutex")
            .calls
            .iter()
            .filter(|candidate| **candidate == method)
            .count()
    }

    fn calls(&self) -> Vec<&'static str> {
        self.state.lock().expect("mock RPC mutex").calls.clone()
    }

    fn broadcast_hashes(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("mock RPC mutex")
            .broadcast_hashes
            .clone()
    }

    fn response(
        &self,
        route_generation_ref: &ContentRef,
        chain_id: u64,
        method: &'static str,
        params: Value,
    ) -> Result<EvmWalletRpcResponse, EvmWalletRpcFailure> {
        if self
            .request
            .policy()
            .route_generation_ref()
            .to_content_ref()
            .ok()
            .as_ref()
            != Some(route_generation_ref)
            || chain_id != self.request.policy().chain_id()
        {
            return Err(EvmWalletRpcFailure::GenerationFenced);
        }
        let mut state = self.state.lock().expect("mock RPC mutex");
        state.calls.push(method);
        match method {
            "eth_sendRawTransaction" => {
                let raw = params
                    .as_array()
                    .and_then(|values| values.first())
                    .and_then(Value::as_str)
                    .and_then(|value| value.strip_prefix("0x"))
                    .and_then(|value| hex::decode(value).ok())
                    .ok_or(EvmWalletRpcFailure::InvalidResponse)?;
                let transaction_hash = format!("{:#x}", keccak256(raw));
                let next_ordinal = state.candidate_ordinals.len();
                state
                    .candidate_ordinals
                    .entry(transaction_hash.clone())
                    .or_insert(next_ordinal);
                state.broadcast_hashes.push(transaction_hash.clone());
                if state.scenario == Scenario::ResponseLost && !state.response_lost {
                    state.response_lost = true;
                    return Err(EvmWalletRpcFailure::ResponseLost);
                }
                if state.scenario == Scenario::AlreadyKnown && state.broadcast_hashes.len() == 1 {
                    return Ok(EvmWalletRpcResponse::Error(
                        crate::wallet_rpc::EvmWalletRpcError::new(
                            -32_000,
                            "already known".to_owned(),
                            true,
                        ),
                    ));
                }
                Ok(EvmWalletRpcResponse::Result(Value::String(
                    transaction_hash,
                )))
            }
            "eth_getTransactionByHash" => {
                let hash = hash_parameter(&params)?;
                let ordinal = candidate_ordinal(&state, &hash)?;
                if state.scenario == Scenario::Replacement && ordinal == 0 {
                    return Ok(EvmWalletRpcResponse::Result(Value::Null));
                }
                let (block_number, block_hash) = active_block(&state);
                Ok(EvmWalletRpcResponse::Result(transaction_json(
                    &self.request,
                    &hash,
                    ordinal,
                    block_number,
                    block_hash,
                )?))
            }
            "eth_getTransactionReceipt" => {
                let hash = hash_parameter(&params)?;
                let ordinal = candidate_ordinal(&state, &hash)?;
                if state.scenario == Scenario::Replacement && ordinal == 0 {
                    return Ok(EvmWalletRpcResponse::Result(Value::Null));
                }
                let (block_number, block_hash) = active_block(&state);
                Ok(EvmWalletRpcResponse::Result(receipt_json(
                    &self.request,
                    &hash,
                    block_number,
                    block_hash,
                    state.scenario == Scenario::Reverted,
                )?))
            }
            "eth_getBlockByNumber" => {
                let selector = params
                    .as_array()
                    .and_then(|values| values.first())
                    .and_then(Value::as_str)
                    .ok_or(EvmWalletRpcFailure::InvalidResponse)?;
                if selector == "finalized" {
                    return Ok(EvmWalletRpcResponse::Result(json!({
                        "hash": FINALIZED_BLOCK,
                        "number": "0x6e",
                    })));
                }
                let (number, hash) = active_block(&state);
                if selector != number {
                    return Err(EvmWalletRpcFailure::InvalidResponse);
                }
                if state.scenario == Scenario::Reorganization && !state.reorged {
                    state.reorged = true;
                    return Ok(EvmWalletRpcResponse::Result(json!({
                        "hash": WRONG_BLOCK,
                        "number": number,
                    })));
                }
                Ok(EvmWalletRpcResponse::Result(json!({
                    "hash": hash,
                    "number": number,
                })))
            }
            _ => Err(EvmWalletRpcFailure::InvalidResponse),
        }
    }
}

impl EvmWalletRpcClient for MockRpc {
    fn exchange<'a>(
        &'a self,
        route_generation_ref: &'a ContentRef,
        chain_id: u64,
        method: &'static str,
        params: Value,
    ) -> EvmWalletRpcFuture<'a> {
        let response = self.response(route_generation_ref, chain_id, method, params);
        Box::pin(async move { response })
    }
}

fn hash_parameter(params: &Value) -> Result<String, EvmWalletRpcFailure> {
    params
        .as_array()
        .and_then(|values| values.first())
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(EvmWalletRpcFailure::InvalidResponse)
}

fn candidate_ordinal(
    state: &MockRpcState,
    transaction_hash: &str,
) -> Result<usize, EvmWalletRpcFailure> {
    state
        .candidate_ordinals
        .get(transaction_hash)
        .copied()
        .ok_or(EvmWalletRpcFailure::InvalidResponse)
}

fn active_block(state: &MockRpcState) -> (&'static str, &'static str) {
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
) -> Result<Value, EvmWalletRpcFailure> {
    let fee = request
        .policy()
        .replacement()
        .fee_candidates()
        .get(fee_ordinal)
        .ok_or(EvmWalletRpcFailure::InvalidResponse)?;
    let to = request
        .template()
        .action()
        .call_destination()
        .map_err(|_| EvmWalletRpcFailure::InvalidResponse)?
        .ok_or(EvmWalletRpcFailure::InvalidResponse)?;
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
        "gas": quantity(request.template().gas_limit_quantity().map_err(|_| EvmWalletRpcFailure::InvalidResponse)?),
        "hash": transaction_hash,
        "input": request.template().input(),
        "maxFeePerGas": quantity(fee.max_fee_quantity().map_err(|_| EvmWalletRpcFailure::InvalidResponse)?),
        "maxPriorityFeePerGas": quantity(fee.max_priority_fee_quantity().map_err(|_| EvmWalletRpcFailure::InvalidResponse)?),
        "nonce": quantity(U256::from(request.policy().initial_nonce().map_err(|_| EvmWalletRpcFailure::InvalidResponse)?)),
        "to": format!("{to:#x}"),
        "transactionIndex": "0x0",
        "type": "0x2",
        "value": quantity(request.template().value_quantity().map_err(|_| EvmWalletRpcFailure::InvalidResponse)?),
    }))
}

fn receipt_json(
    request: &EvmSubmitTransactionRequest,
    transaction_hash: &str,
    block_number: &str,
    block_hash: &str,
    reverted: bool,
) -> Result<Value, EvmWalletRpcFailure> {
    let to = request
        .template()
        .action()
        .call_destination()
        .map_err(|_| EvmWalletRpcFailure::InvalidResponse)?
        .ok_or(EvmWalletRpcFailure::InvalidResponse)?;
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
}

struct Fixture {
    binding: VerifiedExecutorBinding,
    store: MemoryExecutorStore,
    executor: EvmWalletExecutor<MemoryExecutorStore, MockRpc>,
    client: MockRpc,
    signer: GenerationGuardedDeterministicSigningProviderBinder,
    qualification: Arc<EvmWalletRequestQualification>,
    committed: CommittedEffectRequest<EvmSubmitTransactionRequest>,
    request_inputs: RequestInputs,
    signer_calls: Arc<AtomicUsize>,
    signer_available: Arc<AtomicBool>,
}

impl Fixture {
    fn new(scenario: Scenario) -> Self {
        let key = SigningKey::from_slice(&[7_u8; 32]).expect("test signing key");
        let sender = signing_address(&key);
        let tenant_scope_id =
            TenantScopeId::new("mfm.tenant_scope.v1:00000000000000000000000000000041")
                .expect("tenant");
        let generation_ref = reviewed_ref("wallet-generation");
        let fence_ref = reviewed_ref("wallet-generation-fence");
        let wallet_domain_ref = reviewed_ref("wallet-domain");
        let bounds = EvidenceBounds::new(20, 64, 8 * 1024 * 1024, 2, 16 * 1024).expect("bounds");
        let mut routing = EvmRoutingCatalogBuilder::new();
        let route_generation = routing
            .insert(
                "primary",
                "mfm.test.evm-wallet-source",
                1,
                StableId::new("mfm.test.evm-wallet-route-generation").expect("generation id"),
                EvmRpcEndpoint::new("http://127.0.0.1:8545").expect("endpoint"),
                None,
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
                "mfm.executor-delivery-frontier.v1",
                &object_evidence_ref,
            ),
            retained_contract(
                "executor-frontier",
                "mfm.executor-delivery-frontier.v1",
                &object_evidence_ref,
            ),
            retained_contract(
                "terminal-evidence",
                "mfm.terminal-effect-evidence.v1",
                &object_evidence_ref,
            ),
            retained_contract(
                "terminal-tombstone",
                "mfm.executor-terminal-tombstone.v1",
                &object_evidence_ref,
            ),
            retained_contract(
                "terminal-proof",
                "mfm.executor-reference-terminal-proof.v1",
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
        let client = MockRpc::new(request.clone(), scenario);
        let store = MemoryExecutorStore::new(&binding);
        let executor = wallet_executor(
            store.clone(),
            binding.clone(),
            client.clone(),
            binder.clone(),
            Arc::clone(&qualification),
        );
        let committed = committed_request(&binding, &tenant_scope_id, request);
        Self {
            binding,
            store,
            executor,
            client,
            signer: binder,
            qualification,
            committed,
            request_inputs,
            signer_calls,
            signer_available,
        }
    }

    fn restart(
        &self,
        store: MemoryExecutorStore,
    ) -> EvmWalletExecutor<MemoryExecutorStore, MockRpc> {
        wallet_executor(
            store,
            self.binding.clone(),
            self.client.clone(),
            self.signer.clone(),
            Arc::clone(&self.qualification),
        )
    }
}

fn wallet_executor(
    store: MemoryExecutorStore,
    binding: VerifiedExecutorBinding,
    client: MockRpc,
    signer: GenerationGuardedDeterministicSigningProviderBinder,
    qualification: Arc<EvmWalletRequestQualification>,
) -> EvmWalletExecutor<MemoryExecutorStore, MockRpc> {
    let ledger = KeyedExecutorLedger::new(store, binding).expect("keyed ledger");
    EvmWalletExecutor::new(ledger, client, signer, qualification).expect("wallet executor")
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
        EvmWalletConvergencePlan::new(2, 2, 2, 2, 2, 128 * 1024).expect("convergence"),
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
        RecoverabilityContractV1::embedded()
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
    executor: &EvmWalletExecutor<MemoryExecutorStore, MockRpc>,
    committed: &CommittedEffectRequest<EvmSubmitTransactionRequest>,
    client: &MockRpc,
) -> VerifiedEnsureResult {
    for _ in 0..40 {
        let result = executor
            .drive(committed)
            .await
            .unwrap_or_else(|error| panic!("wallet drive {error:?}; calls={:?}", client.calls()));
        let result = result
            .into_parts()
            .unwrap_or_else(|failure| panic!("unexpected safe failure: {failure:?}"));
        if matches!(result.outcome(), Ensure::Terminal { .. }) {
            return result;
        }
    }
    panic!("finite wallet script did not reach terminal evidence")
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

#[tokio::test]
async fn concurrent_ensure_does_not_duplicate_one_plan_and_rejects_request_substitution() {
    let fixture = Fixture::new(Scenario::Succeeded);
    let (left, right) = tokio::join!(
        fixture.executor.drive(&fixture.committed),
        fixture.executor.drive(&fixture.committed)
    );
    let left = left
        .expect("left drive")
        .into_parts()
        .expect("left returned outcome");
    let right = right
        .expect("right drive")
        .into_parts()
        .expect("right returned outcome");
    assert_eq!(fixture.client.operation_count("eth_sendRawTransaction"), 1);
    assert!(
        left.delivery_audit().attempt_count() <= 2 && right.delivery_audit().attempt_count() <= 2
    );

    let substituted = committed_request(
        &fixture.binding,
        &fixture.request_inputs.tenant_scope_id,
        make_request(&fixture.request_inputs, U256::from(6)),
    );
    assert_eq!(
        fixture
            .executor
            .drive(&substituted)
            .await
            .expect_err("same key with another request must fail"),
        ExecutorError::EffectBindingConflict
    );
}

#[tokio::test]
async fn every_qualified_policy_mismatch_fails_before_effect_or_nonce_allocation() {
    let fixture = Fixture::new(Scenario::Succeeded);
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
    changed.bounds =
        EvidenceBounds::new(21, 64, 8 * 1024 * 1024, 2, 16 * 1024).expect("changed max attempts");
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.bounds =
        EvidenceBounds::new(20, 65, 8 * 1024 * 1024, 2, 16 * 1024).expect("changed max records");
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.bounds = EvidenceBounds::new(20, 64, 8 * 1024 * 1024 + 1, 2, 16 * 1024)
        .expect("changed retained bytes");
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.bounds = EvidenceBounds::new(20, 64, 8 * 1024 * 1024, 3, 16 * 1024)
        .expect("changed completion records");
    hostile.push(changed);
    let mut changed = fixture.request_inputs.clone();
    changed.bounds = EvidenceBounds::new(20, 64, 8 * 1024 * 1024, 2, 16 * 1024 + 1)
        .expect("changed completion bytes");
    hostile.push(changed);

    for inputs in hostile {
        let committed = committed_request(
            &fixture.binding,
            &inputs.tenant_scope_id,
            make_request(&inputs, U256::from(5)),
        );
        assert_eq!(
            fixture
                .executor
                .drive(&committed)
                .await
                .expect_err("qualification mismatch"),
            ExecutorError::TargetOperationMismatch
        );
        assert_eq!(fixture.client.call_count(), 0);
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
    let fixture = Fixture::new(Scenario::ResponseLost);
    let pending = fixture
        .executor
        .drive(&fixture.committed)
        .await
        .expect("lost response")
        .into_parts()
        .expect("returned pending outcome");
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
    let terminal = drive_to_terminal(&restarted, &fixture.committed, &fixture.client).await;
    assert!(matches!(terminal.outcome(), Ensure::Terminal { .. }));
    assert_eq!(fixture.client.operation_count("eth_sendRawTransaction"), 1);
    assert_eq!(fixture.signer_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn exact_already_known_response_converges_through_public_hash_observation() {
    let fixture = Fixture::new(Scenario::AlreadyKnown);
    let terminal = drive_to_terminal(&fixture.executor, &fixture.committed, &fixture.client).await;
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
        let fixture = Fixture::new(scenario);
        let result =
            drive_to_terminal(&fixture.executor, &fixture.committed, &fixture.client).await;
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
async fn retained_terminal_generation_fence_and_assurance_must_match_qualification() {
    let fixture = Fixture::new(Scenario::Succeeded);
    let result = drive_to_terminal(&fixture.executor, &fixture.committed, &fixture.client).await;
    let attempt = terminal_attempt(&result);
    let evidence = attempt
        .terminal_evidence()
        .expect("valid terminal attempt")
        .expect("terminal evidence");
    crate::wallet_executor::validate_terminal_binding(
        evidence,
        fixture.committed.request(),
        fixture.qualification.as_ref(),
    )
    .expect("terminal binding");

    for field in [
        "executor_generation_ref",
        "generation_fence_ref",
        "assurance_policy_ref",
    ] {
        let mut wire = serde_json::to_value(evidence).expect("terminal JSON");
        wire[field] = serde_json::to_value(EvmWalletReference::from_content_ref(reviewed_ref(
            &format!("wrong-{field}"),
        )))
        .expect("wrong reference JSON");
        let hostile: mfm_evm::EvmWalletTerminalEvidence =
            serde_json::from_value(wire).expect("hostile terminal evidence");
        assert_eq!(
            crate::wallet_executor::validate_terminal_binding(
                &hostile,
                fixture.committed.request(),
                fixture.qualification.as_ref(),
            ),
            Err(ExecutorError::TerminalProofMismatch),
        );
    }
}

#[tokio::test]
async fn pre_resolution_reorganization_requires_fresh_transaction_receipt_and_inclusion() {
    let fixture = Fixture::new(Scenario::Reorganization);
    let result = drive_to_terminal(&fixture.executor, &fixture.committed, &fixture.client).await;
    let attempt = terminal_attempt(&result);
    let evidence = attempt
        .terminal_evidence()
        .expect("valid terminal attempt")
        .expect("terminal evidence");
    assert_eq!(evidence.inclusion_block().hash(), BLOCK_B);
    assert_eq!(
        fixture.client.operation_count("eth_getTransactionByHash"),
        2
    );
    assert_eq!(
        fixture.client.operation_count("eth_getTransactionReceipt"),
        2
    );
    assert_eq!(fixture.client.operation_count("eth_sendRawTransaction"), 2);
}

#[tokio::test]
async fn bounded_rebroadcast_then_replacement_preserves_nonce_and_semantic_request() {
    let fixture = Fixture::new(Scenario::Replacement);
    let result = drive_to_terminal(&fixture.executor, &fixture.committed, &fixture.client).await;
    let attempt = terminal_attempt(&result);
    let evidence = attempt
        .terminal_evidence()
        .expect("valid terminal attempt")
        .expect("terminal evidence");
    assert_eq!(evidence.candidate().fee_ordinal(), 1);
    assert_eq!(evidence.candidate().allocated_nonce(), Ok(7));
    assert_eq!(evidence.request(), fixture.committed.request());
    let hashes = fixture.client.broadcast_hashes();
    assert_eq!(hashes.len(), 3);
    assert_eq!(hashes[0], hashes[1]);
    assert_ne!(hashes[1], hashes[2]);
}

#[tokio::test]
async fn signer_unavailability_never_creates_a_delivery_authorization() {
    let fixture = Fixture::new(Scenario::Succeeded);
    fixture.signer_available.store(false, Ordering::SeqCst);
    let (outcome, failure) = fixture
        .executor
        .drive(&fixture.committed)
        .await
        .expect("safe signer failure")
        .into_parts()
        .expect_err("signer failure must not return an ensure result");
    assert_eq!(outcome, mfm_capabilities::SafeFailureOutcome::DidNotEnter);
    assert_eq!(
        failure.stable_code(),
        &ReferenceFailureCode::DestinationUnavailable
    );
    assert_eq!(fixture.client.call_count(), 0);
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
