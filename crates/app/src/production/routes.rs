use std::path::{Path, PathBuf};
use std::sync::Arc;

use mfm_evm_live::transport::{
    EvmJsonRpcTransport, EvmRoutingCatalogBuilder, EvmRpcAuthorization, EvmRpcEndpoint,
};
use mfm_portfolio::{EvmRoutingBinding, PortfolioRoutingManifest};

use crate::{runtime_config, ErrorClass, PublicError};

pub(super) struct EvmDeployment {
    pub(super) transport: Arc<EvmJsonRpcTransport>,
    pub(super) routing_manifest: PortfolioRoutingManifest,
}

pub(super) async fn load_evm_deployment(
    runtime_config_path: Option<&Path>,
) -> Result<EvmDeployment, PublicError> {
    let path = runtime_config_path
        .map(Path::to_path_buf)
        .ok_or_else(runtime_config_required)?;
    tokio::task::spawn_blocking(move || load_evm_deployment_sync(path))
        .await
        .map_err(|_| runtime_config_invalid())?
}

fn load_evm_deployment_sync(path: PathBuf) -> Result<EvmDeployment, PublicError> {
    let routes = runtime_config::load_evm_routes(&path).map_err(|_| runtime_config_invalid())?;
    let mut catalog = EvmRoutingCatalogBuilder::new();
    let mut bindings = Vec::with_capacity(routes.len());
    for route in routes {
        let (network_id, source_ref, chain_id, generation_id, rpc_url, auth_header) =
            route.into_parts();
        let endpoint =
            EvmRpcEndpoint::new(rpc_url.into_string()).map_err(|_| runtime_config_invalid())?;
        let authorization = auth_header
            .map(|value| EvmRpcAuthorization::new(value.into_protected()))
            .transpose()
            .map_err(|_| runtime_config_invalid())?;
        let generation = catalog
            .insert(
                network_id.as_str(),
                source_ref.as_str(),
                chain_id,
                generation_id,
                endpoint,
                authorization,
            )
            .map_err(|_| runtime_config_invalid())?;
        bindings.push(
            EvmRoutingBinding::new(network_id.as_str(), generation)
                .map_err(|_| runtime_config_invalid())?,
        );
    }
    let catalog = catalog.build().map_err(|_| runtime_config_invalid())?;
    let transport = EvmJsonRpcTransport::new(catalog).map_err(|_| runtime_config_invalid())?;
    let routing_manifest =
        PortfolioRoutingManifest::new(bindings).map_err(|_| runtime_config_invalid())?;
    Ok(EvmDeployment {
        transport: Arc::new(transport),
        routing_manifest,
    })
}

fn runtime_config_required() -> PublicError {
    PublicError::bad_request(
        "RuntimeConfigRequired",
        "Portfolio snapshot service requires explicit EVM runtime configuration",
    )
}

fn runtime_config_invalid() -> PublicError {
    PublicError::backend(
        ErrorClass::ServiceUnavailable,
        "RuntimeConfigInvalid",
        "EVM runtime configuration is invalid",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn absent_runtime_configuration_has_the_reviewed_public_contract() {
        let error = match load_evm_deployment(None).await {
            Ok(_) => panic!("runtime configuration is required"),
            Err(error) => error,
        };
        assert_eq!(error.class, ErrorClass::BadRequest);
        assert_eq!(error.code, "RuntimeConfigRequired");
        assert!(!error.message.contains("DATABASE_URL"));
    }
}
