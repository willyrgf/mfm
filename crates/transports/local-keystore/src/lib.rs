#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
//! Local keystore transport used by keystore administration and signing states.
//!
//! The transport bridges keystore-specific local side effects into the generic Live IO interface.
//! It intentionally keeps secrets in local process memory and avoids emitting secret-bearing
//! values in transport responses.
//! Mnemonic BIP-39 passphrases are sourced only through explicit request metadata: no passphrase,
//! hidden local prompt, or a hex-encoded UTF-8 file/FIFO path. File and FIFO reads strip trailing
//! `\n` and `\r\n` line endings; all other bytes are preserved as the passphrase.
//!
//! # Examples
//!
//! ```rust
//! use mfm_machine::live_io::LiveIoTransportFactory;
//! use mfm_transports_local_keystore::LocalKeystoreIoTransportFactory;
//!
//! let factory = LocalKeystoreIoTransportFactory;
//! assert_eq!(factory.namespace_group(), "local.keystore");
//! ```
#![warn(missing_docs)]

use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::Utc;
use mfm_collectors_local_keystore::{
    Bip39ExtraSource, KeystoreDeleteRequest, KeystoreImportRequest, KeystoreImportType,
    KeystoreListRequest, KeystoreListSortBy, KeystoreTxSignRequest,
    NAMESPACE_LOCAL_KEYSTORE_DELETE, NAMESPACE_LOCAL_KEYSTORE_IMPORT,
    NAMESPACE_LOCAL_KEYSTORE_LIST, NAMESPACE_LOCAL_KEYSTORE_TX_SIGN,
};
use mfm_core::keystore::{KeyType, Keystore, KeystoreConfig, KeystoreError};
use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::ids::ErrorCode;
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};
use mfm_state_keystore::states::admin::{
    KeystoreDeleteReport, KeystoreImportReport, KeystoreListKey, KeystoreListReport,
};
use mfm_state_keystore::tx::{
    parse_address, parse_data_hex, parse_u128_quantity, resolve_key_id, sign_eip1559_transaction,
    Eip1559TxToSign, KeystoreTxError,
};
use serde::de::DeserializeOwned;
use uuid::Uuid;
use zeroize::Zeroizing;

const ENV_KEYSTORE_PASSWORD_FILE: &str = "MFM_KEYSTORE_PASSWORD_FILE";
const ENV_KEYSTORE_PASSWORD: &str = "MFM_KEYSTORE_PASSWORD";
const ENV_INTEGRATION_TEST: &str = "MFM_INTEGRATION_TEST";

/// Transport factory for the `local.keystore` namespace group.
#[derive(Clone, Default)]
pub struct LocalKeystoreIoTransportFactory;

impl LiveIoTransportFactory for LocalKeystoreIoTransportFactory {
    fn namespace_group(&self) -> &str {
        "local.keystore"
    }

    fn make(&self, _env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(LocalKeystoreIoTransport)
    }
}

struct LocalKeystoreIoTransport;

#[async_trait]
impl LiveIoTransport for LocalKeystoreIoTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        tokio::task::spawn_blocking(move || dispatch_local_keystore_call(call))
            .await
            .map_err(|_| {
                io_other(
                    "local_transport_join_failed",
                    ErrorCategory::Unknown,
                    "local keystore transport worker failed",
                )
            })?
    }
}

type LocalError = LocalTransportError;

fn dispatch_local_keystore_call(call: IoCall) -> Result<serde_json::Value, IoError> {
    match call.namespace.as_str() {
        NAMESPACE_LOCAL_KEYSTORE_IMPORT => handle_keystore_import(call.request),
        NAMESPACE_LOCAL_KEYSTORE_LIST => handle_keystore_list(call.request),
        NAMESPACE_LOCAL_KEYSTORE_DELETE => handle_keystore_delete(call.request),
        NAMESPACE_LOCAL_KEYSTORE_TX_SIGN => handle_keystore_tx_sign(call.request),
        _ => Err(io_other(
            "unknown_namespace",
            ErrorCategory::Unknown,
            "unknown local keystore io namespace",
        )),
    }
}

