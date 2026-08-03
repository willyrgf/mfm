use std::sync::{Arc, Mutex};

use super::EvmTransportOutcome;
use alloy_primitives::{Address, B256, U256};
use mfm_evm::{
    ChainInstanceDeclaration, ChainInstanceRegistryAttestation, EvmAnchorConfirmationRequest,
    EvmAnchoredSource, EvmBlockAnchor, EvmChainIdentityRequest, EvmCheckedSource,
    EvmLatestAnchorRequest, EvmNativeBalanceRequest, EvmNetworkBinding, EvmRoutingGenerationRef,
    EvmSafeFailure, EvmTokenBalanceRequest, EvmTokenDecimalsRequest, EvmWalletReference,
    EVM_READ_MAX_RESPONSE_BYTES,
};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, SchemaId, StableId};
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

fn reference(byte: u8) -> EvmWalletReference {
    let schema = SchemaId::new(
        "mfm.test.evm-live-reference",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        mfm_canonical::sha256_digest_bytes(b"EVM live test reference schema"),
    )
    .expect("reference schema");
    let digest = ContentDigest::from_digest(
        DigestAlgorithm::Sha256V1,
        mfm_canonical::sha256_digest_bytes(&[byte]),
    );
    EvmWalletReference::from_content_ref(ContentRef::new(schema, digest).expect("reference"))
}

fn chain_attestation(
    chain_id: u64,
    namespace: &str,
    registry_lineage_ref: &EvmWalletReference,
    registry_head_ref: &EvmWalletReference,
) -> ChainInstanceRegistryAttestation {
    ChainInstanceRegistryAttestation::new(
        ChainInstanceDeclaration::new(
            registry_lineage_ref.clone(),
            generation_id(namespace),
            chain_id,
            B256::repeat_byte(u8::try_from(chain_id % 250 + 1).expect("genesis byte")),
            U256::from(1_u64),
            B256::repeat_byte(u8::try_from(chain_id % 250 + 2).expect("anchor byte")),
        )
        .expect("chain declaration"),
        reference(u8::try_from(chain_id % 250 + 3).expect("issuance byte")),
        registry_head_ref.clone(),
    )
    .expect("chain attestation")
}

fn route_descriptor(
    network_id: &str,
    chain: &ChainInstanceRegistryAttestation,
    generation: &str,
) -> EvmRoutingGenerationDescriptor {
    EvmRoutingGenerationDescriptor::new(
        network_id,
        "mfm.evm.json-rpc",
        generation_id(generation),
        reference(u8::try_from(chain.declaration().chain_id() % 250 + 4).expect("membership byte")),
        chain.binding().expect("chain binding"),
    )
    .expect("route descriptor")
}

fn transport(endpoint: &str) -> (EvmJsonRpcTransport, EvmNetworkBinding) {
    let registry_lineage_ref = reference(240);
    let registry_head_ref = reference(239);
    let chain = chain_attestation(
        1,
        "mfm.test/mainnet-chain",
        &registry_lineage_ref,
        &registry_head_ref,
    );
    let binding = chain.binding().expect("chain binding");
    let mut routes = EvmRoutingCatalogBuilder::new(registry_head_ref, vec![chain.clone()])
        .expect("catalog builder");
    let generation = routes
        .insert(
            route_descriptor("ethereum-mainnet", &chain, "generation-a"),
            EvmRpcEndpoint::new(endpoint).expect("endpoint"),
            None,
        )
        .expect("route");
    (
        EvmJsonRpcTransport::new(routes.build().expect("catalog")).expect("transport"),
        EvmNetworkBinding::new("ethereum-mainnet", binding, generation).expect("binding"),
    )
}

