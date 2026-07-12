use super::*;

fn binding() -> BtcSourceBinding {
    BtcSourceBinding::new(
        BtcNetworkId::new("bitcoin-mainnet").expect("network"),
        BtcSourceIdentity::new("public-bitcoin-core").expect("source"),
        BitcoinNetworkTag::Main,
    )
}

fn request(selection: BtcHeadSelection) -> BtcChainHeadRequest {
    BtcChainHeadRequest::new(selection)
}

#[test]
fn confirmation_depth_rejects_zero() {
    let error = BtcHeadSelection::confirmed(0).expect_err("zero confirmations");

    assert_eq!(
        error,
        BtcCapabilityError::InvalidRequest {
            reason: BtcInvalidRequest::ZeroConfirmations,
        }
    );
}

#[test]
fn block_hash_validation_rejects_non_hash_values() {
    let error = BtcBlockHash::new("not-a-block-hash").expect_err("invalid hash");

    assert_eq!(
        error,
        BtcCapabilityError::InvalidRequest {
            reason: BtcInvalidRequest::InvalidBlockHash,
        }
    );
}

#[test]
fn block_hash_validation_normalizes_to_lowercase() {
    let hash =
        BtcBlockHash::new("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
            .expect("hash");

    assert_eq!(
        hash.as_str(),
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
}

#[test]
fn chain_head_request_is_operation_only() {
    let request = request(BtcHeadSelection::best());

    assert_eq!(request.selection(), BtcHeadSelection::best());
}

#[test]
fn response_evidence_is_built_from_provider_binding() {
    let binding = binding();
    let evidence =
        RedactedBtcSourceEvidence::from_binding(&binding, "main", BtcSourceStatus::Synced)
            .expect("evidence");

    assert_eq!(evidence.network_id, binding.network_id().clone());
    assert_eq!(evidence.source_identity, binding.source_identity().clone());
    assert_eq!(evidence.bitcoin_network, binding.bitcoin_network().as_str());
    assert_eq!(
        evidence.observed_bitcoin_network,
        binding.bitcoin_network().as_str()
    );
    assert_eq!(evidence.source_status, BtcSourceStatus::Synced);
}

#[test]
fn source_mismatch_diagnostic_stays_redacted() {
    let binding = binding();
    let mismatched = RedactedBtcSourceEvidence {
        source_identity: BtcSourceIdentity::new("different-semantic-source").expect("source"),
        ..RedactedBtcSourceEvidence::from_binding(&binding, "main", BtcSourceStatus::Unknown)
            .expect("evidence")
    };

    let diagnostic = mismatched.source_mismatch_diagnostic();
    let rendered = format!("{diagnostic:?} {diagnostic}");

    assert!(rendered.contains("different-semantic-source"));
    assert!(!rendered.contains("http://"));
    assert!(!rendered.contains(concat!("Bear", "er")));
    assert!(!rendered.contains("secret"));
}

#[test]
fn response_evidence_rejects_observed_bitcoin_network_mismatch() {
    let error =
        RedactedBtcSourceEvidence::from_binding(&binding(), "test", BtcSourceStatus::Synced)
            .expect_err("observed network mismatch");

    assert!(matches!(error, BtcCapabilityError::SourceMismatch { .. }));
}

#[test]
fn provider_failure_carries_only_closed_diagnostics() {
    let diagnostic = btc_diagnostic(ProviderDiagnosticCode::RpcHttpStatus)
        .with_operation(btc_public_id("scantxoutset"))
        .with_field(
            btc_public_id("http_status"),
            ProviderDiagnosticValue::U64(403),
        );
    let error = BtcCapabilityError::provider_failure(diagnostic.clone());
    let rendered = format!("{error:?} {error}");

    assert_eq!(error, BtcCapabilityError::Provider { diagnostic });
    assert!(!rendered.contains("node.invalid"));
    assert!(!rendered.contains(concat!("Author", "ization")));
    assert!(!rendered.contains("secret"));
}
