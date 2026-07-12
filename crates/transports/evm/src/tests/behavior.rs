use super::*;

fn evm_response_invalid_error() -> EvmCapabilityError {
    EvmCapabilityError::provider_failure(evm_diagnostic(ProviderDiagnosticCode::ResponseInvalid))
}

#[tokio::test]
async fn selects_source_by_policy_and_records_redacted_evidence() {
    let server = TestRpcServer::spawn("0x1").await;
    let client = client_for(&server.url, "primary", "mainnet");

    let response = client
        .chain_identity(&chain_request())
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
        .chain_identity(&chain_request())
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
        .chain_identity(&chain_request())
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
        .chain_identity(&chain_request())
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
    let address = address!("0x1111111111111111111111111111111111111111");
    let hash = HASH_HEX.parse::<B256>().expect("hash");

    let block = client
        .read_block(&EvmBlockReadRequest::new(EvmBlockSelector::Latest))
        .await
        .expect("block");
    assert_eq!(block.block_number, 42);

    let balance = client
        .read_balance(&EvmBalanceReadRequest::new(
            address,
            EvmBlockSelector::Latest,
        ))
        .await
        .expect("balance");
    assert_eq!(
        balance.balance_wei,
        U256::from(1_000_000_000_000_000_000u128)
    );

    let call = client
        .read_call(&EvmCallReadRequest::new(
            address,
            vec![0xab, 0xcd],
            EvmBlockSelector::Latest,
        ))
        .await
        .expect("call");
    assert_eq!(call.return_data, vec![0x12, 0x34]);

    let code = client
        .read_code(&EvmCodeReadRequest::new(address, EvmBlockSelector::Latest))
        .await
        .expect("code");
    assert_eq!(code.code, vec![0xde, 0xad, 0xbe, 0xef]);
    assert_eq!(code.code_hash, alloy_primitives::keccak256(&code.code));

    let logs = client
        .read_logs(&EvmLogsReadRequest::new(
            EvmBlockSelector::Latest,
            EvmBlockSelector::Latest,
            Some(address),
            vec![hash],
        ))
        .await
        .expect("logs");
    assert_eq!(logs.logs.len(), 1);

    let nonce = client
        .read_nonce(&EvmNonceReadRequest::new(address, EvmBlockSelector::Latest))
        .await
        .expect("nonce");
    assert_eq!(nonce.nonce, 7);

    let fee = client
        .read_fee(&EvmFeeReadRequest::new())
        .await
        .expect("fee");
    assert_eq!(fee.legacy_gas_price, Some(16));
    assert_eq!(fee.priority_fee_per_gas, Some(2));
    assert_eq!(fee.base_fee_per_gas, Some(32));
    assert_eq!(fee.max_fee_per_gas, Some(66));

    let gas = client
        .estimate_gas(&EvmGasEstimateRequest::new(
            Some(address),
            Some(address),
            0,
            vec![0xab],
        ))
        .await
        .expect("gas");
    assert_eq!(gas.gas_limit, 21_000);

    let submit = client
        .submit_transaction(&EvmTransactionSubmitRequest::new(
            SignedEvmPayload::from_verified_bytes(vec![0x01], hash).expect("payload"),
        ))
        .await
        .expect("submit");
    assert_eq!(submit.transaction_hash, hash);

    let receipt = client
        .read_receipt(&EvmReceiptReadRequest::new(hash))
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
    assert_eq!(
        block_selector_tag(&EvmBlockSelector::Pending).expect("pending tag"),
        "pending"
    );
}

#[test]
fn hash_block_selector_params_require_canonical() {
    let hash = HASH_HEX.parse::<B256>().expect("hash");

    assert_eq!(
        block_selector_param(&EvmBlockSelector::Hash(hash)),
        json!({
            "blockHash": HASH_HEX,
            "requireCanonical": true,
        })
    );
}

#[tokio::test]
async fn pending_receipt_is_typed_capability_error() {
    let server = TestRpcServer::spawn_pending_receipt("0x1").await;
    let client = client_for(&server.url, "primary", "mainnet");
    let error = client
        .read_receipt(&EvmReceiptReadRequest::new(
            HASH_HEX.parse::<B256>().expect("hash"),
        ))
        .await
        .expect_err("pending receipt");

    assert_eq!(error, EvmCapabilityError::ReceiptPending);
}

