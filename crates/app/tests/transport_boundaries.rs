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
        ] {
            assert!(
                !source.contains(banned),
                "{name} must not retain forbidden bound-provider escape hatch {banned}"
            );
        }
    }

    let evm_transport = include_str!("../../transports/evm/src/lib.rs");
    assert!(
        !evm_transport.contains("impl EvmBlockReadProvider for EvmJsonRpcClient"),
        "raw EvmJsonRpcClient must not implement live capability provider traits"
    );
    assert!(
        evm_transport.contains("struct EvmJsonRpcNetworkProvider"),
        "EVM transport must expose a bound network provider"
    );
    assert!(
        include_str!("../../transports/btc-jsonrpc-http/src/lib.rs")
            .contains("struct BtcJsonRpcSourceProvider"),
        "BTC transport must expose a bound source provider"
    );
    assert!(
        include_str!("../../btc-capabilities/src/lib.rs").contains("struct BtcSourceBinding"),
        "BTC capabilities must own BtcSourceBinding"
    );
    assert!(
        include_str!("../../evm-capabilities/src/lib.rs").contains("struct EvmNetworkBinding"),
        "EVM capabilities must own EvmNetworkBinding"
    );
}
