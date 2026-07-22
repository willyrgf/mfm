use super::*;
use mfm_bitcoin::{BitcoinNetworkId, BitcoinNetworkTag, BitcoinSourceIdentity};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

struct TestDispatch {
    shared: Arc<SharedLiveTransports>,
    routes: LiveDispatchRoutes,
}

impl TestDispatch {
    fn new(runtime_config: RuntimeConfigLoader) -> Self {
        let shared = Arc::new(SharedLiveTransports::new());
        let routes = shared.new_dispatch(runtime_config);
        Self { shared, routes }
    }

    fn evm_read_sessions(&self) -> Arc<dyn EvmReadSessionSet> {
        self.routes.evm_read_sessions()
    }

    fn bitcoin_session(&self) -> Arc<dyn BitcoinBalanceSession> {
        self.routes.bitcoin_session()
    }
}

fn evm_binding() -> EvmNetworkBinding {
    EvmNetworkBinding::new(LocalPublicId::new("test-evm").expect("network"), 1).expect("binding")
}

fn btc_binding() -> BitcoinSourceBinding {
    BitcoinSourceBinding::new(
        BitcoinNetworkId::new("test-btc").expect("network"),
        BitcoinNetworkTag::Test,
        BitcoinSourceIdentity::new("primary").expect("source"),
    )
}

fn runtime_with_config(raw: &str) -> (tempfile::TempDir, TestDispatch) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("runtime-secret.toml");
    std::fs::write(&path, raw).expect("write runtime config");
    let runtime = TestDispatch::new(RuntimeConfigLoader::from_path(Some(&path)));
    (dir, runtime)
}

async fn start_evm_chain_server(
    chain_id: &'static str,
) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("listener address");
    let requests = Arc::new(AtomicUsize::new(0));
    let server_requests = Arc::clone(&requests);
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = [0_u8; 4096];
            let read = socket.read(&mut request).await.expect("read request");
            assert!(read > 0, "JSON-RPC request must not be empty");
            server_requests.fetch_add(1, Ordering::SeqCst);
            let body = format!(r#"{{"jsonrpc":"2.0","id":1,"result":"{chain_id}"}}"#);
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        }
    });
    (format!("http://{address}"), requests, server)
}

#[tokio::test(flavor = "current_thread")]
async fn missing_config_preserves_evm_semantic_binding() {
    let runtime = TestDispatch::new(RuntimeConfigLoader::from_path(None));
    let sessions = runtime.evm_read_sessions();
    let binding = evm_binding();
    let error = sessions
        .validate_binding(&binding)
        .await
        .expect_err("missing EVM configuration");
    let diagnostic = error.redacted_diagnostic().expect("diagnostic");

    assert_eq!(
        diagnostic.code(),
        ProviderDiagnosticCode::ProviderConfigurationMissing
    );
    assert_eq!(diagnostic.provider_family().as_str(), "evm");
    assert_eq!(
        serde_json::to_value(diagnostic).expect("diagnostic JSON")["fields"],
        serde_json::json!({"expected_chain_id": 1, "network_id": "test-evm"})
    );
}

#[tokio::test(flavor = "current_thread")]
async fn selective_evm_route_file_loading_runs_on_a_blocking_worker() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("runtime.toml");
    std::fs::write(
        &path,
        r#"
[evm.routes.test-evm]
source_ref = "primary"
rpc_url = { direct = "http://127.0.0.1:8545" }
"#,
    )
    .expect("write runtime config");
    let binding = evm_binding();
    let async_worker = std::thread::current().id();

    let file_worker = load_runtime_config_on_blocking_worker(move || {
        runtime_config::load_evm_route(&path, binding.network_id()).expect("selective EVM route");
        std::thread::current().id()
    })
    .await
    .expect("blocking route-load worker");

    assert_ne!(file_worker, async_worker);
}

#[tokio::test(flavor = "current_thread")]
async fn missing_config_reports_bitcoin_route_failure() {
    let runtime = TestDispatch::new(RuntimeConfigLoader::from_path(None));
    let binding = btc_binding();
    let error = runtime
        .bitcoin_session()
        .validate_binding(&binding)
        .await
        .expect_err("missing Bitcoin configuration");
    let BitcoinCapabilityError::Provider { diagnostic, .. } = error else {
        panic!("missing configuration must be a provider failure")
    };

    assert_eq!(
        diagnostic.code(),
        ProviderDiagnosticCode::ProviderConfigurationMissing
    );
    assert_eq!(diagnostic.provider_family().as_str(), "bitcoin_core");
}

