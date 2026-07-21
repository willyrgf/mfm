use super::*;
use std::sync::atomic::AtomicU64;

use mfm_btc_capabilities::{
    BitcoinAddress, BitcoinNetworkId, BitcoinNetworkTag, BtcHeadSelection, BtcInvalidRequest,
};

const BEST_BLOCK_HASH: &str = "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5";
const NO_REQUESTED_BLOCK_HASH_HEIGHT: u64 = u64::MAX;

struct MockBtcTransport {
    chain: &'static str,
    blocks: u64,
    fail_blockchain_info: bool,
    scan_success: bool,
    scan_height: u64,
    scan_bestblock: String,
    scan_total_sats: u64,
    blockchain_info_calls: AtomicU64,
    block_hash_calls: AtomicU64,
    requested_block_hash_height: AtomicU64,
    block_header_calls: AtomicU64,
    scan_calls: AtomicU64,
}

impl MockBtcTransport {
    fn new(chain: &'static str) -> Self {
        Self {
            chain,
            blocks: 840_000,
            fail_blockchain_info: false,
            scan_success: true,
            scan_height: 840_000,
            scan_bestblock: BEST_BLOCK_HASH.to_string(),
            scan_total_sats: 123_456_789,
            blockchain_info_calls: AtomicU64::new(0),
            block_hash_calls: AtomicU64::new(0),
            requested_block_hash_height: AtomicU64::new(NO_REQUESTED_BLOCK_HASH_HEIGHT),
            block_header_calls: AtomicU64::new(0),
            scan_calls: AtomicU64::new(0),
        }
    }

    fn failing_blockchain_info() -> Self {
        Self {
            fail_blockchain_info: true,
            ..Self::new("main")
        }
    }

    fn with_tip(mut self, height: u64) -> Self {
        self.blocks = height;
        self.scan_height = height;
        self
    }

    fn with_scan_tip(mut self, height: u64, bestblock: &str) -> Self {
        self.scan_height = height;
        self.scan_bestblock = bestblock.to_owned();
        self
    }

    fn with_aborted_scan(mut self) -> Self {
        self.scan_success = false;
        self
    }
}

impl BtcJsonRpcChainHeadTransport for MockBtcTransport {
    fn get_blockchain_info<'a>(&'a self) -> BtcTransportFuture<'a, BlockchainInfo> {
        Box::pin(async move {
            self.blockchain_info_calls.fetch_add(1, Ordering::Relaxed);
            if self.fail_blockchain_info {
                return Err(BtcRpcError::JsonRpcError {
                    code: -32603,
                    message: "Authorization: Bearer secret at http://user:pass@node.invalid"
                        .to_string(),
                });
            }
            Ok(BlockchainInfo {
                blocks: self.blocks,
                bestblockhash: BEST_BLOCK_HASH.to_string(),
                chain: self.chain.to_string(),
                initialblockdownload: Some(false),
            })
        })
    }

    fn get_block_hash<'a>(
        &'a self,
        _verified: &'a VerifiedBtcCall,
        height: u64,
    ) -> BtcTransportFuture<'a, String> {
        Box::pin(async move {
            self.block_hash_calls.fetch_add(1, Ordering::Relaxed);
            self.requested_block_hash_height
                .store(height, Ordering::Relaxed);
            Ok(BEST_BLOCK_HASH.to_string())
        })
    }

    fn get_block_header<'a>(
        &'a self,
        _verified: &'a VerifiedBtcCall,
        block_hash: &'a str,
    ) -> BtcTransportFuture<'a, BlockHeaderInfo> {
        Box::pin(async move {
            self.block_header_calls.fetch_add(1, Ordering::Relaxed);
            let requested_height = self.requested_block_hash_height.load(Ordering::Relaxed);
            let height = if requested_height == NO_REQUESTED_BLOCK_HASH_HEIGHT {
                self.blocks
            } else {
                requested_height
            };
            Ok(BlockHeaderInfo {
                hash: block_hash.to_string(),
                height,
                time: 1_713_571_767,
            })
        })
    }

    fn scan_tx_out_set<'a>(
        &'a self,
        _verified: &'a VerifiedBtcCall,
        _address: &'a str,
    ) -> BtcTransportFuture<'a, ScanTxOutSetResult> {
        Box::pin(async move {
            self.scan_calls.fetch_add(1, Ordering::Relaxed);
            Ok(ScanTxOutSetResult {
                success: self.scan_success,
                height: self.scan_height,
                bestblock: self.scan_bestblock.clone(),
                total_amount: Amount::from_sat(self.scan_total_sats),
            })
        })
    }
}

