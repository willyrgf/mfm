use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::JoinHandle;

use mfm_evm::{EvmEndpoint, EvmPhysicalTarget};

use super::*;

const HOLDER: &str = "0x70997970c51812dc3a010c7d01b50e0d17dc79c8";
const TOKEN: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const BLOCK_HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";

/// Serves exactly one canned JSON body over plain HTTP and then closes.
struct Stub {
    url: String,
    worker: Option<JoinHandle<Vec<u8>>>,
}

/// Serves one response for each accepted request and retains every request body.
struct SequenceStub {
    url: String,
    worker: Option<JoinHandle<Vec<Vec<u8>>>>,
}

impl SequenceStub {
    fn new(bodies: Vec<String>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind sequence stub");
        let url = format!("http://{}", listener.local_addr().expect("stub address"));
        let worker = std::thread::spawn(move || {
            bodies
                .into_iter()
                .map(|body| {
                    let (mut stream, _) = listener.accept().expect("accept sequence request");
                    let request = read_http_request(&mut stream);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                         Connection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).expect("write");
                    stream.flush().expect("flush");
                    request
                })
                .collect()
        });
        Self {
            url,
            worker: Some(worker),
        }
    }

    fn provider(&self) -> JsonRpcEvmProvider {
        JsonRpcEvmProvider::new_http_for_test(self.url.clone()).expect("provider")
    }

    fn observed_requests(&mut self) -> Vec<serde_json::Value> {
        self.worker
            .take()
            .expect("one join")
            .join()
            .expect("join")
            .into_iter()
            .map(|request| serde_json::from_slice(&request).expect("request json"))
            .collect()
    }
}

impl Stub {
    fn new(body: impl Into<String>) -> Self {
        let body = body.into();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
        let url = format!("http://{}", listener.local_addr().expect("stub address"));
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let request = read_http_request(&mut stream);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).expect("write");
            stream.flush().expect("flush");
            request
        });
        Self {
            url,
            worker: Some(worker),
        }
    }

    fn provider(&self) -> JsonRpcEvmProvider {
        JsonRpcEvmProvider::new_http_for_test(self.url.clone()).expect("provider")
    }

    /// Returns the exact JSON body the provider sent.
    fn observed_request(&mut self) -> serde_json::Value {
        let bytes = self.worker.take().expect("one join").join().expect("join");
        serde_json::from_slice(&bytes).expect("request json")
    }
}

fn read_http_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        let Some(header_end) = find_header_end(&buffer) else {
            continue;
        };
        let length = content_length(&buffer[..header_end]);
        if buffer.len() >= header_end + length {
            return buffer[header_end..header_end + length].to_vec();
        }
    }
    buffer
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

fn content_length(headers: &[u8]) -> usize {
    std::str::from_utf8(headers)
        .expect("utf8 headers")
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(str::to_owned)
        })
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

fn target() -> EvmPhysicalTarget {
    EvmPhysicalTarget::new(
        1337,
        EvmEndpoint::new("reth-dev")
            .expect("endpoint")
            .endpoint_ref()
            .expect("endpoint ref"),
    )
    .expect("target")
}

fn intent_bytes(operation: &str, subject: serde_json::Value) -> Vec<u8> {
    let intent: EvmReadIntent = serde_json::from_value(serde_json::json!({
        "operation": operation,
        "chain_id": 1337,
        "subject": subject,
        "route_ref": target().binding_ref().expect("binding"),
    }))
    .expect("checked intent");
    serde_json::to_vec(&intent).expect("intent bytes")
}

fn anchored_intent_bytes(subject: serde_json::Value) -> Vec<u8> {
    let route = mfm_evm::EvmTransactionRoute::new(
        mfm_evm::EvmChainInstance::new(1337, EvmHash::new(BLOCK_HASH).expect("genesis"))
            .expect("chain"),
        EvmEndpoint::new("reth-dev")
            .expect("endpoint")
            .endpoint_ref()
            .expect("endpoint ref"),
    );
    let intent: EvmReadIntent = serde_json::from_value(serde_json::json!({
        "operation": "mfm.evm.read-anchored-contract-call@1",
        "chain_id": 1337,
        "subject": subject,
        "route_ref": route.binding_ref().expect("binding"),
    }))
    .expect("checked anchored intent");
    serde_json::to_vec(&intent).expect("intent bytes")
}

