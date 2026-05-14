//! Pure transaction input helpers for local keystore signing flows.
//!
//! This module owns the non-secret transaction model and parsing helpers shared by
//! op planners, reusable states, and the live local keystore transport.

use alloy_primitives::Address;
use mfm_machine::ids::ContextKey;
use url::Url;

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

/// Parses a `0x`-prefixed Ethereum address.
pub fn parse_address(raw: &str, field_name: &str) -> Result<Address, KeystoreTxError> {
    raw.parse::<Address>().map_err(|_| {
        KeystoreTxError::new(
            "invalid_address",
            format!("{field_name} must be a valid 0x-prefixed Ethereum address"),
        )
    })
}

/// Parses a decimal or `0x`-prefixed quantity into `u128`.
pub fn parse_u128_quantity(raw: &str, field_name: &str) -> Result<u128, KeystoreTxError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(KeystoreTxError::new(
            "invalid_quantity",
            format!("{field_name} cannot be empty"),
        ));
    }

    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        if hex.is_empty() {
            return Err(KeystoreTxError::new(
                "invalid_quantity",
                format!("{field_name} hex quantity is empty"),
            ));
        }
        return u128::from_str_radix(hex, 16).map_err(|_| {
            KeystoreTxError::new(
                "invalid_quantity",
                format!("{field_name} must be a valid u128 decimal or hex value"),
            )
        });
    }

    value.parse::<u128>().map_err(|_| {
        KeystoreTxError::new(
            "invalid_quantity",
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
        .ok_or_else(|| KeystoreTxError::new("invalid_data", "data must be 0x-prefixed hex"))?;
    if normalized.is_empty() {
        return Ok(Vec::new());
    }
    if normalized.len() % 2 != 0 {
        return Err(KeystoreTxError::new(
            "invalid_data",
            "data must contain an even number of hex characters",
        ));
    }

    hex::decode(normalized)
        .map_err(|_| KeystoreTxError::new("invalid_data", "data must be valid hex"))
}

/// Parses and validates an RPC URL used for transaction submission.
pub fn parse_rpc_url(raw: &str) -> Result<Url, KeystoreTxError> {
    let parsed = Url::parse(raw).map_err(|_| {
        KeystoreTxError::new(
            "invalid_rpc_url",
            "rpc url must be a valid absolute URL (for example: http://127.0.0.1:8545)",
        )
    })?;
    if parsed.host_str().is_none() {
        return Err(KeystoreTxError::new(
            "invalid_rpc_url",
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
            "invalid_raw_transaction",
            "raw transaction must be 0x-prefixed hex",
        ));
    }
    if value.len() <= 2 || !value.len().is_multiple_of(2) {
        return Err(KeystoreTxError::new(
            "invalid_raw_transaction",
            "raw transaction must have an even number of hex characters",
        ));
    }
    if !value[2..].chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(KeystoreTxError::new(
            "invalid_raw_transaction",
            "raw transaction must contain only hex characters",
        ));
    }
    Ok(())
}

/// Returns the standard context key used to store a keystore transaction report.
pub fn output_context_key(op_path: &str) -> ContextKey {
    ContextKey(format!("{op_path}.out.report"))
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
        assert_eq!(err.code, "invalid_data");
    }

    #[test]
    fn validate_raw_transaction_hex_rejects_non_hex() {
        let err = validate_raw_transaction_hex("0x00zz").expect_err("non-hex should fail");
        assert_eq!(err.code, "invalid_raw_transaction");
    }

    #[test]
    fn parse_rpc_url_requires_host() {
        let err = parse_rpc_url("http:///").expect_err("hostless url should fail");
        assert_eq!(err.code, "invalid_rpc_url");
    }
}
