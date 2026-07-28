use mfm_canonical::RecoverabilityContractV1;
use mfm_replay::trace_export::{verify_portable_run_export, PORTABLE_RUN_EXPORT_MEDIA_TYPE};
use mfm_replay::v1::ReplayErrorKind;

#[path = "../../../../tests/support/recoverability_v1.rs"]
mod recoverability_v1_support;

#[test]
fn trace_export_executes_the_complete_recoverability_v1_corpus() {
    recoverability_v1_support::run_consumer("mfm-trace-export", |vector| {
        recoverability_v1_support::assert_lower_layer_owner_vector(vector);

        assert_eq!(
            PORTABLE_RUN_EXPORT_MEDIA_TYPE,
            "application/vnd.mfm.run-export.v1+json"
        );
        let contract = RecoverabilityContractV1::embedded().expect("recoverability contract");
        let incomplete = br#"{"members":[]}"#;
        let digest = contract.raw_content_digest(incomplete);
        let error = verify_portable_run_export(incomplete, &digest)
            .expect_err("partial bundle cannot verify offline");
        assert_eq!(error.kind(), ReplayErrorKind::InvalidExport);
        assert!(!error.to_string().contains(vector.id()));

        if vector.kind() == "export_identity" {
            let manifest = recoverability_v1_support::hex_field(vector.vector(), "manifest_hex");
            let external_digest = contract.raw_content_digest(&manifest);
            assert!(external_digest.as_str().starts_with("content:sha256-v1:"));
            let value: serde_json::Value =
                serde_json::from_slice(&manifest).expect("canonical export manifest");
            let object = value.as_object().expect("export manifest object");
            assert!(!object.keys().any(|key| key.contains("digest")));
        }
    });
}
