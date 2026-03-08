use mfm_machine::errors::StateError;
use mfm_machine::events::DomainEvent;
use mfm_machine::recorder::EventRecorder;

use crate::errors as op_errors;

/// Emits a small in-memory domain event without attaching an artifact payload.
pub async fn emit_report_event(
    rec: &mut dyn EventRecorder,
    name: &str,
    payload: serde_json::Value,
) -> Result<(), StateError> {
    rec.emit(DomainEvent {
        name: name.to_string(),
        payload,
        payload_ref: None,
    })
    .await
    .map_err(|_| op_errors::state_unknown("emit_failed", "failed to emit domain event"))
}

/// Ensures a [`StateError`] is attached to the supplied state id.
pub fn attach_state_id(state_id: &mfm_machine::ids::StateId, mut err: StateError) -> StateError {
    if err.state_id.is_none() {
        err.state_id = Some(state_id.clone());
    }
    err
}
