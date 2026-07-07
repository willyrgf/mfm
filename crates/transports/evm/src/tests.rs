use super::*;
use alloy_primitives::{address, B256, U256};
use mfm_evm_capabilities::{EvmTransactionSubmitRequest, SignedEvmPayload};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const HASH_HEX: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const OCCUPYING_HASH_HEX: &str =
    "0x2222222222222222222222222222222222222222222222222222222222222222";

fn evm_response_invalid_error() -> EvmCapabilityError {
    EvmCapabilityError::provider_failure(evm_diagnostic(ProviderDiagnosticCode::ResponseInvalid))
}

#[tokio::test]
async fn selects_source_by_policy_and_records_redacted_evidence() {
    let server = TestRpcServer::spawn("0x1").await;
    let client = client_for(&server.url, "primary", "mainnet");

    let response = client
        .chain_identity(&chain_request("mainnet", 1))
        .await
        .expect("chain identity");

    assert_eq!(response.chain_id, 1);
    assert_eq!(response.evidence.network_id.as_str(), "mainnet");
    assert_eq!(response.evidence.expected_chain_id, 1);
    assert_eq!(response.evidence.observed_chain_id, 1);
    assert_eq!(response.evidence.source_ref.as_str(), "primary");
    assert_eq!(response.evidence.policy_id.as_str(), "mainnet");
    assert!(!format!("{:?}", response.evidence).contains(&server.url));
}

#[tokio::test]
async fn rejects_chain_id_mismatch_without_leaking_source_details() {
    let server = TestRpcServer::spawn("0x2").await;
    let client = client_for(&server.url, "primary", "mainnet");

    let error = client
        .chain_identity(&chain_request("mainnet", 1))
        .await
        .expect_err("source mismatch");

    let rendered = format!("{error:?} {error}");
    let EvmCapabilityError::SourceMismatch { diagnostic } = error else {
        panic!("expected source mismatch error");
    };
    assert_eq!(diagnostic.stable_error_code(), "evm_source_mismatch");
    assert_eq!(
        diagnostic.to_public_details_json()["fields"],
        serde_json::json!({
            "expected_chain_id": 1,
            "network_id": "mainnet",
            "observed_chain_id": 2,
            "policy_id": "mainnet",
            "source_ref": "primary",
        })
    );
    assert!(!rendered.contains(&server.url));
    assert!(!rendered.contains("Bearer"));
}

#[tokio::test]
async fn classifies_http_status_failure_without_body() {
    let server = TestRpcServer::spawn_failure().await;
    let client = client_for(&server.url, "primary", "mainnet");

    let error = client
        .chain_identity(&chain_request("mainnet", 1))
        .await
        .expect_err("provider failure");
    let rendered = format!("{error:?} {error}");
    let EvmCapabilityError::Provider { diagnostic } = error else {
        panic!("expected provider diagnostic");
    };

    assert_eq!(diagnostic.stable_error_code(), "evm_rpc_http_status");
    assert_eq!(
        diagnostic.summary(),
        "rpc_http_status operation=eth_chain_id http_status=500"
    );
    assert!(!rendered.contains(&server.url));
    assert!(!rendered.contains("top-secret"));
}

#[tokio::test]
async fn classifies_json_rpc_failure_without_message() {
    let server = TestRpcServer::spawn_json_rpc_failure().await;
    let client = client_for(&server.url, "primary", "mainnet");

    let error = client
        .chain_identity(&chain_request("mainnet", 1))
        .await
        .expect_err("provider failure");
    let rendered = format!("{error:?} {error}");
    let EvmCapabilityError::Provider { diagnostic } = error else {
        panic!("expected provider diagnostic");
    };

    assert_eq!(diagnostic.stable_error_code(), "evm_rpc_json_error");
    assert_eq!(
        diagnostic.summary(),
        "rpc_json_error operation=eth_chain_id rpc_code=-32601"
    );
    assert!(!rendered.contains("secret provider message"));
    assert!(!rendered.contains("top-secret"));
}

