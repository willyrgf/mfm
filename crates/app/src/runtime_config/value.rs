use std::env;
use std::fs;
use std::io::Read;
use std::path::PathBuf;

use mfm_ids::RuntimeEnvName;
use serde::Deserialize;
use zeroize::{Zeroize, Zeroizing};

use super::{Result, RuntimeConfigError, RuntimeConfigErrorKind};

const MAX_RESOLVED_VALUE_BYTES: usize = 64 * 1024;
const MAX_RUNTIME_PATH_BYTES: usize = 4_096;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ValueSource {
    #[serde(default)]
    direct: Option<String>,
    #[serde(default)]
    env: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    file_env: Option<String>,
}

enum SelectedSource {
    Direct(String),
    Env(String),
    File(String),
    FileEnv(String),
}

pub(crate) struct ResolvedValue(Zeroizing<String>);

impl ResolvedValue {
    pub(crate) fn into_protected(self) -> Zeroizing<String> {
        self.0
    }

    pub(crate) fn into_string(mut self) -> String {
        std::mem::take(&mut *self.0)
    }
}

impl ValueSource {
    pub(super) fn resolve_public(self) -> Result<ResolvedValue> {
        self.select(false)?.resolve()
    }

    pub(super) fn resolve_secret(self) -> Result<ResolvedValue> {
        self.select(true)?.resolve()
    }

    pub(super) fn resolve_path(self) -> Result<PathBuf> {
        let value = self.resolve_public()?.into_string();
        validate_path(&value)?;
        Ok(PathBuf::from(value))
    }

    fn select(self, secret: bool) -> Result<SelectedSource> {
        let mut selected = Vec::with_capacity(1);
        if let Some(value) = self.direct {
            if secret {
                return Err(RuntimeConfigError::new(
                    RuntimeConfigErrorKind::DirectSecretValue,
                ));
            }
            selected.push(SelectedSource::Direct(value));
        }
        if let Some(value) = self.env {
            selected.push(SelectedSource::Env(value));
        }
        if let Some(value) = self.file {
            selected.push(SelectedSource::File(value));
        }
        if let Some(value) = self.file_env {
            selected.push(SelectedSource::FileEnv(value));
        }
        if selected.len() != 1 {
            return Err(RuntimeConfigError::new(
                RuntimeConfigErrorKind::InvalidValueSource,
            ));
        }
        Ok(selected.pop().expect("exactly one selected value source"))
    }
}

impl SelectedSource {
    fn resolve(self) -> Result<ResolvedValue> {
        let value = match self {
            Self::Direct(value) => protected_string(value)?,
            Self::Env(name) => read_environment_value(&name, MAX_RESOLVED_VALUE_BYTES)?,
            Self::File(path) => read_indirection_value(&path)?,
            Self::FileEnv(name) => {
                let path = read_environment_value(&name, MAX_RUNTIME_PATH_BYTES)?;
                validate_path(&path)?;
                read_indirection_value(&path)?
            }
        };
        if value.is_empty() {
            return Err(RuntimeConfigError::new(
                RuntimeConfigErrorKind::EmptyResolvedValue,
            ));
        }
        Ok(ResolvedValue(value))
    }
}

fn protected_string(value: String) -> Result<Zeroizing<String>> {
    if value.len() > MAX_RESOLVED_VALUE_BYTES {
        return Err(RuntimeConfigError::new(
            RuntimeConfigErrorKind::ResolvedValueTooLarge,
        ));
    }
    if value.is_empty() {
        return Err(RuntimeConfigError::new(
            RuntimeConfigErrorKind::EmptyResolvedValue,
        ));
    }
    Ok(Zeroizing::new(value))
}

fn read_environment_value(name: &str, limit: usize) -> Result<Zeroizing<String>> {
    RuntimeEnvName::new(name)
        .map_err(|_| RuntimeConfigError::new(RuntimeConfigErrorKind::InvalidEnvironmentValue))?;
    let value = env::var_os(name)
        .ok_or_else(|| RuntimeConfigError::new(RuntimeConfigErrorKind::MissingEnvironmentValue))?;
    let bytes = os_string_into_protected_bytes(value)?;
    if bytes.len() > limit {
        return Err(RuntimeConfigError::new(
            RuntimeConfigErrorKind::ResolvedValueTooLarge,
        ));
    }
    copy_utf8_with_kind(&bytes, RuntimeConfigErrorKind::InvalidEnvironmentValue)
}

#[cfg(unix)]
fn os_string_into_protected_bytes(value: std::ffi::OsString) -> Result<ProtectedBytes> {
    use std::os::unix::ffi::OsStringExt;
    Ok(ProtectedBytes::from_vec(value.into_vec()))
}

#[cfg(not(unix))]
fn os_string_into_protected_bytes(value: std::ffi::OsString) -> Result<ProtectedBytes> {
    let value = value
        .into_string()
        .map_err(|_| RuntimeConfigError::new(RuntimeConfigErrorKind::InvalidEnvironmentValue))?;
    Ok(ProtectedBytes::from_vec(value.into_bytes()))
}

