use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use mfm_bitcoin::{
    BitcoinBalanceCollectionRequest, BitcoinBalanceCollectionResponse, BitcoinBalanceSession,
    BitcoinCapabilityError, BitcoinSessionFuture, BitcoinSourceBinding, BitcoinSourceIdentity,
    BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID,
};
use mfm_bitcoin_live::transport::{
    BitcoinRpcAuthentication, BitcoinRpcEndpoint, BitcoinRpcSession,
};
use mfm_capabilities::{
    ProviderDiagnosticCode, ProviderDiagnosticValue, RedactedProviderDiagnostic,
};
use mfm_evm::{
    EvmCapabilityError, EvmNetworkBinding, EvmReadSession, EvmReadSessionSet, EvmSessionFuture,
    EVM_JSONRPC_SESSION_IMPLEMENTATION_ID,
};
use mfm_evm_live::transport::{
    EvmJsonRpcSession, EvmJsonRpcTransport, EvmRpcAuthorization, EvmRpcEndpoint, EvmTransportError,
    TransportResult,
};
use mfm_ids::LocalPublicId;
use tokio::sync::Mutex as AsyncMutex;

use crate::runtime_config;

#[derive(Clone)]
pub(crate) struct RuntimeConfigLoader {
    path: Option<PathBuf>,
}

impl RuntimeConfigLoader {
    pub(crate) fn from_path(path: Option<&Path>) -> Self {
        Self {
            path: path.map(Path::to_path_buf),
        }
    }
}

pub(crate) struct LiveTransportRuntime {
    runtime_config: RuntimeConfigLoader,
    evm_sessions: OnceLock<Arc<RoutedEvmReadSessions>>,
    bitcoin_sessions: OnceLock<Arc<RoutedBitcoinSessions>>,
}

struct RoutedEvmReadSessions {
    runtime_config: RuntimeConfigLoader,
    transport: TransportResult<EvmJsonRpcTransport>,
    sessions: AsyncMutex<BTreeMap<(LocalPublicId, LocalPublicId), Arc<EvmJsonRpcSession>>>,
}

struct RoutedBitcoinSessions {
    runtime_config: RuntimeConfigLoader,
    sessions: AsyncMutex<BTreeMap<BitcoinSourceIdentity, Arc<BitcoinRpcSession>>>,
}

impl LiveTransportRuntime {
    pub(crate) fn new(runtime_config: RuntimeConfigLoader) -> Self {
        Self {
            runtime_config,
            evm_sessions: OnceLock::new(),
            bitcoin_sessions: OnceLock::new(),
        }
    }

    pub(crate) fn evm_read_sessions(&self) -> Arc<dyn EvmReadSessionSet> {
        let sessions = self.evm_sessions.get_or_init(|| {
            Arc::new(RoutedEvmReadSessions {
                runtime_config: self.runtime_config.clone(),
                transport: EvmJsonRpcTransport::new(),
                sessions: AsyncMutex::new(BTreeMap::new()),
            })
        });
        Arc::clone(sessions) as Arc<dyn EvmReadSessionSet>
    }

    pub(crate) fn bitcoin_session(&self) -> Arc<dyn BitcoinBalanceSession> {
        let session = self.bitcoin_sessions.get_or_init(|| {
            Arc::new(RoutedBitcoinSessions {
                runtime_config: self.runtime_config.clone(),
                sessions: AsyncMutex::new(BTreeMap::new()),
            })
        });
        Arc::clone(session) as Arc<dyn BitcoinBalanceSession>
    }
}

