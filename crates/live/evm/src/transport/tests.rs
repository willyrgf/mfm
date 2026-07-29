use std::sync::{Arc, Mutex};

use super::EvmTransportOutcome;
use alloy_primitives::{Address, B256, U256};
use mfm_evm::{
    EvmAnchorConfirmationRequest, EvmAnchoredSource, EvmBlockAnchor, EvmChainIdentityRequest,
    EvmCheckedSource, EvmLatestAnchorRequest, EvmNativeBalanceRequest, EvmNetworkBinding,
    EvmRoutingGenerationRef, EvmSafeFailure, EvmTokenBalanceRequest, EvmTokenDecimalsRequest,
    EVM_READ_MAX_RESPONSE_BYTES,
};
use mfm_ids::StableId;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

const ACCOUNT: Address = Address::new([0x11; 20]);
const TOKEN: Address = Address::new([0x22; 20]);
const BLOCK_HASH: B256 = B256::new([0x33; 32]);

struct TestResponse {
    status: u16,
    body: Vec<u8>,
    declared_length: Option<usize>,
    omit_content_length: bool,
    location: Option<String>,
}

impl TestResponse {
    fn json(value: Value) -> Self {
        Self {
            status: 200,
            body: serde_json::to_vec(&value).expect("response JSON"),
            declared_length: None,
            omit_content_length: false,
            location: None,
        }
    }

    fn status(status: u16) -> Self {
        Self {
            status,
            body: Vec::new(),
            declared_length: None,
            omit_content_length: false,
            location: None,
        }
    }
}

struct TestServer {
    endpoint: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}

impl TestServer {
    async fn start(responses: Vec<TestResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind server");
        let endpoint = format!("http://{}", listener.local_addr().expect("server address"));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) = listener.accept().await.expect("accept request");
                let body = read_http_body(&mut socket).await;
                captured
                    .lock()
                    .expect("capture lock")
                    .push(serde_json::from_slice(&body).expect("request JSON"));
                let reason = if response.status == 200 { "OK" } else { "TEST" };
                let length = response.declared_length.unwrap_or(response.body.len());
                let length_header = if response.omit_content_length {
                    String::new()
                } else {
                    format!("content-length: {length}\r\n")
                };
                let location = response
                    .location
                    .as_deref()
                    .map(|value| format!("location: {value}\r\n"))
                    .unwrap_or_default();
                let head = format!(
                    "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\n{}{}connection: close\r\n\r\n",
                    response.status, reason, length_header, location
                );
                socket
                    .write_all(head.as_bytes())
                    .await
                    .expect("write response head");
                let _ = socket.write_all(&response.body).await;
            }
        });
        Self {
            endpoint,
            requests,
            task,
        }
    }

    async fn finish(self) -> Vec<Value> {
        self.task.await.expect("server task");
        Arc::try_unwrap(self.requests)
            .expect("request references")
            .into_inner()
            .expect("request lock")
    }
}

async fn read_http_body(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 1024];
        let count = socket.read(&mut chunk).await.expect("read request");
        assert!(count > 0, "request ended before headers");
        bytes.extend_from_slice(&chunk[..count]);
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
    while bytes.len() < header_end + length {
        let mut chunk = [0_u8; 1024];
        let count = socket.read(&mut chunk).await.expect("read body");
        assert!(count > 0, "request body ended early");
        bytes.extend_from_slice(&chunk[..count]);
    }
    bytes[header_end..header_end + length].to_vec()
}

fn generation_id(raw: &str) -> StableId {
    StableId::new(raw).expect("generation id")
}

fn transport(endpoint: &str) -> (EvmJsonRpcTransport, EvmNetworkBinding) {
    let mut routes = EvmRoutingCatalogBuilder::new();
    let generation = routes
        .insert(
            "ethereum-mainnet",
            "mfm.evm.json-rpc",
            1,
            generation_id("generation-a"),
            EvmRpcEndpoint::new(endpoint).expect("endpoint"),
            None,
        )
        .expect("route");
    (
        EvmJsonRpcTransport::new(routes.build().expect("catalog")).expect("transport"),
        EvmNetworkBinding::new("ethereum-mainnet", 1, generation).expect("binding"),
    )
}

fn generation_ref(id: &str) -> EvmRoutingGenerationRef {
    let mut routes = EvmRoutingCatalogBuilder::new();
    routes
        .insert(
            "ethereum-mainnet",
            "mfm.evm.json-rpc",
            1,
            generation_id(id),
            EvmRpcEndpoint::new("http://127.0.0.1:9").expect("endpoint"),
            None,
        )
        .expect("route")
}

