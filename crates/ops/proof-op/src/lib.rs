#![cfg_attr(test, allow(clippy::disallowed_methods, clippy::disallowed_types))]
#![cfg_attr(not(test), deny(clippy::disallowed_methods, clippy::disallowed_types))]
#![warn(missing_docs)]
//! Proof op (acceptance tests).
//!
//! Source of truth: `docs/redesign.md`.
//!
//! This crate is intentionally thin: it assembles reusable shared states into a deterministic
//! acceptance-test workflow that exercises read IO, idempotent side effects, and output writing.
//!
//! # Examples
//!
//! ```rust
//! use mfm_op_proof::ProofOp;
//! use mfm_sdk::op::Operation;
//!
//! let op = ProofOp::default();
//! assert_eq!(op.op_id().as_str(), "proof");
//! ```

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use async_trait::async_trait;

use mfm_machine::config::RunConfig;
use mfm_machine::context::DynContext;
use mfm_machine::errors::StateError;
use mfm_machine::ids::{ContextKey, FactKey, OpId, OpPath};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::plan::{DependencyEdge, StateGraph, StateNode};
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};
use mfm_state_common::ctx as op_ctx;
use mfm_state_common::output as op_output;
use mfm_state_common::states::meta;
use mfm_state_common::states::proof::{ProofApplySideEffectState, ProofReadState};
use mfm_state_common::states::side_effect::TriggerOnce;

use mfm_sdk::errors::SdkError;
use mfm_sdk::ids::PortKey;
use mfm_sdk::op::{OpIo, Operation};

#[cfg(test)]
use mfm_machine::errors::ErrorCategory;
#[cfg(test)]
use mfm_machine::errors::ErrorInfo;
#[cfg(test)]
use mfm_machine::events::DomainEvent;
#[cfg(test)]
use mfm_state_common::errors as op_errors;
#[cfg(test)]
use mfm_state_common::idempotency as op_idempotency;

const OP_ID: &str = "proof";
const OP_VERSION: &str = "v1";

// Custom domain event (audit only).
const DOMAIN_EVENT_IDEMPOTENCY_KEY: &str = "proof_idempotency_key";

#[cfg(test)]
fn info(code: &'static str, category: ErrorCategory, retryable: bool, message: &str) -> ErrorInfo {
    op_errors::info(code, category, retryable, message)
}

fn ctx_key(s: &'static str) -> ContextKey {
    ContextKey(s.to_string())
}

fn output_fact_key(op_path: &OpPath) -> FactKey {
    FactKey(format!("proof:output|op:{}", op_path.0))
}

/// Proof op implementation used by acceptance tests.
#[derive(Clone, Default)]
pub struct ProofOp {
    orphan_after_side_effect: Option<TriggerOnce>,
}

impl ProofOp {
    /// Configure this op to request the engine to stop after the side-effect state handler returns once.
    ///
    /// Intended for crash/resume tests (orphan attempt simulation).
    pub fn with_orphan_after_side_effect(
        mut self,
        stop_after_handler_once: Arc<AtomicBool>,
    ) -> Self {
        self.orphan_after_side_effect = Some(TriggerOnce::arm(stop_after_handler_once));
        self
    }
}

impl Operation for ProofOp {
    fn op_id(&self) -> OpId {
        OpId::must_new(OP_ID.to_string())
    }

    fn op_version(&self) -> String {
        OP_VERSION.to_string()
    }

    fn io(&self, _op_config: &serde_json::Value) -> Result<OpIo, SdkError> {
        Ok(OpIo {
            imports: Vec::new(),
            exports: vec![PortKey("output".to_string())],
        })
    }

    fn expand(
        &self,
        op_path: OpPath,
        _op_config: &serde_json::Value,
        _run_config: &RunConfig,
    ) -> Result<StateGraph, SdkError> {
        let read_id = format!("{}.read_facts", op_path.0);
        let side_id = format!("{}.apply_side_effect", op_path.0);
        let out_id = format!("{}.write_output", op_path.0);

        let read_sid = mfm_machine::ids::StateId::must_new(read_id);
        let side_sid = mfm_machine::ids::StateId::must_new(side_id);
        let out_sid = mfm_machine::ids::StateId::must_new(out_id);

        let read = Arc::new(ProofReadState {
            state_id: read_sid.clone(),
            purpose: "proof_read",
            output_key: ctx_key("read_fact"),
            io_error_code: "read_fact_io_failed",
            io_error_message: "failed to read input fact",
        });
        let side = Arc::new(ProofApplySideEffectState {
            state_id: side_sid.clone(),
            op_id: OP_ID,
            input_key: ctx_key("read_fact"),
            idempotency_key_output: ctx_key("idempotency_key"),
            output_key: ctx_key("side_effect_result"),
            event_name: DOMAIN_EVENT_IDEMPOTENCY_KEY.to_string(),
            purpose: "proof_side_effect",
            orphan_after_side_effect: self.orphan_after_side_effect.clone(),
        });
        let out = Arc::new(WriteOutputState {
            op_path: op_path.clone(),
        });

        Ok(StateGraph {
            states: vec![
                StateNode {
                    id: read_sid.clone(),
                    state: read,
                },
                StateNode {
                    id: side_sid.clone(),
                    state: side,
                },
                StateNode {
                    id: out_sid.clone(),
                    state: out,
                },
            ],
            edges: vec![
                DependencyEdge {
                    from: read_sid.clone(),
                    to: side_sid.clone(),
                },
                DependencyEdge {
                    from: side_sid,
                    to: out_sid,
                },
            ],
        })
    }
}

struct WriteOutputState {
    op_path: OpPath,
}

#[async_trait]
impl State for WriteOutputState {
    fn meta(&self) -> StateMeta {
        meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let read_fact = op_ctx::read_json_required(
            ctx,
            &ctx_key("read_fact"),
            "missing_read_fact",
            "missing read_fact in context",
        )?;

        let side_effect = op_ctx::read_json_required(
            ctx,
            &ctx_key("side_effect_result"),
            "missing_side_effect",
            "missing side_effect_result in context",
        )?;

        let output = serde_json::json!({
            "read_fact": read_fact,
            "side_effect_result": side_effect,
        });

        op_output::write_output_artifact(
            ctx,
            io,
            rec,
            output_fact_key(&self.op_path),
            output,
            ctx_key("output_artifact_id"),
        )
        .await?;

        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

#[cfg(test)]
#[path = "tests/proof_op_tests.rs"]
mod proof_op_tests;
