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
fn private_adapter_depends_only_on_the_pure_session_contract() {
    let source = include_str!("adapter/mod.rs");
    for forbidden in [
        "crate::transport",
        "super::transport",
        "BitcoinRpcSession",
        "BitcoinRpcAuthentication",
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
        concat!("adap", "ter"),
        concat!("run", "ner"),
        concat!("run", "time"),
        concat!("re", "play"),
        concat!("mfm_", "store"),
        concat!("mfm_", "app"),
    ] {
        assert!(
            !source.contains(forbidden),
            "transport source must not name platform binding authority {forbidden}"
        );
    }
}
