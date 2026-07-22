#![allow(clippy::disallowed_methods)]

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use zeroize::Zeroize;

use crate::commands::result::PublicError;

const MAX_SECRET_INPUT_BYTES: usize = 64 * 1_024;

/// Optional BIP-39 passphrase source parsed by the CLI transport.
pub(crate) enum Bip39ExtraSource {
    /// No passphrase.
    None,
    /// Prompt locally without echo.
    Prompt,
    /// Read from a local file or FIFO.
    FilePath(PathBuf),
}

struct SecretBuffer(String);

impl SecretBuffer {
    fn empty() -> Self {
        Self(String::with_capacity(MAX_SECRET_INPUT_BYTES + 1))
    }

    fn from_string(value: String) -> Self {
        Self(value)
    }

    fn as_str(&self) -> &str {
        &self.0
    }

    fn into_secret_input(mut self) -> mfm_app::SecretInput {
        mfm_app::SecretInput::new(std::mem::take(&mut self.0))
    }
}

impl Drop for SecretBuffer {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Reads and validates one private-key import input.
pub(crate) fn read_private_key_material(
    from_stdin: bool,
) -> Result<mfm_app::SecretInput, PublicError> {
    let mut material = read_secret_material("Enter private key (hex): ", from_stdin)?;
    trim_in_place(&mut material.0);
    if material.0.starts_with("0x") {
        material.0.drain(..2);
    }
    if material.0.len() != 64 || !material.0.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(PublicError::bad_request(
            "invalid_key_material",
            "Key material must be 64 hexadecimal characters",
        ));
    }
    Ok(material.into_secret_input())
}

/// Reads and validates one mnemonic import input.
pub(crate) fn read_mnemonic_material(
    from_stdin: bool,
) -> Result<mfm_app::SecretInput, PublicError> {
    let material = read_secret_material("Enter mnemonic phrase: ", from_stdin)?;
    if material.as_str().split_whitespace().count() < 12 {
        return Err(PublicError::bad_request(
            "invalid_recovery_phrase",
            "Recovery phrase must have at least 12 words",
        ));
    }
    Ok(material.into_secret_input())
}

/// Resolves one optional BIP-39 passphrase into the sole app secret-input type.
pub(crate) fn read_bip39_extra(
    source: Bip39ExtraSource,
) -> Result<Option<mfm_app::SecretInput>, PublicError> {
    match source {
        Bip39ExtraSource::None => Ok(None),
        Bip39ExtraSource::Prompt => read_hidden("Enter BIP-39 passphrase: ").map(Some),
        Bip39ExtraSource::FilePath(path) => read_secret_file(&path)
            .map(SecretBuffer::into_secret_input)
            .map(Some),
    }
}

/// Binds the prompt or managed credential required by one app-prepared selection.
pub(crate) fn bind_prepared_access(
    prepared: mfm_app::PreparedKeystoreAccess,
) -> Result<mfm_app::KeystoreAccess, PublicError> {
    match prepared.credential_requirement() {
        mfm_app::KeystoreCredentialRequirement::Managed => prepared.with_managed_credential(),
        mfm_app::KeystoreCredentialRequirement::Unlock => {
            prepared.with_secret(read_hidden("Enter keystore password: ")?)
        }
        mfm_app::KeystoreCredentialRequirement::Create => {
            let password = read_hidden_buffer("Enter password for new keystore: ")?;
            let confirmation = read_hidden_buffer("Confirm password: ")?;
            if password.as_str() != confirmation.as_str() {
                return Err(PublicError::bad_request(
                    "keystore_error",
                    "credential entries did not match",
                ));
            }
            prepared.with_secret(password.into_secret_input())
        }
    }
}

/// Builds the exact delete selector and rejects ambiguous CLI input before prompting.
pub(crate) fn key_selector(
    id: Option<String>,
    by_label: Option<String>,
) -> Result<mfm_app::KeystoreKeySelector, PublicError> {
    match (id, by_label) {
        (Some(_), Some(_)) => Err(PublicError::bad_request(
            "missing_argument",
            "Specify exactly one key selector: ID or --by-label",
        )),
        (None, None) => Err(PublicError::bad_request(
            "missing_argument",
            "Must specify either key ID or --by-label",
        )),
        (Some(id), None) => {
            uuid::Uuid::parse_str(&id)
                .map_err(|_| PublicError::bad_request("invalid_uuid", "Invalid UUID format"))?;
            Ok(mfm_app::KeystoreKeySelector::Id(id))
        }
        (None, Some(label)) => Ok(mfm_app::KeystoreKeySelector::Label(label)),
    }
}

