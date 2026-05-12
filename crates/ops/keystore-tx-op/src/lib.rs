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

use std::path::PathBuf;
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
    /// Output path where the signed raw transaction was written.
    pub out_path: String,
}

/// Planning input for the keystore transaction signing operation.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TxSignOpConfig {
    /// Optional exact key identifier.
    pub id: Option<String>,
    /// Optional UTF-8 label selector.
    #[serde(default)]
    pub by_label: Option<String>,
    /// Optional hex-encoded UTF-8 label selector.
    #[serde(default)]
    pub by_label_hex: Option<String>,
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
    /// Output file for the raw signed transaction.
    pub out_path: String,
    /// Output write policy. Defaults to `create_new`.
    #[serde(default)]
    pub out_write_mode: LocalFileWriteMode,
    /// Hex-encoded calldata.
    #[serde(default = "default_tx_data")]
    pub data: String,
    /// Optional keystore directory path.
    pub keystore_path: Option<String>,
    /// Optional hex-encoded keystore directory path.
    #[serde(default)]
    pub keystore_path_hex: Option<String>,
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
        let by_label =
            decode_selector_label(cfg.by_label, cfg.by_label_hex).map_err(sdk_error_from_helper)?;
        let keystore_path =
            decode_optional_hex_string(cfg.keystore_path, cfg.keystore_path_hex, "keystore_path")
                .map_err(sdk_error_from_helper)?;
        let state = KeystoreTxSignState {
            state_id: state_id.clone(),
            output_key: tx_sign_report_key_for_op_path(&op_path),
            cfg: KeystoreTxSignStateConfig {
                id: cfg.id,
                by_label,
                tx,
                out_path: PathBuf::from(cfg.out_path),
                out_write_mode: cfg.out_write_mode,
                keystore_path: require_keystore_path(keystore_path)
                    .map_err(sdk_error_from_helper)?,
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

fn require_keystore_path(configured: Option<String>) -> Result<PathBuf, KeystoreTxError> {
    let Some(value) = configured
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    else {
        return Err(KeystoreTxError::new(
            "MissingArgument",
            "Must provide keystore_path or keystore_path_hex",
        ));
    };

    Ok(PathBuf::from(value))
}

fn decode_selector_label(
    by_label: Option<String>,
    by_label_hex: Option<String>,
) -> Result<Option<String>, KeystoreTxError> {
    match (by_label, by_label_hex) {
        (Some(_), Some(_)) => Err(KeystoreTxError::new(
            "InvalidSelectorLabel",
            "selector label must use exactly one input field",
        )),
        (Some(label), None) => Ok(Some(label)),
        (None, Some(raw_hex)) => {
            let bytes = hex::decode(raw_hex).map_err(|_| {
                KeystoreTxError::new(
                    "InvalidSelectorLabel",
                    "selector label hex must be valid lowercase/uppercase hex",
                )
            })?;
            let decoded = String::from_utf8(bytes).map_err(|_| {
                KeystoreTxError::new(
                    "InvalidSelectorLabel",
                    "selector label hex did not decode to utf-8",
                )
            })?;
            Ok(Some(decoded))
        }
        (None, None) => Ok(None),
    }
}

fn decode_optional_hex_string(
    raw: Option<String>,
    raw_hex: Option<String>,
    field_name: &'static str,
) -> Result<Option<String>, KeystoreTxError> {
    match (raw, raw_hex) {
        (Some(_), Some(_)) => Err(KeystoreTxError::new(
            "InvalidPathConfig",
            format!("{field_name} must use exactly one encoding"),
        )),
        (Some(value), None) => Ok(Some(value)),
        (None, Some(value_hex)) => {
            let bytes = hex::decode(value_hex).map_err(|_| {
                KeystoreTxError::new(
                    "InvalidPathConfig",
                    format!("{field_name} hex must be valid hex"),
                )
            })?;
            let decoded = String::from_utf8(bytes).map_err(|_| {
                KeystoreTxError::new(
                    "InvalidPathConfig",
                    format!("{field_name} hex did not decode to utf-8"),
                )
            })?;
            Ok(Some(decoded))
        }
        (None, None) => Ok(None),
    }
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
