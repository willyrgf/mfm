//! Reusable states for keystore administration flows.
//!
//! # Security
//!
//! These states interact with local keystore transport namespaces and must not persist secrets into
//! context, artifacts, reports, or error details. Keep user-facing outputs limited to metadata that
//! is already safe to expose.
//!
//! # Examples
//!
//! ```rust
//! use std::path::PathBuf;
//!
//! use mfm_state_keystore::states::admin::{KeystoreImportStateConfig, KeystoreImportType};
//!
//! let cfg = KeystoreImportStateConfig {
//!     import_type: KeystoreImportType::PrivateKey,
//!     label: Some("deploy".to_string()),
//!     derivation_path: "m/44'/60'/0'/0/0".to_string(),
//!     keystore_path: PathBuf::from("/tmp/keystore"),
//!     stdin: true,
//! };
//!
//! assert_eq!(cfg.label.as_deref(), Some("deploy"));
//! assert!(cfg.stdin);
//! ```

use std::path::PathBuf;

use async_trait::async_trait;
use mfm_evm_core::hex::hex_encode_utf8;
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::ids::{ContextKey, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_sdk::errors::SdkError;
use mfm_state_common::ctx as op_ctx;
use mfm_state_common::errors as op_errors;
use mfm_state_common::idempotency as op_idempotency;
use mfm_state_common::local_io_helpers::{emit_report_event, local_call};
use mfm_state_common::states::meta;
use serde::{Deserialize, Serialize};

const ENV_KEYSTORE_PATH: &str = "MFM_KEYSTORE_PATH";

/// Error returned by keystore administration helpers.
#[derive(Debug, Clone)]
pub struct KeystoreAdminError {
    /// Stable machine-readable error code.
    pub code: &'static str,
    /// Human-readable message safe to return to callers.
    pub message: String,
}

impl KeystoreAdminError {
    /// Creates a new helper error.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Supported keystore import input formats.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum KeystoreImportType {
    /// Import a raw private key.
    #[serde(rename = "pk")]
    PrivateKey,
    /// Import a mnemonic phrase.
    #[serde(rename = "mn")]
    Mnemonic,
}

/// Report written after a successful keystore import.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeystoreImportReport {
    /// Identifier of the imported key entry.
    pub id: String,
    /// Alias recorded for the key.
    pub label: String,
    /// Key type recorded by the keystore.
    pub key_type: String,
    /// Derived address for the imported key.
    pub address: String,
    /// RFC3339 timestamp captured by the import flow.
    pub created_at: String,
}

/// Sort order supported by the keystore list flow.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeystoreListSortBy {
    /// Sort by alias.
    Label,
    /// Sort by creation timestamp.
    Created,
    /// Sort by key type.
    Type,
}

/// Single keystore entry returned by the list flow.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeystoreListKey {
    /// Identifier of the key entry.
    pub id: String,
    /// Alias recorded for the key.
    pub label: String,
    /// Key type recorded by the keystore.
    pub key_type: String,
    /// Optional derived address when address display is enabled.
    pub address: Option<String>,
    /// RFC3339 creation timestamp.
    pub created: String,
}

/// Report written after listing keystore entries.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeystoreListReport {
    /// Keys included in the report.
    pub keys: Vec<KeystoreListKey>,
    /// Whether addresses were requested for display.
    pub show_addresses: bool,
}

/// Report written after deleting a keystore entry.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeystoreDeleteReport {
    /// Identifier of the deleted key.
    pub id: String,
    /// Alias of the deleted key.
    pub label: String,
}

/// Runtime configuration for the keystore import state.
///
/// Secret material itself is never stored here; the state reads it from stdin or a local transport
/// endpoint at execution time.
#[derive(Clone, Debug)]
pub struct KeystoreImportStateConfig {
    /// Input kind to import.
    pub import_type: KeystoreImportType,
    /// Optional alias to assign to the imported key.
    pub label: Option<String>,
    /// Derivation path used for mnemonic imports.
    pub derivation_path: String,
    /// Filesystem path of the keystore directory.
    pub keystore_path: PathBuf,
    /// Whether the secret material should be read from stdin.
    pub stdin: bool,
}

/// Runtime configuration for the keystore list state.
#[derive(Clone, Debug)]
pub struct KeystoreListStateConfig {
    /// Filesystem path of the keystore directory.
    pub keystore_path: PathBuf,
    /// Whether derived addresses should be resolved for output.
    pub show_addresses: bool,
    /// Optional alias filter applied before sorting.
    pub filter_label: Option<String>,
    /// Sort order for the final report.
    pub sort_by: KeystoreListSortBy,
}