fn handle_keystore_import(request: serde_json::Value) -> Result<serde_json::Value, IoError> {
    let req: KeystoreImportRequest = parse_request(request)?;
    let report = keystore_import(req).map_err(LocalError::into_io)?;
    encode_response(serde_json::json!(report))
}

fn handle_keystore_list(request: serde_json::Value) -> Result<serde_json::Value, IoError> {
    let req: KeystoreListRequest = parse_request(request)?;
    let report = keystore_list(req).map_err(LocalError::into_io)?;
    encode_response(serde_json::json!(report))
}

fn handle_keystore_delete(request: serde_json::Value) -> Result<serde_json::Value, IoError> {
    let req: KeystoreDeleteRequest = parse_request(request)?;
    let report = keystore_delete(req).map_err(LocalError::into_io)?;
    encode_response(serde_json::json!(report))
}

fn handle_keystore_tx_sign(request: serde_json::Value) -> Result<serde_json::Value, IoError> {
    let req: KeystoreTxSignRequest = parse_request(request)?;
    let report = keystore_tx_sign(req).map_err(LocalError::into_io)?;
    encode_response(report)
}

fn keystore_import(req: KeystoreImportRequest) -> Result<KeystoreImportReport, LocalError> {
    validate_import_request(&req)?;
    let path = PathBuf::from(decode_required_utf8(
        req.store_path,
        req.store_path_hex,
        "InvalidPathConfig",
        "store_path",
    )?);
    let label = decode_optional_utf8(req.label, req.label_hex, "InvalidPathConfig", "label")?;
    let material = match req.kind {
        KeystoreImportType::PrivateKey => read_input(
            if req.stdin_mode {
                ""
            } else {
                "Enter private key (hex): "
            },
            req.stdin_mode,
        )?,
        KeystoreImportType::Mnemonic => read_input(
            if req.stdin_mode {
                ""
            } else {
                "Enter mnemonic phrase: "
            },
            req.stdin_mode,
        )?,
    };

    match req.kind {
        KeystoreImportType::PrivateKey => {
            let normalized = normalize_private_key(&material)?;
            let mut ks = create_keystore_if_needed(&path)?;
            let label = label
                .unwrap_or_else(|| format!("imported-key-{}", Utc::now().format("%Y%m%d-%H%M%S")));
            let key_id = ks
                .import_private_key(Some(label), normalized.as_str())
                .map_err(admin_error_from_keystore)?;
            let key_info = load_key_info(&ks, key_id)?;
            Ok(KeystoreImportReport {
                id: key_info.id.to_string(),
                label: key_info.alias.unwrap_or_default(),
                key_type: "raw".to_string(),
                address: format!("{:?}", key_info.address),
                created_at: key_info.created_at.to_rfc3339(),
            })
        }
        KeystoreImportType::Mnemonic => {
            validate_mnemonic_basic(&material)?;
            let mut ks = create_keystore_if_needed(&path)?;
            let label = label
                .unwrap_or_else(|| format!("imported-hd-{}", Utc::now().format("%Y%m%d-%H%M%S")));
            let extra = read_bip39_extra(req.bip39_extra)?;
            let key_id = ks
                .import_mnemonic(
                    Some(label),
                    material.as_str(),
                    &req.derive_path,
                    extra.as_ref().map(|v| v.as_str()),
                )
                .map_err(admin_error_from_keystore)?;
            let key_info = load_key_info(&ks, key_id)?;
            Ok(KeystoreImportReport {
                id: key_info.id.to_string(),
                label: key_info.alias.unwrap_or_default(),
                key_type: "hd_derived".to_string(),
                address: format!("{:?}", key_info.address),
                created_at: key_info.created_at.to_rfc3339(),
            })
        }
    }
}

fn validate_import_request(req: &KeystoreImportRequest) -> Result<(), LocalError> {
    if !req.bip39_extra.is_none() && req.kind != KeystoreImportType::Mnemonic {
        return Err(LocalError::new(
            "InvalidImportConfig",
            ErrorCategory::ParsingInput,
            "BIP-39 extra input is only supported for mnemonic imports",
        ));
    }

    if req.stdin_mode && matches!(req.bip39_extra, Bip39ExtraSource::Prompt) {
        return Err(LocalError::new(
            "InvalidImportConfig",
            ErrorCategory::ParsingInput,
            "BIP-39 prompt input cannot be combined with stdin material",
        ));
    }

    Ok(())
}

