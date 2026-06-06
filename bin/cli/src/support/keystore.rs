#![allow(clippy::disallowed_methods)]

use crate::commands::result::CommandError;
use chrono::Utc;
use mfm_core::keystore::{KeyType, Keystore, KeystoreConfig, KeystoreError};
use mfm_evm_core::tx::{
    eip1559_signing_hash, encode_signed_eip1559_tx_hex, parse_address, parse_data_hex,
    parse_u128_quantity, Eip1559TxToSign,
};
use mfm_evm_core::util_error::UtilError;
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zeroize::Zeroizing;

const ENV_KEYSTORE_PASSWORD_FILE: &str = "MFM_KEYSTORE_PASSWORD_FILE";
const ENV_KEYSTORE_PASSWORD: &str = "MFM_KEYSTORE_PASSWORD";
const ENV_INTEGRATION_TEST: &str = "MFM_INTEGRATION_TEST";

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

/// Local output write policy for `keystore tx-sign`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum OutputWriteMode {
    /// Create a new output and fail if the target exists.
    #[default]
    CreateNew,
    /// Replace an existing regular output file.
    Overwrite,
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
    /// Keystore path.
    pub(crate) keystore_path: PathBuf,
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
    /// Keystore path.
    pub(crate) keystore_path: PathBuf,
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
    /// Keystore path.
    pub(crate) keystore_path: PathBuf,
}

/// Keystore delete response.
pub(crate) struct DeletedKey {
    /// Deleted key id.
    pub(crate) id: String,
    /// Deleted key label.
    pub(crate) label: String,
}

/// Keystore transaction signing request.
pub(crate) struct TxSignRequest {
    /// Optional exact id.
    pub(crate) id: Option<String>,
    /// Optional exact label.
    pub(crate) by_label: Option<String>,
    /// Recipient.
    pub(crate) to: String,
    /// Transfer value in wei.
    pub(crate) value_wei: String,
    /// Chain id.
    pub(crate) chain_id: u64,
    /// Nonce.
    pub(crate) nonce: u64,
    /// Max fee per gas in wei.
    pub(crate) max_fee_per_gas: String,
    /// Max priority fee per gas in wei.
    pub(crate) max_priority_fee_per_gas: String,
    /// Gas limit.
    pub(crate) gas_limit: u64,
    /// Output file.
    pub(crate) out_path: PathBuf,
    /// Output write policy.
    pub(crate) out_write_mode: OutputWriteMode,
    /// Calldata hex.
    pub(crate) data: String,
    /// Keystore path.
    pub(crate) keystore_path: PathBuf,
}

/// Keystore transaction signing response.
pub(crate) struct SignedTx {
    /// Sender address.
    pub(crate) from: String,
    /// Recipient address.
    pub(crate) to: String,
    /// Nonce.
    pub(crate) nonce: u64,
    /// Chain id.
    pub(crate) chain_id: u64,
    /// EVM transaction type.
    pub(crate) tx_type: String,
    /// Signing payload hash.
    pub(crate) payload_hash: String,
}

struct ProcessSecretInput;

trait SecretInput {
    fn read_stdin_material(&mut self) -> Result<Zeroizing<String>, CommandError>;

    fn read_hidden(&mut self, prompt: &str) -> Result<Zeroizing<String>, CommandError>;
}

impl SecretInput for ProcessSecretInput {
    fn read_stdin_material(&mut self) -> Result<Zeroizing<String>, CommandError> {
        read_stdin_material()
    }

    fn read_hidden(&mut self, prompt: &str) -> Result<Zeroizing<String>, CommandError> {
        read_password(prompt)
    }
}

