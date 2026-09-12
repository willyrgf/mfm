use super::*;
use axum::body::to_bytes;
use mfm_ids::{DigestBytes, RunId};
use mfm_runtime::{InvocationFailure, RuntimeError};
use std::error::Error;

#[derive(Debug, serde::Serialize, thiserror::Error)]
#[error("reviewed test cause")]
struct Cause {
    code: u64,
}

fn run_id() -> RunId {
    RunId::from_digest(DigestBytes::from_array([19; 32]))
}

#[tokio::test]
async fn incomplete_projection_preserves_explicit_acknowledged_head() {
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
            cause: NativeCause::from_error_with(Cause { code: 1 }, |_| {
                Err(NativeCause::from_error(Cause { code: 2 }))
            }),
        },
    });
    let response = error(failure);
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(report["acknowledged"], expected);
    assert!(report.get("last_observed").is_none());
    assert_eq!(report["report_failure"]["cause"]["code"], 2);
    assert_eq!(
        report["report_failure"]["omissions"][0]["field"],
        "original_detail"
    );
}

#[tokio::test]
async fn incomplete_ambiguous_report_keeps_503_and_exact_recovery_identity() {
    let candidate = mfm_journal::seal_frame(
        &run_id(),
        1,
        None,
        &mfm_canonical::PlainCanonicalJsonBytes::from_json_str("{}").unwrap(),
    )
    .unwrap();
    let failure = RunRequestError::AppendIndeterminate {
        recovery: mfm_app::RunRecovery::Progress { run_id: run_id() },
        invocation: InvocationFailure::Execution {
            run_id: run_id(),
            last_observed: None,
            error: RuntimeError::Recording {
                operation: mfm_runtime::Operation::ReadAdapter,
                failure: Box::new(mfm_runtime::RecordingFailure::Append {
                    original: Some(NativeCause::from_error_with(Cause { code: 3 }, |_| {
                        Err(NativeCause::from_error(Cause { code: 4 }))
                    })),
                    candidate,
                    outcome: mfm_runtime::AppendFailure::Store(
                        mfm_store::StoreError::Indeterminate,
                    ),
                    observation: None,
                    reload_cause: None,
                }),
            },
        },
    };
    let response = error(failure);
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(report["code"], "run_append_indeterminate");
    assert_eq!(report["recovery"]["run_id"], run_id().as_str());
    assert!(report.get("acknowledged").is_none());
    assert_eq!(report["report_failure"]["cause"]["code"], 4);
}

#[tokio::test]
async fn failed_incomplete_encoding_yields_retained_native_body_error_without_json_fallback() {
    let original = RunRequestError::Request(mfm_app::RequestError::RunAppendNotInserted);
    let failure = ReportFailure::new(
        original,
        ReportStage::Prepare,
        NativeCause::from_error(Cause { code: 5 }),
    );
    let response = incomplete(
        StatusCode::CONFLICT,
        failure,
        Err(NativeCause::from_error(Cause { code: 6 })),
    );
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(response.headers().get(CONTENT_TYPE).is_none());
    let body_error = to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap_err();
    let mut source: &dyn Error = &body_error;
    let retained = loop {
        if let Some(retained) =
            source.downcast_ref::<ReportFailure<ReportFailure<RunRequestError>>>()
        {
            break retained;
        }
        source = source
            .source()
            .expect("native report custody in body error chain");
    };
    assert_eq!(
        retained.original().original().code(),
        "run_append_not_inserted"
    );
    assert_eq!(
        retained
            .original()
            .cause()
            .downcast_ref::<Cause>()
            .unwrap()
            .code,
        5
    );
    assert_eq!(retained.cause().downcast_ref::<Cause>().unwrap().code, 6);
}
