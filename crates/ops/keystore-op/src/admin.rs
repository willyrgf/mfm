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
//! use mfm_op_keystore::admin::{KeystoreImportOp, KEYSTORE_IMPORT_OP_ID};
//! use mfm_sdk::op::Operation;
//!
//! let op = KeystoreImportOp;
//! assert_eq!(op.op_id().as_str(), KEYSTORE_IMPORT_OP_ID);
//! ```

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use mfm_machine::config::RunConfig;
use mfm_machine::ids::{ContextKey, OpId, OpPath};
use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{
    leaf_state_id, leaf_state_node, LeafOpSpec, OpInterface, Operation, PlannedOp, PlannedOpKind,
};
use mfm_state_common::errors as op_errors;
use mfm_state_keystore::states::admin::{
    sdk_error_from_helper, KeystoreAdminError, KeystoreDeleteState, KeystoreDeleteStateConfig,
    KeystoreImportState, KeystoreImportStateConfig, KeystoreListState, KeystoreListStateConfig,
};
use mfm_state_keystore::tx::output_context_key;

/// Re-exported keystore admin report and enum types used by callers.
pub use mfm_collectors_local_keystore::{
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
#[serde(deny_unknown_fields)]
pub struct KeystoreImportOpConfig {
    /// Import mode to execute.
    pub import_type: KeystoreImportType,
    /// BIP-32 derivation path for imported material.
    #[serde(default = "default_derivation_path")]
    pub derivation_path: String,
    /// Opaque local binding registered by the app before live execution.
    pub local_resource_handle: String,
}

fn default_derivation_path() -> String {
    "m/44'/60'/0'/0/0".to_string()
}

/// Planning input for the keystore list operation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeystoreListOpConfig {
    /// Opaque local binding registered by the app before live execution.
    pub local_resource_handle: String,
    /// Whether addresses should be included in the report.
    #[serde(default = "default_show_addresses")]
    pub show_addresses: bool,
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
#[serde(deny_unknown_fields)]
pub struct KeystoreDeleteOpConfig {
    /// Optional exact key identifier to delete.
    pub id: Option<String>,
    /// Whether destructive confirmation has already been granted.
    #[serde(default)]
    pub yes: bool,
    /// Opaque local binding registered by the app before live execution.
    pub local_resource_handle: String,
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

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg: KeystoreImportOpConfig =
            serde_json::from_value(op_config.clone()).map_err(|_| {
                op_errors::sdk_parse_error("invalid_op_config", "invalid keystore_import op_config")
            })?;

        let local_resource_handle = require_local_resource_handle(cfg.local_resource_handle)
            .map_err(sdk_error_from_helper)?;

        let state_id = leaf_state_id(&op_path, "import")?;
        let state = KeystoreImportState::new(
            state_id.clone(),
            KEYSTORE_IMPORT_OP_ID,
            report_key_for_op_path(&op_path),
            "keystore_import.completed",
            KeystoreImportStateConfig {
                import_type: cfg.import_type,
                derivation_path: cfg.derivation_path,
                local_resource_handle,
            },
        );

        Ok(PlannedOp {
            interface: OpInterface {
                imports: Vec::new(),
                exports: vec![PortKey("report".to_string())],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(&op_path, "import", Arc::new(state))?],
                edges: Vec::new(),
            }),
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

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg: KeystoreListOpConfig =
            serde_json::from_value(op_config.clone()).map_err(|_| {
                op_errors::sdk_parse_error("invalid_op_config", "invalid keystore_list op_config")
            })?;

        let local_resource_handle = require_local_resource_handle(cfg.local_resource_handle)
            .map_err(sdk_error_from_helper)?;
        let state_id = leaf_state_id(&op_path, "list")?;
        let state = KeystoreListState::new(
            state_id.clone(),
            KEYSTORE_LIST_OP_ID,
            report_key_for_op_path(&op_path),
            "keystore_list.completed",
            KeystoreListStateConfig {
                local_resource_handle,
                show_addresses: cfg.show_addresses,
                sort_by: cfg.sort_by,
            },
        );

        Ok(PlannedOp {
            interface: OpInterface {
                imports: Vec::new(),
                exports: vec![PortKey("report".to_string())],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(&op_path, "list", Arc::new(state))?],
                edges: Vec::new(),
            }),
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

    fn expand(
        &self,
        op_path: OpPath,
        op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<PlannedOp, SdkError> {
        let cfg: KeystoreDeleteOpConfig =
            serde_json::from_value(op_config.clone()).map_err(|_| {
                op_errors::sdk_parse_error("invalid_op_config", "invalid keystore_delete op_config")
            })?;
        let local_resource_handle = require_local_resource_handle(cfg.local_resource_handle)
            .map_err(sdk_error_from_helper)?;

        let state_id = leaf_state_id(&op_path, "delete")?;
        let state = KeystoreDeleteState::new(
            state_id.clone(),
            KEYSTORE_DELETE_OP_ID,
            report_key_for_op_path(&op_path),
            "keystore_delete.completed",
            KeystoreDeleteStateConfig {
                id: cfg.id,
                yes: cfg.yes,
                local_resource_handle,
            },
        );

        Ok(PlannedOp {
            interface: OpInterface {
                imports: Vec::new(),
                exports: vec![PortKey("report".to_string())],
            },
            kind: PlannedOpKind::Leaf(LeafOpSpec {
                states: vec![leaf_state_node(&op_path, "delete", Arc::new(state))?],
                edges: Vec::new(),
            }),
        })
    }
}

fn require_local_resource_handle(configured: String) -> Result<String, KeystoreAdminError> {
    let value = configured.trim();
    if value.is_empty() {
        return Err(KeystoreAdminError::new(
            "MissingArgument",
            "Must provide local_resource_handle",
        ));
    }

    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_config_rejects_legacy_passphrase_field() {
        let err = serde_json::from_value::<KeystoreImportOpConfig>(serde_json::json!({
            "import_type": "mn",
            "derivation_path": "m/44'/60'/0'/0/0",
            "local_resource_handle": "local-keystore:test",
            "passphrase": "do-not-accept"
        }))
        .expect_err("legacy secret-bearing field must be rejected");

        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn import_config_rejects_legacy_hex_path_metadata() {
        let err = serde_json::from_value::<KeystoreImportOpConfig>(serde_json::json!({
            "import_type": "mn",
            "derivation_path": "m/44'/60'/0'/0/0",
            "keystore_path_hex": "2f746d702f6b657973746f7265",
            "local_resource_handle": "local-keystore:test"
        }))
        .expect_err("reversible local path fields must be rejected");

        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn serialized_keystore_configs_do_not_contain_local_paths_or_label_fields() {
        let import_cfg = serde_json::from_value::<KeystoreImportOpConfig>(serde_json::json!({
            "import_type": "mn",
            "derivation_path": "m/44'/60'/0'/0/0",
            "local_resource_handle": "local-keystore:test"
        }))
        .expect("handle-only config should deserialize");
        let list_cfg = KeystoreListOpConfig {
            local_resource_handle: "local-keystore:test".to_string(),
            show_addresses: true,
            sort_by: KeystoreListSortBy::Created,
        };
        let delete_cfg = KeystoreDeleteOpConfig {
            id: Some("550e8400-e29b-41d4-a716-446655440000".to_string()),
            yes: true,
            local_resource_handle: "local-keystore:test".to_string(),
        };

        let rendered = serde_json::to_string(&serde_json::json!({
            "import": import_cfg,
            "list": list_cfg,
            "delete": delete_cfg
        }))
        .expect("serialize configs");

        assert!(!rendered.contains("/tmp/keystore"));
        assert!(!rendered.contains("/tmp/bip39-extra"));
        assert!(!rendered.contains("keystore_path"));
        assert!(!rendered.contains("label_hex"));
        assert!(!rendered.contains("file_path"));
    }

    #[test]
    fn import_config_rejects_empty_local_resource_handle() {
        let err = require_local_resource_handle("  ".to_string())
            .expect_err("empty local resource handles are invalid");

        assert_eq!(err.code, "MissingArgument");
    }
}
