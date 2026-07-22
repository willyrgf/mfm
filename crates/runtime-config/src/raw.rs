use super::*;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(super) struct RawRuntimeConfig {
    #[serde(default)]
    pub(super) evm: Option<Value>,
    #[serde(default)]
    pub(super) btc: Option<Value>,
    #[serde(default)]
    pub(super) keystores: Option<Value>,
    #[serde(default)]
    pub(super) signers: Option<Value>,
    #[serde(flatten)]
    pub(super) extra: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct RawEvmConfig {
    #[serde(default)]
    pub(super) routes: BTreeMap<String, Value>,
    #[serde(flatten)]
    pub(super) extra: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct RawEvmRoute {
    #[serde(default)]
    pub(super) source_ref: Option<String>,
    #[serde(default)]
    pub(super) rpc_url: Option<String>,
    #[serde(default)]
    pub(super) rpc_url_env: Option<String>,
    #[serde(default)]
    pub(super) rpc_url_file: Option<String>,
    #[serde(default)]
    pub(super) rpc_url_file_env: Option<String>,
    #[serde(default)]
    pub(super) auth_header: Option<String>,
    #[serde(default)]
    pub(super) auth_header_env: Option<String>,
    #[serde(default)]
    pub(super) auth_header_file: Option<String>,
    #[serde(default)]
    pub(super) auth_header_file_env: Option<String>,
    #[serde(flatten)]
    pub(super) extra: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct RawBtcConfig {
    #[serde(default)]
    pub(super) routes: BTreeMap<String, RawBtcJsonRpcConfig>,
    #[serde(flatten)]
    pub(super) extra: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct RawBtcJsonRpcConfig {
    #[serde(default)]
    pub(super) scan_timeout_seconds: Option<u64>,
    #[serde(default)]
    pub(super) rpc_url: Option<String>,
    #[serde(default)]
    pub(super) rpc_url_env: Option<String>,
    #[serde(default)]
    pub(super) rpc_url_file: Option<String>,
    #[serde(default)]
    pub(super) rpc_url_file_env: Option<String>,
    #[serde(default)]
    pub(super) rpc_user: Option<String>,
    #[serde(default)]
    pub(super) rpc_user_env: Option<String>,
    #[serde(default)]
    pub(super) rpc_user_file: Option<String>,
    #[serde(default)]
    pub(super) rpc_user_file_env: Option<String>,
    #[serde(default)]
    pub(super) rpc_password: Option<String>,
    #[serde(default)]
    pub(super) rpc_password_env: Option<String>,
    #[serde(default)]
    pub(super) rpc_password_file: Option<String>,
    #[serde(default)]
    pub(super) rpc_password_file_env: Option<String>,
    #[serde(flatten)]
    pub(super) extra: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct RawKeystoreConfig {
    #[serde(default)]
    pub(super) keystore_path: Option<String>,
    #[serde(default)]
    pub(super) keystore_path_env: Option<String>,
    #[serde(default)]
    pub(super) keystore_path_file: Option<String>,
    #[serde(default)]
    pub(super) keystore_path_file_env: Option<String>,
    #[serde(default)]
    pub(super) unlock_file: Option<String>,
    #[serde(default)]
    pub(super) unlock_file_env: Option<String>,
    #[serde(default)]
    pub(super) unlock_file_file: Option<String>,
    #[serde(default)]
    pub(super) unlock_file_file_env: Option<String>,
    #[serde(flatten)]
    pub(super) extra: BTreeMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct RawSignerConfig {
    #[serde(default)]
    pub(super) provider: Option<String>,
    #[serde(default)]
    pub(super) keystore_ref: Option<String>,
    #[serde(default)]
    pub(super) entry_id: Option<String>,
    #[serde(flatten)]
    pub(super) extra: BTreeMap<String, Value>,
}

pub(super) fn parse_raw_config(raw: &str, format: RuntimeConfigFormat) -> Result<RawRuntimeConfig> {
    match format {
        RuntimeConfigFormat::Toml => toml::from_str(raw).map_err(|_| {
            RuntimeConfigError::new(
                RuntimeConfigLocation::Root,
                RuntimeConfigErrorKind::Syntax { format },
            )
        }),
        RuntimeConfigFormat::Json => serde_json::from_str(raw).map_err(|_| {
            RuntimeConfigError::new(
                RuntimeConfigLocation::Root,
                RuntimeConfigErrorKind::Syntax { format },
            )
        }),
    }
}

pub(super) fn reject_extra_fields(
    extra: &BTreeMap<String, Value>,
    location: RuntimeConfigLocation,
) -> Result<()> {
    if let Some(field) = extra.keys().next() {
        let kind = match forbidden_field_kind(field.as_str()) {
            Some(kind) => kind,
            None => RuntimeConfigErrorKind::UnknownField,
        };
        return Err(RuntimeConfigError::new(location, kind));
    }
    Ok(())
}

pub(super) fn forbidden_field_kind(field: &str) -> Option<RuntimeConfigErrorKind> {
    let field = field.to_ascii_lowercase();
    if field == "expected_chain_id" {
        return Some(RuntimeConfigErrorKind::ForbiddenExpectedChainId);
    }
    if field.contains("private_key")
        || field.contains("mnemonic")
        || field.contains("password")
        || field.contains("signed_material")
        || field.contains("signed_transaction")
        || field.contains("raw_transaction")
        || field.contains("raw_tx")
        || field.contains("signature_scalar")
    {
        return Some(RuntimeConfigErrorKind::ForbiddenSecretMaterial);
    }
    None
}
