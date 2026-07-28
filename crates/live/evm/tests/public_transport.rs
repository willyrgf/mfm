use mfm_evm_live::transport::{EvmRpcAuthorization, EvmRpcEndpoint, EvmTransportError};
use mfm_evm_live::EvmReadQualificationArtifacts;
use static_assertions::assert_not_impl_any;
use zeroize::Zeroizing;

assert_not_impl_any!(EvmRpcAuthorization: Clone, Copy, std::fmt::Display, serde::Serialize);
assert_not_impl_any!(EvmRpcAuthorization: serde::de::DeserializeOwned);
assert_not_impl_any!(EvmRpcAuthorization: AsRef<str>, std::borrow::Borrow<str>);
assert_not_impl_any!(
    EvmReadQualificationArtifacts<'static>:
        Clone,
        Copy,
        serde::Serialize,
        serde::de::DeserializeOwned
);

#[test]
fn public_transport_rejects_and_redacts_private_route_material() {
    for invalid in [
        "http://user:secret@127.0.0.1:8545",
        "http://127.0.0.1:8545?token=secret",
        "http://127.0.0.1:8545/#secret",
        "file:///tmp/evm.sock",
    ] {
        assert_eq!(
            EvmRpcEndpoint::new(invalid).expect_err("invalid endpoint"),
            EvmTransportError::InvalidConfiguration
        );
    }

    let endpoint = EvmRpcEndpoint::new("https://rpc.example.invalid/private").expect("endpoint");
    assert_eq!(format!("{endpoint:?}"), "EvmRpcEndpoint(<redacted>)");
    let authorization =
        EvmRpcAuthorization::new(Zeroizing::new("Bearer resolved-secret".to_owned()))
            .expect("authorization");
    assert_eq!(
        format!("{authorization:?}"),
        "EvmRpcAuthorization(<redacted>)"
    );
}
