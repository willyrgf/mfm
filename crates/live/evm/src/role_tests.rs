#[test]
fn live_root_exposes_transport_without_exposing_adapter() {
    let root = include_str!("lib.rs");
    assert!(root.contains("pub mod transport;"));
    assert!(root.contains("mod adapter;"));
    assert!(!root.contains("pub mod adapter;"));
    assert!(!root.contains("pub use transport::"));
    assert!(!root.contains("pub use adapter::*"));
}

#[test]
fn private_adapter_depends_only_on_pure_session_contracts() {
    let source = concat!(
        include_str!("adapter/mod.rs"),
        include_str!("adapter/balance_collection.rs"),
        include_str!("adapter/transaction.rs"),
    );
    for forbidden in [
        "crate::transport",
        "super::transport",
        "EvmJsonRpcSession",
        "EvmJsonRpcTransport",
        "EvmRpcEndpoint",
        "EvmRpcAuthorization",
    ] {
        assert!(
            !source.contains(forbidden),
            "adapter must not depend on concrete transport symbol {forbidden}"
        );
    }
}

#[test]
fn transport_source_has_no_platform_binding_authority() {
    let source = include_str!("transport/mod.rs");
    for forbidden in [
        concat!("mfm_", "runtime::"),
        concat!("mfm_", "replay::"),
        concat!("mfm_", "store::"),
        concat!("mfm_", "app::"),
        "ErasedRunnerRegistry",
        "ReplayBroker",
        "RetainedArtifactReadProvider",
    ] {
        assert!(
            !source.contains(forbidden),
            "transport source must not name platform binding authority {forbidden}"
        );
    }
}
