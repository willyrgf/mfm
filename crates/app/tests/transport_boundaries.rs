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
        "mfm-machine",
        "mfm-sdk",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "production app manifest must not link legacy transport dependency {forbidden}"
        );
    }
}

#[test]
fn transport_sources_exclude_legacy_io_surfaces() {
    for (name, source) in [
        ("proof", include_str!("../../transports/proof/src/lib.rs")),
        (
            "portfolio",
            include_str!("../../adapters/portfolio/src/lib.rs"),
        ),
    ] {
        for banned in [
            concat!("mfm_", "machine"),
            concat!("Io", "Provider"),
            concat!("Live", "IoTransport"),
            concat!("Live", "IoTransportFactory"),
            concat!("Dyn", "Context"),
            concat!("Planned", "Op"),
            concat!("Port", "Key"),
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
                "transport {name} must not expose legacy IO surface {banned}"
            );
        }
    }
}
