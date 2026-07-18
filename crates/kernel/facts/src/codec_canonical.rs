use super::parse::{OrderingTermWire, OrderingWire, QueryScopeWire, ScopeDecisionEvidenceWire};
use super::*;

pub(super) fn canonical_descriptor_value(descriptor: &FactDescriptor) -> Result<CanonicalValue> {
    let mut fields = descriptor.fields.iter().collect::<Vec<_>>();
    fields.sort_by(|left, right| left.field_id.cmp(&right.field_id));
    let fields = fields
        .into_iter()
        .map(canonical_field_descriptor_value)
        .collect::<Result<Vec<_>>>()?;

    let mut orderings = descriptor.orderings.iter().collect::<Vec<_>>();
    orderings.sort_by(|left, right| left.name.cmp(&right.name));
    let orderings = orderings
        .into_iter()
        .map(canonical_ordering_descriptor_value)
        .collect::<Result<Vec<_>>>()?;

    canonical_object([
        (
            "version",
            CanonicalValue::String(FACTS_KERNEL_CONTRACT_VERSION.to_owned()),
        ),
        (
            "fact_kind",
            CanonicalValue::String(descriptor.fact_kind.as_str().to_owned()),
        ),
        (
            "descriptor_schema_id",
            CanonicalValue::String(descriptor.descriptor_schema_id.as_str().to_owned()),
        ),
        (
            "subject_schema_id",
            CanonicalValue::String(descriptor.subject_schema_id.as_str().to_owned()),
        ),
        (
            "response_schema_id",
            CanonicalValue::String(descriptor.response_schema_id.as_str().to_owned()),
        ),
        ("fields", CanonicalValue::Array(fields)),
        ("orderings", CanonicalValue::Array(orderings)),
    ])
}

fn canonical_field_descriptor_value(field: &FactFieldDescriptor) -> Result<CanonicalValue> {
    let mut operators = field.operators.clone();
    operators.sort();
    let operators = operators
        .into_iter()
        .map(|operator| CanonicalValue::String(operator.as_str().to_owned()))
        .collect::<Vec<_>>();

    canonical_object([
        (
            "field_id",
            CanonicalValue::String(field.field_id.as_str().to_owned()),
        ),
        (
            "value_type",
            CanonicalValue::String(field.value_type.as_str().to_owned()),
        ),
        ("extraction", canonical_extraction_value(&field.extraction)?),
        ("operators", CanonicalValue::Array(operators)),
        (
            "exposure",
            CanonicalValue::String(field.exposure.as_str().to_owned()),
        ),
        ("unit", optional_display_value(field.unit.as_ref())),
        ("scale", optional_scale_value(field.scale)),
        ("sortable", CanonicalValue::Bool(field.sortable)),
        ("required", CanonicalValue::Bool(field.required)),
    ])
}

fn canonical_extraction_value(extraction: &FactFieldExtraction) -> Result<CanonicalValue> {
    match extraction {
        FactFieldExtraction::Subject(path) => canonical_object([
            (
                "source",
                CanonicalValue::String(FactFieldSource::Subject.as_str().to_owned()),
            ),
            ("path", CanonicalValue::String(path.as_str().to_owned())),
        ]),
        FactFieldExtraction::Response(path) => canonical_object([
            (
                "source",
                CanonicalValue::String(FactFieldSource::Result.as_str().to_owned()),
            ),
            ("path", CanonicalValue::String(path.as_str().to_owned())),
        ]),
        FactFieldExtraction::Metadata(field) => canonical_object([
            (
                "source",
                CanonicalValue::String(FactFieldSource::Metadata.as_str().to_owned()),
            ),
            ("path", CanonicalValue::String(field.as_str().to_owned())),
        ]),
    }
}

fn canonical_ordering_descriptor_value(ordering: &FactOrderingPolicy) -> Result<CanonicalValue> {
    let terms = ordering
        .terms
        .iter()
        .map(OrderingTermWire::canonical_value)
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        (
            "name",
            CanonicalValue::String(ordering.name.as_str().to_owned()),
        ),
        ("terms", CanonicalValue::Array(terms)),
    ])
}

