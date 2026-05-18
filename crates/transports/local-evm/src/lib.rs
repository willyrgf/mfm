#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
//! Local EVM helper transport for signing and address derivation.
//!
//! This crate exposes a `LiveIoTransportFactory` for the `local.evm.*` namespaces used by shared
//! runtime states. The transport reads signing keys from the local environment and never persists
//! the secret material itself. State-facing typed requests live in `mfm-collectors-local-evm`.
//!
//! # Examples
//!
//! ```rust
//! use mfm_machine::live_io::LiveIoTransportFactory;
//! use mfm_collectors_local_evm::NAMESPACE_LOCAL_EVM;
//! use mfm_transports_local_evm::LocalEvmIoTransportFactory;
//!
//! let factory = LocalEvmIoTransportFactory;
//!
//! assert_eq!(factory.namespace_group(), NAMESPACE_LOCAL_EVM);
//! ```
#![warn(missing_docs)]

use async_trait::async_trait;
use mfm_collectors_local_evm::{
    NAMESPACE_LOCAL_EVM, NAMESPACE_LOCAL_EVM_SIGNER_ADDRESS, NAMESPACE_LOCAL_EVM_SIGN_LEGACY,
};
use mfm_core::crypto::{EthereumKeyError, EthereumPrivateKey};
use mfm_evm_core::tx::{
    encode_signed_legacy_tx_hex, legacy_signing_hash, parse_address, parse_data_hex,
    parse_u128_quantity, parse_u64_quantity, LegacyTxToSign,
};
use mfm_evm_core::util_error::UtilError;
use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
use mfm_machine::ids::ErrorCode;
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use zeroize::Zeroizing;

/// Transport factory for local EVM helper namespaces.
#[derive(Clone, Default)]
pub struct LocalEvmIoTransportFactory;

impl LiveIoTransportFactory for LocalEvmIoTransportFactory {
    fn namespace_group(&self) -> &str {
        NAMESPACE_LOCAL_EVM
    }

    fn make(&self, _env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(LocalEvmIoTransport)
    }
}

struct LocalEvmIoTransport;

#[async_trait]
impl LiveIoTransport for LocalEvmIoTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        match call.namespace.as_str() {
            NAMESPACE_LOCAL_EVM_SIGNER_ADDRESS => handle_evm_signer_address(call.request),
            NAMESPACE_LOCAL_EVM_SIGN_LEGACY => handle_evm_sign_legacy(call.request),
            _ => Err(io_other(
                "unknown_namespace",
                ErrorCategory::Unknown,
                "unknown local evm io namespace",
            )),
        }
    }
}

#[derive(Debug, Deserialize)]
struct EvmSignLegacyCallRequest {
    env_name_hex: String,
    from: String,
    to: Option<String>,
    chain_id: u64,
    nonce_hex: String,
    gas_price_hex: String,
    gas_limit_hex: String,
    value_hex: String,
    data_hex: String,
}

#[derive(Debug, Deserialize)]
struct EvmSignerAddressRequest {
    env_name_hex: String,
}

type LocalError = LocalTransportError;

fn handle_evm_sign_legacy(request: serde_json::Value) -> Result<serde_json::Value, IoError> {
    let req: EvmSignLegacyCallRequest = parse_request(request)?;
    let response = evm_sign_legacy(req).map_err(LocalError::into_io)?;
    encode_response(response)
}

fn handle_evm_signer_address(request: serde_json::Value) -> Result<serde_json::Value, IoError> {
    let req: EvmSignerAddressRequest = parse_request(request)?;
    let response = evm_signer_address(req).map_err(LocalError::into_io)?;
    encode_response(response)
}