fn source(token: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "source_id": "wallet.native",
        "chain_id": 1337,
        "address": HOLDER,
        "token": token,
    })
}

fn anchor() -> serde_json::Value {
    serde_json::json!({ "number": "17", "hash": BLOCK_HASH })
}

fn operation(name: &str) -> StableId {
    StableId::new(name).expect("operation")
}

#[test]
fn conversions_and_calldata_are_exact() {
    assert_eq!(quantity_to_u64("0x539"), Some(1337));
    assert_eq!(quantity_to_u64("0x0"), Some(0));
    assert_eq!(quantity_to_u64("0x00"), None);
    assert_eq!(quantity_to_u64("0xA"), None);
    assert_eq!(quantity_to_u64(&format!("0x{}", "f".repeat(17))), None);
    assert_eq!(quantity_to_u64("539"), None);
    assert_eq!(quantity_to_u64("0x"), None);
    assert_eq!(quantity_to_u64("0xzz"), None);

    assert_eq!(
        quantity_to_decimal("0xd3c21bcecceda1000000").as_deref(),
        Some("1000000000000000000000000")
    );
    assert_eq!(quantity_to_decimal("0x0").as_deref(), Some("0"));
    assert_eq!(quantity_to_decimal("0x01"), None);
    assert_eq!(quantity_to_decimal(&format!("0x{}", "f".repeat(65))), None);

    assert_eq!(
        block_tag(&EvmU256::new("17").expect("block number")),
        Ok("0x11".to_owned())
    );
    assert_eq!(
        block_tag(&EvmU256::new("0").expect("block number")),
        Ok("0x0".to_owned())
    );

    assert_eq!(
        word_to_u8("0x0000000000000000000000000000000000000000000000000000000000000012"),
        Some(18)
    );
    assert_eq!(
        word_to_u8("0x0000000000000000000000000000000000000000000000000000000000000100"),
        None
    );
    assert_eq!(word_to_u8("0x12"), None);

    assert!(EvmHash::new(BLOCK_HASH).is_ok());
    assert!(EvmHash::new("0xAAAA").is_err());
    assert!(EvmHash::new("0x11").is_err());

    assert_eq!(checked_address(HOLDER), Ok(HOLDER.to_owned()));
    // Checksummed input renders back to the exact lowercase 20-byte address.
    assert_eq!(
        checked_address("0x70997970C51812dc3A010C7d01b50e0d17dc79C8"),
        Ok(HOLDER.to_owned())
    );
    assert_eq!(checked_address("0xnothex"), Err(AdapterError::Internal));
    assert_eq!(
        checked_address("wallet.native"),
        Err(AdapterError::Internal)
    );

    assert_eq!(decimals_calldata(), "0x313ce567");
    let calldata = balance_of_calldata(&Address::from_str(HOLDER).expect("holder"));
    assert_eq!(
        calldata,
        "0x70a0823100000000000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c8"
    );
    // Four selector bytes plus one 32-byte word.
    assert_eq!(calldata.len(), 2 + 8 + 64);
}

#[tokio::test]
async fn chain_identity_reads_the_exact_json_rpc_envelope() {
    let mut stub = Stub::new(r#"{"jsonrpc":"2.0","id":1,"result":"0x539"}"#);
    let response = stub
        .provider()
        .request(
            operation("mfm.evm.read-chain-identity@1"),
            intent_bytes(
                "mfm.evm.read-chain-identity@1",
                serde_json::json!({ "kind": "chain_identity" }),
            ),
        )
        .await
        .expect("chain identity");
    assert_eq!(
        response,
        EvmProviderResponse::Read(EvmReadValue::ChainId(1337))
    );
    assert_eq!(
        stub.observed_request(),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "eth_chainId",
            "params": []
        })
    );
}