#[tokio::test(flavor = "current_thread")]
async fn malformed_supplied_config_is_invalid_and_redacted() {
    let (dir, runtime) = runtime_with_config(
        "rpc_url = 'https://alice:password@example.invalid/private'\nauth_header = 'Bearer secret-token'\ninvalid = [",
    );
    let sessions = runtime.evm_read_sessions();
    let binding = evm_binding();
    let error = sessions
        .validate_binding(&binding)
        .await
        .expect_err("malformed configuration");
    let diagnostic = error.redacted_diagnostic().expect("diagnostic");

    assert_eq!(
        diagnostic.code(),
        ProviderDiagnosticCode::ProviderConfigurationInvalid
    );
    let rendered = format!(
        "{}\n{:?}\n{}",
        error,
        error,
        serde_json::to_string(diagnostic).expect("diagnostic JSON")
    );
    for forbidden in [
        "alice",
        "password",
        "example.invalid",
        "Bearer",
        "secret-token",
        dir.path().to_str().expect("UTF-8 temp path"),
    ] {
        assert!(!rendered.contains(forbidden), "leaked {forbidden}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn supplied_config_missing_bitcoin_reports_only_bitcoin_missing() {
    let (_dir, runtime) = runtime_with_config(
        r#"
[evm.routes.test-evm]
source_ref = "primary"
rpc_url = { direct = "http://127.0.0.1:8545" }
"#,
    );
    let binding = btc_binding();
    let error = runtime
        .bitcoin_session()
        .validate_binding(&binding)
        .await
        .expect_err("Bitcoin family is missing");
    let BitcoinCapabilityError::Provider { diagnostic, .. } = error else {
        panic!("missing family must be a provider failure")
    };

    assert_eq!(diagnostic.provider_family().as_str(), "bitcoin_core");
    assert_eq!(
        diagnostic.code(),
        ProviderDiagnosticCode::ProviderConfigurationMissing
    );
}

#[tokio::test]
async fn bitcoin_router_stores_one_checked_session_per_semantic_source() {
    let (_dir, runtime) = runtime_with_config(
        r#"
[bitcoin.routes.primary]
rpc_url = { direct = "http://127.0.0.1:18443" }
scan_timeout_seconds = 30
"#,
    );
    let binding = btc_binding();
    let session = runtime.bitcoin_session();
    session
        .validate_binding(&binding)
        .await
        .expect("configured Bitcoin binding");
    session
        .validate_binding(&binding)
        .await
        .expect("same configured Bitcoin binding");

    let routed = &runtime.routes.bitcoin_sessions;
    assert_eq!(routed.sessions.lock().await.len(), 1);

    let mismatched = BitcoinSourceBinding::new(
        BitcoinNetworkId::new("other-bitcoin").expect("network"),
        BitcoinNetworkTag::Main,
        BitcoinSourceIdentity::new("primary").expect("source"),
    );
    assert_eq!(
        session.validate_binding(&mismatched).await,
        Err(BitcoinCapabilityError::SourceMismatch)
    );
}

#[tokio::test]
async fn failed_bitcoin_route_is_fixed_for_one_dispatch_and_retried_by_the_next() {
    let (dir, runtime) = runtime_with_config(
        r#"
[evm.routes.test-evm]
source_ref = "primary"
rpc_url = { direct = "http://127.0.0.1:8545" }
"#,
    );
    let path = dir.path().join("runtime-secret.toml");
    let binding = btc_binding();
    let session = runtime.bitcoin_session();
    let first = session
        .validate_binding(&binding)
        .await
        .expect_err("Bitcoin route starts missing");

    std::fs::write(
        &path,
        r#"
[bitcoin.routes.primary]
rpc_url = { direct = "http://127.0.0.1:18443" }
scan_timeout_seconds = 45
"#,
    )
    .expect("repair runtime config");
    assert_eq!(
        session.validate_binding(&binding).await,
        Err(first),
        "one dispatch must retain its first route-resolution outcome"
    );

    let next = runtime
        .shared
        .new_dispatch(RuntimeConfigLoader::from_path(Some(&path)));
    next.bitcoin_session()
        .validate_binding(&binding)
        .await
        .expect("the next dispatch re-resolves repaired configuration");
}

#[tokio::test]
async fn a_new_dispatch_rebinds_rewritten_evm_source_and_endpoint() {
    let (url_a, requests_a, server_a) = start_evm_chain_server("0x1").await;
    let (url_b, requests_b, server_b) = start_evm_chain_server("0x1").await;
    let (dir, runtime) = runtime_with_config(&format!(
        r#"
[evm.routes.test-evm]
source_ref = "source-a"
rpc_url = {{ direct = "{url_a}" }}
"#
    ));
    let path = dir.path().join("runtime-secret.toml");
    let binding = evm_binding();
    let sessions = runtime.evm_read_sessions();
    let first = sessions
        .session(&binding)
        .await
        .expect("first dispatch session");
    assert_eq!(first.evidence().source_ref(), "source-a");
    assert_eq!(requests_a.load(Ordering::SeqCst), 1);

    std::fs::write(
        &path,
        format!(
            r#"
[evm.routes.test-evm]
source_ref = "source-b"
rpc_url = {{ direct = "{url_b}" }}
"#
        ),
    )
    .expect("rewrite EVM route");
    let still_first = sessions
        .session(&binding)
        .await
        .expect("same dispatch session");
    assert_eq!(still_first.evidence().source_ref(), "source-a");
    assert_eq!(requests_a.load(Ordering::SeqCst), 1);
    assert_eq!(requests_b.load(Ordering::SeqCst), 0);

    let next = runtime
        .shared
        .new_dispatch(RuntimeConfigLoader::from_path(Some(&path)));
    let rebound = next
        .evm_read_sessions()
        .session(&binding)
        .await
        .expect("next dispatch session");
    assert_eq!(rebound.evidence().source_ref(), "source-b");
    assert_eq!(requests_a.load(Ordering::SeqCst), 1);
    assert_eq!(requests_b.load(Ordering::SeqCst), 1);
    assert!(runtime.shared.evm_transport.get().is_some());

    server_a.abort();
    server_b.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn evm_router_stores_one_checked_session_per_network_and_source() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("listener address");
    let requests = Arc::new(AtomicUsize::new(0));
    let server_requests = Arc::clone(&requests);
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = [0_u8; 4096];
            let read = socket.read(&mut request).await.expect("read request");
            assert!(read > 0, "JSON-RPC request must not be empty");
            server_requests.fetch_add(1, Ordering::SeqCst);
            let body = r#"{"jsonrpc":"2.0","id":1,"result":"0x1"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        }
    });
    let config = format!(
        r#"
[evm.routes.test-evm]
source_ref = "primary"
rpc_url = {{ direct = "http://{address}" }}
"#
    );
    let (_dir, runtime) = runtime_with_config(&config);
    let binding = evm_binding();
    let sessions = runtime.evm_read_sessions();

    assert_eq!(
        sessions.implementation_id(),
        EVM_JSONRPC_SESSION_IMPLEMENTATION_ID
    );
    sessions
        .validate_binding(&binding)
        .await
        .expect("configured EVM binding");
    let session = sessions
        .session(&binding)
        .await
        .expect("cached EVM session");
    let same_session = sessions
        .session(&binding)
        .await
        .expect("same cached EVM session");
    assert!(Arc::ptr_eq(&session, &same_session));
    assert_eq!(session.evidence().source_ref(), "primary");

    let routed = &runtime.routes.evm_sessions;
    assert_eq!(routed.session_count().await, 1);
    assert_eq!(requests.load(Ordering::SeqCst), 1);

    let conflicting = EvmNetworkBinding::new(binding.network_id().clone(), 2)
        .expect("conflicting checked binding");
    assert!(matches!(
        sessions.validate_binding(&conflicting).await,
        Err(EvmCapabilityError::InvalidRequest {
            reason: mfm_evm::EvmInvalidRequest::SessionAuthorityMismatch,
        })
    ));
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    server.abort();
}

