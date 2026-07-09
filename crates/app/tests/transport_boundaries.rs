#![allow(clippy::disallowed_methods)]

#[test]
fn production_app_links_only_supported_transport_crates() {
    let manifest = include_str!("../Cargo.toml");
    for required in [
        "mfm-adapters-portfolio",
        "mfm-transports-evm",
        "mfm-transports-proof",
    ] {
        assert!(
            manifest.contains(required),
            "production app manifest must link {required}"
        );
    }
    for forbidden in [
        "mfm-transports-exec",
        "mfm-transports-local-evm",
        concat!("mfm-transports-evm-", "d", "cv"),
        concat!("mfm-transports-", "portfolio"),
        "mfm-transports-local-fs",
        "mfm-transports-local-keystore",
        "mfm-transports-rpc-control",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "production app manifest must not link unsupported transport dependency {forbidden}"
        );
    }
}

#[test]
fn transport_sources_exclude_artifact_write_surfaces() {
    for (name, source) in [
        ("proof", include_str!("../../transports/proof/src/lib.rs")),
        (
            "portfolio",
            include_str!("../../adapters/portfolio/src/lib.rs"),
        ),
    ] {
        for banned in [
            concat!("request: serde_json::", "Value"),
            "ProofArtifactSink",
            "ProofArtifactSinkFuture",
            "PortfolioArtifactStore",
            "PortfolioArtifactStoreFuture",
            "put_artifact(",
            "put_verified_artifact(",
            "persist_artifact(",
        ] {
            assert!(
                !source.contains(banned),
                "transport {name} must not expose artifact write surface {banned}"
            );
        }
    }
}