fn rpc_result(result: Value) -> TestResponse {
    TestResponse::json(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": result,
    }))
}

#[test]
fn authorization_and_zeroizing_owner_bounds_are_exact() {
    for invalid in [
        Zeroizing::new(String::new()),
        Zeroizing::new("invalid\nheader".to_owned()),
        Zeroizing::new("a".repeat(MAX_AUTHORIZATION_BYTES + 1)),
    ] {
        assert_eq!(
            EvmRpcAuthorization::new(invalid).err(),
            Some(EvmTransportError::InvalidConfiguration)
        );
    }

    let exact = Zeroizing::new("a".repeat(MAX_AUTHORIZATION_BYTES));
    let original = exact.as_ptr();
    let authorization = EvmRpcAuthorization::new(exact).expect("exact-bound authorization");
    assert!(authorization.value.is_sensitive());
    assert_eq!(
        authorization.value.as_bytes().len(),
        MAX_AUTHORIZATION_BYTES
    );
    assert_eq!(authorization.value.as_bytes().as_ptr(), original);
    let authorization_clone = authorization.value.clone();
    assert!(authorization_clone.is_sensitive());
    assert_eq!(authorization_clone.as_bytes().as_ptr(), original);

    let probe = Arc::new(ZeroizingOwnerDropProbe::default());
    let payload = Zeroizing::new(vec![0x5a; 1024]);
    let original = payload.as_ptr();
    let bytes = Bytes::from_owner(ZeroizingBytesOwner::with_probe(payload, Arc::clone(&probe)));
    assert_eq!(bytes.as_ptr(), original);
    let final_owner = bytes.clone();
    drop(bytes);
    assert!(!probe.dropped.load(std::sync::atomic::Ordering::SeqCst));
    drop(final_owner);
    assert!(probe.dropped.load(std::sync::atomic::Ordering::SeqCst));
    assert!(probe.zeroized.load(std::sync::atomic::Ordering::SeqCst));

    let response_probe = Arc::new(ZeroizingOwnerDropProbe::default());
    let mut response = ZeroizingResponseBuffer::with_probe(Arc::clone(&response_probe));
    assert_eq!(response.bytes.capacity(), MAX_RESPONSE_BYTES);
    let response_allocation = response.bytes.as_ptr();
    let exact_response = vec![0x5a; MAX_RESPONSE_BYTES];
    assert_eq!(response.extend(&exact_response), Ok(()));
    assert_eq!(response.bytes.as_ptr(), response_allocation);
    assert_eq!(response.extend(&[0x5a]), Err(MAX_RESPONSE_BYTES + 1));
    assert_eq!(response.len(), MAX_RESPONSE_BYTES);
    assert_eq!(response.bytes.as_ptr(), response_allocation);
    drop(response);
    assert!(response_probe
        .dropped
        .load(std::sync::atomic::Ordering::SeqCst));
    assert!(response_probe
        .zeroized
        .load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn exact_request_owner_moves_unchanged_into_http_body_and_zeroizes_on_drop() {
    let encoded = ExactRpcRequest::ChainIdentity
        .encode()
        .expect("allowlisted request");
    let pointer = encoded.body.as_ptr();
    let length = encoded.body.len();
    let probe = Arc::new(ZeroizingOwnerDropProbe::default());
    let body = request_body_from_owner(ZeroizingBytesOwner::with_probe(
        encoded.body,
        Arc::clone(&probe),
    ));
    let body_bytes = body.as_bytes().expect("in-memory request body");
    assert_eq!(body_bytes.as_ptr(), pointer);
    assert_eq!(body_bytes.len(), length);
    assert!(!probe.dropped.load(std::sync::atomic::Ordering::SeqCst));
    drop(body);
    assert!(probe.dropped.load(std::sync::atomic::Ordering::SeqCst));
    assert!(probe.zeroized.load(std::sync::atomic::Ordering::SeqCst));
}

fn assert_request_owner_wiped(probe: &ExchangeOwnerProbe) {
    assert_eq!(probe.request_created.load(Ordering::SeqCst), 1);
    assert_ne!(probe.request_pointer.load(Ordering::SeqCst), 0);
    assert_ne!(probe.request_length.load(Ordering::SeqCst), 0);
    assert_eq!(probe.request_dropped.load(Ordering::SeqCst), 1);
    assert_eq!(probe.request_zeroized.load(Ordering::SeqCst), 1);
}

fn assert_response_owner_wiped(probe: &ExchangeOwnerProbe) {
    assert_eq!(probe.response_created.load(Ordering::SeqCst), 1);
    assert_eq!(
        probe.response_capacity.load(Ordering::SeqCst),
        MAX_RESPONSE_BYTES
    );
    assert_eq!(probe.response_dropped.load(Ordering::SeqCst), 1);
    assert_eq!(probe.response_zeroized.load(Ordering::SeqCst), 1);
}

async fn observed_chain_identity(
    response: TestResponse,
) -> (
    EvmTransportOutcome<EvmChainIdentityResponse>,
    Arc<ExchangeOwnerProbe>,
) {
    let server = TestServer::start(vec![response]).await;
    let (transport, binding) = transport(&server.endpoint);
    let probe = Arc::new(ExchangeOwnerProbe::default());
    let outcome = with_exchange_owner_probe(
        Arc::clone(&probe),
        transport.chain_identity(&EvmChainIdentityRequest::new(binding)),
    )
    .await;
    assert_eq!(server.finish().await.len(), 1);
    (outcome, probe)
}

#[tokio::test]
async fn production_exchange_owners_are_used_and_wiped_on_success_and_failure_exits() {
    let (outcome, probe) = observed_chain_identity(rpc_result(json!("0x1"))).await;
    assert!(matches!(outcome, EvmTransportOutcome::Returned(_)));
    assert_request_owner_wiped(&probe);
    assert_response_owner_wiped(&probe);

    let (outcome, probe) = observed_chain_identity(TestResponse::status(503)).await;
    assert_eq!(
        outcome,
        EvmTransportOutcome::Indeterminate(EvmSafeFailure::HttpStatus { status: 503 })
    );
    assert_request_owner_wiped(&probe);
    assert_eq!(probe.response_created.load(Ordering::SeqCst), 0);

    let malformed = TestResponse {
        status: 200,
        body: br#"{"jsonrpc":"2.0","id":1,"result":"0x1""#.to_vec(),
        declared_length: None,
        omit_content_length: false,
        location: None,
    };
    let (outcome, probe) = observed_chain_identity(malformed).await;
    assert!(matches!(
        outcome,
        EvmTransportOutcome::Indeterminate(EvmSafeFailure::ResponseInvalid {
            response_kind: EvmResponseInvalidKind::MalformedEnvelope,
            ..
        })
    ));
    assert_request_owner_wiped(&probe);
    assert_response_owner_wiped(&probe);

    let mut exact = br#"{"jsonrpc":"2.0","id":1,"result":"0x1"}"#.to_vec();
    exact.resize(MAX_RESPONSE_BYTES, b' ');
    let (outcome, probe) = observed_chain_identity(TestResponse {
        status: 200,
        body: exact,
        declared_length: None,
        omit_content_length: true,
        location: None,
    })
    .await;
    assert!(matches!(outcome, EvmTransportOutcome::Returned(_)));
    assert_request_owner_wiped(&probe);
    assert_response_owner_wiped(&probe);

    let mut overrun = br#"{"jsonrpc":"2.0","id":1,"result":"0x1"}"#.to_vec();
    overrun.resize(MAX_RESPONSE_BYTES + 1, b' ');
    let (outcome, probe) = observed_chain_identity(TestResponse {
        status: 200,
        body: overrun,
        declared_length: None,
        omit_content_length: true,
        location: None,
    })
    .await;
    assert!(matches!(
        outcome,
        EvmTransportOutcome::Indeterminate(EvmSafeFailure::ResponseTooLarge { .. })
    ));
    assert_request_owner_wiped(&probe);
    assert_response_owner_wiped(&probe);

    let incomplete_body = br#"{"jsonrpc":"2.0","id":1,"result":"0x1"}"#.to_vec();
    let (outcome, probe) = observed_chain_identity(TestResponse {
        status: 200,
        declared_length: Some(incomplete_body.len() + 1),
        body: incomplete_body,
        omit_content_length: false,
        location: None,
    })
    .await;
    assert_eq!(
        outcome,
        EvmTransportOutcome::Indeterminate(EvmSafeFailure::TransportFailed)
    );
    assert_request_owner_wiped(&probe);
    assert_response_owner_wiped(&probe);

    let (transport, binding) = transport("http://127.0.0.1:9");
    let probe = Arc::new(ExchangeOwnerProbe::default());
    let outcome = with_exchange_owner_probe(
        Arc::clone(&probe),
        transport.chain_identity(&EvmChainIdentityRequest::new(binding)),
    )
    .await;
    assert_eq!(
        outcome,
        EvmTransportOutcome::Indeterminate(EvmSafeFailure::TransportFailed)
    );
    assert_request_owner_wiped(&probe);
    assert_eq!(probe.response_created.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancelling_production_exchange_after_response_owner_creation_wipes_both_owners() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stalling server");
    let endpoint = format!(
        "http://{}",
        listener.local_addr().expect("stalling server address")
    );
    let (headers_sent, headers_observed) = tokio::sync::oneshot::channel();
    let (release_server, server_released) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept request");
        let body = read_http_body(&mut socket).await;
        let request: Value = serde_json::from_slice(&body).expect("request JSON");
        assert_eq!(request["method"], "eth_chainId");
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 1\r\nconnection: close\r\n\r\n",
            )
            .await
            .expect("write response headers");
        socket.flush().await.expect("flush response headers");
        headers_sent.send(()).expect("observe response headers");
        let _ = server_released.await;
    });

    let (transport, binding) = transport(&endpoint);
    let request = EvmChainIdentityRequest::new(binding);
    let probe = Arc::new(ExchangeOwnerProbe::default());
    let scoped_probe = Arc::clone(&probe);
    let exchange = tokio::spawn(async move {
        with_exchange_owner_probe(scoped_probe, async move {
            transport.chain_identity(&request).await
        })
        .await
    });

    tokio::time::timeout(std::time::Duration::from_secs(5), headers_observed)
        .await
        .expect("response headers timeout")
        .expect("response headers signal");
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while probe.response_created.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("response owner creation timeout");
    assert_eq!(probe.request_created.load(Ordering::SeqCst), 1);
    assert!(!exchange.is_finished());

    exchange.abort();
    assert!(exchange
        .await
        .expect_err("cancelled exchange")
        .is_cancelled());
    assert_request_owner_wiped(&probe);
    assert_response_owner_wiped(&probe);

    release_server.send(()).expect("release stalling server");
    server.await.expect("stalling server task");
}

