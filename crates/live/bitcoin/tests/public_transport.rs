use std::time::Duration;

use mfm_bitcoin::{
    BitcoinBalanceSession, BitcoinNetworkId, BitcoinNetworkTag, BitcoinSourceBinding,
    BitcoinSourceIdentity, BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID,
};
use mfm_bitcoin_live::transport::{
    BitcoinRpcAuthentication, BitcoinRpcEndpoint, BitcoinRpcError, BitcoinRpcSession,
};
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

fn require_pure_session<T: BitcoinBalanceSession>(_session: &T) {}

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
        Duration::from_secs(30),
    )
    .expect("checked public session");

    require_pure_session(&session);
    assert_eq!(session.binding(), &binding());
    assert_eq!(
        session.implementation_id(),
        BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID
    );
}

#[test]
fn public_transport_rejects_unchecked_endpoint_material() {
    let result = BitcoinRpcEndpoint::new("http://user:secret@127.0.0.1:18443");
    assert!(matches!(result, Err(BitcoinRpcError::InvalidConfiguration)));
}
