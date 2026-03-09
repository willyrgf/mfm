//! Helpers for parsing keystore transaction inputs and producing signed EIP-1559 payloads.
//!
//! These helpers sit below the reusable keystore transaction states and above the raw keystore
//! implementation. They normalize user/config input into canonical transaction data and stable
//! report keys without exposing secret-bearing details.
//!
//! # Examples
//!
//! ```rust
//! use mfm_state_keystore::tx::{output_context_key, parse_data_hex, validate_raw_transaction_hex};
//!
//! assert_eq!(parse_data_hex("0x").unwrap(), Vec::<u8>::new());
//! validate_raw_transaction_hex("0x00").unwrap();
//! assert_eq!(output_context_key("keystore_tx.sign").0, "keystore_tx.sign.report");
//! ```

use alloy_primitives::{keccak256, Address, PrimitiveSignature, B256};
use mfm_evm_core::rlp::{
    rlp_encode_bytes, rlp_encode_list_preencoded, trim_leading_zero_bytes, u128_to_min_be,
    u64_to_min_be,
};
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::ids::{ContextKey, StateId};
use mfm_op_keystore::Keystore;
use mfm_state_common::errors::state_error_with_state;
use url::Url;
use uuid::Uuid;

/// Error returned by keystore transaction helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeystoreTxError {
    /// Stable machine-readable error code.
    pub code: &'static str,
    /// Human-readable message that is safe to surface to callers.
    pub message: String,
}

impl KeystoreTxError {
    /// Creates a new helper error.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Converts this helper error into a state-scoped runtime error.
    pub fn to_state_error(&self, state_id: &StateId, category: ErrorCategory) -> StateError {
        state_error_with_state(
            state_id.clone(),
            self.code,
            category,
            false,
            self.message.clone(),
        )
    }
}

impl std::fmt::Display for KeystoreTxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for KeystoreTxError {}

/// Canonical EIP-1559 transaction fields required for signing.
///
/// This type is intentionally transport-agnostic: op planners can build it from CLI, API, or
/// replayed input before passing it into keystore-backed signing states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eip1559TxToSign {
    /// Destination address for the transaction.
    pub to: Address,
    /// Native token value in wei.
    pub value_wei: u128,
    /// Chain ID used for replay protection.
    pub chain_id: u64,
    /// Sender account nonce.
    pub nonce: u64,
    /// Maximum total gas price in wei.
    pub max_fee_per_gas: u128,
    /// Maximum priority fee in wei.
    pub max_priority_fee_per_gas: u128,
    /// Gas limit for the transaction.
    pub gas_limit: u64,
    /// ABI-encoded calldata bytes.
    pub data: Vec<u8>,
}

/// Result of signing an EIP-1559 transaction with a keystore key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedEip1559Tx {
    /// Hex-encoded sender address recovered from the signing key.
    pub from: String,
    /// Keccak-256 hash of the typed transaction preimage.
    pub payload_hash: String,
    /// Hex-encoded raw signed transaction ready for RPC submission.
    pub raw_tx_hex: String,
}

/// Parses a `0x`-prefixed Ethereum address.
pub fn parse_address(raw: &str, field_name: &str) -> Result<Address, KeystoreTxError> {
    raw.parse::<Address>().map_err(|_| {
        KeystoreTxError::new(
            "InvalidAddress",
            format!("{field_name} must be a valid 0x-prefixed Ethereum address"),
        )
    })
}

/// Parses a decimal or `0x`-prefixed quantity into `u128`.
pub fn parse_u128_quantity(raw: &str, field_name: &str) -> Result<u128, KeystoreTxError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(KeystoreTxError::new(
            "InvalidQuantity",
            format!("{field_name} cannot be empty"),
        ));
    }

    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        if hex.is_empty() {
            return Err(KeystoreTxError::new(
                "InvalidQuantity",
                format!("{field_name} hex quantity is empty"),
            ));
        }
        return u128::from_str_radix(hex, 16).map_err(|_| {
            KeystoreTxError::new(
                "InvalidQuantity",
                format!("{field_name} must be a valid u128 decimal or hex value"),
            )
        });
    }

    value.parse::<u128>().map_err(|_| {
        KeystoreTxError::new(
            "InvalidQuantity",
            format!("{field_name} must be a valid u128 decimal or hex value"),
        )
    })
}

/// Parses optional transaction calldata from `0x`-prefixed hex.
pub fn parse_data_hex(raw: &str) -> Result<Vec<u8>, KeystoreTxError> {
    let value = raw.trim();
    let normalized = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .ok_or_else(|| KeystoreTxError::new("InvalidData", "data must be 0x-prefixed hex"))?;
    if normalized.is_empty() {
        return Ok(Vec::new());
    }
    if normalized.len() % 2 != 0 {
        return Err(KeystoreTxError::new(
            "InvalidData",
            "data must contain an even number of hex characters",
        ));
    }

    hex::decode(normalized)
        .map_err(|_| KeystoreTxError::new("InvalidData", "data must be valid hex"))
}

