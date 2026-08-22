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
    assert_eq!(quantity_to_u64(&format!("0x{}", "f".repeat(17))), None);
    assert_eq!(quantity_to_u64("539"), None);
    assert_eq!(quantity_to_u64("0x"), None);
    assert_eq!(quantity_to_u64("0xzz"), None);

    assert_eq!(
        quantity_to_decimal("0xd3c21bcecceda1000000").as_deref(),
        Some("1000000000000000000000000")
    );
    assert_eq!(quantity_to_decimal("0x0").as_deref(), Some("0"));
    assert_eq!(quantity_to_decimal(&format!("0x{}", "f".repeat(65))), None);

    assert_eq!(block_tag("17"), Ok("0x11".to_owned()));
    assert_eq!(block_tag("0"), Ok("0x0".to_owned()));
    assert_eq!(block_tag("0x11"), Err(ReadAdapterError::Internal));
    assert_eq!(block_tag(&"9".repeat(40)), Err(ReadAdapterError::Internal));

    assert_eq!(
        word_to_u8("0x0000000000000000000000000000000000000000000000000000000000000012"),
        Some(18)
    );
    assert_eq!(
        word_to_u8("0x0000000000000000000000000000000000000000000000000000000000000100"),
        None
    );
    assert_eq!(word_to_u8("0x12"), None);

    assert!(is_block_hash(BLOCK_HASH));
    assert!(!is_block_hash("0xAAAA"));
    assert!(!is_block_hash("0x11"));

    assert_eq!(checked_address(HOLDER), Ok(HOLDER.to_owned()));
    // Checksummed input renders back to the exact lowercase 20-byte address.
    assert_eq!(
        checked_address("0x70997970C51812dc3A010C7d01b50e0d17dc79C8"),
        Ok(HOLDER.to_owned())
    );
    assert_eq!(checked_address("0xnothex"), Err(ReadAdapterError::Internal));
    assert_eq!(
        checked_address("wallet.native"),
        Err(ReadAdapterError::Internal)
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
            "1000000000000000000000000".to_owned()
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
        EvmProviderResponse::Read(EvmReadValue::Anchor {
            number: "17".to_owned(),
            hash: BLOCK_HASH.to_owned(),
        })
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
        assert_eq!(response, Err(ReadAdapterError::Unavailable), "body {body}");
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
    assert_eq!(response, Err(ReadAdapterError::Unavailable));
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
    assert_eq!(response, Err(ReadAdapterError::Unavailable));
    redirect_worker.join().expect("redirect worker");

    let _ = TcpStream::connect(target_address);
    assert!(!target_worker.join().expect("target worker"));
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
        Err(ReadAdapterError::Internal)
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
        Err(ReadAdapterError::Internal)
    );
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
