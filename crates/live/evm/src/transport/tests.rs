use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use alloy_primitives::{address, B256, U256};
use mfm_evm::{EvmBlockSelector, EvmReadSession, EvmReceiptStatus, EvmTransactionSession};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

const BLOCK_HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const OTHER_HASH: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";
const ACCOUNT: Address = address!("1111111111111111111111111111111111111111");
const FIXTURE_ENDPOINT_PATH: &str = "fixture-private-path-letmein";
const FIXTURE_BEARER: &str = "Bearer fixture-token-123456";
const FIXTURE_PROVIDER_BODY: &str = "fixture-raw-provider-payload";
const FIXTURE_PROVIDER_URL: &str =
    "https://fixture-user:fixture-password@provider.invalid/private?access_token=123456";
const FIXTURE_LOW_ENTROPY: &str = "letmein";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PrototypeRoutingGenerationRef(String);

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrototypeRouteResolution {
    Resolved(String),
    DidNotEnter,
}

#[derive(Default)]
struct PrototypeRouteResolver {
    current: Option<PrototypeRoutingGenerationRef>,
    routes: BTreeMap<PrototypeRoutingGenerationRef, String>,
}

impl PrototypeRouteResolver {
    fn install_current(&mut self, generation: PrototypeRoutingGenerationRef, endpoint: String) {
        self.routes.insert(generation.clone(), endpoint);
        self.current = Some(generation);
    }

    fn remove(&mut self, generation: &PrototypeRoutingGenerationRef) {
        self.routes.remove(generation);
    }

    fn resolve_exact(
        &self,
        generation: &PrototypeRoutingGenerationRef,
    ) -> PrototypeRouteResolution {
        self.routes
            .get(generation)
            .cloned()
            .map(PrototypeRouteResolution::Resolved)
            .unwrap_or(PrototypeRouteResolution::DidNotEnter)
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Valid,
    ChainMismatch,
    BadVersion,
    BadId,
    FeeMissing,
    WrongSubmitHash,
    ReceiptNull,
    ReceiptRemoved,
    HttpFailure,
    JsonError,
    Oversized,
    OversizedChunked,
    Stall,
    SlowBalance,
    SubmitHttpFailure,
    HostileCallResult,
    HostileCodeResult,
    AdversarialMalformed,
}

struct TestServer {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    raw_requests: Arc<Mutex<Vec<String>>>,
    max_active: Arc<AtomicUsize>,
}

impl TestServer {
    async fn spawn(mode: Mode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let raw_requests = Arc::new(Mutex::new(Vec::new()));
        let captured_raw = Arc::clone(&raw_requests);
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let observed_active = Arc::clone(&active);
        let observed_max = Arc::clone(&max_active);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let captured = Arc::clone(&captured);
                let captured_raw = Arc::clone(&captured_raw);
                let active = Arc::clone(&observed_active);
                let max_active = Arc::clone(&observed_max);
                tokio::spawn(async move {
                    let mut bytes = vec![0; 16 * 1024];
                    let mut read = 0;
                    loop {
                        let count = stream.read(&mut bytes[read..]).await.expect("read");
                        if count == 0 {
                            return;
                        }
                        read += count;
                        if request_complete(&bytes[..read]) {
                            break;
                        }
                    }
                    captured_raw
                        .lock()
                        .expect("raw requests")
                        .push(String::from_utf8_lossy(&bytes[..read]).into_owned());
                    let request: Value =
                        serde_json::from_slice(request_body(&bytes[..read])).expect("request JSON");
                    captured.lock().expect("requests").push(request.clone());
                    if matches!(mode, Mode::Stall) {
                        std::future::pending::<()>().await;
                    }
                    if matches!(mode, Mode::SlowBalance) && request["method"] == "eth_getBalance" {
                        let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                        max_active.fetch_max(current, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(25)).await;
                        active.fetch_sub(1, Ordering::SeqCst);
                    }
                    let response = response(mode, &request);
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("response");
                });
            }
        });
        Self {
            url: format!("http://{address}"),
            requests,
            raw_requests,
            max_active,
        }
    }

    fn requests(&self) -> Vec<Value> {
        self.requests.lock().expect("requests").clone()
    }

    fn raw_requests(&self) -> Vec<String> {
        self.raw_requests.lock().expect("raw requests").clone()
    }
}

fn assert_synthetic_material_absent(rendered: &str, material: &[&str]) {
    for value in material {
        assert!(!rendered.contains(value));
        let digest = mfm_canonical::sha256_digest_bytes(value.as_bytes());
        let fingerprint = hex::encode(digest.as_bytes());
        assert!(!rendered.contains(&fingerprint));
    }
}

fn transport() -> EvmJsonRpcTransport {
    EvmJsonRpcTransport::new().expect("transport")
}

fn endpoint(value: &str) -> EvmRpcEndpoint {
    EvmRpcEndpoint::new(value).expect("endpoint")
}

