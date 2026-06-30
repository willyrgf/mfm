use std::collections::BTreeMap;

use mfm_evm_capabilities::{EvmSourcePolicyId, EvmSourceRef};
use serde::Deserialize;

const ENV_EVM_NETWORK_ROUTES_JSON: &str = "MFM_EVM_NETWORK_ROUTES_JSON";

#[derive(Clone)]
pub(crate) struct EvmRuntimeRoutes {
    routes: BTreeMap<String, EvmRuntimeRoute>,
}

impl EvmRuntimeRoutes {
    pub(crate) fn from_env() -> mfm_runtime::Result<Self> {
        let raw = std::env::var(ENV_EVM_NETWORK_ROUTES_JSON).unwrap_or_else(|_| "[]".to_owned());
        Self::from_json_str(&raw)
    }

    fn from_json_str(raw: &str) -> mfm_runtime::Result<Self> {
        let entries = serde_json::from_str::<Vec<EnvEvmNetworkRoute>>(raw)
            .map_err(runtime_route_config_error)?;
        let mut routes = BTreeMap::new();
        for entry in entries {
            if entry.network_id.trim().is_empty() {
                return Err(route_error(
                    "EVM network route network_id must be non-empty",
                ));
            }
            let source_ref =
                EvmSourceRef::new(&entry.source_ref).map_err(runtime_route_capability_error)?;
            let policy_id =
                EvmSourcePolicyId::new(&entry.policy_id).map_err(runtime_route_capability_error)?;
            if routes
                .insert(
                    entry.network_id.clone(),
                    EvmRuntimeRoute {
                        source_ref,
                        policy_id,
                    },
                )
                .is_some()
            {
                return Err(route_error("duplicate EVM network route"));
            }
        }
        Ok(Self { routes })
    }

    pub(crate) fn route(&self, network_id: &str) -> mfm_runtime::Result<EvmRuntimeRoute> {
        self.routes
            .get(network_id)
            .cloned()
            .ok_or_else(|| route_error("missing EVM runtime route for network"))
    }

    pub(crate) fn portfolio_routes(
        &self,
    ) -> mfm_runtime::Result<mfm_adapters_portfolio::PortfolioEvmRoutes> {
        let routes = self
            .routes
            .iter()
            .map(|(network_id, route)| {
                let network_id =
                    mfm_portfolio_model::ids::NetworkId::new(network_id).map_err(|_| {
                        route_error("EVM network route network_id must use portfolio id grammar")
                    })?;
                Ok(mfm_adapters_portfolio::PortfolioEvmRoute::new(
                    network_id,
                    route.source_ref.clone(),
                    route.policy_id.clone(),
                ))
            })
            .collect::<mfm_runtime::Result<Vec<_>>>()?;
        mfm_adapters_portfolio::PortfolioEvmRoutes::new(routes)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct EvmRuntimeRoute {
    source_ref: EvmSourceRef,
    policy_id: EvmSourcePolicyId,
}

impl EvmRuntimeRoute {
    pub(crate) fn source_ref(&self) -> &EvmSourceRef {
        &self.source_ref
    }

    pub(crate) fn policy_id(&self) -> &EvmSourcePolicyId {
        &self.policy_id
    }

    pub(crate) fn into_contract_route(self) -> mfm_adapters_evm_contracts::EvmContractRuntimeRoute {
        mfm_adapters_evm_contracts::EvmContractRuntimeRoute::new(self.source_ref, self.policy_id)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvEvmNetworkRoute {
    network_id: String,
    source_ref: String,
    policy_id: String,
}

fn runtime_route_config_error(error: serde_json::Error) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(format!("invalid EVM network route config: {error}"))
}

fn runtime_route_capability_error(
    error: mfm_evm_capabilities::EvmCapabilityError,
) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(format!("invalid EVM network route: {error}"))
}

fn route_error(message: &'static str) -> mfm_runtime::RuntimeError {
    mfm_runtime::RuntimeError::RunnerBinding(message.to_owned())
}