#[tokio::test]
async fn native_balance_reads_the_committed_anchor_by_number() {
    let mut stub = Stub::new(r#"{"jsonrpc":"2.0","id":1,"result":"0xd3c21bcecceda1000000"}"#);
    let response = stub
        .provider()
        .request(
            operation("mfm.evm.read-native-balance@1"),
            intent_bytes(
                "mfm.evm.read-native-balance@1",
                serde_json::json!({
                    "kind": "native_balance",
                    "value": { "source": source(None), "anchor": anchor() }
                }),
            ),
        )
        .await
        .expect("native balance");
    assert_eq!(
        response,
        EvmProviderResponse::Read(EvmReadValue::RawUnits(
            EvmU256::new("1000000000000000000000000").expect("units")
        ))
    );
    assert_eq!(
        stub.observed_request()["params"],
        serde_json::json!([HOLDER, "0x11"])
    );
}

#[tokio::test]
async fn anchor_confirmation_never_asks_for_the_moving_head() {
    let mut stub = Stub::new(format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":{{"number":"0x11","hash":"{BLOCK_HASH}"}}}}"#
    ));
    let response = stub
        .provider()
        .request(
            operation("mfm.evm.confirm-balance-anchor@1"),
            intent_bytes(
                "mfm.evm.confirm-balance-anchor@1",
                serde_json::json!({
                    "kind": "confirm_anchor",
                    "value": { "source": source(None), "anchor": anchor() }
                }),
            ),
        )
        .await
        .expect("confirmed anchor");
    assert_eq!(
        response,
        EvmProviderResponse::Read(EvmReadValue::Anchor(EvmBlockAnchor::new(
            EvmU256::new("17").expect("number"),
            EvmHash::new(BLOCK_HASH).expect("hash"),
        )))
    );
    assert_eq!(
        stub.observed_request()["params"],
        serde_json::json!(["0x11", false])
    );
}

#[tokio::test]
async fn token_decimals_splices_no_address_and_decodes_one_word() {
    let mut stub = Stub::new(
        r#"{"jsonrpc":"2.0","id":1,"result":"0x0000000000000000000000000000000000000000000000000000000000000012"}"#,
    );
    let response = stub
        .provider()
        .request(
            operation("mfm.evm.read-token-decimals@1"),
            intent_bytes(
                "mfm.evm.read-token-decimals@1",
                serde_json::json!({
                    "kind": "token_decimals",
                    "value": { "source": source(Some(TOKEN)), "anchor": anchor() }
                }),
            ),
        )
        .await
        .expect("token decimals");
    assert_eq!(
        response,
        EvmProviderResponse::Read(EvmReadValue::TokenDecimals(18))
    );
    assert_eq!(
        stub.observed_request()["params"],
        serde_json::json!([{ "to": TOKEN, "data": "0x313ce567" }, "0x11"])
    );
}

#[tokio::test]
async fn revert_and_codeless_call_are_definite_safe_failures() {
    for body in [
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":3,"message":"execution reverted"}}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":"0x"}"#,
    ] {
        let response = Stub::new(body)
            .provider()
            .request(
                operation("mfm.evm.read-token-decimals@1"),
                intent_bytes(
                    "mfm.evm.read-token-decimals@1",
                    serde_json::json!({
                        "kind": "token_decimals",
                        "value": { "source": source(Some(TOKEN)), "anchor": anchor() }
                    }),
                ),
            )
            .await
            .expect("safe failure");
        assert_eq!(response, EvmProviderResponse::SafeFailure);
    }
}

