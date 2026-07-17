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

    fn load_optional(
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
    parsed_config: OnceLock<Result<Option<Arc<mfm_runtime_config::RuntimeConfig>>, ()>>,
    evm_client: OnceLock<Result<Arc<mfm_transports_evm::EvmJsonRpcClient>, ProviderDiagnosticCode>>,
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
            parsed_config: OnceLock::new(),
            evm_client: OnceLock::new(),
            btc_router: OnceLock::new(),
        }
    }

    pub(crate) fn validate_evm_network_binding(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<()> {
        self.evm_client(binding)?
            .validate_network_binding(binding)
            .map_err(|error| evm_transport_capability_error(binding, error))
    }

    pub(crate) fn evm_provider(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<mfm_transports_evm::EvmJsonRpcNetworkProvider> {
        let client = self.evm_client(&binding)?;
        client
            .bind_network(binding.clone())
            .map_err(|error| evm_transport_capability_error(&binding, error))
    }

    fn evm_client(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<Arc<mfm_transports_evm::EvmJsonRpcClient>> {
        self.evm_client
            .get_or_init(|| {
                self.evm_runtime_config()
                    .and_then(evm_json_rpc_client)
                    .map(Arc::new)
            })
            .clone()
            .map_err(|code| evm_provider_failure(binding, code))
    }

    fn btc_router_optional(
        &self,
    ) -> Result<
        Option<Arc<mfm_transports_btc_jsonrpc_http::BtcJsonRpcRouter>>,
        ProviderDiagnosticCode,
    > {
        self.btc_router
            .get_or_init(|| {
                self.btc_runtime_config_optional()?
                    .map(btc_json_rpc_router)
                    .transpose()
                    .map(|router| router.map(Arc::new))
            })
            .clone()
    }

    fn btc_router(
        &self,
        binding: &BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<Arc<mfm_transports_btc_jsonrpc_http::BtcJsonRpcRouter>> {
        self.btc_router_optional()
            .map_err(|code| btc_provider_failure(binding, code))?
            .ok_or_else(|| {
                btc_provider_failure(
                    binding,
                    ProviderDiagnosticCode::ProviderConfigurationMissing,
                )
            })
    }

    fn runtime_config_optional(
        &self,
    ) -> Result<Option<Arc<mfm_runtime_config::RuntimeConfig>>, ()> {
        self.parsed_config
            .get_or_init(|| {
                self.runtime_config
                    .load_optional()
                    .map(|config| config.map(Arc::new))
                    .map_err(|_| ())
            })
            .clone()
    }

    fn evm_runtime_config(
        &self,
    ) -> Result<mfm_runtime_config::EvmRuntimeConfig, ProviderDiagnosticCode> {
        self.runtime_config_optional()
            .map_err(|_| ProviderDiagnosticCode::ProviderConfigurationInvalid)?
            .and_then(|config| config.evm().cloned())
            .ok_or(ProviderDiagnosticCode::ProviderConfigurationMissing)
    }

    fn btc_runtime_config_optional(
        &self,
    ) -> Result<Option<mfm_runtime_config::BtcRuntimeConfig>, ProviderDiagnosticCode> {
        self.runtime_config_optional()
            .map_err(|_| ProviderDiagnosticCode::ProviderConfigurationInvalid)
            .map(|config| config.and_then(|config| config.btc().cloned()))
    }
}

impl mfm_adapters_btc_jsonrpc::BtcChainHeadProviderFactory for LiveTransportRuntime {
    fn validate_source_binding(
        &self,
        binding: &BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<()> {
        self.btc_router(binding)?
            .validate_source_binding(binding)
            .map_err(|error| enrich_btc_capability_error(binding, error))
    }

    fn bind_source(
        &self,
        binding: BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<Arc<dyn BtcChainHeadReadProvider>> {
        let router = self.btc_router(&binding)?;
        router
            .bind_source(binding.clone())
            .map(|provider| Arc::new(provider) as Arc<dyn BtcChainHeadReadProvider>)
            .map_err(|error| enrich_btc_capability_error(&binding, error))
    }

    fn bind_balance_source(
        &self,
        binding: BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<Arc<dyn BtcBalanceReadProvider>> {
        let router = self.btc_router(&binding)?;
        router
            .bind_source(binding.clone())
            .map(|provider| Arc::new(provider) as Arc<dyn BtcBalanceReadProvider>)
            .map_err(|error| enrich_btc_capability_error(&binding, error))
    }
}

fn evm_json_rpc_client(
    evm: mfm_runtime_config::EvmRuntimeConfig,
) -> Result<mfm_transports_evm::EvmJsonRpcClient, ProviderDiagnosticCode> {
    let sources = evm
        .sources()
        .iter()
        .map(|(source_ref, source)| {
            mfm_transports_evm::EvmRuntimeSource::new(
                source_ref.clone(),
                source.rpc_url().expose_secret().to_owned(),
                source
                    .auth_header()
                    .map(|value| value.expose_secret().to_owned()),
            )
            .map_err(|_| ProviderDiagnosticCode::ProviderConfigurationInvalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let policies = evm
        .policies()
        .iter()
        .map(|(policy_id, policy)| {
            mfm_transports_evm::EvmSourcePolicy::new(
                policy_id.clone(),
                policy.ordered_sources().to_vec(),
            )
            .map_err(|_| ProviderDiagnosticCode::ProviderConfigurationInvalid)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let routes = evm
        .routes()
        .iter()
        .map(|(network_id, route)| {
            mfm_transports_evm::EvmRoute::new(
                network_id.clone(),
                route.source_ref().clone(),
                route.policy_id().clone(),
            )
        })
        .collect::<Vec<_>>();
    let source_registry = mfm_transports_evm::EvmSourceRegistry::new(sources, policies)
        .map_err(|_| ProviderDiagnosticCode::ProviderConfigurationInvalid)?;
    let route_registry = mfm_transports_evm::EvmRouteRegistry::new(routes)
        .map_err(|_| ProviderDiagnosticCode::ProviderConfigurationInvalid)?;
    Ok(mfm_transports_evm::EvmJsonRpcClient::new(
        source_registry,
        route_registry,
    ))
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
    EvmCapabilityError::provider_failure(enrich_evm_diagnostic(
        binding,
        error.into_provider_diagnostic(),
    ))
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
    use mfm_evm_capabilities::EvmNetworkId;

    fn evm_binding() -> EvmNetworkBinding {
        EvmNetworkBinding::new(EvmNetworkId::new("test-evm").expect("network"), 1).expect("binding")
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

    #[test]
    fn missing_config_preserves_evm_semantic_binding() {
        let runtime = LiveTransportRuntime::new(RuntimeConfigLoader::from_path_or_env(None));
        let error = runtime
            .validate_evm_network_binding(&evm_binding())
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

    #[test]
    fn missing_config_preserves_bitcoin_semantic_binding() {
        let runtime = LiveTransportRuntime::new(RuntimeConfigLoader::from_path_or_env(None));
        let binding = btc_binding();
        let error = mfm_adapters_btc_jsonrpc::BtcChainHeadProviderFactory::validate_source_binding(
            &runtime, &binding,
        )
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

    #[test]
    fn malformed_supplied_config_is_invalid_and_redacted() {
        let (dir, runtime) = runtime_with_config(
            "rpc_url = 'https://alice:password@example.invalid/private'\nauth_header = 'Bearer secret-token'\ninvalid = [",
        );
        let error = runtime
            .validate_evm_network_binding(&evm_binding())
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

    #[test]
    fn supplied_config_missing_bitcoin_reports_only_bitcoin_missing() {
        let (_dir, runtime) = runtime_with_config(
            r#"
[evm.sources.primary]
rpc_url = "http://127.0.0.1:8545"

[evm.routes.test-evm]
source_ref = "primary"
"#,
        );
        let binding = btc_binding();
        let error = mfm_adapters_btc_jsonrpc::BtcChainHeadProviderFactory::validate_source_binding(
            &runtime, &binding,
        )
        .expect_err("Bitcoin family is missing");
        let diagnostic = error.redacted_diagnostic().expect("diagnostic");

        assert_eq!(diagnostic.provider_family().as_str(), "bitcoin");
        assert_eq!(
            diagnostic.code(),
            ProviderDiagnosticCode::ProviderConfigurationMissing
        );
    }

    #[test]
    fn missing_semantic_evm_route_retains_requested_binding() {
        let (_dir, runtime) = runtime_with_config(
            r#"
[evm.sources.primary]
rpc_url = "http://127.0.0.1:8545"

[evm.routes.other-evm]
source_ref = "primary"
"#,
        );
        let error = runtime
            .validate_evm_network_binding(&evm_binding())
            .expect_err("semantic route is missing");
        let diagnostic = error.redacted_diagnostic().expect("diagnostic");

        assert_eq!(diagnostic.code(), ProviderDiagnosticCode::RouteUnavailable);
        assert_eq!(
            serde_json::to_value(diagnostic).expect("diagnostic JSON")["fields"],
            serde_json::json!({"expected_chain_id": 1, "network_id": "test-evm"})
        );
    }
}
