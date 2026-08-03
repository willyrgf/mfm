use std::path::{Path, PathBuf};

use mfm_ids::LocalPublicId;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;

use self::document::RuntimeDocument;
use self::value::ValueSource;

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
    ForbiddenSecretField,
    MissingSection,
    MissingEntry,
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

pub(crate) struct ResolvedKeystoreProfile {
    keystore_path: PathBuf,
    unlock_file_path: PathBuf,
}

impl std::fmt::Debug for ResolvedKeystoreProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedKeystoreProfile")
            .finish_non_exhaustive()
    }
}

impl ResolvedKeystoreProfile {
    pub(crate) fn into_paths(self) -> (PathBuf, PathBuf) {
        (self.keystore_path, self.unlock_file_path)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKeystoreProfile {
    keystore_path: ValueSource,
    unlock_file_path: ValueSource,
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
