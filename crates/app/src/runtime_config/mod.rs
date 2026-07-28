use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mfm_ids::{LocalPublicId, StableId};
use mfm_signing::SignerRef;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use self::document::RuntimeDocument;
use self::value::{ResolvedValue, ValueSource};

mod document;
mod value;

#[cfg(test)]
mod tests;

pub(crate) type Result<T> = std::result::Result<T, RuntimeConfigError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeConfigErrorKind {
    UnsupportedFormat,
    DocumentRead,
    DocumentTooLarge,
    InvalidUtf8,
    Syntax,
    DuplicateJsonKey,
    UnknownTopLevel,
    ForbiddenExpectedChainId,
    ForbiddenSecretField,
    DirectSecretValue,
    MissingSection,
    MissingEntry,
    MissingSigner,
    MissingKeystore,
    InvalidSelectedObject,
    UnknownSelectedField,
    MissingRequiredField,
    InvalidIdentifier,
    InvalidValueSource,
    MissingEnvironmentValue,
    InvalidEnvironmentValue,
    InvalidPath,
    IndirectionRead,
    ResolvedValueTooLarge,
    EmptyResolvedValue,
    InvalidChainId,
    UnsupportedSignerProvider,
    InvalidEntryId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("runtime configuration is invalid")]
pub(crate) struct RuntimeConfigError {
    kind: RuntimeConfigErrorKind,
}

impl RuntimeConfigError {
    pub(super) const fn new(kind: RuntimeConfigErrorKind) -> Self {
        Self { kind }
    }

    pub(crate) const fn kind(self) -> RuntimeConfigErrorKind {
        self.kind
    }
}

pub(crate) struct ResolvedEvmRoute {
    network_id: LocalPublicId,
    source_ref: LocalPublicId,
    chain_id: u64,
    generation_id: StableId,
    rpc_url: ResolvedValue,
    auth_header: Option<ResolvedValue>,
}

impl ResolvedEvmRoute {
    pub(crate) fn into_parts(
        self,
    ) -> (
        LocalPublicId,
        LocalPublicId,
        u64,
        StableId,
        ResolvedValue,
        Option<ResolvedValue>,
    ) {
        (
            self.network_id,
            self.source_ref,
            self.chain_id,
            self.generation_id,
            self.rpc_url,
            self.auth_header,
        )
    }
}

pub(crate) struct ResolvedKeystoreProfile {
    keystore_path: PathBuf,
    unlock_file_path: PathBuf,
}

impl ResolvedKeystoreProfile {
    pub(crate) fn into_paths(self) -> (PathBuf, PathBuf) {
        (self.keystore_path, self.unlock_file_path)
    }
}

pub(crate) struct ResolvedSignerBinding {
    entry_id: Uuid,
    keystore: ResolvedKeystoreProfile,
}

