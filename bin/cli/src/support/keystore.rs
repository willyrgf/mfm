#![allow(clippy::disallowed_methods)]

use crate::commands::result::PublicError;
use chrono::Utc;
use mfm_core::keystore::{KeyType, Keystore, KeystoreConfig, KeystoreError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zeroize::Zeroizing;

/// Supported direct keystore import kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImportKind {
    /// Import raw secp256k1 private key material.
    PrivateKey,
    /// Import a BIP-39 mnemonic and derive one Ethereum key.
    Mnemonic,
}

/// Optional BIP-39 passphrase source for mnemonic imports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Bip39ExtraSource {
    /// No passphrase.
    #[default]
    None,
    /// Prompt locally without echo.
    Prompt,
    /// Read from a local file or FIFO.
    FilePath(PathBuf),
}

/// Resolved keystore access for direct CLI commands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KeystoreAccess {
    path: PathBuf,
    credential: KeystoreCredentialSource,
}

impl KeystoreAccess {
    /// Creates prompt-based access for an explicit keystore path.
    pub(crate) fn explicit_path(path: PathBuf) -> Self {
        Self {
            path,
            credential: KeystoreCredentialSource::Prompt,
        }
    }

    /// Creates file-based access from a runtime-config keystore profile.
    pub(crate) fn runtime_config_profile(path: PathBuf, unlock_file: PathBuf) -> Self {
        Self {
            path,
            credential: KeystoreCredentialSource::File(unlock_file),
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum KeystoreCredentialSource {
    Prompt,
    File(PathBuf),
}

/// Keystore import request built by the CLI parser.
pub(crate) struct ImportKeyRequest {
    /// Import kind.
    pub(crate) kind: ImportKind,
    /// Optional label.
    pub(crate) label: Option<String>,
    /// BIP-32 derivation path for mnemonic imports.
    pub(crate) derivation_path: String,
    /// Whether secret material is read from stdin.
    pub(crate) stdin: bool,
    /// Keystore access.
    pub(crate) access: KeystoreAccess,
    /// Optional BIP-39 extra source.
    pub(crate) bip39_extra: Bip39ExtraSource,
}

/// Keystore import response.
pub(crate) struct ImportedKey {
    /// Key id.
    pub(crate) id: String,
    /// Key label.
    pub(crate) label: String,
    /// Stable key type code.
    pub(crate) key_type: String,
    /// Ethereum address.
    pub(crate) address: String,
    /// RFC3339 creation timestamp.
    pub(crate) created_at: String,
}

/// List sort order.
pub(crate) enum ListSortBy {
    /// Sort by label.
    Label,
    /// Sort by creation time.
    Created,
    /// Sort by key type.
    Type,
}

/// Keystore list request.
pub(crate) struct ListKeysRequest {
    /// Keystore access.
    pub(crate) access: KeystoreAccess,
    /// Whether to include addresses.
    pub(crate) show_addresses: bool,
    /// Optional label regex.
    pub(crate) filter_label: Option<String>,
    /// Sort order.
    pub(crate) sort_by: ListSortBy,
}

/// Single listed key.
pub(crate) struct ListedKey {
    /// Key id.
    pub(crate) id: String,
    /// Key label.
    pub(crate) label: String,
    /// Stable key type code.
    pub(crate) key_type: String,
    /// Optional address.
    pub(crate) address: Option<String>,
    /// Display timestamp.
    pub(crate) created: String,
}

/// Keystore list response.
pub(crate) struct ListedKeys {
    /// Keys.
    pub(crate) keys: Vec<ListedKey>,
    /// Whether addresses were included.
    pub(crate) show_addresses: bool,
}

/// Keystore delete request.
pub(crate) struct DeleteKeyRequest {
    /// Optional exact id.
    pub(crate) id: Option<String>,
    /// Optional exact label.
    pub(crate) by_label: Option<String>,
    /// Confirmation already granted.
    pub(crate) yes: bool,
    /// Keystore access.
    pub(crate) access: KeystoreAccess,
}

/// Keystore delete response.
pub(crate) struct DeletedKey {
    /// Deleted key id.
    pub(crate) id: String,
    /// Deleted key label.
    pub(crate) label: String,
}

struct ProcessSecretInput;

trait SecretInput {
    fn read_stdin_material(&mut self) -> Result<Zeroizing<String>, PublicError>;

    fn read_hidden(&mut self, prompt: &str) -> Result<Zeroizing<String>, PublicError>;
}

impl SecretInput for ProcessSecretInput {
    fn read_stdin_material(&mut self) -> Result<Zeroizing<String>, PublicError> {
        read_stdin_material()
    }

    fn read_hidden(&mut self, prompt: &str) -> Result<Zeroizing<String>, PublicError> {
        read_password(prompt)
    }
}

/// Imports a key directly through `mfm_core::keystore`.
pub(crate) fn import_key(req: ImportKeyRequest) -> Result<ImportedKey, PublicError> {
    let mut input = ProcessSecretInput;
    validate_import_request(&req)?;
    let material = match req.kind {
        ImportKind::PrivateKey => read_secret_input(
            &mut input,
            if req.stdin {
                ""
            } else {
                "Enter private key (hex): "
            },
            req.stdin,
        )?,
        ImportKind::Mnemonic => read_secret_input(
            &mut input,
            if req.stdin {
                ""
            } else {
                "Enter mnemonic phrase: "
            },
            req.stdin,
        )?,
    };

    match req.kind {
        ImportKind::PrivateKey => {
            let normalized = normalize_private_key(&material)?;
            let mut ks = create_keystore_if_needed(&req.access, &mut input)?;
            let label = req
                .label
                .unwrap_or_else(|| format!("imported-key-{}", Utc::now().format("%Y%m%d-%H%M%S")));
            let key_id = ks
                .import_private_key(Some(label), normalized.as_str())
                .map_err(admin_error_from_keystore)?;
            imported_key_from_keystore(&ks, key_id)
        }
        ImportKind::Mnemonic => {
            validate_mnemonic_basic(&material)?;
            let mut ks = create_keystore_if_needed(&req.access, &mut input)?;
            let label = req
                .label
                .unwrap_or_else(|| format!("imported-hd-{}", Utc::now().format("%Y%m%d-%H%M%S")));
            let extra = read_bip39_extra(req.bip39_extra, &mut input)?;
            let key_id = ks
                .import_mnemonic(
                    Some(label),
                    material.as_str(),
                    &req.derivation_path,
                    extra.as_ref().map(|v| v.as_str()),
                )
                .map_err(admin_error_from_keystore)?;
            imported_key_from_keystore(&ks, key_id)
        }
    }
}

/// Lists keys directly through `mfm_core::keystore`.
pub(crate) fn list_keys(req: ListKeysRequest) -> Result<ListedKeys, PublicError> {
    let keystore = load_unlocked_keystore(&req.access)?;
    let mut keys: Vec<ListedKey> = keystore
        .list_keys()
        .map_err(admin_error_from_keystore)?
        .into_iter()
        .map(|key| ListedKey {
            id: key.id.to_string(),
            label: key.alias.unwrap_or_else(|| "<no alias>".to_string()),
            key_type: key_type_code(&key.key_type).to_string(),
            address: if req.show_addresses {
                Some(format!("{:?}", key.address))
            } else {
                None
            },
            created: key.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        })
        .collect();

    if let Some(pattern) = req.filter_label.as_ref() {
        let regex = regex::Regex::new(pattern)
            .map_err(|_| PublicError::bad_request("invalid_regex", "Invalid regex pattern"))?;
        keys.retain(|key| regex.is_match(&key.label));
    }

    match req.sort_by {
        ListSortBy::Label => keys.sort_by(|a, b| a.label.cmp(&b.label)),
        ListSortBy::Created => keys.sort_by(|a, b| a.created.cmp(&b.created)),
        ListSortBy::Type => keys.sort_by(|a, b| a.key_type.cmp(&b.key_type)),
    }

    Ok(ListedKeys {
        keys,
        show_addresses: req.show_addresses,
    })
}

/// Deletes a key directly through `mfm_core::keystore`.
pub(crate) fn delete_key(req: DeleteKeyRequest) -> Result<DeletedKey, PublicError> {
    let mut keystore = load_unlocked_keystore(&req.access)?;
    let key_id = resolve_key_id(&keystore, req.id.as_deref(), req.by_label.as_deref())?;
    let key_to_delete = keystore
        .list_keys()
        .map_err(admin_error_from_keystore)?
        .into_iter()
        .find(|key| key.id == key_id)
        .ok_or_else(|| PublicError::bad_request("key_not_found", "Key not found"))?;

    if !req.yes {
        let label = key_to_delete.alias.as_deref().unwrap_or("<no alias>");
        let prompt = format!(
            "Are you sure you want to delete key '{}' (ID: {})?",
            label, key_to_delete.id
        );
        if !confirm(&prompt)? {
            return Err(PublicError::bad_request(
                "operation_cancelled",
                "Deletion cancelled by user",
            ));
        }
    }

    keystore
        .delete_key(key_id)
        .map_err(admin_error_from_keystore)?;

    Ok(DeletedKey {
        id: key_to_delete.id.to_string(),
        label: key_to_delete.alias.unwrap_or_default(),
    })
}

fn validate_import_request(req: &ImportKeyRequest) -> Result<(), PublicError> {
    if !matches!(req.bip39_extra, Bip39ExtraSource::None) && req.kind != ImportKind::Mnemonic {
        return Err(PublicError::bad_request(
            "invalid_import_config",
            "BIP-39 extra input is only supported for mnemonic imports",
        ));
    }

    if req.stdin && matches!(req.bip39_extra, Bip39ExtraSource::Prompt) {
        return Err(PublicError::bad_request(
            "invalid_import_config",
            "BIP-39 prompt input cannot be combined with stdin material",
        ));
    }

    Ok(())
}

fn imported_key_from_keystore(
    keystore: &Keystore,
    key_id: Uuid,
) -> Result<ImportedKey, PublicError> {
    let key_info = keystore
        .list_keys()
        .map_err(admin_error_from_keystore)?
        .into_iter()
        .find(|key| key.id == key_id)
        .ok_or_else(|| {
            PublicError::bad_request("key_not_found", "Failed to retrieve imported key info")
        })?;

    Ok(ImportedKey {
        id: key_info.id.to_string(),
        label: key_info.alias.unwrap_or_default(),
        key_type: key_type_code(&key_info.key_type).to_string(),
        address: format!("{:?}", key_info.address),
        created_at: key_info.created_at.to_rfc3339(),
    })
}

fn create_keystore_if_needed(
    access: &KeystoreAccess,
    input: &mut dyn SecretInput,
) -> Result<Keystore, PublicError> {
    let path = access.path();
    if path.exists() {
        return load_unlocked_keystore_with_input(access, input);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| {
            PublicError::internal("keystore_error", "Failed to create keystore directory")
        })?;
    }

    let password = get_create_password(access, input)?;
    let mut keystore = Keystore::new_with_config(path, KeystoreConfig::default())
        .map_err(|_| PublicError::internal("keystore_error", "Failed to create keystore"))?;
    keystore
        .unlock(password.as_str())
        .map_err(|_| PublicError::bad_request("keystore_error", "failed to unlock keystore"))?;
    Ok(keystore)
}

fn load_unlocked_keystore(access: &KeystoreAccess) -> Result<Keystore, PublicError> {
    let mut input = ProcessSecretInput;
    load_unlocked_keystore_with_input(access, &mut input)
}

fn load_unlocked_keystore_with_input(
    access: &KeystoreAccess,
    input: &mut dyn SecretInput,
) -> Result<Keystore, PublicError> {
    let path = access.path();
    if !path.exists() {
        return Err(PublicError::bad_request(
            "keystore_error",
            "Keystore not found",
        ));
    }

    let mut keystore = Keystore::new(path)
        .map_err(|_| PublicError::internal("keystore_error", "Failed to open keystore"))?;
    let password = get_unlock_password(access, input)?;
    keystore.unlock(password.as_str()).map_err(|_| {
        PublicError::bad_request("keystore_error", "invalid credential for keystore unlock")
    })?;
    Ok(keystore)
}

fn get_unlock_password(
    access: &KeystoreAccess,
    input: &mut dyn SecretInput,
) -> Result<Zeroizing<String>, PublicError> {
    match &access.credential {
        KeystoreCredentialSource::Prompt => input.read_hidden("Enter keystore password: "),
        KeystoreCredentialSource::File(path) => read_secret_file(path),
    }
}

fn get_create_password(
    access: &KeystoreAccess,
    input: &mut dyn SecretInput,
) -> Result<Zeroizing<String>, PublicError> {
    if let KeystoreCredentialSource::File(path) = &access.credential {
        return read_secret_file(path);
    }
    let password = input.read_hidden("Enter password for new keystore: ")?;
    let confirm_password = input.read_hidden("Confirm password: ")?;
    if password.as_str() != confirm_password.as_str() {
        return Err(PublicError::bad_request(
            "keystore_error",
            "credential entries did not match",
        ));
    }
    Ok(password)
}

fn read_bip39_extra(
    source: Bip39ExtraSource,
    input: &mut dyn SecretInput,
) -> Result<Option<Zeroizing<String>>, PublicError> {
    match source {
        Bip39ExtraSource::None => Ok(None),
        Bip39ExtraSource::Prompt => input.read_hidden("Enter BIP-39 passphrase: ").map(Some),
        Bip39ExtraSource::FilePath(path) => read_secret_file(&path).map(Some),
    }
}

fn read_secret_input(
    input: &mut dyn SecretInput,
    prompt: &str,
    from_stdin: bool,
) -> Result<Zeroizing<String>, PublicError> {
    if from_stdin {
        return input.read_stdin_material();
    }

    input.read_hidden(prompt)
}

fn read_stdin_material() -> Result<Zeroizing<String>, PublicError> {
    let mut input = Zeroizing::new(String::new());
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|_| PublicError::internal("input_error", "Failed to read secret input"))?;
    finalize_stdin_secret_material(&mut input)?;
    Ok(input)
}

fn finalize_stdin_secret_material(input: &mut String) -> Result<(), PublicError> {
    trim_line_endings(input);
    if input.contains(['\r', '\n']) {
        return Err(PublicError::bad_request(
            "input_error",
            "stdin secret material must contain exactly one line",
        ));
    }
    Ok(())
}

fn read_password(prompt: &str) -> Result<Zeroizing<String>, PublicError> {
    print!("{prompt}");
    io::stdout()
        .flush()
        .map_err(|_| PublicError::internal("input_error", "Failed to prompt for input"))?;
    let password = rpassword::read_password()
        .map_err(|_| PublicError::internal("input_error", "Failed to read password"))?;
    Ok(Zeroizing::new(password))
}

fn read_secret_file(path: &Path) -> Result<Zeroizing<String>, PublicError> {
    let mut raw = Zeroizing::new(
        std::fs::read_to_string(path)
            .map_err(|_| PublicError::internal("keystore_error", "Failed to read secret file"))?,
    );
    trim_line_endings(&mut raw);
    if raw.is_empty() {
        return Err(PublicError::bad_request(
            "keystore_error",
            "credential file was empty",
        ));
    }
    Ok(raw)
}

fn trim_line_endings(input: &mut String) {
    while input.ends_with(['\r', '\n']) {
        input.pop();
    }
}

fn normalize_private_key(input: &str) -> Result<Zeroizing<String>, PublicError> {
    let normalized = Zeroizing::new(input.strip_prefix("0x").unwrap_or(input).trim().to_string());
    if normalized.len() != 64 {
        return Err(PublicError::bad_request(
            "invalid_key_material",
            "Key material must be 64 hex characters",
        ));
    }
    if hex::decode(&normalized).is_err() {
        return Err(PublicError::bad_request(
            "invalid_key_material",
            "Key material must be valid hexadecimal",
        ));
    }
    Ok(normalized)
}

fn validate_mnemonic_basic(input: &str) -> Result<(), PublicError> {
    if input.split_whitespace().count() < 12 {
        return Err(PublicError::bad_request(
            "invalid_recovery_phrase",
            "Recovery phrase must have at least 12 words",
        ));
    }
    Ok(())
}

fn confirm(prompt: &str) -> Result<bool, PublicError> {
    loop {
        print!("{prompt} (y/N): ");
        io::stdout()
            .flush()
            .map_err(|_| PublicError::internal("input_error", "Failed to prompt for input"))?;

        let mut input = String::new();
        io::stdin().read_line(&mut input).map_err(|_| {
            PublicError::internal("input_error", "Failed to read confirmation input")
        })?;
        match input.trim().to_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" | "" => return Ok(false),
            _ => {
                println!("Please enter 'y' or 'n'");
            }
        }
    }
}