/// Imports a key directly through `mfm_core::keystore`.
pub(crate) fn import_key(req: ImportKeyRequest) -> Result<ImportedKey, CommandError> {
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
            let mut ks = create_keystore_if_needed(&req.keystore_path, &mut input)?;
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
            let mut ks = create_keystore_if_needed(&req.keystore_path, &mut input)?;
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
pub(crate) fn list_keys(req: ListKeysRequest) -> Result<ListedKeys, CommandError> {
    let keystore = load_unlocked_keystore(&req.keystore_path)?;
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
            .map_err(|_| CommandError::new("invalid_regex", "Invalid regex pattern"))?;
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
pub(crate) fn delete_key(req: DeleteKeyRequest) -> Result<DeletedKey, CommandError> {
    let mut keystore = load_unlocked_keystore(&req.keystore_path)?;
    let key_id = resolve_delete_key_id(&keystore, req.id.as_deref(), req.by_label.as_deref())?;
    let key_to_delete = keystore
        .list_keys()
        .map_err(admin_error_from_keystore)?
        .into_iter()
        .find(|key| key.id == key_id)
        .ok_or_else(|| CommandError::new("key_not_found", "Key not found"))?;

    if !req.yes {
        let label = key_to_delete.alias.as_deref().unwrap_or("<no alias>");
        let prompt = format!(
            "Are you sure you want to delete key '{}' (ID: {})?",
            label, key_to_delete.id
        );
        if !confirm(&prompt)? {
            return Err(CommandError::new(
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

/// Signs an EIP-1559 transaction and writes the raw signed transaction locally.
pub(crate) fn sign_transaction(req: TxSignRequest) -> Result<SignedTx, CommandError> {
    let mut keystore = load_unlocked_keystore(&req.keystore_path)?;
    let key_id = resolve_key_id(&keystore, req.id.as_deref(), req.by_label.as_deref())?;
    let tx = Eip1559TxToSign {
        to: Some(parse_address(&req.to, "to").map_err(command_error_from_util)?),
        value_wei: parse_u128_quantity(&req.value_wei, "value-wei")
            .map_err(command_error_from_util)?,
        chain_id: req.chain_id,
        nonce: req.nonce,
        max_fee_per_gas: parse_u128_quantity(&req.max_fee_per_gas, "max-fee-per-gas")
            .map_err(command_error_from_util)?,
        max_priority_fee_per_gas: parse_u128_quantity(
            &req.max_priority_fee_per_gas,
            "max-priority-fee-per-gas",
        )
        .map_err(command_error_from_util)?,
        gas_limit: req.gas_limit,
        data: parse_data_hex(&req.data).map_err(command_error_from_util)?,
    };

    if tx.max_priority_fee_per_gas > tx.max_fee_per_gas {
        return Err(CommandError::new(
            "invalid_fee_config",
            "max-priority-fee-per-gas must be <= max-fee-per-gas",
        ));
    }

    let secure_key = keystore
        .get_private_key(key_id)
        .map_err(|err| CommandError::new("keystore_error", err.to_string()))?;
    let from_address = secure_key
        .ethereum_address()
        .map_err(|err| CommandError::new("signing_error", err.to_string()))?;
    let to_address = tx
        .to
        .map(|address| format!("{address:?}"))
        .unwrap_or_default();

    let hash = eip1559_signing_hash(&tx);
    let mut hash_bytes = [0u8; 32];
    hash_bytes.copy_from_slice(hash.as_slice());
    let signature = secure_key
        .sign_hash_recoverable(&hash_bytes)
        .map_err(|err| CommandError::new("signing_error", err.to_string()))?;
    let raw_tx_hex = encode_signed_eip1559_tx_hex(&tx, signature);
    write_raw_transaction_file(&req.out_path, &raw_tx_hex, req.out_write_mode)?;

    Ok(SignedTx {
        from: format!("{from_address:?}"),
        to: to_address,
        nonce: tx.nonce,
        chain_id: tx.chain_id,
        tx_type: "0x2".to_string(),
        payload_hash: format!("0x{}", hex::encode(hash.as_slice())),
    })
}

fn validate_import_request(req: &ImportKeyRequest) -> Result<(), CommandError> {
    if !matches!(req.bip39_extra, Bip39ExtraSource::None) && req.kind != ImportKind::Mnemonic {
        return Err(CommandError::new(
            "invalid_import_config",
            "BIP-39 extra input is only supported for mnemonic imports",
        ));
    }

    if req.stdin && matches!(req.bip39_extra, Bip39ExtraSource::Prompt) {
        return Err(CommandError::new(
            "invalid_import_config",
            "BIP-39 prompt input cannot be combined with stdin material",
        ));
    }

    Ok(())
}

fn imported_key_from_keystore(
    keystore: &Keystore,
    key_id: Uuid,
) -> Result<ImportedKey, CommandError> {
    let key_info = keystore
        .list_keys()
        .map_err(admin_error_from_keystore)?
        .into_iter()
        .find(|key| key.id == key_id)
        .ok_or_else(|| {
            CommandError::new("key_not_found", "Failed to retrieve imported key info")
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
    path: &Path,
    input: &mut dyn SecretInput,
) -> Result<Keystore, CommandError> {
    if path.exists() {
        return load_unlocked_keystore_with_input(path, input);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            CommandError::new(
                "keystore_error",
                format!("Failed to create keystore directory: {err}"),
            )
        })?;
    }

    let password = get_create_password(input)?;
    let cfg = if std::env::var(ENV_INTEGRATION_TEST).is_ok() {
        KeystoreConfig::insecure_integration_test()
    } else {
        KeystoreConfig::default()
    };
    let mut keystore = Keystore::new_with_config(path, cfg).map_err(|err| {
        CommandError::new(
            "keystore_error",
            format!("Failed to create keystore: {err}"),
        )
    })?;
    keystore
        .unlock(password.as_str())
        .map_err(|_| CommandError::new("keystore_error", "failed to unlock keystore"))?;
    Ok(keystore)
}

fn load_unlocked_keystore(path: &Path) -> Result<Keystore, CommandError> {
    let mut input = ProcessSecretInput;
    load_unlocked_keystore_with_input(path, &mut input)
}

fn load_unlocked_keystore_with_input(
    path: &Path,
    input: &mut dyn SecretInput,
) -> Result<Keystore, CommandError> {
    if !path.exists() {
        return Err(CommandError::new("keystore_error", "Keystore not found"));
    }

    let mut keystore =
        Keystore::new(path).map_err(|err| CommandError::new("keystore_error", err.to_string()))?;
    let password = get_unlock_password(input)?;
    keystore.unlock(password.as_str()).map_err(|_| {
        CommandError::new("keystore_error", "invalid credential for keystore unlock")
    })?;
    Ok(keystore)
}

fn get_unlock_password(input: &mut dyn SecretInput) -> Result<Zeroizing<String>, CommandError> {
    if let Some(password) = password_from_env_sources()? {
        return Ok(password);
    }
    input.read_hidden("Enter keystore password: ")
}

fn get_create_password(input: &mut dyn SecretInput) -> Result<Zeroizing<String>, CommandError> {
    if let Some(password) = password_from_env_sources()? {
        return Ok(password);
    }
    let password = input.read_hidden("Enter password for new keystore: ")?;
    let confirm_password = input.read_hidden("Confirm password: ")?;
    if password.as_str() != confirm_password.as_str() {
        return Err(CommandError::new(
            "keystore_error",
            "credential entries did not match",
        ));
    }
    Ok(password)
}

fn password_from_env_sources() -> Result<Option<Zeroizing<String>>, CommandError> {
    if let Ok(password_file) = std::env::var(ENV_KEYSTORE_PASSWORD_FILE) {
        return Ok(Some(read_password_file(&password_file)?));
    }

    if let Ok(password) = std::env::var(ENV_KEYSTORE_PASSWORD) {
        eprintln!(
            "Warning: {ENV_KEYSTORE_PASSWORD} may expose secrets; prefer {ENV_KEYSTORE_PASSWORD_FILE}."
        );
        return Ok(Some(Zeroizing::new(password)));
    }

    Ok(None)
}

fn read_bip39_extra(
    source: Bip39ExtraSource,
    input: &mut dyn SecretInput,
) -> Result<Option<Zeroizing<String>>, CommandError> {
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
) -> Result<Zeroizing<String>, CommandError> {
    if from_stdin {
        return input.read_stdin_material();
    }

    input.read_hidden(prompt)
}

fn read_stdin_material() -> Result<Zeroizing<String>, CommandError> {
    let mut input = Zeroizing::new(String::new());
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|err| CommandError::new("input_error", err.to_string()))?;
    finalize_stdin_secret_material(&mut input)?;
    Ok(input)
}

fn finalize_stdin_secret_material(input: &mut String) -> Result<(), CommandError> {
    trim_line_endings(input);
    if input.contains(['\r', '\n']) {
        return Err(CommandError::new(
            "input_error",
            "stdin secret material must contain exactly one line",
        ));
    }
    Ok(())
}

fn read_password(prompt: &str) -> Result<Zeroizing<String>, CommandError> {
    print!("{prompt}");
    io::stdout()
        .flush()
        .map_err(|err| CommandError::new("input_error", err.to_string()))?;
    let password = rpassword::read_password()
        .map_err(|err| CommandError::new("input_error", err.to_string()))?;
    Ok(Zeroizing::new(password))
}

fn read_password_file(path: &str) -> Result<Zeroizing<String>, CommandError> {
    read_secret_file(Path::new(path))
}

fn read_secret_file(path: &Path) -> Result<Zeroizing<String>, CommandError> {
    let mut raw = Zeroizing::new(
        std::fs::read_to_string(path)
            .map_err(|err| CommandError::new("keystore_error", err.to_string()))?,
    );
    trim_line_endings(&mut raw);
    if raw.is_empty() {
        return Err(CommandError::new(
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

fn normalize_private_key(input: &str) -> Result<Zeroizing<String>, CommandError> {
    let normalized = Zeroizing::new(input.strip_prefix("0x").unwrap_or(input).trim().to_string());
    if normalized.len() != 64 {
        return Err(CommandError::new(
            "invalid_key_material",
            "Key material must be 64 hex characters",
        ));
    }
    if hex::decode(&normalized).is_err() {
        return Err(CommandError::new(
            "invalid_key_material",
            "Key material must be valid hexadecimal",
        ));
    }
    Ok(normalized)
}

fn validate_mnemonic_basic(input: &str) -> Result<(), CommandError> {
    if input.split_whitespace().count() < 12 {
        return Err(CommandError::new(
            "invalid_recovery_phrase",
            "Recovery phrase must have at least 12 words",
        ));
    }
    Ok(())
}

fn confirm(prompt: &str) -> Result<bool, CommandError> {
    loop {
        print!("{prompt} (y/N): ");
        io::stdout()
            .flush()
            .map_err(|err| CommandError::new("input_error", err.to_string()))?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(|err| CommandError::new("input_error", err.to_string()))?;
        match input.trim().to_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" | "" => return Ok(false),
            _ => {
                println!("Please enter 'y' or 'n'");
            }
        }
    }
}

fn resolve_delete_key_id(
    keystore: &Keystore,
    id: Option<&str>,
    by_label: Option<&str>,
) -> Result<Uuid, CommandError> {
    match (id, by_label) {
        (Some(_), Some(_)) => Err(CommandError::new(
            "missing_argument",
            "Specify exactly one key selector: ID or --by-label",
        )),
        (None, None) => Err(CommandError::new(
            "missing_argument",
            "Must specify either key ID or --by-label",
        )),
        (Some(raw), None) => Uuid::parse_str(raw)
            .map_err(|_| CommandError::new("invalid_uuid", "Invalid UUID format")),
        (None, Some(label)) => {
            let keys = keystore.list_keys().map_err(admin_error_from_keystore)?;
            let matching: Vec<_> = keys
                .iter()
                .filter(|key| key.alias.as_deref() == Some(label))
                .collect();
            match matching.len() {
                0 => Err(CommandError::new(
                    "key_not_found",
                    "No key found with requested label",
                )),
                1 => Ok(matching[0].id),
                _ => Err(CommandError::new(
                    "ambiguous_label",
                    "Multiple keys found with requested label",
                )),
            }
        }
    }
}

fn resolve_key_id(
    keystore: &Keystore,
    id: Option<&str>,
    by_label: Option<&str>,
) -> Result<Uuid, CommandError> {
    match (id, by_label) {
        (Some(_), Some(_)) => Err(CommandError::new(
            "missing_argument",
            "Specify exactly one key selector: --id or --by-label",
        )),
        (None, None) => Err(CommandError::new(
            "missing_argument",
            "Must specify one key selector: --id or --by-label",
        )),
        (Some(raw), None) => Uuid::parse_str(raw)
            .map_err(|_| CommandError::new("invalid_uuid", "Invalid UUID format")),
        (None, Some(label)) => {
            let keys = keystore
                .list_keys()
                .map_err(|err| CommandError::new("keystore_error", err.to_string()))?;
            let matching: Vec<_> = keys
                .iter()
                .filter(|key| key.alias.as_deref() == Some(label))
                .collect();

            match matching.len() {
                0 => Err(CommandError::new(
                    "key_not_found",
                    format!("No key found with label: {label}"),
                )),
                1 => Ok(matching[0].id),
                _ => Err(CommandError::new(
                    "ambiguous_label",
                    format!("Multiple keys found with label: {label}"),
                )),
            }
        }
    }
}

fn write_raw_transaction_file(
    path: &Path,
    raw_tx_hex: &str,
    mode: OutputWriteMode,
) -> Result<(), CommandError> {
    let parent = validate_output_parent(path)?;
    validate_output_target(path, mode)?;
    let temp_path = temp_output_path(path, &parent)?;

    let write_result = write_temp_output_file(&temp_path, raw_tx_hex)
        .and_then(|()| install_temp_output_file(&temp_path, path, mode))
        .and_then(|()| sync_parent_directory(&parent));

    if write_result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }

    write_result
}

fn validate_output_parent(path: &Path) -> Result<PathBuf, CommandError> {
    let parent = path
        .parent()
        .filter(|candidate| !candidate.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let metadata = std::fs::symlink_metadata(parent).map_err(|err| {
        CommandError::new(
            "file_write_error",
            format!("Failed to inspect output parent directory: {err}"),
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(CommandError::new(
            "file_write_error",
            "Refusing symlinked output parent directory",
        ));
    }
    if !metadata.file_type().is_dir() {
        return Err(CommandError::new(
            "file_write_error",
            "Output parent path is not a directory",
        ));
    }

    #[cfg(unix)]
    {
        let mode = metadata.permissions().mode();
        let world_writable = (mode & 0o002) != 0;
        let sticky = (mode & 0o1000) != 0;
        if world_writable && !sticky {
            return Err(CommandError::new(
                "file_write_error",
                "Refusing unsafe output parent directory permissions",
            ));
        }
    }

    Ok(parent.to_path_buf())
}

fn validate_output_target(path: &Path, mode: OutputWriteMode) -> Result<(), CommandError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(CommandError::new(
                    "file_write_error",
                    "Refusing symlinked output file",
                ));
            }
            if !metadata.file_type().is_file() {
                return Err(CommandError::new(
                    "file_write_error",
                    "Output path is not a regular file",
                ));
            }
            if matches!(mode, OutputWriteMode::CreateNew) {
                return Err(CommandError::new(
                    "file_write_error",
                    "Output file already exists; pass --overwrite to replace it",
                ));
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(CommandError::new(
                "file_write_error",
                format!("Failed to inspect output file: {err}"),
            ));
        }
    }

    Ok(())
}

fn temp_output_path(path: &Path, parent: &Path) -> Result<PathBuf, CommandError> {
    let file_name = path.file_name().ok_or_else(|| {
        CommandError::new("file_write_error", "Output path must include a file name")
    })?;
    Ok(parent.join(format!(
        ".{}.tmp-{}",
        file_name.to_string_lossy(),
        Uuid::new_v4()
    )))
}

fn write_temp_output_file(path: &Path, raw_tx_hex: &str) -> Result<(), CommandError> {
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);

    #[cfg(unix)]
    {
        options.mode(0o600);
    }

    let mut file = options.open(path).map_err(|err| {
        CommandError::new(
            "file_write_error",
            format!("Failed to create temporary output file: {err}"),
        )
    })?;

    file.write_all(raw_tx_hex.as_bytes()).map_err(|err| {
        CommandError::new(
            "file_write_error",
            format!("Failed to write signed transaction file: {err}"),
        )
    })?;
    file.sync_all().map_err(|err| {
        CommandError::new(
            "file_write_error",
            format!("Failed to sync temporary output file: {err}"),
        )
    })?;

    Ok(())
}

fn install_temp_output_file(
    temp_path: &Path,
    path: &Path,
    mode: OutputWriteMode,
) -> Result<(), CommandError> {
    match mode {
        OutputWriteMode::CreateNew => {
            std::fs::hard_link(temp_path, path).map_err(|err| {
                CommandError::new(
                    "file_write_error",
                    format!("Failed to install new output file: {err}"),
                )
            })?;
            std::fs::remove_file(temp_path).map_err(|err| {
                CommandError::new(
                    "file_write_error",
                    format!("Failed to remove temporary output file: {err}"),
                )
            })?;
        }
        OutputWriteMode::Overwrite => {
            std::fs::rename(temp_path, path).map_err(|err| {
                CommandError::new(
                    "file_write_error",
                    format!("Failed to replace output file: {err}"),
                )
            })?;
        }
    }

    Ok(())
}

fn sync_parent_directory(parent: &Path) -> Result<(), CommandError> {
    #[cfg(unix)]
    {
        std::fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|err| {
                CommandError::new(
                    "file_write_error",
                    format!("Failed to sync output parent directory: {err}"),
                )
            })?;
    }

    let _ = parent;
    Ok(())
}

fn admin_error_from_keystore(err: KeystoreError) -> CommandError {
    match err {
        KeystoreError::InvalidPrivateKey => {
            CommandError::new("invalid_key_material", "Key material format is invalid")
        }
        KeystoreError::InvalidMnemonic(_) => {
            CommandError::new("invalid_recovery_phrase", "Recovery phrase is invalid")
        }
        KeystoreError::InvalidDerivationPath(_) => {
            CommandError::new("invalid_derivation_path", "Derivation path is invalid")
        }
        KeystoreError::KeyNotFound(_) => {
            CommandError::new("key_not_found", "Requested key does not exist")
        }
        _ => CommandError::new("keystore_error", err.to_string()),
    }
}

fn command_error_from_util(err: UtilError) -> CommandError {
    CommandError::new(err.code, err.message)
}

fn key_type_code(key_type: &KeyType) -> &'static str {
    match key_type {
        KeyType::PrivateKey => "privatekey",
        KeyType::HdDerived { .. } => "hd_derived",
    }
}
