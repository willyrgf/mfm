use super::*;

#[test]
fn query_plan_computes_canonical_query_hash_and_rejects_zero_limit() {
    let descriptor = descriptor(vec![
        subject_field("subject.chain"),
        sortable_result_field("result.height"),
    ])
    .expect("descriptor");
    let ordering = descriptor
        .orderings()
        .first()
        .expect("descriptor ordering")
        .clone();
    let canonical_query = CanonicalJsonBytes::from_value(
        &CanonicalValue::object([("kind", CanonicalValue::String("chain.head".into()))])
            .expect("query value"),
    );

    let plan = CanonicalFactQueryPlan::new(
        FactQueryCompilerVersion::new("mfm.facts.query.v3").expect("compiler"),
        FactCanonicalizerVersion::new("mfm.canonical.v1").expect("canonicalizer"),
        digest(1),
        canonical_query.clone(),
        ordering.clone(),
        Some(10),
    )
    .expect("plan");

    assert_eq!(
        plan.canonical_query_hash(),
        &canonical_query.content_digest()
    );
    assert!(CanonicalFactQueryPlan::new(
        FactQueryCompilerVersion::new("mfm.facts.query.v3").expect("compiler"),
        FactCanonicalizerVersion::new("mfm.canonical.v1").expect("canonicalizer"),
        digest(1),
        canonical_query,
        ordering,
        Some(0),
    )
    .is_err());
}

#[test]
fn query_compiler_builds_descriptor_scoped_canonical_plan() {
    let descriptor = descriptor(vec![
        subject_field("subject.chain"),
        sortable_result_field("result.height"),
        optional_result_field(
            "result.confirmations",
            "result.confirmations",
            "confirmations",
        ),
    ])
    .expect("descriptor");
    let descriptor_hash = fact_descriptor_hash(&descriptor).expect("descriptor hash");
    let input = default_query_input(
        vec![
            FactQueryPredicate::new(
                FactFieldId::new("subject.chain").expect("field"),
                FactQueryOperator::Equal,
                FactCanonicalScalar::string("bitcoin"),
            ),
            FactQueryPredicate::new(
                FactFieldId::new("result.height").expect("field"),
                FactQueryOperator::GreaterThan,
                FactCanonicalScalar::UnsignedInteger(800_000),
            ),
        ],
        &["subject.chain", "result.height"],
        Some(25),
    );

    let plan = compile_fact_query_plan(&descriptor, input).expect("plan");

    assert_eq!(plan.resolved_descriptor(), &descriptor_hash);
    assert_eq!(
        plan.query_compiler_version().as_str(),
        FACT_QUERY_COMPILER_VERSION
    );
    assert_eq!(
        plan.canonicalizer_version().as_str(),
        FACT_QUERY_CANONICALIZER_VERSION
    );
    assert_eq!(plan.limit(), Some(25));
    assert_eq!(plan.ordering().name().as_str(), "result.height.desc");
    assert_eq!(
        plan.canonical_query_hash(),
        &plan.canonical_query().content_digest()
    );
    let query: serde_json::Value =
        serde_json::from_slice(plan.canonical_query().as_bytes()).expect("query json");
    assert_eq!(query["version"], "mfm.fact-query.v3");
    assert_eq!(query["fact_kind"], "chain.head");
    assert_eq!(query["resolved_descriptor"], descriptor_hash.as_str());
    assert_eq!(query["limit"], 25);
    assert_eq!(query["ordering"], "result.height.desc");
    assert!(query["content_identity"].is_null());
    let predicates = query["predicates"].as_array().expect("predicates");
    assert_eq!(predicates[0]["field_id"], "result.height");
    assert_eq!(predicates[0]["operator"], "greater_than");
    assert_eq!(predicates[0]["value_type"], "unsigned_integer");
    assert_eq!(predicates[1]["field_id"], "subject.chain");
    let return_fields = query["return_fields"].as_array().expect("return fields");
    assert_eq!(return_fields[0], "subject.chain");
    assert_eq!(return_fields[1], "result.height");
    let parsed = parse_canonical_fact_query_shape(&plan).expect("parsed query shape");
    assert_eq!(parsed.predicates().len(), 2);
    assert_eq!(parsed.return_fields().len(), 2);
    assert!(parsed.content_identity().is_none());
}

