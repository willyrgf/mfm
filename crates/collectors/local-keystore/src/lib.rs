#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
//! Typed local keystore collectors.
//!
//! This crate defines typed adapters over the generic `IoCall` surface for local keystore flows.
//! It intentionally does NOT perform IO itself.
#![warn(missing_docs)]

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::hashing::{artifact_id_for_json, CanonicalJsonError};
use mfm_machine::ids::{ErrorCode, FactKey, StateId};
use mfm_machine::io::{IoCall, IoProvider};

/// Canonical namespace group used for local keystore IO calls.
pub const NAMESPACE_LOCAL_KEYSTORE: &str = "local.keystore";
/// Namespace used for keystore import.
pub const NAMESPACE_LOCAL_KEYSTORE_IMPORT: &str = "local.keystore.import";
/// Namespace used for keystore listing.
pub const NAMESPACE_LOCAL_KEYSTORE_LIST: &str = "local.keystore.list";
/// Namespace used for keystore deletion.
pub const NAMESPACE_LOCAL_KEYSTORE_DELETE: &str = "local.keystore.delete";
/// Namespace used for local transaction signing.
pub const NAMESPACE_LOCAL_KEYSTORE_TX_SIGN: &str = "local.keystore.tx_sign";

/// Supported keystore import input formats.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum KeystoreImportType {
    /// Import a raw private key.
    #[serde(rename = "pk")]
    PrivateKey,
    /// Import a mnemonic phrase.
    #[serde(rename = "mn")]
    Mnemonic,
}

/// Non-secret source metadata for the optional BIP-39 passphrase used during mnemonic import.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", content = "file_path_hex", rename_all = "snake_case")]
pub enum Bip39ExtraSource {
    /// Do not use a BIP-39 passphrase.
    #[default]
    None,
    /// Prompt for the BIP-39 passphrase inside the local keystore transport.
    Prompt,
    /// Read the BIP-39 passphrase from this hex-encoded UTF-8 file or FIFO path.
    FilePathHex(String),
}

impl Bip39ExtraSource {
    /// Returns true when no BIP-39 passphrase source was requested.
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

/// Sort order supported by the keystore list flow.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeystoreListSortBy {
    /// Sort by alias.
    Label,
    /// Sort by creation timestamp.
    Created,
    /// Sort by key type.
    Type,
}

/// Typed request for `local.keystore.import`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct KeystoreImportRequest {
    /// Input kind to import.
    pub kind: KeystoreImportType,
    /// Optional UTF-8 label.
    #[serde(default)]
    pub label: Option<String>,
    /// Optional hex-encoded UTF-8 label.
    #[serde(default)]
    pub label_hex: Option<String>,
    /// BIP-32 derivation path.
    pub derive_path: String,
    /// Optional UTF-8 keystore path.
    #[serde(default)]
    pub store_path: Option<String>,
    /// Optional hex-encoded UTF-8 keystore path.
    #[serde(default)]
    pub store_path_hex: Option<String>,
    /// Whether to read secret material from stdin.
    pub stdin_mode: bool,
    /// Optional BIP-39 passphrase source metadata for mnemonic imports.
    #[serde(default, skip_serializing_if = "Bip39ExtraSource::is_none")]
    pub bip39_extra: Bip39ExtraSource,
}

/// Typed request for `local.keystore.list`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeystoreListRequest {
    /// Optional UTF-8 keystore path.
    #[serde(default)]
    pub store_path: Option<String>,
    /// Optional hex-encoded UTF-8 keystore path.
    #[serde(default)]
    pub store_path_hex: Option<String>,
    /// Whether addresses should be included.
    pub show_addrs: bool,
    /// Optional UTF-8 regex label filter.
    #[serde(default)]
    pub filter_label: Option<String>,
    /// Optional hex-encoded UTF-8 regex label filter.
    #[serde(default)]
    pub filter_label_hex: Option<String>,
    /// Sort order for the output.
    pub sort_by: KeystoreListSortBy,
}

