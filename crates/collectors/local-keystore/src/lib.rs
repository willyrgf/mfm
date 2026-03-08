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

/// Typed request for `local.keystore.import`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
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
    /// The request could not be canonically hashed for fact recording.
    NotCanonical(CanonicalJsonError),
}

impl std::fmt::Display for FactKeyDerivationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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

fn fact_key_for_request<Request: Serialize>(
    state_id: &StateId,
    purpose: &str,
    request: &Request,
) -> Result<FactKey, FactKeyDerivationError> {
    let request =
        serde_json::to_value(request).expect("local keystore request must serialize to json");
    let req_id = artifact_id_for_json(&request).map_err(FactKeyDerivationError::NotCanonical)?;
    Ok(FactKey(format!(
        "mfm:local|state:{}|purpose:{purpose}|req:{}",
        state_id.as_str(),
        req_id.0
    )))
}

fn keystore_io_call<Request: Serialize>(
    namespace: &'static str,
    request: Request,
    fact_key: FactKey,
) -> IoCall {
    let request =
        serde_json::to_value(request).expect("local keystore request must serialize to json");
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
        let fact_key = fact_key_for_request(&self.state_id, purpose, &request).map_err(|err| {
            match err {
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
