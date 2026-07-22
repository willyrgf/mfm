use mfm_evm::{
    EvmNetworkBinding, EvmReadSession, EvmReadSessionSet, EvmTransactionSession,
    EvmTransactionSessionSet, EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_evm_live::transport::{
    EvmJsonRpcTransport, EvmRpcAuthorization, EvmRpcEndpoint, EvmTransportError,
};
use mfm_ids::LocalPublicId;
use static_assertions::assert_not_impl_any;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use zeroize::Zeroizing;

assert_not_impl_any!(EvmRpcAuthorization: Clone, Copy, std::fmt::Debug, std::fmt::Display, serde::Serialize);
assert_not_impl_any!(EvmRpcAuthorization: serde::de::DeserializeOwned);
assert_not_impl_any!(EvmRpcAuthorization: AsRef<str>, std::borrow::Borrow<str>);

fn binding() -> EvmNetworkBinding {
    EvmNetworkBinding::new(LocalPublicId::new("ethereum-mainnet").expect("network"), 1)
        .expect("binding")
}

fn require_pure_sessions<T>(_session: &T)
where
    T: EvmReadSession + EvmTransactionSession + EvmReadSessionSet + EvmTransactionSessionSet,
{
}

#[tokio::test]
async fn public_transport_binds_without_runtime_or_registry_assembly() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("listener address");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut request = Vec::new();
        loop {
            let mut bytes = [0_u8; 1024];
            let read = socket.read(&mut bytes).await.expect("read request");
            assert!(read > 0, "JSON-RPC request ended before the method arrived");
            request.extend_from_slice(&bytes[..read]);
            if request
                .windows(b"eth_chainId".len())
                .any(|window| window == b"eth_chainId")
            {
                break;
            }
        }
        let body = r#"{"jsonrpc":"2.0","id":1,"result":"0x1"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response");
    });
    let endpoint = EvmRpcEndpoint::new(format!("http://{address}")).expect("endpoint");
    let authorization =
        EvmRpcAuthorization::new(Zeroizing::new("Bearer resolved-secret".to_owned()))
            .expect("authorization");
    let session = EvmJsonRpcTransport::new()
        .expect("transport")
        .bind(
            binding(),
            LocalPublicId::new("primary").expect("source"),
            endpoint,
            Some(authorization),
        )
        .await
        .expect("checked session");

    require_pure_sessions(&session);
    assert_eq!(
        EvmReadSession::evidence(&session).implementation_id(),
        EVM_JSONRPC_SESSION_IMPLEMENTATION_ID
    );
    EvmReadSessionSet::validate_binding(&session, &binding())
        .await
        .expect("singleton read binding");
    EvmReadSessionSet::session(&session, &binding())
        .await
        .expect("singleton read session");
    EvmTransactionSessionSet::session(&session, &binding())
        .await
        .expect("singleton transaction session");
    server.await.expect("server");
}

#[test]
fn public_transport_rejects_and_redacts_endpoint_material() {
    for invalid in [
        "http://user:secret@127.0.0.1:8545",
        "http://127.0.0.1:8545?token=secret",
        "http://127.0.0.1:8545/#secret",
        "file:///tmp/evm.sock",
    ] {
        assert!(matches!(
            EvmRpcEndpoint::new(invalid),
            Err(EvmTransportError::InvalidConfiguration)
        ));
    }

    let endpoint = EvmRpcEndpoint::new("https://rpc.example.invalid/private").expect("endpoint");
    assert_eq!(format!("{endpoint:?}"), "EvmRpcEndpoint(<redacted>)");
}