fn authorization(value: &str) -> EvmRpcAuthorization {
    EvmRpcAuthorization::new(zeroize::Zeroizing::new(value.to_owned())).expect("authorization")
}

async fn session(server: &TestServer) -> EvmJsonRpcSession {
    transport()
        .bind(
            binding(1),
            LocalPublicId::new("primary").expect("source"),
            endpoint(&server.url),
            Some(authorization("Bearer top-secret")),
        )
        .await
        .expect("bind session")
}

fn binding(chain_id: u64) -> EvmNetworkBinding {
    EvmNetworkBinding::new(LocalPublicId::new("mainnet").expect("network"), chain_id)
        .expect("binding")
}

#[tokio::test]
async fn bind_probes_once_and_every_method_uses_the_same_session() {
    let server = TestServer::spawn(Mode::Valid).await;
    let session = session(&server).await;
    let selector = EvmBlockSelector::ExactHash(BLOCK_HASH.parse().expect("hash"));

    let block = EvmReadSession::read_block(&session, &EvmBlockSelector::Latest)
        .await
        .expect("block");
    let balance = session
        .read_balance(ACCOUNT, &selector)
        .await
        .expect("balance");
    let code = session.read_code(ACCOUNT, &selector).await.expect("code");
    let call = session
        .call(
            &EvmCall::new(
                ACCOUNT,
                Address::from([2; 20]),
                U256::from(7),
                Bytes::from_static(&[0xab]),
                U256::from(50_000),
                Default::default(),
                selector,
                2,
            )
            .expect("call request"),
        )
        .await
        .expect("call");

    assert_eq!(
        block.number_quantity().expect("block number"),
        U256::from(42)
    );
    assert_eq!(balance, U256::from(1_000_000_000_000_000_000_u128));
    assert_eq!(code.bytes, Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]));
    assert_eq!(code.hash, keccak256(&code.bytes));
    assert_eq!(call, Bytes::from_static(&[0x12, 0x34]));
    assert_eq!(EvmReadSession::evidence(&session).source_ref(), "primary");
    assert!(!format!("{session:?}").contains(&server.url));
    assert!(!format!("{session:?}").contains("top-secret"));

    let requests = server.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["method"] == "eth_chainId")
            .count(),
        1
    );
    for request in requests.iter().skip(1) {
        assert_ne!(request["method"], "eth_chainId");
    }
    assert_eq!(
        requests
            .iter()
            .find(|request| request["method"] == "eth_getBalance")
            .expect("balance request")["params"][1],
        json!({"blockHash": BLOCK_HASH, "requireCanonical": true})
    );
    let call_request = requests
        .iter()
        .find(|request| request["method"] == "eth_call")
        .expect("call request");
    assert_eq!(
        call_request["params"],
        json!([{
            "from": format!("{ACCOUNT:#x}"),
            "to": format!("{:#x}", Address::from([2; 20])),
            "value": "0x7",
            "data": "0xab",
            "gas": "0xc350",
            "accessList": [],
        }, {"blockHash": BLOCK_HASH, "requireCanonical": true}])
    );
}

#[tokio::test]
async fn recoverability_prototype_assigns_one_exchange_to_each_read_operation() {
    let server = TestServer::spawn(Mode::Valid).await;
    let session = session(&server).await;
    let exact = EvmBlockSelector::ExactHash(BLOCK_HASH.parse().expect("hash"));
    let token = Address::from([2; 20]);

    EvmReadSession::read_block(&session, &EvmBlockSelector::Latest)
        .await
        .expect("latest anchor");
    EvmReadSession::call(
        &session,
        &EvmCall::new(
            Address::ZERO,
            token,
            U256::ZERO,
            Bytes::copy_from_slice(&[0x31, 0x3c, 0xe5, 0x67]),
            U256::from(100_000),
            Default::default(),
            exact.clone(),
            32,
        )
        .expect("metadata request"),
    )
    .await
    .expect("metadata");
    EvmReadSession::read_balance(&session, ACCOUNT, &exact)
        .await
        .expect("native balance");
    let mut balance_of = vec![0x70, 0xa0, 0x82, 0x31];
    balance_of.extend_from_slice(&[0_u8; 12]);
    balance_of.extend_from_slice(ACCOUNT.as_slice());
    EvmReadSession::call(
        &session,
        &EvmCall::new(
            Address::ZERO,
            token,
            U256::ZERO,
            Bytes::from(balance_of),
            U256::from(100_000),
            Default::default(),
            exact,
            32,
        )
        .expect("token balance request"),
    )
    .await
    .expect("token balance");
    EvmReadSession::read_block(&session, &EvmBlockSelector::Number(U256::from(42)))
        .await
        .expect("anchor confirmation");

    let methods = server
        .requests()
        .iter()
        .map(|request| request["method"].as_str().expect("method").to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        methods,
        [
            "eth_chainId",
            "eth_getBlockByNumber",
            "eth_call",
            "eth_getBalance",
            "eth_call",
            "eth_getBlockByNumber",
        ],
        "each target state invocation must own exactly one JSON-RPC exchange"
    );
    assert_eq!(
        methods
            .iter()
            .filter(|method| method.as_str() == "eth_chainId")
            .count(),
        1,
        "downstream reads must not silently re-bootstrap"
    );
}

