#[derive(Debug, serde::Serialize, serde::Deserialize, mfm_program_derive::MfmValue)]
#[serde(deny_unknown_fields)]
struct EmptyContext {}

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::num::NonZeroU64;
use std::str::FromStr;
use std::thread::JoinHandle;

use mfm_evm::{EvmEndpoint, EvmPhysicalTarget};
use mfm_values::canonicalize_mfm_value;

use super::*;

const HOLDER: &str = "0x70997970c51812dc3a010c7d01b50e0d17dc79c8";
const TOKEN: &str = "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const BLOCK_HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";

/// Serves one response for each accepted request and retains every request body.
struct Stub {
    url: String,
    worker: JoinHandle<Vec<Vec<u8>>>,
}

impl Stub {
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
        Self { url, worker }
    }

    fn provider(&self) -> JsonRpcEvmProvider {
        JsonRpcEvmProvider::connect(&EvmAdapterLocator::parse(&self.url).expect("locator"))
            .expect("provider")
    }

    fn observed_requests(self) -> Vec<serde_json::Value> {
        self.worker
            .join()
            .expect("join")
            .into_iter()
            .map(|request| serde_json::from_slice(&request).expect("request json"))
            .collect()
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
    EvmPhysicalTarget {
        chain_id: NonZeroU64::new(1337).expect("nonzero chain"),
        endpoint_ref: EvmEndpoint::new("reth-dev")
            .expect("endpoint")
            .endpoint_ref()
            .expect("endpoint ref"),
    }
}

fn intent(subject: serde_json::Value) -> EvmReadIntent {
    serde_json::from_value(serde_json::json!({
        "chain_id": 1337,
        "subject": subject,
        "route_ref": target().binding_ref().expect("binding"),
    }))
    .expect("checked intent")
}

fn anchored_intent(subject: serde_json::Value) -> AnchoredContractCallIntent {
    let route = mfm_evm::EvmTransactionRoute {
        chain_instance: mfm_evm::EvmChainInstance {
            chain_id: NonZeroU64::new(1337).expect("nonzero chain"),
            expected_genesis_hash: EvmHash::new(BLOCK_HASH).expect("genesis"),
        },
        endpoint_ref: EvmEndpoint::new("reth-dev")
            .expect("endpoint")
            .endpoint_ref()
            .expect("endpoint ref"),
    };
    let value = subject.get("value").expect("anchored value");
    serde_json::from_value(serde_json::json!({
        "chain_id": 1337,
        "route_ref": route.binding_ref().expect("binding"),
        "anchor": value.get("anchor").expect("anchor"),
        "calldata": value.get("calldata").expect("calldata"),
        "target": value.get("target").expect("target"),
    }))
    .expect("checked anchored intent")
}

async fn observe_broad(
    provider: JsonRpcEvmProvider,
    intent: EvmReadIntent,
) -> Result<EvmReadEvidence, AdapterError<EvmOperationalError>> {
    let (_, intent_value_ref) = canonicalize_mfm_value(&intent).expect("intent ref");
    provider.observe(&intent_value_ref, &intent).await
}

