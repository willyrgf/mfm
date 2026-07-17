use super::*;

pub(super) struct TestRpcServer {
    pub(super) url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl TestRpcServer {
    pub(super) async fn spawn(chain_id: &'static str) -> Self {
        Self::spawn_with_mode(TestRpcMode::Ok { chain_id }).await
    }

    pub(super) async fn spawn_legacy_fee(chain_id: &'static str) -> Self {
        Self::spawn_with_mode(TestRpcMode::LegacyFee { chain_id }).await
    }

    pub(super) async fn spawn_pending_receipt(chain_id: &'static str) -> Self {
        Self::spawn_with_mode(TestRpcMode::PendingReceipt { chain_id }).await
    }

    pub(super) async fn spawn_empty_code(chain_id: &'static str) -> Self {
        Self::spawn_with_mode(TestRpcMode::EmptyCode { chain_id }).await
    }

    pub(super) async fn spawn_malformed_code(chain_id: &'static str) -> Self {
        Self::spawn_with_mode(TestRpcMode::MalformedCode { chain_id }).await
    }

    pub(super) async fn spawn_block_identity_mismatch(chain_id: &'static str) -> Self {
        Self::spawn_with_mode(TestRpcMode::BlockIdentityMismatch { chain_id }).await
    }

    pub(super) async fn spawn_receipt_hash_mismatch(chain_id: &'static str) -> Self {
        Self::spawn_with_mode(TestRpcMode::ReceiptHashMismatch { chain_id }).await
    }

    pub(super) async fn spawn_receipt_missing_block_hash(chain_id: &'static str) -> Self {
        Self::spawn_with_mode(TestRpcMode::ReceiptMissingBlockHash { chain_id }).await
    }

    pub(super) async fn spawn_failure() -> Self {
        Self::spawn_with_mode(TestRpcMode::Failure).await
    }

    pub(super) async fn spawn_json_rpc_failure() -> Self {
        Self::spawn_with_mode(TestRpcMode::JsonRpcFailure).await
    }

    async fn spawn_with_mode(mode: TestRpcMode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&requests);
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let captured = Arc::clone(&captured);
                tokio::spawn(async move {
                    let mut buffer = vec![0_u8; 8192];
                    let mut read = 0_usize;
                    loop {
                        let n = stream.read(&mut buffer[read..]).await.expect("read");
                        if n == 0 {
                            return;
                        }
                        read += n;
                        if request_complete(&buffer[..read]) {
                            break;
                        }
                    }
                    let body = request_body(&buffer[..read]);
                    let request: Value = serde_json::from_slice(body).expect("json request");
                    let method = request
                        .get("method")
                        .and_then(Value::as_str)
                        .expect("method")
                        .to_owned();
                    captured.lock().expect("requests").push(method.clone());
                    let response = match mode {
                        TestRpcMode::Ok { chain_id } => {
                            let result = rpc_result(chain_id, &method);
                            let body = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": 1,
                                "result": result,
                            })
                            .to_string();
                            format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                                body.len(),
                                body
                            )
                        }
                        TestRpcMode::LegacyFee { chain_id } => {
                            let body = if method == "eth_maxPriorityFeePerGas" {
                                serde_json::json!({
                                    "jsonrpc": "2.0",
                                    "id": 1,
                                    "error": {
                                        "code": -32601,
                                        "message": "method not found",
                                    },
                                })
                            } else {
                                serde_json::json!({
                                    "jsonrpc": "2.0",
                                    "id": 1,
                                    "result": legacy_fee_rpc_result(chain_id, &method),
                                })
                            }
                            .to_string();
                            format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                                body.len(),
                                body
                            )
                        }
                        TestRpcMode::PendingReceipt { chain_id } => {
                            let result = if method == "eth_getTransactionReceipt" {
                                serde_json::Value::Null
                            } else {
                                rpc_result(chain_id, &method)
                            };
                            let body = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": 1,
                                "result": result,
                            })
                            .to_string();
                            format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                                body.len(),
                                body
                            )
                        }
                        TestRpcMode::EmptyCode { chain_id } => {
                            let result = if method == "eth_getCode" {
                                json!("0x")
                            } else {
                                rpc_result(chain_id, &method)
                            };
                            let body = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": 1,
                                "result": result,
                            })
                            .to_string();
                            format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                                body.len(),
                                body
                            )
                        }
                        TestRpcMode::MalformedCode { chain_id } => {
                            let result = if method == "eth_getCode" {
                                json!("0xnot-hex")
                            } else {
                                rpc_result(chain_id, &method)
                            };
                            let body = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": 1,
                                "result": result,
                            })
                            .to_string();
                            format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                                body.len(),
                                body
                            )
                        }
                        TestRpcMode::BlockIdentityMismatch { chain_id } => {
                            let result = block_identity_mismatch_rpc_result(chain_id, &method);
                            let body = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": 1,
                                "result": result,
                            })
                            .to_string();
                            format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                                body.len(),
                                body
                            )
                        }
                        TestRpcMode::ReceiptHashMismatch { chain_id } => {
                            let result = receipt_hash_mismatch_rpc_result(chain_id, &method);
                            let body = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": 1,
                                "result": result,
                            })
                            .to_string();
                            format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                                body.len(),
                                body
                            )
                        }
                        TestRpcMode::ReceiptMissingBlockHash { chain_id } => {
                            let result = if method == "eth_getTransactionReceipt" {
                                json!({
                                    "transactionHash": HASH_HEX,
                                    "blockNumber": "0x2a",
                                    "status": "0x1",
                                })
                            } else {
                                rpc_result(chain_id, &method)
                            };
                            let body = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": 1,
                                "result": result,
                            })
                            .to_string();
                            format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                                body.len(),
                                body
                            )
                        }
                        TestRpcMode::Failure => {
                            "HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\n\r\n"
                                .to_owned()
                        }
                        TestRpcMode::JsonRpcFailure => {
                            let body = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": 1,
                                "error": {
                                    "code": -32601,
                                    "message": "secret provider message",
                                },
                            })
                            .to_string();
                            format!(
                                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                                body.len(),
                                body
                            )
                        }
                    };
                    stream.write_all(response.as_bytes()).await.expect("write");
                });
            }
        });
        Self {
            url: format!("http://{addr}"),
            requests,
        }
    }

    pub(super) fn methods(&self) -> Vec<String> {
        self.requests.lock().expect("requests").clone()
    }
}