fn read_bip39_extra(source: Bip39ExtraSource) -> Result<Option<Zeroizing<String>>, LocalError> {
    match source {
        Bip39ExtraSource::None => Ok(None),
        Bip39ExtraSource::Prompt => read_password("Enter BIP-39 passphrase: ").map(Some),
        Bip39ExtraSource::FilePathHex(path_hex) => {
            let path =
                decode_hex_utf8(&path_hex, "InvalidPathConfig", "bip39_extra.file_path_hex")?;
            read_secret_file(&path).map(Some)
        }
    }
}

fn keystore_list(req: KeystoreListRequest) -> Result<KeystoreListReport, LocalError> {
    let path = PathBuf::from(decode_required_utf8(
        req.store_path,
        req.store_path_hex,
        "InvalidPathConfig",
        "store_path",
    )?);
    let filter_label = decode_optional_utf8(
        req.filter_label,
        req.filter_label_hex,
        "InvalidPathConfig",
        "filter_label",
    )?;
    let keystore = load_unlocked_keystore(&path)?;
    let mut keys: Vec<KeystoreListKey> = keystore
        .list_keys()
        .map_err(admin_error_from_keystore)?
        .into_iter()
        .map(|key| KeystoreListKey {
            id: key.id.to_string(),
            label: key.alias.unwrap_or_else(|| "<no alias>".to_string()),
            key_type: key_type_code(&key.key_type).to_string(),
            address: if req.show_addrs {
                Some(format!("{:?}", key.address))
            } else {
                None
            },
            created: key.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        })
        .collect();

    if let Some(pattern) = filter_label.as_ref() {
        let regex = regex::Regex::new(pattern).map_err(|e| {
            LocalError::new(
                "InvalidRegex",
                ErrorCategory::ParsingInput,
                format!("Invalid regex pattern: {e}"),
            )
        })?;
        keys.retain(|k| regex.is_match(&k.label));
    }

    match req.sort_by {
        KeystoreListSortBy::Label => keys.sort_by(|a, b| a.label.cmp(&b.label)),
        KeystoreListSortBy::Created => keys.sort_by(|a, b| a.created.cmp(&b.created)),
        KeystoreListSortBy::Type => keys.sort_by(|a, b| a.key_type.cmp(&b.key_type)),
    }

    Ok(KeystoreListReport {
        keys,
        show_addresses: req.show_addrs,
    })
}

fn keystore_delete(req: KeystoreDeleteRequest) -> Result<KeystoreDeleteReport, LocalError> {
    let path = PathBuf::from(decode_required_utf8(
        req.store_path,
        req.store_path_hex,
        "InvalidPathConfig",
        "store_path",
    )?);
    let label = decode_optional_utf8(req.label, req.label_hex, "InvalidPathConfig", "label")?;
    let mut keystore = load_unlocked_keystore(&path)?;

    let key_id = if let Some(label) = &label {
        let keys = keystore.list_keys().map_err(admin_error_from_keystore)?;
        let matching_keys: Vec<_> = keys
            .iter()
            .filter(|k| k.alias.as_ref() == Some(label))
            .collect();
        match matching_keys.len() {
            0 => {
                return Err(LocalError::new(
                    "KeyNotFound",
                    ErrorCategory::ParsingInput,
                    format!("No key found with label: {label}"),
                ))
            }
            1 => matching_keys[0].id,
            _ => {
                return Err(LocalError::new(
                    "AmbiguousLabel",
                    ErrorCategory::ParsingInput,
                    format!("Multiple keys found with label: {label}"),
                ))
            }
        }
    } else if let Some(id_str) = &req.id {
        Uuid::parse_str(id_str).map_err(|_| {
            LocalError::new(
                "InvalidUuid",
                ErrorCategory::ParsingInput,
                "Invalid UUID format",
            )
        })?
    } else {
        return Err(LocalError::new(
            "MissingArgument",
            ErrorCategory::ParsingInput,
            "Must specify either key ID or --by-label",
        ));
    };

    let keys = keystore.list_keys().map_err(admin_error_from_keystore)?;
    let key_to_delete = keys.into_iter().find(|k| k.id == key_id).ok_or_else(|| {
        LocalError::new("KeyNotFound", ErrorCategory::ParsingInput, "Key not found")
    })?;

    if !req.confirm_yes {
        let label = key_to_delete.alias.as_deref().unwrap_or("<no alias>");
        let prompt = format!(
            "Are you sure you want to delete key '{}' (ID: {})?",
            label, key_to_delete.id
        );
        let confirmed = confirm(&prompt)?;
        if !confirmed {
            return Err(LocalError::new(
                "OperationCancelled",
                ErrorCategory::ParsingInput,
                "Deletion cancelled by user",
            ));
        }
    }

    keystore
        .delete_key(key_id)
        .map_err(admin_error_from_keystore)?;

    Ok(KeystoreDeleteReport {
        id: key_to_delete.id.to_string(),
        label: key_to_delete.alias.unwrap_or_default(),
    })
}