pub(super) fn canonical_subject_namespace_value(
    namespace: &FactSubjectNamespaceV2,
) -> Result<CanonicalValue> {
    canonical_object([
        (
            "version",
            CanonicalValue::String(FactSubjectNamespaceV2::VERSION.to_owned()),
        ),
        (
            "fact_kind",
            CanonicalValue::String(namespace.fact_kind.as_str().to_owned()),
        ),
        (
            "subject_schema_id",
            CanonicalValue::String(namespace.subject_schema_id.as_str().to_owned()),
        ),
    ])
}

pub(super) fn canonical_subject_material_value(
    material: &FactSubjectMaterialV2,
) -> Result<CanonicalValue> {
    canonical_object([
        (
            "version",
            CanonicalValue::String(FactSubjectMaterialV2::VERSION.to_owned()),
        ),
        ("subject", material.subject.clone()),
    ])
}

fn canonical_fact_field_value(value: &FactFieldValue) -> Result<CanonicalValue> {
    canonical_object([
        (
            "field_id",
            CanonicalValue::String(value.field_id().as_str().to_owned()),
        ),
        (
            "value_type",
            CanonicalValue::String(value.value_type().as_str().to_owned()),
        ),
        ("value", value.value().canonical_value()),
    ])
}

pub(super) fn canonical_query_plan_value(plan: &CanonicalFactQueryPlan) -> Result<CanonicalValue> {
    canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-query-plan.v1".to_owned()),
        ),
        (
            "store_scope",
            CanonicalValue::String(plan.store_scope.as_str().to_owned()),
        ),
        (
            "query_scope",
            QueryScopeWire::canonical_value(&plan.query_scope)?,
        ),
        (
            "query_compiler_version",
            CanonicalValue::String(plan.query_compiler_version.as_str().to_owned()),
        ),
        (
            "canonicalizer_version",
            CanonicalValue::String(plan.canonicalizer_version.as_str().to_owned()),
        ),
        (
            "resolved_descriptor",
            CanonicalValue::String(plan.resolved_descriptor.as_str().to_owned()),
        ),
        (
            "scope_decision_evidence",
            ScopeDecisionEvidenceWire::canonical_value(&plan.scope_decision_evidence)?,
        ),
        (
            "canonical_query",
            CanonicalValue::String(plan.canonical_query.as_str().to_owned()),
        ),
        (
            "canonical_query_hash",
            CanonicalValue::String(plan.canonical_query_hash.as_str().to_owned()),
        ),
        ("ordering", OrderingWire::canonical_value(&plan.ordering)?),
        (
            "limit",
            plan.limit
                .map(CanonicalValue::Unsigned)
                .unwrap_or(CanonicalValue::Null),
        ),
    ])
}

fn canonical_query_receipt_value(
    plan_hash: &ContentDigest,
    receipt: &FactQueryReceipt,
) -> Result<CanonicalValue> {
    let returned_refs = receipt
        .returned_refs
        .iter()
        .map(canonical_internal_fact_ref_value)
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-query-receipt.v2".to_owned()),
        ),
        (
            "plan_hash",
            CanonicalValue::String(plan_hash.as_str().to_owned()),
        ),
        (
            "read_frontier",
            canonical_store_read_frontier_value(&receipt.read_frontier)?,
        ),
        (
            "frontier_type",
            CanonicalValue::String(receipt.frontier_type.as_str().to_owned()),
        ),
        ("returned_refs", CanonicalValue::Array(returned_refs)),
        (
            "returned_field_summaries",
            optional_returned_field_summaries_value(receipt.returned_field_summaries.as_ref())?,
        ),
        (
            "result_set_digest",
            CanonicalValue::String(receipt.result_set_digest.as_str().to_owned()),
        ),
        (
            "result_cardinality",
            canonical_query_result_cardinality_value(receipt.result_cardinality)?,
        ),
    ])
}

pub(super) fn canonical_fact_query_result_set_value(
    returned_refs: &[InternalFactRef],
    returned_field_summaries: Option<&ReturnedFieldSummaries>,
) -> Result<CanonicalValue> {
    let returned_refs = returned_refs
        .iter()
        .map(canonical_internal_fact_ref_value)
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-query-result-set.v1".to_owned()),
        ),
        ("returned_refs", CanonicalValue::Array(returned_refs)),
        (
            "returned_field_summaries",
            optional_returned_field_summaries_value(returned_field_summaries)?,
        ),
    ])
}