#[tokio::test]
async fn malformed_null_and_unreachable_ingress_is_unavailable() {
    for body in [
        r#"{"jsonrpc":"2.0","id":1,"result":"0xnothex"}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":null}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":{"number":"0x11"}}"#,
        r#"not json"#,
    ] {
        let response = Stub::new(body)
            .provider()
            .request(
                operation("mfm.evm.read-initial-anchor@1"),
                intent_bytes(
                    "mfm.evm.read-initial-anchor@1",
                    serde_json::json!({ "kind": "initial_anchor" }),
                ),
            )
            .await;
        assert_eq!(response, Err(AdapterError::Unavailable), "body {body}");
    }

    let unbound = TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}", unbound.local_addr().expect("address"));
    drop(unbound);
    let response = JsonRpcEvmProvider::new_http_for_test(url)
        .expect("provider")
        .request(
            operation("mfm.evm.read-chain-identity@1"),
            intent_bytes(
                "mfm.evm.read-chain-identity@1",
                serde_json::json!({ "kind": "chain_identity" }),
            ),
        )
        .await;
    assert_eq!(response, Err(AdapterError::Unavailable));
}

#[tokio::test]
async fn redirects_never_leave_the_selected_endpoint() {
    let target = TcpListener::bind("127.0.0.1:0").expect("bind redirect target");
    let target_address = target.local_addr().expect("redirect target address");
    let target_worker = std::thread::spawn(move || {
        let (mut stream, _) = target.accept().expect("accept redirect target");
        let request = read_http_request(&mut stream);
        if !request.is_empty() {
            let body = r#"{"jsonrpc":"2.0","id":1,"result":"0x539"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).expect("write target");
        }
        !request.is_empty()
    });

    let redirect = TcpListener::bind("127.0.0.1:0").expect("bind redirect source");
    let redirect_url = format!(
        "http://{}",
        redirect.local_addr().expect("redirect address")
    );
    let redirect_worker = std::thread::spawn(move || {
        let (mut stream, _) = redirect.accept().expect("accept redirect source");
        let _ = read_http_request(&mut stream);
        let response = format!(
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{target_address}\r\n\
             Content-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .expect("write redirect");
    });

    let response = JsonRpcEvmProvider::new_http_for_test(redirect_url)
        .expect("provider")
        .request(
            operation("mfm.evm.read-chain-identity@1"),
            intent_bytes(
                "mfm.evm.read-chain-identity@1",
                serde_json::json!({ "kind": "chain_identity" }),
            ),
        )
        .await;
    assert_eq!(response, Err(AdapterError::Unavailable));
    redirect_worker.join().expect("redirect worker");

    let _ = TcpStream::connect(target_address);
    assert!(!target_worker.join().expect("target worker"));
}

#[tokio::test]
async fn complete_raw_submission_followed_by_connection_drop_is_unavailable() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind drop server");
    let url = format!("http://{}", listener.local_addr().expect("server address"));
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept submission");
        read_http_request(&mut stream)
    });
    let raw = ExactRawTransaction::new(vec![0x02, 0xc0]).expect("bounded raw");
    assert_eq!(
        JsonRpcEvmProvider::new_http_for_test(url)
            .expect("provider")
            .submit_raw(&raw)
            .await,
        Err(AdapterError::Unavailable)
    );
    let request: serde_json::Value =
        serde_json::from_slice(&worker.join().expect("drop server")).expect("request JSON");
    assert_eq!(request["method"], "eth_sendRawTransaction");
    assert_eq!(request["params"], serde_json::json!(["0x02c0"]));
}

#[tokio::test]
async fn undecodable_or_mismatched_intent_is_internal_and_never_enters_transport() {
    // No stub is bound: an Internal outcome proves nothing reached a transport.
    let provider =
        JsonRpcEvmProvider::new_http_for_test("http://127.0.0.1:1".to_owned()).expect("provider");
    assert_eq!(
        provider
            .request(operation("mfm.evm.read-chain-identity@1"), b"{}".to_vec())
            .await,
        Err(AdapterError::Internal)
    );
    assert_eq!(
        provider
            .request(
                operation("mfm.evm.read-initial-anchor@1"),
                intent_bytes(
                    "mfm.evm.read-chain-identity@1",
                    serde_json::json!({ "kind": "chain_identity" }),
                ),
            )
            .await,
        Err(AdapterError::Internal)
    );
}