fn evm_sign_legacy(req: EvmSignLegacyCallRequest) -> Result<serde_json::Value, LocalError> {
    let signing_key = signing_key_from_env_name_hex(&req.env_name_hex)?;
    let signer_addr = signing_key
        .address()
        .map_err(local_error_from_ethereum_key)?;
    let configured_from = parse_address(&req.from, "from").map_err(local_error_from_util)?;

    if signer_addr != configured_from {
        return Err(LocalError::new(
            "signing_key_address_mismatch",
            ErrorCategory::ParsingInput,
            "signing key did not match configured from address",
        ));
    }

    let tx = LegacyTxToSign {
        to: req
            .to
            .as_deref()
            .map(|to| parse_address(to, "to"))
            .transpose()
            .map_err(local_error_from_util)?,
        value_wei: parse_u128_quantity(&req.value_hex, "value").map_err(local_error_from_util)?,
        chain_id: req.chain_id,
        nonce: parse_u64_quantity(&req.nonce_hex, "nonce").map_err(local_error_from_util)?,
        gas_price_wei: parse_u128_quantity(&req.gas_price_hex, "gas-price")
            .map_err(local_error_from_util)?,
        gas_limit: parse_u64_quantity(&req.gas_limit_hex, "gas-limit")
            .map_err(local_error_from_util)?,
        data: parse_data_hex(&req.data_hex).map_err(local_error_from_util)?,
    };
    let hash = legacy_signing_hash(&tx);
    let mut hash_bytes = [0u8; 32];
    hash_bytes.copy_from_slice(hash.as_slice());
    let signature = signing_key
        .sign_hash_recoverable(&hash_bytes)
        .map_err(local_error_from_ethereum_key)?;
    let raw_tx_hex = encode_signed_legacy_tx_hex(&tx, signature);

    Ok(serde_json::json!({ "raw_tx_hex": raw_tx_hex }))
}

fn evm_signer_address(req: EvmSignerAddressRequest) -> Result<serde_json::Value, LocalError> {
    let signing_key = signing_key_from_env_name_hex(&req.env_name_hex)?;
    Ok(serde_json::json!({
        "address": format!("{:?}", signing_key.address().map_err(local_error_from_ethereum_key)?),
    }))
}

