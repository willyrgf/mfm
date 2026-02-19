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
use mfm_state_common::ctx as op_ctx;
use mfm_state_common::errors as op_errors;
use mfm_state_common::idempotency as op_idempotency;
use mfm_state_common::local_io_helpers::{emit_report_event, local_call};
use mfm_state_common::states::meta;
use mfm_sdk::errors::SdkError;
use serde::{Deserialize, Serialize};

const ENV_KEYSTORE_PATH: &str = "MFM_KEYSTORE_PATH";

#[derive(Debug, Clone)]
pub struct KeystoreAdminError {
    pub code: &'static str,
    pub message: String,
}

impl KeystoreAdminError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum KeystoreImportType {
    #[serde(rename = "pk")]
    PrivateKey,
    #[serde(rename = "mn")]
    Mnemonic,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeystoreImportReport {
    pub id: String,
    pub label: String,
    pub key_type: String,
    pub address: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeystoreListSortBy {
    Label,
    Created,
    Type,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeystoreListKey {
    pub id: String,
    pub label: String,
    pub key_type: String,
    pub address: Option<String>,
    pub created: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeystoreListReport {
    pub keys: Vec<KeystoreListKey>,
    pub show_addresses: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeystoreDeleteReport {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug)]
pub struct KeystoreImportStateConfig {
    pub import_type: KeystoreImportType,
    pub label: Option<String>,
    pub derivation_path: String,
    pub keystore_path: PathBuf,
    pub stdin: bool,
}

#[derive(Clone, Debug)]
pub struct KeystoreListStateConfig {
    pub keystore_path: PathBuf,
    pub show_addresses: bool,
    pub filter_label: Option<String>,
    pub sort_by: KeystoreListSortBy,
}

#[derive(Clone, Debug)]
pub struct KeystoreDeleteStateConfig {
    pub id: Option<String>,
    pub by_label: Option<String>,
    pub yes: bool,
    pub keystore_path: PathBuf,
}

#[derive(Clone)]
pub struct KeystoreImportState {
    pub state_id: StateId,
    pub op_id_for_meta: &'static str,
    pub report_key: ContextKey,
    pub event_name: &'static str,
    pub cfg: KeystoreImportStateConfig,
}

impl KeystoreImportState {
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

#[derive(Clone)]
pub struct KeystoreListState {
    pub state_id: StateId,
    pub op_id_for_meta: &'static str,
    pub report_key: ContextKey,
    pub event_name: &'static str,
    pub cfg: KeystoreListStateConfig,
}

impl KeystoreListState {
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

#[derive(Clone)]
pub struct KeystoreDeleteState {
    pub state_id: StateId,
    pub op_id_for_meta: &'static str,
    pub report_key: ContextKey,
    pub event_name: &'static str,
    pub cfg: KeystoreDeleteStateConfig,
}

impl KeystoreDeleteState {
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

pub fn resolve_keystore_path(configured: Option<String>) -> PathBuf {
    configured
        .map(PathBuf::from)
        .or_else(|| std::env::var(ENV_KEYSTORE_PATH).ok().map(PathBuf::from))
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home).join(".mfm").join("keystore")
        })
}

pub fn sdk_error_from_helper(err: KeystoreAdminError) -> SdkError {
    let category = op_errors::keystore_error_category(err.code);
    op_errors::sdk_error(err.code, category, false, err.message)
}
