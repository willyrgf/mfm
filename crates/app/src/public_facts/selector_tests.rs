use super::*;

#[test]
fn predicate_strings_decode_to_typed_query_primitives() {
    let predicates = parse_public_fact_predicates(
        ["network=bitcoin-mainnet"],
        ["amount_sat.gt=1000"],
        ["metadata.observed_at.lte=timestamp:2026-07-02T00:00:00Z"],
    )
    .expect("predicates");

    assert_eq!(predicates[0].field_id().as_str(), "subject.network");
    assert_eq!(
        predicates[0].operator(),
        mfm_facts::FactQueryOperator::Equal
    );
    assert_eq!(
        predicates[0].value(),
        &mfm_facts::FactCanonicalScalar::String("bitcoin-mainnet".to_owned())
    );
    assert_eq!(predicates[1].field_id().as_str(), "result.amount_sat");
    assert_eq!(
        predicates[1].operator(),
        mfm_facts::FactQueryOperator::GreaterThan
    );
    assert_eq!(
        predicates[1].value(),
        &mfm_facts::FactCanonicalScalar::UnsignedInteger(1000)
    );
    assert_eq!(predicates[2].field_id().as_str(), "metadata.observed_at");
    assert_eq!(
        predicates[2].operator(),
        mfm_facts::FactQueryOperator::LessThanOrEqual
    );
    assert_eq!(
        predicates[2].value(),
        &mfm_facts::FactCanonicalScalar::Timestamp("2026-07-02T00:00:00Z".to_owned())
    );
}

#[test]
fn selector_builds_typed_request_and_rejects_front_door_invariants() {
    let base_selector = PublicFactQuerySelector {
        return_fields: vec!["result.amount".to_owned()],
        ordering: Some("result.amount.asc".to_owned()),
        limit: Some(10),
        ..PublicFactQuerySelector::default()
    };

    let base = PublicFactQueryRequest::from_selector("mfm.app.test.launch", base_selector.clone())
        .expect("base request");
    assert_eq!(base.fact_kind.as_str(), "mfm.app.test.launch");
    assert_eq!(base.return_fields[0].as_str(), "result.amount");
    assert_eq!(base.ordering.as_str(), "result.amount.asc");
    assert_eq!(base.limit.map(NonZeroU64::get), Some(10));

    let mut missing_order = base_selector.clone();
    missing_order.ordering = None;
    assert_eq!(
        PublicFactQueryRequest::from_selector("mfm.app.test.launch", missing_order)
            .expect_err("missing order")
            .code,
        "FactOrderingMissing"
    );

    let mut zero_limit = base_selector;
    zero_limit.limit = Some(0);
    assert_eq!(
        PublicFactQueryRequest::from_selector("mfm.app.test.launch", zero_limit)
            .expect_err("zero limit")
            .code,
        "FactQueryLimitInvalid"
    );
}

#[test]
fn selector_parses_public_fact_query_pairs() {
    let selector = PublicFactQuerySelector::from_query_pairs([
        ("shape", "mfm.app.test.shape"),
        ("subject", "account=alice"),
        ("result", "amount.gte=u64:10"),
        ("where", "result.status=settled"),
        ("field", "result.amount"),
        ("fields", "result.status"),
        ("order", "result.amount.asc"),
        ("limit", "25"),
    ])
    .expect("query selector");

    assert_eq!(selector.shape.as_deref(), Some("mfm.app.test.shape"));
    assert_eq!(selector.subject_predicates, ["account=alice"]);
    assert_eq!(selector.result_predicates, ["amount.gte=u64:10"]);
    assert_eq!(selector.field_predicates, ["result.status=settled"]);
    assert_eq!(
        selector.return_fields,
        ["result.amount".to_owned(), "result.status".to_owned()]
    );
    assert_eq!(selector.ordering.as_deref(), Some("result.amount.asc"));
    assert_eq!(selector.limit, Some(25));
    assert_eq!(selector.with_limit_override(Some(1)).limit, Some(1));
    assert_eq!(
        PublicFactQuerySelector::from_query_pairs([("unknown", "value")])
            .expect_err("unknown key")
            .code,
        "FactQueryInvalidParameter"
    );
    assert_eq!(
        PublicFactQuerySelector::from_query_pairs([("limit", "not-a-number")])
            .expect_err("invalid limit")
            .code,
        "FactQueryInvalidParameter"
    );
}
