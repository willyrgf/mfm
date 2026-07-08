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
fn transport_provider_boundaries_reject_old_source_binding_surfaces() {
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
            "evm-contract-adapter",
            include_str!("../../adapters/evm-contracts/src/lib.rs"),
        ),
        (
            "portfolio-adapter",
            include_str!("../../adapters/portfolio/src/lib.rs"),
        ),
        ("app", include_str!("../src/lib.rs")),
        ("app-evm-contracts", include_str!("../src/evm_contracts.rs")),
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
            &[
                "EvmChainIdentityRequest",
                "EvmBlockReadRequest",
                "EvmBalanceReadRequest",
                "EvmCallReadRequest",
                "EvmCodeReadRequest",
                "EvmLogsReadRequest",
                "EvmNonceReadRequest",
                "EvmFeeReadRequest",
                "EvmGasEstimateRequest",
                "EvmTransactionSubmitRequest",
                "EvmReceiptReadRequest",
                "EvmNonceOccupancyReadRequest",
            ],
            &["network_id", "expected_chain_id"],
        );
        assert_no_source_bearing_request_constructors(
            name,
            source,
            &["BtcChainHeadRequest", "BtcBalanceReadRequest"],
            &["network_id", "source_identity", "bitcoin_network"],
        );
    }

    let evm_transport = include_str!("../../transports/evm/src/lib.rs");
    for provider_trait in [
        "EvmChainIdentityProvider",
        "EvmBlockReadProvider",
        "EvmBalanceReadProvider",
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
            !evm_transport.contains(&format!("impl {provider_trait} for EvmJsonRpcClient")),
            "raw EvmJsonRpcClient must not implement {provider_trait}"
        );
    }
    assert!(
        evm_transport.contains("impl_network_provider!(")
            && evm_transport.contains("impl $trait for EvmJsonRpcNetworkProvider"),
        "EVM transport provider impl macro must target the bound network provider"
    );
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
