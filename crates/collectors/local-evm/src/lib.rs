#![warn(missing_docs)]
//! Typed adapters for the local EVM helper IO namespaces.
//!
//! This crate owns the state-facing `IoProvider` client and deterministic fact-key derivation for
//! `local.evm.*`. Live private-key loading and signing remain in `mfm-transports-local-evm`.
//!
//! # Examples
//!
//! ```rust
//! use mfm_collectors_local_evm::{
//!     LocalEvmSignLegacyCreateCall, NAMESPACE_LOCAL_EVM_SIGN_LEGACY_CREATE,
//! };
//!
//! let call = LocalEvmSignLegacyCreateCall {
//!     signing_key_env: "MFM_SIGNING_KEY".to_string(),
//!     from: "0x0000000000000000000000000000000000000000".to_string(),
//!     chain_id: 1,
//!     nonce_hex: "0x0".to_string(),
//!     gas_price_hex: "0x1".to_string(),
//!     gas_limit_hex: "0x5208".to_string(),
//!     value_hex: "0x0".to_string(),
//!     data_hex: "0x".to_string(),
//! };
//!
//! assert_eq!(NAMESPACE_LOCAL_EVM_SIGN_LEGACY_CREATE, "local.evm.sign_legacy_create");
//! assert_eq!(call.chain_id, 1);
//! ```

use mfm_evm_core::hex::normalize_hex_str;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::hashing::{artifact_id_for_json, CanonicalJsonError};
use mfm_machine::ids::{ErrorCode, FactKey, StateId};
use mfm_machine::io::{IoCall, IoProvider};

/// Namespace group handled by local EVM helper transports.
pub const NAMESPACE_LOCAL_EVM: &str = "local.evm";
/// Namespace for resolving the address of a local signer.
pub const NAMESPACE_LOCAL_EVM_SIGNER_ADDRESS: &str = "local.evm.signer_address";
/// Namespace for locally signing a legacy contract-creation transaction.
pub const NAMESPACE_LOCAL_EVM_SIGN_LEGACY_CREATE: &str = "local.evm.sign_legacy_create";
/// Namespace for locally signing a legacy contract-call transaction.
pub const NAMESPACE_LOCAL_EVM_SIGN_LEGACY_CALL: &str = "local.evm.sign_legacy_call";

/// Request shape for signing a legacy contract-creation transaction locally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalEvmSignLegacyCreateCall {
    /// Environment variable containing the private key hex.
    pub signing_key_env: String,
    /// Expected sender address for the signing key.
    pub from: String,
    /// Chain ID used for replay protection.
    pub chain_id: u64,
    /// Nonce encoded as a hex quantity.
    pub nonce_hex: String,
    /// Gas price encoded as a hex quantity.
    pub gas_price_hex: String,
    /// Gas limit encoded as a hex quantity.
    pub gas_limit_hex: String,
    /// Value encoded as a hex quantity.
    pub value_hex: String,
    /// Deployment calldata encoded as hex.
    pub data_hex: String,
}

/// Request shape for signing a legacy contract call transaction locally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalEvmSignLegacyCallCall {
    /// Environment variable containing the private key hex.
    pub signing_key_env: String,
    /// Expected sender address for the signing key.
    pub from: String,
    /// Target address for the transaction.
    pub to: String,
    /// Chain ID used for replay protection.
    pub chain_id: u64,
    /// Nonce encoded as a hex quantity.
    pub nonce_hex: String,
    /// Gas price encoded as a hex quantity.
    pub gas_price_hex: String,
    /// Gas limit encoded as a hex quantity.
    pub gas_limit_hex: String,
    /// Value encoded as a hex quantity.
    pub value_hex: String,
    /// Call data encoded as hex.
    pub data_hex: String,
}

/// Thin typed client for the `local.evm.*` helper namespaces.
pub struct LocalEvmIoClient<'a> {
    state_id: StateId,
    io: &'a mut dyn IoProvider,
}

