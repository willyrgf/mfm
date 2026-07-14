use super::*;
use mfm_app::{PublicFactDescriptorRef, PublicFactFieldValue, PublicFactScalarValue};

#[test]
fn rendered_public_fact_outputs_do_not_include_internal_fields() {
    let fact = sample_public_fact();
    let output = QueryOutput(PublicFactQueryPage {
        facts: vec![fact],
        next_cursor: None,
    });

    let json = serde_json::to_string(&output).expect("json");
    let text = output.to_string();
    for forbidden in [
        "artifact_id",
        "artifact_evidence_hash",
        "fact_descriptor_hash",
        "subject_material",
        "subject_material_hash",
        "response_hash",
        "source_run_id",
        "source_seq",
        "source_ordinal",
        "source_event_id",
        "event_id",
        "fact_key",
    ] {
        assert!(!json.contains(forbidden), "json leaked {forbidden}");
        assert!(!text.contains(forbidden), "text leaked {forbidden}");
    }
}

fn sample_public_fact() -> PublicFactRef {
    PublicFactRef {
        public_ref: PublicFactRefId::new(format!("pfr_{}", "a".repeat(64))).expect("public ref"),
        fact_kind: "wallet.balance".to_owned(),
        descriptor: PublicFactDescriptorRef {
            descriptor_schema_id: "mfm.wallet.balance.v1".to_owned(),
            subject_schema_id: "mfm.wallet.balance.subject.v1".to_owned(),
            response_schema_id: "mfm.wallet.balance.response.v1".to_owned(),
        },
        recorded_at: "2026-07-02T00:00:00Z".to_owned(),
        observed_at: Some("2026-07-02T00:00:00Z".to_owned()),
        fields: vec![PublicFactFieldValue {
            field_id: "result.amount_sat".to_owned(),
            path: "result.amount_sat".to_owned(),
            source: "result".to_owned(),
            value_type: "unsigned_integer".to_owned(),
            value: PublicFactScalarValue::UnsignedInteger(1000),
            unit: Some("sat".to_owned()),
            scale: None,
        }],
    }
}