#[test]
fn transport_source_has_no_generic_json_request_path() {
    let transport_source = include_str!("mod.rs");
    for removed in [".json(", "RpcPayload", "rpc_call(", "exchange_with_encoder"] {
        assert!(
            !transport_source.contains(removed),
            "generic JSON request path survived: {removed}"
        );
    }
}

#[tokio::test]
async fn six_typed_methods_issue_exactly_one_protocol_call_each() {
    let block = json!({
        "number": "0x2a",
        "hash": format!("{BLOCK_HASH:#x}"),
    });
    let server = TestServer::start(vec![
        rpc_result(json!("0x1")),
        rpc_result(block.clone()),
        rpc_result(json!("0x2")),
        rpc_result(json!(format!("0x{:064x}", 6))),
        rpc_result(json!(format!("0x{:064x}", 3))),
        rpc_result(block),
    ])
    .await;
    let (transport, binding) = transport(&server.endpoint);

    assert!(matches!(
        transport
            .chain_identity(&EvmChainIdentityRequest::new(binding.clone()))
            .await,
        EvmTransportOutcome::Returned(response)
            if response.chain_id == 1
    ));
    let checked = EvmCheckedSource::new(binding, "mfm.evm.json-rpc", JSON_RPC_IMPLEMENTATION_ID)
        .expect("checked source");
    assert!(matches!(
        transport
            .latest_anchor(&EvmLatestAnchorRequest::new(checked.clone()))
            .await,
        EvmTransportOutcome::Returned(_)
    ));
    let anchored = EvmAnchoredSource::new(checked, EvmBlockAnchor::new(U256::from(42), BLOCK_HASH))
        .expect("anchor");
    assert!(matches!(
        transport
            .native_balance(&EvmNativeBalanceRequest::new(
                anchored.clone(),
                ACCOUNT,
            ))
            .await,
        EvmTransportOutcome::Returned(response)
            if response.quantity().expect("quantity") == U256::from(2)
    ));
    assert!(matches!(
        transport
            .token_decimals(&EvmTokenDecimalsRequest::new(
                anchored.clone(),
                TOKEN,
            ))
            .await,
        EvmTransportOutcome::Returned(response)
            if response.decimals == 6
    ));
    assert!(matches!(
        transport
            .token_balance(&EvmTokenBalanceRequest::new(
                anchored.clone(),
                ACCOUNT,
                TOKEN,
            ))
            .await,
        EvmTransportOutcome::Returned(response)
            if response.quantity().expect("quantity") == U256::from(3)
    ));
    assert!(matches!(
        transport
            .confirm_anchor(&EvmAnchorConfirmationRequest::new(anchored))
            .await,
        EvmTransportOutcome::Returned(_)
    ));

    let requests = server.finish().await;
    assert_eq!(requests.len(), 6);
    assert_eq!(
        requests
            .iter()
            .map(|request| request["method"].as_str().expect("method"))
            .collect::<Vec<_>>(),
        [
            "eth_chainId",
            "eth_getBlockByNumber",
            "eth_getBalance",
            "eth_call",
            "eth_call",
            "eth_getBlockByNumber",
        ]
    );
    assert_eq!(requests[0]["params"], json!([]));
    assert_eq!(requests[1]["params"], json!(["latest", false]));
    let exact_anchor = json!({
        "blockHash": format!("{BLOCK_HASH:#x}"),
        "requireCanonical": true,
    });
    assert_eq!(
        requests[2]["params"],
        json!([format!("{ACCOUNT:#x}"), exact_anchor.clone()])
    );
    assert_eq!(
        requests[3]["params"],
        json!([{
            "to": format!("{TOKEN:#x}"),
            "data": "0x313ce567",
        }, exact_anchor.clone()])
    );
    assert_eq!(
        requests[4]["params"],
        json!([{
            "to": format!("{TOKEN:#x}"),
            "data": format!(
                "0x70a08231{:0>64}",
                hex::encode(ACCOUNT.as_slice())
            ),
        }, exact_anchor])
    );
    assert_eq!(requests[5]["params"], json!(["0x2a", false]));
}

