use std::sync::{Arc, Mutex};

use alloy_primitives::{address, B256, U256};
use mfm_evm_capabilities::{
    EvmBlockSelector, EvmReadSession, EvmReceiptStatus, EvmTransactionSession,
};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

const BLOCK_HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const OTHER_HASH: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";
const ACCOUNT: Address = address!("1111111111111111111111111111111111111111");

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
                    let request: Value =
                        serde_json::from_slice(request_body(&bytes[..read])).expect("request JSON");
                    captured.lock().expect("requests").push(request.clone());
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

async fn session(server: &TestServer) -> EvmJsonRpcSession {
    EvmJsonRpcSession::bind(
        binding(1),
        LocalPublicId::new("primary").expect("source"),
        server.url.clone(),
        Some("Bearer top-secret".to_owned()),
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
            )
            .expect("call request"),
        )
        .await
        .expect("call");

    assert_eq!(block.number, U256::from(42));
    assert_eq!(balance, U256::from(1_000_000_000_000_000_000_u128));
    assert_eq!(code.bytes, Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]));
    assert_eq!(code.hash, keccak256(&code.bytes));
    assert_eq!(call, Bytes::from_static(&[0x12, 0x34]));
    assert_eq!(
        EvmReadSession::evidence(&session).source_ref().as_str(),
        "primary"
    );
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
        transaction.placement.expect("placement").block_number,
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
    let error = EvmJsonRpcSession::bind(
        binding(1),
        LocalPublicId::new("primary").expect("source"),
        server.url.clone(),
        Some("Bearer top-secret".to_owned()),
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
}

#[tokio::test]
async fn response_version_and_id_are_strict() {
    for mode in [Mode::BadVersion, Mode::BadId] {
        let server = TestServer::spawn(mode).await;
        assert!(matches!(
            EvmJsonRpcSession::bind(
                binding(1),
                LocalPublicId::new("primary").expect("source"),
                server.url,
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
    ] {
        let server = TestServer::spawn(mode).await;
        let error = EvmJsonRpcSession::bind(
            binding(1),
            LocalPublicId::new("primary").expect("source"),
            server.url.clone(),
            Some("Bearer top-secret".to_owned()),
        )
        .await
        .expect_err("bind failure");
        let diagnostic = error.into_provider_diagnostic();
        let rendered = format!("{diagnostic:?} {diagnostic}");
        assert_eq!(diagnostic.code(), code);
        assert!(!rendered.contains("secret provider message"));
        assert!(!rendered.contains("top-secret"));
        assert!(!rendered.contains(&server.url));
    }
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
        return "HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\n\r\n".to_owned();
    }
    let method = request["method"].as_str().expect("method");
    let body = if matches!(mode, Mode::JsonError) {
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {"code": -32601, "message": "secret provider message"},
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
        "eth_getCode" => json!("0xdeadbeef"),
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