#[tokio::test(flavor = "current_thread")]
async fn missing_semantic_evm_route_retains_requested_binding() {
    let (_dir, runtime) = runtime_with_config(
        r#"
[evm.routes.other-evm]
source_ref = "primary"
rpc_url = { direct = "http://127.0.0.1:8545" }
"#,
    );
    let sessions = runtime.evm_read_sessions();
    let binding = evm_binding();
    let error = sessions
        .validate_binding(&binding)
        .await
        .expect_err("semantic route is missing");
    let diagnostic = error.redacted_diagnostic().expect("diagnostic");

    assert_eq!(
        diagnostic.code(),
        ProviderDiagnosticCode::ProviderConfigurationMissing
    );
    assert_eq!(
        serde_json::to_value(diagnostic).expect("diagnostic JSON")["fields"],
        serde_json::json!({"expected_chain_id": 1, "network_id": "test-evm"})
    );
}

#[test]
fn transport_source_mismatch_stays_repairable_typed_authority() {
    let binding = evm_binding();
    let mismatch = mfm_evm::source_mismatch_error(
        &binding,
        alloy_primitives::U256::from(2),
        &LocalPublicId::new("wrong-route").expect("source"),
    );
    let EvmCapabilityError::SourceMismatch { diagnostic } = mismatch else {
        panic!("source mismatch variant");
    };
    let error = evm_transport_capability_error(
        &binding,
        mfm_evm_live::transport::EvmTransportError::SourceMismatch { diagnostic },
    );

    assert!(matches!(error, EvmCapabilityError::SourceMismatch { .. }));
    assert_eq!(
        error.failure_disposition(mfm_evm::EvmCapabilityPhase::ReadOnly),
        mfm_evm::EvmCapabilityFailureDisposition::OperationalBlock
    );
}
