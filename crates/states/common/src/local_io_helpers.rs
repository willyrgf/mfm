use mfm_machine::errors::{ErrorCategory, StateError};
use mfm_machine::events::DomainEvent;
use mfm_machine::hashing::{artifact_id_for_json, CanonicalJsonError};
use mfm_machine::ids::{FactKey, StateId};
use mfm_machine::io::{IoCall, IoProvider};
use mfm_machine::recorder::EventRecorder;
use serde::de::DeserializeOwned;

use crate::errors as op_errors;

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

pub async fn local_call<T: DeserializeOwned>(
    state_id: &StateId,
    io: &mut dyn IoProvider,
    namespace: &str,
    purpose: &str,
    request: serde_json::Value,
) -> Result<T, StateError> {
    let fact_key = local_fact_key(state_id, purpose, &request)?;
    let response = io
        .call(IoCall {
            namespace: namespace.to_string(),
            request,
            fact_key: Some(fact_key),
        })
        .await
        .map_err(op_errors::state_from_io)
        .map_err(|err| attach_state_id(state_id, err))?;

    serde_json::from_value(response.response).map_err(|_| {
        op_errors::state_error_with_state(
            state_id.clone(),
            "LocalResponseDecodeFailed",
            ErrorCategory::Unknown,
            false,
            "failed to decode local io response payload",
        )
    })
}

pub fn local_fact_key(
    state_id: &StateId,
    purpose: &str,
    request: &serde_json::Value,
) -> Result<FactKey, StateError> {
    let req_id = artifact_id_for_json(request).map_err(|err| match err {
        CanonicalJsonError::FloatNotAllowed => op_errors::state_error_with_state(
            state_id.clone(),
            "local_request_not_canonical",
            ErrorCategory::ParsingInput,
            false,
            "local io request was not canonical-json-hashable (floats are forbidden)",
        ),
        CanonicalJsonError::SecretsNotAllowed => op_errors::state_error_with_state(
            state_id.clone(),
            "secrets_detected",
            ErrorCategory::Unknown,
            false,
            "local io request contained secrets",
        ),
    })?;

    Ok(FactKey(format!(
        "mfm:local|state:{}|purpose:{purpose}|req:{}",
        state_id.0, req_id.0
    )))
}

pub fn attach_state_id(state_id: &StateId, mut err: StateError) -> StateError {
    if err.state_id.is_none() {
        err.state_id = Some(state_id.clone());
    }
    err
}
