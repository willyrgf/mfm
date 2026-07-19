use super::*;

#[test]
fn scalar_json_conversion_preserves_context_specific_behavior() {
    let field_id = FactFieldId::new("result.height").expect("field");
    let bad_decimal = serde_json::json!("12..34");
    let max_u64 = serde_json::json!(u64::MAX);

    assert_eq!(
        parse_canonical_scalar_value(
            FactFieldValueType::UnsignedInteger,
            &serde_json::Value::Null,
            &field_id,
        )
        .expect_err("null unsigned")
        .to_string(),
        "fact field result.height error: expected unsigned integer value"
    );
    assert_eq!(
        json_to_typed_fact_scalar(
            &serde_json::json!("12.34"),
            FactFieldValueType::DecimalString,
            "price",
        )
        .expect("decimal"),
        CanonicalValue::Decimal(DecimalString::new_variable("12.34").expect("decimal"))
    );
    assert!(parse_canonical_scalar_value(
        FactFieldValueType::DecimalString,
        &bad_decimal,
        &FactFieldId::new("result.price").expect("field"),
    )
    .expect_err("canonical decimal error")
    .to_string()
    .starts_with("fact canonicalization error:"));
    assert!(
        json_to_typed_fact_scalar(&bad_decimal, FactFieldValueType::DecimalString, "price")
            .expect_err("response decimal error")
            .to_string()
            .starts_with("fact descriptor error:")
    );
    let raw = digest(7).as_str().to_owned();
    assert_eq!(
        json_to_typed_fact_scalar(
            &serde_json::Value::String(raw.clone()),
            FactFieldValueType::Digest,
            "tx.hash",
        )
        .expect("digest"),
        CanonicalValue::String(raw)
    );
    assert_eq!(
        json_to_typed_fact_scalar(&max_u64, FactFieldValueType::UnsignedInteger, "height")
            .expect("u64 max"),
        CanonicalValue::Unsigned(u64::MAX)
    );
    assert_eq!(
            json_to_typed_fact_scalar(&max_u64, FactFieldValueType::SignedInteger, "height")
                .expect_err("u64 max signed")
                .to_string(),
            "fact descriptor error: fact response path height does not match declared value type SignedInteger"
        );

    let descriptor = descriptor(vec![
        subject_field("subject.chain"),
        sortable_result_field("result.height"),
    ])
    .expect("descriptor");
    let subject = typed_fact_subject_value(&descriptor, &serde_json::json!({ "chain": "bitcoin" }))
        .expect("typed subject");
    assert_eq!(
        subject,
        CanonicalValue::object([("chain", CanonicalValue::String("bitcoin".to_owned()))])
            .expect("canonical subject")
    );
    let subject_evidence =
        typed_fact_subject_evidence(&descriptor, &serde_json::json!({ "chain": "bitcoin" }))
            .expect("subject evidence");
    assert_eq!(
        subject_evidence.subject_material_hash(),
        &subject_material_hash(
            &extract_subject_material(&descriptor, &subject).expect("subject material")
        )
        .expect("subject material hash")
    );
    assert_eq!(
        typed_fact_subject_value(&descriptor, &serde_json::json!({ "chain": null }))
            .expect_err("null typed subject")
            .to_string(),
        "fact descriptor error: fact subject path chain is not a string"
    );
}

#[test]
fn fact_query_scalar_comparison_uses_numeric_decimal_ordering() {
    let two = FactCanonicalScalar::decimal_variable("2").expect("decimal");
    let ten = FactCanonicalScalar::decimal_variable("10").expect("decimal");
    let negative_two = FactCanonicalScalar::decimal_variable("-2").expect("decimal");
    let negative_ten = FactCanonicalScalar::decimal_variable("-10").expect("decimal");

    assert_eq!(two.query_cmp(&ten), Some(std::cmp::Ordering::Less));
    assert_eq!(
        negative_ten.query_cmp(&negative_two),
        Some(std::cmp::Ordering::Less)
    );
    assert!(ten.matches_query_operator(FactQueryOperator::GreaterThan, &two));
    assert!(negative_ten.matches_query_operator(FactQueryOperator::LessThan, &negative_two));
}

#[test]
fn fact_query_ordering_term_comparison_keeps_null_policy_independent_of_direction() {
    let field_id = FactFieldId::new("result.optional").expect("field");
    let value = FactCanonicalScalar::UnsignedInteger(7);
    for direction in [SortDirection::Ascending, SortDirection::Descending] {
        let nulls_first =
            FactOrderingTerm::new(field_id.clone(), direction, NullOrdering::First, false);
        let nulls_last =
            FactOrderingTerm::new(field_id.clone(), direction, NullOrdering::Last, false);

        assert_eq!(
            nulls_first.compare_values(None, Some(&value)),
            Some(std::cmp::Ordering::Less)
        );
        assert_eq!(
            nulls_first.compare_values(Some(&value), None),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            nulls_last.compare_values(None, Some(&value)),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            nulls_last.compare_values(Some(&value), None),
            Some(std::cmp::Ordering::Less)
        );
    }
}

