//! EVM transaction models, signing preimages, and raw transaction encoders.
//!
//! This module is intentionally non-secret: callers supply transaction fields and signatures, and
//! the helpers return deterministic byte encodings or hashes. Live key loading and signing remain
//! outside this crate.
//!
//! # Examples
//!
//! ```rust
//! use alloy_primitives::{B256, PrimitiveSignature};
//! use mfm_evm_core::tx::{encode_signed_legacy_tx, LegacyTxToSign};
//!
//! let tx = LegacyTxToSign {
//!     to: None,
//!     value_wei: 0,
//!     chain_id: 1,
//!     nonce: 0,
//!     gas_price_wei: 1,
//!     gas_limit: 21_000,
//!     data: vec![0x60, 0x00],
//! };
//! let sig = PrimitiveSignature::from_scalars_and_parity(B256::from([1; 32]), B256::from([2; 32]), false);
//! let raw = encode_signed_legacy_tx(&tx, sig);
//! assert_eq!(raw[0], 0xf8);
//! ```

use alloy_primitives::{keccak256, Address, PrimitiveSignature, B256};

use crate::hex::{bytes_to_hex_prefixed, hex_to_bytes, normalize_hex_str};
use crate::rlp::{
    rlp_encode_bytes, rlp_encode_list, rlp_encode_list_preencoded, trim_leading_zero_bytes,
    u128_to_min_be, u64_to_min_be,
};
use crate::util_error::UtilError;

/// Canonical legacy transaction fields required for signing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyTxToSign {
    /// Optional recipient address; `None` means contract creation.
    pub to: Option<Address>,
    /// Native token value in wei.
    pub value_wei: u128,
    /// Chain ID used for EIP-155 replay protection.
    pub chain_id: u64,
    /// Sender account nonce.
    pub nonce: u64,
    /// Legacy gas price in wei.
    pub gas_price_wei: u128,
    /// Gas limit for the transaction.
    pub gas_limit: u64,
    /// Contract initcode or calldata bytes.
    pub data: Vec<u8>,
}

/// Canonical EIP-1559 transaction fields required for signing.
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

/// Parses a `0x`-prefixed Ethereum address.
pub fn parse_address(raw: &str, field_name: &str) -> Result<Address, UtilError> {
    raw.parse::<Address>().map_err(|_| {
        UtilError::new(
            "invalid_address",
            format!("{field_name} must be a valid 0x-prefixed Ethereum address"),
        )
    })
}

/// Parses a decimal or `0x`-prefixed quantity into `u128`.
pub fn parse_u128_quantity(raw: &str, field_name: &str) -> Result<u128, UtilError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(UtilError::new(
            "invalid_quantity",
            format!("{field_name} cannot be empty"),
        ));
    }

    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        if hex.is_empty() {
            return Err(UtilError::new(
                "invalid_quantity",
                format!("{field_name} hex quantity is empty"),
            ));
        }
        return u128::from_str_radix(hex, 16).map_err(|_| {
            UtilError::new(
                "invalid_quantity",
                format!("{field_name} must be a valid u128 decimal or hex value"),
            )
        });
    }

    value.parse::<u128>().map_err(|_| {
        UtilError::new(
            "invalid_quantity",
            format!("{field_name} must be a valid u128 decimal or hex value"),
        )
    })
}

/// Parses a decimal or `0x`-prefixed quantity into `u64`.
pub fn parse_u64_quantity(raw: &str, field_name: &str) -> Result<u64, UtilError> {
    let value = parse_u128_quantity(raw, field_name)?;
    u64::try_from(value).map_err(|_| {
        UtilError::new(
            "invalid_quantity",
            format!("{field_name} must fit into a u64"),
        )
    })
}

/// Parses optional transaction calldata from `0x`-prefixed hex.
pub fn parse_data_hex(raw: &str) -> Result<Vec<u8>, UtilError> {
    let value = raw.trim();
    let normalized = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .ok_or_else(|| UtilError::new("invalid_data", "data must be 0x-prefixed hex"))?;
    if normalized.is_empty() {
        return Ok(Vec::new());
    }
    if normalized.len() % 2 != 0 {
        return Err(UtilError::new(
            "invalid_data",
            "data must contain an even number of hex characters",
        ));
    }

    hex::decode(normalized).map_err(|_| UtilError::new("invalid_data", "data must be valid hex"))
}

