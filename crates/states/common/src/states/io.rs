use async_trait::async_trait;

use mfm_machine::context::DynContext;
use mfm_machine::errors::StateError;
use mfm_machine::ids::{ContextKey, FactKey};
use mfm_machine::io::{IoCall, IoProvider};
use mfm_machine::meta::StateMeta;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::state::{SnapshotPolicy, State, StateOutcome};

use crate::ctx as op_ctx;
use crate::errors as op_errors;
use crate::states::meta;

/// Generic read-only IO state that stores the JSON response in context.
#[derive(Clone, Debug)]
pub struct NamespaceReadState {
    /// IO namespace routed through the active [`IoProvider`](mfm_machine::io::IoProvider).
    pub namespace: String,
    /// JSON request payload sent to the namespace client.
    pub request: serde_json::Value,
    /// Fact key used to record and replay the request.
    pub fact_key: FactKey,
    /// Context key that receives the JSON response payload.
    pub output_key: ContextKey,
    /// Stable error code returned when the IO call fails.
    pub io_error_code: &'static str,
    /// Human-readable error message returned when the IO call fails.
    pub io_error_message: &'static str,
}

#[async_trait]
impl State for NamespaceReadState {
    fn meta(&self) -> StateMeta {
        meta::read_only_io_with_tag(meta::tags::READ_ONLY_IO)
    }

    async fn handle(
        &self,
        ctx: &mut dyn DynContext,
        io: &mut dyn IoProvider,
        _rec: &mut dyn EventRecorder,
    ) -> Result<StateOutcome, StateError> {
        let res = io
            .call(IoCall {
                namespace: self.namespace.clone(),
                request: self.request.clone(),
                fact_key: Some(self.fact_key.clone()),
            })
            .await
            .map_err(|_| op_errors::state_unknown_msg(self.io_error_code, self.io_error_message))?;

        op_ctx::write_json(ctx, self.output_key.clone(), res.response)?;
        Ok(StateOutcome {
            snapshot: SnapshotPolicy::OnSuccess,
        })
    }
}