/// Parses and validates an RPC URL used for transaction submission.
pub fn parse_rpc_url(raw: &str) -> Result<Url, KeystoreTxError> {
    let parsed = Url::parse(raw).map_err(|_| {
        KeystoreTxError::new(
            "InvalidRpcUrl",
            "rpc url must be a valid absolute URL (for example: http://127.0.0.1:8545)",
        )
    })?;
    if parsed.host_str().is_none() {
        return Err(KeystoreTxError::new(
            "InvalidRpcUrl",
            "rpc url must include a host",
        ));
    }
    Ok(parsed)
}

/// Validates a signed raw transaction string before it is submitted or persisted.
pub fn validate_raw_transaction_hex(raw_tx_hex: &str) -> Result<(), KeystoreTxError> {
    let value = raw_tx_hex.trim();
    if !value.starts_with("0x") {
        return Err(KeystoreTxError::new(
            "InvalidRawTransaction",
            "raw transaction must be 0x-prefixed hex",
        ));
    }
    if value.len() <= 2 || !value.len().is_multiple_of(2) {
        return Err(KeystoreTxError::new(
            "InvalidRawTransaction",
            "raw transaction must have an even number of hex characters",
        ));
    }
    if !value[2..].chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(KeystoreTxError::new(
            "InvalidRawTransaction",
            "raw transaction must contain only hex characters",
        ));
    }
    Ok(())
}

/// Resolves a keystore key selector expressed either by UUID or by alias.
pub fn resolve_key_id(
    keystore: &Keystore,
    id: Option<&str>,
    by_label: Option<&str>,
) -> Result<Uuid, KeystoreTxError> {
    match (id, by_label) {
        (Some(_), Some(_)) => Err(KeystoreTxError::new(
            "MissingArgument",
            "Specify exactly one key selector: --id or --by-label",
        )),
        (None, None) => Err(KeystoreTxError::new(
            "MissingArgument",
            "Must specify one key selector: --id or --by-label",
        )),
        (Some(raw), None) => Uuid::parse_str(raw)
            .map_err(|_| KeystoreTxError::new("InvalidUuid", "Invalid UUID format")),
        (None, Some(label)) => {
            let keys = keystore
                .list_keys()
                .map_err(|e| KeystoreTxError::new("KeystoreError", e.to_string()))?;
            let matching: Vec<_> = keys
                .iter()
                .filter(|k| k.alias.as_deref() == Some(label))
                .collect();

            match matching.len() {
                0 => Err(KeystoreTxError::new(
                    "KeyNotFound",
                    format!("No key found with label: {label}"),
                )),
                1 => Ok(matching[0].id),
                _ => Err(KeystoreTxError::new(
                    "AmbiguousLabel",
                    format!("Multiple keys found with label: {label}"),
                )),
            }
        }
    }
}

/// Signs an EIP-1559 transaction and returns the sender, payload hash, and raw payload.
pub fn sign_eip1559_transaction(
    keystore: &mut Keystore,
    key_id: Uuid,
    tx: &Eip1559TxToSign,
) -> Result<SignedEip1559Tx, KeystoreTxError> {
    if tx.max_priority_fee_per_gas > tx.max_fee_per_gas {
        return Err(KeystoreTxError::new(
            "InvalidFeeConfig",
            "max-priority-fee-per-gas must be <= max-fee-per-gas",
        ));
    }

    let secure_key = keystore
        .get_private_key(key_id)
        .map_err(|e| KeystoreTxError::new("KeystoreError", e.to_string()))?;
    let from_address = secure_key
        .ethereum_address()
        .map_err(|e| KeystoreTxError::new("SigningError", e.to_string()))?;

    let unsigned = encode_eip1559_unsigned_payload(tx);
    let mut preimage = vec![0x02];
    preimage.extend_from_slice(&unsigned);

    let hash: B256 = keccak256(&preimage);
    let mut hash_bytes = [0u8; 32];
    hash_bytes.copy_from_slice(hash.as_slice());

    let signature = secure_key
        .sign_hash(&hash_bytes)
        .map_err(|e| KeystoreTxError::new("SigningError", e.to_string()))?;
    let signed_sig = derive_signature_with_matching_recovery_id(&signature, &hash, from_address)?;

    let signed = encode_eip1559_signed_payload(tx, signed_sig);
    let mut raw = vec![0x02];
    raw.extend_from_slice(&signed);

    Ok(SignedEip1559Tx {
        from: format!("{from_address:?}"),
        payload_hash: format!("0x{}", hex::encode(hash.as_slice())),
        raw_tx_hex: format!("0x{}", hex::encode(raw)),
    })
}

