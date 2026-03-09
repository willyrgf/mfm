#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Keystore administration operations.
//!
//! These operations stay thin by validating config and assembling keystore admin state graphs.
//! Execution lives in `mfm-state-keystore`.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_keystore_admin::{KeystoreImportOp, KEYSTORE_IMPORT_OP_ID};
//! use mfm_sdk::op::Operation;
//!
//! let op = KeystoreImportOp;
//! assert_eq!(op.op_id().as_str(), KEYSTORE_IMPORT_OP_ID);
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use regex::Regex;
use serde::{Deserialize, Serialize};

use mfm_machine::config::RunConfig;
use mfm_machine::errors::ErrorCategory;
use mfm_machine::ids::{ContextKey, OpId, OpPath, StateId};
use mfm_machine::plan::{StateGraph, StateNode};
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{OpIo, Operation};
use mfm_state_common::errors as op_errors;
use mfm_state_keystore::states::admin::{
    decode_optional_hex_string, sdk_error_from_helper, KeystoreAdminError, KeystoreDeleteState,
    KeystoreDeleteStateConfig, KeystoreImportState, KeystoreImportStateConfig, KeystoreListState,
    KeystoreListStateConfig,
};
use mfm_state_keystore::tx::output_context_key;

/// Re-exported keystore admin report and enum types used by callers.
pub use mfm_state_keystore::states::admin::{
    KeystoreDeleteReport, KeystoreImportReport, KeystoreImportType, KeystoreListKey,
    KeystoreListReport, KeystoreListSortBy,
};

/// Stable version string for keystore admin operations.
pub const KEYSTORE_ADMIN_OP_VERSION: &str = "v1";
/// Operation identifier for the keystore import planner.
pub const KEYSTORE_IMPORT_OP_ID: &str = "keystore_import";
/// Operation identifier for the keystore list planner.
pub const KEYSTORE_LIST_OP_ID: &str = "keystore_list";
/// Operation identifier for the keystore delete planner.
pub const KEYSTORE_DELETE_OP_ID: &str = "keystore_delete";

/// Planning input for the keystore import operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeystoreImportOpConfig {
    /// Import mode to execute.
    pub import_type: KeystoreImportType,
    /// Optional UTF-8 label.
    #[serde(default)]
    pub label: Option<String>,
    /// Optional hex-encoded UTF-8 label.
    #[serde(default)]
    pub label_hex: Option<String>,
    /// BIP-32 derivation path for imported material.
    #[serde(default = "default_derivation_path")]
    pub derivation_path: String,
    /// Optional keystore directory path.
    pub keystore_path: Option<String>,
    /// Optional hex-encoded keystore directory path.
    #[serde(default)]
    pub keystore_path_hex: Option<String>,
    /// Whether secret material should be read from stdin.
    #[serde(default)]
    pub stdin: bool,
}

fn default_derivation_path() -> String {
    "m/44'/60'/0'/0/0".to_string()
}

/// Planning input for the keystore list operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeystoreListOpConfig {
    /// Optional keystore directory path.
    pub keystore_path: Option<String>,
    /// Optional hex-encoded keystore directory path.
    #[serde(default)]
    pub keystore_path_hex: Option<String>,
    /// Whether addresses should be included in the report.
    #[serde(default = "default_show_addresses")]
    pub show_addresses: bool,
    /// Optional UTF-8 regex filter applied to labels.
    #[serde(default)]
    pub filter_label: Option<String>,
    /// Optional hex-encoded UTF-8 regex filter applied to labels.
    #[serde(default)]
    pub filter_label_hex: Option<String>,
    /// Sort order for the generated report.
    #[serde(default = "default_list_sort_by")]
    pub sort_by: KeystoreListSortBy,
}

fn default_show_addresses() -> bool {
    true
}

fn default_list_sort_by() -> KeystoreListSortBy {
    KeystoreListSortBy::Created
}

/// Planning input for the keystore delete operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeystoreDeleteOpConfig {
    /// Optional exact key identifier to delete.
    pub id: Option<String>,
    /// Optional UTF-8 label selector.
    #[serde(default)]
    pub by_label: Option<String>,
    /// Optional hex-encoded UTF-8 label selector.
    #[serde(default)]
    pub by_label_hex: Option<String>,
    /// Whether destructive confirmation has already been granted.
    #[serde(default)]
    pub yes: bool,
    /// Optional keystore directory path.
    pub keystore_path: Option<String>,
    /// Optional hex-encoded keystore directory path.
    #[serde(default)]
    pub keystore_path_hex: Option<String>,
}

fn report_key_for_op_path(_op_path: &OpPath) -> ContextKey {
    ContextKey("report".to_string())
}

/// Returns the context key used to publish import reports.
pub fn keystore_import_report_context_key() -> ContextKey {
    output_context_key(&format!("{KEYSTORE_IMPORT_OP_ID}.main"))
}

/// Returns the context key used to publish list reports.
pub fn keystore_list_report_context_key() -> ContextKey {
    output_context_key(&format!("{KEYSTORE_LIST_OP_ID}.main"))
}

/// Returns the context key used to publish delete reports.
pub fn keystore_delete_report_context_key() -> ContextKey {
    output_context_key(&format!("{KEYSTORE_DELETE_OP_ID}.main"))
}