fn signing_key_from_env_name_hex(env_name_hex: &str) -> Result<EthereumPrivateKey, LocalError> {
    let env_name = decode_hex_utf8(env_name_hex, "invalid_op_config", "env_name_hex")?;
    let raw = Zeroizing::new(std::env::var(&env_name).map_err(|_| {
        LocalError::new(
            "missing_signing_key_env",
            ErrorCategory::Unknown,
            "signing key env was not configured",
        )
    })?);
    EthereumPrivateKey::from_hex_secret(raw.as_str()).map_err(local_error_from_ethereum_key)
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

fn local_error_from_util(err: UtilError) -> LocalError {
    LocalError::new(err.code, ErrorCategory::ParsingInput, err.message)
}

fn local_error_from_ethereum_key(err: EthereumKeyError) -> LocalError {
    match err {
        EthereumKeyError::InvalidHex => LocalError::new(
            "invalid_signing_key_env",
            ErrorCategory::ParsingInput,
            "signing key hex was invalid",
        ),
        EthereumKeyError::InvalidLength => LocalError::new(
            "invalid_signing_key_env",
            ErrorCategory::ParsingInput,
            "signing key must be exactly 32 bytes",
        ),
        EthereumKeyError::InvalidPrivateKey => LocalError::new(
            "invalid_signing_key_env",
            ErrorCategory::ParsingInput,
            "signing key did not form a valid secp256k1 key",
        ),
        EthereumKeyError::SigningFailed => LocalError::new(
            "signing_failed",
            ErrorCategory::Unknown,
            "failed to sign transaction",
        ),
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

fn decode_hex_utf8(
    raw: &str,
    code: &'static str,
    field: &'static str,
) -> Result<String, LocalTransportError> {
    let bytes = hex::decode(raw).map_err(|_| {
        LocalTransportError::new(
            code,
            ErrorCategory::ParsingInput,
            format!("{field} must be valid hex"),
        )
    })?;
    String::from_utf8(bytes).map_err(|_| {
        LocalTransportError::new(
            code,
            ErrorCategory::ParsingInput,
            format!("{field} did not decode to utf-8"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evm_sign_legacy_create_emits_raw_tx_hex() {
        let env_name = "MFM_TEST_LOCAL_SIGNING_KEY_VALID";
        std::env::set_var(
            env_name,
            "0x0000000000000000000000000000000000000000000000000000000000000001",
        );

        let request = EvmSignLegacyCallRequest {
            env_name_hex: hex::encode(env_name.as_bytes()),
            from: "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf".to_string(),
            to: None,
            chain_id: 1,
            nonce_hex: "0x0".to_string(),
            gas_price_hex: "0x1".to_string(),
            gas_limit_hex: "0x5208".to_string(),
            value_hex: "0x0".to_string(),
            data_hex: "0x6000".to_string(),
        };

        let out = evm_sign_legacy(request).expect("sign");
        let raw = out
            .get("raw_tx_hex")
            .and_then(|v| v.as_str())
            .expect("raw_tx_hex string");
        assert!(raw.starts_with("0x"));
        assert!(raw.len() > 2);

        std::env::remove_var(env_name);
    }

    #[test]
    fn evm_sign_legacy_call_emits_raw_tx_hex() {
        let env_name = "MFM_TEST_LOCAL_SIGNING_KEY_CALL_VALID";
        std::env::set_var(
            env_name,
            "0x0000000000000000000000000000000000000000000000000000000000000001",
        );

        let request = EvmSignLegacyCallRequest {
            env_name_hex: hex::encode(env_name.as_bytes()),
            from: "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf".to_string(),
            to: Some("0x1111111111111111111111111111111111111111".to_string()),
            chain_id: 1,
            nonce_hex: "0x0".to_string(),
            gas_price_hex: "0x1".to_string(),
            gas_limit_hex: "0x5208".to_string(),
            value_hex: "0x0".to_string(),
            data_hex: "0x".to_string(),
        };

        let out = evm_sign_legacy(request).expect("sign");
        let raw = out
            .get("raw_tx_hex")
            .and_then(|v| v.as_str())
            .expect("raw_tx_hex string");
        assert!(raw.starts_with("0x"));
        assert!(raw.len() > 2);

        std::env::remove_var(env_name);
    }

    #[test]
    fn evm_sign_legacy_create_rejects_address_mismatch() {
        let env_name = "MFM_TEST_LOCAL_SIGNING_KEY_MISMATCH";
        std::env::set_var(
            env_name,
            "0x0000000000000000000000000000000000000000000000000000000000000001",
        );

        let request = EvmSignLegacyCallRequest {
            env_name_hex: hex::encode(env_name.as_bytes()),
            from: "0x1111111111111111111111111111111111111111".to_string(),
            to: None,
            chain_id: 1,
            nonce_hex: "0x0".to_string(),
            gas_price_hex: "0x1".to_string(),
            gas_limit_hex: "0x5208".to_string(),
            value_hex: "0x0".to_string(),
            data_hex: "0x6000".to_string(),
        };

        let err = evm_sign_legacy(request).expect_err("mismatch should fail");
        assert_eq!(err.code, "signing_key_address_mismatch");

        std::env::remove_var(env_name);
    }

    #[test]
    fn invalid_signing_key_env_error_omits_env_name_and_value() {
        let env_name = "MFM_TEST_LOCAL_SIGNING_KEY_SECRET_ERROR";
        let env_value = "not-a-valid-private-key-secret";
        std::env::set_var(env_name, env_value);

        let err = signing_key_from_env_name_hex(&hex::encode(env_name.as_bytes()))
            .expect_err("invalid signing key must fail");
        assert_eq!(err.code, "invalid_signing_key_env");

        match err.into_io() {
            IoError::Other(info) => {
                let rendered = format!("{info:?}");
                assert!(!rendered.contains(env_name));
                assert!(!rendered.contains(env_value));
                assert!(info.details.is_none());
            }
            other => panic!("unexpected io error: {other:?}"),
        }

        std::env::remove_var(env_name);
    }

    #[test]
    fn evm_signer_address_returns_derived_address() {
        let env_name = "MFM_TEST_LOCAL_SIGNING_KEY_DERIVE_ADDRESS";
        std::env::set_var(
            env_name,
            "0x0000000000000000000000000000000000000000000000000000000000000001",
        );

        let request = EvmSignerAddressRequest {
            env_name_hex: hex::encode(env_name.as_bytes()),
        };

        let out = evm_signer_address(request).expect("derive signer address");
        assert_eq!(
            out.get("address").and_then(|v| v.as_str()),
            Some("0x7e5f4552091a69125d5dfcb7b8c2659029395bdf")
        );

        std::env::remove_var(env_name);
    }
}
