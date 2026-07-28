use mfm_app::{AccessPolicyError, ErrorClass, PublicError};
use serde_json::json;

#[test]
fn authentication_grant_and_tenant_hidden_absence_have_distinct_status_classes() {
    let authentication: PublicError = AccessPolicyError::AuthenticationRequired.into();
    assert_public_error(
        &authentication,
        ErrorClass::Unauthorized,
        axum::http::StatusCode::UNAUTHORIZED,
        "AuthenticationRequired",
        "Authentication is required",
    );

    let grant: PublicError = AccessPolicyError::GrantDenied.into();
    assert_public_error(
        &grant,
        ErrorClass::Forbidden,
        axum::http::StatusCode::FORBIDDEN,
        "GrantDenied",
        "The credential does not grant this operation",
    );

    let source = PublicError::source_run_export_denied();
    assert_public_error(
        &source,
        ErrorClass::Forbidden,
        axum::http::StatusCode::FORBIDDEN,
        "SourceRunExportDenied",
        "A required source run does not grant export access",
    );

    let absent = PublicError::run_not_found();
    assert_public_error(
        &absent,
        ErrorClass::NotFound,
        axum::http::StatusCode::NOT_FOUND,
        "RunNotFound",
        "The requested run was not found",
    );

    for error in [
        mfm_replay::v1::ReplayError::InvalidRecordedHistory,
        mfm_replay::v1::ReplayError::CandidateExecutionFailed,
        mfm_replay::v1::ReplayError::ComparisonIntegrityFailed,
        mfm_replay::v1::ReplayError::InvalidExport,
    ] {
        let integrity: PublicError = error.into();
        assert_public_error(
            &integrity,
            ErrorClass::Internal,
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "ReplayVerificationFailed",
            "Recorded run evidence failed verification",
        );
    }

    let unavailable: PublicError = mfm_replay::v1::ReplayError::CandidateUnavailable.into();
    assert_public_error(
        &unavailable,
        ErrorClass::ServiceUnavailable,
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "RuntimeCatalogUnavailable",
        "The exact admitted runtime catalog is unavailable",
    );
}

fn assert_public_error(
    error: &PublicError,
    class: ErrorClass,
    status: axum::http::StatusCode,
    code: &str,
    message: &str,
) {
    assert_eq!(error.class, class);
    assert_eq!(mfm_rest_api::ApiError::from(error.clone()).status(), status);
    assert_eq!(
        serde_json::to_value(error).expect("serialize public error"),
        json!({ "code": code, "message": message }),
        "transport payload must not reveal policy, tenant, or backend diagnostics"
    );
}
