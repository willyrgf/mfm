use super::*;

#[test]
fn query_request_validation_rejects_empty_return_fields() {
    let request = PublicFactQueryRequest {
        fact_kind: mfm_facts::FactKind::new("mfm.app.test.launch").expect("fact kind"),
        shape: None,
        predicates: Vec::new(),
        return_fields: Vec::new(),
        ordering: mfm_facts::FactOrderingName::new("result.amount.asc").expect("ordering"),
        limit: None,
    };

    assert_eq!(
        request.validate().expect_err("missing return field").code,
        "FactReturnFieldMissing"
    );
}
