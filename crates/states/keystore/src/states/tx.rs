//! Reusable states for signing raw transactions via keystore-backed flows.
//!
//! # Security
//!
//! These states bridge local keystore material into durable outputs. They must not leak passwords,
//! raw key material, or decrypted secret buffers through persisted context, reports, or error
//! messages.
//!
//! # Examples
//!
//! ```rust
//! use mfm_collectors_local_keystore::LocalFileWriteMode;
//! use mfm_evm_core::tx::{parse_address, Eip1559TxToSign};
//! use mfm_state_keystore::states::tx::KeystoreTxSignStateConfig;
//!
//! let cfg = KeystoreTxSignStateConfig {
//!     id: Some("550e8400-e29b-41d4-a716-446655440000".to_string()),
//!     tx: Eip1559TxToSign {
//!         to: parse_address("0x0000000000000000000000000000000000000000", "to")?,
//!         value_wei: 0,
//!         chain_id: 1,
//!         nonce: 7,
//!         max_fee_per_gas: 10,
//!         max_priority_fee_per_gas: 1,
//!         gas_limit: 21_000,
//!         data: Vec::new(),
//!     },
//!     out_write_mode: LocalFileWriteMode::CreateNew,
//!     local_resource_handle: "local-keystore:example".to_string(),
//! };
//!
//! assert_eq!(cfg.tx.chain_id, 1);
//! assert_eq!(cfg.local_resource_handle, "local-keystore:example");
//! # Ok::<(), mfm_evm_core::util_error::UtilError>(())
//! ```

use async_trait::async_trait;
use mfm_collectors_local_keystore::{
    KeystoreTxSignRequest, LocalFileWriteMode, LocalKeystoreIoClient,
};
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

use mfm_evm_core::tx::Eip1559TxToSign;

/// Runtime configuration for a keystore-backed transaction signing state.
///
/// Local paths and label selectors are resolved by the live transport through the local resource
/// handle so they do not enter durable run config or snapshots.
#[derive(Clone, Debug)]
pub struct KeystoreTxSignStateConfig {
    /// Optional UUID selector for the signing key.
    pub id: Option<String>,
    /// Transaction payload to sign.
    pub tx: Eip1559TxToSign,
    /// Output write policy for the signed raw transaction file.
    pub out_write_mode: LocalFileWriteMode,
    /// Opaque local binding registered with the live keystore transport.
    pub local_resource_handle: String,
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
                    local_resource_handle: self.cfg.local_resource_handle.clone(),
                    out_write_mode: self.cfg.out_write_mode.clone(),
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
