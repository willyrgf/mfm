use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use mfm_bitcoin::{
    BitcoinBalanceCollectionRequest, BitcoinBalanceCollectionResponse, BitcoinBalanceSession,
    BitcoinCapabilityError, BitcoinSessionFuture, BitcoinSourceBinding, BitcoinSourceIdentity,
    BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID,
};
use mfm_bitcoin_live::transport::{BitcoinRpcAuthentication, BitcoinRpcSession};
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

use crate::MFM_RUNTIME_CONFIG_FILE;

#[derive(Clone)]
pub(crate) struct RuntimeConfigLoader {
    path: Option<PathBuf>,
}

impl RuntimeConfigLoader {
    pub(crate) fn from_path_or_env(path: Option<&Path>) -> Self {
        let path = path
            .map(Path::to_path_buf)
            .or_else(|| env::var_os(MFM_RUNTIME_CONFIG_FILE).map(PathBuf::from));
        Self { path }
    }

    fn load_optional_blocking(
        &self,
    ) -> mfm_runtime_config::Result<Option<mfm_runtime_config::RuntimeConfig>> {
        let Some(path) = self.path.as_ref() else {
            return Ok(None);
        };
        mfm_runtime_config::RuntimeConfig::load_path(path).map(Some)
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

struct ResolvedBitcoinRoute {
    rpc_url: String,
    rpc_user: Option<String>,
    rpc_password: Option<String>,
    scan_timeout: Duration,
}

struct RoutedBitcoinSessions {
    routes: Result<
        Option<BTreeMap<BitcoinSourceIdentity, ResolvedBitcoinRoute>>,
        ProviderDiagnosticCode,
    >,
    sessions: Mutex<BTreeMap<BitcoinSourceIdentity, Arc<BitcoinRpcSession>>>,
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

    fn load_bitcoin_routes_optional(
        &self,
    ) -> Result<Option<BTreeMap<BitcoinSourceIdentity, ResolvedBitcoinRoute>>, ProviderDiagnosticCode>
    {
        self.runtime_config
            .load_optional_blocking()
            .map_err(|_| ProviderDiagnosticCode::ProviderConfigurationInvalid)?
            .and_then(|config| config.btc().cloned())
            .map(resolved_bitcoin_routes)
            .transpose()
    }

    pub(crate) fn bitcoin_session(&self) -> Arc<dyn BitcoinBalanceSession> {
        let session = self.bitcoin_sessions.get_or_init(|| {
            Arc::new(RoutedBitcoinSessions {
                routes: self.load_bitcoin_routes_optional(),
                sessions: Mutex::new(BTreeMap::new()),
            })
        });
        Arc::clone(session) as Arc<dyn BitcoinBalanceSession>
    }
}

impl RoutedEvmReadSessions {
    fn load_route_blocking(
        runtime_config: &RuntimeConfigLoader,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm::EvmCapabilityResult<mfm_runtime_config::EvmRpcRoute> {
        let Some(path) = runtime_config.path.as_ref() else {
            return Err(evm_provider_failure(
                binding,
                ProviderDiagnosticCode::ProviderConfigurationMissing,
            ));
        };
        mfm_runtime_config::RuntimeConfig::load_evm_route(path, binding.network_id()).map_err(
            |error| {
                let code = match error.kind() {
                    mfm_runtime_config::RuntimeConfigErrorKind::MissingFamily
                    | mfm_runtime_config::RuntimeConfigErrorKind::MissingRoute => {
                        ProviderDiagnosticCode::ProviderConfigurationMissing
                    }
                    _ => ProviderDiagnosticCode::ProviderConfigurationInvalid,
                };
                evm_provider_failure(binding, code)
            },
        )
    }

    async fn load_route(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm::EvmCapabilityResult<mfm_runtime_config::EvmRpcRoute> {
        let runtime_config = self.runtime_config.clone();
        let binding = binding.clone();
        let diagnostic_binding = binding.clone();
        load_runtime_config_on_blocking_worker(move || {
            Self::load_route_blocking(&runtime_config, &binding)
        })
        .await
        .map_err(|_| {
            evm_provider_failure(
                &diagnostic_binding,
                ProviderDiagnosticCode::ProviderConfigurationInvalid,
            )
        })?
    }

    async fn session_for(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm::EvmCapabilityResult<Arc<EvmJsonRpcSession>> {
        let route = self.load_route(binding).await?;
        let key = (binding.network_id().clone(), route.source_ref().clone());
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get(&key) {
            return validate_evm_session(session, binding, route.source_ref())
                .map(|()| Arc::clone(session));
        }

        let transport = self
            .transport
            .clone()
            .map_err(|error| evm_transport_capability_error(binding, error))?;
        let endpoint = EvmRpcEndpoint::new(route.rpc_url().expose_secret())
            .map_err(|error| evm_transport_capability_error(binding, error))?;
        let authorization = route
            .auth_header()
            .map(|value| EvmRpcAuthorization::new(value.expose_secret()))
            .transpose()
            .map_err(|error| evm_transport_capability_error(binding, error))?;
        let session = Arc::new(
            transport
                .bind(
                    binding.clone(),
                    route.source_ref().clone(),
                    endpoint,
                    authorization,
                )
                .await
                .map_err(|error| evm_transport_capability_error(binding, error))?,
        );
        validate_evm_session(&session, binding, route.source_ref())?;
        sessions.insert(key, Arc::clone(&session));
        Ok(session)
    }

    #[cfg(test)]
    async fn session_count(&self) -> usize {
        self.sessions.lock().await.len()
    }
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
    fn route(
        &self,
        binding: &BitcoinSourceBinding,
    ) -> Result<&ResolvedBitcoinRoute, BitcoinCapabilityError> {
        let routes = self
            .routes
            .as_ref()
            .map_err(|code| bitcoin_provider_failure(*code, false))?
            .as_ref()
            .ok_or_else(|| {
                bitcoin_provider_failure(
                    ProviderDiagnosticCode::ProviderConfigurationMissing,
                    false,
                )
            })?;
        routes
            .get(binding.semantic_source_identity())
            .ok_or_else(|| {
                bitcoin_provider_failure(ProviderDiagnosticCode::RouteUnavailable, false)
            })
    }

    fn session_for(
        &self,
        binding: &BitcoinSourceBinding,
    ) -> Result<Arc<BitcoinRpcSession>, BitcoinCapabilityError> {
        let source_identity = binding.semantic_source_identity();
        if let Some(session) = self
            .sessions
            .lock()
            .map_err(|_| {
                bitcoin_provider_failure(
                    ProviderDiagnosticCode::ProviderConfigurationInvalid,
                    false,
                )
            })?
            .get(source_identity)
            .cloned()
        {
            return (session.binding() == binding)
                .then_some(session)
                .ok_or(BitcoinCapabilityError::SourceMismatch);
        }

        let route = self.route(binding)?;
        let authentication = match (&route.rpc_user, &route.rpc_password) {
            (Some(username), Some(password)) => Some(
                BitcoinRpcAuthentication::new(username.clone(), password.clone()).map_err(
                    |_| {
                        bitcoin_provider_failure(
                            ProviderDiagnosticCode::ProviderConfigurationInvalid,
                            false,
                        )
                    },
                )?,
            ),
            (None, None) => None,
            _ => {
                return Err(bitcoin_provider_failure(
                    ProviderDiagnosticCode::ProviderConfigurationInvalid,
                    false,
                ))
            }
        };
        let candidate = Arc::new(
            BitcoinRpcSession::new(
                &route.rpc_url,
                authentication,
                binding.clone(),
                route.scan_timeout,
            )
            .map_err(|_| {
                bitcoin_provider_failure(
                    ProviderDiagnosticCode::ProviderConfigurationInvalid,
                    false,
                )
            })?,
        );

        let mut sessions = self.sessions.lock().map_err(|_| {
            bitcoin_provider_failure(ProviderDiagnosticCode::ProviderConfigurationInvalid, false)
        })?;
        if let Some(session) = sessions.get(source_identity) {
            return (session.binding() == binding)
                .then(|| Arc::clone(session))
                .ok_or(BitcoinCapabilityError::SourceMismatch);
        }
        sessions.insert(source_identity.clone(), Arc::clone(&candidate));
        Ok(candidate)
    }
}

impl BitcoinBalanceSession for RoutedBitcoinSessions {
    fn implementation_id(&self) -> &'static str {
        BITCOIN_JSONRPC_BALANCE_COLLECTION_IMPLEMENTATION_ID
    }

    fn validate_binding(
        &self,
        binding: &BitcoinSourceBinding,
    ) -> Result<(), BitcoinCapabilityError> {
        self.session_for(binding).map(|_| ())
    }

    fn collect_balances<'a>(
        &'a self,
        request: &'a BitcoinBalanceCollectionRequest,
    ) -> BitcoinSessionFuture<'a, BitcoinBalanceCollectionResponse> {
        Box::pin(async move {
            let session = self.session_for(request.binding())?;
            session.collect_balances(request).await
        })
    }
}

fn resolved_bitcoin_routes(
    btc: mfm_runtime_config::BtcRuntimeConfig,
) -> Result<BTreeMap<BitcoinSourceIdentity, ResolvedBitcoinRoute>, ProviderDiagnosticCode> {
    let mut routes = BTreeMap::new();
    for (source_identity, route) in btc.routes() {
        routes.insert(
            source_identity.clone(),
            ResolvedBitcoinRoute {
                rpc_url: route.rpc_url().expose_secret().to_owned(),
                rpc_user: route
                    .rpc_user()
                    .map(|value| value.expose_secret().to_owned()),
                rpc_password: route
                    .rpc_password()
                    .map(|value| value.expose_secret().to_owned()),
                scan_timeout: Duration::from_secs(route.scan_timeout_seconds()),
            },
        );
    }
    Ok(routes)
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
        let runtime = LiveTransportRuntime::new(RuntimeConfigLoader::from_path_or_env(Some(&path)));
        (dir, runtime)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn missing_config_preserves_evm_semantic_binding() {
        let runtime = LiveTransportRuntime::new(RuntimeConfigLoader::from_path_or_env(None));
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
rpc_url = "http://127.0.0.1:8545"
"#,
        )
        .expect("write runtime config");
        let binding = evm_binding();
        let async_worker = std::thread::current().id();

        let file_worker = load_runtime_config_on_blocking_worker(move || {
            mfm_runtime_config::RuntimeConfig::load_evm_route(&path, binding.network_id())
                .expect("selective EVM route");
            std::thread::current().id()
        })
        .await
        .expect("blocking route-load worker");

        assert_ne!(file_worker, async_worker);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn missing_config_reports_bitcoin_route_failure() {
        let runtime = LiveTransportRuntime::new(RuntimeConfigLoader::from_path_or_env(None));
        let binding = btc_binding();
        let error = runtime
            .bitcoin_session()
            .validate_binding(&binding)
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
rpc_url = "http://127.0.0.1:8545"
"#,
        );
        let binding = btc_binding();
        let error = runtime
            .bitcoin_session()
            .validate_binding(&binding)
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

    #[test]
    fn bitcoin_router_stores_one_checked_session_per_semantic_source() {
        let (_dir, runtime) = runtime_with_config(
            r#"
[btc.routes.primary]
rpc_url = "http://127.0.0.1:18443"
scan_timeout_seconds = 30
"#,
        );
        let binding = btc_binding();
        let session = runtime.bitcoin_session();
        session
            .validate_binding(&binding)
            .expect("configured Bitcoin binding");
        session
            .validate_binding(&binding)
            .expect("same configured Bitcoin binding");

        let routed = runtime
            .bitcoin_sessions
            .get()
            .expect("initialized routed session set");
        assert_eq!(routed.sessions.lock().expect("sessions").len(), 1);

        let mismatched = BitcoinSourceBinding::new(
            BitcoinNetworkId::new("other-bitcoin").expect("network"),
            BitcoinNetworkTag::Main,
            BitcoinSourceIdentity::new("primary").expect("source"),
        );
        assert_eq!(
            session.validate_binding(&mismatched),
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
rpc_url = "http://{address}"
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
rpc_url = "http://127.0.0.1:8545"
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
