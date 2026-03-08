//! Reusable states for signing raw transactions via keystore-backed flows.
//!
//! # Security
//!
//! These states bridge local keystore material into durable outputs. They must not leak passwords,
//! raw key material, or decrypted secret buffers through persisted context, reports, or error
//! messages.

use std::path::PathBuf;

use async_trait::async_trait;
use mfm_collectors_local_keystore::{KeystoreTxSignRequest, LocalKeystoreIoClient};
use mfm_machine::context::DynContext;
use mfm_machine::errors::StateError;
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

use crate::tx::Eip1559TxToSign;

/// Runtime configuration for a keystore-backed transaction signing state.
#[derive(Clone, Debug)]
pub struct KeystoreTxSignStateConfig {
    /// Optional UUID selector for the signing key.
    pub id: Option<String>,
    /// Optional alias selector for the signing key.
    pub by_label: Option<String>,
    /// Transaction payload to sign.
    pub tx: Eip1559TxToSign,
    /// Destination path for the signed raw transaction file.
    pub out_path: PathBuf,
    /// Filesystem path of the keystore directory.
    pub keystore_path: PathBuf,
}

/// State that signs an EIP-1559 transaction through the local keystore transport.
#[derive(Clone, Debug)]
pub struct KeystoreTxSignState {
    /// Stable state identifier assigned by the planner.
    pub state_id: StateId,
    /// Context key that receives the serialized signing report.
    pub output_key: ContextKey,
    /// Execution-time configuration for signing.
    pub cfg: KeystoreTxSignStateConfig,
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
        let mut client = LocalKeystoreIoClient::new(self.state_id.clone(), io);
        let report_json: serde_json::Value = client
            .tx_sign(
                "tx_sign",
                KeystoreTxSignRequest {
                    id: self.cfg.id.clone(),
                    label: None,
                    label_hex: self
                        .cfg
                        .by_label
                        .as_ref()
                        .map(|v| hex::encode(v.as_bytes())),
                    store_path: None,
                    store_path_hex: Some(hex::encode(
                        self.cfg.keystore_path.display().to_string().as_bytes(),
                    )),
                    out_path: None,
                    out_path_hex: Some(hex::encode(
                        self.cfg.out_path.display().to_string().as_bytes(),
                    )),
                    to: format!("{:?}", self.cfg.tx.to),
                    value_wei: self.cfg.tx.value_wei.to_string(),
                    chain_id: self.cfg.tx.chain_id,
                    nonce: self.cfg.tx.nonce,
                    max_fee_per_gas: self.cfg.tx.max_fee_per_gas.to_string(),
                    max_priority_fee_per_gas: self.cfg.tx.max_priority_fee_per_gas.to_string(),
                    gas_limit: self.cfg.tx.gas_limit,
                    data_hex: format!("0x{}", hex::encode(&self.cfg.tx.data)),
                },
            )
            .await
            .map_err(op_errors::state_from_io)
            .map_err(|err| attach_state_id(&self.state_id, err))?;

        op_ctx::write_json(ctx, self.output_key.clone(), report_json.clone())?;
        emit_report_event(rec, "keystore_tx_sign.completed", report_json).await?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}
