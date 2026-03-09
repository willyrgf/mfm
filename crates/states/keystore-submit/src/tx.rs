//! Reusable state for submitting a previously signed raw transaction.
//!
//! # Security
//!
//! This state reads a local raw transaction artifact and submits it through the EVM RPC transport.
//! It must not leak raw transaction contents through persisted context, reports, or error details.

use std::path::PathBuf;

use async_trait::async_trait;
use mfm_collectors_local_fs::{LocalFsIoClient, ReadTextRequest};
use mfm_evm_runtime::rpc::send_raw_transaction_via_io;
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
use mfm_state_common::local_io_helpers::{attach_state_id, emit_report_event};
use mfm_state_common::states::meta;

/// Runtime configuration for a raw-transaction submission state.
#[derive(Clone, Debug)]
pub struct KeystoreTxSendRawStateConfig {
    /// Route source identifier used by the EVM RPC transport.
    pub route_source_id: String,
    /// Filesystem path to the file that contains the raw transaction payload.
    pub input_path: PathBuf,
}

/// State that submits a raw signed transaction through an EVM RPC route.
#[derive(Clone, Debug)]
pub struct KeystoreTxSendRawState {
    /// Stable state identifier assigned by the planner.
    pub state_id: StateId,
    /// Context key that receives the submission report.
    pub output_key: ContextKey,
    /// Execution-time configuration for submission.
    pub cfg: KeystoreTxSendRawStateConfig,
}

#[async_trait]
impl State for KeystoreTxSendRawState {
    fn meta(&self) -> StateMeta {
        meta::apply_side_effect(op_idempotency::state_purpose(
            "keystore_tx_send_raw",
            &self.state_id,
            "send_raw_tx",
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let mut local_fs = LocalFsIoClient::new(self.state_id.clone(), io);
        let raw_tx_file = local_fs
            .read_text(
                "tx_send_raw_read",
                ReadTextRequest {
                    path: None,
                    path_hex: Some(hex::encode(
                        self.cfg.input_path.display().to_string().as_bytes(),
                    )),
                },
            )
            .await
            .map_err(op_errors::state_from_io)
            .map_err(|err| attach_state_id(&self.state_id, err))?;

        let raw_tx_hex = raw_tx_file.text.trim();
        if raw_tx_hex.is_empty() {
            return Err(op_errors::state_error_with_state(
                self.state_id.clone(),
                "InvalidRawTransaction",
                ErrorCategory::ParsingInput,
                false,
                "input file did not contain a raw transaction payload",
            ));
        }

        let submission =
            send_raw_transaction_via_io(&self.state_id, io, &self.cfg.route_source_id, raw_tx_hex)
                .await?;

        let report_json = serde_json::json!({
            "tx_hash": submission.tx_hash,
            "rpc_url_host": submission.rpc_source_id,
            "submitted_at": submission.submitted_at,
        });

        op_ctx::write_json(ctx, self.output_key.clone(), report_json.clone())?;
        emit_report_event(rec, "keystore_tx_send_raw.submitted", report_json).await?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}
