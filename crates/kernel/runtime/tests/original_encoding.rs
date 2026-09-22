use mfm_ids::{DigestBytes, RunId};
use mfm_runtime::{Operation, RecordingFailure, RuntimeError, Stage};
use mfm_values::{SizeLimitExceeded, SizeResource, SizeViolation, ValueError};

#[test]
fn serialization_stop_reports_a_lower_bound_and_preserves_the_native_encoding_cause() {
    let source = mfm_canonical::to_json_bounded(&"a value longer than the ceiling", 8).unwrap_err();
    let error = RuntimeError::Native {
        operation: Operation::ReadAdapter,
        stage: Stage::Encode,
        cause: ValueError::Canonical(source).into_diagnostic("encode"),
    };
    assert!(
        matches!(error.size_limit(), Some(SizeViolation::SerializationBound {
        resource: SizeResource::CanonicalObject, observed_at_least, limit: 8,
    }) if observed_at_least > 8)
    );
    let projection = serde_json::to_value(error.size_limit().unwrap()).unwrap();
    assert!(projection.get("actual").is_none());
    assert!(
        serde_json::to_value(&error).unwrap()["native"]["cause"]["details"]["canonical"]
            ["serialization_limit"]["source"]["message"]
            .is_string()
    );
}

#[test]
fn append_size_projection_uses_the_physical_outcome_and_ignores_secondary_reload_errors() {
    let run = RunId::from_digest(DigestBytes::from_array([157; 32]));
    let payload = mfm_canonical::PlainCanonicalJsonBytes::from_json_str("{}").unwrap();
    let candidate = mfm_journal::seal_frame(&run, 1, None, &payload).unwrap();
    let size = SizeLimitExceeded::check(11, 10).unwrap_err();
    let secondary = RuntimeError::SizeLimit {
        resource: SizeResource::Frame,
        size,
    };
    let error = RuntimeError::Recording {
        operation: Operation::ReadAdapter,
        failure: Box::new(RecordingFailure::NotInserted {
            original: None,
            candidate: candidate.clone(),
            observation: None,
            reload_cause: Some(Box::new(secondary)),
        }),
    };
    assert!(error.size_limit().is_none());
    let error = RuntimeError::Recording {
        operation: Operation::ReadAdapter,
        failure: Box::new(RecordingFailure::Store {
            original: None,
            candidate,
            cause: mfm_store::StoreError::HistorySize(size),
        }),
    };
    assert_eq!(
        error.size_limit(),
        Some(SizeViolation::Measured {
            resource: SizeResource::HistoryBytes,
            actual: 11,
            limit: 10,
        })
    );
}