/// Typed request for `local.keystore.delete`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeystoreDeleteRequest {
    /// Optional exact key id.
    #[serde(default)]
    pub id: Option<String>,
    /// Optional UTF-8 label selector.
    #[serde(default)]
    pub label: Option<String>,
    /// Optional hex-encoded UTF-8 label selector.
    #[serde(default)]
    pub label_hex: Option<String>,
    /// Whether confirmation has already been granted.
    pub confirm_yes: bool,
    /// Optional UTF-8 keystore path.
    #[serde(default)]
    pub store_path: Option<String>,
    /// Optional hex-encoded UTF-8 keystore path.
    #[serde(default)]
    pub store_path_hex: Option<String>,
}

/// Typed request for `local.keystore.tx_sign`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeystoreTxSignRequest {
    /// Optional exact key id.
    #[serde(default)]
    pub id: Option<String>,
    /// Optional UTF-8 label selector.
    #[serde(default)]
    pub label: Option<String>,
    /// Optional hex-encoded UTF-8 label selector.
    #[serde(default)]
    pub label_hex: Option<String>,
    /// Optional UTF-8 keystore path.
    #[serde(default)]
    pub store_path: Option<String>,
    /// Optional hex-encoded UTF-8 keystore path.
    #[serde(default)]
    pub store_path_hex: Option<String>,
    /// Optional UTF-8 output path.
    #[serde(default)]
    pub out_path: Option<String>,
    /// Optional hex-encoded UTF-8 output path.
    #[serde(default)]
    pub out_path_hex: Option<String>,
    /// Recipient address.
    pub to: String,
    /// Transfer value in wei.
    pub value_wei: String,
    /// Chain id for signing.
    pub chain_id: u64,
    /// Nonce for signing.
    pub nonce: u64,
    /// Max fee per gas.
    pub max_fee_per_gas: String,
    /// Max priority fee per gas.
    pub max_priority_fee_per_gas: String,
    /// Gas limit for the transaction.
    pub gas_limit: u64,
    /// 0x-prefixed calldata hex.
    pub data_hex: String,
}

/// Error returned when a fact key cannot be derived from a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactKeyDerivationError {
    /// The request could not be converted into JSON before hashing.
    Serialization(String),
    /// The request could not be canonically hashed for fact recording.
    NotCanonical(CanonicalJsonError),
}

impl std::fmt::Display for FactKeyDerivationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FactKeyDerivationError::Serialization(err) => {
                write!(f, "request could not be serialized to json: {err}")
            }
            FactKeyDerivationError::NotCanonical(err) => write!(f, "request not canonical: {err}"),
        }
    }
}

impl std::error::Error for FactKeyDerivationError {}

fn info(code: &'static str, category: ErrorCategory, message: &'static str) -> ErrorInfo {
    ErrorInfo {
        code: ErrorCode(code.to_string()),
        category,
        retryable: false,
        message: message.to_string(),
        details: None,
    }
}

fn io_other(code: &'static str, category: ErrorCategory, message: &'static str) -> IoError {
    IoError::Other(info(code, category, message))
}

fn request_to_value<Request: Serialize>(
    request: &Request,
) -> Result<serde_json::Value, FactKeyDerivationError> {
    serde_json::to_value(request)
        .map_err(|err| FactKeyDerivationError::Serialization(err.to_string()))
}

fn fact_key_for_request_value(
    state_id: &StateId,
    purpose: &str,
    request: &serde_json::Value,
) -> Result<FactKey, FactKeyDerivationError> {
    let req_id = artifact_id_for_json(request).map_err(FactKeyDerivationError::NotCanonical)?;
    Ok(FactKey(format!(
        "mfm:local|state:{}|purpose:{purpose}|req:{}",
        state_id.as_str(),
        req_id.as_str()
    )))
}

fn keystore_io_call(
    namespace: &'static str,
    request: serde_json::Value,
    fact_key: FactKey,
) -> IoCall {
    IoCall {
        namespace: namespace.to_string(),
        request,
        fact_key: Some(fact_key),
    }
}

