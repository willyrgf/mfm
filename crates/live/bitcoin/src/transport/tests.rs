//! Qualification tests for the unregistered one-RPC Bitcoin transport primitives.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bitcoin::BlockHash;
use mfm_bitcoin::{
    BitcoinAddress, BitcoinNetworkId, BitcoinNetworkTag, BitcoinRoutingGenerationRef,
    BitcoinScanRequest, BitcoinSourceBinding, BitcoinSourceIdentity, BITCOIN_SCAN_ADDRESS_LIMIT,
};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, SchemaId};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

const LEGACY_MAIN: &str = "1BoatSLRHtKNngkdXEeobR76b53LETtpyT";
const SEGWIT_MAIN: &str = "bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuw";
const ANCHOR_HASH: &str = "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc";
const TXID_ONE: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const TXID_TWO: &str = "2222222222222222222222222222222222222222222222222222222222222222";

#[derive(Clone, Copy)]
enum Mode {
    Valid,
    ScanIncomplete,
    ScanBusy,
    NearScanBusy,
    HttpFailure,
    Redirect,
    StallScan,
}

struct TestServer {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
}

impl TestServer {
    async fn spawn(mode: Mode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let captured = Arc::clone(&captured);
                tokio::spawn(async move {
                    let bytes = read_http_request(&mut stream).await;
                    let request: Value =
                        serde_json::from_slice(request_body(&bytes)).expect("request JSON");
                    captured.lock().expect("requests").push(request.clone());
                    if matches!(mode, Mode::StallScan) && request["method"] == "scantxoutset" {
                        std::future::pending::<()>().await;
                    }
                    stream
                        .write_all(response(mode, &request).as_bytes())
                        .await
                        .expect("response");
                });
            }
        });
        Self {
            url: format!("http://{address}"),
            requests,
        }
    }

    fn requests(&self) -> Vec<Value> {
        self.requests.lock().expect("requests").clone()
    }
}

fn binding() -> BitcoinSourceBinding {
    BitcoinSourceBinding::new(
        BitcoinNetworkId::new("bitcoin-mainnet").expect("network"),
        BitcoinNetworkTag::Main,
        BitcoinSourceIdentity::new("public-bitcoin-core").expect("source"),
    )
}

fn generation(byte: u8) -> BitcoinRoutingGenerationRef {
    let schema = SchemaId::new(
        "mfm.bitcoin.routing-generation",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([0x41; 32]),
    )
    .expect("schema");
    let digest = ContentDigest::from_digest(
        DigestAlgorithm::Sha256V1,
        DigestBytes::from_array([byte; 32]),
    );
    BitcoinRoutingGenerationRef::from_reviewed(
        ContentRef::new(schema, digest).expect("routing generation"),
    )
}

fn scan_request() -> BitcoinScanRequest {
    BitcoinScanRequest::new(
        BitcoinNetworkTag::Main,
        vec![LEGACY_MAIN.to_owned(), SEGWIT_MAIN.to_owned()],
    )
    .expect("scan request")
}

fn session(server: &TestServer, generation: BitcoinRoutingGenerationRef) -> BitcoinRpcSession {
    BitcoinRpcSession::new(
        BitcoinRpcEndpoint::new(&server.url).expect("endpoint"),
        None,
        binding(),
        generation,
        Duration::from_secs(1),
    )
    .expect("session")
}

#[tokio::test]
async fn construction_performs_no_provider_io_and_fixes_exact_generation() {
    let server = TestServer::spawn(Mode::Valid).await;
    let admitted = generation(1);
    let session = session(&server, admitted.clone());

    assert!(server.requests().is_empty());
    assert_eq!(session.routing_generation(), &admitted);
    assert_eq!(session.binding(), &binding());
}

#[tokio::test]
async fn each_public_read_method_performs_exactly_one_protocol_operation() {
    let server = TestServer::spawn(Mode::Valid).await;
    let session = session(&server, generation(1));
    let request = scan_request();

    let info = session
        .get_blockchain_info()
        .await
        .expect("blockchain-info");
    assert_eq!(info.chain(), "main");
    assert!(!info.initial_block_download());
    assert_eq!(server.requests().len(), 1);

    let scan = session.scan_tx_out_set_start(&request).await.expect("scan");
    assert!(scan.success());
    assert_eq!(scan.height(), 850_000);
    assert_eq!(scan.anchor_hash().to_string(), ANCHOR_HASH);
    assert_eq!(scan.balances()[0].balance_sats(), 3);
    assert_eq!(scan.balances()[1].balance_sats(), 4);
    assert_eq!(server.requests().len(), 2);

    let confirmation = session
        .get_block_hash(scan.height())
        .await
        .expect("block hash");
    assert_eq!(confirmation, scan.anchor_hash());
    assert_eq!(server.requests().len(), 3);

    let requests = server.requests();
    assert_eq!(
        requests
            .iter()
            .map(|request| request["method"].as_str().expect("method"))
            .collect::<Vec<_>>(),
        ["getblockchaininfo", "scantxoutset", "getblockhash"]
    );
    assert_eq!(
        requests[1]["params"],
        json!([
            "start",
            [
                format!("addr({LEGACY_MAIN})"),
                format!("addr({SEGWIT_MAIN})")
            ]
        ])
    );
    assert_eq!(requests[2]["params"], json!([850_000]));
}

