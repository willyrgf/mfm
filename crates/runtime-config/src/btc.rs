use std::collections::BTreeMap;
use std::fmt;

use mfm_btc_capabilities::BitcoinSourceIdentity;

use super::raw::{RawBtcConfig, RawBtcJsonRpcConfig};
use super::resolve::{resolve_optional_value, resolve_rpc_url};
use super::{
    reject_extra_fields, Result, RuntimeConfigError, RuntimeConfigErrorKind,
    RuntimeConfigIdentifierKind, RuntimeConfigLocation, RuntimeSecretValue,
};

/// Runtime Bitcoin JSON-RPC descriptor.
#[derive(Clone, PartialEq, Eq)]
pub struct BtcRuntimeConfig {
    routes: BTreeMap<BitcoinSourceIdentity, BtcJsonRpcRuntimeConfig>,
}

impl BtcRuntimeConfig {
    /// Returns Bitcoin JSON-RPC routes keyed by semantic source identity.
    pub const fn routes(&self) -> &BTreeMap<BitcoinSourceIdentity, BtcJsonRpcRuntimeConfig> {
        &self.routes
    }

    pub(super) fn from_raw(raw: RawBtcConfig) -> Result<Self> {
        reject_extra_fields(&raw.extra, RuntimeConfigLocation::Btc)?;
        if raw.routes.is_empty() {
            return Err(RuntimeConfigError::new(
                RuntimeConfigLocation::Btc.with_field("routes"),
                RuntimeConfigErrorKind::MissingRequiredField,
            ));
        }
        let mut routes = BTreeMap::new();
        for (raw_source_identity, raw_route) in raw.routes {
            let source_identity =
                BitcoinSourceIdentity::new(&raw_source_identity).map_err(|_| {
                    RuntimeConfigError::new(
                        RuntimeConfigLocation::BtcRoute {
                            source_identity: None,
                        },
                        RuntimeConfigErrorKind::InvalidIdentifier {
                            kind: RuntimeConfigIdentifierKind::BitcoinSourceIdentity,
                        },
                    )
                })?;
            let location = RuntimeConfigLocation::BtcRoute {
                source_identity: Some(source_identity.to_string()),
            };
            let route = BtcJsonRpcRuntimeConfig::from_raw(raw_route, location)?;
            routes.insert(source_identity, route);
        }
        Ok(Self { routes })
    }
}

impl fmt::Debug for BtcRuntimeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BtcRuntimeConfig")
            .field("routes", &self.routes)
            .finish()
    }
}

/// Runtime Bitcoin Core JSON-RPC endpoint descriptor.
#[derive(Clone, PartialEq, Eq)]
pub struct BtcJsonRpcRuntimeConfig {
    rpc_url: RuntimeSecretValue,
    rpc_user: Option<RuntimeSecretValue>,
    rpc_password: Option<RuntimeSecretValue>,
}

impl BtcJsonRpcRuntimeConfig {
    /// Returns the resolved Bitcoin Core RPC URL.
    pub const fn rpc_url(&self) -> &RuntimeSecretValue {
        &self.rpc_url
    }

    /// Returns the resolved RPC username, when configured.
    pub const fn rpc_user(&self) -> Option<&RuntimeSecretValue> {
        self.rpc_user.as_ref()
    }

    /// Returns the resolved RPC password, when configured.
    pub const fn rpc_password(&self) -> Option<&RuntimeSecretValue> {
        self.rpc_password.as_ref()
    }

    fn from_raw(raw: RawBtcJsonRpcConfig, location: RuntimeConfigLocation) -> Result<Self> {
        reject_extra_fields(&raw.extra, location.clone())?;
        let rpc_url = resolve_rpc_url(
            location.clone(),
            &raw.rpc_url,
            &raw.rpc_url_env,
            &raw.rpc_url_file,
            &raw.rpc_url_file_env,
        )?;
        let rpc_user = resolve_optional_value(
            location.clone(),
            "rpc_user",
            &raw.rpc_user,
            &raw.rpc_user_env,
            &raw.rpc_user_file,
            &raw.rpc_user_file_env,
        )?;
        let rpc_password = resolve_optional_value(
            location.clone(),
            "rpc_password",
            &raw.rpc_password,
            &raw.rpc_password_env,
            &raw.rpc_password_file,
            &raw.rpc_password_file_env,
        )?;
        if rpc_user.is_some() != rpc_password.is_some() {
            return Err(RuntimeConfigError::new(
                location,
                RuntimeConfigErrorKind::IncompleteBasicAuth,
            ));
        }
        Ok(Self {
            rpc_url,
            rpc_user,
            rpc_password,
        })
    }
}

impl fmt::Debug for BtcJsonRpcRuntimeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BtcJsonRpcRuntimeConfig")
            .field("rpc_url", &self.rpc_url)
            .field("rpc_user", &self.rpc_user)
            .field("rpc_password", &self.rpc_password)
            .finish()
    }
}