#[test]
fn query_compiler_binds_exact_content_identity_to_the_descriptor() {
    let descriptor = descriptor(vec![
        subject_field("subject.chain"),
        sortable_result_field("result.height"),
    ])
    .expect("descriptor");
    let descriptor_hash = fact_descriptor_hash(&descriptor).expect("descriptor hash");
    let evidence: FactContentIdentityEvidence = serde_json::from_value(serde_json::json!({
        "fact_descriptor_hash": descriptor_hash.as_str(),
        "subject_material_hash": digest(41).as_str(),
        "response_schema_id": schema_id("mfm.test.response").as_str(),
        "response_hash": digest(42).as_str(),
    }))
    .expect("content identity evidence");
    let input = default_query_input(Vec::new(), &["result.height"], Some(1))
        .with_content_identity(evidence.clone());

    let plan = compile_fact_query_plan(&descriptor, input).expect("identity-pinned plan");
    let query: serde_json::Value =
        serde_json::from_slice(plan.canonical_query().as_bytes()).expect("query json");
    assert_eq!(
        query["content_identity"]["response_hash"],
        digest(42).as_str()
    );
    let shape = parse_canonical_fact_query_shape(&plan).expect("query shape");
    assert_eq!(shape.content_identity(), Some(&evidence));

    let mismatched: FactContentIdentityEvidence = serde_json::from_value(serde_json::json!({
        "fact_descriptor_hash": digest(99).as_str(),
        "subject_material_hash": digest(41).as_str(),
        "response_schema_id": schema_id("mfm.test.response").as_str(),
        "response_hash": digest(42).as_str(),
    }))
    .expect("mismatched evidence");
    let input = default_query_input(Vec::new(), &["result.height"], Some(1))
        .with_content_identity(mismatched);
    let error = compile_fact_query_plan(&descriptor, input).expect_err("descriptor mismatch");
    assert!(error
        .to_string()
        .contains("does not match resolved descriptor"));
}

#[test]
fn query_compiler_rejects_invalid_policy_and_predicate_shapes() {
    #[derive(Clone, Copy)]
    enum Case {
        HiddenPredicate,
        QueryOnlyReturn,
        UndeclaredOperator,
        WrongPredicateType,
    }

    let descriptor = descriptor(vec![
        subject_field("subject.chain"),
        sortable_result_field("result.height"),
        optional_result_field(
            "result.confirmations",
            "result.confirmations",
            "confirmations",
        ),
        metadata_recorded_at_field(),
    ])
    .expect("descriptor");

    for (case, expected) in [
        (Case::HiddenPredicate, "hidden field"),
        (Case::QueryOnlyReturn, "not returnable"),
        (Case::UndeclaredOperator, "not declared"),
        (Case::WrongPredicateType, "field expects"),
    ] {
        let input = match case {
            Case::HiddenPredicate => default_query_input(
                vec![FactQueryPredicate::new(
                    FactFieldId::new("metadata.recorded_at").expect("field"),
                    FactQueryOperator::GreaterThanOrEqual,
                    FactCanonicalScalar::timestamp("2026-01-02T00:00:00Z").expect("timestamp"),
                )],
                &["subject.chain"],
                Some(10),
            ),
            Case::QueryOnlyReturn => {
                default_query_input(Vec::new(), &["result.confirmations"], Some(10))
            }
            Case::UndeclaredOperator => default_query_input(
                vec![FactQueryPredicate::new(
                    FactFieldId::new("result.height").expect("field"),
                    FactQueryOperator::GreaterThanOrEqual,
                    FactCanonicalScalar::UnsignedInteger(800_000),
                )],
                &["result.height"],
                Some(10),
            ),
            Case::WrongPredicateType => default_query_input(
                vec![FactQueryPredicate::new(
                    FactFieldId::new("result.height").expect("field"),
                    FactQueryOperator::GreaterThan,
                    FactCanonicalScalar::string("800000"),
                )],
                &["result.height"],
                Some(10),
            ),
        };
        let error = compile_fact_query_plan(&descriptor, input).expect_err(expected);

        assert!(error.to_string().contains(expected));
    }
}

#[test]
fn selection_evidence_requires_sorted_unique_indices() {
    assert!(FactSelectionEvidence::new(digest(1), vec![0, 2, 4], None).is_ok());
    assert!(FactSelectionEvidence::new(digest(1), vec![0, 2, 2], None).is_err());
    assert!(FactSelectionEvidence::new(digest(1), vec![2, 1], None).is_err());
}
