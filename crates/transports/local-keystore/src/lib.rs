#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
//! Local keystore transport used by keystore administration and signing states.
//!
//! The transport bridges keystore-specific local side effects into the generic Live IO interface.
//! It intentionally keeps secrets in local process memory and avoids emitting secret-bearing
//! values in transport responses.
//! Mnemonic BIP-39 passphrases are sourced only through app-registered local resources: no
//! passphrase, hidden local prompt, or a local file/FIFO path. File and FIFO reads strip trailing
//! `\n` and `\r\n` line endings; all other bytes are preserved as the passphrase.
//! Interactive private-key, mnemonic, password, and passphrase prompts use hidden terminal input;
//! `stdin` imports are reserved for controlled pipes and files.
//! Blocking filesystem, terminal, and keystore work is isolated on Tokio's blocking pool before
//! returning sanitized IO errors to the async state-machine runtime.
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

use std::collections::HashMap;
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use async_trait::async_trait;
use chrono::Utc;
use mfm_collectors_local_keystore::tx::KeystoreTxError;
use mfm_collectors_local_keystore::{
    KeystoreDeleteReport, KeystoreDeleteRequest, KeystoreImportReport, KeystoreImportRequest,
    KeystoreImportType, KeystoreListKey, KeystoreListReport, KeystoreListRequest,
    KeystoreListSortBy, KeystoreTxSignRequest, LocalFileWriteMode, NAMESPACE_LOCAL_KEYSTORE_DELETE,
    NAMESPACE_LOCAL_KEYSTORE_IMPORT, NAMESPACE_LOCAL_KEYSTORE_LIST,
    NAMESPACE_LOCAL_KEYSTORE_TX_SIGN,
};
use mfm_core::keystore::{KeyType, Keystore, KeystoreConfig, KeystoreError};
use mfm_evm_core::tx::{
    eip1559_signing_hash, encode_signed_eip1559_tx_hex, parse_address, parse_data_hex,
    parse_u128_quantity, Eip1559TxToSign,
};
use mfm_evm_core::util_error::UtilError;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::ids::ErrorCode;
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};
use serde::de::DeserializeOwned;
use uuid::Uuid;
use zeroize::Zeroizing;

const ENV_KEYSTORE_PASSWORD_FILE: &str = "MFM_KEYSTORE_PASSWORD_FILE";
const ENV_KEYSTORE_PASSWORD: &str = "MFM_KEYSTORE_PASSWORD";
const ENV_INTEGRATION_TEST: &str = "MFM_INTEGRATION_TEST";

/// Local BIP-39 passphrase source registered for a mnemonic import.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LocalKeystoreBip39ExtraSource {
    /// Do not use a BIP-39 passphrase.
    #[default]
    None,
    /// Prompt for the BIP-39 passphrase inside the local keystore transport.
    Prompt,
    /// Read the BIP-39 passphrase from this local file or FIFO path.
    FilePath(PathBuf),
}

/// Local runtime inputs for a keystore import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalKeystoreImportResource {
    /// Filesystem path of the keystore directory.
    pub keystore_path: PathBuf,
    /// Optional alias to assign to the imported key.
    pub label: Option<String>,
    /// Whether the secret material should be read from stdin.
    pub stdin_mode: bool,
    /// Optional BIP-39 passphrase source for mnemonic imports.
    pub bip39_extra: LocalKeystoreBip39ExtraSource,
}

/// Local runtime inputs for a keystore list operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalKeystoreListResource {
    /// Filesystem path of the keystore directory.
    pub keystore_path: PathBuf,
    /// Optional regex filter applied to labels before sorting.
    pub filter_label: Option<String>,
}

/// Local runtime inputs for a keystore delete operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalKeystoreDeleteResource {
    /// Filesystem path of the keystore directory.
    pub keystore_path: PathBuf,
    /// Optional alias selector for the key to delete.
    pub by_label: Option<String>,
}

/// Local runtime inputs for a transaction signing operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalKeystoreTxSignResource {
    /// Filesystem path of the keystore directory.
    pub keystore_path: PathBuf,
    /// Optional alias selector for the signing key.
    pub by_label: Option<String>,
    /// Destination path for the signed raw transaction file.
    pub out_path: PathBuf,
}

/// Local keystore resource variants registered by app-level entrypoints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalKeystoreResource {
    /// Local runtime inputs for a keystore import.
    Import(LocalKeystoreImportResource),
    /// Local runtime inputs for a keystore list operation.
    List(LocalKeystoreListResource),
    /// Local runtime inputs for a keystore delete operation.
    Delete(LocalKeystoreDeleteResource),
    /// Local runtime inputs for a transaction signing operation.
    TxSign(LocalKeystoreTxSignResource),
}

static LOCAL_KEYSTORE_RESOURCES: OnceLock<Mutex<HashMap<String, LocalKeystoreResource>>> =
    OnceLock::new();

/// Registers a local keystore resource under an opaque handle for live execution.
pub fn register_local_keystore_resource(
    handle: impl Into<String>,
    resource: LocalKeystoreResource,
) {
    local_keystore_resources()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(handle.into(), resource);
}

fn local_keystore_resources() -> &'static Mutex<HashMap<String, LocalKeystoreResource>> {
    LOCAL_KEYSTORE_RESOURCES.get_or_init(|| Mutex::new(HashMap::new()))
}

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
        run_blocking_local_keystore(move || dispatch_local_keystore_call(call)).await
    }
}

type LocalError = LocalTransportError;

async fn run_blocking_local_keystore<F>(f: F) -> Result<serde_json::Value, IoError>
where
    F: FnOnce() -> Result<serde_json::Value, IoError> + Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(|_| {
        io_other(
            "local_transport_join_failed",
            ErrorCategory::Unknown,
            "local keystore transport worker failed",
        )
    })?
}

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
    let mut input = ProcessSecretInput;
    keystore_import_with_input(req, &mut input)
}

