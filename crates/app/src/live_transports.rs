use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use mfm_btc_capabilities::{BtcBalanceReadProvider, BtcChainHeadReadProvider, BtcSourceBinding};
use mfm_capabilities::{
    ProviderDiagnosticCode, ProviderDiagnosticValue, RedactedProviderDiagnostic,
};
use mfm_evm_capabilities::{EvmCapabilityError, EvmNetworkBinding};
use mfm_ids::LocalPublicId;
use mfm_signing::{DeterministicSigningProvider, SignerRef, SigningError};

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
    evm_transport:
        OnceLock<mfm_transports_evm::TransportResult<mfm_transports_evm::EvmJsonRpcTransport>>,
    btc_router: OnceLock<
        Result<
            Option<Arc<mfm_transports_btc_jsonrpc_http::BtcJsonRpcRouter>>,
            ProviderDiagnosticCode,
        >,
    >,
}

impl LiveTransportRuntime {
    pub(crate) fn new(runtime_config: RuntimeConfigLoader) -> Self {
        Self {
            runtime_config,
            evm_transport: OnceLock::new(),
            btc_router: OnceLock::new(),
        }
    }

    pub(crate) async fn validate_evm_read_route(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<()> {
        self.load_evm_route_async(binding).await.map(|_| ())
    }

    pub(crate) async fn bind_evm_read_session(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<Arc<dyn mfm_evm_capabilities::EvmReadSession>> {
        let route = self.load_evm_route_async(binding.clone()).await?;
        self.evm_transport(&binding)?
            .bind(
                binding.clone(),
                route.source_ref().clone(),
                route.rpc_url().expose_secret().to_owned(),
                route
                    .auth_header()
                    .map(|value| value.expose_secret().to_owned()),
            )
            .await
            .map(|session| Arc::new(session) as Arc<dyn mfm_evm_capabilities::EvmReadSession>)
            .map_err(|error| evm_transport_capability_error(&binding, error))
    }

    pub(crate) async fn bind_evm_transaction_session(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<Arc<dyn mfm_evm_capabilities::EvmTransactionSession>> {
        let route = self.load_evm_route_async(binding.clone()).await?;
        self.evm_transport(&binding)?
            .bind(
                binding.clone(),
                route.source_ref().clone(),
                route.rpc_url().expose_secret().to_owned(),
                route
                    .auth_header()
                    .map(|value| value.expose_secret().to_owned()),
            )
            .await
            .map(|session| {
                Arc::new(session) as Arc<dyn mfm_evm_capabilities::EvmTransactionSession>
            })
            .map_err(|error| evm_transport_capability_error(&binding, error))
    }

    pub(crate) async fn validate_evm_mutation_binding(
        &self,
        binding: EvmNetworkBinding,
        signer_ref: SignerRef,
    ) -> mfm_runtime::Result<()> {
        let runtime_config = self.runtime_config.clone();
        tokio::task::spawn_blocking(move || {
            let runtime = Self::new(runtime_config);
            runtime
                .load_evm_route(&binding)
                .map(|_| ())
                .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))?;
            runtime
                .assemble_evm_signer(signer_ref)
                .map(|_| ())
                .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))
        })
        .await
        .map_err(|_| {
            mfm_runtime::RuntimeError::RunnerBinding(
                "EVM mutation ingress validation task failed".to_owned(),
            )
        })?
    }

    pub(crate) async fn bind_evm_signer(
        &self,
        signer_ref: SignerRef,
    ) -> mfm_signing::Result<Arc<dyn DeterministicSigningProvider>> {
        let path = self.runtime_config.path.clone().ok_or_else(|| {
            SigningError::redacted_provider_failure("runtime signer config is absent")
        })?;
        tokio::task::spawn_blocking(move || {
            crate::transaction_signing::assemble_keystore_signer(path, signer_ref)
                .map(|provider| Arc::new(provider) as Arc<dyn DeterministicSigningProvider>)
                .map_err(SigningError::redacted_provider_failure)
        })
        .await
        .map_err(SigningError::redacted_provider_failure)?
    }

    fn assemble_evm_signer(
        &self,
        signer_ref: SignerRef,
    ) -> mfm_signing::Result<mfm_signers_keystore::KeystoreSignerProvider> {
        let path = self.runtime_config.path.clone().ok_or_else(|| {
            SigningError::redacted_provider_failure("runtime signer config is absent")
        })?;
        crate::transaction_signing::assemble_keystore_signer(path, signer_ref)
            .map_err(SigningError::redacted_provider_failure)
    }

    fn load_evm_route(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<mfm_runtime_config::EvmRpcRoute> {
        let Some(path) = self.runtime_config.path.as_ref() else {
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

    fn evm_transport(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<mfm_transports_evm::EvmJsonRpcTransport> {
        self.evm_transport
            .get_or_init(mfm_transports_evm::EvmJsonRpcTransport::new)
            .clone()
            .map_err(|error| evm_transport_capability_error(binding, error))
    }

    async fn load_evm_route_async(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<mfm_runtime_config::EvmRpcRoute> {
        let runtime_config = self.runtime_config.clone();
        let diagnostic_binding = binding.clone();
        load_runtime_config_on_blocking_worker(move || {
            Self::new(runtime_config).load_evm_route(&binding)
        })
        .await
        .map_err(|_| {
            evm_provider_failure(
                &diagnostic_binding,
                ProviderDiagnosticCode::ProviderConfigurationInvalid,
            )
        })?
    }

    fn load_btc_router_optional(
        &self,
    ) -> Result<
        Option<Arc<mfm_transports_btc_jsonrpc_http::BtcJsonRpcRouter>>,
        ProviderDiagnosticCode,
    > {
        self.runtime_config
            .load_optional_blocking()
            .map_err(|_| ProviderDiagnosticCode::ProviderConfigurationInvalid)?
            .and_then(|config| config.btc().cloned())
            .map(btc_json_rpc_router)
            .transpose()
            .map(|router| router.map(Arc::new))
    }

    fn cached_btc_router(
        &self,
        binding: &BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<Arc<mfm_transports_btc_jsonrpc_http::BtcJsonRpcRouter>> {
        self.btc_router
            .get()
            .ok_or_else(|| {
                btc_provider_failure(
                    binding,
                    ProviderDiagnosticCode::ProviderConfigurationInvalid,
                )
            })?
            .clone()
            .map_err(|code| btc_provider_failure(binding, code))?
            .ok_or_else(|| {
                btc_provider_failure(
                    binding,
                    ProviderDiagnosticCode::ProviderConfigurationMissing,
                )
            })
    }

    async fn validate_btc_source_binding(
        &self,
        binding: BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<()> {
        self.btc_router_async(&binding)
            .await?
            .validate_source_binding(&binding)
            .map_err(|error| enrich_btc_capability_error(&binding, error))
    }

    async fn btc_router_async(
        &self,
        binding: &BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<Arc<mfm_transports_btc_jsonrpc_http::BtcJsonRpcRouter>> {
        if let Some(cached) = self.btc_router.get() {
            return cached
                .clone()
                .map_err(|code| btc_provider_failure(binding, code))?
                .ok_or_else(|| {
                    btc_provider_failure(
                        binding,
                        ProviderDiagnosticCode::ProviderConfigurationMissing,
                    )
                });
        }
        let runtime_config = self.runtime_config.clone();
        let loaded = load_runtime_config_on_blocking_worker(move || {
            Self::new(runtime_config).load_btc_router_optional()
        })
        .await
        .map_err(|_| {
            btc_provider_failure(
                binding,
                ProviderDiagnosticCode::ProviderConfigurationInvalid,
            )
        })?;
        let _ = self.btc_router.set(loaded);
        self.cached_btc_router(binding)
    }
}

async fn load_runtime_config_on_blocking_worker<T, F>(load: F) -> Result<T, tokio::task::JoinError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tokio::task::spawn_blocking(load).await
}

impl mfm_adapters_btc_jsonrpc::BtcChainHeadProviderFactory for LiveTransportRuntime {
    fn validate_source_binding<'a>(
        &'a self,
        binding: BtcSourceBinding,
    ) -> mfm_adapters_btc_jsonrpc::BtcSourceBindingValidationFuture<'a> {
        Box::pin(async move { self.validate_btc_source_binding(binding).await })
    }

    fn bind_source(
        &self,
        binding: BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<Arc<dyn BtcChainHeadReadProvider>> {
        let router = self.cached_btc_router(&binding)?;
        router
            .bind_source(binding.clone())
            .map(|provider| Arc::new(provider) as Arc<dyn BtcChainHeadReadProvider>)
            .map_err(|error| enrich_btc_capability_error(&binding, error))
    }

    fn bind_balance_source(
        &self,
        binding: BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<Arc<dyn BtcBalanceReadProvider>> {
        let router = self.cached_btc_router(&binding)?;
        router
            .bind_source(binding.clone())
            .map(|provider| Arc::new(provider) as Arc<dyn BtcBalanceReadProvider>)
            .map_err(|error| enrich_btc_capability_error(&binding, error))
    }
}

fn btc_json_rpc_router(
    btc: mfm_runtime_config::BtcRuntimeConfig,
) -> Result<mfm_transports_btc_jsonrpc_http::BtcJsonRpcRouter, ProviderDiagnosticCode> {
    let mut routes = BTreeMap::new();
    for (source_identity, route) in btc.routes() {
        let client = btc_json_rpc_client(route)?;
        routes.insert(source_identity.clone(), Arc::new(client));
    }
    Ok(mfm_transports_btc_jsonrpc_http::BtcJsonRpcRouter::new(
        routes,
    ))
}

fn btc_json_rpc_client(
    json_rpc: &mfm_runtime_config::BtcJsonRpcRuntimeConfig,
) -> Result<mfm_transports_btc_jsonrpc_http::BtcJsonRpcClient, ProviderDiagnosticCode> {
    let config = mfm_transports_btc_jsonrpc_http::BtcJsonRpcConfig {
        rpc_url: json_rpc.rpc_url().expose_secret().to_owned(),
        rpc_user: json_rpc
            .rpc_user()
            .map(|value| value.expose_secret().to_owned()),
        rpc_password: json_rpc
            .rpc_password()
            .map(|value| value.expose_secret().to_owned()),
    };
    mfm_transports_btc_jsonrpc_http::BtcJsonRpcClient::new(config)
        .map_err(|_| ProviderDiagnosticCode::ProviderConfigurationInvalid)
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
        mfm_evm_capabilities::evm_diagnostic(code),
    ))
}

fn evm_transport_capability_error(
    binding: &EvmNetworkBinding,
    error: mfm_transports_evm::EvmTransportError,
) -> EvmCapabilityError {
    match error {
        mfm_transports_evm::EvmTransportError::SourceMismatch { diagnostic } => {
            EvmCapabilityError::SourceMismatch {
                diagnostic: enrich_evm_diagnostic(binding, diagnostic),
            }
        }
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

fn btc_provider_failure(
    binding: &BtcSourceBinding,
    code: ProviderDiagnosticCode,
) -> mfm_btc_capabilities::BtcCapabilityError {
    mfm_btc_capabilities::BtcCapabilityError::provider_failure(enrich_btc_diagnostic(
        binding,
        mfm_btc_capabilities::btc_diagnostic(code),
    ))
}

fn enrich_btc_capability_error(
    binding: &BtcSourceBinding,
    error: mfm_btc_capabilities::BtcCapabilityError,
) -> mfm_btc_capabilities::BtcCapabilityError {
    match error {
        mfm_btc_capabilities::BtcCapabilityError::Provider { diagnostic } => {
            mfm_btc_capabilities::BtcCapabilityError::provider_failure(enrich_btc_diagnostic(
                binding, diagnostic,
            ))
        }
        error => error,
    }
}

fn enrich_btc_diagnostic(
    binding: &BtcSourceBinding,
    diagnostic: RedactedProviderDiagnostic,
) -> RedactedProviderDiagnostic {
    diagnostic
        .with_field(
            diagnostic_id("network_id"),
            ProviderDiagnosticValue::Id(diagnostic_id(binding.network_id().as_str())),
        )
        .with_field(
            diagnostic_id("source_identity"),
            ProviderDiagnosticValue::Id(diagnostic_id(binding.source_identity().as_str())),
        )
        .with_field(
            diagnostic_id("bitcoin_network"),
            ProviderDiagnosticValue::Id(diagnostic_id(binding.bitcoin_network().as_str())),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfm_btc_capabilities::{BitcoinNetworkTag, BtcNetworkId, BtcSourceIdentity};

    fn evm_binding() -> EvmNetworkBinding {
        EvmNetworkBinding::new(LocalPublicId::new("test-evm").expect("network"), 1)
            .expect("binding")
    }

    fn btc_binding() -> BtcSourceBinding {
        BtcSourceBinding::new(
            BtcNetworkId::new("test-btc").expect("network"),
            BtcSourceIdentity::new("primary").expect("source"),
            BitcoinNetworkTag::Test,
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
        let error = runtime
            .validate_evm_read_route(evm_binding())
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
    async fn missing_config_preserves_bitcoin_semantic_binding() {
        let runtime = LiveTransportRuntime::new(RuntimeConfigLoader::from_path_or_env(None));
        let binding = btc_binding();
        let error = mfm_adapters_btc_jsonrpc::BtcChainHeadProviderFactory::validate_source_binding(
            &runtime, binding,
        )
        .await
        .expect_err("missing Bitcoin configuration");
        let diagnostic = error.redacted_diagnostic().expect("diagnostic");

        assert_eq!(
            diagnostic.code(),
            ProviderDiagnosticCode::ProviderConfigurationMissing
        );
        assert_eq!(diagnostic.provider_family().as_str(), "bitcoin");
        assert_eq!(
            serde_json::to_value(diagnostic).expect("diagnostic JSON")["fields"],
            serde_json::json!({
                "bitcoin_network": "test",
                "network_id": "test-btc",
                "source_identity": "primary"
            })
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn malformed_supplied_config_is_invalid_and_redacted() {
        let (dir, runtime) = runtime_with_config(
            "rpc_url = 'https://alice:password@example.invalid/private'\nauth_header = 'Bearer secret-token'\ninvalid = [",
        );
        let error = runtime
            .validate_evm_read_route(evm_binding())
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
        let error = mfm_adapters_btc_jsonrpc::BtcChainHeadProviderFactory::validate_source_binding(
            &runtime, binding,
        )
        .await
        .expect_err("Bitcoin family is missing");
        let diagnostic = error.redacted_diagnostic().expect("diagnostic");

        assert_eq!(diagnostic.provider_family().as_str(), "bitcoin");
        assert_eq!(
            diagnostic.code(),
            ProviderDiagnosticCode::ProviderConfigurationMissing
        );
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
        let error = runtime
            .validate_evm_read_route(evm_binding())
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
        let mismatch = mfm_evm_capabilities::source_mismatch_error(
            &binding,
            alloy_primitives::U256::from(2),
            &LocalPublicId::new("wrong-route").expect("source"),
        );
        let EvmCapabilityError::SourceMismatch { diagnostic } = mismatch else {
            panic!("source mismatch variant");
        };
        let error = evm_transport_capability_error(
            &binding,
            mfm_transports_evm::EvmTransportError::SourceMismatch { diagnostic },
        );

        assert!(matches!(error, EvmCapabilityError::SourceMismatch { .. }));
        assert_eq!(
            error.failure_disposition(mfm_evm_capabilities::EvmCapabilityPhase::ReadOnly),
            mfm_evm_capabilities::EvmCapabilityFailureDisposition::OperationalBlock
        );
    }
}
