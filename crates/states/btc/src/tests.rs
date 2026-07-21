use super::*;

#[test]
fn state_error_from_capability_provider_failure_is_redacted() {
    let diagnostic = mfm_btc_capabilities::btc_diagnostic(
        mfm_capabilities::ProviderDiagnosticCode::TransportFailed,
    );
    let error = BtcStateError::from(BtcCapabilityError::provider_failure(diagnostic));
    let text = error.to_string();

    assert_eq!(error, BtcStateError::ProviderFailed);
    assert!(!text.contains("password"));
    assert!(!text.contains("localhost"));
    assert!(!text.contains("http://"));
}

#[test]
fn state_crate_manifest_stays_inside_state_boundaries() {
    let manifest = include_str!("../Cargo.toml");

    for forbidden in [
        "mfm-app",
        "mfm-runtime",
        "mfm-store",
        "mfm-storage-postgres",
        "mfm-btc-jsonrpc-http",
        "mfm-adapters-btc-jsonrpc",
        "mfm-op-btc-collectors",
    ] {
        assert!(
            !manifest.contains(forbidden),
            "state crate must not depend on forbidden boundary crate {forbidden}"
        );
    }
}
