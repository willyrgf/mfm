use mfm_machine::context::DynContext;
use mfm_machine::errors::StateError;
use mfm_machine::events::{ArtifactWritten, DomainEvent, DOMAIN_EVENT_ARTIFACT_WRITTEN};
use mfm_machine::ids::{ContextKey, FactKey};
use mfm_machine::io::IoProvider;
use mfm_machine::recorder::EventRecorder;
use mfm_machine::stores::ArtifactKind;

use crate::ctx::write_json;
use crate::errors::state_unknown;

/// Records a JSON output payload as an output artifact and stores its id in context.
///
/// When the fact key was not already bound, this helper also emits the standard
/// `artifact_written` domain event for the new output artifact.
pub async fn write_output_artifact(
    ctx: &mut dyn DynContext,
    io: &mut dyn IoProvider,
    rec: &mut dyn EventRecorder,
    fact_key: FactKey,
    value: serde_json::Value,
    output_ctx_key: ContextKey,
) -> Result<(), StateError> {
    let existed = io
        .get_recorded_fact(&fact_key)
        .await
        .map_err(|_| state_unknown("io_fact_lookup_failed", "failed to lookup recorded fact"))?
        .is_some();

    let payload_id = io
        .record_value(fact_key, value)
        .await
        .map_err(|_| state_unknown("output_record_failed", "output record failed"))?;

    write_json(ctx, output_ctx_key, serde_json::json!(payload_id.0.clone()))?;

    if !existed {
        let payload = serde_json::to_value(ArtifactWritten {
            artifact_id: payload_id,
            kind: ArtifactKind::Output,
            meta: serde_json::json!({}),
        })
        .map_err(|_| {
            state_unknown(
                "artifact_written_serialize_failed",
                "failed to serialize ArtifactWritten",
            )
        })?;

        rec.emit(DomainEvent {
            name: DOMAIN_EVENT_ARTIFACT_WRITTEN.to_string(),
            payload,
            payload_ref: None,
        })
        .await
        .map_err(|_| state_unknown("emit_failed", "failed to emit artifact_written"))?;
    }

    Ok(())
}
