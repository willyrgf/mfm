use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use mfm_btc_capabilities::{BtcChainHeadReadProvider, BtcSourceBinding};
use mfm_evm_capabilities::EvmNetworkBinding;
use mfm_signers_keystore::{KeystoreSignerProvider, KeystoreSignerRegistryEntry};

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

    fn load_evm(&self) -> mfm_runtime::Result<mfm_runtime_config::EvmRuntimeConfig> {
        self.load_runtime_config_with_requirements(
            mfm_runtime_config::RuntimeConfigRequirement::evm(),
        )
        .and_then(|config| {
            config.evm().cloned().ok_or_else(|| {
                mfm_runtime::RuntimeError::RunnerBinding("missing EVM runtime config".to_owned())
            })
        })
    }

    fn load_optional_btc(
        &self,
    ) -> mfm_runtime::Result<Option<mfm_runtime_config::BtcRuntimeConfig>> {
        let Some(path) = self.path.as_ref() else {
            return Ok(None);
        };
        match mfm_runtime_config::RuntimeConfig::load_path_with_requirements(
            path,
            mfm_runtime_config::RuntimeConfigRequirement::btc(),
        ) {
            Ok(config) => Ok(config.btc().cloned()),
            Err(error)
                if (error.location() == &mfm_runtime_config::RuntimeConfigLocation::Btc
                    && error.kind()
                        == &mfm_runtime_config::RuntimeConfigErrorKind::MissingFamily)
                    || matches!(
                        error.kind(),
                        mfm_runtime_config::RuntimeConfigErrorKind::Syntax { .. }
                    ) =>
            {
                Ok(None)
            }
            Err(error) => Err(runtime_config_error(error)),
        }
    }

    fn load_runtime_config_with_signers(
        &self,
    ) -> mfm_runtime::Result<mfm_runtime_config::RuntimeConfig> {
        self.load_runtime_config_with_requirements(
            mfm_runtime_config::RuntimeConfigRequirement::evm_with_signers(),
        )
    }

    fn load_runtime_config_with_requirements(
        &self,
        requirements: mfm_runtime_config::RuntimeConfigRequirement,
    ) -> mfm_runtime::Result<mfm_runtime_config::RuntimeConfig> {
        let path = self.path.as_ref().ok_or_else(|| {
            mfm_runtime::RuntimeError::RunnerBinding("missing runtime config file".to_owned())
        })?;
        mfm_runtime_config::RuntimeConfig::load_path_with_requirements(path, requirements)
            .map_err(runtime_config_error)
    }
}

pub(crate) struct LiveTransportRuntime {
    runtime_config: RuntimeConfigLoader,
    evm_client: OnceLock<mfm_runtime::Result<Arc<mfm_transports_evm::EvmJsonRpcClient>>>,
    btc_router: OnceLock<
        mfm_runtime::Result<Option<Arc<mfm_transports_btc_jsonrpc_http::BtcJsonRpcRouter>>>,
    >,
    signer_provider: OnceLock<mfm_runtime::Result<Arc<KeystoreSignerProvider>>>,
}

impl LiveTransportRuntime {
    pub(crate) fn new(runtime_config: RuntimeConfigLoader) -> Self {
        Self {
            runtime_config,
            evm_client: OnceLock::new(),
            btc_router: OnceLock::new(),
            signer_provider: OnceLock::new(),
        }
    }

    pub(crate) fn btc_configured(&self) -> mfm_runtime::Result<bool> {
        self.btc_router_optional().map(|router| router.is_some())
    }

    pub(crate) fn validate_evm_network_binding(
        &self,
        binding: &EvmNetworkBinding,
    ) -> mfm_runtime::Result<()> {
        self.evm_client()?
            .validate_network_binding(binding)
            .map_err(runtime_evm_transport_error)
    }

    pub(crate) fn evm_provider(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_runtime::Result<mfm_transports_evm::EvmJsonRpcNetworkProvider> {
        self.evm_client()?
            .bind_network(binding)
            .map_err(runtime_evm_transport_error)
    }

    pub(crate) fn signer_provider(&self) -> mfm_runtime::Result<Arc<KeystoreSignerProvider>> {
        self.signer_provider
            .get_or_init(|| {
                self.runtime_config
                    .load_runtime_config_with_signers()
                    .and_then(|config| keystore_signer_provider_from_config(&config))
                    .map(Arc::new)
            })
            .clone()
    }

    fn evm_client(&self) -> mfm_runtime::Result<Arc<mfm_transports_evm::EvmJsonRpcClient>> {
        self.evm_client
            .get_or_init(|| {
                self.runtime_config
                    .load_evm()
                    .and_then(evm_json_rpc_client)
                    .map(Arc::new)
            })
            .clone()
    }

    fn btc_router_optional(
        &self,
    ) -> mfm_runtime::Result<Option<Arc<mfm_transports_btc_jsonrpc_http::BtcJsonRpcRouter>>> {
        self.btc_router
            .get_or_init(|| {
                self.runtime_config
                    .load_optional_btc()?
                    .map(btc_json_rpc_router)
                    .transpose()
                    .map(|router| router.map(Arc::new))
            })
            .clone()
    }

    fn btc_router(
        &self,
    ) -> mfm_runtime::Result<Arc<mfm_transports_btc_jsonrpc_http::BtcJsonRpcRouter>> {
        self.btc_router_optional()?.ok_or_else(|| {
            mfm_runtime::RuntimeError::RunnerBinding("missing Bitcoin runtime config".to_owned())
        })
    }
}

impl mfm_adapters_portfolio::PortfolioTransportFactory for LiveTransportRuntime {
    fn validate_evm_network_binding(&self, binding: &EvmNetworkBinding) -> mfm_runtime::Result<()> {
        LiveTransportRuntime::validate_evm_network_binding(self, binding)
    }