/// First-class local keystore client wrapper over `IoProvider`.
pub struct LocalKeystoreIoClient<'a> {
    state_id: StateId,
    io: &'a mut dyn IoProvider,
}

impl<'a> LocalKeystoreIoClient<'a> {
    /// Creates a new client for the given state and IO provider.
    pub fn new(state_id: StateId, io: &'a mut dyn IoProvider) -> Self {
        Self { state_id, io }
    }

    async fn call<Response, Request>(
        &mut self,
        namespace: &'static str,
        purpose: &str,
        request: Request,
    ) -> Result<Response, IoError>
    where
        Response: DeserializeOwned,
        Request: Serialize,
    {
        let request = request_to_value(&request).map_err(|err| match err {
            FactKeyDerivationError::Serialization(_) => io_other(
                "local_request_serialize_failed",
                ErrorCategory::ParsingInput,
                "local keystore request could not be serialized to json",
            ),
            FactKeyDerivationError::NotCanonical(_) => {
                unreachable!("request_to_value only returns serialization errors")
            }
        })?;
        let fact_key = fact_key_for_request_value(&self.state_id, purpose, &request).map_err(|err| {
            match err {
                FactKeyDerivationError::Serialization(_) => unreachable!(
                    "fact_key_for_request_value only hashes already-serialized json values"
                ),
                FactKeyDerivationError::NotCanonical(CanonicalJsonError::FloatNotAllowed) => {
                    io_other(
                        "local_request_not_canonical",
                        ErrorCategory::ParsingInput,
                        "local keystore request was not canonical-json-hashable (floats are forbidden)",
                    )
                }
                FactKeyDerivationError::NotCanonical(CanonicalJsonError::SecretsNotAllowed) => {
                    io_other(
                        "secrets_detected",
                        ErrorCategory::Unknown,
                        "local keystore request contained secrets",
                    )
                }
            }
        })?;

        let result = self
            .io
            .call(keystore_io_call(namespace, request, fact_key))
            .await?;
        serde_json::from_value(result.response).map_err(|_| {
            io_other(
                "local_response_invalid",
                ErrorCategory::Unknown,
                "local keystore response payload had an unexpected shape",
            )
        })
    }

    /// Imports a key into the local keystore transport.
    pub async fn import<Response: DeserializeOwned>(
        &mut self,
        purpose: &str,
        request: KeystoreImportRequest,
    ) -> Result<Response, IoError> {
        self.call(NAMESPACE_LOCAL_KEYSTORE_IMPORT, purpose, request)
            .await
    }

    /// Lists keys through the local keystore transport.
    pub async fn list<Response: DeserializeOwned>(
        &mut self,
        purpose: &str,
        request: KeystoreListRequest,
    ) -> Result<Response, IoError> {
        self.call(NAMESPACE_LOCAL_KEYSTORE_LIST, purpose, request)
            .await
    }

    /// Deletes a key through the local keystore transport.
    pub async fn delete<Response: DeserializeOwned>(
        &mut self,
        purpose: &str,
        request: KeystoreDeleteRequest,
    ) -> Result<Response, IoError> {
        self.call(NAMESPACE_LOCAL_KEYSTORE_DELETE, purpose, request)
            .await
    }