fn keystore_import_with_input(
    req: KeystoreImportRequest,
    input: &mut dyn SecretInput,
) -> Result<KeystoreImportReport, LocalError> {
    let resource = resolve_import_resource(&req.local_resource_handle)?;
    validate_import_resource(&req, &resource)?;
    let path = resource.keystore_path;
    let label = resource.label;
    let material = match req.kind {
        KeystoreImportType::PrivateKey => read_input(
            input,
            if resource.stdin_mode {
                ""
            } else {
                "Enter private key (hex): "
            },
            resource.stdin_mode,
        )?,
        KeystoreImportType::Mnemonic => read_input(
            input,
            if resource.stdin_mode {
                ""
            } else {
                "Enter mnemonic phrase: "
            },
            resource.stdin_mode,
        )?,
    };

    match req.kind {
        KeystoreImportType::PrivateKey => {
            let normalized = normalize_private_key(&material)?;
            let mut ks = create_keystore_if_needed_with_input(&path, input)?;
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
            let mut ks = create_keystore_if_needed_with_input(&path, input)?;
            let label = label
                .unwrap_or_else(|| format!("imported-hd-{}", Utc::now().format("%Y%m%d-%H%M%S")));
            let extra = read_bip39_extra(resource.bip39_extra, input)?;
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

fn validate_import_resource(
    req: &KeystoreImportRequest,
    resource: &LocalKeystoreImportResource,
) -> Result<(), LocalError> {
    if !matches!(resource.bip39_extra, LocalKeystoreBip39ExtraSource::None)
        && req.kind != KeystoreImportType::Mnemonic
    {
        return Err(LocalError::new(
            "invalid_import_config",
            ErrorCategory::ParsingInput,
            "BIP-39 extra input is only supported for mnemonic imports",
        ));
    }

    if resource.stdin_mode && matches!(resource.bip39_extra, LocalKeystoreBip39ExtraSource::Prompt)
    {
        return Err(LocalError::new(
            "invalid_import_config",
            ErrorCategory::ParsingInput,
            "BIP-39 prompt input cannot be combined with stdin material",
        ));
    }

    Ok(())
}

trait SecretInput {
    fn allow_env_password_sources(&self) -> bool {
        true
    }

    fn read_stdin_material(&mut self) -> Result<Zeroizing<String>, LocalError>;

    fn read_hidden(&mut self, prompt: &str) -> Result<Zeroizing<String>, LocalError>;
}

struct ProcessSecretInput;

impl SecretInput for ProcessSecretInput {
    fn read_stdin_material(&mut self) -> Result<Zeroizing<String>, LocalError> {
        read_stdin_material()
    }

    fn read_hidden(&mut self, prompt: &str) -> Result<Zeroizing<String>, LocalError> {
        read_password(prompt)
    }
}

fn read_bip39_extra(
    source: LocalKeystoreBip39ExtraSource,
    input: &mut dyn SecretInput,
) -> Result<Option<Zeroizing<String>>, LocalError> {
    match source {
        LocalKeystoreBip39ExtraSource::None => Ok(None),
        LocalKeystoreBip39ExtraSource::Prompt => {
            input.read_hidden("Enter BIP-39 passphrase: ").map(Some)
        }
        LocalKeystoreBip39ExtraSource::FilePath(path) => read_secret_file(&path).map(Some),
    }
}

fn keystore_list(req: KeystoreListRequest) -> Result<KeystoreListReport, LocalError> {
    let resource = resolve_list_resource(&req.local_resource_handle)?;
    let path = resource.keystore_path;
    let filter_label = resource.filter_label;
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
        let regex = regex::Regex::new(pattern).map_err(|_| {
            LocalError::new(
                "invalid_regex",
                ErrorCategory::ParsingInput,
                "Invalid regex pattern",
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
    let resource = resolve_delete_resource(&req.local_resource_handle)?;
    let path = resource.keystore_path;
    let label = resource.by_label;
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
                    "key_not_found",
                    ErrorCategory::ParsingInput,
                    "No key found with requested label",
                ))
            }
            1 => matching_keys[0].id,
            _ => {
                return Err(LocalError::new(
                    "ambiguous_label",
                    ErrorCategory::ParsingInput,
                    "Multiple keys found with requested label",
                ))
            }
        }
    } else if let Some(id_str) = &req.id {
        Uuid::parse_str(id_str).map_err(|_| {
            LocalError::new(
                "invalid_uuid",
                ErrorCategory::ParsingInput,
                "Invalid UUID format",
            )
        })?
    } else {
        return Err(LocalError::new(
            "missing_argument",
            ErrorCategory::ParsingInput,
            "Must specify either key ID or --by-label",
        ));
    };

    let keys = keystore.list_keys().map_err(admin_error_from_keystore)?;
    let key_to_delete = keys.into_iter().find(|k| k.id == key_id).ok_or_else(|| {
        LocalError::new(
            "key_not_found",
            ErrorCategory::ParsingInput,
            "Key not found",
        )
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
                "operation_cancelled",
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
    let mut input = ProcessSecretInput;
    keystore_tx_sign_with_input(req, &mut input)
}

fn keystore_tx_sign_with_input(
    req: KeystoreTxSignRequest,
    input: &mut dyn SecretInput,
) -> Result<serde_json::Value, LocalError> {
    let resource = resolve_tx_sign_resource(&req.local_resource_handle)?;
    let path = resource.keystore_path;
    let label = resource.by_label;
    let out_path = resource.out_path;
    let out_write_mode = req.out_write_mode;
    let mut keystore = load_unlocked_keystore_with_input(&path, input)?;

    let key_id = resolve_key_id(&keystore, req.id.as_deref(), label.as_deref())
        .map_err(local_error_from_keystore_tx)?;

    let tx = Eip1559TxToSign {
        to: parse_address(&req.to, "to")
            .map_err(keystore_tx_error_from_util)
            .map_err(local_error_from_keystore_tx)?,
        value_wei: parse_u128_quantity(&req.value_wei, "value-wei")
            .map_err(keystore_tx_error_from_util)
            .map_err(local_error_from_keystore_tx)?,
        chain_id: req.chain_id,
        nonce: req.nonce,
        max_fee_per_gas: parse_u128_quantity(&req.max_fee_per_gas, "max-fee-per-gas")
            .map_err(keystore_tx_error_from_util)
            .map_err(local_error_from_keystore_tx)?,
        max_priority_fee_per_gas: parse_u128_quantity(
            &req.max_priority_fee_per_gas,
            "max-priority-fee-per-gas",
        )
        .map_err(keystore_tx_error_from_util)
        .map_err(local_error_from_keystore_tx)?,
        gas_limit: req.gas_limit,
        data: parse_data_hex(&req.data_hex)
            .map_err(keystore_tx_error_from_util)
            .map_err(local_error_from_keystore_tx)?,
    };

    let signed = sign_eip1559_transaction(&mut keystore, key_id, &tx)
        .map_err(local_error_from_keystore_tx)?;
    write_raw_transaction_file(&out_path, &signed.raw_tx_hex, out_write_mode)
        .map_err(local_error_from_keystore_tx)?;

    Ok(serde_json::json!({
        "from": signed.from,
        "to": format!("{:?}", tx.to),
        "nonce": tx.nonce,
        "chain_id": tx.chain_id,
        "tx_type": "0x2",
        "payload_hash": signed.payload_hash,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SignedEip1559Tx {
    from: String,
    payload_hash: String,
    raw_tx_hex: String,
}

fn resolve_key_id(
    keystore: &Keystore,
    id: Option<&str>,
    by_label: Option<&str>,
) -> Result<Uuid, KeystoreTxError> {
    match (id, by_label) {
        (Some(_), Some(_)) => Err(KeystoreTxError::new(
            "missing_argument",
            "Specify exactly one key selector: --id or --by-label",
        )),
        (None, None) => Err(KeystoreTxError::new(
            "missing_argument",
            "Must specify one key selector: --id or --by-label",
        )),
        (Some(raw), None) => Uuid::parse_str(raw)
            .map_err(|_| KeystoreTxError::new("invalid_uuid", "Invalid UUID format")),
        (None, Some(label)) => {
            let keys = keystore
                .list_keys()
                .map_err(|e| KeystoreTxError::new("keystore_error", e.to_string()))?;
            let matching: Vec<_> = keys
                .iter()
                .filter(|key| key.alias.as_deref() == Some(label))
                .collect();

            match matching.len() {
                0 => Err(KeystoreTxError::new(
                    "key_not_found",
                    format!("No key found with label: {label}"),
                )),
                1 => Ok(matching[0].id),
                _ => Err(KeystoreTxError::new(
                    "ambiguous_label",
                    format!("Multiple keys found with label: {label}"),
                )),
            }
        }
    }
}

fn sign_eip1559_transaction(
    keystore: &mut Keystore,
    key_id: Uuid,
    tx: &Eip1559TxToSign,
) -> Result<SignedEip1559Tx, KeystoreTxError> {
    if tx.max_priority_fee_per_gas > tx.max_fee_per_gas {
        return Err(KeystoreTxError::new(
            "invalid_fee_config",
            "max-priority-fee-per-gas must be <= max-fee-per-gas",
        ));
    }

    let secure_key = keystore
        .get_private_key(key_id)
        .map_err(|e| KeystoreTxError::new("keystore_error", e.to_string()))?;
    let from_address = secure_key
        .ethereum_address()
        .map_err(|e| KeystoreTxError::new("signing_error", e.to_string()))?;

    let hash = eip1559_signing_hash(tx);
    let mut hash_bytes = [0u8; 32];
    hash_bytes.copy_from_slice(hash.as_slice());

    let signature = secure_key
        .sign_hash_recoverable(&hash_bytes)
        .map_err(|e| KeystoreTxError::new("signing_error", e.to_string()))?;

    Ok(SignedEip1559Tx {
        from: format!("{from_address:?}"),
        payload_hash: format!("0x{}", hex::encode(hash.as_slice())),
        raw_tx_hex: encode_signed_eip1559_tx_hex(tx, signature),
    })
}

fn write_raw_transaction_file(
    path: &Path,
    raw_tx_hex: &str,
    mode: LocalFileWriteMode,
) -> Result<(), KeystoreTxError> {
    let parent = validate_output_parent(path)?;
    validate_output_target(path, &mode)?;
    let temp_path = temp_output_path(path, &parent)?;

    let write_result = write_temp_output_file(&temp_path, raw_tx_hex)
        .and_then(|()| install_temp_output_file(&temp_path, path, mode))
        .and_then(|()| sync_parent_directory(&parent));

    if write_result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }

    write_result
}

fn validate_output_parent(path: &Path) -> Result<PathBuf, KeystoreTxError> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let metadata = std::fs::symlink_metadata(parent).map_err(|e| {
        KeystoreTxError::new(
            "file_write_error",
            format!("Failed to inspect output parent directory: {e}"),
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(KeystoreTxError::new(
            "file_write_error",
            "Refusing symlinked output parent directory",
        ));
    }
    if !metadata.file_type().is_dir() {
        return Err(KeystoreTxError::new(
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
            return Err(KeystoreTxError::new(
                "file_write_error",
                "Refusing unsafe output parent directory permissions",
            ));
        }
    }

    Ok(parent.to_path_buf())
}

fn validate_output_target(path: &Path, mode: &LocalFileWriteMode) -> Result<(), KeystoreTxError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(KeystoreTxError::new(
                    "file_write_error",
                    "Refusing symlinked output file",
                ));
            }
            if !metadata.file_type().is_file() {
                return Err(KeystoreTxError::new(
                    "file_write_error",
                    "Output path is not a regular file",
                ));
            }
            if matches!(mode, LocalFileWriteMode::CreateNew) {
                return Err(KeystoreTxError::new(
                    "file_write_error",
                    "Output file already exists; pass --overwrite to replace it",
                ));
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(KeystoreTxError::new(
                "file_write_error",
                format!("Failed to inspect output file: {err}"),
            ));
        }
    }

    Ok(())
}

fn temp_output_path(path: &Path, parent: &Path) -> Result<PathBuf, KeystoreTxError> {
    let file_name = path.file_name().ok_or_else(|| {
        KeystoreTxError::new("file_write_error", "Output path must include a file name")
    })?;
    Ok(parent.join(format!(
        ".{}.tmp-{}",
        file_name.to_string_lossy(),
        Uuid::new_v4()
    )))
}

fn write_temp_output_file(path: &Path, raw_tx_hex: &str) -> Result<(), KeystoreTxError> {
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);

    #[cfg(unix)]
    {
        options.mode(0o600);
    }

    let mut file = options.open(path).map_err(|e| {
        KeystoreTxError::new(
            "file_write_error",
            format!("Failed to create temporary output file: {e}"),
        )
    })?;

    file.write_all(raw_tx_hex.as_bytes()).map_err(|e| {
        KeystoreTxError::new(
            "file_write_error",
            format!("Failed to write signed transaction file: {e}"),
        )
    })?;
    file.sync_all().map_err(|e| {
        KeystoreTxError::new(
            "file_write_error",
            format!("Failed to sync temporary output file: {e}"),
        )
    })?;

    Ok(())
}

fn install_temp_output_file(
    temp_path: &Path,
    path: &Path,
    mode: LocalFileWriteMode,
) -> Result<(), KeystoreTxError> {
    match mode {
        LocalFileWriteMode::CreateNew => {
            std::fs::hard_link(temp_path, path).map_err(|e| {
                KeystoreTxError::new(
                    "file_write_error",
                    format!("Failed to install new output file: {e}"),
                )
            })?;
            std::fs::remove_file(temp_path).map_err(|e| {
                KeystoreTxError::new(
                    "file_write_error",
                    format!("Failed to remove temporary output file: {e}"),
                )
            })?;
        }
        LocalFileWriteMode::Overwrite => {
            std::fs::rename(temp_path, path).map_err(|e| {
                KeystoreTxError::new(
                    "file_write_error",
                    format!("Failed to replace output file: {e}"),
                )
            })?;
        }
    }

    Ok(())
}

fn sync_parent_directory(parent: &Path) -> Result<(), KeystoreTxError> {
    #[cfg(unix)]
    {
        std::fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| {
                KeystoreTxError::new(
                    "file_write_error",
                    format!("Failed to sync output parent directory: {e}"),
                )
            })?;
    }

    let _ = parent;
    Ok(())
}

fn create_keystore_if_needed_with_input(
    path: &Path,
    input: &mut dyn SecretInput,
) -> Result<Keystore, LocalError> {
    if path.exists() {
        return load_unlocked_keystore_with_input(path, input);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            LocalError::new(
                "keystore_error",
                ErrorCategory::Unknown,
                format!("Failed to create keystore directory: {e}"),
            )
        })?;
    }

    let password = get_create_password_with_input(input)?;
    let cfg = if std::env::var(ENV_INTEGRATION_TEST).is_ok() {
        KeystoreConfig::insecure_integration_test()
    } else {
        KeystoreConfig::default()
    };

    let mut keystore = Keystore::new_with_config(path, cfg).map_err(|e| {
        LocalError::new(
            "keystore_error",
            ErrorCategory::Unknown,
            format!("Failed to create keystore: {e}"),
        )
    })?;
    keystore.unlock(password.as_str()).map_err(|_| {
        LocalError::new(
            "keystore_error",
            ErrorCategory::Unknown,
            "failed to unlock keystore",
        )
    })?;
    Ok(keystore)
}

fn load_unlocked_keystore(path: &Path) -> Result<Keystore, LocalError> {
    let mut input = ProcessSecretInput;
    load_unlocked_keystore_with_input(path, &mut input)
}

fn load_unlocked_keystore_with_input(
    path: &Path,
    input: &mut dyn SecretInput,
) -> Result<Keystore, LocalError> {
    if !path.exists() {
        return Err(LocalError::new(
            "keystore_error",
            ErrorCategory::Unknown,
            "Keystore not found",
        ));
    }

    let mut keystore = Keystore::new(path)
        .map_err(|e| LocalError::new("keystore_error", ErrorCategory::Unknown, e.to_string()))?;
    let password = get_unlock_password_with_input(input)?;
    keystore.unlock(password.as_str()).map_err(|_| {
        LocalError::new(
            "keystore_error",
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
            "key_not_found",
            ErrorCategory::ParsingInput,
            "Failed to retrieve imported key info",
        )
    })
}

fn get_unlock_password_with_input(
    input: &mut dyn SecretInput,
) -> Result<Zeroizing<String>, LocalError> {
    if input.allow_env_password_sources() {
        if let Some(password) = password_from_env_sources()? {
            return Ok(password);
        }
    }
    input.read_hidden("Enter keystore password: ")
}

fn get_create_password_with_input(
    input: &mut dyn SecretInput,
) -> Result<Zeroizing<String>, LocalError> {
    if input.allow_env_password_sources() {
        if let Some(password) = password_from_env_sources()? {
            return Ok(password);
        }
    }
    let password = input.read_hidden("Enter password for new keystore: ")?;
    let confirm_password = input.read_hidden("Confirm password: ")?;
    if password.as_str() != confirm_password.as_str() {
        return Err(LocalError::new(
            "keystore_error",
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
        .map_err(|e| LocalError::new("input_error", ErrorCategory::Unknown, e.to_string()))?;
    let password = rpassword::read_password()
        .map_err(|e| LocalError::new("input_error", ErrorCategory::Unknown, e.to_string()))?;
    Ok(Zeroizing::new(password))
}

fn read_password_file(path: &str) -> Result<Zeroizing<String>, LocalError> {
    read_secret_file(Path::new(path))
}

fn read_secret_file(path: &Path) -> Result<Zeroizing<String>, LocalError> {
    let mut raw =
        Zeroizing::new(std::fs::read_to_string(path).map_err(|e| {
            LocalError::new("keystore_error", ErrorCategory::Unknown, e.to_string())
        })?);
    trim_line_endings(&mut raw);
    if raw.is_empty() {
        return Err(LocalError::new(
            "keystore_error",
            ErrorCategory::Unknown,
            "credential file was empty",
        ));
    }
    Ok(raw)
}

fn read_input(
    input: &mut dyn SecretInput,
    prompt: &str,
    from_stdin: bool,
) -> Result<Zeroizing<String>, LocalError> {
    if from_stdin {
        return input.read_stdin_material();
    }

    input.read_hidden(prompt)
}

fn read_stdin_material() -> Result<Zeroizing<String>, LocalError> {
    let mut input = Zeroizing::new(String::new());
    io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| LocalError::new("input_error", ErrorCategory::Unknown, e.to_string()))?;
    finalize_stdin_secret_material(&mut input)?;
    Ok(input)
}

fn finalize_stdin_secret_material(input: &mut String) -> Result<(), LocalError> {
    trim_line_endings(input);
    if input.contains(['\r', '\n']) {
        return Err(LocalError::new(
            "input_error",
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
            .map_err(|e| LocalError::new("input_error", ErrorCategory::Unknown, e.to_string()))?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(|e| LocalError::new("input_error", ErrorCategory::Unknown, e.to_string()))?;
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
            "invalid_key_material",
            ErrorCategory::ParsingInput,
            "Key material must be 64 hex characters",
        ));
    }
    if hex::decode(&normalized).is_err() {
        return Err(LocalError::new(
            "invalid_key_material",
            ErrorCategory::ParsingInput,
            "Key material must be valid hexadecimal",
        ));
    }
    Ok(normalized)
}

fn validate_mnemonic_basic(input: &str) -> Result<(), LocalError> {
    if input.split_whitespace().count() < 12 {
        return Err(LocalError::new(
            "invalid_recovery_phrase",
            ErrorCategory::ParsingInput,
            "Recovery phrase must have at least 12 words",
        ));
    }
    Ok(())
}

fn admin_error_from_keystore(err: KeystoreError) -> LocalError {
    match err {
        KeystoreError::InvalidPrivateKey => LocalError::new(
            "invalid_key_material",
            ErrorCategory::ParsingInput,
            "Key material format is invalid",
        ),
        KeystoreError::InvalidMnemonic(_) => LocalError::new(
            "invalid_recovery_phrase",
            ErrorCategory::ParsingInput,
            "Recovery phrase is invalid",
        ),
        KeystoreError::InvalidDerivationPath(_) => LocalError::new(
            "invalid_derivation_path",
            ErrorCategory::ParsingInput,
            "Derivation path is invalid",
        ),
        KeystoreError::KeyNotFound(_) => LocalError::new(
            "key_not_found",
            ErrorCategory::ParsingInput,
            "Requested key does not exist",
        ),
        _ => LocalError::new("keystore_error", ErrorCategory::Unknown, err.to_string()),
    }
}

fn key_type_code(key_type: &KeyType) -> &'static str {
    match key_type {
        KeyType::PrivateKey => "raw",
        KeyType::HdDerived { .. } => "hd_derived",
    }
}

fn local_error_from_keystore_tx(err: KeystoreTxError) -> LocalError {
    let code = match err.code {
        "invalid_address" => "invalid_address",
        "invalid_quantity" => "invalid_quantity",
        "invalid_data" => "invalid_data",
        "invalid_fee_config" => "invalid_fee_config",
        "invalid_raw_transaction" => "invalid_raw_transaction",
        "invalid_rpc_url" => "invalid_rpc_url",
        "invalid_path_config" => "invalid_path_config",
        "invalid_selector_label" => "invalid_selector_label",
        "invalid_uuid" => "invalid_uuid",
        "missing_argument" => "missing_argument",
        "ambiguous_label" => "ambiguous_label",
        "key_not_found" => "key_not_found",
        "rpc_invalid_response" => "rpc_invalid_response",
        "keystore_error" => "keystore_error",
        "signing_error" => "signing_error",
        "file_write_error" => "file_write_error",
        _ => "keystore_tx_error",
    };
    let category = match code {
        "invalid_address"
        | "invalid_quantity"
        | "invalid_data"
        | "invalid_fee_config"
        | "invalid_raw_transaction"
        | "invalid_rpc_url"
        | "invalid_path_config"
        | "invalid_selector_label"
        | "invalid_uuid"
        | "missing_argument"
        | "ambiguous_label"
        | "key_not_found" => ErrorCategory::ParsingInput,
        "rpc_invalid_response" => ErrorCategory::Rpc,
        _ => ErrorCategory::Unknown,
    };
    LocalError::new(code, category, err.message)
}

fn keystore_tx_error_from_util(err: UtilError) -> KeystoreTxError {
    KeystoreTxError::new(err.code, err.message)
}

fn resolve_resource(handle: &str) -> Result<LocalKeystoreResource, LocalTransportError> {
    if handle.trim().is_empty() {
        return Err(LocalTransportError::new(
            "local_resource_not_found",
            ErrorCategory::ParsingInput,
            "local keystore resource handle was not registered",
        ));
    }

    local_keystore_resources()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(handle)
        .cloned()
        .ok_or_else(|| {
            LocalTransportError::new(
                "local_resource_not_found",
                ErrorCategory::ParsingInput,
                "local keystore resource handle was not registered",
            )
        })
}

fn resolve_import_resource(
    handle: &str,
) -> Result<LocalKeystoreImportResource, LocalTransportError> {
    match resolve_resource(handle)? {
        LocalKeystoreResource::Import(resource) => Ok(resource),
        _ => Err(local_resource_kind_mismatch()),
    }
}

fn resolve_list_resource(handle: &str) -> Result<LocalKeystoreListResource, LocalTransportError> {
    match resolve_resource(handle)? {
        LocalKeystoreResource::List(resource) => Ok(resource),
        _ => Err(local_resource_kind_mismatch()),
    }
}

fn resolve_delete_resource(
    handle: &str,
) -> Result<LocalKeystoreDeleteResource, LocalTransportError> {
    match resolve_resource(handle)? {
        LocalKeystoreResource::Delete(resource) => Ok(resource),
        _ => Err(local_resource_kind_mismatch()),
    }
}

fn resolve_tx_sign_resource(
    handle: &str,
) -> Result<LocalKeystoreTxSignResource, LocalTransportError> {
    match resolve_resource(handle)? {
        LocalKeystoreResource::TxSign(resource) => Ok(resource),
        _ => Err(local_resource_kind_mismatch()),
    }
}

fn local_resource_kind_mismatch() -> LocalTransportError {
    LocalTransportError::new(
        "local_resource_kind_mismatch",
        ErrorCategory::ParsingInput,
        "local keystore resource handle had the wrong operation type",
    )
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
        code: ErrorCode::must_new(code),
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

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::VecDeque;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use std::time::Duration;

    const TEST_PASSWORD: &str = "CorrectHorseBatteryStaple123!";
    const TEST_PRIVATE_KEY: &str =
        "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
    const TEST_MNEMONIC: &str =
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[derive(Default)]
    struct FakeSecretInput {
        stdin_values: VecDeque<String>,
        hidden_values: VecDeque<String>,
        hidden_prompts: Vec<String>,
        stdin_reads: usize,
    }

    impl FakeSecretInput {
        fn with_stdin(mut self, value: impl Into<String>) -> Self {
            self.stdin_values.push_back(value.into());
            self
        }

        fn with_hidden(mut self, value: impl Into<String>) -> Self {
            self.hidden_values.push_back(value.into());
            self
        }
    }

    impl SecretInput for FakeSecretInput {
        fn allow_env_password_sources(&self) -> bool {
            false
        }

        fn read_stdin_material(&mut self) -> Result<Zeroizing<String>, LocalError> {
            self.stdin_reads += 1;
            let mut value = self.stdin_values.pop_front().ok_or_else(|| {
                LocalError::new(
                    "input_error",
                    ErrorCategory::Unknown,
                    "missing fake stdin input",
                )
            })?;
            finalize_stdin_secret_material(&mut value)?;
            Ok(Zeroizing::new(value))
        }

        fn read_hidden(&mut self, prompt: &str) -> Result<Zeroizing<String>, LocalError> {
            self.hidden_prompts.push(prompt.to_string());
            let value = self.hidden_values.pop_front().ok_or_else(|| {
                LocalError::new(
                    "input_error",
                    ErrorCategory::Unknown,
                    "missing fake hidden input",
                )
            })?;
            Ok(Zeroizing::new(value))
        }
    }

    fn test_keystore_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .canonicalize()
            .expect("canonical temp dir")
            .join(format!("mfm-{name}-{}.keystore", Uuid::new_v4()))
    }

    fn test_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir()
            .canonicalize()
            .expect("canonical temp dir")
            .join(format!("mfm-{name}-{}", Uuid::new_v4()));
        std::fs::create_dir(&path).expect("create test directory");
        path
    }

    fn create_test_keystore(path: &Path) {
        let mut keystore =
            Keystore::new_with_config(path, KeystoreConfig::insecure_integration_test())
                .expect("create test keystore");
        keystore
            .unlock(TEST_PASSWORD)
            .expect("unlock test keystore");
    }

    fn create_test_keystore_with_private_key(path: &Path, label: &str) {
        let mut keystore =
            Keystore::new_with_config(path, KeystoreConfig::insecure_integration_test())
                .expect("create test keystore");
        keystore
            .unlock(TEST_PASSWORD)
            .expect("unlock test keystore");
        keystore
            .import_private_key(Some(label.to_string()), TEST_PRIVATE_KEY)
            .expect("import test private key");
    }

    fn test_handle(name: &str) -> String {
        format!("local-keystore:test:{name}:{}", Uuid::new_v4())
    }

    fn tx_sign_request(
        keystore_path: &Path,
        out_path: &Path,
        mode: LocalFileWriteMode,
    ) -> KeystoreTxSignRequest {
        let handle = test_handle("tx-sign");
        register_local_keystore_resource(
            handle.clone(),
            LocalKeystoreResource::TxSign(LocalKeystoreTxSignResource {
                keystore_path: keystore_path.to_path_buf(),
                by_label: Some("tx-signer".to_string()),
                out_path: out_path.to_path_buf(),
            }),
        );
        KeystoreTxSignRequest {
            id: None,
            local_resource_handle: handle,
            out_write_mode: mode,
            to: "0x1111111111111111111111111111111111111111".to_string(),
            value_wei: "1".to_string(),
            chain_id: 1,
            nonce: 0,
            max_fee_per_gas: "2000000000".to_string(),
            max_priority_fee_per_gas: "1000000000".to_string(),
            gas_limit: 21_000,
            data_hex: "0x".to_string(),
        }
    }

    #[test]
    fn import_request_rejects_legacy_passphrase_field() {
        let request = serde_json::json!({
            "kind": "mn",
            "derive_path": "m/44'/60'/0'/0/0",
            "local_resource_handle": "local-keystore:test",
            "passphrase": "do-not-accept"
        });

        let err = parse_request::<KeystoreImportRequest>(request)
            .expect_err("legacy secret-bearing request field must be rejected");

        match err {
            IoError::Other(info) => assert_eq!(info.code.as_str(), "invalid_local_request"),
            other => panic!("unexpected io error: {other:?}"),
        }
    }

    #[test]
    fn unregistered_local_resource_handle_returns_sanitized_error() {
        let mut input = FakeSecretInput::default().with_hidden(TEST_PRIVATE_KEY);

        let err = keystore_import_with_input(
            KeystoreImportRequest {
                kind: KeystoreImportType::PrivateKey,
                derive_path: "m/44'/60'/0'/0/0".to_string(),
                local_resource_handle: "local-keystore:missing".to_string(),
            },
            &mut input,
        )
        .expect_err("missing local resource handle should fail before prompting");

        assert_eq!(err.code, "local_resource_not_found");
        assert!(!err.message.contains("local-keystore:missing"));
        assert_eq!(input.hidden_prompts.len(), 0);
    }

    #[tokio::test]
    async fn join_failure_maps_to_sanitized_io_error() {
        let err = run_blocking_local_keystore(|| panic!("secret panic payload"))
            .await
            .expect_err("panic should map to io error");

        match err {
            IoError::Other(info) => {
                assert_eq!(info.code.as_str(), "local_transport_join_failed");
                assert!(!info.message.contains("secret panic payload"));
                assert!(info.details.is_none());
            }
            other => panic!("unexpected io error: {other:?}"),
        }
    }

    #[test]
    fn current_thread_runtime_progresses_while_local_keystore_work_is_blocking() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let progressed = Arc::new(AtomicBool::new(false));
            let progressed_task = Arc::clone(&progressed);
            let blocking = run_blocking_local_keystore(|| {
                std::thread::sleep(Duration::from_millis(75));
                Ok(serde_json::json!({"ok": true}))
            });
            let progress = async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                progressed_task.store(true, Ordering::SeqCst);
            };

            let (blocking_result, ()) = tokio::join!(blocking, progress);

            blocking_result.expect("blocking work should finish");
            assert!(progressed.load(Ordering::SeqCst));
        });
    }

    #[test]
    fn stdin_secret_material_rejects_two_field_protocol() {
        let mut input = "field-one\nfield-two\n".to_string();

        let err = finalize_stdin_secret_material(&mut input)
            .expect_err("stdin import must not accept a second field");

        assert_eq!(err.code, "input_error");
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

    #[test]
    fn read_input_interactive_uses_hidden_reader() {
        let mut input = FakeSecretInput::default().with_hidden("hidden-value");

        let material = read_input(&mut input, "Enter private key (hex): ", false)
            .expect("interactive input should use hidden reader");

        assert_eq!(material.as_str(), "hidden-value");
        assert_eq!(input.hidden_prompts, vec!["Enter private key (hex): "]);
        assert_eq!(input.stdin_reads, 0);
    }

    #[test]
    fn read_input_stdin_uses_noninteractive_reader() {
        let mut input = FakeSecretInput::default().with_stdin("stdin-value\n");

        let material =
            read_input(&mut input, "", true).expect("stdin input should use noninteractive reader");

        assert_eq!(material.as_str(), "stdin-value");
        assert!(input.hidden_prompts.is_empty());
        assert_eq!(input.stdin_reads, 1);
    }

    #[test]
    fn interactive_private_key_import_uses_hidden_reader() {
        let path = test_keystore_path("interactive-private-key");
        create_test_keystore(&path);
        let handle = test_handle("interactive-private-key");
        register_local_keystore_resource(
            handle.clone(),
            LocalKeystoreResource::Import(LocalKeystoreImportResource {
                keystore_path: path.clone(),
                label: Some("interactive-pk".to_string()),
                stdin_mode: false,
                bip39_extra: LocalKeystoreBip39ExtraSource::None,
            }),
        );
        let mut input = FakeSecretInput::default()
            .with_hidden(TEST_PRIVATE_KEY)
            .with_hidden(TEST_PASSWORD);

        let report = keystore_import_with_input(
            KeystoreImportRequest {
                kind: KeystoreImportType::PrivateKey,
                derive_path: "m/44'/60'/0'/0/0".to_string(),
                local_resource_handle: handle,
            },
            &mut input,
        )
        .expect("interactive private key import should succeed");

        assert_eq!(report.key_type, "raw");
        assert_eq!(
            input.hidden_prompts,
            vec!["Enter private key (hex): ", "Enter keystore password: "]
        );
        assert_eq!(input.stdin_reads, 0);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn interactive_mnemonic_import_uses_hidden_reader_for_phrase_and_extra() {
        let path = test_keystore_path("interactive-mnemonic");
        create_test_keystore(&path);
        let handle = test_handle("interactive-mnemonic");
        register_local_keystore_resource(
            handle.clone(),
            LocalKeystoreResource::Import(LocalKeystoreImportResource {
                keystore_path: path.clone(),
                label: Some("interactive-mn".to_string()),
                stdin_mode: false,
                bip39_extra: LocalKeystoreBip39ExtraSource::Prompt,
            }),
        );
        let mut input = FakeSecretInput::default()
            .with_hidden(TEST_MNEMONIC)
            .with_hidden(TEST_PASSWORD)
            .with_hidden("test extra input");

        let report = keystore_import_with_input(
            KeystoreImportRequest {
                kind: KeystoreImportType::Mnemonic,
                derive_path: "m/44'/60'/0'/0/0".to_string(),
                local_resource_handle: handle,
            },
            &mut input,
        )
        .expect("interactive mnemonic import should succeed");

        assert_eq!(report.key_type, "hd_derived");
        assert_eq!(
            input.hidden_prompts,
            vec![
                "Enter mnemonic phrase: ",
                "Enter keystore password: ",
                "Enter BIP-39 passphrase: "
            ]
        );
        assert_eq!(input.stdin_reads, 0);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn invalid_interactive_material_error_does_not_include_entered_secret() {
        let path = test_keystore_path("invalid-interactive-material");
        let entered = "not-a-valid-private-key";
        let handle = test_handle("invalid-interactive-material");
        register_local_keystore_resource(
            handle.clone(),
            LocalKeystoreResource::Import(LocalKeystoreImportResource {
                keystore_path: path.clone(),
                label: None,
                stdin_mode: false,
                bip39_extra: LocalKeystoreBip39ExtraSource::None,
            }),
        );
        let mut input = FakeSecretInput::default().with_hidden(entered);

        let err = keystore_import_with_input(
            KeystoreImportRequest {
                kind: KeystoreImportType::PrivateKey,
                derive_path: "m/44'/60'/0'/0/0".to_string(),
                local_resource_handle: handle,
            },
            &mut input,
        )
        .expect_err("invalid key material should fail");

        assert!(!err.message.contains(entered));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn safe_writer_creates_new_file_with_restrictive_permissions() {
        let dir = test_dir("safe-writer-new");
        let path = dir.join("signed.tx");

        write_raw_transaction_file(&path, "0xabc", LocalFileWriteMode::CreateNew)
            .expect("write raw tx");

        assert_eq!(
            std::fs::read_to_string(&path).expect("read output"),
            "0xabc"
        );
        #[cfg(unix)]
        {
            let mode = std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn safe_writer_rejects_existing_file_without_overwrite() {
        let dir = test_dir("safe-writer-existing");
        let path = dir.join("signed.tx");
        std::fs::write(&path, "old").expect("write old output");

        let err = write_raw_transaction_file(&path, "0xabc", LocalFileWriteMode::CreateNew)
            .expect_err("existing file should require overwrite");

        assert_eq!(err.code, "file_write_error");
        assert!(!err.message.contains(&path.display().to_string()));
        assert_eq!(std::fs::read_to_string(&path).expect("read output"), "old");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn safe_writer_overwrites_existing_regular_file_when_requested() {
        let dir = test_dir("safe-writer-overwrite");
        let path = dir.join("signed.tx");
        std::fs::write(&path, "old").expect("write old output");

        write_raw_transaction_file(&path, "0xabc", LocalFileWriteMode::Overwrite)
            .expect("overwrite raw tx");

        assert_eq!(
            std::fs::read_to_string(&path).expect("read output"),
            "0xabc"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn safe_writer_rejects_symlink_target_without_following_it() {
        let dir = test_dir("safe-writer-symlink-target");
        let target = dir.join("target.txt");
        let link = dir.join("signed.tx");
        std::fs::write(&target, "keep").expect("write target");
        std::os::unix::fs::symlink(&target, &link).expect("create symlink");

        let err = write_raw_transaction_file(&link, "0xabc", LocalFileWriteMode::Overwrite)
            .expect_err("symlink output target must be rejected");

        assert_eq!(err.code, "file_write_error");
        assert_eq!(
            std::fs::read_to_string(&target).expect("read target"),
            "keep"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn safe_writer_rejects_symlink_parent_directory() {
        let dir = test_dir("safe-writer-symlink-parent");
        let real_parent = dir.join("real");
        let linked_parent = dir.join("linked");
        std::fs::create_dir(&real_parent).expect("create real parent");
        std::os::unix::fs::symlink(&real_parent, &linked_parent).expect("create parent symlink");

        let err = write_raw_transaction_file(
            &linked_parent.join("signed.tx"),
            "0xabc",
            LocalFileWriteMode::CreateNew,
        )
        .expect_err("symlink output parent must be rejected");

        assert_eq!(err.code, "file_write_error");
        assert!(!real_parent.join("signed.tx").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn safe_writer_rejects_world_writable_non_sticky_parent() {
        let dir = test_dir("safe-writer-unsafe-parent");
        let parent = dir.join("unsafe");
        std::fs::create_dir(&parent).expect("create parent");
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o777))
            .expect("set unsafe permissions");

        let err = write_raw_transaction_file(
            &parent.join("signed.tx"),
            "0xabc",
            LocalFileWriteMode::CreateNew,
        )
        .expect_err("unsafe parent permissions must be rejected");

        assert_eq!(err.code, "file_write_error");
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700))
            .expect("restore permissions");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tx_sign_writes_new_output_with_safe_writer() {
        let dir = test_dir("tx-sign-safe-writer");
        let keystore_path = dir.join("signer.keystore");
        let out_path = dir.join("signed.tx");
        create_test_keystore_with_private_key(&keystore_path, "tx-signer");
        let mut input = FakeSecretInput::default().with_hidden(TEST_PASSWORD);

        let report = keystore_tx_sign_with_input(
            tx_sign_request(&keystore_path, &out_path, LocalFileWriteMode::CreateNew),
            &mut input,
        )
        .expect("tx-sign should write output");

        assert_eq!(report["tx_type"].as_str(), Some("0x2"));
        let raw_tx = std::fs::read_to_string(&out_path).expect("read signed tx");
        assert!(raw_tx.starts_with("0x02"));
        let rendered_report = serde_json::to_string(&report).expect("serialize report");
        assert!(report.get("raw_tx_hex").is_none());
        assert!(report.get("out_path").is_none());
        assert!(!rendered_report.contains(&raw_tx));
        assert!(!rendered_report.contains(TEST_PRIVATE_KEY));
        assert!(!rendered_report.contains(TEST_PASSWORD));
        assert!(!rendered_report.contains("signature"));
        #[cfg(unix)]
        {
            let mode = std::fs::metadata(&out_path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