    fn bind_evm_network(
        &self,
        binding: EvmNetworkBinding,
    ) -> mfm_evm_capabilities::Result<Arc<dyn mfm_adapters_portfolio::PortfolioEvmProvider>> {
        self.evm_client()
            .map_err(portfolio_runtime_config_capability_error)?
            .bind_network(binding)
            .map(|provider| {
                Arc::new(provider) as Arc<dyn mfm_adapters_portfolio::PortfolioEvmProvider>
            })
            .map_err(|error| {
                mfm_evm_capabilities::EvmCapabilityError::provider_failure(
                    error.into_provider_diagnostic(),
                )
            })
    }

    fn validate_btc_source_binding(&self, binding: &BtcSourceBinding) -> mfm_runtime::Result<()> {
        self.btc_router()?
            .validate_source_binding(binding)
            .map_err(runtime_btc_capability_error)
    }

    fn bind_btc_source(
        &self,
        binding: BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<Arc<dyn mfm_adapters_portfolio::PortfolioBtcProvider>> {
        let router = self
            .btc_router()
            .map_err(runtime_config_btc_capability_error)?;
        router.bind_source(binding).map(|provider| {
            Arc::new(provider) as Arc<dyn mfm_adapters_portfolio::PortfolioBtcProvider>
        })
    }
}

impl mfm_adapters_btc_jsonrpc::BtcChainHeadProviderFactory for LiveTransportRuntime {
    fn validate_source_binding(
        &self,
        binding: &BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<()> {
        let router = self
            .btc_router()
            .map_err(runtime_config_btc_capability_error)?;
        router.validate_source_binding(binding)
    }

    fn bind_source(
        &self,
        binding: BtcSourceBinding,
    ) -> mfm_btc_capabilities::Result<Arc<dyn BtcChainHeadReadProvider>> {
        let router = self
            .btc_router()
            .map_err(runtime_config_btc_capability_error)?;
        router
            .bind_source(binding)
            .map(|provider| Arc::new(provider) as Arc<dyn BtcChainHeadReadProvider>)
    }
}

fn evm_json_rpc_client(
    evm: mfm_runtime_config::EvmRuntimeConfig,
) -> mfm_runtime::Result<mfm_transports_evm::EvmJsonRpcClient> {
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
            .map_err(runtime_evm_transport_error)
        })
        .collect::<mfm_runtime::Result<Vec<_>>>()?;
    let policies = evm
        .policies()
        .iter()
        .map(|(policy_id, policy)| {
            mfm_transports_evm::EvmSourcePolicy::new(
                policy_id.clone(),
                policy.ordered_sources().to_vec(),
            )
            .map_err(runtime_evm_transport_error)
        })
        .collect::<mfm_runtime::Result<Vec<_>>>()?;
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
        .map_err(runtime_evm_transport_error)?;
    let route_registry =
        mfm_transports_evm::EvmRouteRegistry::new(routes).map_err(runtime_evm_transport_error)?;
    Ok(mfm_transports_evm::EvmJsonRpcClient::new(
        source_registry,
        route_registry,
    ))
}

fn btc_json_rpc_router(
    btc: mfm_runtime_config::BtcRuntimeConfig,
) -> mfm_runtime::Result<mfm_transports_btc_jsonrpc_http::BtcJsonRpcRouter> {
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
) -> mfm_runtime::Result<mfm_transports_btc_jsonrpc_http::BtcJsonRpcClient> {
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
        .map_err(|error| mfm_runtime::RuntimeError::RunnerBinding(error.to_string()))
}

fn keystore_signer_provider_from_config(
    runtime_config: &mfm_runtime_config::RuntimeConfig,
) -> mfm_runtime::Result<KeystoreSignerProvider> {
    let entries = runtime_config.signers().iter().map(|(signer_ref, signer)| {
        let signer = signer.as_keystore();
        let keystore = runtime_config
            .keystores()
            .get(signer.keystore_ref())
            .ok_or_else(|| {
                mfm_runtime::RuntimeError::RunnerBinding(
                    "missing keystore profile for signer binding".to_owned(),
                )
            })?;
        Ok(KeystoreSignerRegistryEntry::new(
            signer_ref.clone(),
            signer.entry_id(),
            keystore.keystore_path().expose_path(),
            keystore.unlock_file().expose_path(),
        ))
    });
    Ok(KeystoreSignerProvider::new(
        entries.collect::<mfm_runtime::Result<Vec<_>>>()?,
    ))
}

fn runtime_config_error(
    error: mfm_runtime_config::RuntimeConfigError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

fn runtime_evm_transport_error(
    error: mfm_transports_evm::EvmTransportError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

fn runtime_btc_capability_error(
    error: mfm_btc_capabilities::BtcCapabilityError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(error.to_string())
}

fn portfolio_runtime_config_capability_error(
    _error: mfm_runtime::RuntimeError,
) -> mfm_evm_capabilities::EvmCapabilityError {
    mfm_evm_capabilities::EvmCapabilityError::provider_failure(
        mfm_evm_capabilities::evm_diagnostic(
            mfm_capabilities::ProviderDiagnosticCode::ProviderConfigurationInvalid,
        ),
    )
}

fn runtime_config_btc_capability_error(
    _error: mfm_runtime::RuntimeError,
) -> mfm_btc_capabilities::BtcCapabilityError {
    mfm_btc_capabilities::BtcCapabilityError::provider_failure(
        mfm_btc_capabilities::btc_diagnostic(
            mfm_capabilities::ProviderDiagnosticCode::ProviderConfigurationInvalid,
        ),
    )
}