pub(super) fn canonical_query_evidence_value(
    evidence: &FactQueryEvidence,
) -> Result<CanonicalValue> {
    let plan_hash = fact_query_plan_hash(&evidence.plan)?;
    canonical_object([
        (
            "version",
            CanonicalValue::String(FACT_QUERY_EVIDENCE_CONTRACT_VERSION.to_owned()),
        ),
        ("plan", canonical_query_plan_value(&evidence.plan)?),
        (
            "receipt",
            canonical_query_receipt_value(&plan_hash, &evidence.receipt)?,
        ),
        (
            "selection",
            canonical_fact_selection_evidence_value(&evidence.selection)?,
        ),
    ])
}

fn canonical_store_read_frontier_value(frontier: &StoreReadFrontier) -> Result<CanonicalValue> {
    canonical_object([
        (
            "store_scope",
            CanonicalValue::String(frontier.store_scope.as_str().to_owned()),
        ),
        (
            "query_scope",
            QueryScopeWire::canonical_value(&frontier.query_scope)?,
        ),
        (
            "descriptor_catalog_watermark",
            CanonicalValue::Unsigned(frontier.descriptor_catalog_watermark.as_u64()),
        ),
        (
            "store_commit_order",
            CanonicalValue::Unsigned(frontier.store_commit_order.as_u64()),
        ),
    ])
}

fn canonical_internal_fact_ref_value(reference: &InternalFactRef) -> Result<CanonicalValue> {
    let parts = &reference.parts;
    canonical_object([
        (
            "fact_claim_id",
            canonical_fact_claim_id_value(&parts.fact_claim_id)?,
        ),
        (
            "source_event_id",
            CanonicalValue::String(parts.source_event_id.as_str().to_owned()),
        ),
        (
            "recorded_at",
            CanonicalValue::String(parts.recorded_at.clone()),
        ),
        (
            "producer_node_id",
            CanonicalValue::String(parts.producer_node_id.as_str().to_owned()),
        ),
        (
            "observed_at",
            optional_string_value(parts.observed_at.as_deref()),
        ),
        (
            "visibility",
            canonical_fact_visibility_value(&parts.visibility)?,
        ),
        (
            "fact_kind",
            CanonicalValue::String(parts.fact_kind.as_str().to_owned()),
        ),
        (
            "fact_descriptor_hash",
            CanonicalValue::String(parts.fact_descriptor_hash.as_str().to_owned()),
        ),
        (
            "fact_subject_namespace_hash",
            CanonicalValue::String(
                parts
                    .subject
                    .fact_subject_namespace_hash()
                    .as_str()
                    .to_owned(),
            ),
        ),
        (
            "fact_key",
            CanonicalValue::String(parts.subject.fact_key().as_str().to_owned()),
        ),
        (
            "subject_material_hash",
            CanonicalValue::String(parts.subject.subject_material_hash().as_str().to_owned()),
        ),
        (
            "request_schema_id",
            optional_display_value(
                parts
                    .request
                    .as_ref()
                    .map(FactRequestEvidence::request_schema_id),
            ),
        ),
        (
            "request_hash",
            optional_display_value(
                parts
                    .request
                    .as_ref()
                    .map(FactRequestEvidence::request_hash),
            ),
        ),
        (
            "response_schema_id",
            CanonicalValue::String(parts.response.response_schema_id().as_str().to_owned()),
        ),
        (
            "response_hash",
            CanonicalValue::String(parts.response.response_hash().as_str().to_owned()),
        ),
        (
            "artifact_id",
            CanonicalValue::String(parts.response.artifact_id().as_str().to_owned()),
        ),
        (
            "artifact_evidence_hash",
            CanonicalValue::String(parts.response.artifact_evidence_hash().as_str().to_owned()),
        ),
        (
            "capability_kind",
            CanonicalValue::String(parts.producer.capability_kind().as_str().to_owned()),
        ),
        (
            "capability_version",
            CanonicalValue::String(parts.producer.capability_version().as_str().to_owned()),
        ),
        (
            "adapter_kind",
            CanonicalValue::String(parts.producer.adapter_kind().as_str().to_owned()),
        ),
        (
            "adapter_version",
            CanonicalValue::String(parts.producer.adapter_version().as_str().to_owned()),
        ),
    ])
}