#[tokio::test]
async fn recoverability_prototype_resolves_only_the_admitted_routing_generation() {
    let first = TestServer::spawn(Mode::Valid).await;
    let current = TestServer::spawn(Mode::Valid).await;
    let first_generation = PrototypeRoutingGenerationRef("sha256:generation-a".to_owned());
    let current_generation = PrototypeRoutingGenerationRef("sha256:generation-b".to_owned());
    let mut resolver = PrototypeRouteResolver::default();
    resolver.install_current(first_generation.clone(), first.url.clone());
    let admitted = resolver.current.clone().expect("admitted generation");
    resolver.install_current(current_generation, current.url.clone());

    let PrototypeRouteResolution::Resolved(resumed_endpoint) = resolver.resolve_exact(&admitted)
    else {
        panic!("admitted routing generation must remain resolvable");
    };
    transport()
        .bind(
            binding(1),
            LocalPublicId::new("primary").expect("source"),
            endpoint(&resumed_endpoint),
            None,
        )
        .await
        .expect("resume binds exact admitted generation");
    assert_eq!(first.requests().len(), 1);
    assert!(current.requests().is_empty());

    resolver.remove(&admitted);
    assert_eq!(
        resolver.resolve_exact(&admitted),
        PrototypeRouteResolution::DidNotEnter,
        "a missing admitted generation must not fall back to current routing"
    );
    assert!(current.requests().is_empty());
}

#[tokio::test]
async fn code_and_call_results_bound_the_body_before_json_decoding() {
    let call_server = TestServer::spawn(Mode::HostileCallResult).await;
    let call_session = session(&call_server).await;
    let request = EvmCall::new(
        ACCOUNT,
        Address::from([2; 20]),
        U256::ZERO,
        Bytes::new(),
        U256::from(50_000),
        Default::default(),
        EvmBlockSelector::ExactHash(BLOCK_HASH.parse().expect("hash")),
        2,
    )
    .expect("call request");

    let error = call_session
        .call_contract(&request)
        .await
        .expect_err("near-body-limit result must fail the two-byte call bound");
    assert_eq!(error, EvmTransportError::ResponseTooLarge);
    assert!(
        RpcResponseBodyLimit::HexResult {
            maximum_decoded_bytes: request.max_response_bytes(),
        }
        .maximum_body_bytes()
        .expect("call body bound")
            < MAX_RESPONSE_BYTES
    );
    assert_eq!(
        call_server
            .requests()
            .iter()
            .filter(|request| request["method"] == "eth_call")
            .count(),
        1
    );

    let code_server = TestServer::spawn(Mode::HostileCodeResult).await;
    let code_session = session(&code_server).await;
    let error = code_session
        .code(ACCOUNT, request.block())
        .await
        .expect_err("near-body-limit result must fail the deployed-code bound");
    assert_eq!(error, EvmTransportError::ResponseTooLarge);
    assert!(
        RpcResponseBodyLimit::HexResult {
            maximum_decoded_bytes: EVM_CODE_MAX_RESPONSE_BYTES,
        }
        .maximum_body_bytes()
        .expect("code body bound")
            < MAX_RESPONSE_BYTES
    );
}

#[tokio::test]
async fn transaction_view_uses_u256_checked_fees_and_strict_observations() {
    let server = TestServer::spawn(Mode::Valid).await;
    let session = session(&server).await;
    let transaction_hash = B256::from([7; 32]);

    assert_eq!(
        session.pending_nonce(ACCOUNT).await.expect("nonce"),
        U256::from(7)
    );
    let fees = session.fee_inputs().await.expect("fees");
    assert_eq!(fees.base_fee_per_gas, U256::from(32));
    assert_eq!(fees.max_priority_fee_per_gas, U256::from(2));
    assert_eq!(fees.max_fee_per_gas, U256::from(66));
    let transaction = session
        .transaction_by_hash(transaction_hash)
        .await
        .expect("transaction")
        .expect("present transaction");
    assert_eq!(transaction.transaction_hash, transaction_hash);
    assert_eq!(transaction.chain_id, U256::from(1));
    assert_eq!(transaction.to, TxKind::Call(ACCOUNT));
    assert_eq!(
        transaction
            .placement
            .expect("placement")
            .block
            .number_quantity()
            .expect("block number"),
        U256::from(42)
    );
    let receipt = session
        .receipt_by_hash(transaction_hash)
        .await
        .expect("receipt")
        .expect("present receipt");
    assert_eq!(receipt.status, EvmReceiptStatus::Success);
    assert_eq!(receipt.logs.len(), 1);
    receipt.validate().expect("coherent receipt");
}

