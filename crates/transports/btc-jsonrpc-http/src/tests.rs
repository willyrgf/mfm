use std::sync::{Arc, Mutex};

use mfm_btc_capabilities::{
    BitcoinAddress, BitcoinBalanceCollectionRequest, BitcoinBalanceSession, BitcoinCapabilityError,
    BitcoinNetworkId, BitcoinNetworkTag, BitcoinSourceIdentity,
};
use mfm_capabilities::ProviderDiagnosticCode;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

const LEGACY_MAIN: &str = "1BoatSLRHtKNngkdXEeobR76b53LETtpyT";
const SEGWIT_MAIN: &str = "bc1qvzvkjn4q3nszqxrv3nraga2r822xjty3ykvkuw";
const ANCHOR_HASH: &str = "00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc";
const OTHER_HASH: &str = "00000000000000000002b2a7f3e0d5c4b6a897887766554433221100ffeeddcc";
const TXID_ONE: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const TXID_TWO: &str = "2222222222222222222222222222222222222222222222222222222222222222";

#[derive(Clone, Copy)]
enum Mode {
    Valid,
    Zero,
    ChainMismatch,
    InitialBlockDownload,
    Reorganization,
    ScanBusy,
    NearScanBusy,
    StallScan,
    BadVersion,
    BadId,
    ResultAndError,
    MissingArms,
    HttpFailure,
    Redirect,
    OversizedLength,
    OversizedChunked,
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

fn request() -> BitcoinBalanceCollectionRequest {
    BitcoinBalanceCollectionRequest::new(
        binding(),
        vec![LEGACY_MAIN.to_owned(), SEGWIT_MAIN.to_owned()],
    )
    .expect("request")
}

fn session(server: &TestServer) -> BitcoinRpcSession {
    BitcoinRpcSession::new(&server.url, None, binding(), Duration::from_secs(1)).expect("session")
}

#[tokio::test]
async fn aggregate_collection_is_exactly_one_sorted_scan_between_anchor_checks() {
    let server = TestServer::spawn(Mode::Valid).await;
    let response = session(&server)
        .collect_balances(&request())
        .await
        .expect("collection");

    assert_eq!(response.binding(), &binding());
    assert_eq!(response.anchor_height(), 850_000);
    assert_eq!(response.anchor_hash().to_string(), ANCHOR_HASH);
    assert_eq!(response.final_canonical_hash(), response.anchor_hash());
    assert_eq!(response.balances().len(), 2);
    assert_eq!(response.balances()[0].address(), LEGACY_MAIN);
    assert_eq!(response.balances()[0].balance_sats(), 3);
    assert_eq!(response.balances()[1].address(), SEGWIT_MAIN);
    assert_eq!(response.balances()[1].balance_sats(), 4);

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
    assert!(requests.iter().all(|request| request["jsonrpc"] == "2.0"));
}

#[tokio::test]
async fn aggregate_collection_preseeds_zero_balances_at_one_shared_anchor() {
    let server = TestServer::spawn(Mode::Zero).await;
    let response = session(&server)
        .collect_balances(&request())
        .await
        .expect("collection");

    assert_eq!(
        response
            .balances()
            .iter()
            .map(|balance| balance.balance_sats())
            .collect::<Vec<_>>(),
        [0, 0]
    );
    assert_eq!(response.anchor_hash(), response.final_canonical_hash());
    assert_eq!(server.requests().len(), 3);
}

#[tokio::test]
async fn source_checks_stop_before_scan() {
    for mode in [Mode::ChainMismatch, Mode::InitialBlockDownload] {
        let server = TestServer::spawn(mode).await;
        assert_eq!(
            session(&server)
                .collect_balances(&request())
                .await
                .expect_err("source mismatch"),
            BitcoinCapabilityError::SourceMismatch
        );
        assert_eq!(server.requests().len(), 1);
    }
}

#[tokio::test]
async fn scan_busy_is_retryable_only_for_the_exact_core_classification() {
    for (mode, expected_retryable) in [(Mode::ScanBusy, true), (Mode::NearScanBusy, false)] {
        let server = TestServer::spawn(mode).await;
        let error = session(&server)
            .collect_balances(&request())
            .await
            .expect_err("scan failure");
        let BitcoinCapabilityError::Provider {
            diagnostic,
            retryable,
        } = error
        else {
            panic!("expected provider error");
        };
        assert_eq!(diagnostic.code(), ProviderDiagnosticCode::RpcJsonError);
        assert_eq!(retryable, expected_retryable);
        assert_eq!(server.requests().len(), 2);
    }
}

#[tokio::test]
async fn reorganization_fails_after_the_three_call_observation() {
    let server = TestServer::spawn(Mode::Reorganization).await;
    let error = session(&server)
        .collect_balances(&request())
        .await
        .expect_err("reorganization");
    assert_provider(&error, ProviderDiagnosticCode::ResponseInvalid, false);
    assert_eq!(server.requests().len(), 3);
}

#[tokio::test]
async fn scan_timeout_drops_the_request_without_status_or_abort() {
    let server = TestServer::spawn(Mode::StallScan).await;
    let error = session(&server)
        .collect_balances(&request())
        .await
        .expect_err("timeout");
    assert_provider(&error, ProviderDiagnosticCode::TransportFailed, true);
    assert_eq!(
        server
            .requests()
            .iter()
            .map(|request| request["method"].as_str().expect("method").to_owned())
            .collect::<Vec<_>>(),
        ["getblockchaininfo", "scantxoutset"]
    );
}

#[tokio::test]
async fn cancellation_drops_the_request_without_status_or_abort() {
    let server = TestServer::spawn(Mode::StallScan).await;
    let session = Arc::new(session(&server));
    let request = Arc::new(request());
    let task = {
        let session = Arc::clone(&session);
        let request = Arc::clone(&request);
        tokio::spawn(async move { session.collect_balances(&request).await })
    };
    for _ in 0..100 {
        if server.requests().len() == 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    task.abort();
    assert!(task.await.expect_err("cancelled").is_cancelled());
    assert_eq!(
        server
            .requests()
            .iter()
            .map(|request| request["method"].as_str().expect("method").to_owned())
            .collect::<Vec<_>>(),
        ["getblockchaininfo", "scantxoutset"]
    );
}

#[tokio::test]
async fn protocol_and_http_failures_are_closed_and_redacted() {
    for mode in [
        Mode::BadVersion,
        Mode::BadId,
        Mode::ResultAndError,
        Mode::MissingArms,
        Mode::OversizedLength,
        Mode::OversizedChunked,
    ] {
        let server = TestServer::spawn(mode).await;
        let error = session(&server)
            .collect_balances(&request())
            .await
            .expect_err("invalid response");
        assert_provider(&error, ProviderDiagnosticCode::ResponseInvalid, false);
    }

    let server = TestServer::spawn(Mode::HttpFailure).await;
    let error = session(&server)
        .collect_balances(&request())
        .await
        .expect_err("HTTP failure");
    assert_provider(&error, ProviderDiagnosticCode::RpcHttpStatus, true);
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains("provider-secret"));

    let server = TestServer::spawn(Mode::Redirect).await;
    let error = session(&server)
        .collect_balances(&request())
        .await
        .expect_err("redirect");
    assert_provider(&error, ProviderDiagnosticCode::RpcHttpStatus, false);
    assert_eq!(server.requests().len(), 1, "redirects must not be followed");
}

#[test]
fn configuration_and_debug_surfaces_reject_or_redact_secret_bearing_inputs() {
    for endpoint in [
        "ftp://127.0.0.1",
        "http://user:password@127.0.0.1",
        "http://127.0.0.1?secret=value",
        "http://127.0.0.1#secret",
    ] {
        assert!(matches!(
            BitcoinRpcSession::new(endpoint, None, binding(), Duration::from_secs(1)),
            Err(BitcoinRpcError::InvalidConfiguration)
        ));
    }
    for timeout in [Duration::ZERO, Duration::from_secs(86_401)] {
        assert!(matches!(
            BitcoinRpcSession::new("http://127.0.0.1", None, binding(), timeout),
            Err(BitcoinRpcError::InvalidConfiguration)
        ));
    }
    assert!(matches!(
        BitcoinRpcAuthentication::new("", "password"),
        Err(BitcoinRpcError::InvalidConfiguration)
    ));

    let authentication =
        BitcoinRpcAuthentication::new("secret-user", "secret-password").expect("authentication");
    let session = BitcoinRpcSession::new(
        "http://127.0.0.1:18443/secret-path",
        Some(authentication.clone()),
        binding(),
        Duration::from_secs(1),
    )
    .expect("session");
    let rendered = format!("{authentication:?} {session:?}");
    for secret in ["secret-user", "secret-password", "secret-path", "18443"] {
        assert!(!rendered.contains(secret));
    }
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
    let duplicate = format!("{first},{first}");
    assert_reduce_invalid(&valid(&duplicate, "0.00000002"));

    let unknown = unspent(TXID_TWO, 0, "00", "0.00000001", 849_999);
    assert_reduce_invalid(&valid(&unknown, "0.00000001"));

    let known = format!(
        "{},{}",
        first,
        unspent(TXID_TWO, 1, &segwit_script, "0.00000002", 850_000)
    );
    assert_reduce_invalid(&valid(&known, "0.00000004"));
    assert_reduce_invalid(&valid(
        &unspent(TXID_TWO, u64::from(u32::MAX) + 1, &legacy_script, "1", 1),
        "1",
    ));
    assert_reduce_invalid(&valid(
        &unspent(TXID_TWO, 0, &legacy_script, "1", 850_001),
        "1",
    ));
}

#[test]
fn strict_decoders_require_known_fields_and_reject_duplicate_members_at_any_depth() {
    let required = format!(
        r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","unspents":[],"total_amount":0}}"#
    );
    assert!(matches!(
        decode_unique::<ScanTxOutSetResult>(required.as_bytes()),
        Err(BitcoinRpcError::ResponseInvalid)
    ));
    let negative_txouts = format!(
        r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":-1,"unspents":[],"total_amount":0}}"#
    );
    assert!(matches!(
        decode_unique::<ScanTxOutSetResult>(negative_txouts.as_bytes()),
        Err(BitcoinRpcError::ResponseInvalid)
    ));

    for duplicate in [
        br#"{"jsonrpc":"2.0","id":1,"id":1,"result":{}}"#.as_slice(),
        br#"{"jsonrpc":"2.0","id":1,"error":{"code":-8,"code":-8,"message":"x"}}"#,
    ] {
        assert!(matches!(
            decode_unique::<RpcEnvelope>(duplicate),
            Err(BitcoinRpcError::ResponseInvalid)
        ));
    }

    let script = script_hex(LEGACY_MAIN);
    let duplicate_unspent = format!(
        r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":1,"unspents":[{{"txid":"{TXID_ONE}","vout":0,"scriptPubKey":"{script}","scriptPubKey":"{script}","desc":"ignored","amount":1,"height":1}}],"total_amount":1}}"#
    );
    assert!(matches!(
        decode_unique::<ScanTxOutSetResult>(duplicate_unspent.as_bytes()),
        Err(BitcoinRpcError::ResponseInvalid)
    ));

    let additive = format!(
        r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":0,"unspents":[],"total_amount":0,"future":{{"nested":{{"unique":true}}}}}}"#
    );
    let decoded =
        decode_unique::<ScanTxOutSetResult>(additive.as_bytes()).expect("additive fields");
    assert!(reduce_scan(&request(), decoded).is_ok());

    let duplicate_unknown =
        br#"{"chain":"main","initialblockdownload":false,"future":{"value":1,"value":2}}"#;
    assert!(matches!(
        decode_unique::<BlockchainInfo>(duplicate_unknown),
        Err(BitcoinRpcError::ResponseInvalid)
    ));

    let oversized_message = format!(
        r#"{{"jsonrpc":"2.0","id":1,"error":{{"code":-1,"message":"{}"}}}}"#,
        "x".repeat(MAX_JSON_RPC_ERROR_MESSAGE_BYTES + 1)
    );
    assert!(matches!(
        decode_unique::<RpcEnvelope>(oversized_message.as_bytes()),
        Err(BitcoinRpcError::ResponseInvalid)
    ));
}

#[test]
fn remaining_scan_semantics_fail_closed_before_response_construction() {
    for malformed in ["0", "zz"] {
        assert_eq!(
            decode_script(malformed),
            Err(BitcoinRpcError::ResponseInvalid)
        );
    }
    assert_eq!(
        decode_script(&"00".repeat(MAX_SCRIPT_BYTES + 1)),
        Err(BitcoinRpcError::ResponseInvalid)
    );

    let incomplete = format!(
        r#"{{"success":false,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":0,"unspents":[],"total_amount":0}}"#
    );
    let scan = decode_unique::<ScanTxOutSetResult>(incomplete.as_bytes()).expect("typed scan");
    assert!(matches!(
        reduce_scan(&request(), scan),
        Err(BitcoinRpcError::OperationIncomplete)
    ));

    let legacy_script = script_hex(LEGACY_MAIN);
    for invalid in [
        format!(
            r#"{{"success":true,"height":850000,"bestblock":"invalid","txouts":1,"unspents":[],"total_amount":0}}"#
        ),
        format!(
            r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":1,"unspents":[{}],"total_amount":0.00000001}}"#,
            unspent("invalid", 0, &legacy_script, "0.00000001", 1)
        ),
        format!(
            r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":1,"unspents":[{}],"total_amount":0.00000001}}"#,
            unspent(TXID_ONE, 0, "0", "0.00000001", 1)
        ),
        format!(
            r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":1,"unspents":[{{"txid":"{TXID_ONE}","vout":0,"scriptPubKey":"{legacy_script}","desc":"{}","amount":0.00000001,"height":1}}],"total_amount":0.00000001}}"#,
            "x".repeat(MAX_DESCRIPTOR_BYTES + 1)
        ),
    ] {
        assert_reduce_invalid(&invalid);
    }

    for malformed_txouts in [
        format!(
            r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":"1","unspents":[],"total_amount":0}}"#
        ),
        format!(
            r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":18446744073709551616,"unspents":[],"total_amount":0}}"#
        ),
    ] {
        assert!(matches!(
            decode_unique::<ScanTxOutSetResult>(malformed_txouts.as_bytes()),
            Err(BitcoinRpcError::ResponseInvalid)
        ));
    }
}

fn assert_reduce_invalid(raw: &str) {
    let scan = decode_unique::<ScanTxOutSetResult>(raw.as_bytes()).expect("typed scan");
    assert!(matches!(
        reduce_scan(&request(), scan),
        Err(BitcoinRpcError::ResponseInvalid)
    ));
}

fn assert_provider(
    error: &BitcoinCapabilityError,
    expected_code: ProviderDiagnosticCode,
    expected_retryable: bool,
) {
    let BitcoinCapabilityError::Provider {
        diagnostic,
        retryable,
    } = error
    else {
        panic!("expected provider error");
    };
    assert_eq!(diagnostic.code(), expected_code);
    assert_eq!(*retryable, expected_retryable);
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
        return http_response(
            "500 Internal Server Error",
            r#"{"provider":"provider-secret"}"#,
        );
    }
    if matches!(mode, Mode::Redirect) {
        return "HTTP/1.1 302 Found\r\nlocation: http://127.0.0.1:9/provider-secret\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned();
    }
    if matches!(mode, Mode::OversizedLength) {
        return format!(
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            MAX_BITCOIN_JSON_RPC_BODY_BYTES + 1
        );
    }
    if matches!(mode, Mode::OversizedChunked) {
        let body = "x".repeat(MAX_BITCOIN_JSON_RPC_BODY_BYTES + 1);
        return format!(
            "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",
            body.len(),
            body
        );
    }