/// Validates a signed raw transaction string before it is submitted or persisted.
pub fn validate_raw_transaction_hex(raw_tx_hex: &str) -> Result<(), UtilError> {
    let value = raw_tx_hex.trim();
    if !value.starts_with("0x") {
        return Err(UtilError::new(
            "invalid_raw_transaction",
            "raw transaction must be 0x-prefixed hex",
        ));
    }
    if value.len() <= 2 || !value.len().is_multiple_of(2) {
        return Err(UtilError::new(
            "invalid_raw_transaction",
            "raw transaction must have an even number of hex characters",
        ));
    }
    if !value[2..].chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(UtilError::new(
            "invalid_raw_transaction",
            "raw transaction must contain only hex characters",
        ));
    }
    Ok(())
}

/// Returns the signing hash for an EIP-155 protected legacy transaction.
pub fn legacy_signing_hash(tx: &LegacyTxToSign) -> B256 {
    keccak256(legacy_unsigned_payload(tx))
}

/// Returns the signing hash for an EIP-1559 transaction.
pub fn eip1559_signing_hash(tx: &Eip1559TxToSign) -> B256 {
    let mut preimage = vec![0x02];
    preimage.extend_from_slice(&encode_eip1559_unsigned_payload(tx));
    keccak256(preimage)
}

/// Encodes a signed EIP-155 protected legacy transaction as raw bytes.
pub fn encode_signed_legacy_tx(tx: &LegacyTxToSign, sig: PrimitiveSignature) -> Vec<u8> {
    let r = trim_leading_zero_bytes(&sig.r().to_be_bytes::<32>());
    let s = trim_leading_zero_bytes(&sig.s().to_be_bytes::<32>());
    let v = u128::from(tx.chain_id) * 2 + 35 + u128::from(u8::from(sig.v()));
    let to = tx
        .to
        .map(|addr| addr.as_slice().to_vec())
        .unwrap_or_default();

    rlp_encode_list(&[
        u64_to_min_be(tx.nonce),
        u128_to_min_be(tx.gas_price_wei),
        u64_to_min_be(tx.gas_limit),
        to,
        u128_to_min_be(tx.value_wei),
        tx.data.clone(),
        u128_to_min_be(v),
        r,
        s,
    ])
}

/// Encodes a signed EIP-155 protected legacy transaction as `0x`-prefixed hex.
pub fn encode_signed_legacy_tx_hex(tx: &LegacyTxToSign, sig: PrimitiveSignature) -> String {
    bytes_to_hex_prefixed(&encode_signed_legacy_tx(tx, sig))
}

/// Encodes a signed EIP-1559 transaction as type-prefixed raw bytes.
pub fn encode_signed_eip1559_tx(tx: &Eip1559TxToSign, sig: PrimitiveSignature) -> Vec<u8> {
    let mut raw = vec![0x02];
    raw.extend_from_slice(&encode_eip1559_signed_payload(tx, sig));
    raw
}

/// Encodes a signed EIP-1559 transaction as `0x`-prefixed hex.
pub fn encode_signed_eip1559_tx_hex(tx: &Eip1559TxToSign, sig: PrimitiveSignature) -> String {
    bytes_to_hex_prefixed(&encode_signed_eip1559_tx(tx, sig))
}

/// Computes the expected EVM transaction hash for signed raw transaction bytes.
pub fn raw_transaction_hash_bytes(bytes: &[u8]) -> Result<String, UtilError> {
    if bytes.is_empty() {
        return Err(UtilError::new(
            "invalid_raw_transaction",
            "raw transaction bytes must not be empty",
        ));
    }
    Ok(bytes_to_hex_prefixed(keccak256(bytes).as_slice()))
}