fn keystore_tx_sign(req: KeystoreTxSignRequest) -> Result<serde_json::Value, LocalError> {
    let path = PathBuf::from(decode_required_utf8(
        req.store_path,
        req.store_path_hex,
        "InvalidPathConfig",
        "store_path",
    )?);
    let label = decode_optional_utf8(req.label, req.label_hex, "InvalidSelectorLabel", "label")?;
    let out_path = PathBuf::from(decode_required_utf8(
        req.out_path,
        req.out_path_hex,
        "InvalidPathConfig",
        "out_path",
    )?);
    let mut keystore = load_unlocked_keystore(&path)?;

    let key_id = resolve_key_id(&keystore, req.id.as_deref(), label.as_deref())
        .map_err(local_error_from_keystore_tx)?;

    let tx = Eip1559TxToSign {
        to: parse_address(&req.to, "to").map_err(local_error_from_keystore_tx)?,
        value_wei: parse_u128_quantity(&req.value_wei, "value-wei")
            .map_err(local_error_from_keystore_tx)?,
        chain_id: req.chain_id,
        nonce: req.nonce,
        max_fee_per_gas: parse_u128_quantity(&req.max_fee_per_gas, "max-fee-per-gas")
            .map_err(local_error_from_keystore_tx)?,
        max_priority_fee_per_gas: parse_u128_quantity(
            &req.max_priority_fee_per_gas,
            "max-priority-fee-per-gas",
        )
        .map_err(local_error_from_keystore_tx)?,
        gas_limit: req.gas_limit,
        data: parse_data_hex(&req.data_hex).map_err(local_error_from_keystore_tx)?,
    };

    let signed = sign_eip1559_transaction(&mut keystore, key_id, &tx)
        .map_err(local_error_from_keystore_tx)?;
    write_raw_transaction_file(&out_path, &signed.raw_tx_hex)
        .map_err(local_error_from_keystore_tx)?;

    Ok(serde_json::json!({
        "from": signed.from,
        "to": format!("{:?}", tx.to),
        "nonce": tx.nonce,
        "chain_id": tx.chain_id,
        "tx_type": "0x2",
        "payload_hash": signed.payload_hash,
        "out_path": out_path.display().to_string(),
    }))
}

fn write_raw_transaction_file(path: &Path, raw_tx_hex: &str) -> Result<(), KeystoreTxError> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);

    #[cfg(unix)]
    {
        options.mode(0o600);
    }

    let mut file = options.open(path).map_err(|e| {
        KeystoreTxError::new(
            "FileWriteError",
            format!("Failed to open output file '{}': {e}", path.display()),
        )
    })?;

    file.write_all(raw_tx_hex.as_bytes()).map_err(|e| {
        KeystoreTxError::new(
            "FileWriteError",
            format!(
                "Failed to write signed transaction file '{}': {e}",
                path.display()
            ),
        )
    })?;

    #[cfg(unix)]
    {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| {
            KeystoreTxError::new(
                "FileWriteError",
                format!(
                    "Failed to set output file permissions '{}': {e}",
                    path.display()
                ),
            )
        })?;
    }

    Ok(())
}