#[tokio::test]
async fn transaction_provider_uses_exact_calls_and_strict_checked_receipts() {
    let genesis = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let mut chain = SequenceStub::new(vec![
        r#"{"jsonrpc":"2.0","id":1,"result":"0x539"}"#.to_owned(),
        format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"number":"0x0","hash":"{genesis}"}}}}"#),
    ]);
    let observed = chain
        .provider()
        .chain_instance()
        .await
        .expect("chain instance");
    assert_eq!(observed.chain_id(), 1337);
    assert_eq!(observed.genesis_hash().as_str(), genesis);
    let requests = chain.observed_requests();
    assert_eq!(requests[0]["method"], "eth_chainId");
    assert_eq!(requests[1]["method"], "eth_getBlockByNumber");
    assert_eq!(requests[1]["params"], serde_json::json!(["0x0", false]));

    let transaction_hash =
        EvmHash::new("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
            .expect("transaction hash");
    let receipt_body = format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":{{"transactionHash":"{}","from":"{HOLDER}","to":"{TOKEN}","contractAddress":null,"status":"0x1","blockNumber":"0x11","blockHash":"{BLOCK_HASH}"}}}}"#,
        transaction_hash.as_str()
    );
    let mut receipt_stub = Stub::new(receipt_body);
    let receipt = receipt_stub
        .provider()
        .receipt(&transaction_hash)
        .await
        .expect("receipt call")
        .expect("present receipt");
    assert_eq!(receipt.transaction_hash(), &transaction_hash);
    assert!(matches!(
        receipt.result(),
        ProviderReceiptResult::SuccessCall { target } if target.as_str() == TOKEN
    ));
    assert_eq!(
        receipt_stub.observed_request()["params"],
        serde_json::json!([transaction_hash.as_str()])
    );

    let mut pending_stub = Stub::new(r#"{"jsonrpc":"2.0","id":1,"result":"0x7"}"#);
    assert_eq!(
        pending_stub
            .provider()
            .pending_nonce(&mfm_evm::EvmAddress::new(HOLDER).expect("sender"))
            .await,
        Ok(7)
    );
    assert_eq!(
        pending_stub.observed_request()["params"],
        serde_json::json!([HOLDER, "pending"])
    );

    let raw = ExactRawTransaction::new(vec![0x02, 0xc0]).expect("bounded raw");
    let mut submit_stub = Stub::new(format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":"{}"}}"#,
        transaction_hash.as_str()
    ));
    assert_eq!(
        submit_stub.provider().submit_raw(&raw).await,
        Ok(transaction_hash.clone())
    );
    assert_eq!(
        submit_stub.observed_request()["params"],
        serde_json::json!(["0x02c0"])
    );

    for body in [
        r#"{"jsonrpc":"2.0","id":1,"result":null}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"opaque"}}"#,
    ] {
        let result = Stub::new(body).provider().receipt(&transaction_hash).await;
        if body.contains("result") {
            assert_eq!(result, Ok(None));
        } else {
            assert_eq!(result, Err(AdapterError::Unavailable));
        }
    }
}