fn source_identity() -> BitcoinSourceIdentity {
    BitcoinSourceIdentity::new("public-bitcoin-core").expect("source")
}

fn source_binding() -> BitcoinSourceBinding {
    BitcoinSourceBinding::new(
        BitcoinNetworkId::new("bitcoin-mainnet").expect("network"),
        source_identity(),
        BitcoinNetworkTag::Main,
    )
}

fn router_with(
    source_identity: BitcoinSourceIdentity,
    transport: Arc<MockBtcTransport>,
) -> BtcJsonRpcRouter {
    let mut routes = BTreeMap::new();
    routes.insert(
        source_identity,
        transport as Arc<dyn BtcJsonRpcChainHeadTransport>,
    );
    BtcJsonRpcRouter::new_for_transport(routes)
}

fn provider_with(
    source_identity: BitcoinSourceIdentity,
    transport: Arc<MockBtcTransport>,
) -> BtcJsonRpcSourceProvider {
    router_with(source_identity, transport)
        .bind_source(source_binding())
        .expect("bind source")
}

fn chain_head_request(selection: BtcHeadSelection) -> BtcChainHeadRequest {
    BtcChainHeadRequest::new(selection)
}

fn balance_request() -> BtcBalanceReadRequest {
    balance_request_at(840_000, BEST_BLOCK_HASH)
}

fn balance_request_at(height: u64, block_hash: &str) -> BtcBalanceReadRequest {
    balance_request_for_address(
        "bc1qns9f7yfx3ry9lj6yz7c9er0vwa0ye2eklpzqfw",
        height,
        block_hash,
    )
}

fn balance_request_for_address(
    address: &str,
    height: u64,
    block_hash: &str,
) -> BtcBalanceReadRequest {
    BtcBalanceReadRequest::new(
        BitcoinAddress::new(address).expect("address"),
        height,
        BitcoinBlockHash::new(block_hash).expect("block hash"),
    )
}

#[test]
fn config_creates_without_auth() {
    let config = BtcJsonRpcConfig {
        rpc_url: "http://127.0.0.1:8332".to_string(),
        rpc_user: None,
        rpc_password: None,
    };
    let _ = BtcJsonRpcClient::new(config).expect("constructor should not fail");
}

#[test]
fn config_debug_redacts_secret_bearing_fields() {
    let config = BtcJsonRpcConfig {
        rpc_url: "http://url_user:url_password@example.com:8332/rpc?api_key=query_secret&token=query_token#frag".to_string(),
        rpc_user: Some("rpc_user_secret".to_string()),
        rpc_password: Some("rpc_password_secret".to_string()),
    };

    let rendered = format!("{config:?}");

    assert!(rendered.contains("BtcJsonRpcConfig"));
    assert!(rendered.contains("http://example.com:8332"));
    assert!(!rendered.contains("url_user"));
    assert!(!rendered.contains("url_password"));
    assert!(!rendered.contains("api_key"));
    assert!(!rendered.contains("query_secret"));
    assert!(!rendered.contains("query_token"));
    assert!(!rendered.contains("rpc_user_secret"));
    assert!(!rendered.contains("rpc_password_secret"));
}

#[test]
fn http_status_error_omits_raw_response_body() {
    let err = BtcRpcError::HttpStatus {
        status: 500,
        body_len: Some(
            "Authorization: Bearer body_token password=body_password token=body_secret".len(),
        ),
        content_type: Some("text/plain".to_string()),
    };

    let rendered = format!("{err}");
    let debug = format!("{err:?}");

    assert!(rendered.contains("btc rpc http status 500"));
    assert!(rendered.contains("body_len="));
    assert!(!rendered.contains("body_token"));
    assert!(!rendered.contains("body_password"));
    assert!(!rendered.contains("body_secret"));
    assert!(!debug.contains("body_token"));
    assert!(!debug.contains("body_password"));
    assert!(!debug.contains("body_secret"));
}

