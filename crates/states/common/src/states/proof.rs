use async_trait::async_trait;

use mfm_collectors_proof::{
    fact_key_for_request, ProofIoClient, ProofReadRequest, ProofSideEffectRequest,
};
use mfm_machine::context::DynContext;
use mfm_machine::errors::StateError;
use mfm_machine::events::DomainEvent;
use mfm_machine::ids::{ContextKey, StateId};
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};

use crate::ctx as op_ctx;
use crate::errors as op_errors;
use crate::idempotency as op_idempotency;
use crate::states::meta;
use crate::states::side_effect::TriggerOnce;

/// Typed proof read state that records a fact and writes the decoded response into context.
#[derive(Clone, Debug)]
pub struct ProofReadState {
    /// Stable state identifier assigned by the planner.
    pub state_id: StateId,
    /// Stable fact-key purpose segment used by the proof client.
    pub purpose: &'static str,
    /// Context key that receives the serialized response payload.
    pub output_key: ContextKey,
    /// Stable error code returned when the proof read fails.
    pub io_error_code: &'static str,
    /// Human-readable error message returned when the proof read fails.
    pub io_error_message: &'static str,
}

#[async_trait]
impl State for ProofReadState {
    fn meta(&self) -> StateMeta {
        meta::read_only_io_with_tag(meta::tags::READ_ONLY_IO)
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn mfm_machine::io::IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let mut client = ProofIoClient::new(self.state_id.clone(), io);
        let response = client
            .read(self.purpose, ProofReadRequest::default())
            .await
            .map_err(|_| op_errors::state_unknown_msg(self.io_error_code, self.io_error_message))?;

        let response = serde_json::to_value(response).map_err(|_| {
            op_errors::state_unknown_msg(
                "proof_response_serialize_failed",
                "failed to serialize proof read response",
            )
        })?;
        op_ctx::write_json(ctx, self.output_key.clone(), response)?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

/// Typed proof side-effect state that binds requests to a stable idempotency key.
#[derive(Clone)]
pub struct ProofApplySideEffectState {
    /// Stable state identifier assigned by the planner.
    pub state_id: StateId,
    /// Operation id used when building shared metadata.
    pub op_id: &'static str,
    /// Context key that holds the input payload used to derive the idempotency key.
    pub input_key: ContextKey,
    /// Context key that receives the derived idempotency key.
    pub idempotency_key_output: ContextKey,
    /// Context key that receives the proof-side-effect response payload.
    pub output_key: ContextKey,
    /// Domain-event name emitted before a new side effect is applied.
    pub event_name: String,
    /// Stable fact-key purpose segment used by the proof client.
    pub purpose: &'static str,
    /// Optional one-shot failpoint trigger used by recovery tests.
    pub orphan_after_side_effect: Option<TriggerOnce>,
}

#[async_trait]
impl State for ProofApplySideEffectState {
    fn meta(&self) -> StateMeta {
        meta::apply_side_effect(op_idempotency::state_purpose(
            self.op_id,
            &self.state_id,
            "apply_side_effect",
        ))
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn mfm_machine::io::IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let read_fact = op_ctx::read_json_required(
            ctx,
            &self.input_key,
            "missing_read_fact",
            "missing read_fact in context",
        )?;

        let id_key = op_idempotency::idempotency_key_for_value(&read_fact)?;
        op_ctx::write_json(
            ctx,
            self.idempotency_key_output.clone(),
            serde_json::json!(id_key.clone()),
        )?;

        let request = ProofSideEffectRequest {
            idempotency_key: id_key.clone(),
        };
        let fact_key =
            fact_key_for_request(&self.state_id, self.purpose, &request).map_err(|_| {
                op_errors::state_unknown_msg(
                    "proof_request_not_canonical",
                    "proof side-effect request was not canonical-json-hashable",
                )
            })?;
        let existing = io.get_recorded_fact(&fact_key).await.map_err(|_| {
            op_errors::state_unknown_msg("io_fact_lookup_failed", "failed to lookup recorded fact")
        })?;

        if existing.is_none() {
            rec.emit(DomainEvent {
                name: self.event_name.clone(),
                payload: serde_json::json!({"key": id_key}),
                payload_ref: None,
            })
            .await
            .map_err(|_| {
                op_errors::state_unknown_msg("emit_failed", "failed to emit idempotency event")
            })?;
        }

        let mut client = ProofIoClient::new(self.state_id.clone(), io);
        let response = client
            .apply_side_effect(self.purpose, request)
            .await
            .map_err(|_| {
                op_errors::state_unknown_msg("side_effect_io_failed", "side effect io failed")
            })?;

        if let Some(orphan) = &self.orphan_after_side_effect {
            orphan.trigger_if_armed();
        }

        let response = serde_json::to_value(response).map_err(|_| {
            op_errors::state_unknown_msg(
                "proof_response_serialize_failed",
                "failed to serialize proof side-effect response",
            )
        })?;
        op_ctx::write_json(ctx, self.output_key.clone(), response)?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}