#[tokio::test]
async fn supports_core_evm_json_rpc_calls() {
    let server = TestRpcServer::spawn("0x1").await;
    let client = client_for(&server.url, "primary", "mainnet");
    let guard = guard("mainnet", 1);
    let address = address!("0x1111111111111111111111111111111111111111");
    let hash = HASH_HEX.parse::<B256>().expect("hash");

    let block = client
        .read_block(&EvmBlockReadRequest {
            guard: guard.clone(),
            block: EvmBlockSelector::Latest,
        })
        .await
        .expect("block");
    assert_eq!(block.block_number, 42);

    let balance = client
        .read_balance(&EvmBalanceReadRequest {
            guard: guard.clone(),
            account: address,
            block: EvmBlockSelector::Latest,
        })
        .await
        .expect("balance");
    assert_eq!(
        balance.balance_wei,
        U256::from(1_000_000_000_000_000_000u128)
    );

    let call = client
        .read_call(&EvmCallReadRequest {
            guard: guard.clone(),
            to: address,
            calldata: vec![0xab, 0xcd],
            block: EvmBlockSelector::Latest,
        })
        .await
        .expect("call");
    assert_eq!(call.return_data, vec![0x12, 0x34]);

    let code = client
        .read_code(&EvmCodeReadRequest {
            guard: guard.clone(),
            address,
            block: EvmBlockSelector::Latest,
        })
        .await
        .expect("code");
    assert_eq!(code.code, vec![0xde, 0xad, 0xbe, 0xef]);
    assert_eq!(code.code_hash, alloy_primitives::keccak256(&code.code));

    let logs = client
        .read_logs(&EvmLogsReadRequest {
            guard: guard.clone(),
            from_block: EvmBlockSelector::Latest,
            to_block: EvmBlockSelector::Latest,
            address: Some(address),
            topics: vec![hash],
        })
        .await
        .expect("logs");
    assert_eq!(logs.logs.len(), 1);

    let nonce = client
        .read_nonce(&EvmNonceReadRequest {
            guard: guard.clone(),
            account: address,
            block: EvmBlockSelector::Latest,
        })
        .await
        .expect("nonce");
    assert_eq!(nonce.nonce, 7);

    let fee = client
        .read_fee(&EvmFeeReadRequest {
            guard: guard.clone(),
        })
        .await
        .expect("fee");
    assert_eq!(fee.legacy_gas_price, Some(16));
    assert_eq!(fee.priority_fee_per_gas, Some(2));
    assert_eq!(fee.base_fee_per_gas, Some(32));
    assert_eq!(fee.max_fee_per_gas, Some(66));

    let gas = client
        .estimate_gas(&EvmGasEstimateRequest {
            guard: guard.clone(),
            from: Some(address),
            to: Some(address),
            value_wei: 0,
            data: vec![0xab],
        })
        .await
        .expect("gas");
    assert_eq!(gas.gas_limit, 21_000);

    let submit = client
        .submit_transaction(&EvmTransactionSubmitRequest {
            guard: guard.clone(),
            signed_payload: SignedEvmPayload::from_verified_bytes(vec![0x01], hash)
                .expect("payload"),
        })
        .await
        .expect("submit");
    assert_eq!(submit.transaction_hash, hash);

    let receipt = client
        .read_receipt(&EvmReceiptReadRequest {
            guard,
            transaction_hash: hash,
        })
        .await
        .expect("receipt");
    assert_eq!(receipt.block_number, 42);
    assert!(receipt.status);

    assert_eq!(
        server.methods(),
        [
            "eth_chainId",
            "eth_getBlockByNumber",
            "eth_chainId",
            "eth_getBalance",
            "eth_chainId",
            "eth_call",
            "eth_chainId",
            "eth_getCode",
            "eth_chainId",
            "eth_getLogs",
            "eth_chainId",
            "eth_getTransactionCount",
            "eth_chainId",
            "eth_gasPrice",
            "eth_maxPriorityFeePerGas",
            "eth_getBlockByNumber",
            "eth_chainId",
            "eth_estimateGas",
            "eth_chainId",
            "eth_sendRawTransaction",
            "eth_chainId",
            "eth_getTransactionReceipt",
        ]
    );
}

#[test]
fn block_selector_tag_supports_pending_nonce_reads() {
    assert_eq!(block_selector_tag(&EvmBlockSelector::Pending), "pending");
}

#[tokio::test]
async fn pending_receipt_is_typed_capability_error() {
    let server = TestRpcServer::spawn_pending_receipt("0x1").await;
    let client = client_for(&server.url, "primary", "mainnet");
    let error = client
        .read_receipt(&EvmReceiptReadRequest {
            guard: guard("mainnet", 1),
            transaction_hash: HASH_HEX.parse::<B256>().expect("hash"),
        })
        .await
        .expect_err("pending receipt");

    assert_eq!(error, EvmCapabilityError::ReceiptPending);
}

