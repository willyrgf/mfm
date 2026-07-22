use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::Utc;
use mfm_keystore::{Keystore, KeystoreConfig, KeystoreError};
use mfm_runtime_config::{KeystoreRef, RuntimeConfig, RuntimeConfigRequirement};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::{PublicError, MFM_RUNTIME_CONFIG_FILE};

const DEFAULT_KEYSTORE_REF: &str = "default";
const MAX_RUNTIME_PATH_BYTES: usize = 4_096;
const MAX_SECRET_FILE_BYTES: usize = 64 * 1_024;

pub use mfm_keystore::{KeyInfo as KeystoreKeyMetadata, KeyType as KeystoreKeyType};

/// Consuming binary-to-app secret input.
///
/// The type deliberately implements no cloning, formatting, serialization, borrowing, or string
/// accessor contract. Application services consume it on their blocking worker.
pub struct SecretInput(Zeroizing<String>);

impl SecretInput {
    /// Moves one caller-owned string immediately into zeroizing storage.
    pub fn new(secret: String) -> Self {
        Self(Zeroizing::new(secret))
    }

    fn from_zeroizing(secret: Zeroizing<String>) -> Self {
        Self(secret)
    }

    fn into_zeroizing(self) -> Zeroizing<String> {
        self.0
    }
}

/// Opaque raw selection for one keystore command.
pub struct KeystoreSelection(SelectionKind);

enum SelectionKind {
    Explicit(PathBuf),
    RuntimeProfile {
        runtime_config_path: Option<PathBuf>,
        keystore_ref: Option<String>,
    },
}

impl KeystoreSelection {
    /// Selects one explicit local keystore path.
    pub fn explicit(path: PathBuf) -> Self {
        Self(SelectionKind::Explicit(path))
    }

    /// Selects a path-only keystore profile from runtime configuration.
    pub fn runtime_profile(
        runtime_config_path: Option<PathBuf>,
        keystore_ref: Option<String>,
    ) -> Self {
        Self(SelectionKind::RuntimeProfile {
            runtime_config_path,
            keystore_ref,
        })
    }
}

/// Safe prompt requirement returned for an opaque prepared keystore selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeystoreCredentialRequirement {
    /// Prompt once to unlock an existing explicit keystore.
    Unlock,
    /// Prompt and confirm the credential for a new explicit keystore.
    Create,
    /// The selected runtime profile supplies a process-local unlock file.
    Managed,
}

/// Opaque resolved selection awaiting its one credential binding.
pub struct PreparedKeystoreAccess {
    path: PathBuf,
    credential: PreparedCredential,
    create: bool,
}

enum PreparedCredential {
    OneShot,
    File(PathBuf),
}

impl PreparedKeystoreAccess {
    /// Returns the transport-safe prompt requirement for this selection.
    pub const fn credential_requirement(&self) -> KeystoreCredentialRequirement {
        match (&self.credential, self.create) {
            (PreparedCredential::OneShot, false) => KeystoreCredentialRequirement::Unlock,
            (PreparedCredential::OneShot, true) => KeystoreCredentialRequirement::Create,
            (PreparedCredential::File(_), _) => KeystoreCredentialRequirement::Managed,
        }
    }

    /// Consumes an explicit selection and binds its one-shot credential.
    pub fn with_secret(self, secret: SecretInput) -> Result<KeystoreAccess, PublicError> {
        if !matches!(self.credential, PreparedCredential::OneShot) {
            return Err(invalid_credential_binding());
        }
        Ok(KeystoreAccess {
            path: self.path,
            credential: AccessCredential::OneShot(secret),
            create: self.create,
        })
    }

    /// Consumes a runtime-profile selection and retains only its redacted unlock-file path.
    pub fn with_managed_credential(self) -> Result<KeystoreAccess, PublicError> {
        let PreparedCredential::File(path) = self.credential else {
            return Err(invalid_credential_binding());
        };
        Ok(KeystoreAccess {
            path: self.path,
            credential: AccessCredential::File(path),
            create: self.create,
        })
    }
}

