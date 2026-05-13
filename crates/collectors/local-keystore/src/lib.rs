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

/// Local file write policy for outputs created by local keystore operations.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LocalFileWriteMode {
    /// Create a new output and fail if the target already exists.
    #[default]
    CreateNew,
    /// Atomically replace an existing regular output file.
    Overwrite,
}

/// Typed request for `local.keystore.import`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct KeystoreImportRequest {
    /// Input kind to import.
    pub kind: KeystoreImportType,
    /// BIP-32 derivation path.
    pub derive_path: String,
    /// Opaque local binding registered with the live keystore transport.
    pub local_resource_handle: String,
}

/// Typed request for `local.keystore.list`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct KeystoreListRequest {
    /// Opaque local binding registered with the live keystore transport.
    pub local_resource_handle: String,
    /// Whether addresses should be included.
    pub show_addrs: bool,
    /// Sort order for the output.
    pub sort_by: KeystoreListSortBy,
}

/// Typed request for `local.keystore.delete`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct KeystoreDeleteRequest {
    /// Optional exact key id.
    #[serde(default)]
    pub id: Option<String>,
    /// Whether confirmation has already been granted.
    pub confirm_yes: bool,
    /// Opaque local binding registered with the live keystore transport.
    pub local_resource_handle: String,
}

/// Typed request for `local.keystore.tx_sign`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct KeystoreTxSignRequest {
    /// Optional exact key id.
    #[serde(default)]
    pub id: Option<String>,
    /// Opaque local binding registered with the live keystore transport.
    pub local_resource_handle: String,
    /// Write policy for the output path.
    #[serde(default)]
    pub out_write_mode: LocalFileWriteMode,
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
        code: ErrorCode::must_new(code),
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
            IoError::Other(info) => {
                assert_eq!(info.code.as_str(), "local_request_serialize_failed")
            }
            other => panic!("unexpected io error: {other:?}"),
        }
    }

    #[test]
    fn local_keystore_requests_do_not_serialize_local_paths_or_labels() {
        let request = serde_json::json!({
            "import": KeystoreImportRequest {
                kind: KeystoreImportType::Mnemonic,
                derive_path: "m/44'/60'/0'/0/0".to_string(),
                local_resource_handle: "local-keystore:test".to_string(),
            },
            "list": KeystoreListRequest {
                local_resource_handle: "local-keystore:test".to_string(),
                show_addrs: true,
                sort_by: KeystoreListSortBy::Created,
            },
            "delete": KeystoreDeleteRequest {
                id: Some("550e8400-e29b-41d4-a716-446655440000".to_string()),
                confirm_yes: true,
                local_resource_handle: "local-keystore:test".to_string(),
            },
            "tx_sign": KeystoreTxSignRequest {
                id: Some("550e8400-e29b-41d4-a716-446655440000".to_string()),
                local_resource_handle: "local-keystore:test".to_string(),
                out_write_mode: LocalFileWriteMode::CreateNew,
                to: "0x1111111111111111111111111111111111111111".to_string(),
                value_wei: "1".to_string(),
                chain_id: 1,
                nonce: 0,
                max_fee_per_gas: "2000000000".to_string(),
                max_priority_fee_per_gas: "1000000000".to_string(),
                gas_limit: 21_000,
                data_hex: "0x".to_string(),
            }
        });

        let value = request_to_value(&request).expect("request should serialize to json");
        let rendered = value.to_string();
        assert!(!rendered.contains("/tmp/keystore"));
        assert!(!rendered.contains("/tmp/bip39-extra"));
        assert!(!rendered.contains("store_path"));
        assert!(!rendered.contains("label_hex"));
        assert!(!rendered.contains("file_path"));
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
            "local_resource_handle": "local-keystore:test",
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
            IoError::Other(info) => assert_eq!(info.code.as_str(), "secrets_detected"),
            other => panic!("unexpected io error: {other:?}"),
        }
    }
}