#[test]
fn diagnostic_message_redacts_secret_patterns() {
    let rendered = BtcRpcError::JsonRpcError {
        code: -32603,
        message: diagnostic_message(
            "Authorization: Bearer auth_token api_key=query_secret password=rpc_password",
        ),
    }
    .to_string();

    assert!(rendered.contains(REDACTED_DIAGNOSTIC));
    assert!(!rendered.contains("auth_token"));
    assert!(!rendered.contains("query_secret"));
    assert!(!rendered.contains("rpc_password"));
}

#[test]
fn content_type_metadata_redacts_secret_markers() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("text/plain; token=header_secret"),
    );

    let rendered = sanitized_content_type(&headers).expect("content type should parse");

    assert_eq!(rendered, REDACTED_SECRET);
    assert!(!rendered.contains("header_secret"));
}

#[test]
fn missing_source_binding_is_redacted() {
    let router = BtcJsonRpcRouter::new(BTreeMap::new());
    let error = router
        .validate_source_binding(&source_binding())
        .expect_err("route should be missing");
    let rendered = format!("{error:?} {error}");

    let BtcCapabilityError::Provider { diagnostic } = error else {
        panic!("expected provider diagnostic");
    };
    assert_eq!(diagnostic.stable_error_code(), "bitcoin_route_unavailable");
    assert!(rendered.contains("public-bitcoin-core"));
    assert!(!rendered.contains("http://"));
    assert!(!rendered.contains(concat!("Author", "ization")));
    assert!(!rendered.contains("secret"));
}

#[tokio::test]
async fn observed_network_mismatch_fails_before_operation_rpc() {
    let transport = Arc::new(MockBtcTransport::new("test"));
    let provider = provider_with(source_identity(), transport.clone());
    let request = chain_head_request(BtcHeadSelection::confirmed(6).expect("selection"));

    let error = provider
        .read_chain_head(&request)
        .await
        .expect_err("source mismatch");
    let rendered = format!("{error:?} {error}");

    assert!(matches!(error, BtcCapabilityError::SourceMismatch { .. }));
    assert_eq!(transport.blockchain_info_calls.load(Ordering::Relaxed), 1);
    assert_eq!(transport.block_hash_calls.load(Ordering::Relaxed), 0);
    assert_eq!(transport.block_header_calls.load(Ordering::Relaxed), 0);
    assert!(!rendered.contains("http://"));
    assert!(!rendered.contains(concat!("Author", "ization")));
    assert!(!rendered.contains("secret"));
}