async fn observe_anchored(
    provider: JsonRpcEvmProvider,
    intent: AnchoredContractCallIntent,
) -> Result<AnchoredContractCallEvidence, AdapterError<EvmOperationalError>> {
    let (_, intent_value_ref) = canonicalize_mfm_value(&intent).expect("intent ref");
    provider
        .observe_anchored_call(&intent_value_ref, &intent)
        .await
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

fn word_to_u8(word: AbiWord) -> Option<u8> {
    u8::try_from(decode_abi_u256(word)).ok()
}

fn abi_word(value: &str) -> Result<AbiWord, RpcRejection> {
    AbiWord::try_from(RpcData::parse(value, 33)?)
}

#[test]
fn conversions_and_calldata_are_exact() {
    for (encoded, value) in [("0x539", 1337), ("0x0", 0), ("0x1", 1)] {
        assert_eq!(
            RpcQuantity::parse(encoded).expect("quantity").to_u64(),
            Some(value)
        );
    }
    for encoded in ["0x00", "0x01", "0xA", "539", "0x", "0xzz"] {
        assert!(RpcQuantity::parse(encoded).is_err());
    }
    assert_eq!(
        RpcQuantity::parse(&format!("0x{}", "f".repeat(17)))
            .expect("u256 quantity")
            .to_u64(),
        None
    );
    assert!(RpcQuantity::parse(&format!("0x{}", "f".repeat(65))).is_err());
    assert_eq!(
        RpcQuantity::parse("0xd3c21bcecceda1000000")
            .expect("quantity")
            .decimal(),
        "1000000000000000000000000"
    );
    assert_eq!(RpcQuantity::parse("0x0").expect("zero").decimal(), "0");

    assert!(RpcData::parse("0x", 1).expect("empty data").0.is_empty());
    assert_eq!(RpcData::parse("0x00", 1).expect("zero byte").0, [0_u8]);
    assert_eq!(RpcData::parse("0x0", 1), Err(RpcRejection::Encoding));
    assert_eq!(
        RpcData::parse("0x0000", 1),
        Err(RpcRejection::Size {
            limit: 1,
            observed: ObservedSize::Exact { value: 2 }
        })
    );

    let zero_word = format!("0x{}", "00".repeat(32));
    let one_word = format!("0x{}01", "00".repeat(31));
    assert_eq!(
        decode_abi_u256(abi_word(&zero_word).expect("zero word")),
        U256::ZERO
    );
    assert_eq!(
        decode_abi_u256(abi_word(&one_word).expect("one word")),
        U256::from(1)
    );
    assert!(abi_word(&format!("0x{}", "00".repeat(31))).is_err());
    assert!(abi_word(&format!("0x{}", "00".repeat(33))).is_err());

    assert_eq!(
        block_tag(&EvmU256::new("17").expect("block number")).unwrap(),
        "0x11"
    );
    assert_eq!(
        block_tag(&EvmU256::new("0").expect("block number")).unwrap(),
        "0x0"
    );

    assert_eq!(
        word_to_u8(
            abi_word("0x0000000000000000000000000000000000000000000000000000000000000012")
                .expect("decimals word")
        ),
        Some(18)
    );
    assert_eq!(
        word_to_u8(
            abi_word("0x0000000000000000000000000000000000000000000000000000000000000100")
                .expect("oversized decimals")
        ),
        None
    );

    assert!(EvmHash::new(BLOCK_HASH).is_ok());
    assert!(EvmHash::new("0xAAAA").is_err());
    assert!(EvmHash::new("0x11").is_err());

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
    let stub = Stub::new(vec![r#"{"jsonrpc":"2.0","id":1,"result":"0x539"}"#.into()]);
    let response = observe_broad(
        stub.provider(),
        intent(serde_json::json!({ "kind": "chain_identity" })),
    )
    .await
    .expect("chain identity");
    assert!(matches!(
        response,
        EvmReadEvidence::Returned {
            value: EvmReadValue::ChainId(chain_id),
            ..
        } if chain_id == NonZeroU64::new(1337).expect("nonzero chain")
    ));
    assert_eq!(
        stub.observed_requests()[0],
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
    let stub = Stub::new(vec![
        r#"{"jsonrpc":"2.0","id":1,"result":"0xd3c21bcecceda1000000"}"#.into(),
    ]);
    let response = observe_broad(
        stub.provider(),
        intent(serde_json::json!({
            "kind": "native_balance",
            "value": { "source": source(None), "anchor": anchor() }
        })),
    )
    .await
    .expect("native balance");
    assert!(matches!(
        response,
        EvmReadEvidence::Returned {
            value: EvmReadValue::RawUnits(units),
            ..
        } if units == EvmU256::new("1000000000000000000000000").expect("units")
    ));
    assert_eq!(
        stub.observed_requests()[0]["params"],
        serde_json::json!([HOLDER, "0x11"])
    );
}

#[tokio::test]
async fn anchor_confirmation_never_asks_for_the_moving_head() {
    let stub = Stub::new(vec![format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":{{"number":"0x11","hash":"{BLOCK_HASH}"}}}}"#
    )]);
    let response = observe_broad(
        stub.provider(),
        intent(serde_json::json!({
            "kind": "confirm_anchor",
            "value": { "source": source(None), "anchor": anchor() }
        })),
    )
    .await
    .expect("confirmed anchor");
    assert!(matches!(
        response,
        EvmReadEvidence::Returned {
            value: EvmReadValue::Anchor(observed),
            ..
        } if observed == EvmBlockAnchor { number: EvmU256::new("17").expect("number"), hash: EvmHash::new(BLOCK_HASH).expect("hash") }
    ));
    assert_eq!(
        stub.observed_requests()[0]["params"],
        serde_json::json!(["0x11", false])
    );
}

#[tokio::test]
async fn token_decimals_splices_no_address_and_decodes_one_word() {
    let stub = Stub::new(vec![r#"{"jsonrpc":"2.0","id":1,"result":"0x0000000000000000000000000000000000000000000000000000000000000012"}"#.into()]);
    let response = observe_broad(
        stub.provider(),
        intent(serde_json::json!({
            "kind": "token_decimals",
            "value": { "source": source(Some(TOKEN)), "anchor": anchor() }
        })),
    )
    .await
    .expect("token decimals");
    assert!(matches!(
        response,
        EvmReadEvidence::Returned {
            value: EvmReadValue::TokenDecimals(decimals),
            ..
        } if decimals == EvmTokenDecimals::new(18).expect("decimals")
    ));
    assert_eq!(
        stub.observed_requests()[0]["params"],
        serde_json::json!([{ "to": TOKEN, "data": "0x313ce567" }, "0x11"])
    );
}

#[tokio::test]
async fn token_balance_decodes_a_zero_padded_abi_word() {
    let stub = Stub::new(vec![r#"{"jsonrpc":"2.0","id":1,"result":"0x0000000000000000000000000000000000000000000000000000000000000001"}"#.into()]);
    let response = observe_broad(
        stub.provider(),
        intent(serde_json::json!({
            "kind": "token_balance",
            "value": { "source": source(Some(TOKEN)), "anchor": anchor() }
        })),
    )
    .await
    .expect("token balance");
    assert!(matches!(
        response,
        EvmReadEvidence::Returned {
            value: EvmReadValue::RawUnits(units),
            ..
        } if units == EvmU256::new("1").expect("units")
    ));
    assert_eq!(
        stub.observed_requests()[0]["params"],
        serde_json::json!([{
            "to": TOKEN,
            "data": "0x70a0823100000000000000000000000070997970c51812dc3a010c7d01b50e0d17dc79c8"
        }, "0x11"])
    );
}

#[tokio::test]
async fn malformed_or_wrong_sized_abi_data_is_unavailable() {
    for result in [
        format!("0x{}", "00".repeat(31)),
        format!("0x{}", "00".repeat(33)),
        "0x0".to_owned(),
        "0xgg".to_owned(),
    ] {
        let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "result": result });
        let response = observe_broad(
            Stub::new(vec![(body.to_string())]).provider(),
            intent(serde_json::json!({
                "kind": "token_balance",
                "value": { "source": source(Some(TOKEN)), "anchor": anchor() }
            })),
        )
        .await;
        assert!(matches!(
            response,
            Err(AdapterError::Operational(error)) if error.kind() == EvmOperationalKind::Unavailable
        ));
    }
}

#[tokio::test]
async fn rpc_errors_are_unavailable_but_empty_token_data_is_safe_failure() {
    let token_intent = || {
        intent(serde_json::json!({
            "kind": "token_decimals",
            "value": { "source": source(Some(TOKEN)), "anchor": anchor() }
        }))
    };
    let rpc_error = observe_broad(
        Stub::new(vec![
            (r#"{"jsonrpc":"2.0","id":1,"error":{"code":3,"message":"execution reverted"}}"#)
                .into(),
        ])
        .provider(),
        token_intent(),
    )
    .await;
    assert!(matches!(
        rpc_error,
        Err(AdapterError::Operational(error)) if error.kind() == EvmOperationalKind::Unavailable
    ));

    let empty = observe_broad(
        Stub::new(vec![r#"{"jsonrpc":"2.0","id":1,"result":"0x"}"#.into()]).provider(),
        token_intent(),
    )
    .await
    .expect("safe failure");
    assert!(matches!(empty, EvmReadEvidence::SafeFailure { .. }));
}

#[tokio::test]
async fn malformed_null_and_unreachable_ingress_is_unavailable() {
    for body in [
        r#"{"jsonrpc":"2.0","id":1,"result":"0xnothex"}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":null}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":{"number":"0x11"}}"#,
        r#"not json"#,
    ] {
        let response = observe_broad(
            Stub::new(vec![body.into()]).provider(),
            intent(serde_json::json!({ "kind": "initial_anchor" })),
        )
        .await;
        assert!(
            matches!(
                response,
                Err(AdapterError::Operational(error)) if error.kind() == EvmOperationalKind::Unavailable
            ),
            "body {body}"
        );
    }

    let unbound = TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}", unbound.local_addr().expect("address"));
    drop(unbound);
    let response = observe_broad(
        JsonRpcEvmProvider::connect(&EvmAdapterLocator::parse(url).expect("locator"))
            .expect("provider"),
        intent(serde_json::json!({ "kind": "chain_identity" })),
    )
    .await;
    assert!(matches!(
        response,
        Err(AdapterError::Operational(error)) if error.kind() == EvmOperationalKind::Unavailable
    ));
}

#[tokio::test]
async fn incomplete_or_malformed_rpc_errors_are_unavailable() {
    for error in [
        serde_json::json!({}),
        serde_json::json!({ "code": -1 }),
        serde_json::json!({ "message": "opaque" }),
        serde_json::json!({ "code": "-1", "message": "opaque" }),
        serde_json::json!({ "code": -1, "message": 1 }),
        serde_json::json!({ "code": -1, "message": "opaque", "unexpected": true }),
    ] {
        let body = serde_json::json!({ "jsonrpc": "2.0", "id": 1, "error": error });
        let response = observe_broad(
            Stub::new(vec![(body.to_string())]).provider(),
            intent(serde_json::json!({ "kind": "chain_identity" })),
        )
        .await;
        assert!(matches!(
            response,
            Err(AdapterError::Operational(error)) if error.kind() == EvmOperationalKind::Unavailable
        ));
    }
}

#[test]
fn only_complete_exact_rpc_error_objects_parse() {
    let envelope: RpcEnvelope<'_> = serde_json::from_str(
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"opaque","data":null}}"#,
    )
    .unwrap();
    let error: RpcError<'_> = serde_json::from_str(envelope.error.unwrap().get()).unwrap();
    assert_eq!(error.code, -1);
    assert_eq!(error.data.unwrap().get(), "null");
    for error in [
        serde_json::json!({}),
        serde_json::json!({ "code": -1 }),
        serde_json::json!({ "message": "opaque" }),
        serde_json::json!({ "code": -1, "message": "opaque", "extra": true }),
    ] {
        assert!(serde_json::from_str::<RpcError<'_>>(&error.to_string()).is_err());
    }
}

#[test]
fn receipt_nullable_action_fields_are_present_even_when_null() {
    let complete = serde_json::json!({
        "transactionHash": "0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "from": HOLDER,
        "to": null,
        "contractAddress": TOKEN,
        "status": "0x1",
        "blockNumber": "0x11",
        "blockHash": BLOCK_HASH,
    });
    assert!(serde_json::from_value::<RpcReceipt>(complete.clone()).is_ok());
    for field in ["to", "contractAddress"] {
        let mut incomplete = complete.clone();
        incomplete
            .as_object_mut()
            .expect("receipt object")
            .remove(field);
        assert!(
            serde_json::from_value::<RpcReceipt>(incomplete).is_err(),
            "missing {field}"
        );
    }
}

#[tokio::test]
async fn rpc_envelope_version_id_and_fields_are_exact() {
    for body in [
        r#"{"jsonrpc":"1.0","id":1,"result":"0x539"}"#,
        r#"{"jsonrpc":"2.0","id":2,"result":"0x539"}"#,
        r#"{"jsonrpc":"2.0","id":"1","result":"0x539"}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":"0x539","extra":true}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":"0x539","error":{"code":-1,"message":"opaque"}}"#,
        r#"{"jsonrpc":"2.0","id":1}"#,
    ] {
        let response = observe_broad(
            Stub::new(vec![body.into()]).provider(),
            intent(serde_json::json!({ "kind": "chain_identity" })),
        )
        .await;
        assert!(
            matches!(
                response,
                Err(AdapterError::Operational(error)) if error.kind() == EvmOperationalKind::Unavailable
            ),
            "body {body}"
        );
    }
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

    let response = observe_broad(
        JsonRpcEvmProvider::connect(&EvmAdapterLocator::parse(redirect_url).expect("locator"))
            .expect("provider"),
        intent(serde_json::json!({ "kind": "chain_identity" })),
    )
    .await;
    assert!(matches!(
        response,
        Err(AdapterError::Operational(error)) if error.kind() == EvmOperationalKind::Unavailable
    ));
    redirect_worker.join().expect("redirect worker");

    let _ = TcpStream::connect(target_address);
    assert!(!target_worker.join().expect("target worker"));
}

#[tokio::test]
async fn loopback_submission_acknowledgement_drop_after_full_request_is_unavailable() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind drop server");
    let url = format!("http://{}", listener.local_addr().expect("server address"));
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept submission");
        read_http_request(&mut stream)
    });
    let raw = ExactRawTransaction::new(vec![0x02, 0xc0]).expect("bounded raw");
    assert!(matches!(
        JsonRpcEvmProvider::connect(&EvmAdapterLocator::parse(url).expect("locator"))
            .expect("provider")
            .submit_raw(&raw)
            .await,
        Err(AdapterError::Operational(error)) if error.kind() == EvmOperationalKind::Unavailable
    ));
    let request: serde_json::Value =
        serde_json::from_slice(&worker.join().expect("drop server")).expect("request JSON");
    assert_eq!(request["method"], "eth_sendRawTransaction");
    assert_eq!(request["params"], serde_json::json!(["0x02c0"]));
}

#[tokio::test]
async fn transaction_provider_uses_exact_calls_and_strict_checked_receipts() {
    let genesis = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let chain = Stub::new(vec![
        r#"{"jsonrpc":"2.0","id":1,"result":"0x539"}"#.to_owned(),
        format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"number":"0x0","hash":"{genesis}"}}}}"#),
    ]);
    let observed = chain
        .provider()
        .chain_instance()
        .await
        .expect("chain instance");
    assert_eq!(
        observed.chain_id,
        NonZeroU64::new(1337).expect("nonzero chain")
    );
    assert_eq!(observed.expected_genesis_hash.to_string(), genesis);
    let requests = chain.observed_requests();
    assert_eq!(requests[0]["method"], "eth_chainId");
    assert_eq!(requests[1]["method"], "eth_getBlockByNumber");
    assert_eq!(requests[1]["params"], serde_json::json!(["0x0", false]));

    let transaction_hash =
        EvmHash::new("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
            .expect("transaction hash");
    let receipt_body = format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":{{"transactionHash":"{}","from":"{HOLDER}","to":"{TOKEN}","contractAddress":null,"status":"0x1","blockNumber":"0x11","blockHash":"{BLOCK_HASH}"}}}}"#,
        transaction_hash
    );
    let receipt_stub = Stub::new(vec![(receipt_body)]);
    let receipt = receipt_stub
        .provider()
        .receipt(&transaction_hash)
        .await
        .expect("receipt call")
        .expect("present receipt");
    assert_eq!(receipt.transaction_hash(), &transaction_hash);
    assert!(matches!(
        receipt.result(),
        ProviderReceiptResult::SuccessCall { target } if target.to_string() == TOKEN
    ));
    assert_eq!(
        receipt_stub.observed_requests()[0]["params"],
        serde_json::json!([transaction_hash.to_string()])
    );

    let pending_stub = Stub::new(vec![r#"{"jsonrpc":"2.0","id":1,"result":"0x7"}"#.into()]);
    assert_eq!(
        pending_stub
            .provider()
            .pending_nonce(&mfm_evm::EvmAddress::new(HOLDER).expect("sender"))
            .await
            .unwrap(),
        7
    );
    assert_eq!(
        pending_stub.observed_requests()[0]["params"],
        serde_json::json!([HOLDER, "pending"])
    );

    let raw = ExactRawTransaction::new(vec![0x02, 0xc0]).expect("bounded raw");
    let submit_stub = Stub::new(vec![format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":"{}"}}"#,
        transaction_hash
    )]);
    assert_eq!(
        submit_stub.provider().submit_raw(&raw).await.unwrap(),
        transaction_hash.clone()
    );
    assert_eq!(
        submit_stub.observed_requests()[0]["params"],
        serde_json::json!(["0x02c0"])
    );

    for body in [
        r#"{"jsonrpc":"2.0","id":1,"result":null}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":"0xnothex"}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"opaque"}}"#,
        r#"not json"#,
    ] {
        assert!(
            matches!(
                Stub::new(vec![body.into()])
                    .provider()
                    .submit_raw(&raw)
                    .await,
                Err(AdapterError::Operational(error)) if error.kind() == EvmOperationalKind::Unavailable
            ),
            "body {body}"
        );
    }

    for body in [
        r#"{"jsonrpc":"2.0","id":1,"result":null}"#,
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"opaque"}}"#,
    ] {
        let result = Stub::new(vec![body.into()])
            .provider()
            .receipt(&transaction_hash)
            .await;
        if body.contains("result") {
            assert_eq!(result.unwrap(), None);
        } else {
            assert!(matches!(
                result,
                Err(AdapterError::Operational(error)) if error.kind() == EvmOperationalKind::Unavailable
            ));
        }
    }
}

