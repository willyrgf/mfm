use std::fmt;
use std::path::{Path, PathBuf};

use mfm_evm::EvmEndpoint;
use serde::Deserialize;
use tokio::io::AsyncReadExt;

use crate::ComposeError;

/// Maximum encoded size of `deployment.toml`.
pub const MAX_DEPLOYMENT_DOCUMENT_BYTES: usize = 256 * 1024;

/// Error returned when an environment resolver name violates its grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("environment variable name is invalid")]
pub struct EnvironmentNameError;

/// A bounded environment resolver name safe to render in local diagnostics.
#[derive(Clone, PartialEq, Eq)]
pub struct EnvironmentName(String);

impl EnvironmentName {
    /// Checks `[A-Z_][A-Z0-9_]*` with an encoded length of 1 through 64.
    pub fn new(value: impl AsRef<str>) -> Result<Self, EnvironmentNameError> {
        let value = value.as_ref();
        let bytes = value.as_bytes();
        if bytes.is_empty()
            || bytes.len() > 64
            || !matches!(bytes[0], b'A'..=b'Z' | b'_')
            || bytes[1..]
                .iter()
                .any(|byte| !matches!(byte, b'A'..=b'Z' | b'0'..=b'9' | b'_'))
        {
            return Err(EnvironmentNameError);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the checked resolver name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for EnvironmentName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("EnvironmentName")
            .field(&self.0)
            .finish()
    }
}

impl fmt::Display for EnvironmentName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EnvironmentName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Strict operator bootstrap used only to compose private live authority.
pub struct Deployment {
    store: StoreDeployment,
    evm_routes: Vec<EvmRouteDeployment>,
}

impl Deployment {
    /// Resolves, bounds, reads, and strictly parses one `deployment.toml`.
    pub async fn load(override_path: Option<&Path>) -> Result<Self, ComposeError> {
        let path = resolve_path(override_path).ok_or(ComposeError::Deployment)?;
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|_| ComposeError::Deployment)?;
        let mut encoded = Vec::new();
        file.take((MAX_DEPLOYMENT_DOCUMENT_BYTES + 1) as u64)
            .read_to_end(&mut encoded)
            .await
            .map_err(|_| ComposeError::Deployment)?;
        if encoded.is_empty() || encoded.len() > MAX_DEPLOYMENT_DOCUMENT_BYTES {
            return Err(ComposeError::Deployment);
        }
        tokio::task::spawn_blocking(move || parse(encoded))
            .await
            .map_err(|_| ComposeError::Deployment)?
    }

    pub(crate) const fn runtime_locator_env(&self) -> &EnvironmentName {
        &self.store.runtime_locator_env
    }

    pub(crate) fn evm_routes(&self) -> &[EvmRouteDeployment] {
        &self.evm_routes
    }
}

pub(crate) struct EvmRouteDeployment {
    chain_id: u64,
    endpoint: EvmEndpoint,
    adapter_locator_env: EnvironmentName,
}

impl EvmRouteDeployment {
    pub(crate) const fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub(crate) const fn endpoint(&self) -> &EvmEndpoint {
        &self.endpoint
    }

    pub(crate) const fn adapter_locator_env(&self) -> &EnvironmentName {
        &self.adapter_locator_env
    }
}

