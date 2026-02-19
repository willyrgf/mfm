use std::path::PathBuf;

use async_trait::async_trait;
use mfm_evm_runtime::rpc::send_raw_transaction_via_io;
use mfm_machine::context::DynContext;
use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::ids::{ContextKey, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_op_common::ctx as op_ctx;
use mfm_op_common::errors as op_errors;
use mfm_op_common::idempotency as op_idempotency;
use mfm_op_common::local_io_helpers::{emit_report_event, local_call};
use mfm_op_common::states::meta;
use serde::Deserialize;

use crate::tx::Eip1559TxToSign;

#[derive(Clone, Debug)]
pub struct KeystoreTxSignStateConfig {
    pub id: Option<String>,
    pub by_label: Option<String>,
    pub tx: Eip1559TxToSign,
    pub out_path: PathBuf,
    pub keystore_path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct KeystoreTxSendRawStateConfig {
    pub route_source_id: String,
    pub input_path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct KeystoreTxSignState {
    pub state_id: StateId,
    pub output_key: ContextKey,
    pub cfg: KeystoreTxSignStateConfig,
}

#[derive(Clone, Debug)]
pub struct KeystoreTxSendRawState {
    pub state_id: StateId,
    pub output_key: ContextKey,
    pub cfg: KeystoreTxSendRawStateConfig,
}

#[derive(Debug, Clone, Deserialize)]
struct ReadTextResponse {
    text: String,
}

#[async_trait]
impl State for KeystoreTxSignState {
    fn meta(&self) -> StateMeta {
        meta::apply_side_effect(op_idempotency::state_purpose(
            "keystore_tx_sign",
            &self.state_id,
            "sign_tx",
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let report_json: serde_json::Value = local_call(
            &self.state_id,
            io,
            "local.keystore.tx_sign",
            "tx_sign",
            serde_json::json!({
                "id": self.cfg.id.clone(),
                "label_hex": self.cfg.by_label.as_ref().map(|v| hex::encode(v.as_bytes())),
                "store_path_hex": hex::encode(self.cfg.keystore_path.display().to_string().as_bytes()),
                "out_path_hex": hex::encode(self.cfg.out_path.display().to_string().as_bytes()),
                "to": format!("{:?}", self.cfg.tx.to),
                "value_wei": self.cfg.tx.value_wei.to_string(),
                "chain_id": self.cfg.tx.chain_id,
                "nonce": self.cfg.tx.nonce,
                "max_fee_per_gas": self.cfg.tx.max_fee_per_gas.to_string(),
                "max_priority_fee_per_gas": self.cfg.tx.max_priority_fee_per_gas.to_string(),
                "gas_limit": self.cfg.tx.gas_limit,
                "data_hex": format!("0x{}", hex::encode(&self.cfg.tx.data)),
            }),
        )
        .await?;

        op_ctx::write_json(ctx, self.output_key.clone(), report_json.clone())?;
        emit_report_event(rec, "keystore_tx_sign.completed", report_json).await?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
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
        let raw_tx_file: ReadTextResponse = local_call(
            &self.state_id,
            io,
            "local.fs.read_text",
            "tx_send_raw_read",
            serde_json::json!({
                "path_hex": hex::encode(self.cfg.input_path.display().to_string().as_bytes()),
            }),
        )
        .await?;

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