/// Planner for keystore import runs.
#[derive(Clone, Default)]
pub struct KeystoreImportOp;

impl Operation for KeystoreImportOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(KEYSTORE_IMPORT_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        KEYSTORE_ADMIN_OP_VERSION.to_string()
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
        let cfg: KeystoreImportOpConfig =
            serde_json::from_value(op_config.clone()).map_err(|_| {
                op_errors::sdk_parse_error("invalid_op_config", "invalid keystore_import op_config")
            })?;

        let keystore_path =
            decode_optional_hex_string(cfg.keystore_path, cfg.keystore_path_hex, "keystore_path")
                .map_err(sdk_error_from_helper)?;
        let label = decode_optional_hex_string(cfg.label, cfg.label_hex, "label")
            .map_err(sdk_error_from_helper)?;

        let state_id = StateId::must_new(format!("{}.import", op_path.0));
        let state = KeystoreImportState::new(
            state_id.clone(),
            KEYSTORE_IMPORT_OP_ID,
            report_key_for_op_path(&op_path),
            "keystore_import.completed",
            KeystoreImportStateConfig {
                import_type: cfg.import_type,
                label,
                derivation_path: cfg.derivation_path,
                keystore_path: require_keystore_path(keystore_path)
                    .map_err(sdk_error_from_helper)?,
                stdin: cfg.stdin,
            },
        );

        Ok(StateGraph {
            states: vec![StateNode {
                id: state_id,
                state: Arc::new(state),
            }],
            edges: Vec::new(),
        })
    }
}

/// Planner for keystore list runs.
#[derive(Clone, Default)]
pub struct KeystoreListOp;

impl Operation for KeystoreListOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(KEYSTORE_LIST_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        KEYSTORE_ADMIN_OP_VERSION.to_string()
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
        let cfg: KeystoreListOpConfig =
            serde_json::from_value(op_config.clone()).map_err(|_| {
                op_errors::sdk_parse_error("invalid_op_config", "invalid keystore_list op_config")
            })?;

        let filter_label =
            decode_optional_hex_string(cfg.filter_label, cfg.filter_label_hex, "filter_label")
                .map_err(sdk_error_from_helper)?;

        if let Some(pattern) = filter_label.as_ref() {
            Regex::new(pattern).map_err(|e| {
                op_errors::sdk_error(
                    "InvalidRegex",
                    ErrorCategory::ParsingInput,
                    false,
                    format!("Invalid regex pattern: {e}"),
                )
            })?;
        }

        let keystore_path =
            decode_optional_hex_string(cfg.keystore_path, cfg.keystore_path_hex, "keystore_path")
                .map_err(sdk_error_from_helper)?;
        let state_id = StateId::must_new(format!("{}.list", op_path.0));
        let state = KeystoreListState::new(
            state_id.clone(),
            KEYSTORE_LIST_OP_ID,
            report_key_for_op_path(&op_path),
            "keystore_list.completed",
            KeystoreListStateConfig {
                keystore_path: require_keystore_path(keystore_path)
                    .map_err(sdk_error_from_helper)?,
                show_addresses: cfg.show_addresses,
                filter_label,
                sort_by: cfg.sort_by,
            },
        );

        Ok(StateGraph {
            states: vec![StateNode {
                id: state_id,
                state: Arc::new(state),
            }],
            edges: Vec::new(),
        })
    }
}

/// Planner for keystore delete runs.
#[derive(Clone, Default)]
pub struct KeystoreDeleteOp;

impl Operation for KeystoreDeleteOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(KEYSTORE_DELETE_OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        KEYSTORE_ADMIN_OP_VERSION.to_string()
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
        let cfg: KeystoreDeleteOpConfig =
            serde_json::from_value(op_config.clone()).map_err(|_| {
                op_errors::sdk_parse_error("invalid_op_config", "invalid keystore_delete op_config")
            })?;
        let by_label = decode_optional_hex_string(cfg.by_label, cfg.by_label_hex, "by_label")
            .map_err(sdk_error_from_helper)?;
        let keystore_path =
            decode_optional_hex_string(cfg.keystore_path, cfg.keystore_path_hex, "keystore_path")
                .map_err(sdk_error_from_helper)?;

        let state_id = StateId::must_new(format!("{}.delete", op_path.0));
        let state = KeystoreDeleteState::new(
            state_id.clone(),
            KEYSTORE_DELETE_OP_ID,
            report_key_for_op_path(&op_path),
            "keystore_delete.completed",
            KeystoreDeleteStateConfig {
                id: cfg.id,
                by_label,
                yes: cfg.yes,
                keystore_path: require_keystore_path(keystore_path)
                    .map_err(sdk_error_from_helper)?,
            },
        );

        Ok(StateGraph {
            states: vec![StateNode {
                id: state_id,
                state: Arc::new(state),
            }],
            edges: Vec::new(),
        })
    }
}

fn require_keystore_path(configured: Option<String>) -> Result<PathBuf, KeystoreAdminError> {
    let Some(value) = configured
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    else {
        return Err(KeystoreAdminError::new(
            "MissingArgument",
            "Must provide keystore_path or keystore_path_hex",
        ));
    };

    Ok(PathBuf::from(value))
}