/// Computes the expected EVM transaction hash for a signed raw transaction.
pub fn raw_transaction_hash(raw_tx_hex: &str) -> Result<String, UtilError> {
    let normalized = normalize_hex_str(raw_tx_hex).map_err(|_| {
        UtilError::new("invalid_raw_transaction", "raw transaction hex was invalid")
    })?;
    let bytes = hex_to_bytes(&normalized).map_err(|_| {
        UtilError::new("invalid_raw_transaction", "raw transaction hex was invalid")
    })?;
    raw_transaction_hash_bytes(&bytes)
}

fn legacy_unsigned_payload(tx: &LegacyTxToSign) -> Vec<u8> {
    let to = tx
        .to
        .map(|addr| addr.as_slice().to_vec())
        .unwrap_or_default();
    rlp_encode_list(&[
        u64_to_min_be(tx.nonce),
        u128_to_min_be(tx.gas_price_wei),
        u64_to_min_be(tx.gas_limit),
        to,
        u128_to_min_be(tx.value_wei),
        tx.data.clone(),
        u64_to_min_be(tx.chain_id),
        Vec::new(),
        Vec::new(),
    ])
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
        rlp_encode_bytes(&u64_to_min_be(u64::from(sig.v()))),
        rlp_encode_bytes(&r),
        rlp_encode_bytes(&s),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature(parity: bool) -> PrimitiveSignature {
        let mut r = [0u8; 32];
        r[31] = 1;
        let mut s = [0u8; 32];
        s[31] = 2;
        PrimitiveSignature::from_scalars_and_parity(B256::from(r), B256::from(s), parity)
    }

    #[test]
    fn signed_legacy_create_encoding_is_stable() {
        let tx = LegacyTxToSign {
            to: None,
            value_wei: 0,
            chain_id: 1,
            nonce: 0,
            gas_price_wei: 1,
            gas_limit: 21_000,
            data: vec![0x60, 0x00],
        };

        assert_eq!(
            bytes_to_hex_prefixed(&encode_signed_legacy_tx(&tx, signature(false))),
            "0xcd80018252088080826000250102"
        );
    }

    #[test]
    fn signed_legacy_call_encoding_is_stable() {
        let tx = LegacyTxToSign {
            to: Some(Address::from([0x11; 20])),
            value_wei: 0,
            chain_id: 1,
            nonce: 1,
            gas_price_wei: 2,
            gas_limit: 21_000,
            data: Vec::new(),
        };

        assert_eq!(
            bytes_to_hex_prefixed(&encode_signed_legacy_tx(&tx, signature(true))),
            "0xdf01028252089411111111111111111111111111111111111111118080260102"
        );
    }

    #[test]
    fn signed_eip1559_encoding_uses_canonical_zero_y_parity() {
        let tx = Eip1559TxToSign {
            to: Address::from([0u8; 20]),
            value_wei: 0,
            chain_id: 1,
            nonce: 0,
            max_fee_per_gas: 2,
            max_priority_fee_per_gas: 1,
            gas_limit: 21_000,
            data: Vec::new(),
        };

        assert_eq!(
            bytes_to_hex_prefixed(&encode_signed_eip1559_tx(&tx, signature(false))),
            "0x02e2018001028252089400000000000000000000000000000000000000008080c0800102"
        );
    }

    #[test]
    fn parse_quantity_supports_decimal_and_hex() {
        assert_eq!(parse_u128_quantity("42", "value").expect("decimal"), 42);
        assert_eq!(parse_u128_quantity("0x2a", "value").expect("hex"), 42);
    }

    #[test]
    fn parse_data_hex_rejects_non_prefixed_input() {
        let err = parse_data_hex("1234").expect_err("missing prefix should fail");
        assert_eq!(err.code, "invalid_data");
    }

    #[test]
    fn validate_raw_transaction_hex_rejects_non_hex() {
        let err = validate_raw_transaction_hex("0x00zz").expect_err("non-hex should fail");
        assert_eq!(err.code, "invalid_raw_transaction");
    }

    #[test]
    fn raw_transaction_hash_rejects_empty_bytes() {
        let err = raw_transaction_hash_bytes(&[]).expect_err("empty raw tx should fail");
        assert_eq!(err.code, "invalid_raw_transaction");
    }
}