#[tokio::test]
async fn unavailable_or_mismatched_generation_never_enters_transport() {
    let (unavailable_transport, _) = transport("http://127.0.0.1:9");
    let missing =
        EvmNetworkBinding::new("ethereum-mainnet", 1, generation_ref("missing-generation"))
            .expect("binding");
    assert_eq!(
        unavailable_transport
            .chain_identity(&EvmChainIdentityRequest::new(missing))
            .await,
        EvmTransportOutcome::DidNotEnter(EvmSafeFailure::RoutingGenerationUnavailable)
    );

    let (transport, binding) = transport("http://127.0.0.1:9");
    let wrong_network = EvmNetworkBinding::new(
        "ethereum-sepolia",
        1,
        binding.routing_generation_ref().clone(),
    )
    .expect("binding");
    assert_eq!(
        transport
            .chain_identity(&EvmChainIdentityRequest::new(wrong_network))
            .await,
        EvmTransportOutcome::DidNotEnter(EvmSafeFailure::RequestInvalid)
    );
    let wrong_chain = EvmNetworkBinding::new(
        "ethereum-mainnet",
        2,
        binding.routing_generation_ref().clone(),
    )
    .expect("binding");
    assert_eq!(
        transport
            .chain_identity(&EvmChainIdentityRequest::new(wrong_chain))
            .await,
        EvmTransportOutcome::DidNotEnter(EvmSafeFailure::RequestInvalid)
    );
    let mismatched = EvmCheckedSource::new(binding, "different-source", JSON_RPC_IMPLEMENTATION_ID)
        .expect("source");
    assert_eq!(
        transport
            .latest_anchor(&EvmLatestAnchorRequest::new(mismatched))
            .await,
        EvmTransportOutcome::DidNotEnter(EvmSafeFailure::RequestInvalid)
    );
}

