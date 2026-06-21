use super::{CodecError, IdentityError};

/// Domain parse/encode functions for kernel events, projections, and saga types.
///
/// Defined at the `v1` root (where the surrounding store internals live) and re-exported
/// here so the Postgres adapter consumes one implementation through this module.
pub use super::{
    attempt_projection_json, cell_projection_json, error_category_str, error_info_json,
    event_artifact_json, fact_projection_json, failure_phase_str, manual_resolution_note_json,
    manual_resolution_outcome_str, manual_resolution_projection_json, parse_attempt_projection,
    parse_cell_projection, parse_error_category, parse_error_info, parse_event_artifact,
    parse_fact_projection, parse_failure_phase, parse_manual_resolution_note,
    parse_manual_resolution_outcome, parse_manual_resolution_projection,
    parse_public_output_projection, parse_resource_key_evidence, parse_resource_lane_projection,
    parse_resource_touched_set_evidence, parse_retention_manifest_projection,
    parse_run_completion_outcome, parse_run_completion_projection, parse_run_state,
    parse_saga_engagement_projection, parse_side_effect_ledger_purpose,
    parse_side_effect_projection, parse_skip_reason, public_output_projection_json,
    resource_key_evidence_json, resource_lane_projection_json, resource_touched_set_evidence_json,
    retention_manifest_projection_json, retention_ref_json, run_completion_claim_str,
    run_completion_outcome_json, run_completion_outcome_str, run_completion_projection_json,
    run_state_str, saga_engagement_projection_json, side_effect_ledger_purpose_json,
    side_effect_projection_json, skip_reason_json,
};

/// Codec result over the backend-neutral [`CodecError`].
pub type CodecResult<T> = std::result::Result<T, CodecError>;

/// Parses a typed identity from its canonical string form.
pub fn parse_identity<T>(value: &str) -> CodecResult<T>
where
    T: std::str::FromStr<Err = IdentityError>,
{
    Ok(value.parse()?)
}

/// Returns a required string field.
pub fn required_str<'a>(json: &'a serde_json::Value, field: &'static str) -> CodecResult<&'a str> {
    json.get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| CodecError::Field(format!("missing string field {field}")))
}

/// Returns an optional string field, treating null or absent as `None`.
pub fn optional_str<'a>(
    json: &'a serde_json::Value,
    field: &'static str,
) -> CodecResult<Option<&'a str>> {
    match json.get(field) {
        Some(serde_json::Value::Null) | None => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| CodecError::Field(format!("field {field} was not a string"))),
    }
}

/// Returns a required object field.
pub fn required_obj<'a>(
    json: &'a serde_json::Value,
    field: &'static str,
) -> CodecResult<&'a serde_json::Value> {
    let value = json
        .get(field)
        .ok_or_else(|| CodecError::Field(format!("missing object field {field}")))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(CodecError::Field(format!(
            "field {field} was not an object"
        )))
    }
}

/// Returns an optional object field, treating null or absent as `None`.
pub fn optional_obj<'a>(
    json: &'a serde_json::Value,
    field: &'static str,
) -> CodecResult<Option<&'a serde_json::Value>> {
    match json.get(field) {
        Some(serde_json::Value::Null) | None => Ok(None),
        Some(value) if value.is_object() => Ok(Some(value)),
        Some(_) => Err(CodecError::Field(format!(
            "field {field} was not an object"
        ))),
    }
}

/// Returns a required unsigned 64-bit field.
pub fn required_u64(json: &serde_json::Value, field: &'static str) -> CodecResult<u64> {
    json.get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| CodecError::Field(format!("missing u64 field {field}")))
}

/// Returns a required unsigned 32-bit field.
pub fn required_u32(json: &serde_json::Value, field: &'static str) -> CodecResult<u32> {
    required_u64(json, field)?
        .try_into()
        .map_err(|_| CodecError::Field(format!("{field} overflowed u32")))
}

/// Returns a required boolean field.
pub fn required_bool(json: &serde_json::Value, field: &'static str) -> CodecResult<bool> {
    json.get(field)
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| CodecError::Field(format!("missing bool field {field}")))
}
