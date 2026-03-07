//! Reusable idempotent side-effect state helpers.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;

use mfm_machine::context::DynContext;
use mfm_machine::errors::StateError;
use mfm_machine::events::DomainEvent;
use mfm_machine::ids::{ContextKey, FactKey, OpPath, StateId};
use mfm_machine::io::{IoCall, IoProvider};
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};

use crate::ctx as op_ctx;
use crate::errors as op_errors;
use crate::idempotency as op_idempotency;
use crate::states::meta;

/// Test-only trigger that arms a single post-handler failpoint.
#[derive(Clone)]
pub struct TriggerOnce {
    stop_after_handler_once: Arc<AtomicBool>,
    armed_once: Arc<AtomicBool>,
}

impl TriggerOnce {
    /// Arms a one-shot trigger backed by the supplied atomic failpoint flag.
    pub fn arm(stop_after_handler_once: Arc<AtomicBool>) -> Self {
        Self {
            stop_after_handler_once,
            armed_once: Arc::new(AtomicBool::new(true)),
        }
    }

    fn trigger_if_armed(&self) {
        if self.armed_once.swap(false, Ordering::SeqCst) {
            self.stop_after_handler_once.store(true, Ordering::SeqCst);
        }
    }
}

/// Generic side-effecting state that records an idempotency key and reuses recorded facts.
#[derive(Clone)]
pub struct IdempotentSideEffectState {
    /// Stable id of the state executing the side effect.
    pub state_id: StateId,
    /// Operation id used when building idempotency scopes.
    pub op_id: &'static str,
    /// Operation path used in recorded fact keys.
    pub op_path: OpPath,
    /// Context key that holds the input payload used to derive the idempotency key.
    pub input_key: ContextKey,
    /// Context key that receives the derived idempotency key.
    pub idempotency_key_output: ContextKey,
    /// Context key that receives the side-effect response payload.
    pub output_key: ContextKey,
    /// IO namespace that performs the side effect.
    pub namespace: String,
    /// Prefix used when constructing the recorded fact key.
    pub fact_key_prefix: &'static str,
    /// Domain-event name emitted before a new side effect is applied.
    pub event_name: String,
    /// Optional one-shot failpoint trigger used by recovery tests.
    pub orphan_after_side_effect: Option<TriggerOnce>,
}

#[async_trait]
impl State for IdempotentSideEffectState {
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
        io: &mut dyn IoProvider,
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

        let fact_key = FactKey(format!(
            "{}|op:{}|id:{id_key}",
            self.fact_key_prefix, self.op_path.0
        ));
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

        let res = io
            .call(IoCall {
                namespace: self.namespace.clone(),
                request: serde_json::json!({"idempotency_key": id_key}),
                fact_key: Some(fact_key),
            })
            .await
            .map_err(|_| {
                op_errors::state_unknown_msg("side_effect_io_failed", "side effect io failed")
            })?;

        if let Some(orphan) = &self.orphan_after_side_effect {
            orphan.trigger_if_armed();
        }

        op_ctx::write_json(ctx, self.output_key.clone(), res.response)?;
        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}
