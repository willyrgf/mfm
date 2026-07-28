use std::time::Duration;

use mfm_bitcoin::{
    BitcoinNetworkId, BitcoinNetworkTag, BitcoinRoutingGenerationRef, BitcoinSourceBinding,
    BitcoinSourceIdentity,
};
use mfm_bitcoin_live::transport::{
    BitcoinRpcAuthentication, BitcoinRpcEndpoint, BitcoinRpcError, BitcoinRpcSession,
};
use mfm_ids::{ContentDigest, ContentRef, DigestAlgorithm, DigestBytes, SchemaId};
use static_assertions::assert_not_impl_any;
use zeroize::Zeroizing;

assert_not_impl_any!(BitcoinRpcAuthentication: Clone, Copy, std::fmt::Debug, std::fmt::Display, serde::Serialize);
assert_not_impl_any!(BitcoinRpcAuthentication: serde::de::DeserializeOwned);
assert_not_impl_any!(BitcoinRpcAuthentication: AsRef<str>, std::borrow::Borrow<str>);

fn binding() -> BitcoinSourceBinding {
    BitcoinSourceBinding::new(
        BitcoinNetworkId::new("bitcoin-regtest").expect("network"),
        BitcoinNetworkTag::Regtest,
        BitcoinSourceIdentity::new("local-core").expect("source"),
    )
}

fn generation() -> BitcoinRoutingGenerationRef {
    let schema = SchemaId::new(
        "mfm.bitcoin.routing-generation",
        "1",
        DigestAlgorithm::Sha256JcsV1,
        DigestBytes::from_array([1; 32]),
    )
    .expect("schema");
    let digest =
        ContentDigest::from_digest(DigestAlgorithm::Sha256V1, DigestBytes::from_array([2; 32]));
    BitcoinRoutingGenerationRef::from_reviewed(ContentRef::new(schema, digest).expect("generation"))
}

#[test]
fn public_transport_constructs_without_runtime_or_registry_assembly() {
    let authentication = BitcoinRpcAuthentication::new(
        "resolved-user".to_owned(),
        Zeroizing::new("resolved-password".to_owned()),
    )
    .expect("auth");
    let session = BitcoinRpcSession::new(
        BitcoinRpcEndpoint::new("http://127.0.0.1:18443").expect("endpoint"),
        Some(authentication),
        binding(),
        generation(),
        Duration::from_secs(30),
    )
    .expect("checked public session");

    assert_eq!(session.binding(), &binding());
    assert_eq!(session.routing_generation(), &generation());
}

#[test]
fn public_transport_rejects_unchecked_endpoint_material() {
    let result = BitcoinRpcEndpoint::new("http://user:secret@127.0.0.1:18443");
    assert!(matches!(result, Err(BitcoinRpcError::InvalidConfiguration)));
}