/// Runtime configuration for the keystore delete state.
///
/// Exactly one of `id` or `by_label` should be supplied by the planner.
#[derive(Clone, Debug)]
pub struct KeystoreDeleteStateConfig {
    /// Optional UUID selector for the key to delete.
    pub id: Option<String>,
    /// Optional alias selector for the key to delete.
    pub by_label: Option<String>,
    /// Whether deletion confirmation has already been granted.
    pub yes: bool,
    /// Filesystem path of the keystore directory.
    pub keystore_path: PathBuf,
}

/// State that imports a key into the local keystore transport.
#[derive(Clone)]
pub struct KeystoreImportState {
    /// Stable state identifier assigned by the planner.
    pub state_id: StateId,
    /// Operation identifier used to build shared metadata.
    pub op_id_for_meta: &'static str,
    /// Context key that receives the serialized report.
    pub report_key: ContextKey,
    /// Domain-event name emitted for the import report.
    pub event_name: &'static str,
    /// Execution-time configuration for the import.
    pub cfg: KeystoreImportStateConfig,
}

impl KeystoreImportState {
    /// Creates a new import state instance.
    pub fn new(
        state_id: StateId,
        op_id_for_meta: &'static str,
        report_key: ContextKey,
        event_name: &'static str,
        cfg: KeystoreImportStateConfig,
    ) -> Self {
        Self {
            state_id,
            op_id_for_meta,
            report_key,
            event_name,
            cfg,
        }
    }
}