/// Opaque one-use keystore access consumed by an application service.
pub struct KeystoreAccess {
    path: PathBuf,
    credential: AccessCredential,
    create: bool,
}

enum AccessCredential {
    OneShot(SecretInput),
    File(PathBuf),
}

/// Resolves one selection for an operation that requires an existing keystore.
pub async fn prepare_existing_keystore_access(
    selection: KeystoreSelection,
) -> Result<PreparedKeystoreAccess, PublicError> {
    prepare_keystore_access(selection, false).await
}

/// Resolves one selection for import, allowing creation at the selected path.
pub async fn prepare_import_keystore_access(
    selection: KeystoreSelection,
) -> Result<PreparedKeystoreAccess, PublicError> {
    prepare_keystore_access(selection, true).await
}

async fn prepare_keystore_access(
    selection: KeystoreSelection,
    allow_create: bool,
) -> Result<PreparedKeystoreAccess, PublicError> {
    tokio::task::spawn_blocking(move || prepare_keystore_access_on_worker(selection, allow_create))
        .await
        .map_err(|_| keystore_worker_error())?
}

fn prepare_keystore_access_on_worker(
    selection: KeystoreSelection,
    allow_create: bool,
) -> Result<PreparedKeystoreAccess, PublicError> {
    let (path, credential) = match selection.0 {
        SelectionKind::Explicit(path) => {
            validate_runtime_path(&path)?;
            (path, PreparedCredential::OneShot)
        }
        SelectionKind::RuntimeProfile {
            runtime_config_path,
            keystore_ref,
        } => {
            let runtime_config_path = runtime_config_path
                .or_else(|| env::var_os(MFM_RUNTIME_CONFIG_FILE).map(PathBuf::from))
                .ok_or_else(|| {
                    PublicError::bad_request(
                        "missing_keystore_selection",
                        "provide --keystore or --runtime-config",
                    )
                })?;
            validate_runtime_path(&runtime_config_path)?;
            let keystore_ref = KeystoreRef::new(
                keystore_ref.as_deref().unwrap_or(DEFAULT_KEYSTORE_REF),
            )
            .map_err(|_| {
                PublicError::bad_request(
                    "invalid_argument",
                    "--keystore-ref is not a valid profile ref",
                )
            })?;
            let config = RuntimeConfig::load_path_with_requirements(
                runtime_config_path,
                RuntimeConfigRequirement::keystores(),
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
            let path = profile.keystore_path().expose_path().to_path_buf();
            let unlock_file = profile.unlock_file().expose_path().to_path_buf();
            validate_runtime_path(&path)?;
            validate_runtime_path(&unlock_file)?;
            (path, PreparedCredential::File(unlock_file))
        }
    };

    let create = !path.exists();
    if create && !allow_create {
        return Err(PublicError::bad_request(
            "keystore_error",
            "Keystore not found",
        ));
    }
    Ok(PreparedKeystoreAccess {
        path,
        credential,
        create,
    })
}

/// One key selector accepted by the delete service.
pub enum KeystoreKeySelector {
    /// Select by exact UUID text.
    Id(String),
    /// Select by exact label.
    Label(String),
}

/// Consuming keystore import request.
pub struct KeystoreImportRequest {
    access: KeystoreAccess,
    label: Option<String>,
    material: SecretInput,
    kind: ImportKind,
}

enum ImportKind {
    PrivateKey,
    Mnemonic {
        derivation_path: String,
        passphrase: Option<SecretInput>,
    },
}

impl KeystoreImportRequest {
    /// Builds one raw private-key import request.
    pub fn private_key(
        access: KeystoreAccess,
        label: Option<String>,
        material: SecretInput,
    ) -> Self {
        Self {
            access,
            label,
            material,
            kind: ImportKind::PrivateKey,
        }
    }

    /// Builds one mnemonic import request with an optional BIP-39 passphrase.
    pub fn mnemonic(
        access: KeystoreAccess,
        label: Option<String>,
        material: SecretInput,
        derivation_path: String,
        passphrase: Option<SecretInput>,
    ) -> Self {
        Self {
            access,
            label,
            material,
            kind: ImportKind::Mnemonic {
                derivation_path,
                passphrase,
            },
        }
    }
}

/// Imports one key and returns only its public metadata.
pub async fn import_keystore_key(
    request: KeystoreImportRequest,
) -> Result<KeystoreKeyMetadata, PublicError> {
    tokio::task::spawn_blocking(move || import_keystore_key_on_worker(request))
        .await
        .map_err(|_| keystore_worker_error())?
}

fn import_keystore_key_on_worker(
    request: KeystoreImportRequest,
) -> Result<KeystoreKeyMetadata, PublicError> {
    let mut material = request.material.into_zeroizing();
    match &request.kind {
        ImportKind::PrivateKey => normalize_private_key_in_place(&mut material)?,
        ImportKind::Mnemonic { .. } => validate_mnemonic_basic(&material)?,
    }

    let mut keystore = open_keystore(request.access)?;
    let key_id = match request.kind {
        ImportKind::PrivateKey => {
            let label = request
                .label
                .unwrap_or_else(|| format!("imported-key-{}", Utc::now().format("%Y%m%d-%H%M%S")));
            keystore
                .import_private_key(Some(label), material.as_str())
                .map_err(admin_error_from_keystore)?
        }
        ImportKind::Mnemonic {
            derivation_path,
            passphrase,
        } => {
            let label = request
                .label
                .unwrap_or_else(|| format!("imported-hd-{}", Utc::now().format("%Y%m%d-%H%M%S")));
            let passphrase = passphrase.map(SecretInput::into_zeroizing);
            keystore
                .import_mnemonic(
                    Some(label),
                    material.as_str(),
                    &derivation_path,
                    passphrase.as_ref().map(|value| value.as_str()),
                )
                .map_err(admin_error_from_keystore)?
        }
    };
    key_metadata(&keystore, key_id)
}

/// Lists public metadata from one selected keystore.
pub async fn list_keystore_keys(
    access: KeystoreAccess,
) -> Result<Vec<KeystoreKeyMetadata>, PublicError> {
    tokio::task::spawn_blocking(move || {
        let keystore = open_keystore(access)?;
        keystore.list_keys().map_err(admin_error_from_keystore)
    })
    .await
    .map_err(|_| keystore_worker_error())?
}

/// Deletes one selected key and returns its former public metadata.
pub async fn delete_keystore_key(
    access: KeystoreAccess,
    selector: KeystoreKeySelector,
) -> Result<KeystoreKeyMetadata, PublicError> {
    tokio::task::spawn_blocking(move || {
        let mut keystore = open_keystore(access)?;
        let key_id = resolve_key_id(&keystore, selector)?;
        let metadata = key_metadata(&keystore, key_id)?;
        keystore
            .delete_key(key_id)
            .map_err(admin_error_from_keystore)?;
        Ok(metadata)
    })
    .await
    .map_err(|_| keystore_worker_error())?
}

fn open_keystore(access: KeystoreAccess) -> Result<Keystore, PublicError> {
    if access.create {
        if let Some(parent) = access.path.parent() {
            fs::create_dir_all(parent).map_err(|_| {
                PublicError::internal("keystore_error", "Failed to create keystore directory")
            })?;
        }
    } else if !access.path.exists() {
        return Err(PublicError::bad_request(
            "keystore_error",
            "Keystore not found",
        ));
    }

    let credential = match access.credential {
        AccessCredential::OneShot(secret) => secret.into_zeroizing(),
        AccessCredential::File(path) => read_secret_file(&path)?.into_zeroizing(),
    };
    let mut keystore = if access.create {
        Keystore::new_with_config(&access.path, KeystoreConfig::default())
    } else {
        Keystore::new(&access.path)
    }
    .map_err(|_| PublicError::internal("keystore_error", "Failed to open keystore"))?;
    keystore.unlock(credential.as_str()).map_err(|_| {
        PublicError::bad_request("keystore_error", "invalid credential for keystore unlock")
    })?;
    drop(credential);
    Ok(keystore)
}

fn key_metadata(keystore: &Keystore, key_id: Uuid) -> Result<KeystoreKeyMetadata, PublicError> {
    keystore
        .list_keys()
        .map_err(admin_error_from_keystore)?
        .into_iter()
        .find(|key| key.id == key_id)
        .ok_or_else(|| PublicError::bad_request("key_not_found", "Key not found"))
}

fn resolve_key_id(keystore: &Keystore, selector: KeystoreKeySelector) -> Result<Uuid, PublicError> {
    match selector {
        KeystoreKeySelector::Id(raw) => Uuid::parse_str(&raw)
            .map_err(|_| PublicError::bad_request("invalid_uuid", "Invalid UUID format")),
        KeystoreKeySelector::Label(label) => {
            let matching = keystore
                .list_keys()
                .map_err(admin_error_from_keystore)?
                .into_iter()
                .filter(|key| key.alias.as_deref() == Some(label.as_str()))
                .collect::<Vec<_>>();
            match matching.as_slice() {
                [] => Err(PublicError::bad_request(
                    "key_not_found",
                    "No key found with requested label",
                )),
                [key] => Ok(key.id),
                _ => Err(PublicError::bad_request(
                    "ambiguous_label",
                    "Multiple keys found with requested label",
                )),
            }
        }
    }
}

fn normalize_private_key_in_place(material: &mut String) -> Result<(), PublicError> {
    trim_in_place(material);
    if material.starts_with("0x") {
        material.drain(..2);
    }
    if material.len() != 64 || !material.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PublicError::bad_request(
            "invalid_key_material",
            "Key material must be 64 hexadecimal characters",
        ));
    }
    Ok(())
}