#[tokio::test]
async fn closed_semaphore_is_classified_as_pre_entry_cancellation() {
    let (transport, binding) = transport("http://127.0.0.1:9");
    transport
        .routes
        .resolve(binding.routing_generation_ref())
        .expect("route")
        .limit
        .close();
    assert_eq!(
        transport
            .chain_identity(&EvmChainIdentityRequest::new(binding))
            .await,
        EvmTransportOutcome::DidNotEnter(EvmSafeFailure::AccessCancelled)
    );
}

#[tokio::test]
async fn queued_exchange_does_not_encode_before_both_permits_and_wipes_sensitive_state() {
    let (route_blocked_transport, binding) = transport("http://127.0.0.1:9");
    let route = route_blocked_transport
        .routes
        .resolve(binding.routing_generation_ref())
        .expect("route");
    let held_route_permits = Arc::clone(&route.limit)
        .acquire_many_owned(
            u32::try_from(MAX_IN_FLIGHT_EXCHANGES_PER_GENERATION).expect("route permit count"),
        )
        .await
        .expect("hold route permits");
    let route_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let route_encode_probe = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let route_owner_probe = Arc::new(ExchangeOwnerProbe::default());
    let route_probe = Arc::new(ZeroizingOwnerDropProbe::default());
    let sensitive =
        ZeroizingBytesOwner::with_probe(Zeroizing::new(vec![0x5a; 1024]), Arc::clone(&route_probe));
    let queued_transport = route_blocked_transport.clone();
    let route_for_exchange = Arc::clone(&route);
    let called = Arc::clone(&route_called);
    let exchange = with_exchange_owner_probe(
        Arc::clone(&route_owner_probe),
        exact::with_encode_probe(Arc::clone(&route_encode_probe), async move {
            let outcome = queued_transport
                .exchange(route_for_exchange, ExactRpcRequest::ChainIdentity)
                .await;
            called.store(true, std::sync::atomic::Ordering::SeqCst);
            drop(sensitive);
            outcome
        }),
    );
    tokio::pin!(exchange);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut exchange)
            .await
            .is_err()
    );
    assert!(!route_called.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!route_encode_probe.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(route_owner_probe.request_created.load(Ordering::SeqCst), 0);
    assert_eq!(route_owner_probe.response_created.load(Ordering::SeqCst), 0);
    assert!(!route_probe
        .dropped
        .load(std::sync::atomic::Ordering::SeqCst));
    route.limit.close();
    assert!(matches!(
        exchange.await,
        Err(BoundaryFailure::DidNotEnter(
            EvmSafeFailure::AccessCancelled
        ))
    ));
    assert!(route_called.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!route_encode_probe.load(std::sync::atomic::Ordering::SeqCst));
    assert!(route_probe
        .dropped
        .load(std::sync::atomic::Ordering::SeqCst));
    assert!(route_probe
        .zeroized
        .load(std::sync::atomic::Ordering::SeqCst));
    drop(held_route_permits);

    let (global_blocked_transport, binding) = transport("http://127.0.0.1:9");
    let route = global_blocked_transport
        .routes
        .resolve(binding.routing_generation_ref())
        .expect("route");
    let held_global_permits = Arc::clone(&global_blocked_transport.shared.global_limit)
        .acquire_many_owned(
            u32::try_from(MAX_GLOBAL_IN_FLIGHT_EXCHANGES).expect("global permit count"),
        )
        .await
        .expect("hold global permits");
    let global_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let global_encode_probe = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let global_owner_probe = Arc::new(ExchangeOwnerProbe::default());
    let global_probe = Arc::new(ZeroizingOwnerDropProbe::default());
    let sensitive = ZeroizingBytesOwner::with_probe(
        Zeroizing::new(vec![0x5a; 1024]),
        Arc::clone(&global_probe),
    );
    let queued_transport = global_blocked_transport.clone();
    let route_for_exchange = Arc::clone(&route);
    let called = Arc::clone(&global_called);
    let exchange = with_exchange_owner_probe(
        Arc::clone(&global_owner_probe),
        exact::with_encode_probe(Arc::clone(&global_encode_probe), async move {
            let outcome = queued_transport
                .exchange(route_for_exchange, ExactRpcRequest::ChainIdentity)
                .await;
            called.store(true, std::sync::atomic::Ordering::SeqCst);
            drop(sensitive);
            outcome
        }),
    );
    tokio::pin!(exchange);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut exchange)
            .await
            .is_err()
    );
    assert_eq!(
        route.limit.available_permits(),
        MAX_IN_FLIGHT_EXCHANGES_PER_GENERATION - 1
    );
    assert!(!global_called.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!global_encode_probe.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(global_owner_probe.request_created.load(Ordering::SeqCst), 0);
    assert_eq!(
        global_owner_probe.response_created.load(Ordering::SeqCst),
        0
    );
    assert!(!global_probe
        .dropped
        .load(std::sync::atomic::Ordering::SeqCst));
    global_blocked_transport.shared.global_limit.close();
    assert!(matches!(
        exchange.await,
        Err(BoundaryFailure::DidNotEnter(
            EvmSafeFailure::AccessCancelled
        ))
    ));
    assert!(global_called.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!global_encode_probe.load(std::sync::atomic::Ordering::SeqCst));
    assert!(global_probe
        .dropped
        .load(std::sync::atomic::Ordering::SeqCst));
    assert!(global_probe
        .zeroized
        .load(std::sync::atomic::Ordering::SeqCst));
    drop(held_global_permits);
}