fn create_keystore_if_needed(path: &Path) -> Result<Keystore, LocalError> {
    if path.exists() {
        return load_unlocked_keystore(path);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            LocalError::new(
                "KeystoreError",
                ErrorCategory::Unknown,
                format!(
                    "Failed to create keystore directory '{}': {e}",
                    parent.display()
                ),
            )
        })?;
    }

    let password = get_create_password()?;
    let cfg = if std::env::var(ENV_INTEGRATION_TEST).is_ok() {
        KeystoreConfig::insecure_integration_test()
    } else {
        KeystoreConfig::default()
    };

    let mut keystore = Keystore::new_with_config(path, cfg).map_err(|e| {
        LocalError::new(
            "KeystoreError",
            ErrorCategory::Unknown,
            format!("Failed to create keystore: {e}"),
        )
    })?;
    keystore.unlock(password.as_str()).map_err(|_| {
        LocalError::new(
            "KeystoreError",
            ErrorCategory::Unknown,
            "failed to unlock keystore",
        )
    })?;
    Ok(keystore)
}

fn load_unlocked_keystore(path: &Path) -> Result<Keystore, LocalError> {
    if !path.exists() {
        return Err(LocalError::new(
            "KeystoreError",
            ErrorCategory::Unknown,
            format!("Keystore not found at: {}", path.display()),
        ));
    }

    let mut keystore = Keystore::new(path)
        .map_err(|e| LocalError::new("KeystoreError", ErrorCategory::Unknown, e.to_string()))?;
    let password = get_unlock_password()?;
    keystore.unlock(password.as_str()).map_err(|_| {
        LocalError::new(
            "KeystoreError",
            ErrorCategory::Unknown,
            "invalid credential for keystore unlock",
        )
    })?;
    Ok(keystore)
}

fn load_key_info(
    keystore: &Keystore,
    key_id: Uuid,
) -> Result<mfm_core::keystore::KeyInfo, LocalError> {
    let keys = keystore.list_keys().map_err(admin_error_from_keystore)?;
    keys.into_iter().find(|k| k.id == key_id).ok_or_else(|| {
        LocalError::new(
            "KeyNotFound",
            ErrorCategory::ParsingInput,
            "Failed to retrieve imported key info",
        )
    })
}

fn get_unlock_password() -> Result<Zeroizing<String>, LocalError> {
    if let Some(password) = password_from_env_sources()? {
        return Ok(password);
    }
    read_password("Enter keystore password: ")
}

fn get_create_password() -> Result<Zeroizing<String>, LocalError> {
    if let Some(password) = password_from_env_sources()? {
        return Ok(password);
    }
    let password = read_password("Enter password for new keystore: ")?;
    let confirm_password = read_password("Confirm password: ")?;
    if password.as_str() != confirm_password.as_str() {
        return Err(LocalError::new(
            "KeystoreError",
            ErrorCategory::Unknown,
            "credential entries did not match",
        ));
    }
    Ok(password)
}

