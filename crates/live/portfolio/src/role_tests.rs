#[test]
fn portfolio_live_has_no_transport_or_concrete_store_dependency() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in [
        "mfm-bitcoin-live",
        "mfm-evm-live",
        "mfm-storage-postgres",
        "mfm-app",
        "reqwest",
        "sqlx",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "portfolio live manifest must not depend on {forbidden}"
        );
    }
}

#[test]
fn registration_accepts_exactly_one_shared_store_value() {
    let source = include_str!("lib.rs");
    assert!(source.contains("pub fn register_portfolio_live<S>"));
    assert!(source.contains("store: Arc<S>"));
    assert!(source.contains("SelectHoldingsExecutor::new(store)"));
    assert!(!source.contains("PortfolioRunnerCapabilities"));
    assert!(!source.contains("query_store:"));
    assert!(!source.contains("artifact_store:"));
}

#[test]
fn live_source_uses_pure_canonical_decoding_without_provider_implementations() {
    let source = concat!(include_str!("lib.rs"), include_str!("selection.rs"));
    assert!(source.contains("PortfolioHoldingFactEvidence::from_canonical_bytes"));
    for forbidden in [
        "decode_bitcoin_balance_snapshot_response",
        "decode_evm_balance_snapshot_response",
        "impl store::FactQueryStore",
        "impl store::RetainedArtifactReadProvider",
        "serde_json",
        "transport",
    ] {
        assert!(
            !source.contains(forbidden),
            "portfolio live production source must not own {forbidden}"
        );
    }
}