impl<'a> LocalEvmIoClient<'a> {
    /// Creates a new local EVM client for the given state and IO provider.
    pub fn new(state_id: StateId, io: &'a mut dyn IoProvider) -> Self {
        Self { state_id, io }
    }

    fn fact_key(&self, purpose: &str, request: &serde_json::Value) -> Result<FactKey, IoError> {
        let req_id = artifact_id_for_json(request).map_err(|e| match e {
            CanonicalJsonError::FloatNotAllowed => io_other(
                "local_request_not_canonical",
                ErrorCategory::ParsingInput,
                "local io request was not canonical-json-hashable (floats are forbidden)",
            ),
            CanonicalJsonError::SecretsNotAllowed => io_other(
                "secrets_detected",
                ErrorCategory::Unknown,
                "local io request contained secrets (policy forbids persisting secrets)",
            ),
        })?;
        Ok(FactKey(format!(
            "mfm:local|state:{}|purpose:{purpose}|req:{}",
            self.state_id.as_str(),
            req_id.as_str()
        )))
    }

    /// Resolves the signer address for a configured private-key environment variable.
    pub async fn signer_address(&mut self, signing_key_env: &str) -> Result<String, IoError> {
        let request = serde_json::json!({
            "env_name_hex": hex::encode(signing_key_env.as_bytes()),
        });
        let fact_key = self.fact_key("resolve_signing_key_address", &request)?;
        let result = self
            .io
            .call(IoCall {
                namespace: NAMESPACE_LOCAL_EVM_SIGNER_ADDRESS.to_string(),
                request,
                fact_key: Some(fact_key),
            })
            .await?;
        let address = result
            .response
            .get("address")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                io_other(
                    "evm_response_invalid",
                    ErrorCategory::ParsingInput,
                    "local signer returned non-string address",
                )
            })?;

        normalize_address(address).map_err(|_| {
            io_other(
                "evm_response_invalid",
                ErrorCategory::ParsingInput,
                "local signer returned invalid address",
            )
        })
    }

    /// Signs a legacy contract-creation transaction through the local transport.
    pub async fn sign_legacy_create(
        &mut self,
        req: LocalEvmSignLegacyCreateCall,
    ) -> Result<String, IoError> {
        let request = serde_json::json!({
            "env_name_hex": hex::encode(req.signing_key_env.as_bytes()),
            "from": req.from,
            "chain_id": req.chain_id,
            "nonce_hex": req.nonce_hex,
            "gas_price_hex": req.gas_price_hex,
            "gas_limit_hex": req.gas_limit_hex,
            "value_hex": req.value_hex,
            "data_hex": req.data_hex,
        });
        let result = self
            .io
            .call(IoCall {
                namespace: NAMESPACE_LOCAL_EVM_SIGN_LEGACY_CREATE.to_string(),
                request,
                fact_key: None,
            })
            .await?;
        raw_tx_hex_from_response(&result.response)
    }

    /// Signs a legacy contract call transaction through the local transport.
    pub async fn sign_legacy_call(
        &mut self,
        req: LocalEvmSignLegacyCallCall,
    ) -> Result<String, IoError> {
        let request = serde_json::json!({
            "env_name_hex": hex::encode(req.signing_key_env.as_bytes()),
            "from": req.from,
            "to": req.to,
            "chain_id": req.chain_id,
            "nonce_hex": req.nonce_hex,
            "gas_price_hex": req.gas_price_hex,
            "gas_limit_hex": req.gas_limit_hex,
            "value_hex": req.value_hex,
            "data_hex": req.data_hex,
        });
        let result = self
            .io
            .call(IoCall {
                namespace: NAMESPACE_LOCAL_EVM_SIGN_LEGACY_CALL.to_string(),
                request,
                fact_key: None,
            })
            .await?;
        raw_tx_hex_from_response(&result.response)
    }
}

