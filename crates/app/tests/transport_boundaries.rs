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
fn transport_provider_boundaries_keep_one_bound_session_per_evm_view() {
    let production_sources = [
        (
            "btc-capabilities",
            include_str!("../../btc-capabilities/src/lib.rs"),
        ),
        (
            "evm-capabilities",
            include_str!("../../evm-capabilities/src/lib.rs"),
        ),
        (
            "btc-jsonrpc-transport",
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
            "portfolio-adapter",
            include_str!("../../adapters/portfolio/src/lib.rs"),
        ),
        ("app", include_str!("../src/lib.rs")),
        ("app-btc-collector", include_str!("../src/btc_collector.rs")),
    ];

    for (name, source) in production_sources {
        for forbidden in [
            concat!("EvmRequest", "Authority"),
            concat!("EvmRequest", "Source"),
            concat!("BtcRequest", "Source"),
            concat!("BtcJsonRpc", "ChainHeadProvider"),
            concat!("PortfolioRuntime", "Validator"),
            concat!("pub fn with_", "middleware"),
            concat!("pub trait Middle", "ware"),
            concat!("pub struct Middle", "ware"),
            concat!("pub trait Lay", "er"),
            concat!("pub struct Lay", "er"),
            concat!("pub fn in", "ner("),
            concat!("pub fn un", "checked"),
            concat!("skip_", "validation"),
            concat!("pub struct EvmSource", "Guard"),
            concat!("pub struct EvmChain", "Guard"),
            concat!("pub struct BtcSource", "Guard"),
            concat!("pub struct BtcChain", "Guard"),
            concat!("pub struct Source", "Guard"),
            concat!("pub struct Chain", "Guard"),
        ] {
            assert!(
                !source.contains(forbidden),
                "{name} must not expose old live transport source-binding surface {forbidden}"
            );
        }

        assert_no_source_bearing_request_constructors(
            name,
            source,
            &["BtcChainHeadRequest", "BtcBalanceReadRequest"],
            &["network_id", "source_identity", "bitcoin_network"],
        );
    }

    let evm_capabilities = include_str!("../../evm-capabilities/src/lib.rs");
    assert!(evm_capabilities.contains("EvmReadCapability"));
    assert!(evm_capabilities.contains("pub trait EvmReadSession"));
    assert!(evm_capabilities.contains("EvmTransactionCapability"));
    assert!(evm_capabilities.contains("pub trait EvmTransactionSession"));
    for deleted in [
        concat!("EvmBlockRead", "Provider"),
        concat!("EvmBalanceRead", "Provider"),
        concat!("EvmCallRead", "Provider"),
        concat!("EvmSource", "PolicyId"),
    ] {
        assert!(
            !evm_capabilities.contains(deleted),
            "EVM capability surface must not retain {deleted}"
        );
    }

    let evm_transport = include_str!("../../transports/evm/src/lib.rs");
    assert!(
        evm_transport.contains("pub struct EvmJsonRpcSession")
            && evm_transport.contains("impl EvmReadSession for EvmJsonRpcSession")
            && evm_transport.contains("impl EvmTransactionSession for EvmJsonRpcSession"),
        "EVM transport must expose the two views on one bound session"
    );
    assert_eq!(
        evm_transport.matches("rpc_call(\"eth_chainId\"").count(),
        1,
        "a session must probe chain identity exactly once while binding"
    );
    for deleted in [
        concat!("EvmSource", "Registry"),
        concat!("EvmRoute", "Registry"),
        concat!("EvmSource", "Policy"),
        concat!("EvmJsonRpc", "Client"),
    ] {
        assert!(
            !evm_transport.contains(deleted),
            "EVM transport must not retain routing/client surface {deleted}"
        );
    }

    let evm_adapter = include_str!("../../adapters/evm/src/lib.rs");
    let app_evm = include_str!("../src/evm_collector.rs");
    assert!(
        !evm_adapter.contains(concat!("EvmProvider", "Factory"))
            && !evm_adapter.contains(concat!("EvmBound", "Provider"))
            && !app_evm.contains(concat!("BoundLiveEvm", "Provider")),
        "adapter and app must bind sessions directly without provider proxies"
    );

    let btc_transport = include_str!("../../transports/btc-jsonrpc-http/src/lib.rs");
    for forbidden in [
        "pub trait BtcJsonRpcChainHeadTransport",
        "pub type BtcTransportFuture",
        "pub async fn get_blockchain_info",
        "pub async fn get_block_hash",
        "pub async fn get_block_header",
        "pub async fn scan_tx_out_set",
        "pub async fn address_balance_sats",
    ] {
        assert!(
            !btc_transport.contains(forbidden),
            "BTC transport must not expose raw live JSON-RPC bypass surface {forbidden}"
        );
    }
    for provider_trait in ["BtcChainHeadReadProvider", "BtcBalanceReadProvider"] {
        for raw_type in ["BtcJsonRpcClient", "BtcJsonRpcRouter"] {
            assert!(
                !btc_transport.contains(&format!("impl {provider_trait} for {raw_type}")),
                "raw BTC type {raw_type} must not implement {provider_trait}"
            );
        }
    }
    assert_btc_operation_io_requires_verified_call(btc_transport);

    let app = include_str!("../src/services_read.rs");
    assert!(
        app.contains("ReplayVerifierRegistry::production().verify(&broker"),
        "app replay must use the compiled recorded-evidence verifier registry"
    );
}

fn assert_btc_operation_io_requires_verified_call(source: &str) {
    for operation in ["get_block_hash", "get_block_header", "scan_tx_out_set"] {
        for signature in [format!("fn {operation}("), format!("fn {operation}<'")] {
            for (offset, _) in source.match_indices(&signature) {
                let args = function_args(source, offset);
                assert!(
                    args.contains("VerifiedBtcCall"),
                    "BTC operation helper {operation} must require VerifiedBtcCall"
                );
            }
        }
    }

    for (offset, _) in source.match_indices("fn selected_head") {
        let args = function_args(source, offset);
        assert!(
            args.contains("VerifiedBtcCall")
                && !args.contains("BtcJsonRpcChainHeadTransport")
                && !args.contains("BlockchainInfo"),
            "BTC selected_head must use VerifiedBtcCall instead of raw transport or probe output"
        );
    }
}

fn assert_no_source_bearing_request_constructors(
    name: &str,
    source: &str,
    request_types: &[&str],
    forbidden_args: &[&str],
) {
    for request_type in request_types {
        let constructor = format!("{request_type}::new(");
        for (offset, _) in source.match_indices(&constructor) {
            let args = constructor_args(source, offset, &constructor);
            for forbidden_arg in forbidden_args {
                assert!(
                    !args.contains(forbidden_arg),
                    "{name} must not pass {forbidden_arg} into {request_type}::new"
                );
            }
        }
    }
}

fn function_args(source: &str, offset: usize) -> &str {
    let Some(open_relative) = source[offset..].find('(') else {
        return "";
    };
    let start = offset + open_relative + 1;
    let mut depth = 1_usize;
    for (relative, ch) in source[start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &source[start..start + relative];
                }
            }
            _ => {}
        }
    }
    &source[start..source.len().min(start + 240)]
}

fn constructor_args<'a>(source: &'a str, offset: usize, constructor: &str) -> &'a str {
    let start = offset + constructor.len();
    let mut depth = 1_usize;
    for (relative, ch) in source[start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return &source[start..start + relative];
                }
            }
            _ => {}
        }
    }
    &source[start..source.len().min(start + 240)]
}