#[tokio::test]
async fn gas_estimation_sends_complete_call_and_create_type_two_descriptions() {
    let server = TestServer::spawn(Mode::Valid).await;
    let session = session(&server).await;
    let storage_key = B256::from([0x33; 32]);
    let call = EvmTransactionEstimate::new(
        U256::from(u64::MAX),
        U256::from(u64::MAX),
        ACCOUNT,
        TxKind::Call(Address::from([2; 20])),
        U256::MAX,
        Bytes::from_static(&[0xaa, 0xbb]),
        AccessList(vec![AccessListItem {
            address: Address::from([3; 20]),
            storage_keys: vec![storage_key],
        }]),
        U256::from(u128::MAX),
        U256::from(u128::MAX - 1),
    )
    .expect("call estimate");
    let create = EvmTransactionEstimate::new(
        U256::from(1),
        U256::from(8),
        ACCOUNT,
        TxKind::Create,
        U256::ZERO,
        Bytes::from_static(&[0x60, 0x00]),
        AccessList::default(),
        U256::from(66),
        U256::from(2),
    )
    .expect("create estimate");

    assert_eq!(
        session.estimate_gas(&call).await.expect("call gas"),
        U256::from(21_000)
    );
    assert_eq!(
        session.estimate_gas(&create).await.expect("create gas"),
        U256::from(21_000)
    );

    let requests = server
        .requests()
        .into_iter()
        .filter(|request| request["method"] == "eth_estimateGas")
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0]["params"],
        json!([{
            "type": "0x2",
            "chainId": "0xffffffffffffffff",
            "nonce": "0xffffffffffffffff",
            "from": format!("{ACCOUNT:#x}"),
            "to": format!("{:#x}", Address::from([2; 20])),
            "value": format!("0x{:x}", U256::MAX),
            "data": "0xaabb",
            "accessList": [{
                "address": format!("{:#x}", Address::from([3; 20])),
                "storageKeys": [format!("{storage_key:#x}")],
            }],
            "maxFeePerGas": "0xffffffffffffffffffffffffffffffff",
            "maxPriorityFeePerGas": "0xfffffffffffffffffffffffffffffffe",
        }, "pending"])
    );
    assert_eq!(
        requests[1]["params"],
        json!([{
            "type": "0x2",
            "chainId": "0x1",
            "nonce": "0x8",
            "from": format!("{ACCOUNT:#x}"),
            "value": "0x0",
            "data": "0x6000",
            "accessList": [],
            "maxFeePerGas": "0x42",
            "maxPriorityFeePerGas": "0x2",
        }, "pending"])
    );
    assert!(requests[1]["params"][0].get("to").is_none());
}

#[tokio::test]
async fn transaction_submit_preserves_a_wrong_provider_hash_for_adapter_ambiguity() {
    let server = TestServer::spawn(Mode::WrongSubmitHash).await;
    let session = session(&server).await;
    let signed_bytes = [0x01, 0x02, 0x03];
    let expected_hash = keccak256(signed_bytes);

    let returned_hash = session
        .submit_raw_transaction(&signed_bytes, expected_hash)
        .await
        .expect("well-formed provider hash");

    assert_eq!(returned_hash, OTHER_HASH.parse::<B256>().expect("hash"));
    assert_ne!(returned_hash, expected_hash);
}

#[tokio::test]
async fn one_submission_capability_call_owns_one_http_exchange() {
    let server = TestServer::spawn(Mode::SubmitHttpFailure).await;
    let session = session(&server).await;
    let signed_bytes = [0x01, 0x02, 0x03];
    let expected_hash = keccak256(signed_bytes);

    session
        .submit_raw_transaction(&signed_bytes, expected_hash)
        .await
        .expect_err("HTTP failure must be returned without an implicit retry");

    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|request| request["method"] == "eth_sendRawTransaction")
            .count(),
        1
    );
}

#[tokio::test]
async fn sessions_share_the_process_local_source_concurrency_bound() {
    let server = TestServer::spawn(Mode::SlowBalance).await;
    let transport = transport();
    let first = transport
        .bind(
            binding(1),
            LocalPublicId::new("primary").expect("source"),
            endpoint(&server.url),
            None,
        )
        .await
        .expect("first session");
    let second = transport
        .bind(
            binding(1),
            LocalPublicId::new("primary").expect("source"),
            endpoint(&server.url),
            None,
        )
        .await
        .expect("second session");
    let selector = EvmBlockSelector::ExactHash(BLOCK_HASH.parse().expect("hash"));
    let mut reads = tokio::task::JoinSet::new();
    for index in 0..(MAX_IN_FLIGHT_EXCHANGES_PER_SOURCE * 3) {
        let session = if index.is_multiple_of(2) {
            first.clone()
        } else {
            second.clone()
        };
        let selector = selector.clone();
        reads.spawn(async move { session.read_balance(ACCOUNT, &selector).await });
    }
    while let Some(result) = reads.join_next().await {
        result.expect("read task").expect("balance read");
    }

    let max_active = server.max_active.load(Ordering::SeqCst);
    assert!(max_active > 1, "test must exercise concurrent exchanges");
    assert!(
        max_active <= MAX_IN_FLIGHT_EXCHANGES_PER_SOURCE,
        "shared source bound was exceeded: {max_active}"
    );
}

