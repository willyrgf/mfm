use mfm_btc_capabilities::*;
use mfm_capabilities::CapabilitySpec;

#[test]
fn capability_identity_describes_read_authority() {
    assert_eq!(
        BtcChainHeadReadCapability::name(),
        "mfm.bitcoin.chain_head.read"
    );
    assert_eq!(BtcBalanceReadCapability::name(), "mfm.bitcoin.balance.read");
    assert!(!BtcChainHeadReadCapability::name().contains("collector"));
    assert!(!BtcChainHeadReadCapability::name().contains("workflow"));
    assert!(!BtcBalanceReadCapability::name().contains("collector"));
    assert!(!BtcBalanceReadCapability::name().contains("workflow"));

    let kind = BtcChainHeadReadCapability::kind()
        .expect("kind")
        .to_string();
    let version = BtcChainHeadReadCapability::version()
        .expect("version")
        .to_string();
    let balance_kind = BtcBalanceReadCapability::kind()
        .expect("balance kind")
        .to_string();
    let balance_version = BtcBalanceReadCapability::version()
        .expect("balance version")
        .to_string();

    assert!(kind.starts_with("capability:mfm.bitcoin:chain_head.read:"));
    assert_eq!(version, "mfm.bitcoin.chain_head.read.v1");
    assert!(balance_kind.starts_with("capability:mfm.bitcoin:balance.read:"));
    assert_eq!(balance_version, "mfm.bitcoin.balance.read.v1");
}

#[test]
fn contracts_do_not_expose_concrete_source_details() {
    let source = include_str!("../src/lib.rs");
    let forbidden = [
        concat!("rpc", "_", "url"),
        concat!("end", "point"),
        concat!("author", "ization"),
        concat!("api", "_", "key"),
        concat!("bear", "er"),
        concat!("pass", "word"),
        concat!("private", "_", "key"),
        concat!("mn", "emonic"),
        concat!("key", "store", "_", "path"),
    ];

    for term in forbidden {
        assert!(
            !source.to_ascii_lowercase().contains(term),
            "forbidden concrete source detail: {term}"
        );
    }
}

#[test]
fn source_identity_is_semantic_not_route_material() {
    let identity = BtcSourceIdentity::new("public-bitcoin-core").expect("source identity");

    assert_eq!(identity.as_str(), "public-bitcoin-core");

    let error = BtcSourceIdentity::new("http://node.invalid:8332")
        .expect_err("source identity should reject route material");

    assert_eq!(
        error,
        BtcCapabilityError::InvalidRequest {
            reason: BtcInvalidRequest::InvalidIdentifier,
        }
    );
}

#[test]
fn balance_response_evidence_verifies_request() {
    let block_hash =
        BtcBlockHash::new("00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc")
            .expect("block hash");
    let request = BtcBalanceReadRequest {
        guard: BtcChainGuard::new(
            BtcNetworkId::new("bitcoin-mainnet").expect("network"),
            BtcSourceIdentity::new("bitcoin-mainnet").expect("source"),
            "main",
        )
        .expect("guard"),
        address: BtcAddress::new("bc1qns9f7yfx3ry9lj6yz7c9er0vwa0ye2eklpzqfw").expect("address"),
        block_height: 850_000,
        block_hash: block_hash.clone(),
    };
    let evidence =
        RedactedBtcSourceEvidence::from_guard(&request.guard, "main", BtcSourceStatus::Synced)
            .expect("evidence");
    let response = BtcBalanceReadResponse {
        evidence,
        address: request.address.clone(),
        balance_sats: 42,
        block_height: request.block_height,
        block_hash,
    };

    response
        .verify_request(&request)
        .expect("matching balance evidence");
}