impl RoutedEvmReadSessions {
    async fn load_route(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm::EvmCapabilityResult<PreparedEvmRoute> {
        let Some(path) = self.runtime_config.path.clone() else {
            return Err(evm_provider_failure(
                binding,
                ProviderDiagnosticCode::ProviderConfigurationMissing,
            ));
        };
        let network_id = binding.network_id().clone();
        let diagnostic_binding = binding.clone();
        load_runtime_config_on_blocking_worker(move || {
            let route = runtime_config::load_evm_route(&path, &network_id)
                .map_err(config_diagnostic_code)?;
            let (source_ref, rpc_url, auth_header) = route.into_parts();
            let endpoint = EvmRpcEndpoint::new(rpc_url.into_string())
                .map_err(|_| ProviderDiagnosticCode::ProviderConfigurationInvalid)?;
            let authorization = auth_header
                .map(|value| EvmRpcAuthorization::new(value.into_protected()))
                .transpose()
                .map_err(|_| ProviderDiagnosticCode::ProviderConfigurationInvalid)?;
            Ok(PreparedEvmRoute {
                source_ref,
                endpoint,
                authorization,
            })
        })
        .await
        .map_err(|_| {
            evm_provider_failure(
                &diagnostic_binding,
                ProviderDiagnosticCode::ProviderConfigurationInvalid,
            )
        })?
        .map_err(|code| evm_provider_failure(&diagnostic_binding, code))
    }

    async fn session_for(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm::EvmCapabilityResult<Arc<EvmJsonRpcSession>> {
        let mut sessions = self.sessions.lock().await;
        if let Some(((network_id, source_ref), session)) = sessions
            .iter()
            .find(|((network_id, _), _)| network_id == binding.network_id())
        {
            debug_assert_eq!(network_id, binding.network_id());
            return validate_evm_session(session, binding, source_ref)
                .map(|()| Arc::clone(session));
        }

        let route = self.load_route(binding).await?;
        let key = (binding.network_id().clone(), route.source_ref.clone());

        let transport = self
            .transport
            .clone()
            .map_err(|error| evm_transport_capability_error(binding, error))?;
        let session = Arc::new(
            transport
                .bind(
                    binding.clone(),
                    route.source_ref.clone(),
                    route.endpoint,
                    route.authorization,
                )
                .await
                .map_err(|error| evm_transport_capability_error(binding, error))?,
        );
        validate_evm_session(&session, binding, &route.source_ref)?;
        sessions.insert(key, Arc::clone(&session));
        Ok(session)
    }

    #[cfg(test)]
    async fn session_count(&self) -> usize {
        self.sessions.lock().await.len()
    }
}

struct PreparedEvmRoute {
    source_ref: LocalPublicId,
    endpoint: EvmRpcEndpoint,
    authorization: Option<EvmRpcAuthorization>,
}

impl EvmReadSessionSet for RoutedEvmReadSessions {
    fn implementation_id(&self) -> &str {
        EVM_JSONRPC_SESSION_IMPLEMENTATION_ID
    }

    fn validate_binding<'a>(&'a self, binding: &'a EvmNetworkBinding) -> EvmSessionFuture<'a, ()> {
        Box::pin(async move { self.session_for(binding).await.map(|_| ()) })
    }

    fn session<'a>(
        &'a self,
        binding: &'a EvmNetworkBinding,
    ) -> EvmSessionFuture<'a, Arc<dyn EvmReadSession>> {
        Box::pin(async move {
            self.session_for(binding)
                .await
                .map(|session| session as Arc<dyn EvmReadSession>)
        })
    }
}

fn validate_evm_session(
    session: &EvmJsonRpcSession,
    binding: &EvmNetworkBinding,
    source_ref: &LocalPublicId,
) -> mfm_evm::EvmCapabilityResult<()> {
    let evidence = EvmReadSession::evidence(session);
    if evidence.matches_binding(binding)
        && evidence.source_ref() == source_ref.as_str()
        && evidence.implementation_id() == EVM_JSONRPC_SESSION_IMPLEMENTATION_ID
    {
        Ok(())
    } else {
        Err(mfm_evm::EvmCapabilityError::InvalidRequest {
            reason: mfm_evm::EvmInvalidRequest::SessionAuthorityMismatch,
        })
    }
}

async fn load_runtime_config_on_blocking_worker<T, F>(load: F) -> Result<T, tokio::task::JoinError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tokio::task::spawn_blocking(load).await
}

impl RoutedBitcoinSessions {
    async fn session_for(
        &self,
        binding: &BitcoinSourceBinding,
    ) -> Result<Arc<BitcoinRpcSession>, BitcoinCapabilityError> {
        let source_identity = binding.semantic_source_identity();
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get(source_identity).cloned() {
            return (session.binding() == binding)
                .then_some(session)
                .ok_or(BitcoinCapabilityError::SourceMismatch);
        }

        let Some(path) = self.runtime_config.path.clone() else {
            return Err(bitcoin_provider_failure(
                ProviderDiagnosticCode::ProviderConfigurationMissing,
                false,
            ));
        };
        let selected_source = source_identity.clone();
        let selected_binding = binding.clone();
        let candidate = load_runtime_config_on_blocking_worker(move || {
            let route = runtime_config::load_bitcoin_route(&path, &selected_source)
                .map_err(config_diagnostic_code)?;
            let (rpc_url, rpc_user, rpc_password, scan_timeout_seconds) = route.into_parts();
            let endpoint = BitcoinRpcEndpoint::new(rpc_url.into_string())
                .map_err(|_| ProviderDiagnosticCode::ProviderConfigurationInvalid)?;
            let authentication = match (rpc_user, rpc_password) {
                (Some(username), Some(password)) => Some(
                    BitcoinRpcAuthentication::new(
                        username.into_string(),
                        password.into_protected(),
                    )
                    .map_err(|_| ProviderDiagnosticCode::ProviderConfigurationInvalid)?,
                ),
                (None, None) => None,
                _ => return Err(ProviderDiagnosticCode::ProviderConfigurationInvalid),
            };
            BitcoinRpcSession::new(
                endpoint,
                authentication,
                selected_binding,
                Duration::from_secs(scan_timeout_seconds),
            )
            .map(Arc::new)
            .map_err(|_| ProviderDiagnosticCode::ProviderConfigurationInvalid)
        })
        .await
        .map_err(|_| {
            bitcoin_provider_failure(ProviderDiagnosticCode::ProviderConfigurationInvalid, false)
        })?
        .map_err(|code| bitcoin_provider_failure(code, false))?;
        sessions.insert(source_identity.clone(), Arc::clone(&candidate));
        Ok(candidate)
    }
}