fn trim_in_place(value: &mut String) {
    let start = value
        .char_indices()
        .find(|(_, character)| !character.is_whitespace())
        .map_or(value.len(), |(index, _)| index);
    let end = value
        .char_indices()
        .rfind(|(_, character)| !character.is_whitespace())
        .map_or(start, |(index, character)| index + character.len_utf8());
    value.truncate(end);
    value.drain(..start);
}

fn validate_mnemonic_basic(material: &str) -> Result<(), PublicError> {
    if material.split_whitespace().count() < 12 {
        return Err(PublicError::bad_request(
            "invalid_recovery_phrase",
            "Recovery phrase must have at least 12 words",
        ));
    }
    Ok(())
}

fn read_secret_file(path: &Path) -> Result<SecretInput, PublicError> {
    let file = fs::File::open(path)
        .map_err(|_| PublicError::internal("keystore_error", "Failed to read secret file"))?;
    secret_from_reader(file)
}

fn secret_from_reader(reader: impl Read) -> Result<SecretInput, PublicError> {
    let mut bytes = ProtectedBytes::new();
    reader
        .take((MAX_SECRET_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes.value)
        .map_err(|_| PublicError::internal("keystore_error", "Failed to read secret file"))?;
    if bytes.value.len() > MAX_SECRET_FILE_BYTES {
        return Err(PublicError::bad_request(
            "keystore_error",
            "credential file exceeded the supported size",
        ));
    }
    strip_one_line_ending(&mut bytes.value);
    if bytes.value.is_empty() {
        return Err(PublicError::bad_request(
            "keystore_error",
            "credential file was empty",
        ));
    }
    let decoded = std::str::from_utf8(&bytes.value).map_err(|_| {
        PublicError::bad_request("keystore_error", "credential file was not valid UTF-8")
    })?;
    let mut secret = Zeroizing::new(String::with_capacity(decoded.len()));
    secret.push_str(decoded);
    Ok(SecretInput::from_zeroizing(secret))
}

struct ProtectedBytes {
    value: Zeroizing<Vec<u8>>,
}

impl ProtectedBytes {
    fn new() -> Self {
        Self {
            value: Zeroizing::new(Vec::with_capacity(MAX_SECRET_FILE_BYTES + 1)),
        }
    }
}

impl Drop for ProtectedBytes {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}

fn strip_one_line_ending(value: &mut Vec<u8>) {
    if value.ends_with(b"\r\n") {
        value.truncate(value.len() - 2);
    } else if value.ends_with(b"\n") {
        value.truncate(value.len() - 1);
    }
}

fn validate_runtime_path(path: &Path) -> Result<(), PublicError> {
    let Some(encoded) = path.to_str() else {
        return Err(invalid_runtime_path());
    };
    if encoded.is_empty() || encoded.len() > MAX_RUNTIME_PATH_BYTES {
        return Err(invalid_runtime_path());
    }
    Ok(())
}

fn invalid_runtime_path() -> PublicError {
    PublicError::bad_request("invalid_argument", "selected keystore path is invalid")
}

fn invalid_credential_binding() -> PublicError {
    PublicError::bad_request(
        "invalid_keystore_credential",
        "selected keystore credential source is invalid",
    )
}

fn admin_error_from_keystore(error: KeystoreError) -> PublicError {
    match error {
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

fn keystore_worker_error() -> PublicError {
    PublicError::internal("keystore_worker_failed", "Keystore operation worker failed")
}

#[cfg(any(test, feature = "test-support"))]
/// Initializes a fast test keystore and optionally seeds private keys through app ownership.
pub fn initialize_insecure_keystore_for_test(
    path: PathBuf,
    unlock: SecretInput,
    private_keys: Vec<(Option<String>, SecretInput)>,
) -> Result<Vec<KeystoreKeyMetadata>, PublicError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|_| PublicError::internal("keystore_error", "test keystore setup failed"))?;
    }
    let unlock = unlock.into_zeroizing();
    let mut keystore = Keystore::new_with_config(path, KeystoreConfig::insecure_integration_test())
        .map_err(|_| PublicError::internal("keystore_error", "test keystore setup failed"))?;
    keystore
        .unlock(unlock.as_str())
        .map_err(|_| PublicError::internal("keystore_error", "test keystore setup failed"))?;
    drop(unlock);
    let mut imported = Vec::with_capacity(private_keys.len());
    for (label, material) in private_keys {
        let mut material = material.into_zeroizing();
        normalize_private_key_in_place(&mut material)?;
        let id = keystore
            .import_private_key(label, material.as_str())
            .map_err(admin_error_from_keystore)?;
        imported.push(key_metadata(&keystore, id)?);
    }
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::assert_not_impl_any;

    assert_not_impl_any!(SecretInput: Clone, Copy, std::fmt::Debug, std::fmt::Display, serde::Serialize);
    assert_not_impl_any!(SecretInput: serde::de::DeserializeOwned);
    assert_not_impl_any!(SecretInput: AsRef<str>, std::borrow::Borrow<str>);
    assert_not_impl_any!(KeystoreImportRequest: Clone, std::fmt::Debug, std::fmt::Display, serde::Serialize);
    assert_not_impl_any!(KeystoreAccess: Clone, std::fmt::Debug, std::fmt::Display, serde::Serialize);
    assert_not_impl_any!(KeystoreSelection: Clone, std::fmt::Debug, std::fmt::Display, serde::Serialize);

    #[tokio::test]
    async fn app_owns_import_list_delete_and_redacts_failures() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("private-keystore-name");
        initialize_insecure_keystore_for_test(
            path.clone(),
            SecretInput::new("strong_password_123".to_owned()),
            Vec::new(),
        )
        .expect("initialize keystore");

        let prepared = prepare_import_keystore_access(KeystoreSelection::explicit(path.clone()))
            .await
            .expect("prepare import");
        assert_eq!(
            prepared.credential_requirement(),
            KeystoreCredentialRequirement::Unlock
        );
        let access = prepared
            .with_secret(SecretInput::new("strong_password_123".to_owned()))
            .expect("bind unlock");
        let imported = import_keystore_key(KeystoreImportRequest::private_key(
            access,
            Some("deployer".to_owned()),
            SecretInput::new(
                "0000000000000000000000000000000000000000000000000000000000000001".to_owned(),
            ),
        ))
        .await
        .expect("import");

        let access = prepare_existing_keystore_access(KeystoreSelection::explicit(path.clone()))
            .await
            .expect("prepare list")
            .with_secret(SecretInput::new("strong_password_123".to_owned()))
            .expect("bind unlock");
        let listed = list_keystore_keys(access).await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, imported.id);

        let access = prepare_existing_keystore_access(KeystoreSelection::explicit(path.clone()))
            .await
            .expect("prepare delete")
            .with_secret(SecretInput::new("strong_password_123".to_owned()))
            .expect("bind unlock");
        let deleted =
            delete_keystore_key(access, KeystoreKeySelector::Label("deployer".to_owned()))
                .await
                .expect("delete");
        assert_eq!(deleted.id, imported.id);

        let secret = "wrong-secret-must-not-escape";
        let access = prepare_existing_keystore_access(KeystoreSelection::explicit(path.clone()))
            .await
            .expect("prepare failed list")
            .with_secret(SecretInput::new(secret.to_owned()))
            .expect("bind wrong unlock");
        let error = list_keystore_keys(access).await.expect_err("wrong unlock");
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains(secret));
        assert!(!rendered.contains("private-keystore-name"));
    }

    #[tokio::test]
    async fn managed_access_resolves_and_rereads_only_the_unlock_file_path() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("managed.keystore");
        let unlock_file = directory.path().join("managed.unlock");
        let runtime_config = directory.path().join("runtime.toml");
        let password = "managed_password_123";
        fs::write(&unlock_file, format!("{password}\n")).expect("unlock file");
        initialize_insecure_keystore_for_test(
            path.clone(),
            SecretInput::new(password.to_owned()),
            Vec::new(),
        )
        .expect("initialize keystore");
        fs::write(
            &runtime_config,
            format!(
                "[keystores.default]\nkeystore_path = {}\nunlock_file = {}\n",
                serde_json::to_string(&path.display().to_string()).expect("path"),
                serde_json::to_string(&unlock_file.display().to_string()).expect("unlock path")
            ),
        )
        .expect("runtime config");

        let prepared = prepare_existing_keystore_access(KeystoreSelection::runtime_profile(
            Some(runtime_config.clone()),
            None,
        ))
        .await
        .expect("prepare managed access");
        assert_eq!(
            prepared.credential_requirement(),
            KeystoreCredentialRequirement::Managed
        );
        list_keystore_keys(
            prepared
                .with_managed_credential()
                .expect("bind managed credential"),
        )
        .await
        .expect("managed list");

        fs::write(&unlock_file, "changed_password_123\n").expect("replace unlock file");
        let access = prepare_existing_keystore_access(KeystoreSelection::runtime_profile(
            Some(runtime_config),
            None,
        ))
        .await
        .expect("prepare changed managed access")
        .with_managed_credential()
        .expect("bind managed credential");
        let error = list_keystore_keys(access)
            .await
            .expect_err("changed unlock must reject");
        let rendered = format!("{error:?} {error}");
        assert!(!rendered.contains("changed_password_123"));
        assert!(!rendered.contains("managed.unlock"));
    }
}
