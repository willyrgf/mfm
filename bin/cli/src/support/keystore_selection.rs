#![allow(clippy::disallowed_methods)]

use std::env;
use std::path::PathBuf;

use crate::commands::result::PublicError;
use crate::support::keystore::KeystoreAccess;

const DEFAULT_KEYSTORE_REF: &str = "default";

/// Raw keystore selection arguments shared by direct keystore CLI commands.
pub(crate) struct KeystoreSelectionArgs<'a> {
    /// Explicit keystore path.
    pub(crate) keystore: Option<&'a PathBuf>,
    /// Explicit runtime config path.
    pub(crate) runtime_config: Option<&'a PathBuf>,
    /// Runtime-config keystore profile reference.
    pub(crate) keystore_ref: Option<&'a str>,
}

/// Resolves direct keystore command access from explicit args or runtime config.
pub(crate) fn resolve_keystore_access(
    args: KeystoreSelectionArgs<'_>,
) -> Result<KeystoreAccess, PublicError> {
    if let Some(path) = args.keystore {
        if args.runtime_config.is_some() || args.keystore_ref.is_some() {
            return Err(PublicError::bad_request(
                "invalid_argument",
                "--keystore cannot be combined with --runtime-config or --keystore-ref",
            ));
        }
        if path.as_os_str().is_empty() {
            return Err(PublicError::bad_request(
                "invalid_argument",
                "--keystore path cannot be empty",
            ));
        }
        return Ok(KeystoreAccess::explicit_path(path.clone()));
    }

    let runtime_config = args
        .runtime_config
        .cloned()
        .or_else(|| env::var_os(mfm_app::MFM_RUNTIME_CONFIG_FILE).map(PathBuf::from));
    let Some(runtime_config) = runtime_config else {
        return Err(PublicError::bad_request(
            "missing_keystore_selection",
            "provide --keystore or --runtime-config",
        ));
    };
    let keystore_ref = args.keystore_ref.unwrap_or(DEFAULT_KEYSTORE_REF);
    resolve_runtime_config_keystore(runtime_config, keystore_ref)
}

fn resolve_runtime_config_keystore(
    runtime_config: PathBuf,
    keystore_ref: &str,
) -> Result<KeystoreAccess, PublicError> {
    if runtime_config.as_os_str().is_empty() {
        return Err(PublicError::bad_request(
            "invalid_argument",
            "--runtime-config path cannot be empty",
        ));
    }
    let keystore_ref = mfm_runtime_config::KeystoreRef::new(keystore_ref).map_err(|_| {
        PublicError::bad_request(
            "invalid_argument",
            "--keystore-ref is not a valid profile ref",
        )
    })?;
    let config = mfm_runtime_config::RuntimeConfig::load_path_with_requirements(
        &runtime_config,
        mfm_runtime_config::RuntimeConfigRequirement::keystores(),
    )
    .map_err(|_| {
        PublicError::bad_request("RuntimeConfigInvalid", "Runtime configuration is invalid")
    })?;
    let profile = config.keystores().get(&keystore_ref).ok_or_else(|| {
        PublicError::bad_request(
            "keystore_profile_not_found",
            "runtime config keystore profile was not found",
        )
    })?;
    Ok(KeystoreAccess::runtime_config_profile(
        profile.keystore_path().expose_path().to_path_buf(),
        profile.unlock_file().expose_path().to_path_buf(),
    ))
}
