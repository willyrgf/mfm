#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Keystore-backed transaction planning operations.
//!
//! These operations validate input, wire state graphs, and leave signing side effects to the
//! shared keystore state crate.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_keystore_tx::{KeystoreTxSignOp, TX_SIGN_OP_ID};
//! use mfm_sdk::op::Operation;
//!
//! let op = KeystoreTxSignOp;
//! assert_eq!(op.op_id().as_str(), TX_SIGN_OP_ID);
//! ```

use std::sync::Arc;

use mfm_machine::config::RunConfig;
use mfm_machine::errors::ErrorCategory;
use mfm_machine::ids::{ContextKey, OpId, OpPath};
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{
    leaf_state_id, leaf_state_node, LeafOpSpec, OpInterface, Operation, PlannedOp, PlannedOpKind,
};
use mfm_state_common::errors as op_errors;
use mfm_state_keystore::states::tx::{KeystoreTxSignState, KeystoreTxSignStateConfig};
use mfm_state_keystore::tx::{
    output_context_key, parse_address, parse_data_hex, parse_u128_quantity, Eip1559TxToSign,
    KeystoreTxError,
};
use serde::{Deserialize, Serialize};

/// Re-exported output write mode used by tx-sign callers.
pub use mfm_state_keystore::states::tx::LocalFileWriteMode;

/// Stable version string for keystore transaction operations.
pub const TX_OP_VERSION: &str = "v1";

/// Operation identifier for transaction signing.
pub const TX_SIGN_OP_ID: &str = "keystore_tx_sign";

/// Report emitted after a transaction is signed and written locally.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TxSignReport {
    /// Signer address.
    pub from: String,
    /// Recipient address.
    pub to: String,
    /// Nonce included in the signed transaction.
    pub nonce: u64,
    /// Chain identifier included in the signed transaction.
    pub chain_id: u64,
    /// Transaction type label, typically `eip1559`.
    pub tx_type: String,
    /// Hash of the signed transaction payload.
    pub payload_hash: String,
}

/// Planning input for the keystore transaction signing operation.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TxSignOpConfig {
    /// Optional exact key identifier.
    pub id: Option<String>,
    /// Opaque local binding registered by the app before live execution.
    pub local_resource_handle: String,
    /// Recipient address.
    pub to: String,
    /// Transfer value in wei, encoded as decimal or quantity string supported by helpers.
    pub value_wei: String,
    /// Chain ID for signing.
    pub chain_id: u64,
    /// Nonce for the transaction.
    pub nonce: u64,
    /// Max fee per gas.
    pub max_fee_per_gas: String,
    /// Max priority fee per gas.
    pub max_priority_fee_per_gas: String,
    /// Gas limit for the transaction.
    pub gas_limit: u64,
    /// Output write policy. Defaults to `create_new`.
    #[serde(default)]
    pub out_write_mode: LocalFileWriteMode,
    /// Hex-encoded calldata.
    #[serde(default = "default_tx_data")]
    pub data: String,
}

fn default_tx_data() -> String {
    "0x".to_string()
}

fn tx_sign_report_key_for_op_path(op_path: &OpPath) -> ContextKey {
    let _ = op_path;
    ContextKey("report".to_string())
}

/// Returns the context key used to publish signing reports.
pub fn tx_sign_report_context_key() -> ContextKey {
    output_context_key(&format!("{TX_SIGN_OP_ID}.main"))
}

/// Planner for keystore-backed EIP-1559 signing.
#[derive(Clone, Default)]
pub struct KeystoreTxSignOp;