#[test]
fn bound_provider_boundary_forbids_request_owned_source_binding() {
    let surfaces = [
        (
            "btc-capabilities",
            include_str!("../../btc-capabilities/src/lib.rs"),
        ),
        (
            "evm-capabilities",
            include_str!("../../evm-capabilities/src/lib.rs"),
        ),
        (
            "btc-jsonrpc-http",
            include_str!("../../transports/btc-jsonrpc-http/src/lib.rs"),
        ),
        (
            "evm-transport",
            include_str!("../../transports/evm/src/lib.rs"),
        ),
        (
            "btc-jsonrpc-adapter",
            include_str!("../../adapters/btc-jsonrpc/src/lib.rs"),
        ),
        (
            "evm-contracts-adapter",
            include_str!("../../adapters/evm-contracts/src/lib.rs"),
        ),
        (
            "portfolio-adapter",
            include_str!("../../adapters/portfolio/src/lib.rs"),
        ),
    ];
    for (name, source) in surfaces {
        for banned in [
            "EvmRequestAuthority",
            "EvmRequestSource",
            "BtcRequestSource",
            "BtcJsonRpcChainHeadProvider",
            "with_middleware",
            "skip_validation",
            "fn unchecked",
            "fn skip_validation",
        ] {
            assert!(
                !source.contains(banned),
                "{name} must not retain forbidden bound-provider escape hatch {banned}"
            );
        }
    }

    let evm_transport = include_str!("../../transports/evm/src/lib.rs");
    let btc_transport = include_str!("../../transports/btc-jsonrpc-http/src/lib.rs");
    let evm_adapter = include_str!("../../adapters/evm-contracts/src/lib.rs");
    let recorded_btc = include_str!("../../adapters/btc-jsonrpc/src/lib.rs");

    for trait_name in [
        "EvmBlockReadProvider",
        "EvmBalanceReadProvider",
        "EvmChainIdentityProvider",
        "EvmCallReadProvider",
        "EvmCodeReadProvider",
        "EvmLogsReadProvider",
        "EvmNonceReadProvider",
        "EvmFeeReadProvider",
        "EvmGasEstimateProvider",
        "EvmTransactionSubmitProvider",
        "EvmReceiptReadProvider",
        "EvmNonceOccupancyReadProvider",
    ] {
        assert!(
            !evm_transport.contains(&format!("impl {trait_name} for EvmJsonRpcClient")),
            "raw EvmJsonRpcClient must not implement {trait_name}"
        );
    }
    for trait_name in ["BtcChainHeadReadProvider", "BtcBalanceReadProvider"] {
        assert!(
            !btc_transport.contains(&format!("impl {trait_name} for BtcJsonRpcRouter")),
            "raw BtcJsonRpcRouter must not implement {trait_name}"
        );
        assert!(
            !btc_transport.contains(&format!("impl {trait_name} for BtcJsonRpcClient")),
            "raw BtcJsonRpcClient must not implement {trait_name}"
        );
    }

    assert!(
        evm_transport.contains("struct EvmJsonRpcNetworkProvider"),
        "EVM transport must expose a bound network provider"
    );
    assert!(
        evm_transport.contains("struct VerifiedEvmCall"),
        "EVM transport must mint private VerifiedEvmCall tokens"
    );
    assert!(
        evm_transport.contains("async fn prepare_call"),
        "EVM bound provider must own prepare_call"
    );
    assert!(
        evm_transport.contains("verified: &VerifiedEvmCall"),
        "EVM operation rpc_call must require VerifiedEvmCall"
    );
    assert!(
        !evm_transport
            .contains("async fn rpc_call(\n        &self,\n        source: &EvmRuntimeSource"),
        "EVM operation rpc_call must not accept bare EvmRuntimeSource"
    );
    assert!(
        btc_transport.contains("struct BtcJsonRpcSourceProvider"),
        "BTC transport must expose a bound source provider"
    );
    assert!(
        btc_transport.contains("struct VerifiedBtcCall"),
        "BTC transport must mint private VerifiedBtcCall tokens"
    );
    assert!(
        btc_transport.contains("async fn prepare_call"),
        "BTC bound provider must own prepare_call"
    );
    assert!(
        btc_transport.contains("verified: &VerifiedBtcCall")
            || btc_transport.contains("fn get_block_hash(&self, height: u64)")
                && btc_transport.contains("impl VerifiedBtcCall"),
        "BTC operation helpers must require VerifiedBtcCall"
    );
    assert!(
        btc_transport.contains("impl VerifiedBtcCall"),
        "BTC operation helpers must be token-gated methods on VerifiedBtcCall"
    );
    assert!(
        !btc_transport.contains("pub async fn get_blockchain_info"),
        "BTC client must not expose public get_blockchain_info probe surface"
    );
    assert!(
        !btc_transport.contains("pub async fn get_block_hash"),
        "BTC client must not expose public get_block_hash operation surface"
    );
    assert!(
        !btc_transport.contains("pub async fn get_block_header"),
        "BTC client must not expose public get_block_header operation surface"
    );
    assert!(
        !btc_transport.contains("pub async fn scan_tx_out_set"),
        "BTC client must not expose public scan_tx_out_set operation surface"
    );
    assert!(
        !btc_transport.contains("pub async fn address_balance_sats"),
        "BTC client must not expose public address_balance_sats surface"
    );
    assert!(
        !btc_transport.contains("pub trait BtcJsonRpcChainHeadTransport"),
        "BTC transport trait must not be a public production escape hatch"
    );
    assert!(
        !btc_transport
            .contains("pub fn new(routes: BTreeMap<BtcSourceIdentity, Arc<dyn BtcJsonRpc"),
        "BTC router must not accept public dyn transport maps"
    );
    assert!(
        btc_transport.contains("BTreeMap<BtcSourceIdentity, Arc<BtcJsonRpcClient>>"),
        "BTC router production constructor must accept concrete BtcJsonRpcClient map"
    );
    assert!(
        include_str!("../../btc-capabilities/src/lib.rs").contains("struct BtcSourceBinding"),
        "BTC capabilities must own BtcSourceBinding"
    );
    assert!(
        include_str!("../../evm-capabilities/src/lib.rs").contains("struct EvmNetworkBinding"),
        "EVM capabilities must own EvmNetworkBinding"
    );

    assert!(
        !evm_adapter.contains("fn verified_chain_identity"),
        "EVM contracts adapter must not re-own live chain identity gates"
    );
    assert!(
        !evm_adapter.contains("let _verified_chain"),
        "EVM contracts adapter must not gate mutations on extra live chain probes"
    );
    assert!(
        !evm_adapter.contains("ContractMutationNetworkAuthority"),
        "EVM contracts adapter must not use Authority naming for constructible network context"
    );

    // Recorded/replay paths must not consult live runtime config.
    for banned in [
        "RuntimeConfig",
        "EvmJsonRpcClient::new",
        "MFM_RUNTIME_CONFIG_FILE",
        "reqwest::Client",
    ] {
        // Only the recorded provider module section: whole adapter file may mention other things.
        // RecordedBtcChainHeadProvider implementation body is pure — scan the helper.
        assert!(
            !recorded_btc
                .split("impl BtcChainHeadReadProvider for RecordedBtcChainHeadProvider")
                .nth(1)
                .unwrap_or("")
                .split("/// Redaction-safe")
                .next()
                .unwrap_or("")
                .contains(banned),
            "recorded BTC provider must not use live surface {banned}"
        );
    }
}

#[test]
fn capability_operation_requests_are_source_free() {
    for (name, source) in [
        (
            "evm-capabilities",
            include_str!("../../evm-capabilities/src/lib.rs"),
        ),
        (
            "btc-capabilities",
            include_str!("../../btc-capabilities/src/lib.rs"),
        ),
    ] {
        // Extract request struct bodies approximately by scanning for `pub struct *Request`.
        let mut in_request = false;
        let mut brace_depth = 0i32;
        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("pub struct ") && trimmed.contains("Request") {
                // Skip evidence/response types that happen to contain Request in name.
                if trimmed.contains("Response") {
                    continue;
                }
                in_request = true;
                brace_depth = 0;
            }
            if !in_request {
                continue;
            }
            brace_depth += line.chars().filter(|c| *c == '{').count() as i32;
            brace_depth -= line.chars().filter(|c| *c == '}').count() as i32;
            for banned in [
                "network_id:",
                "expected_chain_id:",
                "source_identity:",
                "bitcoin_network:",
            ] {
                assert!(
                    !trimmed.starts_with(banned) && !trimmed.contains(&format!(" {banned}")),
                    "{name} operation request must not contain source-binding field {banned}: {trimmed}"
                );
            }
            if brace_depth <= 0 && trimmed.contains('}') {
                in_request = false;
            }
        }
    }
}