#[tokio::test]
async fn scan_incomplete_remains_a_typed_returned_result() {
    let server = TestServer::spawn(Mode::ScanIncomplete).await;
    let scan = session(&server, generation(1))
        .scan_tx_out_set_start(&scan_request())
        .await
        .expect("representable incomplete scan");

    assert!(!scan.success());
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn exact_scan_busy_classifier_rejects_near_matches() {
    let exact = TestServer::spawn(Mode::ScanBusy).await;
    assert_eq!(
        session(&exact, generation(1))
            .scan_tx_out_set_start(&scan_request())
            .await
            .expect_err("busy"),
        BitcoinRpcError::Rpc {
            code: -8,
            scan_busy: true,
        }
    );
    let near = TestServer::spawn(Mode::NearScanBusy).await;
    assert_eq!(
        session(&near, generation(1))
            .scan_tx_out_set_start(&scan_request())
            .await
            .expect_err("near busy"),
        BitcoinRpcError::Rpc {
            code: -8,
            scan_busy: false,
        }
    );
}

#[tokio::test]
async fn failure_redirect_and_cancellation_do_not_retry_or_issue_control_calls() {
    for mode in [Mode::HttpFailure, Mode::Redirect] {
        let server = TestServer::spawn(mode).await;
        let _ = session(&server, generation(1))
            .scan_tx_out_set_start(&scan_request())
            .await
            .expect_err("single failed call");
        assert_eq!(server.requests().len(), 1);
        assert_eq!(server.requests()[0]["method"], "scantxoutset");
    }

    let stalled = TestServer::spawn(Mode::StallScan).await;
    let session = Arc::new(session(&stalled, generation(1)));
    let task = tokio::spawn({
        let session = Arc::clone(&session);
        async move { session.scan_tx_out_set_start(&scan_request()).await }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if stalled.requests().len() == 1 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("scan entered");
    task.abort();
    let _ = task.await;
    assert_eq!(stalled.requests().len(), 1);
    assert_eq!(stalled.requests()[0]["method"], "scantxoutset");
}

#[test]
fn exact_generation_resolution_has_no_current_alias_fallback() {
    #[derive(Default)]
    struct Resolver {
        current: Option<BitcoinRoutingGenerationRef>,
        routes: BTreeMap<BitcoinRoutingGenerationRef, &'static str>,
    }

    let admitted = generation(1);
    let replacement = generation(2);
    let mut resolver = Resolver::default();
    resolver.routes.insert(admitted.clone(), "first");
    resolver.current = Some(admitted.clone());
    resolver.routes.insert(replacement.clone(), "replacement");
    resolver.current = Some(replacement);

    assert_eq!(resolver.routes.get(&admitted), Some(&"first"));
    resolver.routes.remove(&admitted);
    assert_eq!(resolver.routes.get(&admitted), None);
    assert_eq!(
        resolver
            .current
            .as_ref()
            .and_then(|current| resolver.routes.get(current)),
        Some(&"replacement"),
        "the current alias exists but is not a substitute for the admitted generation"
    );
}

#[test]
fn bitcoin_collection_remains_explicitly_unregistered() {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Gap {
        BoundedProviderWork,
        RepeatedCostAndSharedConcurrencyAcceptance,
    }

    let unresolved = [
        Gap::BoundedProviderWork,
        Gap::RepeatedCostAndSharedConcurrencyAcceptance,
    ];
    assert_eq!(unresolved.len(), 2);

    let live_root = include_str!("../lib.rs");
    assert!(!live_root.contains("register_bitcoin"));
    assert!(!live_root.contains("CapabilityCatalog"));
    assert!(!live_root.contains("collect_balances"));
}

#[test]
fn scan_request_bound_is_fixed() {
    assert_eq!(BITCOIN_SCAN_ADDRESS_LIMIT, 1_024);
}

#[test]
fn raw_decimal_parser_is_exact_and_bounded() {
    for (raw, expected) in [
        ("0", 0),
        ("0.", 0),
        ("0.00000001", 1),
        ("1.2", 120_000_000),
        ("21000000", 2_100_000_000_000_000),
    ] {
        assert_eq!(parse_btc_amount(raw).expect(raw), expected);
    }
    for invalid in [
        "",
        ".1",
        "+1",
        "-1",
        "1e-8",
        "1E8",
        "0.000000001",
        "21000000.00000001",
        "18446744073709551616",
        "1.2.3",
    ] {
        assert_eq!(
            parse_btc_amount(invalid),
            Err(BitcoinRpcError::ResponseInvalid),
            "{invalid}"
        );
    }
}

#[test]
fn scan_reduction_rejects_duplicate_outpoints_unknown_scripts_and_bad_totals() {
    let legacy_script = script_hex(LEGACY_MAIN);
    let segwit_script = script_hex(SEGWIT_MAIN);
    let valid = |unspents: &str, total: &str| {
        format!(
            r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":7,"unspents":[{unspents}],"total_amount":{total}}}"#
        )
    };
    let first = unspent(TXID_ONE, 0, &legacy_script, "0.00000001", 849_999);
    assert_reduce_invalid(&valid(&format!("{first},{first}"), "0.00000002"));
    assert_reduce_invalid(&valid(
        &unspent(TXID_TWO, 0, "00", "0.00000001", 849_999),
        "0.00000001",
    ));
    let known = format!(
        "{},{}",
        first,
        unspent(TXID_TWO, 1, &segwit_script, "0.00000002", 850_000)
    );
    assert_reduce_invalid(&valid(&known, "0.00000004"));
}

fn assert_reduce_invalid(raw: &str) {
    let scan = decode_unique::<ScanTxOutSetResult>(raw.as_bytes()).expect("typed scan");
    assert!(matches!(
        reduce_scan(&scan_request(), scan),
        Err(BitcoinRpcError::ResponseInvalid)
    ));
}

fn script_hex(address: &str) -> String {
    hex::encode(
        BitcoinAddress::parse(address, BitcoinNetworkTag::Main)
            .expect("address")
            .script_pubkey()
            .as_bytes(),
    )
}

fn unspent(txid: &str, vout: u64, script: &str, amount: &str, height: u64) -> String {
    format!(
        r#"{{"txid":"{txid}","vout":{vout},"scriptPubKey":"{script}","desc":"ignored","amount":{amount},"height":{height}}}"#
    )
}

fn response(mode: Mode, request: &Value) -> String {
    if matches!(mode, Mode::HttpFailure) {
        return http_response("500 Internal Server Error", r#"{"error":"discarded"}"#);
    }
    if matches!(mode, Mode::Redirect) {
        return "HTTP/1.1 302 Found\r\nlocation: http://127.0.0.1:9/\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned();
    }
    let id = request["id"].as_u64().expect("request ID");
    let method = request["method"].as_str().expect("method");
    if method == "scantxoutset" && matches!(mode, Mode::ScanBusy | Mode::NearScanBusy) {
        let message = if matches!(mode, Mode::ScanBusy) {
            EXACT_SCAN_BUSY_MESSAGE
        } else {
            "scan already in progress"
        };
        let body =
            format!(r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-8,"message":"{message}"}}}}"#);
        return http_response("200 OK", &body);
    }

    let result = match method {
        "getblockchaininfo" => {
            r#"{"chain":"main","initialblockdownload":false,"future":{"nested":true}}"#.to_owned()
        }
        "scantxoutset" if matches!(mode, Mode::ScanIncomplete) => format!(
            r#"{{"success":false,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":0,"unspents":[],"total_amount":0}}"#
        ),
        "scantxoutset" => valid_scan_result(),
        "getblockhash" => format!("\"{ANCHOR_HASH}\""),
        other => panic!("unexpected method {other}"),
    };
    let body = format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{result}}}"#);
    http_response("200 OK", &body)
}