struct StoreDeployment {
    runtime_locator_env: EnvironmentName,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeploymentWire {
    store: StoreWire,
    #[serde(default)]
    evm_routes: Vec<EvmRouteWire>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreWire {
    runtime_locator_env: EnvironmentName,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvmRouteWire {
    chain_id: i64,
    endpoint_id: String,
    adapter_locator_env: EnvironmentName,
}

fn parse(encoded: Vec<u8>) -> Result<Deployment, ComposeError> {
    if encoded.is_empty() || encoded.len() > MAX_DEPLOYMENT_DOCUMENT_BYTES {
        return Err(ComposeError::Deployment);
    }
    let text = std::str::from_utf8(&encoded).map_err(|_| ComposeError::Deployment)?;
    let wire: DeploymentWire = toml::from_str(text).map_err(|_| ComposeError::Deployment)?;
    if wire.evm_routes.len() > crate::MAX_EVM_BINDINGS
        || wire.evm_routes.windows(2).any(|pair| {
            (pair[0].chain_id, pair[0].endpoint_id.as_str())
                >= (pair[1].chain_id, pair[1].endpoint_id.as_str())
        })
    {
        return Err(ComposeError::Deployment);
    }
    let mut evm_routes = Vec::new();
    evm_routes
        .try_reserve_exact(wire.evm_routes.len())
        .map_err(|_| ComposeError::Deployment)?;
    for route in wire.evm_routes {
        let chain_id = u64::try_from(route.chain_id)
            .ok()
            .filter(|chain_id| *chain_id != 0)
            .ok_or(ComposeError::Deployment)?;
        let endpoint = EvmEndpoint::new(route.endpoint_id).map_err(|_| ComposeError::Deployment)?;
        evm_routes.push(EvmRouteDeployment {
            chain_id,
            endpoint,
            adapter_locator_env: route.adapter_locator_env,
        });
    }
    Ok(Deployment {
        store: StoreDeployment {
            runtime_locator_env: wire.store.runtime_locator_env,
        },
        evm_routes,
    })
}

fn resolve_path(override_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = override_path {
        return Some(path.to_owned());
    }
    resolve_default_path(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

fn resolve_default_path(
    xdg_config_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    usable_absolute_path(xdg_config_home)
        .map(|base| base.join("mfm/deployment.toml"))
        .or_else(|| usable_absolute_path(home).map(|base| base.join(".config/mfm/deployment.toml")))
}

fn usable_absolute_path(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let value = value?;
    let path = PathBuf::from(value);
    (path.is_absolute() && !path.as_os_str().is_empty()).then_some(path)
}

pub(crate) fn resolve_environment(name: &EnvironmentName) -> Result<String, ComposeError> {
    std::env::var(name.as_str()).map_err(|_| ComposeError::Environment(name.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_document(routes: &str) -> Vec<u8> {
        format!("[store]\nruntime_locator_env = \"MFM_RUNTIME\"\n{routes}").into_bytes()
    }

    #[test]
    fn strict_deployment_checks_routes_and_environment_names() {
        let deployment = parse(valid_document(
            "[[evm_routes]]\nchain_id = 1\nendpoint_id = \"alpha\"\nadapter_locator_env = \"MFM_EVM_ALPHA\"\n\n[[evm_routes]]\nchain_id = 2\nendpoint_id = \"beta\"\nadapter_locator_env = \"MFM_EVM_BETA\"\n",
        ))
        .expect("deployment");
        assert_eq!(deployment.evm_routes().len(), 2);
        assert_eq!(deployment.evm_routes()[0].chain_id(), 1);
        assert!(parse(valid_document("unknown = true\n")).is_err());
        assert!(parse(valid_document(
            "[[evm_routes]]\nchain_id = 0\nendpoint_id = \"alpha\"\nadapter_locator_env = \"MFM_EVM\"\n"
        ))
        .is_err());
        assert!(parse(valid_document(
            "[[evm_routes]]\nchain_id = 2\nendpoint_id = \"alpha\"\nadapter_locator_env = \"MFM_EVM\"\n[[evm_routes]]\nchain_id = 1\nendpoint_id = \"alpha\"\nadapter_locator_env = \"MFM_EVM\"\n"
        ))
        .is_err());
        assert!(parse(valid_document(
            "[[evm_routes]]\nchain_id = 1\nendpoint_id = \"alpha\"\nadapter_locator_env = \"lower\"\n"
        ))
        .is_err());
        for invalid in [
            b"\xff".to_vec(),
            valid_document("include = \"other.toml\"\n"),
            valid_document("profile = \"dev\"\n"),
            valid_document("[store]\nruntime_locator_env = \"MFM_OTHER\"\n"),
            valid_document(
                "[[evm_routes]]\nchain_id = -1\nendpoint_id = \"alpha\"\nadapter_locator_env = \"MFM_EVM\"\n",
            ),
            valid_document(
                "[[evm_routes]]\nchain_id = 1.0\nendpoint_id = \"alpha\"\nadapter_locator_env = \"MFM_EVM\"\n",
            ),
        ] {
            assert!(parse(invalid).is_err());
        }
        assert!(parse(vec![b'x'; MAX_DEPLOYMENT_DOCUMENT_BYTES + 1]).is_err());
        let routes = (1..=crate::MAX_EVM_BINDINGS + 1)
            .map(|chain_id| {
                format!(
                    "[[evm_routes]]\nchain_id = {chain_id}\nendpoint_id = \"endpoint\"\nadapter_locator_env = \"MFM_EVM\"\n"
                )
            })
            .collect::<String>();
        assert!(parse(valid_document(&routes)).is_err());
    }

    #[test]
    fn chain_id_upper_toml_bound_is_lossless() {
        let document = valid_document(&format!(
            "[[evm_routes]]\nchain_id = {}\nendpoint_id = \"alpha\"\nadapter_locator_env = \"MFM_EVM\"\n",
            i64::MAX
        ));
        assert_eq!(
            parse(document).expect("maximum").evm_routes()[0].chain_id(),
            i64::MAX as u64
        );
    }

    #[test]
    fn environment_name_grammar_is_exact() {
        for accepted in ["A", "_", "MFM_STORE_1", &"A".repeat(64)] {
            assert!(EnvironmentName::new(accepted).is_ok());
        }
        for rejected in [
            "",
            "lowercase",
            "1MFM",
            "MFM-STORE",
            "MFM STORE",
            "${OTHER}",
            &"A".repeat(65),
        ] {
            assert!(EnvironmentName::new(rejected).is_err());
        }
    }

    #[test]
    fn default_paths_follow_exact_xdg_and_home_precedence() {
        let explicit = Path::new("relative-override.toml");
        assert_eq!(resolve_path(Some(explicit)), Some(explicit.to_owned()));
        assert_eq!(
            resolve_default_path(Some("/xdg".into()), Some("/home/user".into())),
            Some(PathBuf::from("/xdg/mfm/deployment.toml"))
        );
        for unusable_xdg in [None, Some("".into()), Some("relative".into())] {
            assert_eq!(
                resolve_default_path(unusable_xdg, Some("/home/user".into())),
                Some(PathBuf::from("/home/user/.config/mfm/deployment.toml"))
            );
        }
        assert_eq!(resolve_default_path(None, None), None);
        assert_eq!(
            resolve_default_path(Some("/missing-but-selected".into()), Some("/home".into())),
            Some(PathBuf::from("/missing-but-selected/mfm/deployment.toml")),
            "file existence never changes XDG/HOME precedence"
        );
    }
}
