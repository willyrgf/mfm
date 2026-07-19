use std::collections::BTreeMap;
use std::fmt;

use mfm_ids::LocalPublicId;
use serde_json::Value;

use super::raw::{RawEvmConfig, RawEvmRoute};
use super::resolve::{parse_local_public_id, resolve_optional_http_authorization, resolve_rpc_url};
use super::{
    deserialize_family, reject_extra_fields, Result, RuntimeConfigError, RuntimeConfigErrorKind,
    RuntimeConfigIdentifierKind, RuntimeConfigLocation, RuntimeSecretValue,
};

/// Runtime-local direct network-to-source configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvmRuntimeConfig {
    routes: BTreeMap<LocalPublicId, EvmRpcRoute>,
}

impl EvmRuntimeConfig {
    /// Returns direct semantic network routes.
    pub const fn routes(&self) -> &BTreeMap<LocalPublicId, EvmRpcRoute> {
        &self.routes
    }

    /// Returns one direct route.
    pub fn route(&self, network_id: &LocalPublicId) -> Option<&EvmRpcRoute> {
        self.routes.get(network_id)
    }

    pub(super) fn from_raw(raw: RawEvmConfig) -> Result<Self> {
        reject_extra_fields(&raw.extra, RuntimeConfigLocation::Evm)?;
        let mut routes = BTreeMap::new();
        for (raw_network_id, raw_route) in raw.routes {
            let network_id = parse_network_id(&raw_network_id)?;
            let location = RuntimeConfigLocation::EvmRoute {
                network_id: Some(network_id.to_string()),
            };
            let raw_route = deserialize_family(
                raw_route,
                location.clone(),
                RuntimeConfigErrorKind::InvalidFamilyConfig,
            )?;
            routes.insert(network_id, EvmRpcRoute::from_raw(raw_route, location)?);
        }
        Ok(Self { routes })
    }

    pub(super) fn select(raw: Value, network_id: &LocalPublicId) -> Result<EvmRpcRoute> {
        let raw = deserialize_family::<RawEvmConfig>(
            raw,
            RuntimeConfigLocation::Evm,
            RuntimeConfigErrorKind::InvalidFamilyConfig,
        )?;
        reject_extra_fields(&raw.extra, RuntimeConfigLocation::Evm)?;
        let location = RuntimeConfigLocation::EvmRoute {
            network_id: Some(network_id.to_string()),
        };
        let raw_route = raw.routes.get(network_id.as_str()).ok_or_else(|| {
            RuntimeConfigError::new(location.clone(), RuntimeConfigErrorKind::MissingRoute)
        })?;
        let raw_route = deserialize_family::<RawEvmRoute>(
            raw_route.clone(),
            location.clone(),
            RuntimeConfigErrorKind::InvalidFamilyConfig,
        )?;
        EvmRpcRoute::from_raw(raw_route, location)
    }
}

/// One direct runtime route with a redacted source id and private endpoint.
#[derive(Clone, PartialEq, Eq)]
pub struct EvmRpcRoute {
    source_ref: LocalPublicId,
    rpc_url: RuntimeSecretValue,
    auth_header: Option<RuntimeSecretValue>,
}

impl EvmRpcRoute {
    /// Returns the redacted process-local source reference.
    pub const fn source_ref(&self) -> &LocalPublicId {
        &self.source_ref
    }

    /// Returns the resolved RPC URL.
    pub const fn rpc_url(&self) -> &RuntimeSecretValue {
        &self.rpc_url
    }

    /// Returns the resolved authorization header, when configured.
    pub const fn auth_header(&self) -> Option<&RuntimeSecretValue> {
        self.auth_header.as_ref()
    }

    fn from_raw(raw: RawEvmRoute, location: RuntimeConfigLocation) -> Result<Self> {
        reject_extra_fields(&raw.extra, location.clone())?;
        let source_ref = raw
            .source_ref
            .as_deref()
            .ok_or_else(|| {
                RuntimeConfigError::new(
                    location.clone().with_field("source_ref"),
                    RuntimeConfigErrorKind::MissingRequiredField,
                )
            })
            .and_then(|value| parse_local_public_id(value, location.clone()))?;
        let rpc_url = resolve_rpc_url(
            location.clone(),
            &raw.rpc_url,
            &raw.rpc_url_env,
            &raw.rpc_url_file,
            &raw.rpc_url_file_env,
        )?;
        let auth_header = resolve_optional_http_authorization(
            location,
            &raw.auth_header,
            &raw.auth_header_env,
            &raw.auth_header_file,
            &raw.auth_header_file_env,
        )?;
        Ok(Self {
            source_ref,
            rpc_url,
            auth_header,
        })
    }
}

impl fmt::Debug for EvmRpcRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EvmRpcRoute")
            .field("source_ref", &self.source_ref)
            .field("rpc_url", &self.rpc_url)
            .field("auth_header", &self.auth_header)
            .finish()
    }
}

fn parse_network_id(raw: &str) -> Result<LocalPublicId> {
    LocalPublicId::new(raw).map_err(|_| {
        RuntimeConfigError::new(
            RuntimeConfigLocation::EvmRoute { network_id: None },
            RuntimeConfigErrorKind::InvalidIdentifier {
                kind: RuntimeConfigIdentifierKind::NetworkId,
            },
        )
    })
}