#[tokio::test]
async fn code_read_preserves_empty_code_observation() {
    let server = TestRpcServer::spawn_empty_code("0x1").await;
    let client = client_for(&server.url, "primary", "mainnet");
    let response = client
        .read_code(&EvmCodeReadRequest {
            guard: guard("mainnet", 1),
            address: address!("0x1111111111111111111111111111111111111111"),
            block: EvmBlockSelector::Latest,
        })
        .await
        .expect("empty code response");

    assert!(response.code.is_empty());
    assert_eq!(response.code_hash, alloy_primitives::keccak256([]));
}

#[tokio::test]
async fn code_read_rejects_chain_id_mismatch_without_code_authority() {
    let server = TestRpcServer::spawn("0x2").await;
    let client = client_for(&server.url, "primary", "mainnet");
    let error = client
        .read_code(&EvmCodeReadRequest {
            guard: guard("mainnet", 1),
            address: address!("0x1111111111111111111111111111111111111111"),
            block: EvmBlockSelector::Latest,
        })
        .await
        .expect_err("source mismatch");

    assert!(matches!(error, EvmCapabilityError::SourceMismatch { .. }));
    assert_eq!(server.methods(), ["eth_chainId"]);
}

#[tokio::test]
async fn nonce_occupancy_read_classifies_anchor_and_non_anchor_transactions() {
    for (name, excluded_transaction_hash, expected) in [
        (
            "non-anchor transaction",
            HASH_HEX,
            EvmNonceOccupancy::Occupied {
                transaction_hash: OCCUPYING_HASH_HEX.parse::<B256>().expect("occupying hash"),
                block_number: Some(42),
            },
        ),
        (
            "recorded anchor",
            OCCUPYING_HASH_HEX,
            EvmNonceOccupancy::Unknown,
        ),
    ] {
        let server = TestRpcServer::spawn_nonce_occupancy("0x1").await;
        let client = client_for(&server.url, "primary", "mainnet");
        let response = client
            .read_nonce_occupancy(&EvmNonceOccupancyReadRequest {
                guard: guard("mainnet", 1),
                account: address!("0x1111111111111111111111111111111111111111"),
                nonce: 7,
                excluded_transaction_hash: excluded_transaction_hash
                    .parse::<B256>()
                    .expect("excluded hash"),
            })
            .await
            .expect("nonce occupancy");

        assert_eq!(response.evidence.observed_chain_id, 1, "{name}");
        assert_eq!(response.outcome, expected, "{name}");
    }
}

#[tokio::test]
async fn supports_legacy_fee_source_without_eip1559_methods() {
    let server = TestRpcServer::spawn_legacy_fee("0x1").await;
    let client = client_for(&server.url, "primary", "mainnet");

    let fee = client
        .read_fee(&EvmFeeReadRequest {
            guard: guard("mainnet", 1),
        })
        .await
        .expect("legacy fee response");

    assert_eq!(fee.legacy_gas_price, Some(16));
    assert_eq!(fee.priority_fee_per_gas, None);
    assert_eq!(fee.base_fee_per_gas, None);
    assert_eq!(fee.max_fee_per_gas, None);
}

#[test]
fn route_binding_validation_does_not_require_guard_or_live_io() {
    let client = client_for("http://127.0.0.1:1", "primary", "mainnet");

    client
        .validate_route_binding(&EvmNetworkId::new("mainnet").expect("network"))
        .expect("route binding");

    let missing = client
        .validate_route_binding(&EvmNetworkId::new("sepolia").expect("network"))
        .expect_err("missing route");
    assert_eq!(missing, EvmTransportError::RouteUnavailable);
}

#[tokio::test]
async fn rejects_explicit_block_identity_mismatch() {
    let server = TestRpcServer::spawn_block_identity_mismatch("0x1").await;
    let client = client_for(&server.url, "primary", "mainnet");
    let guard = guard("mainnet", 1);

    let number_error = client
        .read_block(&EvmBlockReadRequest {
            guard: guard.clone(),
            block: EvmBlockSelector::Number(42),
        })
        .await
        .expect_err("number mismatch");
    assert_eq!(number_error, evm_response_invalid_error());

    let hash_error = client
        .read_block(&EvmBlockReadRequest {
            guard,
            block: EvmBlockSelector::Hash(HASH_HEX.parse::<B256>().expect("hash")),
        })
        .await
        .expect_err("hash mismatch");
    assert_eq!(hash_error, evm_response_invalid_error());
}