fn password_from_env_sources() -> Result<Option<Zeroizing<String>>, LocalError> {
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

fn read_password(prompt: &str) -> Result<Zeroizing<String>, LocalError> {
    print!("{prompt}");
    io::stdout()
        .flush()
        .map_err(|e| LocalError::new("InputError", ErrorCategory::Unknown, e.to_string()))?;
    let password = rpassword::read_password()
        .map_err(|e| LocalError::new("InputError", ErrorCategory::Unknown, e.to_string()))?;
    Ok(Zeroizing::new(password))
}

fn read_password_file(path: &str) -> Result<Zeroizing<String>, LocalError> {
    read_secret_file(path)
}

fn read_secret_file(path: &str) -> Result<Zeroizing<String>, LocalError> {
    let mut raw =
        Zeroizing::new(std::fs::read_to_string(path).map_err(|e| {
            LocalError::new("KeystoreError", ErrorCategory::Unknown, e.to_string())
        })?);
    trim_line_endings(&mut raw);
    if raw.is_empty() {
        return Err(LocalError::new(
            "KeystoreError",
            ErrorCategory::Unknown,
            "credential file was empty",
        ));
    }
    Ok(raw)
}

fn read_input(prompt: &str, from_stdin: bool) -> Result<Zeroizing<String>, LocalError> {
    if from_stdin {
        let mut input = Zeroizing::new(String::new());
        io::stdin()
            .read_to_string(&mut input)
            .map_err(|e| LocalError::new("InputError", ErrorCategory::Unknown, e.to_string()))?;
        finalize_stdin_secret_material(&mut input)?;
        return Ok(input);
    }

    print!("{prompt}");
    io::stdout()
        .flush()
        .map_err(|e| LocalError::new("InputError", ErrorCategory::Unknown, e.to_string()))?;
    let mut input = Zeroizing::new(String::new());
    io::stdin()
        .read_line(&mut input)
        .map_err(|e| LocalError::new("InputError", ErrorCategory::Unknown, e.to_string()))?;
    trim_line_endings(&mut input);
    Ok(input)
}

fn finalize_stdin_secret_material(input: &mut String) -> Result<(), LocalError> {
    trim_line_endings(input);
    if input.contains(['\r', '\n']) {
        return Err(LocalError::new(
            "InputError",
            ErrorCategory::ParsingInput,
            "stdin secret material must contain exactly one line",
        ));
    }
    Ok(())
}

fn trim_line_endings(input: &mut String) {
    while input.ends_with(['\r', '\n']) {
        input.pop();
    }
}

fn confirm(prompt: &str) -> Result<bool, LocalError> {
    loop {
        print!("{prompt} (y/N): ");
        io::stdout()
            .flush()
            .map_err(|e| LocalError::new("InputError", ErrorCategory::Unknown, e.to_string()))?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(|e| LocalError::new("InputError", ErrorCategory::Unknown, e.to_string()))?;
        match input.trim().to_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" | "" => return Ok(false),
            _ => {
                println!("Please enter 'y' or 'n'");
            }
        }
    }
}

fn normalize_private_key(input: &str) -> Result<Zeroizing<String>, LocalError> {
    let normalized = Zeroizing::new(input.strip_prefix("0x").unwrap_or(input).trim().to_string());
    if normalized.len() != 64 {
        return Err(LocalError::new(
            "InvalidKeyMaterial",
            ErrorCategory::ParsingInput,
            "Key material must be 64 hex characters",
        ));
    }
    if hex::decode(&normalized).is_err() {
        return Err(LocalError::new(
            "InvalidKeyMaterial",
            ErrorCategory::ParsingInput,
            "Key material must be valid hexadecimal",
        ));
    }
    Ok(normalized)
}

fn validate_mnemonic_basic(input: &str) -> Result<(), LocalError> {
    if input.split_whitespace().count() < 12 {
        return Err(LocalError::new(
            "InvalidRecoveryPhrase",
            ErrorCategory::ParsingInput,
            "Recovery phrase must have at least 12 words",
        ));
    }
    Ok(())
}

fn admin_error_from_keystore(err: KeystoreError) -> LocalError {
    match err {
        KeystoreError::InvalidPrivateKey => LocalError::new(
            "InvalidKeyMaterial",
            ErrorCategory::ParsingInput,
            "Key material format is invalid",
        ),
        KeystoreError::InvalidMnemonic(_) => LocalError::new(
            "InvalidRecoveryPhrase",
            ErrorCategory::ParsingInput,
            "Recovery phrase is invalid",
        ),
        KeystoreError::InvalidDerivationPath(_) => LocalError::new(
            "InvalidDerivationPath",
            ErrorCategory::ParsingInput,
            "Derivation path is invalid",
        ),
        KeystoreError::KeyNotFound(_) => LocalError::new(
            "KeyNotFound",
            ErrorCategory::ParsingInput,
            "Requested key does not exist",
        ),
        _ => LocalError::new("KeystoreError", ErrorCategory::Unknown, err.to_string()),
    }
}

fn key_type_code(key_type: &KeyType) -> &'static str {
    match key_type {
        KeyType::PrivateKey => "raw",
        KeyType::HdDerived { .. } => "hd_derived",
    }
}

fn local_error_from_keystore_tx(err: KeystoreTxError) -> LocalError {
    let category = match err.code {
        "InvalidAddress"
        | "InvalidQuantity"
        | "InvalidData"
        | "InvalidFeeConfig"
        | "InvalidRawTransaction"
        | "InvalidRpcUrl"
        | "InvalidPathConfig"
        | "InvalidSelectorLabel"
        | "InvalidUuid"
        | "MissingArgument"
        | "AmbiguousLabel"
        | "KeyNotFound" => ErrorCategory::ParsingInput,
        "RpcInvalidResponse" => ErrorCategory::Rpc,
        _ => ErrorCategory::Unknown,
    };
    LocalError::new(err.code, category, err.message)
}