#[tokio::test]
async fn code_read_preserves_empty_code_observation() {
    let server = TestRpcServer::spawn_empty_code("0x1").await;
    let client = client_for(&server.url, "primary", "mainnet");
    let response = client
        .read_code(&EvmCodeReadRequest::new(
            address!("0x1111111111111111111111111111111111111111"),
            EvmBlockSelector::Latest,
        ))
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
        .read_code(&EvmCodeReadRequest::new(
            address!("0x1111111111111111111111111111111111111111"),
            EvmBlockSelector::Latest,
        ))
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
            .read_nonce_occupancy(&EvmNonceOccupancyReadRequest::new(
                address!("0x1111111111111111111111111111111111111111"),
                7,
                excluded_transaction_hash
                    .parse::<B256>()
                    .expect("excluded hash"),
            ))
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
        .read_fee(&EvmFeeReadRequest::new())
        .await
        .expect("legacy fee response");

    assert_eq!(fee.legacy_gas_price, Some(16));
    assert_eq!(fee.priority_fee_per_gas, None);
    assert_eq!(fee.base_fee_per_gas, None);
    assert_eq!(fee.max_fee_per_gas, None);
}

#[test]
fn network_binding_validation_does_not_require_guard_or_live_io() {
    let client = raw_client_for("http://127.0.0.1:1", "primary", "mainnet");

    client
        .validate_network_binding(&network_binding("mainnet", 1))
        .expect("route binding");

    let missing = client
        .validate_network_binding(&network_binding("sepolia", 1))
        .expect_err("missing route");
    assert_eq!(missing, EvmTransportError::RouteUnavailable);
}

#[tokio::test]
async fn rejects_explicit_block_identity_mismatch() {
    let server = TestRpcServer::spawn_block_identity_mismatch("0x1").await;
    let client = client_for(&server.url, "primary", "mainnet");

    let number_error = client
        .read_block(&EvmBlockReadRequest::new(EvmBlockSelector::Number(42)))
        .await
        .expect_err("number mismatch");
    assert_eq!(number_error, evm_response_invalid_error());

    let hash_error = client
        .read_block(&EvmBlockReadRequest::new(EvmBlockSelector::Hash(
            HASH_HEX.parse::<B256>().expect("hash"),
        )))
        .await
        .expect_err("hash mismatch");
    assert_eq!(hash_error, evm_response_invalid_error());
}

#[tokio::test]
async fn rejects_receipt_transaction_hash_mismatch() {
    let server = TestRpcServer::spawn_receipt_hash_mismatch("0x1").await;
    let client = client_for(&server.url, "primary", "mainnet");

    let error = client
        .read_receipt(&EvmReceiptReadRequest::new(
            HASH_HEX.parse::<B256>().expect("hash"),
        ))
        .await
        .expect_err("receipt hash mismatch");

    assert_eq!(error, evm_response_invalid_error());
}

#[tokio::test]
async fn rejects_log_entries_that_contradict_filter() {
    let server = TestRpcServer::spawn_log_filter_mismatch("0x1").await;
    let client = client_for(&server.url, "primary", "mainnet");

    let error = client
        .read_logs(&EvmLogsReadRequest::new(
            EvmBlockSelector::Number(42),
            EvmBlockSelector::Number(42),
            Some(address!("0x1111111111111111111111111111111111111111")),
            vec![HASH_HEX.parse::<B256>().expect("hash")],
        ))
        .await
        .expect_err("log filter mismatch");

    assert_eq!(error, evm_response_invalid_error());
}

#[tokio::test]
async fn rejects_numeric_log_range_when_log_omits_block_number() {
    let server = TestRpcServer::spawn_log_missing_block_number("0x1").await;
    let client = client_for(&server.url, "primary", "mainnet");

    let error = client
        .read_logs(&EvmLogsReadRequest::new(
            EvmBlockSelector::Number(42),
            EvmBlockSelector::Number(42),
            Some(address!("0x1111111111111111111111111111111111111111")),
            vec![HASH_HEX.parse::<B256>().expect("hash")],
        ))
        .await
        .expect_err("missing block number cannot prove numeric range");

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
    )
    .bind_network(network_binding("mainnet", 1))
    .expect("network binding");

    let response = client
        .chain_identity(&chain_request())
        .await
        .expect("fallback response");

    assert_eq!(response.evidence.source_ref.as_str(), "secondary");
    assert_eq!(response.chain_id, 1);
}

fn client_for(url: &str, source_id: &str, policy_id: &str) -> EvmJsonRpcNetworkProvider {
    raw_client_for(url, source_id, policy_id)
        .bind_network(network_binding(policy_id, 1))
        .expect("network binding")
}

fn raw_client_for(url: &str, source_id: &str, policy_id: &str) -> EvmJsonRpcClient {
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

fn network_binding(network_id: &str, expected_chain_id: u64) -> EvmNetworkBinding {
    EvmNetworkBinding::new(
        EvmNetworkId::new(network_id).expect("network"),
        expected_chain_id,
    )
    .expect("network binding")
}

fn chain_request() -> EvmChainIdentityRequest {
    EvmChainIdentityRequest::new()
}