#[tokio::test]
async fn rejects_receipt_transaction_hash_mismatch() {
    let server = TestRpcServer::spawn_receipt_hash_mismatch("0x1").await;
    let client = client_for(&server.url, "primary", "mainnet");

    let error = client
        .read_receipt(&EvmReceiptReadRequest {
            guard: guard("mainnet", 1),
            transaction_hash: HASH_HEX.parse::<B256>().expect("hash"),
        })
        .await
        .expect_err("receipt hash mismatch");

    assert_eq!(error, evm_response_invalid_error());
}

#[tokio::test]
async fn rejects_log_entries_that_contradict_filter() {
    let server = TestRpcServer::spawn_log_filter_mismatch("0x1").await;
    let client = client_for(&server.url, "primary", "mainnet");

    let error = client
        .read_logs(&EvmLogsReadRequest {
            guard: guard("mainnet", 1),
            from_block: EvmBlockSelector::Number(42),
            to_block: EvmBlockSelector::Number(42),
            address: Some(address!("0x1111111111111111111111111111111111111111")),
            topics: vec![HASH_HEX.parse::<B256>().expect("hash")],
        })
        .await
        .expect_err("log filter mismatch");

    assert_eq!(error, evm_response_invalid_error());
}

#[tokio::test]
async fn runtime_sources_redact_url_and_authorization() {
    let server = TestRpcServer::spawn("0x1").await;
    let source = EvmRuntimeSource::new(
        EvmSourceRef::new("primary").expect("source"),
        &server.url,
        Some("Bearer top-secret".to_owned()),
    )
    .expect("source");
    let registry = EvmSourceRegistry::single_source(
        source,
        EvmSourcePolicyId::new("mainnet").expect("policy"),
    )
    .expect("registry");
    let rendered = format!("{registry:?}");

    assert!(!rendered.contains(&server.url));
    assert!(!rendered.contains("top-secret"));
}

#[tokio::test]
async fn ordered_policy_falls_back_after_request_failure() {
    let failing = TestRpcServer::spawn_failure().await;
    let healthy = TestRpcServer::spawn("0x1").await;
    let primary = EvmRuntimeSource::new(
        EvmSourceRef::new("primary").expect("source"),
        &failing.url,
        None,
    )
    .expect("primary source");
    let secondary = EvmRuntimeSource::new(
        EvmSourceRef::new("secondary").expect("source"),
        &healthy.url,
        None,
    )
    .expect("secondary source");
    let policy = EvmSourcePolicy::new(
        EvmSourcePolicyId::new("mainnet").expect("policy"),
        vec![
            EvmSourceRef::new("primary").expect("primary"),
            EvmSourceRef::new("secondary").expect("secondary"),
        ],
    )
    .expect("policy");
    let client = EvmJsonRpcClient::new(
        EvmSourceRegistry::new([primary, secondary], [policy]).expect("registry"),
        EvmRouteRegistry::new([EvmRoute::new(
            EvmNetworkId::new("mainnet").expect("network"),
            EvmSourceRef::new("primary").expect("source"),
            EvmSourcePolicyId::new("mainnet").expect("policy"),
        )])
        .expect("routes"),
    );

    let response = client
        .chain_identity(&chain_request("mainnet", 1))
        .await
        .expect("fallback response");

    assert_eq!(response.evidence.source_ref.as_str(), "secondary");
    assert_eq!(response.chain_id, 1);
}

fn client_for(url: &str, source_id: &str, policy_id: &str) -> EvmJsonRpcClient {
    let source = EvmRuntimeSource::new(
        EvmSourceRef::new(source_id).expect("source"),
        url,
        Some("Bearer top-secret".to_owned()),
    )
    .expect("runtime source");
    let registry = EvmSourceRegistry::single_source(
        source,
        EvmSourcePolicyId::new(policy_id).expect("policy"),
    )
    .expect("registry");
    let routes = EvmRouteRegistry::new([EvmRoute::new(
        EvmNetworkId::new(policy_id).expect("network"),
        EvmSourceRef::new(source_id).expect("source"),
        EvmSourcePolicyId::new(policy_id).expect("policy"),
    )])
    .expect("routes");
    EvmJsonRpcClient::new(registry, routes)
}

fn chain_request(network_id: &str, expected_chain_id: u64) -> EvmChainIdentityRequest {
    EvmChainIdentityRequest {
        guard: guard(network_id, expected_chain_id),
    }
}

fn guard(network_id: &str, expected_chain_id: u64) -> EvmChainGuard {
    EvmChainGuard::new(
        EvmNetworkId::new(network_id).expect("network"),
        expected_chain_id,
    )
    .expect("guard")
}