impl ResolvedSignerBinding {
    pub(crate) fn into_parts(self) -> (Uuid, PathBuf, PathBuf) {
        let (keystore_path, unlock_file_path) = self.keystore.into_paths();
        (self.entry_id, keystore_path, unlock_file_path)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRoutesSection {
    routes: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEvmRoute {
    source_ref: String,
    chain_id: u64,
    generation_id: String,
    rpc_url: ValueSource,
    #[serde(default)]
    auth_header: Option<ValueSource>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKeystoreProfile {
    keystore_path: ValueSource,
    unlock_file_path: ValueSource,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSignerBinding {
    provider: String,
    keystore_ref: String,
    entry_id: String,
}

pub(crate) fn load_evm_routes(path: &Path) -> Result<Vec<ResolvedEvmRoute>> {
    let document = RuntimeDocument::load(path)?;
    let section: RawRoutesSection = decode_selected(document.take_section("evm")?)?;
    if section.routes.is_empty() {
        return Err(RuntimeConfigError::new(
            RuntimeConfigErrorKind::MissingEntry,
        ));
    }
    section
        .routes
        .into_iter()
        .map(|(network_id, value)| {
            let network_id = LocalPublicId::new(network_id)
                .map_err(|_| RuntimeConfigError::new(RuntimeConfigErrorKind::InvalidIdentifier))?;
            let raw: RawEvmRoute = decode_selected(value)?;
            if raw.chain_id == 0 {
                return Err(RuntimeConfigError::new(
                    RuntimeConfigErrorKind::InvalidChainId,
                ));
            }
            let source_ref = LocalPublicId::new(&raw.source_ref)
                .map_err(|_| RuntimeConfigError::new(RuntimeConfigErrorKind::InvalidIdentifier))?;
            let generation_id = StableId::new(&raw.generation_id)
                .map_err(|_| RuntimeConfigError::new(RuntimeConfigErrorKind::InvalidIdentifier))?;
            Ok(ResolvedEvmRoute {
                network_id,
                source_ref,
                chain_id: raw.chain_id,
                generation_id,
                rpc_url: raw.rpc_url.resolve_public()?,
                auth_header: raw
                    .auth_header
                    .map(ValueSource::resolve_secret)
                    .transpose()?,
            })
        })
        .collect()
}

pub(crate) fn load_keystore_profile(
    path: &Path,
    keystore_ref: &str,
) -> Result<ResolvedKeystoreProfile> {
    LocalPublicId::new(keystore_ref)
        .map_err(|_| RuntimeConfigError::new(RuntimeConfigErrorKind::InvalidIdentifier))?;
    let document = RuntimeDocument::load(path)?;
    let raw: RawKeystoreProfile = decode_selected(
        take_entry(document, "keystores", keystore_ref).map_err(|error| {
            if error.kind() == RuntimeConfigErrorKind::MissingEntry {
                RuntimeConfigError::new(RuntimeConfigErrorKind::MissingKeystore)
            } else {
                error
            }
        })?,
    )?;
    resolve_keystore(raw)
}

pub(crate) fn load_signer_binding(
    path: &Path,
    signer_ref: &SignerRef,
) -> Result<ResolvedSignerBinding> {
    let mut document = RuntimeDocument::load(path)?;
    let signer_value = document
        .take_entry("signers", signer_ref.as_str())
        .map_err(|error| {
            if error.kind() == RuntimeConfigErrorKind::MissingEntry {
                RuntimeConfigError::new(RuntimeConfigErrorKind::MissingSigner)
            } else {
                error
            }
        })?;
    let signer: RawSignerBinding = decode_selected(signer_value)?;
    if signer.provider != "keystore" {
        return Err(RuntimeConfigError::new(
            RuntimeConfigErrorKind::UnsupportedSignerProvider,
        ));
    }
    LocalPublicId::new(&signer.keystore_ref)
        .map_err(|_| RuntimeConfigError::new(RuntimeConfigErrorKind::InvalidIdentifier))?;
    let entry_id = Uuid::parse_str(&signer.entry_id)
        .map_err(|_| RuntimeConfigError::new(RuntimeConfigErrorKind::InvalidEntryId))?;
    let keystore: RawKeystoreProfile = decode_selected(
        document
            .take_entry("keystores", &signer.keystore_ref)
            .map_err(|error| {
                if error.kind() == RuntimeConfigErrorKind::MissingEntry {
                    RuntimeConfigError::new(RuntimeConfigErrorKind::MissingKeystore)
                } else {
                    error
                }
            })?,
    )?;
    Ok(ResolvedSignerBinding {
        entry_id,
        keystore: resolve_keystore(keystore)?,
    })
}

fn take_entry(mut document: RuntimeDocument, section: &'static str, key: &str) -> Result<Value> {
    document.take_entry(section, key)
}

fn resolve_keystore(raw: RawKeystoreProfile) -> Result<ResolvedKeystoreProfile> {
    Ok(ResolvedKeystoreProfile {
        keystore_path: raw.keystore_path.resolve_path()?,
        unlock_file_path: raw.unlock_file_path.resolve_path()?,
    })
}

fn decode_selected<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(classify_selected_decode_error)
}

fn classify_selected_decode_error(error: serde_json::Error) -> RuntimeConfigError {
    let message = error.to_string();
    let kind = if message.contains("unknown field") {
        RuntimeConfigErrorKind::UnknownSelectedField
    } else if message.contains("missing field") {
        RuntimeConfigErrorKind::MissingRequiredField
    } else {
        RuntimeConfigErrorKind::InvalidSelectedObject
    };
    RuntimeConfigError::new(kind)
}
