use std::path::PathBuf;

use crate::commands::result::PublicError;

/// Raw keystore selection arguments shared by direct keystore CLI commands.
pub(crate) struct KeystoreSelectionArgs<'a> {
    /// Explicit keystore path.
    pub(crate) keystore: Option<&'a PathBuf>,
    /// Explicit runtime config path.
    pub(crate) runtime_config: Option<&'a PathBuf>,
    /// Runtime-config keystore profile reference.
    pub(crate) keystore_ref: Option<&'a str>,
}

/// Converts CLI selection syntax into one opaque app-owned selection.
pub(crate) fn resolve_keystore_selection(
    args: KeystoreSelectionArgs<'_>,
) -> Result<mfm_app::KeystoreSelection, PublicError> {
    if let Some(path) = args.keystore {
        if args.runtime_config.is_some() || args.keystore_ref.is_some() {
            return Err(PublicError::bad_request(
                "invalid_argument",
                "--keystore cannot be combined with --runtime-config or --keystore-ref",
            ));
        }
        return Ok(mfm_app::KeystoreSelection::explicit(path.clone()));
    }

    Ok(mfm_app::KeystoreSelection::runtime_profile(
        args.runtime_config.cloned(),
        args.keystore_ref.map(str::to_owned),
    ))
}
