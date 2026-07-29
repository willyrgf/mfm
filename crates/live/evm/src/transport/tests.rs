use std::sync::{Arc, Mutex};

use super::EvmTransportOutcome;
use alloy_primitives::{Address, B256, U256};
use mfm_evm::{
    EvmAnchorConfirmationRequest, EvmAnchoredSource, EvmBlockAnchor, EvmChainIdentityRequest,
    EvmCheckedSource, EvmLatestAnchorRequest, EvmNativeBalanceRequest, EvmNetworkBinding,
    EvmRoutingGenerationRef, EvmSafeFailure, EvmTokenBalanceRequest, EvmTokenDecimalsRequest,
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
    location: Option<String>,
}

impl TestResponse {
    fn json(value: Value) -> Self {
        Self {
            status: 200,
            body: serde_json::to_vec(&value).expect("response JSON"),
            declared_length: None,
            location: None,
        }
    }

    fn status(status: u16) -> Self {
        Self {
            status,
            body: Vec::new(),
            declared_length: None,
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
                let location = response
                    .location
                    .as_deref()
                    .map(|value| format!("location: {value}\r\n"))
                    .unwrap_or_default();
                let head = format!(
                    "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n{}connection: close\r\n\r\n",
                    response.status, reason, length, location
                );
                socket
                    .write_all(head.as_bytes())
                    .await
                    .expect("write response head");
                socket
                    .write_all(&response.body)
                    .await
                    .expect("write response body");
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
async fn destination_and_invalid_response_failures_are_closed_and_single_call() {
    let cases = vec![
        (
            TestResponse::status(503),
            EvmSafeFailure::HttpStatus { status: 503 },
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
    let (transport, binding) = transport(&oversized.endpoint);
    assert_eq!(
        transport
            .chain_identity(&EvmChainIdentityRequest::new(binding))
            .await,
        EvmTransportOutcome::Indeterminate(EvmSafeFailure::ResponseTooLarge {
            size_class: EvmCoarseSizeClass::UpTo1Mib,
        })
    );
    assert_eq!(oversized.finish().await.len(), 1);
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
    let authorization =
        EvmRpcAuthorization::new(Zeroizing::new("Bearer resolved-secret".to_owned()))
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
        "resolved-secret",
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
