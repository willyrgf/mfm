#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Keystore-backed transaction planning operations.
//!
//! These operations validate input, wire state graphs, and leave signing or submission side
//! effects to the shared keystore state crates.
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
use mfm_machine::ids::{ContextKey, OpId, OpPath, StateId};
use mfm_machine::plan::{StateGraph, StateNode};
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{OpIo, Operation};
use mfm_state_common::errors as op_errors;
use mfm_state_keystore::states::tx::{KeystoreTxSignState, KeystoreTxSignStateConfig};
use mfm_state_keystore::tx::{
    output_context_key, parse_address, parse_data_hex, parse_u128_quantity, Eip1559TxToSign,
    KeystoreTxError,
};
use mfm_state_keystore_submit::tx::{KeystoreTxSendRawState, KeystoreTxSendRawStateConfig};
use serde::{Deserialize, Serialize};

/// Stable version string for keystore transaction operations.
pub const TX_OP_VERSION: &str = "v1";

/// Operation identifier for transaction signing.
pub const TX_SIGN_OP_ID: &str = "keystore_tx_sign";
/// Operation identifier for raw transaction submission.
pub const TX_SEND_RAW_OP_ID: &str = "keystore_tx_send_raw";

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

/// Report emitted after a raw transaction is submitted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TxSendRawReport {
    /// Submitted transaction hash.
    pub tx_hash: String,
    /// Host portion of the RPC URL used for submission.
    pub rpc_url_host: String,
    /// Submission timestamp recorded by the state.
    pub submitted_at: String,
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
    /// Hex-encoded calldata.
    #[serde(default = "default_tx_data")]
    pub data: String,
    /// Optional keystore directory path.
    pub keystore_path: Option<String>,
    /// Optional hex-encoded keystore directory path.
    #[serde(default)]
    pub keystore_path_hex: Option<String>,
}

/// Planning input for the raw transaction submission operation.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TxSendRawOpConfig {
    /// Optional route source identifier for the RPC transport.
    pub source_id: Option<String>,
    /// File containing the raw signed transaction.
    pub input_path: String,
}

fn default_tx_data() -> String {
    "0x".to_string()
}

fn tx_sign_report_key_for_op_path(op_path: &OpPath) -> ContextKey {
    let _ = op_path;
    ContextKey("report".to_string())
}

fn tx_send_raw_report_key_for_op_path(op_path: &OpPath) -> ContextKey {
    let _ = op_path;
    ContextKey("report".to_string())
}

/// Returns the context key used to publish signing reports.
pub fn tx_sign_report_context_key() -> ContextKey {
    output_context_key(&format!("{TX_SIGN_OP_ID}.main"))
}

/// Returns the context key used to publish raw-send reports.
pub fn tx_send_raw_report_context_key() -> ContextKey {
    output_context_key(&format!("{TX_SEND_RAW_OP_ID}.main"))
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

    fn io(&self, _op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        Ok(OpIo {
            imports: Vec::new(),
            exports: vec![PortKey("report".to_string())],
        })
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
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

        let state_id = StateId::must_new(format!("{}.sign_and_write", op_path.0));
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
                keystore_path: require_keystore_path(keystore_path)
                    .map_err(sdk_error_from_helper)?,
            },
        };

        Ok(StateGraph {
            states: vec![StateNode {
                id: state_id,
                state: Arc::new(state),
            }],
            edges: Vec::new(),
        })
    }
}

/// Planner for raw transaction submission.
#[derive(Clone, Default)]
pub struct KeystoreTxSendRawOp;

