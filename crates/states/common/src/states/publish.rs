use async_trait::async_trait;

use mfm_machine::context::DynContext;
use mfm_machine::errors::StateError;
use mfm_machine::ids::{ContextKey, FactKey, StateId};
use mfm_machine::io::IoProvider;
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};

use crate::ctx;
use crate::output;
use crate::states::meta;

/// Pure shared state that writes a fixed JSON value into context.
#[derive(Clone)]
pub struct WriteJsonValueState {
    /// Stable runtime state id.
    pub state_id: StateId,
    /// Context key that should receive the value.
    pub output_key: ContextKey,
    /// Deterministic JSON payload to publish.
    pub value: serde_json::Value,
}

#[async_trait]
impl State for WriteJsonValueState {
    fn meta(&self) -> StateMeta {
        meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        _io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        ctx::write_json(ctx, self.output_key.clone(), self.value.clone())?;
        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}

/// Pure shared state that persists a JSON value from context as an output artifact.
#[derive(Clone)]
pub struct WriteContextValueArtifactState {
    /// Stable runtime state id.
    pub state_id: StateId,
    /// Context key that must already contain the JSON payload to persist.
    pub input_key: ContextKey,
    /// Fact key used for deterministic content-addressed persistence.
    pub fact_key: FactKey,
    /// Context key that should receive the persisted artifact id.
    pub output_artifact_id_key: ContextKey,
    /// Stable error code used when the input key is missing.
    pub missing_input_code: &'static str,
    /// Stable error message used when the input key is missing.
    pub missing_input_message: &'static str,
}

#[async_trait]
impl State for WriteContextValueArtifactState {
    fn meta(&self) -> StateMeta {
        meta::pure()
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let value = ctx::read_json_required(
            ctx,
            &self.input_key,
            self.missing_input_code,
            self.missing_input_message,
        )?;
        output::write_output_artifact(
            ctx,
            io,
            rec,
            self.fact_key.clone(),
            value,
            self.output_artifact_id_key.clone(),
        )
        .await?;
        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}