/// Returns the standard context key used to store a keystore transaction report.
pub fn output_context_key(op_path: &str) -> ContextKey {
    ContextKey(format!("{op_path}.report"))
}

fn derive_signature_with_matching_recovery_id(
    signature: &k256::ecdsa::Signature,
    hash: &B256,
    expected_from: Address,
) -> Result<PrimitiveSignature, KeystoreTxError> {
    for parity in [false, true] {
        let candidate = PrimitiveSignature::from_signature_and_parity(*signature, parity);
        if let Ok(recovered) = candidate.recover_address_from_prehash(hash) {
            if recovered == expected_from {
                return Ok(candidate);
            }
        }
    }

    Err(KeystoreTxError::new(
        "SigningError",
        "Failed to derive recovery id for signed transaction",
    ))
}

fn encode_eip1559_unsigned_payload(tx: &Eip1559TxToSign) -> Vec<u8> {
    rlp_encode_list_preencoded(&[
        rlp_encode_bytes(&u64_to_min_be(tx.chain_id)),
        rlp_encode_bytes(&u64_to_min_be(tx.nonce)),
        rlp_encode_bytes(&u128_to_min_be(tx.max_priority_fee_per_gas)),
        rlp_encode_bytes(&u128_to_min_be(tx.max_fee_per_gas)),
        rlp_encode_bytes(&u64_to_min_be(tx.gas_limit)),
        rlp_encode_bytes(tx.to.as_slice()),
        rlp_encode_bytes(&u128_to_min_be(tx.value_wei)),
        rlp_encode_bytes(&tx.data),
        rlp_encode_list_preencoded(&[]),
    ])
}

fn encode_eip1559_signed_payload(tx: &Eip1559TxToSign, sig: PrimitiveSignature) -> Vec<u8> {
    let y_parity = sig.v();
    let r = trim_leading_zero_bytes(&sig.r().to_be_bytes::<32>());
    let s = trim_leading_zero_bytes(&sig.s().to_be_bytes::<32>());

    rlp_encode_list_preencoded(&[
        rlp_encode_bytes(&u64_to_min_be(tx.chain_id)),
        rlp_encode_bytes(&u64_to_min_be(tx.nonce)),
        rlp_encode_bytes(&u128_to_min_be(tx.max_priority_fee_per_gas)),
        rlp_encode_bytes(&u128_to_min_be(tx.max_fee_per_gas)),
        rlp_encode_bytes(&u64_to_min_be(tx.gas_limit)),
        rlp_encode_bytes(tx.to.as_slice()),
        rlp_encode_bytes(&u128_to_min_be(tx.value_wei)),
        rlp_encode_bytes(&tx.data),
        rlp_encode_list_preencoded(&[]),
        rlp_encode_bytes(&u64_to_min_be(u64::from(y_parity))),
        rlp_encode_bytes(&r),
        rlp_encode_bytes(&s),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_u128_quantity_supports_decimal_and_hex() {
        assert_eq!(parse_u128_quantity("42", "value").expect("decimal"), 42);
        assert_eq!(parse_u128_quantity("0x2a", "value").expect("hex"), 42);
    }

    #[test]
    fn parse_data_hex_rejects_non_prefixed_input() {
        let err = parse_data_hex("1234").expect_err("missing prefix should fail");
        assert_eq!(err.code, "InvalidData");
    }

    #[test]
    fn rlp_encode_empty_list_is_c0() {
        assert_eq!(rlp_encode_list_preencoded(&[]), vec![0xc0]);
    }

    #[test]
    fn validate_raw_transaction_hex_rejects_non_hex() {
        let err = validate_raw_transaction_hex("0x00zz").expect_err("non-hex should fail");
        assert_eq!(err.code, "InvalidRawTransaction");
    }

    #[test]
    fn parse_rpc_url_requires_host() {
        let err = parse_rpc_url("http:///").expect_err("hostless url should fail");
        assert_eq!(err.code, "InvalidRpcUrl");
    }

    #[test]
    fn encode_eip1559_signed_payload_uses_canonical_zero_y_parity() {
        let tx = Eip1559TxToSign {
            to: Address::from([0u8; 20]),
            value_wei: 1,
            chain_id: 1,
            nonce: 0,
            max_fee_per_gas: 2,
            max_priority_fee_per_gas: 1,
            gas_limit: 21_000,
            data: Vec::new(),
        };
        let sig = PrimitiveSignature::from_scalars_and_parity(
            B256::from([1u8; 32]),
            B256::from([2u8; 32]),
            false,
        );

        let encoded = encode_eip1559_signed_payload(&tx, sig);
        let y_parity_idx = encoded.len() - 67;

        // Canonical integer RLP uses empty bytes for zero (0x80), not 0x00.
        assert_eq!(encoded[y_parity_idx], 0x80);
        assert_eq!(encoded[y_parity_idx + 1], 0xa0);
    }
}
