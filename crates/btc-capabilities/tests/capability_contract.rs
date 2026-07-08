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
fn balance_request_exposes_semantic_accessors_and_builds_evidence() {
    let block_hash =
        BtcBlockHash::new("00000000000000000001b2a7f3e0d5c4b6a897887766554433221100ffeeddcc")
            .expect("block hash");
    let address = BtcAddress::new("bc1qns9f7yfx3ry9lj6yz7c9er0vwa0ye2eklpzqfw").expect("address");
    let request = BtcBalanceReadRequest::new(
        BtcNetworkId::new("bitcoin-mainnet").expect("network"),
        BtcSourceIdentity::new("bitcoin-mainnet").expect("source"),
        "main",
        address.clone(),
        850_000,
        block_hash.clone(),
    )
    .expect("request");
    let evidence =
        RedactedBtcSourceEvidence::from_balance_request(&request, "main", BtcSourceStatus::Synced)
            .expect("evidence");
    let response = BtcBalanceReadResponse {
        evidence,
        address: address.clone(),
        balance_sats: 42,
        block_height: request.block_height(),
        block_hash: block_hash.clone(),
    };

    assert_eq!(request.network_id().as_str(), "bitcoin-mainnet");
    assert_eq!(request.source_identity().as_str(), "bitcoin-mainnet");
    assert_eq!(request.bitcoin_network(), "main");
    assert_eq!(request.address(), &address);
    assert_eq!(request.block_height(), 850_000);
    assert_eq!(request.block_hash(), &block_hash);
    assert_eq!(response.evidence.network_id, request.network_id().clone());
    assert_eq!(
        response.evidence.source_identity,
        request.source_identity().clone()
    );
    assert_eq!(response.evidence.bitcoin_network, request.bitcoin_network());
    assert_eq!(
        response.evidence.observed_bitcoin_network,
        request.bitcoin_network()
    );
    assert_eq!(response.address, address);
    assert_eq!(response.block_height, request.block_height());
    assert_eq!(response.block_hash, block_hash);
}
