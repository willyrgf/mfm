use super::*;

pub(crate) fn execute_fact_query_projection(
    projection: &ProjectionSnapshot,
    plan: &mfm_facts::CanonicalFactQueryPlan,
) -> Result<Vec<mfm_facts::FactQueryResultRow>> {
    let shape = mfm_facts::parse_canonical_fact_query_shape(plan)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let mut rows = Vec::new();
    for (_claim_id, entry) in projection.fact_query_entries() {
        if entry.fact_descriptor_hash() != plan.resolved_descriptor() {
            continue;
        }
        let fact_ref = entry.internal_ref()?;
        if shape
            .content_identity()
            .is_some_and(|identity| !identity.matches_internal_ref(&fact_ref))
            || !entry_matches_predicates(entry, &shape)
        {
            continue;
        }
        rows.push(mfm_facts::FactQueryResultRow::new(
            fact_ref,
            returned_fields(entry, &shape)?,
        ));
    }
    rows.sort_by(|left, right| compare_rows(projection, plan, left, right));
    if let Some(limit) = plan.limit() {
        rows.truncate(rows.len().min(usize::try_from(limit).unwrap_or(usize::MAX)));
    }
    Ok(rows)
}

pub(crate) fn execute_fact_query_projection_result(
    projection: &ProjectionSnapshot,
    frontier: StoreReadFrontier,
    plan: &mfm_facts::CanonicalFactQueryPlan,
) -> Result<mfm_facts::FactQueryResult> {
    let shape = mfm_facts::parse_canonical_fact_query_shape(plan)
        .map_err(|error| StoreError::Identity(error.to_string()))?;
    let rows = execute_fact_query_projection(projection, plan)?;
    let receipt = mfm_facts::FactQueryReceipt::from_rows(
        frontier,
        &rows,
        !shape.return_fields().is_empty(),
        plan.limit(),
    )
    .map_err(|error| StoreError::Identity(error.to_string()))?;
    mfm_facts::FactQueryResult::new(rows, receipt)
        .map_err(|error| StoreError::Identity(error.to_string()))
}

fn entry_matches_predicates(
    projection: &FactQueryProjection,
    shape: &mfm_facts::CompiledFactQueryShape,
) -> bool {
    shape.predicates().iter().all(|predicate| {
        projection
            .term(predicate.field_id())
            .is_some_and(|term| predicate.matches_scalar(term.value()))
    })
}

fn returned_fields(
    projection: &FactQueryProjection,
    shape: &mfm_facts::CompiledFactQueryShape,
) -> Result<Vec<mfm_facts::FactFieldValue>> {
    shape
        .return_fields()
        .iter()
        .filter_map(|field_id| {
            projection.term(field_id).map(|term| {
                mfm_facts::FactFieldValue::new(
                    term.field_id().clone(),
                    term.value_type(),
                    term.value().clone(),
                )
                .map_err(|error| StoreError::Identity(error.to_string()))
            })
        })
        .collect()
}

fn compare_rows(
    projection: &ProjectionSnapshot,
    plan: &mfm_facts::CanonicalFactQueryPlan,
    left: &mfm_facts::FactQueryResultRow,
    right: &mfm_facts::FactQueryResultRow,
) -> std::cmp::Ordering {
    for term in plan.ordering().terms() {
        let left_value = projection
            .fact_query_entry(left.fact_ref().fact_claim_id())
            .and_then(|entry| entry.term(term.field_id()))
            .map(mfm_facts::FactQueryTerm::value);
        let right_value = projection
            .fact_query_entry(right.fact_ref().fact_claim_id())
            .and_then(|entry| entry.term(term.field_id()))
            .map(mfm_facts::FactQueryTerm::value);
        let ordering = term
            .compare_values(left_value, right_value)
            .unwrap_or(std::cmp::Ordering::Equal);
        if ordering != std::cmp::Ordering::Equal {
            return ordering;
        }
    }
    left.fact_ref()
        .fact_claim_id()
        .cmp(right.fact_ref().fact_claim_id())
}