#[tokio::test]
async fn redirects_are_rejected_without_contacting_or_authorizing_the_target() {
    let fixture = RedirectFixture::spawn().await;
    let error = transport()
        .bind(
            binding(1),
            LocalPublicId::new("primary").expect("source"),
            endpoint(&fixture.origin_url),
            Some(authorization("Bearer top-secret")),
        )
        .await
        .expect_err("redirect response must fail binding");

    assert!(matches!(
        error,
        EvmTransportError::RpcHttpStatus { status: 307, .. }
    ));
    let origin_requests = fixture.origin_requests.lock().expect("origin requests");
    assert_eq!(origin_requests.len(), 1);
    assert!(
        origin_requests[0]
            .to_ascii_lowercase()
            .contains("authorization: bearer top-secret"),
        "origin must receive its configured authorization"
    );
    assert!(
        fixture
            .target_requests
            .lock()
            .expect("target requests")
            .is_empty(),
        "redirect target must receive neither a request nor authorization"
    );
}

#[tokio::test]
async fn null_observations_are_absence_not_fabricated_receipts() {
    let server = TestServer::spawn(Mode::ReceiptNull).await;
    let session = session(&server).await;
    assert_eq!(
        session
            .receipt_by_hash(B256::from([7; 32]))
            .await
            .expect("lookup"),
        None
    );
}

#[tokio::test]
async fn removed_receipt_logs_fail_closed() {
    let server = TestServer::spawn(Mode::ReceiptRemoved).await;
    let session = session(&server).await;
    let error = session
        .receipt_by_hash(B256::from([7; 32]))
        .await
        .expect_err("removed log");
    assert_eq!(
        error.redacted_diagnostic().expect("diagnostic").code(),
        ProviderDiagnosticCode::ResponseInvalid
    );
}

#[tokio::test]
async fn missing_eip1559_fee_input_fails_closed() {
    let server = TestServer::spawn(Mode::FeeMissing).await;
    let session = session(&server).await;
    assert!(session.fee_inputs().await.is_err());
}

#[tokio::test]
async fn bind_rejects_chain_mismatch_and_redacts_endpoint() {
    let server = TestServer::spawn(Mode::ChainMismatch).await;
    let error = transport()
        .bind(
            binding(1),
            LocalPublicId::new("primary").expect("source"),
            endpoint(&server.url),
            Some(authorization("Bearer top-secret")),
        )
        .await
        .expect_err("chain mismatch");
    let rendered = format!("{error:?} {error}");
    let EvmTransportError::SourceMismatch { diagnostic } = error else {
        panic!("expected source mismatch");
    };
    assert_eq!(diagnostic.stable_error_code(), "evm_source_mismatch");
    assert!(!rendered.contains(&server.url));
    assert!(!rendered.contains("top-secret"));
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "eth_chainId");
}

#[tokio::test]
async fn oversized_bind_response_fails_before_body_read() {
    let server = TestServer::spawn(Mode::Oversized).await;
    let error = transport()
        .bind(
            binding(1),
            LocalPublicId::new("primary").expect("source"),
            endpoint(&server.url),
            None,
        )
        .await
        .expect_err("oversized response");

    assert_eq!(error, EvmTransportError::ResponseTooLarge);
}

#[tokio::test]
async fn chunked_bind_response_without_content_length_obeys_body_bound() {
    let server = TestServer::spawn(Mode::OversizedChunked).await;
    let error = transport()
        .bind(
            binding(1),
            LocalPublicId::new("primary").expect("source"),
            endpoint(&server.url),
            None,
        )
        .await
        .expect_err("oversized chunked response");

    assert_eq!(error, EvmTransportError::ResponseTooLarge);
}

#[tokio::test(start_paused = true)]
async fn stalled_bind_response_obeys_request_timeout() {
    let server = TestServer::spawn(Mode::Stall).await;
    let transport = transport();
    let url = server.url.clone();
    let bind = tokio::spawn(async move {
        transport
            .bind(
                binding(1),
                LocalPublicId::new("primary").expect("source"),
                endpoint(&url),
                None,
            )
            .await
    });
    while server.requests().is_empty() {
        tokio::task::yield_now().await;
    }

    tokio::time::advance(REQUEST_TIMEOUT + Duration::from_secs(1)).await;
    let error = bind
        .await
        .expect("bind task")
        .expect_err("stalled response must time out");
    assert!(matches!(error, EvmTransportError::TransportFailed { .. }));
}