impl BitcoinBalanceSession for RoutedBitcoinSessions {
    fn implementation_id(&self) -> &'static str {
        BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID
    }

    fn validate_binding<'a>(
        &'a self,
        binding: &'a BitcoinSourceBinding,
    ) -> BitcoinSessionFuture<'a, ()> {
        Box::pin(async move { self.session_for(binding).await.map(|_| ()) })
    }

    fn collect_balances<'a>(
        &'a self,
        request: &'a BitcoinBalanceCollectionRequest,
    ) -> BitcoinSessionFuture<'a, BitcoinBalanceCollectionResponse> {
        Box::pin(async move {
            let session = self.session_for(request.binding()).await?;
            session.collect_balances(request).await
        })
    }
}

fn config_diagnostic_code(error: runtime_config::RuntimeConfigError) -> ProviderDiagnosticCode {
    if error.is_missing_selection() {
        ProviderDiagnosticCode::ProviderConfigurationMissing
    } else {
        ProviderDiagnosticCode::ProviderConfigurationInvalid
    }
}

fn diagnostic_id(value: &str) -> LocalPublicId {
    LocalPublicId::new(value).expect("semantic provider binding is checked public text")
}

fn evm_provider_failure(
    binding: &EvmNetworkBinding,
    code: ProviderDiagnosticCode,
) -> EvmCapabilityError {
    EvmCapabilityError::provider_failure(enrich_evm_diagnostic(
        binding,
        mfm_evm::evm_diagnostic(code),
    ))
}

fn evm_transport_capability_error(
    binding: &EvmNetworkBinding,
    error: EvmTransportError,
) -> EvmCapabilityError {
    match error {
        EvmTransportError::SourceMismatch { diagnostic } => EvmCapabilityError::SourceMismatch {
            diagnostic: enrich_evm_diagnostic(binding, diagnostic),
        },
        error => EvmCapabilityError::provider_failure(enrich_evm_diagnostic(
            binding,
            error.into_provider_diagnostic(),
        )),
    }
}

fn enrich_evm_diagnostic(
    binding: &EvmNetworkBinding,
    diagnostic: RedactedProviderDiagnostic,
) -> RedactedProviderDiagnostic {
    diagnostic
        .with_field(
            diagnostic_id("network_id"),
            ProviderDiagnosticValue::Id(diagnostic_id(binding.network_id().as_str())),
        )
        .with_field(
            diagnostic_id("expected_chain_id"),
            ProviderDiagnosticValue::U64(binding.expected_chain_id()),
        )
}

fn bitcoin_provider_failure(
    code: ProviderDiagnosticCode,
    retryable: bool,
) -> BitcoinCapabilityError {
    BitcoinCapabilityError::provider(code, "route_selection", retryable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_bitcoin::{BitcoinNetworkId, BitcoinNetworkTag, BitcoinSourceIdentity};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn evm_binding() -> EvmNetworkBinding {
        EvmNetworkBinding::new(LocalPublicId::new("test-evm").expect("network"), 1)
            .expect("binding")
    }

    fn btc_binding() -> BitcoinSourceBinding {
        BitcoinSourceBinding::new(
            BitcoinNetworkId::new("test-btc").expect("network"),
            BitcoinNetworkTag::Test,
            BitcoinSourceIdentity::new("primary").expect("source"),
        )
    }

    fn runtime_with_config(raw: &str) -> (tempfile::TempDir, LiveTransportRuntime) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("runtime-secret.toml");
        std::fs::write(&path, raw).expect("write runtime config");
        let runtime = LiveTransportRuntime::new(RuntimeConfigLoader::from_path(Some(&path)));
        (dir, runtime)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn missing_config_preserves_evm_semantic_binding() {
        let runtime = LiveTransportRuntime::new(RuntimeConfigLoader::from_path(None));
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
            runtime_config::load_evm_route(&path, binding.network_id())
                .expect("selective EVM route");
            std::thread::current().id()
        })
        .await
        .expect("blocking route-load worker");

        assert_ne!(file_worker, async_worker);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn missing_config_reports_bitcoin_route_failure() {
        let runtime = LiveTransportRuntime::new(RuntimeConfigLoader::from_path(None));
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

        let routed = runtime
            .bitcoin_sessions
            .get()
            .expect("initialized routed session set");
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
        assert_eq!(session.evidence().source_ref(), "primary");

        let routed = runtime
            .evm_sessions
            .get()
            .expect("initialized routed EVM session set");
        assert_eq!(routed.session_count().await, 1);
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
}