impl Operation for KeystoreTxSendRawOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(TX_SEND_RAW_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        TX_OP_VERSION.to_string()
    }

    fn io(&self, _op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        Ok(OpIo {
            imports: Vec::new(),
            exports: vec![PortKey("report".to_string())],
        })
    }

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
        let cfg: TxSendRawOpConfig = serde_json::from_value(op_config.clone()).map_err(|_| {
            op_errors::sdk_parse_error(
                "invalid_op_config",
                "invalid keystore_tx_send_raw op_config",
            )
        })?;

        let source_id = require_rpc_source_id(cfg.source_id).map_err(sdk_error_from_helper)?;

        let state_id = StateId::must_new(format!("{}.send_raw", op_path.0));
        let state = KeystoreTxSendRawState {
            state_id: state_id.clone(),
            output_key: tx_send_raw_report_key_for_op_path(&op_path),
            cfg: KeystoreTxSendRawStateConfig {
                route_source_id: source_id,
                input_path: PathBuf::from(cfg.input_path),
            },
        };

        Ok(StateGraph {
            states: vec![StateNode {
                id: state_id,
                state: Arc::new(state),
            }],
            edges: Vec::new(),
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

fn require_rpc_source_id(configured: Option<String>) -> Result<String, KeystoreTxError> {
    let value = configured;
    let Some(value) = value else {
        return Err(KeystoreTxError::new(
            "MissingArgument",
            "Must provide source_id",
        ));
    };

    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(KeystoreTxError::new(
            "MissingArgument",
            "Must provide source_id",
        ));
    }

    Ok(trimmed.to_string())
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
        | "InvalidRawTransaction"
        | "InvalidRpcSourceId"
        | "InvalidPathConfig"
        | "InvalidSelectorLabel"
        | "InvalidUuid"
        | "MissingArgument"
        | "AmbiguousLabel"
        | "KeyNotFound" => ErrorCategory::ParsingInput,
        "RpcInvalidResponse" => ErrorCategory::Rpc,
        _ => ErrorCategory::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use async_trait::async_trait;
    use mfm_machine::engine::{ExecutionEngine, RunPhase};
    use mfm_machine::errors::{ErrorCategory, ErrorInfo, IoError};
    use mfm_machine::ids::{ArtifactId, ErrorCode};
    use mfm_machine::io::IoCall;
    use mfm_machine::live_io::{LiveIoEnv, LiveIoTransport, LiveIoTransportFactory};
    use mfm_machine::live_io_router::RouterLiveIoTransportFactory;
    use mfm_machine::runtime::{DefaultExecutionEngine, PlanResolver};
    use mfm_sdk::unstable::SdkPlanResolver;
    use mfm_state_common::test_support as op_test_support;
    use mfm_transports_local_fs::LocalFsIoTransportFactory;
    use mfm_transports_local_keystore::LocalKeystoreIoTransportFactory;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn io_info(code: &'static str, message: &'static str) -> ErrorInfo {
        ErrorInfo {
            code: ErrorCode(code.to_string()),
            category: ErrorCategory::Rpc,
            retryable: false,
            message: message.to_string(),
            details: None,
        }
    }

    #[derive(Clone)]
    struct TestEvmFactory;

    impl LiveIoTransportFactory for TestEvmFactory {
        fn namespace_group(&self) -> &str {
            "rpc.control"
        }

        fn make(&self, _env: LiveIoEnv) -> Box<dyn LiveIoTransport> {
            Box::new(TestEvmTransport)
        }
    }

    struct TestEvmTransport;

    #[async_trait]
    impl LiveIoTransport for TestEvmTransport {
        async fn call(&mut self, call: IoCall) -> Result<serde_json::Value, IoError> {
            let kind = call
                .request
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if kind != "evm_call" {
                return Err(IoError::Other(io_info(
                    "unexpected_request_kind",
                    "unexpected rpc.control request kind",
                )));
            }
            let method = call
                .request
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if method != "eth_sendRawTransaction" {
                return Err(IoError::Other(io_info(
                    "unexpected_method",
                    "unexpected json-rpc method",
                )));
            }
            let source_id = call
                .request
                .get("route")
                .and_then(|v| v.get("source_id"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if source_id != "reth_local" {
                return Err(IoError::Other(io_info(
                    "unexpected_route_source_id",
                    "unexpected route source id",
                )));
            }
            Ok(serde_json::json!(
                "0x1111111111111111111111111111111111111111111111111111111111111111"
            ))
        }
    }

    fn test_transport_factory() -> Arc<dyn LiveIoTransportFactory> {
        Arc::new(
            RouterLiveIoTransportFactory::from_factories(vec![
                Arc::new(LocalFsIoTransportFactory),
                Arc::new(LocalKeystoreIoTransportFactory),
                Arc::new(TestEvmFactory),
            ])
            .expect("router factories"),
        )
    }

    #[tokio::test]
    async fn tx_send_raw_op_run_reports_submission() {
        let temp = TempDir::new().expect("temp dir");
        let input_path = temp.path().join("signed.raw");
        std::fs::write(&input_path, "0x0201").expect("write raw tx");

        let op: mfm_sdk::op::DynOperation = Arc::new(KeystoreTxSendRawOp);
        let (registry, planner, pipeline) = op_test_support::single_op_plan(
            op,
            serde_json::json!({
                "source_id": "reth_local",
                "input_path": input_path.display().to_string(),
            }),
        )
        .expect("pipeline");

        let resolver: Arc<dyn PlanResolver> = Arc::new(SdkPlanResolver::new(
            Arc::clone(&registry),
            Arc::clone(&planner),
        ));
        let engine = DefaultExecutionEngine::new(resolver)
            .with_live_transport_factory(test_transport_factory());
        let engine: Arc<dyn ExecutionEngine> = Arc::new(engine);

        let stores = op_test_support::in_memory_stores();
        let cfg = op_test_support::run_config_live();

        let run = op_test_support::start_pipeline_with_defaults(
            Arc::clone(&engine),
            &stores,
            Arc::clone(&registry),
            Arc::clone(&planner),
            pipeline,
            cfg,
        )
        .await
        .expect("run");

        assert_eq!(run.phase, RunPhase::Completed);

        let final_snapshot_id = run.final_snapshot_id.expect("final snapshot id");
        let bytes = stores
            .artifacts
            .get(&ArtifactId(final_snapshot_id.0.clone()))
            .await
            .expect("snapshot bytes");
        let snapshot: serde_json::Value = serde_json::from_slice(&bytes).expect("snapshot json");

        let report_key = tx_send_raw_report_context_key();
        let report_value = snapshot
            .get(&report_key.0)
            .cloned()
            .expect("report in snapshot");
        let report: TxSendRawReport = serde_json::from_value(report_value).expect("report value");

        assert_eq!(
            report.tx_hash,
            "0x1111111111111111111111111111111111111111111111111111111111111111"
        );
        assert_eq!(report.rpc_url_host, "reth_local");
        assert!(!report.submitted_at.is_empty());
    }

    #[test]
    fn tx_send_raw_requires_source_id() {
        let op = KeystoreTxSendRawOp;
        let result = op
            .expand(
                OpPath("keystore_tx_send_raw.main".to_string()),
                &serde_json::json!({"input_path": "/tmp/x"}),
                &RunConfig {
                    io_mode: mfm_machine::config::IoMode::Live,
                    retry_policy: mfm_machine::config::RetryPolicy {
                        max_attempts: 1,
                        backoff: mfm_machine::config::BackoffPolicy::Fixed {
                            delay: std::time::Duration::from_millis(0),
                        },
                    },
                    event_profile: mfm_machine::config::EventProfile::Normal,
                    execution_mode: mfm_machine::config::ExecutionMode::Sequential,
                    context_checkpointing:
                        mfm_machine::config::ContextCheckpointing::AfterEveryState,
                    replay_missing_fact_retryable: false,
                    skip_tags: Vec::new(),
                    nix_flake_allowlist: mfm_machine::config::default_nix_flake_allowlist(),
                },
            )
            .map_err(|err| err.info.code.0);

        assert_eq!(result.err().as_deref(), Some("MissingArgument"));
    }
}