#[tokio::test]
async fn response_version_and_id_are_strict() {
    for mode in [Mode::BadVersion, Mode::BadId] {
        let server = TestServer::spawn(mode).await;
        assert!(matches!(
            transport()
                .bind(
                    binding(1),
                    LocalPublicId::new("primary").expect("source"),
                    endpoint(&server.url),
                    None,
                )
                .await,
            Err(EvmTransportError::InvalidResponse)
        ));
    }
}

#[tokio::test]
async fn http_and_json_rpc_diagnostics_exclude_bodies() {
    for (mode, code) in [
        (Mode::HttpFailure, ProviderDiagnosticCode::RpcHttpStatus),
        (Mode::JsonError, ProviderDiagnosticCode::RpcJsonError),
        (
            Mode::AdversarialMalformed,
            ProviderDiagnosticCode::ResponseInvalid,
        ),
        (
            Mode::OversizedChunked,
            ProviderDiagnosticCode::ResponseInvalid,
        ),
    ] {
        let server = TestServer::spawn(mode).await;
        let endpoint_url = format!("{}/{FIXTURE_ENDPOINT_PATH}", server.url);
        let error = transport()
            .bind(
                binding(1),
                LocalPublicId::new("primary").expect("source"),
                endpoint(&endpoint_url),
                Some(authorization(FIXTURE_BEARER)),
            )
            .await
            .expect_err("bind failure");
        let error_rendered = format!("{error:?} {error}");
        let diagnostic = error.into_provider_diagnostic();
        let retained_json = serde_json::to_string(&diagnostic).expect("typed diagnostic JSON");
        let rendered = format!("{error_rendered} {diagnostic:?} {diagnostic} {retained_json}");
        assert_eq!(diagnostic.code(), code);
        assert_synthetic_material_absent(
            &rendered,
            &[
                &server.url,
                &endpoint_url,
                FIXTURE_ENDPOINT_PATH,
                FIXTURE_BEARER,
                FIXTURE_PROVIDER_BODY,
                FIXTURE_PROVIDER_URL,
                FIXTURE_LOW_ENTROPY,
                "123456",
            ],
        );
        let raw = server.raw_requests().join("\n");
        assert!(raw.contains(FIXTURE_ENDPOINT_PATH));
        assert!(raw
            .to_ascii_lowercase()
            .contains(&format!("authorization: {}", FIXTURE_BEARER).to_ascii_lowercase()));
    }
}

#[tokio::test]
async fn retained_session_evidence_excludes_endpoint_and_authorization_material() {
    let server = TestServer::spawn(Mode::Valid).await;
    let endpoint_url = format!("{}/{FIXTURE_ENDPOINT_PATH}", server.url);
    let session = transport()
        .bind(
            binding(1),
            LocalPublicId::new("primary").expect("source"),
            endpoint(&endpoint_url),
            Some(authorization(FIXTURE_BEARER)),
        )
        .await
        .expect("checked session");
    let evidence =
        serde_json::to_string(EvmReadSession::evidence(&session)).expect("retained evidence");
    let rendered = format!("{session:?} {evidence}");
    assert_synthetic_material_absent(
        &rendered,
        &[
            &server.url,
            &endpoint_url,
            FIXTURE_ENDPOINT_PATH,
            FIXTURE_BEARER,
            FIXTURE_LOW_ENTROPY,
            "123456",
        ],
    );
    let raw = server.raw_requests().join("\n");
    assert!(raw.contains(FIXTURE_ENDPOINT_PATH));
    assert!(raw
        .to_ascii_lowercase()
        .contains(&format!("authorization: {}", FIXTURE_BEARER).to_ascii_lowercase()));
}

#[test]
fn endpoint_rejection_does_not_retain_userinfo_query_fragment_or_low_entropy_tokens() {
    for rejected in [
        format!("http://fixture-user:fixture-password@127.0.0.1/{FIXTURE_LOW_ENTROPY}"),
        "http://127.0.0.1/?access_token=123456".to_owned(),
        format!("http://127.0.0.1/#{FIXTURE_LOW_ENTROPY}"),
    ] {
        let error = EvmRpcEndpoint::new(&rejected).expect_err("unsafe endpoint");
        let rendered = format!("{error:?} {error}");
        assert_synthetic_material_absent(
            &rendered,
            &[
                &rejected,
                "fixture-user",
                "fixture-password",
                FIXTURE_LOW_ENTROPY,
                "123456",
            ],
        );
    }
}