#[derive(Clone, Copy)]
enum TestRpcMode {
    Ok { chain_id: &'static str },
    LegacyFee { chain_id: &'static str },
    PendingReceipt { chain_id: &'static str },
    EmptyCode { chain_id: &'static str },
    MalformedCode { chain_id: &'static str },
    BlockIdentityMismatch { chain_id: &'static str },
    ReceiptHashMismatch { chain_id: &'static str },
    ReceiptMissingBlockHash { chain_id: &'static str },
    Failure,
    JsonRpcFailure,
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
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);
    bytes.len() >= header_end + 4 + content_len
}

fn request_body(bytes: &[u8]) -> &[u8] {
    let header_end = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("headers");
    &bytes[header_end + 4..]
}

fn rpc_result(chain_id: &str, method: &str) -> Value {
    match method {
        "eth_chainId" => json!(chain_id),
        "web3_clientVersion" => json!("mfm-test-rpc"),
        "eth_getBlockByNumber" => json!({
            "number": "0x2a",
            "hash": HASH_HEX,
            "baseFeePerGas": "0x20",
        }),
        "eth_getBlockByHash" => json!({
            "number": "0x2a",
            "hash": HASH_HEX,
        }),
        "eth_getBalance" => json!("0xde0b6b3a7640000"),
        "eth_call" => json!("0x1234"),
        "eth_getCode" => json!("0xdeadbeef"),
        "eth_getTransactionCount" => json!("0x7"),
        "eth_gasPrice" => json!("0x10"),
        "eth_maxPriorityFeePerGas" => json!("0x2"),
        "eth_estimateGas" => json!("0x5208"),
        "eth_sendRawTransaction" => json!(HASH_HEX),
        "eth_getTransactionReceipt" => json!({
            "transactionHash": HASH_HEX,
            "blockNumber": "0x2a",
            "blockHash": HASH_HEX,
            "status": "0x1",
        }),
        other => panic!("unexpected method {other}"),
    }
}

fn legacy_fee_rpc_result(chain_id: &str, method: &str) -> Value {
    match method {
        "eth_chainId" => json!(chain_id),
        "eth_gasPrice" => json!("0x10"),
        "eth_getBlockByNumber" => json!({
            "number": "0x2a",
            "hash": HASH_HEX,
        }),
        other => panic!("unexpected legacy fee method {other}"),
    }
}

fn block_identity_mismatch_rpc_result(chain_id: &str, method: &str) -> Value {
    match method {
        "eth_chainId" => json!(chain_id),
        "eth_getBlockByNumber" => json!({
            "number": "0x2b",
            "hash": HASH_HEX,
        }),
        "eth_getBlockByHash" => json!({
            "number": "0x2a",
            "hash": OCCUPYING_HASH_HEX,
        }),
        other => rpc_result(chain_id, other),
    }
}

fn receipt_hash_mismatch_rpc_result(chain_id: &str, method: &str) -> Value {
    match method {
        "eth_chainId" => json!(chain_id),
        "eth_getTransactionReceipt" => json!({
            "transactionHash": OCCUPYING_HASH_HEX,
            "blockNumber": "0x2a",
            "blockHash": HASH_HEX,
            "status": "0x1",
        }),
        other => rpc_result(chain_id, other),
    }
}
