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
    #[cfg(any(test, feature = "test-support"))]
    panic_on_access: bool,
}

impl RuntimeConfigLoader {
    pub(crate) fn from_path(path: Option<&Path>) -> Self {
        Self {
            path: path.map(Path::to_path_buf),
            #[cfg(any(test, feature = "test-support"))]
            panic_on_access: false,
        }
    }

    fn path_for_live_access(&self) -> Option<PathBuf> {
        #[cfg(any(test, feature = "test-support"))]
        assert!(
            !self.panic_on_access,
            "evidence-only work accessed live runtime configuration"
        );
        self.path.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn panicking_for_test() -> Self {
        Self {
            path: None,
            panic_on_access: true,
        }
    }
}

pub(crate) struct SharedLiveTransports {
    evm_transport: OnceLock<TransportResult<EvmJsonRpcTransport>>,
    #[cfg(test)]
    dispatch_count: std::sync::atomic::AtomicUsize,
}

pub(crate) struct LiveDispatchRoutes {
    evm_sessions: Arc<RoutedEvmReadSessions>,
    bitcoin_sessions: Arc<RoutedBitcoinSessions>,
}

struct RoutedEvmReadSessions {
    runtime_config: RuntimeConfigLoader,
    shared_transports: Arc<SharedLiveTransports>,
    cache: AsyncMutex<EvmRouteCache>,
}

struct RoutedBitcoinSessions {
    runtime_config: RuntimeConfigLoader,
    sessions: AsyncMutex<BTreeMap<BitcoinSourceIdentity, CachedBitcoinSession>>,
}

#[derive(Default)]
struct EvmRouteCache {
    sessions: BTreeMap<(LocalPublicId, LocalPublicId), CachedEvmSession>,
    failures: BTreeMap<LocalPublicId, CachedEvmFailure>,
}

struct CachedEvmSession {
    binding: EvmNetworkBinding,
    session: Arc<EvmJsonRpcSession>,
}

struct CachedEvmFailure {
    binding: EvmNetworkBinding,
    error: EvmCapabilityError,
}

struct CachedBitcoinSession {
    binding: BitcoinSourceBinding,
    outcome: Result<Arc<BitcoinRpcSession>, BitcoinCapabilityError>,
}

impl SharedLiveTransports {
    pub(crate) fn new() -> Self {
        Self {
            evm_transport: OnceLock::new(),
            #[cfg(test)]
            dispatch_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn evm_transport(&self) -> TransportResult<EvmJsonRpcTransport> {
        self.evm_transport
            .get_or_init(EvmJsonRpcTransport::new)
            .clone()
    }

    pub(crate) fn new_dispatch(
        self: &Arc<Self>,
        runtime_config: RuntimeConfigLoader,
    ) -> LiveDispatchRoutes {
        #[cfg(test)]
        self.dispatch_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        LiveDispatchRoutes {
            evm_sessions: Arc::new(RoutedEvmReadSessions {
                runtime_config: runtime_config.clone(),
                shared_transports: Arc::clone(self),
                cache: AsyncMutex::new(EvmRouteCache::default()),
            }),
            bitcoin_sessions: Arc::new(RoutedBitcoinSessions {
                runtime_config,
                sessions: AsyncMutex::new(BTreeMap::new()),
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn dispatch_count(&self) -> usize {
        self.dispatch_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl LiveDispatchRoutes {
    pub(crate) fn evm_read_sessions(&self) -> Arc<dyn EvmReadSessionSet> {
        Arc::clone(&self.evm_sessions) as Arc<dyn EvmReadSessionSet>
    }

    pub(crate) fn bitcoin_session(&self) -> Arc<dyn BitcoinBalanceSession> {
        Arc::clone(&self.bitcoin_sessions) as Arc<dyn BitcoinBalanceSession>
    }
}

impl RoutedEvmReadSessions {
    async fn load_route(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm::EvmCapabilityResult<PreparedEvmRoute> {
        let Some(path) = self.runtime_config.path_for_live_access() else {
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
        let mut cache = self.cache.lock().await;
        if let Some(((network_id, source_ref), cached)) = cache
            .sessions
            .iter()
            .find(|((network_id, _), _)| network_id == binding.network_id())
        {
            debug_assert_eq!(network_id, binding.network_id());
            if &cached.binding != binding {
                return Err(evm_session_authority_mismatch());
            }
            return validate_evm_session(&cached.session, binding, source_ref)
                .map(|()| Arc::clone(&cached.session));
        }
        if let Some(cached) = cache.failures.get(binding.network_id()) {
            return if &cached.binding == binding {
                Err(cached.error.clone())
            } else {
                Err(evm_session_authority_mismatch())
            };
        }

        match self.resolve_session(binding).await {
            Ok((source_ref, session)) => {
                let key = (binding.network_id().clone(), source_ref);
                cache.sessions.insert(
                    key,
                    CachedEvmSession {
                        binding: binding.clone(),
                        session: Arc::clone(&session),
                    },
                );
                Ok(session)
            }
            Err(error) => {
                cache.failures.insert(
                    binding.network_id().clone(),
                    CachedEvmFailure {
                        binding: binding.clone(),
                        error: error.clone(),
                    },
                );
                Err(error)
            }
        }
    }

    async fn resolve_session(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm::EvmCapabilityResult<(LocalPublicId, Arc<EvmJsonRpcSession>)> {
        let route = self.load_route(binding).await?;
        let transport = self
            .shared_transports
            .evm_transport()
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
        Ok((route.source_ref, session))
    }

    #[cfg(test)]
    async fn session_count(&self) -> usize {
        self.cache.lock().await.sessions.len()
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

fn evm_session_authority_mismatch() -> EvmCapabilityError {
    EvmCapabilityError::InvalidRequest {
        reason: mfm_evm::EvmInvalidRequest::SessionAuthorityMismatch,
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
        if let Some(cached) = sessions.get(source_identity) {
            if &cached.binding != binding {
                return Err(BitcoinCapabilityError::SourceMismatch);
            }
            return cached.outcome.clone();
        }

        let outcome = self.resolve_session(binding).await;
        sessions.insert(
            source_identity.clone(),
            CachedBitcoinSession {
                binding: binding.clone(),
                outcome: outcome.clone(),
            },
        );
        outcome
    }

    async fn resolve_session(
        &self,
        binding: &BitcoinSourceBinding,
    ) -> Result<Arc<BitcoinRpcSession>, BitcoinCapabilityError> {
        let Some(path) = self.runtime_config.path_for_live_access() else {
            return Err(bitcoin_provider_failure(
                ProviderDiagnosticCode::ProviderConfigurationMissing,
                false,
            ));
        };
        let selected_source = binding.semantic_source_identity().clone();
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
#[path = "live_transports_tests.rs"]
mod tests;