fn read_indirection_value(path: &str) -> Result<Zeroizing<String>> {
    validate_path(path)?;
    let file = fs::File::open(path)
        .map_err(|_| RuntimeConfigError::new(RuntimeConfigErrorKind::IndirectionRead))?;
    let mut bytes = read_bounded_file(
        file,
        MAX_RESOLVED_VALUE_BYTES,
        RuntimeConfigErrorKind::IndirectionRead,
        RuntimeConfigErrorKind::ResolvedValueTooLarge,
    )?;
    strip_one_line_ending(bytes.as_mut_vec());
    copy_utf8(&bytes)
}

fn validate_path(path: &str) -> Result<()> {
    if path.is_empty() || path.len() > MAX_RUNTIME_PATH_BYTES {
        return Err(RuntimeConfigError::new(RuntimeConfigErrorKind::InvalidPath));
    }
    Ok(())
}

fn copy_utf8(bytes: &ProtectedBytes) -> Result<Zeroizing<String>> {
    copy_utf8_with_kind(bytes, RuntimeConfigErrorKind::InvalidUtf8)
}

fn copy_utf8_with_kind(
    bytes: &ProtectedBytes,
    invalid_utf8: RuntimeConfigErrorKind,
) -> Result<Zeroizing<String>> {
    let decoded =
        std::str::from_utf8(bytes.as_slice()).map_err(|_| RuntimeConfigError::new(invalid_utf8))?;
    if decoded.is_empty() {
        return Err(RuntimeConfigError::new(
            RuntimeConfigErrorKind::EmptyResolvedValue,
        ));
    }
    let mut value = Zeroizing::new(String::with_capacity(decoded.len()));
    value.push_str(decoded);
    Ok(value)
}

pub(super) fn read_bounded_file(
    reader: impl Read,
    limit: usize,
    read_error: RuntimeConfigErrorKind,
    limit_error: RuntimeConfigErrorKind,
) -> Result<ProtectedBytes> {
    read_bounded(reader, limit, read_error, limit_error, None)
}

fn read_bounded(
    reader: impl Read,
    limit: usize,
    read_error: RuntimeConfigErrorKind,
    limit_error: RuntimeConfigErrorKind,
    #[cfg(test)] witness: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    #[cfg(not(test))] _witness: Option<()>,
) -> Result<ProtectedBytes> {
    let mut bytes = ProtectedBytes::with_capacity(
        limit + 1,
        #[cfg(test)]
        witness,
    );
    reader
        .take((limit + 1) as u64)
        .read_to_end(bytes.as_mut_vec())
        .map_err(|_| RuntimeConfigError::new(read_error))?;
    if bytes.len() > limit {
        return Err(RuntimeConfigError::new(limit_error));
    }
    Ok(bytes)
}

fn strip_one_line_ending(value: &mut Vec<u8>) {
    if value.ends_with(b"\r\n") {
        value.truncate(value.len() - 2);
    } else if value.ends_with(b"\n") {
        value.truncate(value.len() - 1);
    }
}

pub(super) struct ProtectedBytes {
    value: Zeroizing<Vec<u8>>,
    #[cfg(test)]
    witness: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl ProtectedBytes {
    fn with_capacity(
        capacity: usize,
        #[cfg(test)] witness: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    ) -> Self {
        Self {
            value: Zeroizing::new(Vec::with_capacity(capacity)),
            #[cfg(test)]
            witness,
        }
    }

    fn from_vec(value: Vec<u8>) -> Self {
        Self {
            value: Zeroizing::new(value),
            #[cfg(test)]
            witness: None,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.value.len()
    }

    fn as_slice(&self) -> &[u8] {
        &self.value
    }

    pub(super) fn as_mut_vec(&mut self) -> &mut Vec<u8> {
        &mut self.value
    }

    pub(super) fn as_utf8(&self) -> Result<&str> {
        std::str::from_utf8(self.as_slice())
            .map_err(|_| RuntimeConfigError::new(RuntimeConfigErrorKind::InvalidUtf8))
    }
}

impl Drop for ProtectedBytes {
    fn drop(&mut self) {
        self.value.as_mut_slice().zeroize();
        #[cfg(test)]
        if let Some(witness) = &self.witness {
            witness.store(
                self.value.iter().all(|byte| *byte == 0),
                std::sync::atomic::Ordering::SeqCst,
            );
        }
        self.value.clear();
    }
}

#[cfg(test)]
pub(super) fn read_bounded_with_witness(
    reader: impl Read,
    limit: usize,
    witness: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<ProtectedBytes> {
    read_bounded(
        reader,
        limit,
        RuntimeConfigErrorKind::IndirectionRead,
        RuntimeConfigErrorKind::ResolvedValueTooLarge,
        Some(witness),
    )
}
