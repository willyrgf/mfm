use mfm_btc_capabilities::*;
use mfm_capabilities::CapabilitySpec;

#[test]
fn capability_name_describes_read_authority() {
    assert_eq!(
        BtcChainHeadReadCapability::name(),
        "mfm.bitcoin.chain_head.read"
    );
    assert!(!BtcChainHeadReadCapability::name().contains("collector"));
    assert!(!BtcChainHeadReadCapability::name().contains("workflow"));
}

#[test]
fn capability_kind_and_version_are_stable() {
    let kind = BtcChainHeadReadCapability::kind()
        .expect("kind")
        .to_string();
    let version = BtcChainHeadReadCapability::version()
        .expect("version")
        .to_string();

    assert!(kind.starts_with("capability:mfm.bitcoin:chain_head.read:"));
    assert_eq!(version, "mfm.bitcoin.chain_head.read.v1");
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
}

#[test]
fn source_identity_rejects_route_like_material() {
    let error = BtcSourceIdentity::new("http://node.invalid:8332")
        .expect_err("source identity should reject route material");

    assert_eq!(
        error,
        BtcCapabilityError::InvalidRequest {
            reason: BtcInvalidRequest::InvalidIdentifier,
        }
    );
}