/// Returns the current stable presentation code for public key metadata.
pub(crate) fn key_type_code(key_type: &mfm_app::KeystoreKeyType) -> &'static str {
    match key_type {
        mfm_app::KeystoreKeyType::PrivateKey => "privatekey",
        mfm_app::KeystoreKeyType::HdDerived { .. } => "hd_derived",
    }
}

/// Prompts for a non-secret delete confirmation.
pub(crate) fn confirm(prompt: &str) -> Result<bool, PublicError> {
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
            _ => println!("Please enter 'y' or 'n'"),
        }
    }
}

fn read_secret_material(prompt: &str, from_stdin: bool) -> Result<SecretBuffer, PublicError> {
    if from_stdin {
        return read_stdin_material();
    }
    read_hidden_buffer(prompt)
}

fn read_stdin_material() -> Result<SecretBuffer, PublicError> {
    let mut input = SecretBuffer::empty();
    io::stdin()
        .take((MAX_SECRET_INPUT_BYTES + 1) as u64)
        .read_to_string(&mut input.0)
        .map_err(|_| PublicError::internal("input_error", "Failed to read secret input"))?;
    if input.0.len() > MAX_SECRET_INPUT_BYTES {
        return Err(PublicError::bad_request(
            "input_error",
            "stdin secret material exceeded the supported size",
        ));
    }
    strip_one_line_ending(&mut input.0);
    if input.0.contains(['\r', '\n']) {
        return Err(PublicError::bad_request(
            "input_error",
            "stdin secret material must contain exactly one line",
        ));
    }
    Ok(input)
}

fn read_hidden(prompt: &str) -> Result<mfm_app::SecretInput, PublicError> {
    read_hidden_buffer(prompt).map(SecretBuffer::into_secret_input)
}

fn read_hidden_buffer(prompt: &str) -> Result<SecretBuffer, PublicError> {
    print!("{prompt}");
    io::stdout()
        .flush()
        .map_err(|_| PublicError::internal("input_error", "Failed to prompt for input"))?;
    rpassword::read_password()
        .map(SecretBuffer::from_string)
        .map_err(|_| PublicError::internal("input_error", "Failed to read password"))
}

fn read_secret_file(path: &Path) -> Result<SecretBuffer, PublicError> {
    let file = std::fs::File::open(path)
        .map_err(|_| PublicError::internal("keystore_error", "Failed to read secret file"))?;
    let mut raw = SecretBuffer::empty();
    file.take((MAX_SECRET_INPUT_BYTES + 1) as u64)
        .read_to_string(&mut raw.0)
        .map_err(|_| PublicError::internal("keystore_error", "Failed to read secret file"))?;
    if raw.0.len() > MAX_SECRET_INPUT_BYTES {
        return Err(PublicError::bad_request(
            "keystore_error",
            "secret file exceeded the supported size",
        ));
    }
    strip_one_line_ending(&mut raw.0);
    if raw.0.is_empty() {
        return Err(PublicError::bad_request(
            "keystore_error",
            "secret file was empty",
        ));
    }
    Ok(raw)
}

fn strip_one_line_ending(input: &mut String) {
    if input.ends_with("\r\n") {
        input.truncate(input.len() - 2);
    } else if input.ends_with('\n') {
        input.pop();
    }
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

#[cfg(test)]
mod tests {
    use super::SecretBuffer;
    use static_assertions::assert_not_impl_any;

    assert_not_impl_any!(SecretBuffer: Clone, Copy, std::fmt::Debug, std::fmt::Display, serde::Serialize);
    assert_not_impl_any!(SecretBuffer: serde::de::DeserializeOwned);
    assert_not_impl_any!(SecretBuffer: AsRef<str>, std::borrow::Borrow<str>);
}