#[tokio::test]
async fn anchored_call_observes_code_and_same_anchor_before_returning_bytes() {
    let block =
        format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"number":"0x11","hash":"{BLOCK_HASH}"}}}}"#);
    let mut stub = SequenceStub::new(vec![
        block.clone(),
        r#"{"jsonrpc":"2.0","id":1,"result":"0x6000"}"#.to_owned(),
        r#"{"jsonrpc":"2.0","id":1,"result":"0x0102"}"#.to_owned(),
        block,
    ]);
    let response = stub
        .provider()
        .request(
            operation("mfm.evm.read-anchored-contract-call@1"),
            anchored_intent_bytes(serde_json::json!({
                "kind": "anchored_contract_call",
                "value": {
                    "anchor": anchor(),
                    "calldata": "q80",
                    "target": TOKEN,
                }
            })),
        )
        .await
        .expect("anchored call");
    let EvmProviderResponse::Read(EvmReadValue::AnchoredContractCall(result)) = response else {
        panic!("expected anchored result")
    };
    assert_eq!(result.return_bytes().expect("return bytes"), [1, 2]);
    assert_eq!(result.anchor().hash().as_str(), BLOCK_HASH);
    let requests = stub.observed_requests();
    assert_eq!(
        requests
            .iter()
            .map(|request| request["method"].as_str().expect("method"))
            .collect::<Vec<_>>(),
        [
            "eth_getBlockByNumber",
            "eth_getCode",
            "eth_call",
            "eth_getBlockByNumber"
        ]
    );
    let block_selector = serde_json::json!({
        "blockHash": BLOCK_HASH,
        "requireCanonical": true,
    });
    assert_eq!(
        requests[1]["params"],
        serde_json::json!([TOKEN, block_selector.clone()])
    );
    assert_eq!(
        requests[2]["params"],
        serde_json::json!([{ "to": TOKEN, "data": "0xabcd" }, block_selector])
    );
}

#[tokio::test]
async fn anchored_absence_codeless_and_anchor_replacement_are_closed_evidence() {
    let absent = Stub::new(r#"{"jsonrpc":"2.0","id":1,"result":null}"#)
        .provider()
        .request(
            operation("mfm.evm.read-anchored-contract-call@1"),
            anchored_intent_bytes(serde_json::json!({
                "kind": "anchored_contract_call",
                "value": { "anchor": anchor(), "calldata": "", "target": TOKEN }
            })),
        )
        .await;
    assert_eq!(absent, Ok(EvmProviderResponse::SafeFailure));

    let replacement = "0x3333333333333333333333333333333333333333333333333333333333333333";
    let replaced = Stub::new(format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":{{"number":"0x11","hash":"{replacement}"}}}}"#
    ))
    .provider()
    .request(
        operation("mfm.evm.read-anchored-contract-call@1"),
        anchored_intent_bytes(serde_json::json!({
            "kind": "anchored_contract_call",
            "value": { "anchor": anchor(), "calldata": "", "target": TOKEN }
        })),
    )
    .await;
    assert_eq!(replaced, Ok(EvmProviderResponse::IntegrityBlocked));

    let block =
        format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"number":"0x11","hash":"{BLOCK_HASH}"}}}}"#);
    let codeless = SequenceStub::new(vec![
        block,
        r#"{"jsonrpc":"2.0","id":1,"result":"0x"}"#.to_owned(),
    ])
    .provider()
    .request(
        operation("mfm.evm.read-anchored-contract-call@1"),
        anchored_intent_bytes(serde_json::json!({
            "kind": "anchored_contract_call",
            "value": { "anchor": anchor(), "calldata": "", "target": TOKEN }
        })),
    )
    .await;
    assert_eq!(codeless, Ok(EvmProviderResponse::Rejected));
}

#[test]
fn an_unusable_url_fails_construction_without_naming_it() {
    // The provider implements no `Debug`, so no accident can print the URL it holds.
    let Err(error) = JsonRpcEvmProvider::new_http_for_test("not-a-url".to_owned()) else {
        panic!("an unusable url must fail construction");
    };
    assert_eq!(
        error.to_string(),
        "evm provider transport could not be constructed"
    );
}

#[test]
fn locator_accepts_raw_http_urls_and_rejects_other_forms() {
    for accepted in ["http://127.0.0.1:8545", "https://example.com"] {
        let locator = EvmAdapterLocator::parse(accepted).expect("HTTP locator");
        JsonRpcEvmProvider::connect(&locator).expect("provider");
    }

    for rejected in [
        "ftp://127.0.0.1/x",
        "https://127.0.0.1/x#fragment",
        "https://127.0.0.1/x\n",
        r#"{"v":1,"url":"https://127.0.0.1/x"}"#,
    ] {
        assert!(EvmAdapterLocator::parse(rejected).is_err(), "{rejected}");
    }
}
