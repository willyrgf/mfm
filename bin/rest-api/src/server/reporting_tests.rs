use super::*;
use axum::body::to_bytes;
use mfm_ids::{DigestBytes, RunId};
use mfm_runtime::{InvocationFailure, RuntimeError};
use std::error::Error;

fn run_id() -> RunId {
    RunId::from_digest(DigestBytes::from_array([19; 32]))
}

// A REST projection failure after confirmed insertion must preserve the acknowledged frame
// alongside the projection cause.
#[tokio::test]
async fn projection_error_preserves_explicit_acknowledged_head() {
    let candidate = mfm_journal::seal_frame(
        &run_id(),
        1,
        None,
        &mfm_canonical::PlainCanonicalJsonBytes::from_json_str("{}").unwrap(),
    )
    .unwrap();
    let acknowledged = mfm_store::RunSummary::new(
        run_id(),
        1,
        candidate.head_digest().clone(),
        candidate.canonical_bytes().len() as u64,
    )
    .unwrap();
    let expected = serde_json::to_value(&acknowledged).unwrap();
    let failure = RunRequestError::Invocation(InvocationFailure::Execution {
        run_id: run_id(),
        last_observed: None,
        error: RuntimeError::Projection {
            acknowledged: Box::new(acknowledged),
            cause: Box::new(RuntimeError::ArithmeticOverflow),
        },
    });
    let response = error(failure);
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        report["invocation"]["cause"]["projection"]["acknowledged"],
        expected
    );
    assert_eq!(
        report["invocation"]["cause"]["projection"]["cause"],
        "arithmetic_overflow"
    );
}

// REST must report an uncertain append with HTTP 503 while preserving the RunId and exact
// attempted frame for recovery.
#[tokio::test]
async fn ambiguous_append_keeps_503_and_exact_recovery_identity() {
    let candidate = mfm_journal::seal_frame(
        &run_id(),
        1,
        None,
        &mfm_canonical::PlainCanonicalJsonBytes::from_json_str("{}").unwrap(),
    )
    .unwrap();
    let digest = candidate.head_digest().to_string();
    let failure = RunRequestError::AppendIndeterminate {
        recovery: mfm_app::RunRecovery::Progress { run_id: run_id() },
        invocation: InvocationFailure::Execution {
            run_id: run_id(),
            last_observed: None,
            error: RuntimeError::Recording {
                operation: mfm_runtime::Operation::ReadAdapter,
                failure: Box::new(mfm_runtime::RecordingFailure::Store {
                    original: None,
                    candidate,
                    cause: mfm_store::StoreError::Indeterminate(
                        mfm_values::DiagnosticEvidence::from_value(
                            serde_json::json!({"operation": "test.store", "injected": "Indeterminate"}),
                        ),
                    ),
                }),
            },
        },
    };
    let response = error(failure);
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(report["code"], "run_append_indeterminate");
    assert_eq!(
        report["invocation"]["cause"]["recording"]["failure"]["store"]["cause"]["indeterminate"],
        serde_json::json!({"operation": "test.store", "injected": "Indeterminate"})
    );
    assert_eq!(report["recovery"]["run_id"], run_id().as_str());
    assert_eq!(
        report["invocation"]["cause"]["recording"]["failure"]["store"]["candidate"]["digest"],
        digest
    );
}

// If even the final failure body cannot be encoded, its concrete JSON cause must survive in the
// body error channel.
#[tokio::test]
async fn final_encoding_failure_preserves_concrete_json_error_in_body_channel() {
    let source = serde_json::from_str::<bool>("invalid terminal JSON").unwrap_err();
    let expected = source.to_string();
    let response = final_response(
        StatusCode::CONFLICT,
        Err(mfm_canonical::JsonError::new(source)),
    );
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(response.headers().get(CONTENT_TYPE).is_none());
    let body_error = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap_err();
    let mut source: &dyn Error = &body_error;
    loop {
        if let Some(json) = source.downcast_ref::<mfm_canonical::JsonError>() {
            assert_eq!(serde_json::to_value(json).unwrap()["message"], expected);
            break;
        }
        source = source.source().expect("concrete JSON cause");
    }
}
