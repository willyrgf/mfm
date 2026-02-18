use alloy_primitives::keccak256;
use async_trait::async_trait;
use k256::ecdsa::SigningKey;
use mfm_evm_core::hex::{
    bytes_to_hex_prefixed, hex_to_bytes, normalize_hex_str, normalize_nonempty_hex_str,
};
use mfm_evm_core::rlp::{rlp_encode_list, trim_leading_zero_bytes, u128_to_min_be};
use mfm_machine::errors::{ErrorCategory, IoError};
use mfm_machine::io::IoCall;
use mfm_machine::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};
use mfm_op_common::local_transport::{
    decode_hex_utf8, encode_response, io_other, parse_request, LocalTransportError,
};
use serde::Deserialize;
use zeroize::Zeroizing;

#[derive(Clone, Default)]
pub struct LocalEvmIoTransportFactory;

impl LiveIoTransportFactory for LocalEvmIoTransportFactory {
    fn make(&self, _env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
        Box::new(LocalEvmIoTransport)
    }
}

struct LocalEvmIoTransport;

#[async_trait]
impl LiveIoTransport for LocalEvmIoTransport {
    async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
        match call.namespace.as_str() {
            "local.evm.signer_address" => handle_evm_signer_address(call.request),
            "local.evm.sign_legacy_create" => handle_evm_sign_legacy_create(call.request),
            _ => Err(io_other(
                "unknown_namespace",
                ErrorCategory::Unknown,
                "unknown local evm io namespace",
            )),
        }
    }
}