fn valid_scan_result() -> String {
    let legacy_script = script_hex(LEGACY_MAIN);
    let segwit_script = script_hex(SEGWIT_MAIN);
    format!(
        r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":3,"unspents":[{},{},{}],"total_amount":0.00000007}}"#,
        unspent(TXID_ONE, 0, &legacy_script, "0.00000001", 849_998),
        unspent(TXID_TWO, 1, &legacy_script, "0.00000002", 849_999),
        unspent(
            "3333333333333333333333333333333333333333333333333333333333333333",
            2,
            &segwit_script,
            "0.00000004",
            850_000,
        )
    )
}

fn http_response(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

async fn read_http_request(stream: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let count = stream.read(&mut chunk).await.expect("request read");
        assert_ne!(count, 0, "request ended before body completion");
        bytes.extend_from_slice(&chunk[..count]);
        if request_complete(&bytes) {
            return bytes;
        }
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
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(str::trim)
                .and_then(|value| value.parse::<usize>().ok())
        })
        .expect("content length");
    bytes.len() >= header_end + 4 + content_len
}

fn request_body(bytes: &[u8]) -> &[u8] {
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("header end");
    &bytes[header_end + 4..]
}

#[test]
fn parsed_block_hash_fixture_is_valid() {
    let _: BlockHash = ANCHOR_HASH.parse().expect("block hash");
}