#[tokio::test]
async fn destination_and_invalid_response_failures_are_closed_and_single_call() {
    let cases = vec![
        (
            TestResponse::status(503),
            EvmSafeFailure::HttpStatus { status: 503 },
        ),
        (
            TestResponse::status(201),
            EvmSafeFailure::HttpStatus { status: 201 },
        ),
        (
            TestResponse::json(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "error": {"code": -32005, "message": "discard me"}
            })),
            EvmSafeFailure::JsonRpcError {
                json_rpc_code: -32005,
            },
        ),
        (
            TestResponse::json(json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": "0x1"
            })),
            EvmSafeFailure::ResponseInvalid {
                response_kind: EvmResponseInvalidKind::MalformedEnvelope,
                size_class: EvmCoarseSizeClass::UpTo16Kib,
            },
        ),
        (
            TestResponse::json(json!({"jsonrpc": "2.0", "id": 1})),
            EvmSafeFailure::ResponseMissingResult {
                size_class: EvmCoarseSizeClass::UpTo16Kib,
            },
        ),
    ];
    for (response, expected) in cases {
        let server = TestServer::start(vec![response]).await;
        let (transport, binding) = transport(&server.endpoint);
        assert_eq!(
            transport
                .chain_identity(&EvmChainIdentityRequest::new(binding))
                .await,
            EvmTransportOutcome::Indeterminate(expected)
        );
        assert_eq!(server.finish().await.len(), 1);
    }
}