#[tokio::test]
async fn transport_failure_discards_the_internal_endpoint_error() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("reserve port");
    let address = listener.local_addr().expect("reserved address");
    drop(listener);
    let endpoint_url = format!("http://{address}/{FIXTURE_ENDPOINT_PATH}");
    let error = transport()
        .bind(
            binding(1),
            LocalPublicId::new("primary").expect("source"),
            endpoint(&endpoint_url),
            Some(authorization(FIXTURE_BEARER)),
        )
        .await
        .expect_err("connection must fail");
    let diagnostic = error.into_provider_diagnostic();
    let rendered = format!("{diagnostic:?} {diagnostic}");
    assert_eq!(diagnostic.code(), ProviderDiagnosticCode::TransportFailed);
    assert_synthetic_material_absent(
        &rendered,
        &[
            &endpoint_url,
            FIXTURE_ENDPOINT_PATH,
            FIXTURE_BEARER,
            FIXTURE_LOW_ENTROPY,
            "123456",
        ],
    );
}

#[test]
fn private_codecs_reject_noncanonical_quantities_and_data() {
    assert_eq!(parse_quantity("0x0").expect("zero"), U256::ZERO);
    assert_eq!(parse_quantity("0x2a").expect("quantity"), U256::from(42));
    for invalid in ["", "0X2a", "2a", "0x", "0x00", "0x01", "0xgg"] {
        assert_eq!(
            parse_quantity(invalid),
            Err(EvmTransportError::InvalidResponse)
        );
    }
    assert_eq!(parse_bytes("0x").expect("empty"), Bytes::new());
    assert_eq!(
        parse_bytes("0x00ff").expect("bytes"),
        Bytes::from_static(&[0, 255])
    );
    for invalid in ["", "0X00", "00", "0x0", "0xzz"] {
        assert_eq!(
            parse_bytes(invalid),
            Err(EvmTransportError::InvalidResponse)
        );
    }
}

fn response(mode: Mode, request: &Value) -> String {
    if matches!(mode, Mode::HttpFailure) {
        let body = format!(
            "{FIXTURE_PROVIDER_BODY} {FIXTURE_PROVIDER_URL} {FIXTURE_BEARER} {FIXTURE_LOW_ENTROPY}"
        );
        return format!(
            "HTTP/1.1 500 Internal Server Error\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
    }
    if matches!(mode, Mode::SubmitHttpFailure) && request["method"] == "eth_sendRawTransaction" {
        return "HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\n\r\n".to_owned();
    }
    if matches!(mode, Mode::Oversized) {
        return format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            MAX_RESPONSE_BYTES + 1
        );
    }
    if matches!(mode, Mode::OversizedChunked) {
        let marker = format!("{FIXTURE_PROVIDER_BODY}{FIXTURE_LOW_ENTROPY}");
        let body = marker.repeat(MAX_RESPONSE_BYTES / marker.len() + 1);
        return format!(
            "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            body.len(),
            body
        );
    }
    if matches!(mode, Mode::AdversarialMalformed) {
        let body = format!(
            "{FIXTURE_PROVIDER_BODY} {FIXTURE_PROVIDER_URL} {FIXTURE_BEARER} {FIXTURE_LOW_ENTROPY}"
        );
        return format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
    }
    let method = request["method"].as_str().expect("method");
    let body = if matches!(mode, Mode::JsonError) {
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32601,
                "message": format!(
                    "{FIXTURE_PROVIDER_BODY} {FIXTURE_PROVIDER_URL} {FIXTURE_LOW_ENTROPY}"
                ),
                "data": {
                    "authorization": FIXTURE_BEARER,
                    "nested": {"token": "123456"},
                },
            },
        })
    } else {
        let result = rpc_result(mode, request, method);
        json!({
            "jsonrpc": if matches!(mode, Mode::BadVersion) { "1.0" } else { "2.0" },
            "id": if matches!(mode, Mode::BadId) { 2 } else { 1 },
            "result": result,
        })
    }
    .to_string();
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(), body
    )
}

struct RedirectFixture {
    origin_url: String,
    origin_requests: Arc<Mutex<Vec<String>>>,
    target_requests: Arc<Mutex<Vec<String>>>,
}

impl RedirectFixture {
    async fn spawn() -> Self {
        let target_listener = TcpListener::bind("127.0.0.1:0").await.expect("target bind");
        let target_address = target_listener.local_addr().expect("target address");
        let target_requests = Arc::new(Mutex::new(Vec::new()));
        let captured_target = Arc::clone(&target_requests);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = target_listener.accept().await else {
                    return;
                };
                let captured_target = Arc::clone(&captured_target);
                tokio::spawn(async move {
                    let request = read_http_request(&mut stream).await;
                    captured_target
                        .lock()
                        .expect("target requests")
                        .push(request);
                    let body = json!({"jsonrpc": "2.0", "id": 1, "result": "0x1"}).to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                        body.len(), body
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("target response");
                });
            }
        });

        let origin_listener = TcpListener::bind("127.0.0.1:0").await.expect("origin bind");
        let origin_address = origin_listener.local_addr().expect("origin address");
        let origin_requests = Arc::new(Mutex::new(Vec::new()));
        let captured_origin = Arc::clone(&origin_requests);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = origin_listener.accept().await else {
                    return;
                };
                let captured_origin = Arc::clone(&captured_origin);
                tokio::spawn(async move {
                    let request = read_http_request(&mut stream).await;
                    captured_origin
                        .lock()
                        .expect("origin requests")
                        .push(request);
                    let response = format!(
                        "HTTP/1.1 307 Temporary Redirect\r\nlocation: http://{target_address}\r\ncontent-length: 0\r\n\r\n"
                    );
                    stream
                        .write_all(response.as_bytes())
                        .await
                        .expect("origin response");
                });
            }
        });

        Self {
            origin_url: format!("http://{origin_address}"),
            origin_requests,
            target_requests,
        }
    }
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> String {
    let mut bytes = vec![0; 16 * 1024];
    let mut read = 0;
    loop {
        let count = stream.read(&mut bytes[read..]).await.expect("request read");
        assert_ne!(count, 0, "request ended before the body was complete");
        read += count;
        if request_complete(&bytes[..read]) {
            return String::from_utf8(bytes[..read].to_vec()).expect("HTTP request is UTF-8");
        }
    }
}