impl Operation for KeystoreTxSignOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(TX_SIGN_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        TX_OP_VERSION.to_string()
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg: TxSignOpConfig = serde_json::from_value(op_config.clone()).map_err(|_| {
            op_errors::sdk_parse_error("invalid_op_config", "invalid keystore_tx_sign op_config")
        })?;

        let to = parse_address(&cfg.to, "to").map_err(sdk_error_from_helper)?;
        let value_wei =
            parse_u128_quantity(&cfg.value_wei, "value-wei").map_err(sdk_error_from_helper)?;
        let max_fee_per_gas = parse_u128_quantity(&cfg.max_fee_per_gas, "max-fee-per-gas")
            .map_err(sdk_error_from_helper)?;
        let max_priority_fee_per_gas =
            parse_u128_quantity(&cfg.max_priority_fee_per_gas, "max-priority-fee-per-gas")
                .map_err(sdk_error_from_helper)?;
        let data = parse_data_hex(&cfg.data).map_err(sdk_error_from_helper)?;

        let tx = Eip1559TxToSign {
            to,
            value_wei,
            chain_id: cfg.chain_id,
            nonce: cfg.nonce,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            gas_limit: cfg.gas_limit,
            data,
        };

        let state_id = leaf_state_id(&op_path, "sign_and_write")?;
        let local_resource_handle = require_local_resource_handle(cfg.local_resource_handle)
            .map_err(sdk_error_from_helper)?;
        let state = KeystoreTxSignState {
            state_id: state_id.clone(),
            output_key: tx_sign_report_key_for_op_path(&op_path),
            cfg: KeystoreTxSignStateConfig {
                id: cfg.id,
                tx,
                out_write_mode: cfg.out_write_mode,
                local_resource_handle,
            },
        };

        Ok(PlannedOp {
            interface: OpInterface {
                imports: Vec::new(),
                exports: vec![PortKey("report".to_string())],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(
                    &op_path,
                    "sign_and_write",
                    Arc::new(state),
                )?],
                edges: Vec::new(),
            }),
        })
    }
}

fn require_local_resource_handle(configured: String) -> Result<String, KeystoreTxError> {
    let value = configured.trim();
    if value.is_empty() {
        return Err(KeystoreTxError::new(
            "MissingArgument",
            "Must provide local_resource_handle",
        ));
    }

    Ok(value.to_string())
}

fn sdk_error_from_helper(err: KeystoreTxError) -> SdkError {
    let category = helper_category(err.code);
    op_errors::sdk_error(err.code, category, false, err.message)
}

fn helper_category(code: &str) -> ErrorCategory {
    match code {
        "InvalidAddress"
        | "InvalidQuantity"
        | "InvalidData"
        | "InvalidFeeConfig"
        | "InvalidPathConfig"
        | "InvalidSelectorLabel"
        | "InvalidUuid"
        | "MissingArgument"
        | "AmbiguousLabel"
        | "KeyNotFound" => ErrorCategory::ParsingInput,
        _ => ErrorCategory::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_tx_config() -> serde_json::Value {
        serde_json::json!({
            "id": null,
            "local_resource_handle": "local-keystore:test",
            "to": "0x1111111111111111111111111111111111111111",
            "value_wei": "1",
            "chain_id": 1,
            "nonce": 0,
            "max_fee_per_gas": "2000000000",
            "max_priority_fee_per_gas": "1000000000",
            "gas_limit": 21000,
            "out_write_mode": "create_new",
            "data": "0x"
        })
    }

    #[test]
    fn tx_sign_config_rejects_legacy_hex_path_and_label_fields() {
        let mut value = valid_tx_config();
        value["keystore_path_hex"] = serde_json::json!("2f746d702f6b657973746f7265");
        value["by_label_hex"] = serde_json::json!("7369676e6572");

        let err = serde_json::from_value::<TxSignOpConfig>(value)
            .expect_err("reversible local fields must be rejected");

        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn serialized_tx_sign_config_contains_no_local_paths_or_labels() {
        let cfg = serde_json::from_value::<TxSignOpConfig>(valid_tx_config())
            .expect("handle-only config should deserialize");

        let rendered = serde_json::to_string(&cfg).expect("serialize config");

        assert!(!rendered.contains("/tmp/keystore"));
        assert!(!rendered.contains("/tmp/signed.tx"));
        assert!(!rendered.contains("keystore_path"));
        assert!(!rendered.contains("out_path"));
        assert!(!rendered.contains("by_label"));
    }

    #[test]
    fn tx_sign_config_rejects_empty_local_resource_handle() {
        let err = require_local_resource_handle("  ".to_string())
            .expect_err("empty local resource handles are invalid");

        assert_eq!(err.code, "MissingArgument");
    }
}