#[derive(Debug, Clone)]
struct LocalTransportError {
    code: &'static str,
    category: ErrorCategory,
    message: String,
}

impl LocalTransportError {
    fn new(code: &'static str, category: ErrorCategory, message: impl Into<String>) -> Self {
        Self {
            code,
            category,
            message: message.into(),
        }
    }

    fn into_io(self) -> IoError {
        io_other(self.code, self.category, self.message)
    }
}

fn io_other(code: &'static str, category: ErrorCategory, message: impl Into<String>) -> IoError {
    IoError::Other(ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.into(),
        details: None,
    })
}

fn parse_request<T: DeserializeOwned>(request: serde_json::Value) -> Result<T, IoError> {
    serde_json::from_value(request).map_err(|_| {
        io_other(
            "invalid_local_request",
            ErrorCategory::ParsingInput,
            "invalid local io request payload",
        )
    })
}

fn encode_response(value: serde_json::Value) -> Result<serde_json::Value, IoError> {
    serde_json::to_value(value).map_err(|_| {
        io_other(
            "local_response_serialize_failed",
            ErrorCategory::Unknown,
            "failed to serialize local io response payload",
        )
    })
}

fn decode_hex_utf8(
    raw: &str,
    code: &'static str,
    field: &'static str,
) -> Result<String, LocalTransportError> {
    let bytes = hex::decode(raw).map_err(|_| {
        LocalTransportError::new(
            code,
            ErrorCategory::ParsingInput,
            format!("{field} must be valid hex"),
        )
    })?;
    String::from_utf8(bytes).map_err(|_| {
        LocalTransportError::new(
            code,
            ErrorCategory::ParsingInput,
            format!("{field} did not decode to utf-8"),
        )
    })
}

fn decode_optional_utf8(
    raw: Option<String>,
    raw_hex: Option<String>,
    code: &'static str,
    field: &'static str,
) -> Result<Option<String>, LocalTransportError> {
    match (raw, raw_hex) {
        (Some(_), Some(_)) => Err(LocalTransportError::new(
            code,
            ErrorCategory::ParsingInput,
            format!("{field} must use exactly one encoding"),
        )),
        (Some(value), None) => Ok(Some(value)),
        (None, Some(value_hex)) => decode_hex_utf8(&value_hex, code, field).map(Some),
        (None, None) => Ok(None),
    }
}

fn decode_required_utf8(
    raw: Option<String>,
    raw_hex: Option<String>,
    code: &'static str,
    field: &'static str,
) -> Result<String, LocalTransportError> {
    decode_optional_utf8(raw, raw_hex, code, field)?.ok_or_else(|| {
        LocalTransportError::new(
            code,
            ErrorCategory::ParsingInput,
            format!("{field} is required"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_request_rejects_legacy_passphrase_field() {
        let request = serde_json::json!({
            "kind": "mn",
            "derive_path": "m/44'/60'/0'/0/0",
            "store_path_hex": hex::encode("/tmp/keystore"),
            "stdin_mode": true,
            "passphrase": "do-not-accept"
        });

        let err = parse_request::<KeystoreImportRequest>(request)
            .expect_err("legacy secret-bearing request field must be rejected");

        match err {
            IoError::Other(info) => assert_eq!(info.code.0, "invalid_local_request"),
            other => panic!("unexpected io error: {other:?}"),
        }
    }

    #[test]
    fn stdin_secret_material_rejects_two_field_protocol() {
        let mut input = "field-one\nfield-two\n".to_string();

        let err = finalize_stdin_secret_material(&mut input)
            .expect_err("stdin import must not accept a second field");

        assert_eq!(err.code, "InputError");
        assert!(!err.message.contains("field-one"));
        assert!(!err.message.contains("field-two"));
    }

    #[test]
    fn stdin_secret_material_allows_single_trailing_newline() {
        let mut input = "abc123\r\n".to_string();

        finalize_stdin_secret_material(&mut input)
            .expect("single trailing line ending should be accepted");

        assert_eq!(input, "abc123");
    }
}