#[async_trait]
impl State for KeystoreImportState {
    fn meta(&self) -> StateMeta {
        meta::apply_side_effect(op_idempotency::state_purpose(
            self.op_id_for_meta,
            &self.state_id,
            "import_key",
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let report: KeystoreImportReport = local_call(
            &self.state_id,
            io,
            "local.keystore.import",
            "keystore_import",
            serde_json::json!({
                "kind": self.cfg.import_type.clone(),
                "label_hex": self.cfg.label.as_ref().map(|v| hex_encode_utf8(v)),
                "derive_path": self.cfg.derivation_path.clone(),
                "store_path_hex": hex_encode_utf8(self.cfg.keystore_path.to_string_lossy().as_ref()),
                "stdin_mode": self.cfg.stdin,
            }),
        )
        .await?;

        let report_json = serde_json::to_value(&report).map_err(|_| {
            op_errors::state_error_with_state(
                self.state_id.clone(),
                "SerializeReportFailed",
                ErrorCategory::Unknown,
                false,
                "failed to serialize keystore import report",
            )
        })?;

        op_ctx::write_json(ctx, self.report_key.clone(), report_json.clone())?;
        emit_report_event(rec, self.event_name, report_json).await?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

/// State that lists keystore entries through the local keystore transport.
#[derive(Clone)]
pub struct KeystoreListState {
    /// Stable state identifier assigned by the planner.
    pub state_id: StateId,
    /// Operation identifier used to build shared metadata.
    pub op_id_for_meta: &'static str,
    /// Context key that receives the serialized report.
    pub report_key: ContextKey,
    /// Domain-event name emitted for the list report.
    pub event_name: &'static str,
    /// Execution-time configuration for the list flow.
    pub cfg: KeystoreListStateConfig,
}

impl KeystoreListState {
    /// Creates a new list state instance.
    pub fn new(
        state_id: StateId,
        op_id_for_meta: &'static str,
        report_key: ContextKey,
        event_name: &'static str,
        cfg: KeystoreListStateConfig,
    ) -> Self {
        Self {
            state_id,
            op_id_for_meta,
            report_key,
            event_name,
            cfg,
        }
    }
}

#[async_trait]
impl State for KeystoreListState {
    fn meta(&self) -> StateMeta {
        meta::apply_side_effect(op_idempotency::state_purpose(
            self.op_id_for_meta,
            &self.state_id,
            "list_keys",
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let report: KeystoreListReport = local_call(
            &self.state_id,
            io,
            "local.keystore.list",
            "keystore_list",
            serde_json::json!({
                "store_path_hex": hex_encode_utf8(self.cfg.keystore_path.to_string_lossy().as_ref()),
                "show_addrs": self.cfg.show_addresses,
                "filter_label_hex": self.cfg.filter_label.as_ref().map(|v| hex_encode_utf8(v)),
                "sort_by": self.cfg.sort_by.clone(),
            }),
        )
        .await?;

        let report_json = serde_json::to_value(&report).map_err(|_| {
            op_errors::state_error_with_state(
                self.state_id.clone(),
                "SerializeReportFailed",
                ErrorCategory::Unknown,
                false,
                "failed to serialize keystore list report",
            )
        })?;

        op_ctx::write_json(ctx, self.report_key.clone(), report_json.clone())?;
        emit_report_event(rec, self.event_name, report_json).await?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

/// State that deletes a key through the local keystore transport.
#[derive(Clone)]
pub struct KeystoreDeleteState {
    /// Stable state identifier assigned by the planner.
    pub state_id: StateId,
    /// Operation identifier used to build shared metadata.
    pub op_id_for_meta: &'static str,
    /// Context key that receives the serialized report.
    pub report_key: ContextKey,
    /// Domain-event name emitted for the delete report.
    pub event_name: &'static str,
    /// Execution-time configuration for the delete flow.
    pub cfg: KeystoreDeleteStateConfig,
}

impl KeystoreDeleteState {
    /// Creates a new delete state instance.
    pub fn new(
        state_id: StateId,
        op_id_for_meta: &'static str,
        report_key: ContextKey,
        event_name: &'static str,
        cfg: KeystoreDeleteStateConfig,
    ) -> Self {
        Self {
            state_id,
            op_id_for_meta,
            report_key,
            event_name,
            cfg,
        }
    }
}

#[async_trait]
impl State for KeystoreDeleteState {
    fn meta(&self) -> StateMeta {
        meta::apply_side_effect(op_idempotency::state_purpose(
            self.op_id_for_meta,
            &self.state_id,
            "delete_key",
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let report: KeystoreDeleteReport = local_call(
            &self.state_id,
            io,
            "local.keystore.delete",
            "keystore_delete",
            serde_json::json!({
                "id": self.cfg.id.clone(),
                "label_hex": self.cfg.by_label.as_ref().map(|v| hex_encode_utf8(v)),
                "confirm_yes": self.cfg.yes,
                "store_path_hex": hex_encode_utf8(self.cfg.keystore_path.to_string_lossy().as_ref()),
            }),
        )
        .await?;

        let report_json = serde_json::to_value(&report).map_err(|_| {
            op_errors::state_error_with_state(
                self.state_id.clone(),
                "SerializeReportFailed",
                ErrorCategory::Unknown,
                false,
                "failed to serialize keystore delete report",
            )
        })?;

        op_ctx::write_json(ctx, self.report_key.clone(), report_json.clone())?;
        emit_report_event(rec, self.event_name, report_json).await?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

/// Decodes a string field supplied in either raw or hex-encoded form.
pub fn decode_optional_hex_string(
    raw: Option<String>,
    raw_hex: Option<String>,
    field_name: &'static str,
) -> Result<Option<String>, KeystoreAdminError> {
    match (raw, raw_hex) {
        (Some(_), Some(_)) => Err(KeystoreAdminError::new(
            "InvalidPathConfig",
            format!("{field_name} must use exactly one encoding"),
        )),
        (Some(value), None) => Ok(Some(value)),
        (None, Some(value_hex)) => {
            let bytes = hex::decode(value_hex).map_err(|_| {
                KeystoreAdminError::new(
                    "InvalidPathConfig",
                    format!("{field_name} hex must be valid hex"),
                )
            })?;
            let decoded = String::from_utf8(bytes).map_err(|_| {
                KeystoreAdminError::new(
                    "InvalidPathConfig",
                    format!("{field_name} hex did not decode to utf-8"),
                )
            })?;
            Ok(Some(decoded))
        }
        (None, None) => Ok(None),
    }
}

/// Resolves the effective keystore path from explicit config, environment, or the default home path.
pub fn resolve_keystore_path(configured: Option<String>) -> PathBuf {
    configured
        .map(PathBuf::from)
        .or_else(|| std::env::var(ENV_KEYSTORE_PATH).ok().map(PathBuf::from))
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".mfm").join("keystore")
        })
}

/// Converts a helper error into the SDK error shape expected by op planners.
pub fn sdk_error_from_helper(err: KeystoreAdminError) -> SdkError {
    let category = op_errors::keystore_error_category(err.code);
    op_errors::sdk_error(err.code, category, false, err.message)
}
