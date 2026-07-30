use mfm_canonical::RecoverabilityContract;
use mfm_ids::ContentRef;
use mfm_replay::trace_export::{
    verify_portable_run_export_stream, PORTABLE_RUN_EXPORT_STREAM_MEDIA_TYPE,
};
use mfm_replay::ReplayErrorKind;

#[path = "../../../../tests/support/recoverability_v1.rs"]
mod recoverability_v1_support;

#[tokio::test]
async fn trace_export_executes_the_complete_recoverability_v1_corpus() {
    recoverability_v1_support::run_consumer("mfm-trace-export", |vector| {
        recoverability_v1_support::assert_lower_layer_owner_vector(vector);

        assert_eq!(
            PORTABLE_RUN_EXPORT_STREAM_MEDIA_TYPE,
            "application/vnd.mfm.run-export-stream.v1+json-seq"
        );

        if vector.kind() == "export_identity" {
            let contract = RecoverabilityContract::embedded().expect("recoverability contract");
            let stream = recoverability_v1_support::hex_field(vector.vector(), "stream_hex");
            let external_digest = contract.raw_content_digest(&stream);
            assert!(external_digest.as_str().starts_with("content:sha256-v1:"));
        }
    });

    let contract = RecoverabilityContract::embedded().expect("recoverability contract");
    let incomplete = b"";
    let expected_ref = ContentRef::new(
        contract
            .schema_id("mfm.portable-run-export-stream.v1")
            .expect("portable stream schema")
            .clone(),
        contract.raw_content_digest(incomplete),
    )
    .expect("portable stream reference");
    let error = verify_portable_run_export_stream(incomplete.as_slice(), &expected_ref)
        .await
        .expect_err("premature clean EOF cannot verify offline");
    assert_eq!(error.kind(), ReplayErrorKind::InvalidExport);
}