    /// Signs a transaction through the local keystore transport.
    pub async fn tx_sign<Response: DeserializeOwned>(
        &mut self,
        purpose: &str,
        request: KeystoreTxSignRequest,
    ) -> Result<Response, IoError> {
        self.call(NAMESPACE_LOCAL_KEYSTORE_TX_SIGN, purpose, request)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use mfm_machine::ids::ArtifactId;
    use std::collections::BTreeMap;

    struct PanicIo;

    #[async_trait]
    impl IoProvider for PanicIo {
        async fn call(&mut self, _call: IoCall) -> Result<mfm_machine::io::IoResult, IoError> {
            panic!("local keystore client should fail before reaching the io provider")
        }

        async fn record_value(
            &mut self,
            _key: FactKey,
            _value: serde_json::Value,
        ) -> Result<ArtifactId, IoError> {
            Ok(ArtifactId::must_new("0".repeat(64)))
        }

        async fn get_recorded_fact(
            &mut self,
            _key: &FactKey,
        ) -> Result<Option<ArtifactId>, IoError> {
            Ok(None)
        }

        async fn now_millis(&mut self) -> Result<u64, IoError> {
            Ok(0)
        }

        async fn random_bytes(&mut self, _n: usize) -> Result<Vec<u8>, IoError> {
            Ok(Vec::new())
        }

        async fn sleep_ms(&mut self, _duration_ms: u64) -> Result<(), IoError> {
            Ok(())
        }
    }

    #[test]
    fn request_to_value_reports_serialization_failure() {
        let mut request = BTreeMap::new();
        request.insert((1u8, 2u8), 3u8);

        let err = request_to_value(&request)
            .expect_err("tuple-key map should not serialize to json objects");

        match err {
            FactKeyDerivationError::Serialization(message) => {
                assert!(
                    !message.is_empty(),
                    "serialization error should include context"
                );
            }
            other => panic!("unexpected fact-key derivation error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn local_keystore_client_returns_typed_error_on_request_serialization_failure() {
        let mut io = PanicIo;
        let mut client = LocalKeystoreIoClient::new(
            StateId::must_new("local_keystore.tests.client".to_string()),
            &mut io,
        );
        let mut request = BTreeMap::new();
        request.insert((1u8, 2u8), 3u8);

        let err = client
            .call::<serde_json::Value, _>(
                NAMESPACE_LOCAL_KEYSTORE_IMPORT,
                "local.keystore.import",
                request,
            )
            .await
            .expect_err("bad json serialization should be surfaced as an io error");

        match err {
            IoError::Other(info) => assert_eq!(info.code.0, "local_request_serialize_failed"),
            other => panic!("unexpected io error: {other:?}"),
        }
    }

    #[test]
    fn import_request_bip39_extra_metadata_is_fact_key_safe() {
        let request = KeystoreImportRequest {
            kind: KeystoreImportType::Mnemonic,
            label: None,
            label_hex: Some("77616c6c6574".to_string()),
            derive_path: "m/44'/60'/0'/0/0".to_string(),
            store_path: None,
            store_path_hex: Some("2f746d702f6b657973746f7265".to_string()),
            stdin_mode: true,
            bip39_extra: Bip39ExtraSource::FilePathHex(
                "2f746d702f62697033392d6578747261".to_string(),
            ),
        };

        let value = request_to_value(&request).expect("request should serialize to json");
        let rendered = value.to_string();
        assert!(rendered.contains("bip39_extra"));
        assert!(!rendered.contains("passphrase"));

        fact_key_for_request_value(
            &StateId::must_new("local_keystore.tests.import".to_string()),
            "local.keystore.import",
            &value,
        )
        .expect("non-secret source metadata should be fact-key safe");
    }

    #[tokio::test]
    async fn local_keystore_client_rejects_secret_shaped_import_request() {
        let mut io = PanicIo;
        let mut client = LocalKeystoreIoClient::new(
            StateId::must_new("local_keystore.tests.client".to_string()),
            &mut io,
        );

        let request = serde_json::json!({
            "kind": "mn",
            "derive_path": "m/44'/60'/0'/0/0",
            "store_path_hex": "2f746d702f6b657973746f7265",
            "stdin_mode": true,
            "passphrase": "do-not-serialize"
        });

        let err = client
            .call::<serde_json::Value, _>(
                NAMESPACE_LOCAL_KEYSTORE_IMPORT,
                "local.keystore.import",
                request,
            )
            .await
            .expect_err("secret-shaped request should fail before io");

        match err {
            IoError::Other(info) => assert_eq!(info.code.0, "secrets_detected"),
            other => panic!("unexpected io error: {other:?}"),
        }
    }
}
