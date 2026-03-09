#![allow(clippy::disallowed_methods)]

use std::path::PathBuf;

use crate::commands::result::CommandError;

const ENV_EVM_RPC_SOURCE_ID: &str = "MFM_EVM_RPC_SOURCE_ID";
const ENV_KEYSTORE_PATH: &str = "MFM_KEYSTORE_PATH";

/// Resolves the effective keystore path from explicit CLI input, environment, or the standard default.
pub fn resolve_keystore_path(configured: Option<&PathBuf>) -> PathBuf {
    configured
        .cloned()
        .or_else(|| std::env::var(ENV_KEYSTORE_PATH).ok().map(PathBuf::from))
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".mfm").join("keystore")
        })
}

/// Resolves the effective RPC source id from explicit CLI input or environment.
pub fn resolve_rpc_source_id(configured: Option<&str>) -> Result<String, CommandError> {
    let value = configured
        .map(str::to_string)
        .or_else(|| std::env::var(ENV_EVM_RPC_SOURCE_ID).ok())
        .ok_or_else(|| {
            CommandError::new(
                "MissingArgument",
                "Missing source id (--source-id or MFM_EVM_RPC_SOURCE_ID)",
            )
        })?;

    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CommandError::new(
            "MissingArgument",
            "Missing source id (--source-id or MFM_EVM_RPC_SOURCE_ID)",
        ));
    }

    Ok(trimmed.to_string())
}