#[derive(Debug, Deserialize)]
struct EvmSignLegacyCreateRequest {
    env_name_hex: String,
    from: String,
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

fn handle_evm_sign_legacy_create(request: serde_json::Value) -> Result<serde_json::Value, IoError> {
    let req: EvmSignLegacyCreateRequest = parse_request(request)?;
    let response = evm_sign_legacy_create(req).map_err(LocalError::into_io)?;
    encode_response(response)
}

fn handle_evm_signer_address(request: serde_json::Value) -> Result<serde_json::Value, IoError> {
    let req: EvmSignerAddressRequest = parse_request(request)?;
    let response = evm_signer_address(req).map_err(LocalError::into_io)?;
    encode_response(response)
}

fn evm_sign_legacy_create(
    req: EvmSignLegacyCreateRequest,
) -> Result<serde_json::Value, LocalError> {
    let signing_key = signing_key_from_env_name_hex(&req.env_name_hex)?;
    let signer_addr = signer_address_hex(&signing_key);
    let configured_from = normalize_address(&req.from).map_err(|_| {
        LocalError::new(
            "invalid_op_config",
            ErrorCategory::ParsingInput,
            "invalid from address",
        )
    })?;

    if signer_addr != configured_from {
        return Err(LocalError::new(
            "signing_key_address_mismatch",
            ErrorCategory::ParsingInput,
            "signing key did not match configured from address",
        ));
    }

    let data_hex = normalize_hex_str(&req.data_hex).map_err(|_| {
        LocalError::new(
            "invalid_op_config",
            ErrorCategory::ParsingInput,
            "invalid deployment data hex",
        )
    })?;
    let data_bytes = hex_to_bytes(&data_hex).map_err(|_| {
        LocalError::new(
            "invalid_op_config",
            ErrorCategory::ParsingInput,
            "invalid deployment data hex",
        )
    })?;

    let raw_tx_hex = sign_legacy_create_raw_tx(
        &signing_key,
        req.chain_id,
        &req.nonce_hex,
        &req.gas_price_hex,
        &req.gas_limit_hex,
        &req.value_hex,
        &data_bytes,
    )?;

    Ok(serde_json::json!({ "raw_tx_hex": raw_tx_hex }))
}

fn evm_signer_address(req: EvmSignerAddressRequest) -> Result<serde_json::Value, LocalError> {
    let signing_key = signing_key_from_env_name_hex(&req.env_name_hex)?;
    Ok(serde_json::json!({
        "address": signer_address_hex(&signing_key),
    }))
}

fn signing_key_from_env_name_hex(env_name_hex: &str) -> Result<SigningKey, LocalError> {
    let env_name = decode_hex_utf8(env_name_hex, "invalid_op_config", "env_name_hex")?;
    let raw = Zeroizing::new(std::env::var(&env_name).map_err(|_| {
        LocalError::new(
            "missing_signing_key_env",
            ErrorCategory::Unknown,
            "signing key env was not configured",
        )
    })?);
    signing_key_from_hex(raw.as_str())
}

fn signing_key_from_hex(raw: &str) -> Result<SigningKey, LocalError> {
    let normalized = Zeroizing::new(normalize_hex_str(raw).map_err(|_| {
        LocalError::new(
            "invalid_signing_key_env",
            ErrorCategory::ParsingInput,
            "signing key hex was invalid",
        )
    })?);
    let bytes = Zeroizing::new(hex_to_bytes(normalized.as_str()).map_err(|_| {
        LocalError::new(
            "invalid_signing_key_env",
            ErrorCategory::ParsingInput,
            "signing key hex was invalid",
        )
    })?);
    if bytes.len() != 32 {
        return Err(LocalError::new(
            "invalid_signing_key_env",
            ErrorCategory::ParsingInput,
            "signing key must be exactly 32 bytes",
        ));
    }

    let mut key = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(bytes.as_slice());
    SigningKey::from_bytes((&*key).into()).map_err(|_| {
        LocalError::new(
            "invalid_signing_key_env",
            ErrorCategory::ParsingInput,
            "signing key did not form a valid secp256k1 key",
        )
    })
}

fn signer_address_hex(signing_key: &SigningKey) -> String {
    let public_key = signing_key.verifying_key().to_encoded_point(false);
    let hash = keccak256(&public_key.as_bytes()[1..]);
    bytes_to_hex_prefixed(&hash.as_slice()[12..])
}

fn sign_legacy_create_raw_tx(
    signing_key: &SigningKey,
    chain_id: u64,
    nonce_hex: &str,
    gas_price_hex: &str,
    gas_limit_hex: &str,
    value_hex: &str,
    data: &[u8],
) -> Result<String, LocalError> {
    let nonce = hex_quantity_to_rlp_bytes(nonce_hex)?;
    let gas_price = hex_quantity_to_rlp_bytes(gas_price_hex)?;
    let gas_limit = hex_quantity_to_rlp_bytes(gas_limit_hex)?;
    let value = hex_quantity_to_rlp_bytes(value_hex)?;
    let chain_id_bytes = u128_to_min_be(u128::from(chain_id));

    let unsigned = rlp_encode_list(&[
        nonce.clone(),
        gas_price.clone(),
        gas_limit.clone(),
        Vec::new(),
        value.clone(),
        data.to_vec(),
        chain_id_bytes.clone(),
        Vec::new(),
        Vec::new(),
    ]);

    let sighash = keccak256(&unsigned);
    let (sig, recid) = signing_key
        .sign_prehash_recoverable(sighash.as_slice())
        .map_err(|_| {
            LocalError::new(
                "signing_failed",
                ErrorCategory::Unknown,
                "failed to sign deployment transaction",
            )
        })?;
    let sig_bytes = sig.to_bytes();
    let r = trim_leading_zero_bytes(&sig_bytes[..32]);
    let s = trim_leading_zero_bytes(&sig_bytes[32..]);
    let v = u128::from(chain_id) * 2 + 35 + u128::from(u8::from(recid));
    let v_bytes = u128_to_min_be(v);

    let signed = rlp_encode_list(&[
        nonce,
        gas_price,
        gas_limit,
        Vec::new(),
        value,
        data.to_vec(),
        v_bytes,
        r,
        s,
    ]);
    Ok(bytes_to_hex_prefixed(&signed))
}

fn hex_quantity_to_rlp_bytes(value: &str) -> Result<Vec<u8>, LocalError> {
    let normalized = normalize_nonempty_hex_str(value).map_err(|_| {
        LocalError::new(
            "invalid_op_config",
            ErrorCategory::ParsingInput,
            "invalid transaction quantity hex",
        )
    })?;
    let bytes = hex_to_bytes(&normalized).map_err(|_| {
        LocalError::new(
            "invalid_op_config",
            ErrorCategory::ParsingInput,
            "invalid transaction quantity hex",
        )
    })?;
    Ok(trim_leading_zero_bytes(&bytes))
}

fn normalize_address(raw: &str) -> Result<String, ()> {
    let normalized = normalize_hex_str(raw).map_err(|_| ())?;
    if normalized.len() != 42 {
        return Err(());
    }
    Ok(normalized.to_ascii_lowercase())
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

        let request = EvmSignLegacyCreateRequest {
            env_name_hex: hex::encode(env_name.as_bytes()),
            from: "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf".to_string(),
            chain_id: 1,
            nonce_hex: "0x0".to_string(),
            gas_price_hex: "0x1".to_string(),
            gas_limit_hex: "0x5208".to_string(),
            value_hex: "0x0".to_string(),
            data_hex: "0x6000".to_string(),
        };

        let out = evm_sign_legacy_create(request).expect("sign");
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

        let request = EvmSignLegacyCreateRequest {
            env_name_hex: hex::encode(env_name.as_bytes()),
            from: "0x1111111111111111111111111111111111111111".to_string(),
            chain_id: 1,
            nonce_hex: "0x0".to_string(),
            gas_price_hex: "0x1".to_string(),
            gas_limit_hex: "0x5208".to_string(),
            value_hex: "0x0".to_string(),
            data_hex: "0x6000".to_string(),
        };

        let err = evm_sign_legacy_create(request).expect_err("mismatch should fail");
        assert_eq!(err.code, "signing_key_address_mismatch");

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