fn rpc_result(mode: Mode, request: &Value, method: &str) -> Value {
    match method {
        "eth_chainId" => json!(if matches!(mode, Mode::ChainMismatch) {
            "0x2"
        } else {
            "0x1"
        }),
        "eth_getBlockByNumber" => {
            let mut block = json!({"number": "0x2a", "hash": BLOCK_HASH, "baseFeePerGas": "0x20"});
            if matches!(mode, Mode::FeeMissing) {
                block
                    .as_object_mut()
                    .expect("object")
                    .remove("baseFeePerGas");
            }
            block
        }
        "eth_getBlockByHash" => json!({"number": "0x2a", "hash": BLOCK_HASH}),
        "eth_getBalance" => json!("0xde0b6b3a7640000"),
        "eth_getCode" if matches!(mode, Mode::HostileCodeResult) => {
            let decoded_len = (MAX_RESPONSE_BYTES - 128) / 2;
            json!(format!("0x{}", "ab".repeat(decoded_len)))
        }
        "eth_getCode" => json!("0xdeadbeef"),
        "eth_call" if matches!(mode, Mode::HostileCallResult) => {
            let decoded_len = (MAX_RESPONSE_BYTES - 128) / 2;
            json!(format!("0x{}", "ab".repeat(decoded_len)))
        }
        "eth_call" => json!("0x1234"),
        "eth_getTransactionCount" => json!("0x7"),
        "eth_maxPriorityFeePerGas" => json!("0x2"),
        "eth_estimateGas" => json!("0x5208"),
        "eth_sendRawTransaction" if matches!(mode, Mode::WrongSubmitHash) => json!(OTHER_HASH),
        "eth_sendRawTransaction" => {
            let bytes =
                parse_bytes(request["params"][0].as_str().expect("raw bytes")).expect("bytes");
            json!(format!("{:#x}", keccak256(bytes)))
        }
        "eth_getTransactionByHash" => {
            let hash = request["params"][0].as_str().expect("hash");
            json!({
                "hash": hash,
                "type": "0x2",
                "chainId": "0x1",
                "nonce": "0x7",
                "from": format!("{ACCOUNT:#x}"),
                "to": format!("{ACCOUNT:#x}"),
                "value": "0x0",
                "input": "0x1234",
                "gas": "0x5208",
                "maxFeePerGas": "0x42",
                "maxPriorityFeePerGas": "0x2",
                "accessList": [],
                "blockNumber": "0x2a",
                "blockHash": BLOCK_HASH,
                "transactionIndex": "0x3"
            })
        }
        "eth_getTransactionReceipt" if matches!(mode, Mode::ReceiptNull) => Value::Null,
        "eth_getTransactionReceipt" => {
            let hash = request["params"][0].as_str().expect("hash");
            json!({
                "transactionHash": hash,
                "transactionIndex": "0x3",
                "blockNumber": "0x2a",
                "blockHash": BLOCK_HASH,
                "from": format!("{ACCOUNT:#x}"),
                "to": format!("{ACCOUNT:#x}"),
                "contractAddress": null,
                "status": "0x1",
                "gasUsed": "0x5208",
                "cumulativeGasUsed": "0x5208",
                "logs": [{
                    "address": format!("{ACCOUNT:#x}"),
                    "topics": [OTHER_HASH],
                    "data": "0x1234",
                    "blockNumber": "0x2a",
                    "blockHash": BLOCK_HASH,
                    "transactionHash": hash,
                    "transactionIndex": "0x3",
                    "logIndex": "0x0",
                    "removed": matches!(mode, Mode::ReceiptRemoved),
                }]
            })
        }
        other => panic!("unexpected method {other}"),
    }
}

fn request_complete(bytes: &[u8]) -> bool {
    let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_len = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or_default();
    bytes.len() >= header_end + 4 + content_len
}

fn request_body(bytes: &[u8]) -> &[u8] {
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("headers");
    &bytes[header_end + 4..]
}
