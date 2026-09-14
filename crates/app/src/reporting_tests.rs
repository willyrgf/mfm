use crate::*;
use mfm_runtime::{Operation, RecordingFailure};
use mfm_values::{InvocationDiagnostic, SizeResource};

#[test]
fn final_run_report_keeps_primary_noninsertion_and_candidate_despite_secondary_size() {
    let run_id = RunId::from_digest(mfm_ids::DigestBytes::from_array([78; 32]));
    let candidate = mfm_journal::seal_frame(
        &run_id,
        1,
        None,
        &mfm_canonical::PlainCanonicalJsonBytes::from_json_str("{}").unwrap(),
    )
    .unwrap();
    let digest = candidate.head_digest().to_string();
    let error = RunRequestError::Invocation(InvocationFailure::Execution {
        run_id: run_id.clone(),
        last_observed: None,
        error: RuntimeError::Recording {
            operation: Operation::Record,
            failure: Box::new(RecordingFailure::NotInserted {
                original: None,
                candidate,
                observation: None,
                reload_cause: Some(Box::new(RuntimeError::SizeLimit {
                    resource: SizeResource::Frame,
                    size: mfm_values::SizeLimitExceeded::check(11, 10).unwrap_err(),
                })),
            }),
        },
    });
    assert_eq!(error.code(), "run_append_not_inserted");
    assert_eq!(
        error.request_error(),
        Some(RequestError::RunAppendNotInserted)
    );
    let diagnostic = InvocationDiagnostic::from_fields(
        "json_error",
        "emit_error",
        &serde_json::json!({"message": "encoding failure"}),
        None,
    );
    let model = SerializableClientError::failed_run_report(
        &error,
        "run append inserted nothing",
        &diagnostic,
        None,
    );
    let report: serde_json::Value =
        serde_json::from_str(encode_response(&model).unwrap().get()).unwrap();
    assert_eq!(report["code"], "run_append_not_inserted");
    assert_eq!(report["run_id"], run_id.as_str());
    assert_eq!(report["candidate"]["digest"], digest);
    assert_eq!(report["candidate"]["sequence"], 1);
    assert!(report["candidate"].get("payload").is_none());
    assert!(report["observation"].is_null());
    assert!(report["original_report"].is_null());
    assert!(report.get("size_limit").is_none());
    assert_eq!(
        report["diagnostic"]["details"]["message"],
        "encoding failure"
    );
}

#[test]
fn final_run_report_preserves_acknowledgement_and_exact_encoded_original() {
    let run_id = RunId::from_digest(mfm_ids::DigestBytes::from_array([79; 32]));
    let candidate = mfm_journal::seal_frame(
        &run_id,
        1,
        None,
        &mfm_canonical::PlainCanonicalJsonBytes::from_json_str("{}").unwrap(),
    )
    .unwrap();
    let head = mfm_store::RunSummary::new(
        run_id.clone(),
        1,
        candidate.head_digest().clone(),
        candidate.canonical_bytes().len() as u64,
    )
    .unwrap();
    let expected = serde_json::to_value(&head).unwrap();
    let error = RunRequestError::Invocation(InvocationFailure::Execution {
        run_id,
        last_observed: None,
        error: RuntimeError::Projection {
            acknowledged: Box::new(head),
            cause: Box::new(RuntimeError::ArithmeticOverflow),
        },
    });
    let diagnostic =
        InvocationDiagnostic::from_fields("output_io", "emit_error", &"write failed", None);
    let original =
        serde_json::value::RawValue::from_string("{\"ratio\":1.2300e-4}".into()).unwrap();
    let model = SerializableClientError::failed_run_report(
        &error,
        "projection failed",
        &diagnostic,
        Some(&original),
    );
    let bytes = encode_response(&model).unwrap();
    assert!(bytes
        .get()
        .contains("\"original_report\":{\"ratio\":1.2300e-4}"));
    let report: serde_json::Value = serde_json::from_str(bytes.get()).unwrap();
    assert_eq!(report["acknowledged"], expected);
    assert!(report.get("last_observed").is_none());
    assert_eq!(report["code"], error.code());
}

#[test]
fn encoding_calls_the_serializer_once_and_returns_its_concrete_error() {
    struct Failing(std::cell::Cell<u32>);
    impl Serialize for Failing {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            self.0.set(self.0.get() + 1);
            Err(serde::ser::Error::custom("distinct terminal failure"))
        }
    }
    let value = Failing(std::cell::Cell::new(0));
    let cause = encode_response(&value).unwrap_err();
    assert_eq!(value.0.get(), 1);
    let fields = serde_json::to_value(cause).unwrap();
    assert_eq!(fields["category"], "data");
    assert_eq!(fields["message"], "distinct terminal failure");
}