#[tokio::test]
async fn anchored_call_observes_code_and_same_anchor_before_returning_bytes() {
    let block =
        format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"number":"0x11","hash":"{BLOCK_HASH}"}}}}"#);
    let stub = Stub::new(vec![
        block.clone(),
        r#"{"jsonrpc":"2.0","id":1,"result":"0x6000"}"#.to_owned(),
        r#"{"jsonrpc":"2.0","id":1,"result":"0x0102"}"#.to_owned(),
        block,
    ]);
    let response = observe_anchored(
        stub.provider(),
        anchored_intent(serde_json::json!({
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
    let AnchoredContractCallEvidence::Returned { result, .. } = response else {
        panic!("expected anchored result")
    };
    assert_eq!(result.return_bytes(), [1, 2]);
    assert_eq!(result.anchor().hash.to_string(), BLOCK_HASH);
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
    let absent = observe_anchored(
        Stub::new(vec![r#"{"jsonrpc":"2.0","id":1,"result":null}"#.into()]).provider(),
        anchored_intent(serde_json::json!({
            "kind": "anchored_contract_call",
            "value": { "anchor": anchor(), "calldata": "", "target": TOKEN }
        })),
    )
    .await;
    assert!(matches!(
        absent,
        Ok(AnchoredContractCallEvidence::SafeFailure { .. })
    ));

    let replacement = "0x3333333333333333333333333333333333333333333333333333333333333333";
    let replaced = observe_anchored(
        Stub::new(vec![format!(
            r#"{{"jsonrpc":"2.0","id":1,"result":{{"number":"0x11","hash":"{replacement}"}}}}"#
        )])
        .provider(),
        anchored_intent(serde_json::json!({
            "kind": "anchored_contract_call",
            "value": { "anchor": anchor(), "calldata": "", "target": TOKEN }
        })),
    )
    .await;
    assert!(matches!(
        replaced,
        Ok(AnchoredContractCallEvidence::IntegrityBlocked { .. })
    ));

    let block =
        format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"number":"0x11","hash":"{BLOCK_HASH}"}}}}"#);
    let codeless = observe_anchored(
        Stub::new(vec![
            block,
            r#"{"jsonrpc":"2.0","id":1,"result":"0x"}"#.to_owned(),
        ])
        .provider(),
        anchored_intent(serde_json::json!({
            "kind": "anchored_contract_call",
            "value": { "anchor": anchor(), "calldata": "", "target": TOKEN }
        })),
    )
    .await;
    assert!(matches!(
        codeless,
        Ok(AnchoredContractCallEvidence::Rejected { .. })
    ));
}

#[test]
fn an_unusable_url_fails_construction_without_naming_it() {
    // The locator implements no `Debug`, so no accident can print the URL it holds.
    let Err(error) = EvmAdapterLocator::parse("not-a-url") else {
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

#[tokio::test]
async fn nullable_rpc_results_require_the_result_field() {
    let hash = EvmHash::from_bytes([1; 32]);
    for body in [
        r#"{"jsonrpc":"2.0","id":1}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":null,"error":{"code":-1,"message":"opaque"}}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":null,"result":null}"#,
    ] {
        assert!(matches!(
            Stub::new(vec![body.into()]).provider().receipt(&hash).await,
            Err(AdapterError::Operational(error)) if error.kind() == EvmOperationalKind::Unavailable
        ));
        let intent = anchored_intent(serde_json::json!({
            "kind": "anchored_contract_call",
            "value": { "anchor": anchor(), "calldata": "", "target": TOKEN }
        }));
        assert!(matches!(
            observe_anchored(Stub::new(vec![body.into()]).provider(), intent.clone()).await,
            Err(AdapterError::Operational(error)) if error.kind() == EvmOperationalKind::Unavailable
        ));
        let block = format!(
            r#"{{"jsonrpc":"2.0","id":1,"result":{{"number":"0x11","hash":"{BLOCK_HASH}"}}}}"#
        );
        let stub = Stub::new(vec![
            block,
            r#"{"jsonrpc":"2.0","id":1,"result":"0x6000"}"#.to_owned(),
            r#"{"jsonrpc":"2.0","id":1,"result":"0x"}"#.to_owned(),
            body.to_owned(),
        ]);
        assert!(matches!(
            observe_anchored(stub.provider(), intent).await,
            Err(AdapterError::Operational(error)) if error.kind() == EvmOperationalKind::Unavailable
        ));
        assert_eq!(stub.observed_requests().len(), 4);
    }
    assert_eq!(
        Stub::new(vec![r#"{"jsonrpc":"2.0","id":1,"result":null}"#.into()])
            .provider()
            .receipt(&hash)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn transport_deadline_and_http_rate_limit_preserve_typed_operational_causes() {
    for timeout in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let locator =
            EvmAdapterLocator::parse(format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let (release, released) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_http_request(&mut stream);
            if timeout {
                // Keep the response pending until the real provider deadline returns.
                released.recv().unwrap();
            } else {
                stream.write_all(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            }
            request
        });
        let provider = JsonRpcEvmProvider::connect(&locator).unwrap();
        let response = observe_broad(
            provider,
            intent(serde_json::json!({ "kind": "chain_identity" })),
        )
        .await;
        if timeout {
            release.send(()).unwrap();
        }
        let request: serde_json::Value = serde_json::from_slice(&worker.join().unwrap()).unwrap();
        assert_eq!(request["method"], "eth_chainId");
        let Err(AdapterError::Operational(error)) = response else {
            panic!("operational failure");
        };
        assert!(if timeout {
            error.kind() == EvmOperationalKind::Timeout
        } else {
            error.kind() == EvmOperationalKind::RateLimited
        });
        assert_eq!(error.provider_failure().method, EvmRpcMethod::ChainId);
        assert_eq!(
            error.provider_failure().stage,
            if timeout {
                RpcStage::Send
            } else {
                RpcStage::Status
            }
        );
    }
}

#[tokio::test]
async fn rpc_codes_remain_distinct_in_committed_and_cold_domain_failures() {
    use mfm_evm::{
        CheckChainIdentity, EvmBalanceContext, EvmBalanceFailure, EvmBalanceRequest,
        EvmChainIdentityRead,
    };
    use mfm_program::{
        Identity, NoParams, Occurrence, Operation, OperationExpansion, ProgramLimits,
    };
    use mfm_runtime::{RunViewState, Runtime, RuntimeAssemblyBuilder};
    use std::sync::Arc;

    struct ReadChain {
        target: EvmPhysicalTarget,
    }
    impl Operation for ReadChain {
        type Input = EvmBalanceContext<EmptyContext>;
        type Output = Self::Input;
        type Failure = EvmBalanceFailure;
        fn validate_input(&self, _: &Self::Input) -> mfm_program::Result<()> {
            Ok(())
        }
        fn expand(
            &self,
            body: &mut OperationExpansion<Self::Input, Self::Output, Self::Failure>,
        ) -> mfm_program::Result<()> {
            body.read::<CheckChainIdentity<EmptyContext>, EvmChainIdentityRead, Identity<EvmBalanceFailure>>(
                &self.target.binding_ref().unwrap(), NoParams, Occurrence::new())
        }
    }
    let mut retained = Vec::new();
    for (ordinal, code) in [-32001, -32002].into_iter().enumerate() {
        let stub = Stub::new(vec![format!(
            r#"{{"jsonrpc":"2.0","id":1,"error":{{"code":{code},"message":"secret-canary","data":{{"credential":"secret-canary"}}}}}}"#
        )]);
        let target = EvmPhysicalTarget {
            chain_id: NonZeroU64::new(1).unwrap(),
            endpoint_ref: EvmEndpoint::new("audit-fixture")
                .unwrap()
                .endpoint_ref()
                .unwrap(),
        };
        let request = EvmBalanceRequest::new(
            vec![EvmBalanceSource::new(
                "wallet",
                target.chain_id,
                EvmAddress::from_bytes([1; 20]),
                None,
            )
            .unwrap()],
            18,
        )
        .unwrap();
        let input = EvmBalanceContext::new(
            request,
            EmptyContext {},
            0,
            "collection".into(),
            target.binding_ref().unwrap(),
        )
        .unwrap();
        let program = mfm_program::expand_program(
            mfm_ids::EntryPointId::new("mfm.test.evm/causal-read@1").unwrap(),
            &ReadChain {
                target: target.clone(),
            },
            &input,
            ProgramLimits::new(0),
        )
        .unwrap();
        let mut builder = RuntimeAssemblyBuilder::new().unwrap();
        builder
            .register_read::<CheckChainIdentity<EmptyContext>, EvmChainIdentityRead>()
            .unwrap();
        crate::register_evm_reads(&mut builder, target, Arc::new(stub.provider())).unwrap();
        let runtime = Runtime::new(builder.finish(), Arc::new(mfm_store::MemoryStore::new()));
        let run =
            mfm_ids::RunId::from_digest(mfm_ids::DigestBytes::from_array([ordinal as u8 + 1; 32]));
        let hot = runtime.start(run.clone(), program, input).await.unwrap();
        let cold = runtime.read(&run).await.unwrap();
        assert_eq!(hot.head_digest(), cold.head_digest());
        let mut errors = Vec::new();
        for view in [&hot, &cold] {
            let RunViewState::Failed(report) = view.state() else {
                panic!("failed read");
            };
            let incident = report.failure();
            assert!(matches!(incident, mfm_runtime::Failure::Read { .. }));
            let error = incident.original().decode::<EvmOperationalError>().unwrap();
            assert_eq!(
                mfm_program::ClassifyError::classify(&error),
                mfm_program::Classification::Retryable
            );
            let cause = error.provider_failure();
            assert_eq!(cause.method, EvmRpcMethod::ChainId);
            assert_eq!(cause.stage, RpcStage::Envelope);
            let response = cause.diagnostics.response().as_ref().unwrap();
            assert_eq!(response.status().get(), 200);
            assert_eq!(response.rpc_code(), Some(code));
            assert!(cause.diagnostics.sources().layers().is_empty());
            assert_eq!(
                cause.diagnostics.sources().end(),
                mfm_diagnostics::ChainEnd::Complete
            );
            assert_eq!(cause.diagnostics.omissions().len(), 2);
            let (bytes, _) = canonicalize_mfm_value(&error).unwrap();
            assert!(!bytes.as_str().contains("secret-canary"));
            assert!(!format!("{error:?} {error}").contains("secret-canary"));
            errors.push(error);
        }
        assert_eq!(errors[0], errors[1]);
        retained.push(errors.remove(0));
        assert_eq!(stub.observed_requests().len(), 1);
    }
    assert_ne!(retained[0], retained[1]);
}

#[tokio::test]
async fn response_status_survives_body_failure_and_size_refusal() {
    for bounded in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let locator =
            EvmAdapterLocator::parse(format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_http_request(&mut stream);
            let length = if bounded { MAX_RESPONSE_BYTES + 1 } else { 20 };
            stream.write_all(format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {length}\r\nConnection: close\r\n\r\nx").as_bytes()).unwrap();
        });
        let error = JsonRpcEvmProvider::connect(&locator)
            .unwrap()
            .chain_id()
            .await
            .unwrap_err();
        worker.join().unwrap();
        let AdapterError::Operational(error) = error else {
            panic!("provider failure");
        };
        let cause = error.provider_failure();
        assert_eq!(cause.stage, RpcStage::Body);
        assert_eq!(
            cause
                .diagnostics
                .response()
                .as_ref()
                .unwrap()
                .status()
                .get(),
            206
        );
        if bounded {
            assert_eq!(
                cause.failure,
                ProviderFailureKind::Rejected {
                    field: RpcField::Data,
                    cause: RpcRejection::Size {
                        limit: 524288,
                        observed: ObservedSize::Exact { value: 524289 }
                    }
                }
            );
            assert!(cause.diagnostics.sources().layers().is_empty());
        } else {
            assert_eq!(cause.failure, ProviderFailureKind::Client);
            assert_eq!(
                cause.diagnostics.sources().layers()[0].kind(),
                mfm_diagnostics::SourceKind::Transport
            );
            assert!(!cause.diagnostics.omissions().is_empty());
        }
        let (bytes, _) = canonicalize_mfm_value(&error).unwrap();
        assert_eq!(
            serde_json::from_slice::<EvmOperationalError>(bytes.as_bytes()).unwrap(),
            error
        );
    }
}

#[tokio::test]
async fn development_funding_uses_the_shared_causal_rpc_boundary() {
    let transaction_hash = EvmHash::from_bytes([7; 32]);
    let stub = Stub::new(vec![
        format!(r#"{{"jsonrpc":"2.0","id":1,"result":["{HOLDER}"]}}"#),
        format!(r#"{{"jsonrpc":"2.0","id":1,"result":"{transaction_hash}"}}"#),
    ]);
    let sender = EvmAddress::from_bytes([2; 20]);
    let value = EvmU256::from_u64(1_000_000_000_000_000_000);
    let result = stub
        .provider()
        .fund_development_sender(&sender, &value, 10, 2)
        .await
        .unwrap();
    assert_eq!(result, transaction_hash);
    let requests = stub.observed_requests();
    assert_eq!(requests[0]["method"], "eth_accounts");
    assert_eq!(requests[1]["method"], "eth_sendTransaction");
    assert_eq!(
        requests[1]["params"],
        serde_json::json!([{
            "from": HOLDER, "to": sender, "value": "0xde0b6b3a7640000", "gas": "0x5208",
            "maxFeePerGas": "0xa", "maxPriorityFeePerGas": "0x2",
        }])
    );
    let rejected = Stub::new(vec![
        format!(r#"{{"jsonrpc":"2.0","id":1,"result":["{HOLDER}"]}}"#),
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32011,"message":"secret-canary","data":null}}"#
            .into(),
    ]);
    let AdapterError::Operational(error) = rejected
        .provider()
        .fund_development_sender(&sender, &value, 10, 2)
        .await
        .unwrap_err()
    else {
        panic!("funding cause");
    };
    assert_eq!(
        error.provider_failure().method,
        EvmRpcMethod::SendTransaction
    );
    assert_eq!(
        error
            .provider_failure()
            .diagnostics
            .response()
            .as_ref()
            .unwrap()
            .rpc_code(),
        Some(-32011)
    );
    assert!(!canonicalize_mfm_value(&error)
        .unwrap()
        .0
        .as_str()
        .contains("secret-canary"));
    assert_eq!(rejected.observed_requests().len(), 2);
}

#[tokio::test]
async fn body_deadline_retains_headers_and_parser_location_is_reviewed() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let locator =
        EvmAdapterLocator::parse(format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let (release, released) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_http_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n")
            .unwrap();
        released.recv().unwrap();
    });
    let provider = JsonRpcEvmProvider {
        url: locator.url,
        http: reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap(),
    };
    let error = provider.chain_id().await.unwrap_err();
    release.send(()).unwrap();
    worker.join().unwrap();
    let AdapterError::Operational(error) = error else {
        panic!("body timeout");
    };
    assert_eq!(error.kind(), EvmOperationalKind::Timeout);
    let source = error.provider_failure();
    assert_eq!(source.stage, RpcStage::Body);
    assert_eq!(
        source
            .diagnostics
            .response()
            .as_ref()
            .unwrap()
            .status()
            .get(),
        200
    );
    assert_eq!(
        source.diagnostics.sources().layers()[0].kind(),
        mfm_diagnostics::SourceKind::Transport
    );

    let stub = Stub::new(vec!["{\n  \"jsonrpc\": \"2.0\",\n  ?secret-canary".into()]);
    let AdapterError::Operational(error) = stub.provider().chain_id().await.unwrap_err() else {
        panic!("parser cause");
    };
    assert_eq!(error.provider_failure().stage, RpcStage::Envelope);
    assert_eq!(
        error.provider_failure().diagnostics.sources().layers()[0].facts(),
        &[mfm_diagnostics::SourceFact::Parse {
            category: mfm_diagnostics::ParseCategory::Syntax,
            location: mfm_diagnostics::ParseLocation::LineColumn { line: 3, column: 3 }
        },]
    );
    assert!(!canonicalize_mfm_value(&error)
        .unwrap()
        .0
        .as_str()
        .contains("secret-canary"));
    assert_eq!(stub.observed_requests().len(), 1);
}

#[tokio::test]
async fn local_range_failure_retains_field_and_checked_observation() {
    let stub = Stub::new(vec![
        r#"{"jsonrpc":"2.0","id":1,"result":"0x10000000000000000"}"#.into(),
    ]);
    let AdapterError::Operational(error) = stub
        .provider()
        .pending_nonce(&EvmAddress::from_bytes([1; 20]))
        .await
        .unwrap_err()
    else {
        panic!("checked nonce rejection");
    };
    assert_eq!(
        error.provider_failure().method,
        EvmRpcMethod::GetTransactionCount
    );
    assert_eq!(
        error.provider_failure().failure,
        ProviderFailureKind::Rejected {
            field: RpcField::Nonce,
            cause: RpcRejection::Range {
                minimum: 0,
                maximum: u64::MAX,
                observed: EvmU256::new("18446744073709551616").unwrap()
            },
        }
    );
    assert!(error
        .provider_failure()
        .diagnostics
        .sources()
        .layers()
        .is_empty());
    assert_eq!(stub.observed_requests().len(), 1);
}

#[tokio::test]
async fn send_failure_retains_os_ancestry_without_request_credentials() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let locator =
        EvmAdapterLocator::parse(format!("http://user:secret-canary@{address}/secret-canary"))
            .unwrap();
    let AdapterError::Operational(error) = JsonRpcEvmProvider::connect(&locator)
        .unwrap()
        .chain_id()
        .await
        .unwrap_err()
    else {
        panic!("connect failure");
    };
    let cause = error.provider_failure();
    assert_eq!(cause.stage, RpcStage::Send);
    assert!(cause.diagnostics.response().is_none());
    assert!(cause
        .diagnostics
        .sources()
        .layers()
        .iter()
        .any(|layer| layer.kind() == mfm_diagnostics::SourceKind::Os));
    assert!(cause
        .diagnostics
        .omissions()
        .iter()
        .any(|omission| omission.field() == mfm_diagnostics::OmittedField::Url));
    assert!(!canonicalize_mfm_value(&error)
        .unwrap()
        .0
        .as_str()
        .contains("secret-canary"));
    assert!(!format!("{error:?} {error}").contains("secret-canary"));
}

#[tokio::test]
async fn streamed_body_overflow_records_a_lower_bound_without_an_invented_source() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let locator =
        EvmAdapterLocator::parse(format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        read_http_request(&mut stream);
        stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n80001\r\n").unwrap();
        stream.write_all(&vec![b'x'; 524289]).unwrap();
    });
    let AdapterError::Operational(error) = JsonRpcEvmProvider::connect(&locator)
        .unwrap()
        .chain_id()
        .await
        .unwrap_err()
    else {
        panic!("body bound");
    };
    worker.join().unwrap();
    assert_eq!(error.provider_failure().stage, RpcStage::Body);
    assert!(matches!(error.provider_failure().failure,
        ProviderFailureKind::Rejected { field: RpcField::Data, cause: RpcRejection::Size {
            limit: 524288, observed: ObservedSize::AtLeast { value },
        } } if value >= 524289));
    assert!(error
        .provider_failure()
        .diagnostics
        .sources()
        .layers()
        .is_empty());
}