#[tokio::test]
async fn redirects_and_oversized_results_are_not_retried_or_followed() {
    let redirect = TestServer::start(vec![TestResponse {
        status: 307,
        body: Vec::new(),
        declared_length: None,
        omit_content_length: false,
        location: Some("http://127.0.0.1:9/elsewhere".to_owned()),
    }])
    .await;
    let (redirect_transport, binding) = transport(&redirect.endpoint);
    assert_eq!(
        redirect_transport
            .chain_identity(&EvmChainIdentityRequest::new(binding))
            .await,
        EvmTransportOutcome::Indeterminate(EvmSafeFailure::HttpStatus { status: 307 })
    );
    assert_eq!(redirect.finish().await.len(), 1);

    let oversized = TestServer::start(vec![rpc_result(json!(
        "x".repeat(EVM_READ_MAX_RESPONSE_BYTES + 1)
    ))])
    .await;
    let (oversized_transport, binding) = transport(&oversized.endpoint);
    assert_eq!(
        oversized_transport
            .chain_identity(&EvmChainIdentityRequest::new(binding))
            .await,
        EvmTransportOutcome::Indeterminate(EvmSafeFailure::ResponseTooLarge {
            size_class: EvmCoarseSizeClass::UpTo1Mib,
        })
    );
    assert_eq!(oversized.finish().await.len(), 1);

    let declared_oversized = TestServer::start(vec![TestResponse {
        status: 200,
        body: Vec::new(),
        declared_length: Some(MAX_RESPONSE_BYTES + 1),
        omit_content_length: false,
        location: None,
    }])
    .await;
    let (declared_transport, binding) = transport(&declared_oversized.endpoint);
    assert_eq!(
        declared_transport
            .chain_identity(&EvmChainIdentityRequest::new(binding))
            .await,
        EvmTransportOutcome::Indeterminate(EvmSafeFailure::ResponseTooLarge {
            size_class: EvmCoarseSizeClass::Over1Mib,
        })
    );
    assert_eq!(declared_oversized.finish().await.len(), 1);

    let mut exact_body = br#"{"jsonrpc":"2.0","id":1,"result":"0x1"}"#.to_vec();
    exact_body.resize(MAX_RESPONSE_BYTES, b' ');
    let exact_outer = TestServer::start(vec![TestResponse {
        status: 200,
        body: exact_body,
        declared_length: None,
        omit_content_length: true,
        location: None,
    }])
    .await;
    let (exact_transport, binding) = transport(&exact_outer.endpoint);
    assert!(matches!(
        exact_transport
            .chain_identity(&EvmChainIdentityRequest::new(binding))
            .await,
        EvmTransportOutcome::Returned(response) if response.chain_id == 1
    ));
    assert_eq!(exact_outer.finish().await.len(), 1);

    let mut streamed_overrun_body = br#"{"jsonrpc":"2.0","id":1,"result":"0x1"}"#.to_vec();
    streamed_overrun_body.resize(MAX_RESPONSE_BYTES + 1, b' ');
    let streamed_overrun = TestServer::start(vec![TestResponse {
        status: 200,
        body: streamed_overrun_body,
        declared_length: None,
        omit_content_length: true,
        location: None,
    }])
    .await;
    let (streamed_transport, binding) = transport(&streamed_overrun.endpoint);
    assert_eq!(
        streamed_transport
            .chain_identity(&EvmChainIdentityRequest::new(binding))
            .await,
        EvmTransportOutcome::Indeterminate(EvmSafeFailure::ResponseTooLarge {
            size_class: EvmCoarseSizeClass::Over1Mib,
        })
    );
    assert_eq!(streamed_overrun.finish().await.len(), 1);
}

