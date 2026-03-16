#![allow(clippy::disallowed_methods)]

use std::path::PathBuf;

const ENV_KEYSTORE_PATH: &str = "MFM_KEYSTORE_PATH";

/// Resolves the effective keystore path from explicit CLI input, environment, or the standard default.
pub(crate) fn resolve_keystore_path(configured: Option<&PathBuf>) -> PathBuf {
    configured
        .cloned()
        .or_else(|| std::env::var(ENV_KEYSTORE_PATH).ok().map(PathBuf::from))
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".mfm").join("keystore")
        })
}