pub(super) fn canonical_fact_claim_id_value(claim_id: &FactClaimId) -> Result<CanonicalValue> {
    canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-claim-id.v1".to_owned()),
        ),
        (
            "source_run_id",
            CanonicalValue::String(claim_id.source_run_id.as_str().to_owned()),
        ),
        ("source_seq", CanonicalValue::Unsigned(claim_id.source_seq)),
        (
            "source_ordinal",
            CanonicalValue::Unsigned(u64::from(claim_id.source_ordinal)),
        ),
    ])
}

fn canonical_fact_visibility_value(visibility: &FactVisibility) -> Result<CanonicalValue> {
    match visibility {
        FactVisibility::RunPrivate => {
            canonical_object([("kind", CanonicalValue::String("run_private".to_owned()))])
        }
        FactVisibility::Indexed { audience, scope } => canonical_object([
            ("kind", CanonicalValue::String("indexed".to_owned())),
            (
                "audience",
                CanonicalValue::String(audience.as_str().to_owned()),
            ),
            ("scope", CanonicalValue::String(scope.as_str().to_owned())),
        ]),
    }
}

fn optional_returned_field_summaries_value(
    summaries: Option<&ReturnedFieldSummaries>,
) -> Result<CanonicalValue> {
    summaries
        .map(canonical_returned_field_summaries_value)
        .unwrap_or(Ok(CanonicalValue::Null))
}

fn canonical_returned_field_summaries_value(
    summaries: &ReturnedFieldSummaries,
) -> Result<CanonicalValue> {
    let summaries = summaries
        .summaries
        .iter()
        .map(canonical_returned_fact_field_summary_value)
        .collect::<Result<Vec<_>>>()?;
    Ok(CanonicalValue::Array(summaries))
}

pub(super) fn canonical_returned_fact_field_summary_value(
    summary: &ReturnedFactFieldSummary,
) -> Result<CanonicalValue> {
    let fields = summary
        .fields
        .iter()
        .map(canonical_fact_field_value)
        .collect::<Result<Vec<_>>>()?;
    canonical_object([
        (
            "fact_claim_id",
            canonical_fact_claim_id_value(&summary.fact_claim_id)?,
        ),
        ("fields", CanonicalValue::Array(fields)),
    ])
}

fn canonical_query_result_cardinality_value(
    cardinality: QueryResultCardinality,
) -> Result<CanonicalValue> {
    match cardinality {
        QueryResultCardinality::Exact(value) => canonical_object([
            ("kind", CanonicalValue::String("exact".to_owned())),
            ("value", CanonicalValue::Unsigned(value)),
        ]),
        QueryResultCardinality::AtLeast(value) => canonical_object([
            ("kind", CanonicalValue::String("at_least".to_owned())),
            ("value", CanonicalValue::Unsigned(value)),
        ]),
    }
}

fn canonical_fact_selection_evidence_value(
    selection: &FactSelectionEvidence,
) -> Result<CanonicalValue> {
    canonical_object([
        (
            "selection_policy_hash",
            CanonicalValue::String(selection.selection_policy_hash.as_str().to_owned()),
        ),
        (
            "selected_indices",
            CanonicalValue::Array(
                selection
                    .selected_indices
                    .iter()
                    .copied()
                    .map(CanonicalValue::Unsigned)
                    .collect(),
            ),
        ),
        (
            "selected_summaries_digest",
            optional_display_value(selection.selected_summaries_digest.as_ref()),
        ),
    ])
}

fn optional_string_value(value: Option<&str>) -> CanonicalValue {
    value
        .map(|value| CanonicalValue::String(value.to_owned()))
        .unwrap_or(CanonicalValue::Null)
}

fn optional_display_value<T>(value: Option<&T>) -> CanonicalValue
where
    T: fmt::Display,
{
    value
        .map(|value| CanonicalValue::String(value.to_string()))
        .unwrap_or(CanonicalValue::Null)
}

fn optional_scale_value(scale: Option<FactScale>) -> CanonicalValue {
    scale
        .map(|scale| CanonicalValue::Signed(i64::from(scale.exponent())))
        .unwrap_or(CanonicalValue::Null)
}