#[test]
fn route_catalog_is_exact_generation_only_and_rejects_duplicates() {
    let mut routes = EvmRoutingCatalogBuilder::new();
    let generation = routes
        .insert(
            "ethereum-mainnet",
            "mfm.evm.json-rpc",
            1,
            generation_id("generation"),
            EvmRpcEndpoint::new("https://rpc.example.invalid").expect("endpoint"),
            None,
        )
        .expect("first route");
    assert_eq!(
        routes
            .insert(
                "ethereum-mainnet",
                "mfm.evm.json-rpc",
                1,
                generation_id("generation"),
                EvmRpcEndpoint::new("https://rpc.example.invalid").expect("endpoint"),
                None,
            )
            .err(),
        Some(EvmTransportError::DuplicateGeneration)
    );
    let routes = routes.build().expect("catalog");
    assert_eq!(routes.len(), 1);
    assert!(!routes.is_empty());
    assert!(routes.contains(&generation));
    assert!(!format!("{routes:?}").contains("rpc.example"));
}

#[test]
fn route_descriptors_are_canonical_sorted_and_secret_free() {
    assert_eq!(
        EvmRoutingCatalogBuilder::new().build().err(),
        Some(EvmTransportError::EmptyCatalog)
    );

    let mut routes = EvmRoutingCatalogBuilder::new();
    let authorization = EvmRpcAuthorization::new(Zeroizing::new(format!(
        "Bearer route-test-{}",
        std::process::id()
    )))
    .expect("authorization");
    let second = routes
        .insert(
            "ethereum-sepolia",
            "mfm.evm.json-rpc",
            11_155_111,
            generation_id("generation-b"),
            EvmRpcEndpoint::new("https://second.rpc.example.invalid/private").expect("endpoint"),
            Some(authorization),
        )
        .expect("route");
    let first = routes
        .insert(
            "ethereum-mainnet",
            "mfm.evm.json-rpc",
            1,
            generation_id("generation-a"),
            EvmRpcEndpoint::new("https://first.rpc.example.invalid/private").expect("endpoint"),
            None,
        )
        .expect("route");
    let routes = routes.build().expect("catalog");
    let mut expected = vec![first, second];
    expected.sort();
    assert_eq!(routes.descriptor().ordered_generation_refs(), expected);

    let catalog_json = routes
        .descriptor()
        .canonical()
        .expect("catalog canonical JSON");
    let generations = routes
        .generation_descriptors()
        .map(|(_, descriptor)| {
            String::from_utf8(
                descriptor
                    .canonical()
                    .expect("generation canonical JSON")
                    .as_bytes()
                    .to_vec(),
            )
            .expect("UTF-8")
        })
        .collect::<Vec<_>>();
    let retained = format!(
        "{}{}",
        String::from_utf8(catalog_json.as_bytes().to_vec()).expect("UTF-8"),
        generations.join("")
    );
    for forbidden in [
        "route-test-",
        "rpc.example",
        "/private",
        "authorization",
        "endpoint",
    ] {
        assert!(!retained.contains(forbidden));
    }
    assert!(retained.contains(EVM_JSON_RPC_PROVIDER_CLASS));
    assert!(retained.contains(EVM_ROUTE_POLICY_ID));

    let generation = routes
        .generation_descriptors()
        .next()
        .expect("generation")
        .1;
    for (field, hostile) in [
        ("version", json!("mfm.evm.routing-generation.v2")),
        ("provider_class", json!("unreviewed-provider")),
        ("route_policy_id", json!("fallback-policy")),
        ("route_policy_version", json!("2")),
    ] {
        let mut wire = serde_json::to_value(generation).expect("descriptor JSON");
        wire[field] = hostile;
        assert!(
            serde_json::from_value::<EvmRoutingGenerationDescriptor>(wire).is_err(),
            "{field} must be exact"
        );
    }

    let mut catalog_wire = serde_json::to_value(routes.descriptor()).expect("catalog JSON");
    catalog_wire["ordered_generation_refs"]
        .as_array_mut()
        .expect("generation refs")
        .reverse();
    assert!(serde_json::from_value::<EvmRoutingCatalogDescriptor>(catalog_wire).is_err());
}