struct TestRpcServer {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl TestRpcServer {
    async fn spawn(chain_id: &'static str) -> Self {
        Self::spawn_with_mode(TestRpcMode::Ok { chain_id }).await
    }

    async fn spawn_legacy_fee(chain_id: &'static str) -> Self {
        Self::spawn_with_mode(TestRpcMode::LegacyFee { chain_id }).await
    }

    async fn spawn_pending_receipt(chain_id: &'static str) -> Self {
        Self::spawn_with_mode(TestRpcMode::PendingReceipt { chain_id }).await
    }

    async fn spawn_empty_code(chain_id: &'static str) -> Self {
        Self::spawn_with_mode(TestRpcMode::EmptyCode { chain_id }).await
    }

    async fn spawn_nonce_occupancy(chain_id: &'static str) -> Self {
        Self::spawn_with_mode(TestRpcMode::NonceOccupancy { chain_id }).await
    }

    async fn spawn_block_identity_mismatch(chain_id: &'static str) -> Self {
        Self::spawn_with_mode(TestRpcMode::BlockIdentityMismatch { chain_id }).await
    }

    async fn spawn_receipt_hash_mismatch(chain_id: &'static str) -> Self {
        Self::spawn_with_mode(TestRpcMode::ReceiptHashMismatch { chain_id }).await
    }

    async fn spawn_log_filter_mismatch(chain_id: &'static str) -> Self {
        Self::spawn_with_mode(TestRpcMode::LogFilterMismatch { chain_id }).await
    }

    async fn spawn_failure() -> Self {
        Self::spawn_with_mode(TestRpcMode::Failure).await
    }

    async fn spawn_json_rpc_failure() -> Self {
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
                        TestRpcMode::NonceOccupancy { chain_id } => {
                            let result = nonce_occupancy_rpc_result(chain_id, &method);
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
                        TestRpcMode::LogFilterMismatch { chain_id } => {
                            let result = log_filter_mismatch_rpc_result(chain_id, &method);
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

    fn methods(&self) -> Vec<String> {
        self.requests.lock().expect("requests").clone()
    }
}

#[derive(Clone, Copy)]
enum TestRpcMode {
    Ok { chain_id: &'static str },
    LegacyFee { chain_id: &'static str },
    PendingReceipt { chain_id: &'static str },
    EmptyCode { chain_id: &'static str },
    NonceOccupancy { chain_id: &'static str },
    BlockIdentityMismatch { chain_id: &'static str },
    ReceiptHashMismatch { chain_id: &'static str },
    LogFilterMismatch { chain_id: &'static str },
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
        "eth_getLogs" => json!([{
            "address": "0x1111111111111111111111111111111111111111",
            "topics": [HASH_HEX],
            "data": "0x1234",
            "blockNumber": "0x2a",
            "transactionHash": HASH_HEX,
            "logIndex": "0x0",
        }]),
        "eth_getTransactionCount" => json!("0x7"),
        "eth_gasPrice" => json!("0x10"),
        "eth_maxPriorityFeePerGas" => json!("0x2"),
        "eth_estimateGas" => json!("0x5208"),
        "eth_sendRawTransaction" => json!(HASH_HEX),
        "eth_getTransactionReceipt" => json!({
            "transactionHash": HASH_HEX,
            "blockNumber": "0x2a",
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

fn nonce_occupancy_rpc_result(chain_id: &str, method: &str) -> Value {
    match method {
        "eth_chainId" => json!(chain_id),
        "eth_getBlockByNumber" => json!({
            "number": "0x2a",
            "hash": HASH_HEX,
            "transactions": [{
                "from": "0x1111111111111111111111111111111111111111",
                "nonce": "0x7",
                "hash": OCCUPYING_HASH_HEX,
                "blockNumber": "0x2a",
            }],
        }),
        other => panic!("unexpected nonce occupancy method {other}"),
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
            "status": "0x1",
        }),
        other => rpc_result(chain_id, other),
    }
}

fn log_filter_mismatch_rpc_result(chain_id: &str, method: &str) -> Value {
    match method {
        "eth_chainId" => json!(chain_id),
        "eth_getLogs" => json!([{
            "address": "0x2222222222222222222222222222222222222222",
            "topics": [OCCUPYING_HASH_HEX],
            "data": "0x1234",
            "blockNumber": "0x2b",
            "transactionHash": HASH_HEX,
            "logIndex": "0x0",
        }]),
        other => rpc_result(chain_id, other),
    }
}