fn resolve_key_id(
    keystore: &Keystore,
    id: Option<&str>,
    by_label: Option<&str>,
) -> Result<Uuid, PublicError> {
    match (id, by_label) {
        (Some(_), Some(_)) => Err(PublicError::bad_request(
            "missing_argument",
            "Specify exactly one key selector: ID or --by-label",
        )),
        (None, None) => Err(PublicError::bad_request(
            "missing_argument",
            "Must specify either key ID or --by-label",
        )),
        (Some(raw), None) => Uuid::parse_str(raw)
            .map_err(|_| PublicError::bad_request("invalid_uuid", "Invalid UUID format")),
        (None, Some(label)) => {
            let keys = keystore
                .list_keys()
                .map_err(|_| PublicError::internal("keystore_error", "Failed to list keys"))?;
            let matching: Vec<_> = keys
                .iter()
                .filter(|key| key.alias.as_deref() == Some(label))
                .collect();

            match matching.len() {
                0 => Err(PublicError::bad_request(
                    "key_not_found",
                    "No key found with requested label",
                )),
                1 => Ok(matching[0].id),
                _ => Err(PublicError::bad_request(
                    "ambiguous_label",
                    "Multiple keys found with requested label",
                )),
            }
        }
    }
}

fn admin_error_from_keystore(err: KeystoreError) -> PublicError {
    match err {
        KeystoreError::InvalidPrivateKey => {
            PublicError::bad_request("invalid_key_material", "Key material format is invalid")
        }
        KeystoreError::InvalidMnemonic(_) => {
            PublicError::bad_request("invalid_recovery_phrase", "Recovery phrase is invalid")
        }
        KeystoreError::InvalidDerivationPath(_) => {
            PublicError::bad_request("invalid_derivation_path", "Derivation path is invalid")
        }
        KeystoreError::KeyNotFound(_) => {
            PublicError::bad_request("key_not_found", "Requested key does not exist")
        }
        _ => PublicError::internal("keystore_error", "Keystore operation failed"),
    }
}

fn key_type_code(key_type: &KeyType) -> &'static str {
    match key_type {
        KeyType::PrivateKey => "privatekey",
        KeyType::HdDerived { .. } => "hd_derived",
    }
}