fn raw_tx_hex_from_response(response: &serde_json::Value) -> Result<String, IoError> {
    let raw_tx_hex = response
        .get("raw_tx_hex")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            io_other(
                "evm_response_invalid",
                ErrorCategory::ParsingInput,
                "local signer returned non-string raw transaction",
            )
        })?;

    normalize_hex_str(raw_tx_hex).map_err(|_| {
        io_other(
            "evm_response_invalid",
            ErrorCategory::ParsingInput,
            "local signer returned invalid raw transaction hex",
        )
    })
}

fn normalize_address(raw: &str) -> Result<String, ()> {
    let normalized = normalize_hex_str(raw).map_err(|_| ())?;
    if normalized.len() != 42 {
        return Err(());
    }
    Ok(normalized.to_ascii_lowercase())
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

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use mfm_machine::ids::ArtifactId;
    use mfm_machine::io::IoResult;

    struct CapturingIo {
        response: serde_json::Value,
        calls: Vec<IoCall>,
    }

    #[async_trait]
    impl IoProvider for CapturingIo {
        async fn call(&mut self, call: IoCall) -> Result<IoResult, IoError> {
            self.calls.push(call);
            Ok(IoResult {
                response: self.response.clone(),
                recorded_payload_id: None,
            })
        }

        async fn record_value(
            &mut self,
            _key: FactKey,
            _value: serde_json::Value,
        ) -> Result<ArtifactId, IoError> {
            unreachable!("local evm client records through call fact keys only")
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

        async fn random_bytes(&mut self, n: usize) -> Result<Vec<u8>, IoError> {
            Ok(vec![0; n])
        }

        async fn sleep_ms(&mut self, _duration_ms: u64) -> Result<(), IoError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn signer_address_fact_request_omits_raw_env_name() {
        let mut io = CapturingIo {
            response: serde_json::json!({
                "address": "0x7E5f4552091A69125D5DfCb7b8C2659029395Bdf",
            }),
            calls: Vec::new(),
        };
        let mut client =
            LocalEvmIoClient::new(StateId::must_new("evm.write.signer".to_string()), &mut io);

        let address = client
            .signer_address("MFM_TEST_PRIVATE_KEY_SECRET")
            .await
            .expect("signer address");

        assert_eq!(address, "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf");
        let call = io.calls.pop().expect("io call");
        assert_eq!(call.namespace, NAMESPACE_LOCAL_EVM_SIGNER_ADDRESS);
        assert!(call.fact_key.is_some());
        let rendered = serde_json::to_string(&call.request).expect("request json");
        assert!(!rendered.contains("MFM_TEST_PRIVATE_KEY_SECRET"));
        assert_eq!(
            call.request.get("env_name_hex").and_then(|v| v.as_str()),
            Some(hex::encode("MFM_TEST_PRIVATE_KEY_SECRET").as_str())
        );
    }

    #[tokio::test]
    async fn signing_calls_do_not_record_raw_signed_transactions_as_facts() {
        let mut io = CapturingIo {
            response: serde_json::json!({ "raw_tx_hex": "0x01" }),
            calls: Vec::new(),
        };
        let mut client =
            LocalEvmIoClient::new(StateId::must_new("evm.write.sign".to_string()), &mut io);

        let _ = client
            .sign_legacy_create(LocalEvmSignLegacyCreateCall {
                signing_key_env: "MFM_SIGNING_KEY".to_string(),
                from: "0x0000000000000000000000000000000000000000".to_string(),
                chain_id: 1,
                nonce_hex: "0x0".to_string(),
                gas_price_hex: "0x1".to_string(),
                gas_limit_hex: "0x5208".to_string(),
                value_hex: "0x0".to_string(),
                data_hex: "0x".to_string(),
            })
            .await
            .expect("sign create");

        let call = io.calls.pop().expect("io call");
        assert_eq!(call.namespace, NAMESPACE_LOCAL_EVM_SIGN_LEGACY_CREATE);
        assert!(call.fact_key.is_none());
    }
}
