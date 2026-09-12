use axum::body::{Body, Bytes};
use axum::http::{header::CONTENT_TYPE, StatusCode};
use axum::response::{IntoResponse, Response};
use mfm_app::{
    encode_response, ReportFailure, ReportStage, RunRequestError, SerializableClientError,
    SerializableRunView, StartRunResult,
};
use mfm_runtime::RunView;
use mfm_values::NativeCause;

pub(super) fn start(result: StartRunResult) -> Response {
    let encoded = result
        .serializable()
        .map_err(|cause| (ReportStage::Prepare, cause))
        .and_then(|model| encode_response(&model).map_err(|cause| (ReportStage::Encode, cause)));
    match encoded {
        Ok(bytes) => json(StatusCode::OK, bytes),
        Err((stage, cause)) => {
            let mut failure = ReportFailure::new(result, stage, cause);
            let status = report_failure_status(&failure);
            let encoded = encode_response(&failure.incomplete());
            incomplete(status, failure, encoded)
        }
    }
}

pub(super) fn view(view: RunView) -> Response {
    let encoded = SerializableRunView::new(&view)
        .map_err(|cause| (ReportStage::Prepare, cause))
        .and_then(|model| encode_response(&model).map_err(|cause| (ReportStage::Encode, cause)));
    match encoded {
        Ok(bytes) => json(StatusCode::OK, bytes),
        Err((stage, cause)) => {
            let mut failure = ReportFailure::new(view, stage, cause);
            let status = report_failure_status(&failure);
            let encoded = encode_response(&failure.incomplete());
            incomplete(status, failure, encoded)
        }
    }
}

pub(super) fn error(error: RunRequestError) -> Response {
    let status = error
        .request_error()
        .map(super::request_error_status)
        .unwrap_or(StatusCode::SERVICE_UNAVAILABLE);
    let message = error.to_string();
    let encoded = SerializableClientError::for_run(&error, &message)
        .map_err(|cause| (ReportStage::Prepare, cause))
        .and_then(|model| encode_response(&model).map_err(|cause| (ReportStage::Encode, cause)));
    match encoded {
        Ok(bytes) => json(status, bytes),
        Err((stage, cause)) => {
            let mut failure = ReportFailure::new(error, stage, cause);
            let encoded = encode_response(&failure.incomplete());
            incomplete(status, failure, encoded)
        }
    }
}

fn report_failure_status<T>(failure: &ReportFailure<T>) -> StatusCode {
    if failure.size_limit().is_some() {
        StatusCode::UNPROCESSABLE_ENTITY
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

fn json(status: StatusCode, bytes: Vec<u8>) -> Response {
    (status, [(CONTENT_TYPE, "application/json")], bytes).into_response()
}

fn incomplete<T: Send + Sync + 'static>(
    status: StatusCode,
    failure: ReportFailure<T>,
    encoded: Result<Vec<u8>, NativeCause>,
) -> Response {
    match encoded {
        Ok(bytes) => json(status, bytes),
        Err(cause) => {
            // Handoff retains every preparation failure. A body error does not establish delivery
            // or permit another JSON attempt through a serializer that has already failed.
            let failure = ReportFailure::new(failure, ReportStage::Encode, cause);
            let body = Body::from_stream(tokio_stream::once(Err::<Bytes, _>(failure)));
            (status, body).into_response()
        }
    }
}

#[cfg(test)]
#[path = "reporting_tests.rs"]
mod tests;