#[test]
fn descriptor_accepts_valid_subject_result_and_ordering() {
    let descriptor = descriptor(vec![
        subject_field("subject.chain"),
        sortable_result_field("result.height"),
    ])
    .expect("valid descriptor");

    assert_eq!(descriptor.fact_kind().as_str(), "chain.head");
    assert_eq!(descriptor.fields().len(), 2);
}

#[test]
fn descriptor_rejects_invalid_field_sets() {
    let optional_subject = FactFieldDescriptor::new(
        FactFieldId::new("subject.network").expect("field id"),
        FactFieldValueType::String,
        FactFieldExtraction::Subject(CanonicalValuePath::new("network").expect("path")),
        FactFieldPolicy::new(
            vec![FactQueryOperator::Equal],
            FactFieldExposure::Returnable,
        ),
    )
    .expect("field constructor allows subject required check at descriptor level");

    for (fields, expected) in [
        (
            vec![
                subject_field("subject.chain"),
                subject_field("subject.chain"),
                sortable_result_field("result.height"),
            ],
            "duplicate field id",
        ),
        (
            vec![sortable_result_field("result.height")],
            "at least one subject field",
        ),
    ] {
        let error = descriptor(fields).expect_err(expected);

        assert!(error.to_string().contains(expected));
    }

    descriptor(vec![
        subject_field("subject.chain"),
        optional_subject,
        sortable_result_field("result.height"),
    ])
    .expect("optional union-arm subject fields are valid");
}

#[test]
fn field_path_is_derived_from_extraction() {
    let field = FactFieldDescriptor::new(
        FactFieldId::new("subject.chain").expect("field id"),
        FactFieldValueType::String,
        FactFieldExtraction::Subject(CanonicalValuePath::new("chain").expect("path")),
        FactFieldPolicy::new(
            vec![FactQueryOperator::Equal],
            FactFieldExposure::Returnable,
        )
        .required(),
    )
    .expect("field");

    assert_eq!(field.path(), "subject.chain");
}

#[test]
fn field_constructor_rejects_incompatible_operator() {
    let error = FactFieldDescriptor::new(
        FactFieldId::new("subject.chain").expect("field id"),
        FactFieldValueType::String,
        FactFieldExtraction::Subject(CanonicalValuePath::new("chain").expect("path")),
        FactFieldPolicy::new(
            vec![FactQueryOperator::GreaterThan],
            FactFieldExposure::Returnable,
        )
        .required(),
    )
    .expect_err("incompatible operator");

    assert!(error.to_string().contains("incompatible"));
}

#[test]
fn descriptor_rejects_invalid_ordering_terms() {
    enum Case {
        MissingField,
        NonSortableField,
    }

    for (case, expected) in [
        (Case::MissingField, "unknown field"),
        (Case::NonSortableField, "non-sortable"),
    ] {
        let error = match case {
            Case::MissingField => {
                let fields = vec![subject_field("subject.chain")];
                descriptor(fields).expect_err("missing ordering field")
            }
            Case::NonSortableField => FactDescriptor::new(
                FactKind::new("chain.head").expect("kind"),
                schema_id("mfm.test.fact.descriptor"),
                schema_id("mfm.test.subject"),
                schema_id("mfm.test.response"),
                vec![subject_field("subject.chain")],
                vec![FactOrderingPolicy::new(
                    FactOrderingName::new("subject.chain.asc").expect("ordering"),
                    vec![FactOrderingTerm::new(
                        FactFieldId::new("subject.chain").expect("field"),
                        SortDirection::Ascending,
                        NullOrdering::Last,
                        false,
                    )],
                )
                .expect("ordering")],
            )
            .expect_err("non-sortable ordering field"),
        };

        assert!(error.to_string().contains(expected));
    }
}

#[test]
fn checked_strings_reject_invalid_fact_kind() {
    let error = FactKind::new("Wallet Balance").expect_err("invalid kind");

    assert!(error.to_string().contains("lowercase ascii"));
}

#[test]
fn visibility_indexed_default_records_audience_and_scope() {
    let visibility = FactVisibility::indexed_default(FactAudience::Control);

    assert_eq!(
        visibility,
        FactVisibility::Indexed {
            audience: FactAudience::Control,
            scope: FactVisibilityScope::Default,
        }
    );
}
