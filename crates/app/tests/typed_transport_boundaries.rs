#![allow(clippy::disallowed_methods)]

#[test]
fn production_typed_app_links_only_typed_transport_crates() {
    let manifest = include_str!("../Cargo.toml");
    for required in [
        "mfm-transports-proof",
        "mfm-transports-portfolio",
        "mfm-transports-evm-dcv",
    ] {
        assert!(
            manifest.contains(required),
            "typed app manifest must link {required}"
        );
    }
    for forbidden in [
        "mfm-transports-exec",
        "mfm-transports-local-evm",
        "mfm-transports-local-fs",
        "mfm-transports-local-keystore",
        "mfm-transports-rpc-control",
        "mfm-machine",
        "mfm-sdk",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "typed app manifest must not link legacy transport dependency {forbidden}"
        );
    }
}

#[test]
fn typed_transport_sources_exclude_legacy_io_surfaces() {
    for (name, source) in [
        ("proof", include_str!("../../transports/proof/src/lib.rs")),
        (
            "portfolio",
            include_str!("../../transports/portfolio/src/lib.rs"),
        ),
        (
            "evm-dcv",
            include_str!("../../transports/evm-dcv/src/lib.rs"),
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
        ] {
            assert!(
                !source.contains(banned),
                "typed transport {name} must not expose legacy IO surface {banned}"
            );
        }
    }
}
