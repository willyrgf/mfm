use mfm_bitcoin::*;
use mfm_capabilities::CapabilitySpec;

#[test]
fn capability_identity_describes_one_aggregate_read_authority() {
    assert_eq!(
        BitcoinBalanceCollectionReadCapability::name(),
        "mfm.bitcoin.balance_collection.read"
    );
    assert_eq!(
        BitcoinBalanceCollectionReadCapability::version()
            .expect("version")
            .to_string(),
        "mfm.bitcoin.balance_collection.read.v1"
    );
    assert!(BitcoinBalanceCollectionReadCapability::kind()
        .expect("kind")
        .to_string()
        .starts_with("capability:mfm.bitcoin:balance_collection.read:"));
}

#[test]
fn contracts_do_not_expose_concrete_source_details() {
    let source = concat!(
        include_str!("../src/model.rs"),
        include_str!("../src/capability.rs")
    );
    for term in [
        concat!("rpc", "_", "url"),
        concat!("end", "point"),
        concat!("author", "ization"),
        concat!("api", "_", "key"),
        concat!("bear", "er"),
        concat!("pass", "word"),
        concat!("private", "_", "key"),
        concat!("mn", "emonic"),
        concat!("key", "store", "_", "path"),
    ] {
        assert!(
            !source.to_ascii_lowercase().contains(term),
            "forbidden concrete source detail: {term}"
        );
    }
}

#[test]
fn source_identity_is_semantic_not_route_material() {
    let identity = BitcoinSourceIdentity::new("public-bitcoin-core").expect("source identity");
    assert_eq!(identity.as_str(), "public-bitcoin-core");
    assert_eq!(
        BitcoinSourceIdentity::new("http://node.invalid:8332")
            .expect_err("route material is not an identity"),
        BitcoinCapabilityError::InvalidRequest {
            reason: BitcoinInvalidRequest::InvalidSourceIdentity,
        }
    );
}