    let id = request["id"].as_u64().expect("request ID");
    if matches!(mode, Mode::BadVersion) {
        return json_response(
            id,
            "1.1",
            r#"{"chain":"main","initialblockdownload":false}"#,
        );
    }
    if matches!(mode, Mode::BadId) {
        return json_response(
            id + 1,
            "2.0",
            r#"{"chain":"main","initialblockdownload":false}"#,
        );
    }
    if matches!(mode, Mode::ResultAndError) {
        let body = format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{}},"error":{{"code":-1,"message":"provider-secret"}}}}"#
        );
        return http_response("200 OK", &body);
    }
    if matches!(mode, Mode::MissingArms) {
        let body = format!(r#"{{"jsonrpc":"2.0","id":{id}}}"#);
        return http_response("200 OK", &body);
    }

    let method = request["method"].as_str().expect("method");
    if method == "scantxoutset" && matches!(mode, Mode::ScanBusy | Mode::NearScanBusy) {
        let message = if matches!(mode, Mode::ScanBusy) {
            "Scan already in progress: use status to check"
        } else {
            "scan already in progress"
        };
        let body = format!(
            r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":-8,"message":"{message}","data":"provider-secret"}}}}"#
        );
        return http_response("200 OK", &body);
    }

    let result = match method {
        "getblockchaininfo" => {
            let chain = if matches!(mode, Mode::ChainMismatch) {
                "test"
            } else {
                "main"
            };
            let ibd = matches!(mode, Mode::InitialBlockDownload);
            format!(
                r#"{{"chain":"{chain}","initialblockdownload":{ibd},"blocks":850000,"bestblockhash":"{ANCHOR_HASH}","future":{{"nested":true}}}}"#
            )
        }
        "scantxoutset" if matches!(mode, Mode::Zero) => format!(
            r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":99,"unspents":[],"total_amount":0,"future":true}}"#
        ),
        "scantxoutset" => valid_scan_result(),
        "getblockhash" => format!(
            "\"{}\"",
            if matches!(mode, Mode::Reorganization) {
                OTHER_HASH
            } else {
                ANCHOR_HASH
            }
        ),
        other => panic!("unexpected method {other}"),
    };
    json_response(id, "2.0", &result)
}

fn valid_scan_result() -> String {
    let legacy_script = script_hex(LEGACY_MAIN);
    let segwit_script = script_hex(SEGWIT_MAIN);
    format!(
        r#"{{"success":true,"height":850000,"bestblock":"{ANCHOR_HASH}","txouts":12345,"unspents":[{},{},{}],"total_amount":0.00000007,"future":{{"nested":true}}}}"#,
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

fn json_response(id: u64, version: &str, result: &str) -> String {
    let body = format!(r#"{{"jsonrpc":"{version}","id":{id},"result":{result}}}"#);
    http_response("200 OK", &body)
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
        assert_ne!(count, 0, "request ended before the body was complete");
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
