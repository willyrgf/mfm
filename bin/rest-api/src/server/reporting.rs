use axum::body::{Body, Bytes};
use axum::http::{header::CONTENT_TYPE, StatusCode};
use axum::response::{IntoResponse, Response};
use mfm_app::{
    encode_response, RunRequestError, SerializableClientError, SerializableRunView, StartRunResult,
};
use mfm_runtime::RunView;
use mfm_values::InvocationDiagnostic;
use serde_json::value::RawValue;

pub(super) fn start(result: StartRunResult) -> Response {
    let encoded = result.serializable().and_then(|model| {
        encode_response(&model)
            .map_err(|cause| InvocationDiagnostic::from_fields("json_error", "start", &cause, None))
    });
    match encoded {
        Ok(bytes) => json(StatusCode::OK, bytes),
        Err(diagnostic) => {
            let status = report_failure_status(&diagnostic);
            let terminal =
                SerializableClientError::failed_view_report(result.run(), &diagnostic, None);
            final_response(status, encode_response(&terminal))
        }
    }
}

pub(super) fn view(view: RunView) -> Response {
    let encoded = SerializableRunView::new(&view).and_then(|model| {
        encode_response(&model)
            .map_err(|cause| InvocationDiagnostic::from_fields("json_error", "view", &cause, None))
    });
    match encoded {
        Ok(bytes) => json(StatusCode::OK, bytes),
        Err(diagnostic) => {
            let status = report_failure_status(&diagnostic);
            let terminal = SerializableClientError::failed_view_report(&view, &diagnostic, None);
            final_response(status, encode_response(&terminal))
        }
    }
}

pub(super) fn error(error: RunRequestError) -> Response {
    let status = error
        .request_error()
        .map(super::request_error_status)
        .unwrap_or(StatusCode::SERVICE_UNAVAILABLE);
    let message = error.to_string();
    let encoded = SerializableClientError::for_run(&error, &message).and_then(|model| {
        encode_response(&model)
            .map_err(|cause| InvocationDiagnostic::from_fields("json_error", "error", &cause, None))
    });
    match encoded {
        Ok(bytes) => json(status, bytes),
        Err(diagnostic) => {
            let terminal =
                SerializableClientError::failed_run_report(&error, &message, &diagnostic, None);
            final_response(status, encode_response(&terminal))
        }
    }
}

fn report_failure_status(diagnostic: &InvocationDiagnostic) -> StatusCode {
    if diagnostic.size().is_some() {
        StatusCode::UNPROCESSABLE_ENTITY
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

fn json(status: StatusCode, bytes: Box<RawValue>) -> Response {
    let text: Box<str> = bytes.into();
    (
        status,
        [(CONTENT_TYPE, "application/json")],
        String::from(text),
    )
        .into_response()
}

fn final_response(
    status: StatusCode,
    encoded: Result<Box<RawValue>, mfm_canonical::JsonError>,
) -> Response {
    match encoded {
        Ok(bytes) => json(status, bytes),
        Err(cause) => {
            // A one-shot body failure preserves the concrete encoder error without another report.
            let body = Body::from_stream(tokio_stream::once(Err::<Bytes, _>(cause)));
            (status, body).into_response()
        }
    }
}

#[cfg(test)]
#[path = "reporting_tests.rs"]
mod tests;