#[tokio::test]
async fn successful_response_evidence_matches_binding_by_construction() {
    let transport = Arc::new(MockBtcTransport::new("main"));
    let provider = provider_with(source_identity(), transport.clone());
    let binding = source_binding();
    let request = chain_head_request(BtcHeadSelection::best());

    let response = provider
        .read_chain_head(&request)
        .await
        .expect("successful response");

    assert_eq!(response.evidence.network_id, binding.network_id().clone());
    assert_eq!(
        response.evidence.source_identity,
        binding.source_identity().clone()
    );
    assert_eq!(
        response.evidence.bitcoin_network,
        binding.bitcoin_network().as_str()
    );
    assert_eq!(
        response.evidence.observed_bitcoin_network,
        binding.bitcoin_network().as_str()
    );
    assert_eq!(response.head_kind, request.selection().head_kind());
    assert_eq!(response.finality, request.selection().finality());
    assert_eq!(response.block_height, 840_000);
    assert_eq!(response.block_hash.to_string(), BEST_BLOCK_HASH);
    assert_eq!(response.provider_time_unix_ms, Some(1_713_571_767_000));
    assert_eq!(transport.blockchain_info_calls.load(Ordering::Relaxed), 1);
    assert_eq!(transport.block_header_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn confirmed_depth_one_selects_current_tip() {
    let transport = Arc::new(MockBtcTransport::new("main").with_tip(840_000));
    let provider = provider_with(source_identity(), transport.clone());
    let request = chain_head_request(BtcHeadSelection::confirmed(1).expect("selection"));

    let response = provider
        .read_chain_head(&request)
        .await
        .expect("confirmed head");

    assert_eq!(response.block_height, 840_000);
    assert_eq!(
        transport
            .requested_block_hash_height
            .load(Ordering::Relaxed),
        840_000
    );
    assert_eq!(transport.block_hash_calls.load(Ordering::Relaxed), 1);
    assert_eq!(transport.block_header_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn confirmed_depth_selects_highest_satisfying_height() {
    let transport = Arc::new(MockBtcTransport::new("main").with_tip(840_000));
    let provider = provider_with(source_identity(), transport.clone());
    let request = chain_head_request(BtcHeadSelection::confirmed(6).expect("selection"));

    let response = provider
        .read_chain_head(&request)
        .await
        .expect("confirmed head");

    assert_eq!(response.block_height, 839_995);
    assert_eq!(
        transport
            .requested_block_hash_height
            .load(Ordering::Relaxed),
        839_995
    );
    assert_eq!(transport.block_hash_calls.load(Ordering::Relaxed), 1);
    assert_eq!(transport.block_header_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn confirmed_depth_rejects_insufficient_available_height() {
    let transport = Arc::new(MockBtcTransport::new("main").with_tip(4));
    let provider = provider_with(source_identity(), transport.clone());
    let request = chain_head_request(BtcHeadSelection::confirmed(6).expect("selection"));

    let error = provider
        .read_chain_head(&request)
        .await
        .expect_err("insufficient chain height");

    let BtcCapabilityError::Provider { diagnostic } = error else {
        panic!("expected provider diagnostic");
    };
    assert_eq!(
        diagnostic.stable_error_code(),
        "bitcoin_operation_incomplete"
    );
    assert_eq!(transport.block_hash_calls.load(Ordering::Relaxed), 0);
    assert_eq!(transport.block_header_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn bound_provider_probes_source_for_each_operation_call() {
    let transport = Arc::new(MockBtcTransport::new("main"));
    let provider = provider_with(source_identity(), transport.clone());

    provider
        .read_chain_head(&chain_head_request(BtcHeadSelection::best()))
        .await
        .expect("chain head");
    provider
        .read_balance(&balance_request())
        .await
        .expect("balance");

    assert_eq!(transport.blockchain_info_calls.load(Ordering::Relaxed), 2);
    assert_eq!(transport.block_header_calls.load(Ordering::Relaxed), 1);
    assert_eq!(transport.scan_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn balance_source_mismatch_fails_before_scan() {
    let transport = Arc::new(MockBtcTransport::new("test"));
    let provider = provider_with(source_identity(), transport.clone());
    let request = balance_request();

    let error = provider
        .read_balance(&request)
        .await
        .expect_err("source mismatch");

    assert!(matches!(error, BtcCapabilityError::SourceMismatch { .. }));
    assert_eq!(transport.blockchain_info_calls.load(Ordering::Relaxed), 1);
    assert_eq!(transport.scan_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn balance_reads_exact_tip_with_scan_tx_out_set() {
    let transport = Arc::new(MockBtcTransport::new("main"));
    let provider = provider_with(source_identity(), transport.clone());
    let request = balance_request();

    let response = provider.read_balance(&request).await.expect("balance read");

    assert_eq!(response.address, request.address().clone());
    assert_eq!(response.balance_sats, 123_456_789);
    assert_eq!(response.block_height, request.block_height());
    assert_eq!(response.block_hash, request.block_hash().clone());
    assert_eq!(response.evidence.source_identity, source_identity());
    assert_eq!(transport.blockchain_info_calls.load(Ordering::Relaxed), 1);
    assert_eq!(transport.block_hash_calls.load(Ordering::Relaxed), 0);
    assert_eq!(transport.block_header_calls.load(Ordering::Relaxed), 0);
    assert_eq!(transport.scan_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn balance_reads_accept_every_checked_chain_family() {
    for (chain, network, address) in [
        (
            "main",
            BitcoinNetworkTag::Main,
            "bc1qns9f7yfx3ry9lj6yz7c9er0vwa0ye2eklpzqfw",
        ),
        (
            "test",
            BitcoinNetworkTag::Test,
            "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7",
        ),
        (
            "testnet4",
            BitcoinNetworkTag::Testnet4,
            "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7",
        ),
        (
            "signet",
            BitcoinNetworkTag::Signet,
            "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7",
        ),
        (
            "regtest",
            BitcoinNetworkTag::Regtest,
            "bcrt1q2nfxmhd4n3c8834pj72xagvyr9gl57n5r94fsl",
        ),
    ] {
        let transport = Arc::new(MockBtcTransport::new(chain));
        let binding = BitcoinSourceBinding::new(
            BitcoinNetworkId::new(format!("bitcoin-{chain}")).expect("semantic network id"),
            source_identity(),
            network,
        );
        let provider = router_with(source_identity(), transport.clone())
            .bind_source(binding)
            .expect("bind source");
        let response = provider
            .read_balance(&balance_request_for_address(
                address,
                840_000,
                BEST_BLOCK_HASH,
            ))
            .await
            .unwrap_or_else(|error| panic!("{chain} balance should succeed: {error}"));

        assert_eq!(response.address.as_str(), address);
        assert_eq!(transport.scan_calls.load(Ordering::Relaxed), 1);
    }
}

#[tokio::test]
async fn balance_read_rejects_wrong_address_family_before_scan() {
    let transport = Arc::new(MockBtcTransport::new("main"));
    let provider = provider_with(source_identity(), transport.clone());
    let request = balance_request_for_address(
        "tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7",
        840_000,
        BEST_BLOCK_HASH,
    );

    assert_eq!(
        provider.read_balance(&request).await.unwrap_err(),
        BtcCapabilityError::InvalidRequest {
            reason: BtcInvalidRequest::AddressNetworkMismatch,
        }
    );
    assert_eq!(transport.scan_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn balance_rejects_stale_requested_tip_before_scan() {
    let transport = Arc::new(MockBtcTransport::new("main"));
    let provider = provider_with(source_identity(), transport.clone());
    let request = balance_request_at(839_999, BEST_BLOCK_HASH);

    let error = provider
        .read_balance(&request)
        .await
        .expect_err("stale requested anchor cannot be scanned exactly");

    let BtcCapabilityError::Provider { diagnostic } = error else {
        panic!("expected provider diagnostic");
    };
    assert_eq!(
        diagnostic.stable_error_code(),
        "bitcoin_operation_incomplete"
    );
    assert_eq!(transport.blockchain_info_calls.load(Ordering::Relaxed), 1);
    assert_eq!(transport.scan_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn balance_rejects_scan_tip_drift() {
    let drift_hash = "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a6";
    let transport = Arc::new(MockBtcTransport::new("main").with_scan_tip(840_001, drift_hash));
    let provider = provider_with(source_identity(), transport.clone());
    let request = balance_request();

    let error = provider
        .read_balance(&request)
        .await
        .expect_err("scan tip drift must fail");

    let BtcCapabilityError::Provider { diagnostic } = error else {
        panic!("expected provider diagnostic");
    };
    assert_eq!(
        diagnostic.stable_error_code(),
        "bitcoin_operation_incomplete"
    );
    assert_eq!(transport.blockchain_info_calls.load(Ordering::Relaxed), 1);
    assert_eq!(transport.scan_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn balance_rejects_aborted_scan() {
    let transport = Arc::new(MockBtcTransport::new("main").with_aborted_scan());
    let provider = provider_with(source_identity(), transport.clone());
    let request = balance_request();

    let error = provider
        .read_balance(&request)
        .await
        .expect_err("aborted scan must fail");

    let BtcCapabilityError::Provider { diagnostic } = error else {
        panic!("expected provider diagnostic");
    };
    assert_eq!(
        diagnostic.stable_error_code(),
        "bitcoin_operation_incomplete"
    );
    assert_eq!(transport.blockchain_info_calls.load(Ordering::Relaxed), 1);
    assert_eq!(transport.scan_calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn provider_diagnostic_discards_provider_messages() {
    let transport = Arc::new(MockBtcTransport::failing_blockchain_info());
    let provider = provider_with(source_identity(), transport);
    let request = chain_head_request(BtcHeadSelection::best());

    let error = provider
        .read_chain_head(&request)
        .await
        .expect_err("provider failure");
    let rendered = format!("{error:?} {error}");

    assert!(matches!(error, BtcCapabilityError::Provider { .. }));
    assert!(!rendered.contains("http://"));
    assert!(!rendered.contains("user:pass"));
    assert!(!rendered.contains(concat!("Author", "ization")));
    assert!(!rendered.contains("secret"));
}

#[test]
fn blockchain_info_deserializes() {
    let json = serde_json::json!({
        "chain": "main",
        "blocks": 840000,
        "headers": 840000,
        "bestblockhash": "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5",
        "difficulty": 83148355189239.77,
        "time": 1713571767,
        "mediantime": 1713569060,
        "verificationprogress": 0.9999,
        "initialblockdownload": false,
        "chainwork": "00000000000000000000000000000000000000007b48a3b73a8f3af5bf6d5e5e",
        "size_on_disk": 650000000000_u64,
        "pruned": false,
        "warnings": ""
    });
    let info: BlockchainInfo = serde_json::from_value(json).expect("deserialize");
    assert_eq!(info.blocks, 840000);
    assert_eq!(
        info.bestblockhash,
        "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5"
    );
    assert_eq!(info.chain, "main");
    assert_eq!(info.initialblockdownload, Some(false));
}

#[test]
fn block_header_info_deserializes() {
    let json = serde_json::json!({
        "hash": "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5",
        "confirmations": 12,
        "height": 840000,
        "version": 536870912,
        "versionHex": "20000000",
        "merkleroot": "4d5e...",
        "time": 1713571767,
        "mediantime": 1713569060,
        "nonce": 0,
        "bits": "17034219",
        "difficulty": 83148355189239.77,
        "chainwork": "00000000000000000000000000000000000000007b48a3b73a8f3af5bf6d5e5e",
        "nTx": 3200,
        "previousblockhash": "0000000000000000000011111111111111111111111111111111111111111111",
    });
    let header: BlockHeaderInfo = serde_json::from_value(json).expect("deserialize");

    assert_eq!(header.height, 840000);
    assert_eq!(
        header.hash,
        "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5"
    );
    assert_eq!(header.time, 1713571767);
}

#[test]
fn scan_tx_out_set_results_deserialize_populated_and_empty_responses() {
    for (case, json, expected_total_sats) in [
        (
            "populated",
            r#"{
                "success": true,
                "txouts": 120000000,
                "height": 840000,
                "bestblock": "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5",
                "unspents": [
                    {
                        "txid": "abc123",
                        "vout": 0,
                        "scriptPubKey": "76a914...",
                        "desc": "addr(1BoatSLRHtKNngkdXEeobR76b53LETtpyT)#...",
                        "amount": 0.00000001,
                        "height": 839999
                    }
                ],
                "total_amount": 0.05000000
            }"#,
            5_000_000,
        ),
        (
            "empty",
            r#"{
                "success": true,
                "txouts": 120000000,
                "height": 840000,
                "bestblock": "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5",
                "unspents": [],
                "total_amount": 0
            }"#,
            0,
        ),
    ] {
        let result: ScanTxOutSetResult = serde_json::from_str(json).expect(case);

        assert!(result.success, "{case}");
        assert_eq!(result.height, 840000, "{case}");
        assert_eq!(
            result.bestblock, "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5",
            "{case}"
        );
        assert_eq!(result.total_amount.to_sat(), expected_total_sats, "{case}");
    }
}

#[test]
fn btc_amount_json_to_amount_parses_exact_satoshis() {
    for (amount, expected_sats) in [
        ("0.00000001", 1),
        ("0.05000000", 5_000_000),
        ("\"1.23000000\"", 123_000_000),
        ("21000000.00000000", Amount::MAX_MONEY.to_sat()),
    ] {
        assert_eq!(
            btc_amount_json_to_amount(amount).unwrap().to_sat(),
            expected_sats,
            "{amount}"
        );
    }
}

#[test]
fn btc_amount_json_to_amount_rejects_invalid_amounts() {
    for (amount, expected_error) in [
        ("0.000000001", BtcAmountParseError::TooPrecise),
        ("-0.00000001", BtcAmountParseError::Negative),
        ("2.1e7", BtcAmountParseError::Invalid),
        ("18446744073709551615", BtcAmountParseError::Overflow),
        ("21000000.00000001", BtcAmountParseError::ExceedsMaxMoney),
    ] {
        assert_eq!(
            btc_amount_json_to_amount(amount).unwrap_err(),
            expected_error
        );
    }
}