fn generation_ref(id: &str) -> EvmRoutingGenerationRef {
    let registry_lineage_ref = reference(241);
    let registry_head_ref = reference(238);
    let chain = chain_attestation(
        1,
        "mfm.test/generation-ref-chain",
        &registry_lineage_ref,
        &registry_head_ref,
    );
    let mut routes = EvmRoutingCatalogBuilder::new(registry_head_ref, vec![chain.clone()])
        .expect("catalog builder");
    routes
        .insert(
            route_descriptor("ethereum-mainnet", &chain, id),
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
        EvmTransportOutcome::SafeFailure(EvmSafeFailure::HttpStatus { status: 503 })
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
        EvmTransportOutcome::SafeFailure(EvmSafeFailure::ResponseInvalid {
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
        EvmTransportOutcome::SafeFailure(EvmSafeFailure::ResponseTooLarge { .. })
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
        EvmTransportOutcome::SafeFailure(EvmSafeFailure::TransportFailed)
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
        EvmTransportOutcome::SafeFailure(EvmSafeFailure::TransportFailed)
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
    let (unavailable_transport, available_binding) = transport("http://127.0.0.1:9");
    let missing = EvmNetworkBinding::new(
        "ethereum-mainnet",
        available_binding.chain_instance().clone(),
        generation_ref("missing-generation"),
    )
    .expect("binding");
    assert_eq!(
        unavailable_transport
            .chain_identity(&EvmChainIdentityRequest::new(missing))
            .await,
        EvmTransportOutcome::SafeFailure(EvmSafeFailure::RoutingGenerationUnavailable)
    );

    let (transport, binding) = transport("http://127.0.0.1:9");
    let wrong_network = EvmNetworkBinding::new(
        "ethereum-sepolia",
        binding.chain_instance().clone(),
        binding.routing_generation_ref().clone(),
    )
    .expect("binding");
    assert_eq!(
        transport
            .chain_identity(&EvmChainIdentityRequest::new(wrong_network))
            .await,
        EvmTransportOutcome::SafeFailure(EvmSafeFailure::RequestInvalid)
    );
    let registry_lineage_ref = reference(240);
    let registry_head_ref = reference(239);
    let foreign_chain = chain_attestation(
        2,
        "mfm.test/foreign-chain",
        &registry_lineage_ref,
        &registry_head_ref,
    )
    .binding()
    .expect("foreign chain binding");
    let wrong_chain = EvmNetworkBinding::new(
        "ethereum-mainnet",
        foreign_chain,
        binding.routing_generation_ref().clone(),
    )
    .expect("binding");
    assert_eq!(
        transport
            .chain_identity(&EvmChainIdentityRequest::new(wrong_chain))
            .await,
        EvmTransportOutcome::SafeFailure(EvmSafeFailure::RequestInvalid)
    );
    let mismatched = EvmCheckedSource::new(binding, "different-source", JSON_RPC_IMPLEMENTATION_ID)
        .expect("source");
    assert_eq!(
        transport
            .latest_anchor(&EvmLatestAnchorRequest::new(mismatched))
            .await,
        EvmTransportOutcome::SafeFailure(EvmSafeFailure::RequestInvalid)
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
        EvmTransportOutcome::SafeFailure(EvmSafeFailure::AccessCancelled)
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
        Err(BoundaryFailure::BeforeEntry(
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
        Err(BoundaryFailure::BeforeEntry(
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
            EvmTransportOutcome::SafeFailure(expected)
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
        EvmTransportOutcome::SafeFailure(EvmSafeFailure::HttpStatus { status: 307 })
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
        EvmTransportOutcome::SafeFailure(EvmSafeFailure::ResponseTooLarge {
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
        EvmTransportOutcome::SafeFailure(EvmSafeFailure::ResponseTooLarge {
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
        EvmTransportOutcome::SafeFailure(EvmSafeFailure::ResponseTooLarge {
            size_class: EvmCoarseSizeClass::Over1Mib,
        })
    );
    assert_eq!(streamed_overrun.finish().await.len(), 1);
}

#[test]
fn route_catalog_is_exact_generation_only_and_rejects_duplicates() {
    let registry_lineage_ref = reference(242);
    let registry_head_ref = reference(237);
    let chain = chain_attestation(
        1,
        "mfm.test/duplicate-chain",
        &registry_lineage_ref,
        &registry_head_ref,
    );
    let descriptor = route_descriptor("ethereum-mainnet", &chain, "generation");
    let mut routes =
        EvmRoutingCatalogBuilder::new(registry_head_ref, vec![chain]).expect("catalog builder");
    let generation = routes
        .insert(
            descriptor.clone(),
            EvmRpcEndpoint::new("https://rpc.example.invalid").expect("endpoint"),
            None,
        )
        .expect("first route");
    assert_eq!(
        routes
            .insert(
                descriptor,
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
    let registry_lineage_ref = reference(243);
    let registry_head_ref = reference(236);
    let mainnet = chain_attestation(
        1,
        "mfm.test/canonical-mainnet",
        &registry_lineage_ref,
        &registry_head_ref,
    );
    assert_eq!(
        EvmRoutingCatalogBuilder::new(registry_head_ref.clone(), vec![mainnet.clone()])
            .expect("empty catalog builder")
            .build()
            .err(),
        Some(EvmTransportError::EmptyCatalog)
    );

    let sepolia = chain_attestation(
        11_155_111,
        "mfm.test/canonical-sepolia",
        &registry_lineage_ref,
        &registry_head_ref,
    );
    let mut routes =
        EvmRoutingCatalogBuilder::new(registry_head_ref, vec![mainnet.clone(), sepolia.clone()])
            .expect("catalog builder");
    let authorization = EvmRpcAuthorization::new(Zeroizing::new(format!(
        "Bearer route-test-{}",
        std::process::id()
    )))
    .expect("authorization");
    let second = routes
        .insert(
            route_descriptor("ethereum-sepolia", &sepolia, "generation-b"),
            EvmRpcEndpoint::new("https://second.rpc.example.invalid/private").expect("endpoint"),
            Some(authorization),
        )
        .expect("route");
    let first = routes
        .insert(
            route_descriptor("ethereum-mainnet", &mainnet, "generation-a"),
            EvmRpcEndpoint::new("https://first.rpc.example.invalid/private").expect("endpoint"),
            None,
        )
        .expect("route");
    let routes = routes.build().expect("catalog");
    let mut expected = vec![first, second];
    expected.sort();
    let actual = routes
        .descriptor()
        .generations()
        .iter()
        .map(|descriptor| descriptor.generation_ref().expect("generation ref"))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);

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
        ("version", json!("mfm.invalid.routing-generation.v1")),
        ("provider_class", json!("unreviewed-provider")),
        ("route_policy_id", json!("fallback-policy")),
        ("route_policy_version", json!("2")),
    ] {
        let mut wire = serde_json::to_value(generation).expect("descriptor JSON");
        wire[field] = hostile;
        let forged = serde_json::from_value::<EvmRoutingGenerationDescriptor>(wire)
            .expect("structural descriptor");
        assert!(forged.validate().is_err(), "{field} must be exact");
    }

    let mut catalog_wire = serde_json::to_value(routes.descriptor()).expect("catalog JSON");
    catalog_wire["generations"]
        .as_array_mut()
        .expect("generation refs")
        .reverse();
    let forged = serde_json::from_value::<EvmRoutingCatalogDescriptor>(catalog_wire)
        .expect("structural catalog");
    assert!(forged.validate().is_err());
}

fn inventory_challenge(
    generation_ref: ContentRef,
    ordinal: u8,
    finish_authorization: &[u8],
) -> EvmRpcInventoryChallenge {
    EvmRpcInventoryChallenge::new(
        generation_ref,
        EvmRpcTargetIdentity::new([ordinal.wrapping_add(10); 32]),
        EvmRpcAssemblyLease::new([0x41; 32]),
        EvmRpcInventoryCheckpoint::new([0x42; 32]),
        EvmRpcRouteChallenge::new([ordinal.wrapping_add(20); 32]),
        *mfm_canonical::sha256_digest_bytes(finish_authorization).as_bytes(),
    )
}

fn inventory_response(
    challenge: &EvmRpcInventoryChallenge,
    ordinal: u16,
    proof: &[u8],
) -> TestResponse {
    TestResponse::json(json!({
        "jsonrpc": "2.0",
        "id": ordinal,
        "result": {
            "protocol": "mfm.evm.rpc-inventory-qualification.v1",
            "ordinal": ordinal,
            "route_generation_ref": challenge.route_generation_ref(),
            "target_identity": hex::encode(challenge.target_identity().as_bytes()),
            "assembly_lease": hex::encode(challenge.assembly_lease().as_bytes()),
            "checkpoint": hex::encode(challenge.checkpoint().as_bytes()),
            "route_challenge": hex::encode(challenge.route_challenge().as_bytes()),
            "finish_authorization_commitment":
                hex::encode(challenge.finish_authorization_commitment()),
            "proof": hex::encode(proof),
        }
    }))
}

fn inventory_catalog(
    endpoint: &str,
    authorization: Option<EvmRpcAuthorization>,
) -> (EvmRoutingCatalog, ContentRef) {
    let registry_lineage_ref = reference(231);
    let registry_head_ref = reference(230);
    let chain = chain_attestation(
        1,
        "mfm.test/inventory-chain",
        &registry_lineage_ref,
        &registry_head_ref,
    );
    let descriptor = route_descriptor("ethereum-mainnet", &chain, "inventory-generation");
    let mut builder = EvmRoutingCatalogBuilder::new(registry_head_ref, vec![chain])
        .expect("inventory catalog builder");
    let generation = builder
        .insert(
            descriptor,
            EvmRpcEndpoint::new(endpoint).expect("inventory endpoint"),
            authorization,
        )
        .expect("inventory route");
    (
        builder.build().expect("inventory catalog"),
        generation
            .to_content_ref()
            .expect("inventory generation ref"),
    )
}

fn two_route_inventory_catalog() -> (EvmRoutingCatalog, Vec<ContentRef>) {
    let registry_lineage_ref = reference(229);
    let registry_head_ref = reference(228);
    let first_chain = chain_attestation(
        1,
        "mfm.test/two-route-inventory-chain-a",
        &registry_lineage_ref,
        &registry_head_ref,
    );
    let second_chain = chain_attestation(
        11_155_111,
        "mfm.test/two-route-inventory-chain-b",
        &registry_lineage_ref,
        &registry_head_ref,
    );
    let mut builder = EvmRoutingCatalogBuilder::new(
        registry_head_ref,
        vec![first_chain.clone(), second_chain.clone()],
    )
    .expect("two-route inventory builder");
    for (network, generation, chain) in [
        ("ethereum-mainnet", "inventory-generation-a", &first_chain),
        ("ethereum-sepolia", "inventory-generation-b", &second_chain),
    ] {
        builder
            .insert(
                route_descriptor(network, chain, generation),
                EvmRpcEndpoint::new("http://127.0.0.1:9").expect("inventory endpoint"),
                None,
            )
            .expect("inventory route");
    }
    let catalog = builder.build().expect("two-route inventory catalog");
    let generations = catalog
        .generation_descriptors()
        .map(|(generation, _)| generation.to_content_ref().expect("generation ref"))
        .collect();
    (catalog, generations)
}

#[test]
fn inventory_affine_types_are_not_cloneable_or_serializable() {
    static_assertions::assert_not_impl_any!(PendingEvmRpcInventory: Clone, serde::Serialize);
    static_assertions::assert_not_impl_any!(CompletedEvmRpcInventoryExchange: Clone, serde::Serialize);
    static_assertions::assert_not_impl_any!(EvmRpcInventoryFinishAuthorization: Clone, serde::Serialize);
    static_assertions::assert_not_impl_any!(EvmRpcInventoryProofs: Clone, serde::Serialize);
}

#[tokio::test]
async fn inventory_exchange_uses_only_retained_endpoint_and_redacts_private_values() {
    let finish_authorization = [0xa5; 32];
    let (_, provisional_generation) = inventory_catalog("http://127.0.0.1:9", None);
    let challenge = inventory_challenge(provisional_generation, 0, &finish_authorization);
    let retained = TestServer::start(vec![inventory_response(&challenge, 0, &[0x5a; 64])]).await;
    let substituted = TestServer::start(Vec::new()).await;
    let credential = format!("Bearer private-inventory-{}", std::process::id());
    let authorization = EvmRpcAuthorization::new(Zeroizing::new(credential.clone()))
        .expect("inventory authorization");
    let (catalog, actual_generation) = inventory_catalog(&retained.endpoint, Some(authorization));
    assert_eq!(actual_generation, *challenge.route_generation_ref());

    let pending = PendingEvmRpcInventory::new(catalog).expect("pending inventory");
    let private_endpoint = retained.endpoint.clone();
    for redacted in [
        format!("{pending:?}"),
        format!("{challenge:?}"),
        format!("{:?}", EvmTransportError::InventoryExchangeFailed),
    ] {
        assert!(!redacted.contains(&private_endpoint));
        assert!(!redacted.contains(&credential));
    }

    let (pending, proofs) = pending
        .exchange(EvmRpcInventoryChallenges::new(vec![challenge]).expect("challenges"))
        .await
        .expect("inventory exchange");
    assert_eq!(proofs.iter().count(), 1);
    let proof_debug = format!("{proofs:?}");
    assert!(!proof_debug.contains(&private_endpoint));
    assert!(!proof_debug.contains(&credential));
    assert!(!proof_debug.contains(&hex::encode([0x5a; 64])));
    let exchange_ref = proofs.exchange_ref();

    let completed = pending
        .finish(
            EvmRpcInventoryFinishAuthorization::new(Zeroizing::new(finish_authorization.to_vec()))
                .expect("finish authorization"),
        )
        .expect("completed exchange");
    assert_eq!(completed.exchange_ref(), exchange_ref);
    assert_eq!(
        completed
            .transport()
            .routing_generation_descriptors()
            .count(),
        1
    );

    let requests = retained.finish().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], json!("mfm_qualifyRpcInventory"));
    assert!(substituted.finish().await.is_empty());
}

#[tokio::test]
async fn inventory_rejects_foreign_stale_and_mismatched_finish_material() {
    let finish_authorization = [0xb6; 32];
    let (_, generation) = inventory_catalog("http://127.0.0.1:9", None);
    let challenge = inventory_challenge(generation, 0, &finish_authorization);
    let mut stale = serde_json::to_value(json!({
        "jsonrpc": "2.0",
        "id": 0,
        "result": {
            "protocol": "mfm.evm.rpc-inventory-qualification.v1",
            "ordinal": 0,
            "route_generation_ref": challenge.route_generation_ref(),
            "target_identity": hex::encode(challenge.target_identity().as_bytes()),
            "assembly_lease": hex::encode(challenge.assembly_lease().as_bytes()),
            "checkpoint": hex::encode([0xff; 32]),
            "route_challenge": hex::encode(challenge.route_challenge().as_bytes()),
            "finish_authorization_commitment":
                hex::encode(challenge.finish_authorization_commitment()),
            "proof": hex::encode([0x77; 64]),
        }
    }))
    .expect("stale response");
    let server = TestServer::start(vec![TestResponse::json(std::mem::take(&mut stale))]).await;
    let (catalog, _) = inventory_catalog(&server.endpoint, None);
    assert_eq!(
        PendingEvmRpcInventory::new(catalog)
            .expect("pending inventory")
            .exchange(EvmRpcInventoryChallenges::new(vec![challenge]).expect("challenge closure"))
            .await
            .err(),
        Some(EvmTransportError::InvalidInventoryProof)
    );
    assert_eq!(server.finish().await.len(), 1);

    let (_, generation) = inventory_catalog("http://127.0.0.1:9", None);
    let challenge = inventory_challenge(generation, 0, &finish_authorization);
    let server = TestServer::start(vec![inventory_response(&challenge, 0, &[0x66; 64])]).await;
    let (catalog, _) = inventory_catalog(&server.endpoint, None);
    let (pending, _) = PendingEvmRpcInventory::new(catalog)
        .expect("pending inventory")
        .exchange(EvmRpcInventoryChallenges::new(vec![challenge]).expect("challenge closure"))
        .await
        .expect("valid exchange");
    assert_eq!(
        pending
            .finish(
                EvmRpcInventoryFinishAuthorization::new(Zeroizing::new(vec![0xcc; 32]))
                    .expect("wrong finish authorization")
            )
            .err(),
        Some(EvmTransportError::InvalidInventoryFinishAuthorization)
    );
    assert_eq!(server.finish().await.len(), 1);
}

#[tokio::test]
async fn inventory_rejects_missing_duplicate_reordered_and_foreign_responses() {
    let finish_authorization = [0xc8; 32];
    let (_, generation) = inventory_catalog("http://127.0.0.1:9", None);
    let challenge = inventory_challenge(generation, 0, &finish_authorization);
    let proof_hex = hex::encode([0x55; 64]);
    let valid = inventory_response(&challenge, 0, &[0x55; 64]);
    let mut missing: Value = serde_json::from_slice(&valid.body).expect("valid response");
    missing["result"]
        .as_object_mut()
        .expect("result object")
        .remove("proof");
    let mut reordered: Value = serde_json::from_slice(&valid.body).expect("valid response");
    reordered["result"]["ordinal"] = json!(1);
    let mut foreign: Value = serde_json::from_slice(&valid.body).expect("valid response");
    foreign["result"]["target_identity"] = hex::encode([0xfe; 32]).into();
    let valid_text = String::from_utf8(valid.body).expect("response UTF-8");
    let proof_member = format!(r#""proof":"{proof_hex}""#);
    let duplicate_text =
        valid_text.replace(&proof_member, &format!("{proof_member},{proof_member}"));
    assert_ne!(duplicate_text, valid_text);

    let invalid_responses = [
        TestResponse::json(missing),
        TestResponse {
            status: 200,
            body: duplicate_text.into_bytes(),
            declared_length: None,
            omit_content_length: false,
            location: None,
        },
        TestResponse::json(reordered),
        TestResponse::json(foreign),
    ];
    for response in invalid_responses {
        let server = TestServer::start(vec![response]).await;
        let (catalog, _) = inventory_catalog(&server.endpoint, None);
        assert_eq!(
            PendingEvmRpcInventory::new(catalog)
                .expect("pending inventory")
                .exchange(
                    EvmRpcInventoryChallenges::new(vec![challenge.clone()])
                        .expect("challenge closure")
                )
                .await
                .err(),
            Some(EvmTransportError::InvalidInventoryProof)
        );
        assert_eq!(server.finish().await.len(), 1);
    }
}

#[tokio::test]
async fn inventory_rejects_missing_duplicate_reordered_and_foreign_challenges_before_io() {
    let finish_authorization = [0xd7; 32];

    let (catalog, generations) = two_route_inventory_catalog();
    let missing = vec![inventory_challenge(
        generations[0].clone(),
        0,
        &finish_authorization,
    )];
    assert_eq!(
        PendingEvmRpcInventory::new(catalog)
            .expect("pending inventory")
            .exchange(EvmRpcInventoryChallenges::new(missing).expect("bounded challenges"))
            .await
            .err(),
        Some(EvmTransportError::InvalidInventoryChallenge)
    );

    let (catalog, generations) = two_route_inventory_catalog();
    let duplicate = vec![
        inventory_challenge(generations[0].clone(), 0, &finish_authorization),
        inventory_challenge(generations[0].clone(), 1, &finish_authorization),
    ];
    assert_eq!(
        PendingEvmRpcInventory::new(catalog)
            .expect("pending inventory")
            .exchange(EvmRpcInventoryChallenges::new(duplicate).expect("bounded challenges"))
            .await
            .err(),
        Some(EvmTransportError::InvalidInventoryChallenge)
    );

    let (catalog, generations) = two_route_inventory_catalog();
    let reordered = vec![
        inventory_challenge(generations[1].clone(), 0, &finish_authorization),
        inventory_challenge(generations[0].clone(), 1, &finish_authorization),
    ];
    assert_eq!(
        PendingEvmRpcInventory::new(catalog)
            .expect("pending inventory")
            .exchange(EvmRpcInventoryChallenges::new(reordered).expect("bounded challenges"))
            .await
            .err(),
        Some(EvmTransportError::InvalidInventoryChallenge)
    );

    let (catalog, generations) = two_route_inventory_catalog();
    let shared_challenge = EvmRpcRouteChallenge::new([0xee; 32]);
    let duplicate_challenges = generations
        .into_iter()
        .enumerate()
        .map(|(index, generation)| {
            let challenge = inventory_challenge(
                generation,
                u8::try_from(index).expect("ordinal"),
                &finish_authorization,
            );
            EvmRpcInventoryChallenge::new(
                challenge.route_generation_ref().clone(),
                challenge.target_identity(),
                challenge.assembly_lease(),
                challenge.checkpoint(),
                shared_challenge,
                *challenge.finish_authorization_commitment(),
            )
        })
        .collect();
    assert_eq!(
        PendingEvmRpcInventory::new(catalog)
            .expect("pending inventory")
            .exchange(
                EvmRpcInventoryChallenges::new(duplicate_challenges).expect("bounded challenges")
            )
            .await
            .err(),
        Some(EvmTransportError::InvalidInventoryChallenge)
    );

    let (catalog, generations) = two_route_inventory_catalog();
    let foreign = vec![
        inventory_challenge(
            reference(227).to_content_ref().expect("foreign ref"),
            0,
            &finish_authorization,
        ),
        inventory_challenge(generations[1].clone(), 1, &finish_authorization),
    ];
    assert_eq!(
        PendingEvmRpcInventory::new(catalog)
            .expect("pending inventory")
            .exchange(EvmRpcInventoryChallenges::new(foreign).expect("bounded challenges"))
            .await
            .err(),
        Some(EvmTransportError::InvalidInventoryChallenge)
    );
}
