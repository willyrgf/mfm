use super::*;

pub(super) fn parse_fact_field_value(
    value: &serde_json::Value,
    context: &'static str,
) -> Result<FactFieldValue> {
    let object = json_object(value, context)?;
    let field_id = FactFieldId::new(json_str(object, "field_id")?)?;
    let value_type = parse_field_value_type(json_required(object, "value_type")?)?;
    let scalar_value = json_required(object, "value")?;
    let scalar = parse_canonical_scalar_value(value_type, scalar_value, &field_id)?;
    FactFieldValue::new(field_id, value_type, scalar)
}

fn parse_field_value_type(value: &serde_json::Value) -> Result<FactFieldValueType> {
    let value = value
        .as_str()
        .ok_or_else(|| FactError::descriptor("field value type must be a string"))?;
    value
        .parse::<FactFieldValueType>()
        .map_err(|_| FactError::descriptor(format!("unknown field value type {value:?}")))
}

pub(crate) fn parse_canonical_scalar_value(
    value_type: FactFieldValueType,
    value: &serde_json::Value,
    field_id: &FactFieldId,
) -> Result<FactCanonicalScalar> {
    FactCanonicalScalar::from_json_value(value_type, value, ScalarJsonContext::Field(field_id))
}

fn parse_query_operator(field_id: &FactFieldId, value: &str) -> Result<FactQueryOperator> {
    value.parse::<FactQueryOperator>().map_err(|_| {
        FactError::field(
            field_id.clone(),
            format!("unknown query operator {value:?}"),
        )
    })
}

pub(super) fn parse_canonical_fact_query_plan(
    value: &serde_json::Value,
) -> Result<CanonicalFactQueryPlan> {
    let object = json_object(value, "fact query plan")?;
    require_version(object, "mfm.fact-query-plan.v1", "fact query plan")?;
    let canonical_query =
        canonical_json_bytes_from_canonical_json_str(json_str(object, "canonical_query")?)?;
    let expected_query_hash: ContentDigest = parse_json_str(object, "canonical_query_hash")?;
    let query_hash = canonical_query.content_digest();
    if query_hash != expected_query_hash {
        return Err(FactError::descriptor(
            "fact query plan canonical query hash mismatch",
        ));
    }
    let plan = CanonicalFactQueryPlan::new(
        StoreScopeRef::new(json_str(object, "store_scope")?)?,
        QueryScopeWire::parse(json_required(object, "query_scope")?)?,
        FactQueryCompilerVersion::new(json_str(object, "query_compiler_version")?)?,
        FactCanonicalizerVersion::new(json_str(object, "canonicalizer_version")?)?,
        parse_json_str(object, "resolved_descriptor")?,
        ScopeDecisionEvidenceWire::parse(json_required(object, "scope_decision_evidence")?)?,
        canonical_query,
        OrderingWire::parse(json_required(object, "ordering")?)?,
        json_optional_u64(object, "limit")?,
    )?;
    Ok(plan)
}

pub(super) fn parse_fact_query_receipt(
    value: &serde_json::Value,
    expected_plan_hash: &ContentDigest,
) -> Result<FactQueryReceipt> {
    let object = json_object(value, "fact query receipt")?;
    require_version(object, "mfm.fact-query-receipt.v2", "fact query receipt")?;
    let plan_hash: ContentDigest = parse_json_str(object, "plan_hash")?;
    if &plan_hash != expected_plan_hash {
        return Err(FactError::descriptor(
            "fact query receipt plan hash does not match plan",
        ));
    }
    let read_frontier = parse_store_read_frontier(json_required(object, "read_frontier")?)?;
    let frontier_type = parse_tag(
        json_str(object, "frontier_type")?,
        "store read frontier type",
    )?;
    let returned_refs = json_array(object, "returned_refs")?
        .iter()
        .map(parse_internal_fact_ref)
        .collect::<Result<Vec<_>>>()?;
    let returned_field_summaries = parse_optional_returned_field_summaries(json_required(
        object,
        "returned_field_summaries",
    )?)?;
    let result_set_digest = parse_json_str(object, "result_set_digest")?;
    let result_cardinality =
        parse_query_result_cardinality(json_required(object, "result_cardinality")?)?;
    Ok(FactQueryReceipt::from_parts(
        read_frontier,
        frontier_type,
        returned_refs,
        returned_field_summaries,
        result_set_digest,
        result_cardinality,
    ))
}

pub(super) fn parse_fact_selection_evidence(
    value: &serde_json::Value,
) -> Result<FactSelectionEvidence> {
    let object = json_object(value, "fact selection evidence")?;
    let selected_indices = json_array(object, "selected_indices")?
        .iter()
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| FactError::descriptor("selected index must be an unsigned integer"))
        })
        .collect::<Result<Vec<_>>>()?;
    FactSelectionEvidence::new(
        parse_json_str(object, "selection_policy_hash")?,
        selected_indices,
        parse_optional_digest(json_required(object, "selected_summaries_digest")?)?,
    )
}

fn parse_store_read_frontier(value: &serde_json::Value) -> Result<StoreReadFrontier> {
    let object = json_object(value, "store read frontier")?;
    Ok(StoreReadFrontier::new(
        StoreScopeRef::new(json_str(object, "store_scope")?)?,
        QueryScopeWire::parse(json_required(object, "query_scope")?)?,
        DescriptorCatalogWatermark::new(json_u64(object, "descriptor_catalog_watermark")?),
        StoreCommitOrder::new(json_u64(object, "store_commit_order")?),
    ))
}

fn parse_internal_fact_ref(value: &serde_json::Value) -> Result<InternalFactRef> {
    let object = json_object(value, "internal fact ref")?;
    let request_schema_id = parse_optional_identity(json_required(object, "request_schema_id")?)?;
    let request_hash = parse_optional_digest(json_required(object, "request_hash")?)?;
    let request = match (request_schema_id, request_hash) {
        (Some(request_schema_id), Some(request_hash)) => {
            Some(FactRequestEvidence::new(request_schema_id, request_hash))
        }
        (None, None) => None,
        _ => {
            return Err(FactError::descriptor(
                "request schema and hash must be present or absent together",
            ));
        }
    };
    InternalFactRef::new(InternalFactRefParts {
        fact_claim_id: parse_fact_claim_id(json_required(object, "fact_claim_id")?)?,
        source_event_id: parse_json_str(object, "source_event_id")?,
        recorded_at: json_str(object, "recorded_at")?.to_owned(),
        producer_node_id: parse_json_str(object, "producer_node_id")?,
        observed_at: parse_optional_string(json_required(object, "observed_at")?)?,
        visibility: parse_fact_visibility(json_required(object, "visibility")?)?,
        fact_kind: FactKind::new(json_str(object, "fact_kind")?)?,
        fact_descriptor_hash: parse_json_str(object, "fact_descriptor_hash")?,
        subject: FactSubjectRef::new(
            parse_json_str(object, "fact_subject_namespace_hash")?,
            FactKey::from_digest(parse_json_str(object, "fact_key")?),
            parse_json_str(object, "subject_material_hash")?,
        ),
        request,
        response: FactResponseEvidence::new(
            parse_json_str(object, "response_schema_id")?,
            parse_json_str(object, "response_hash")?,
            parse_json_str(object, "artifact_id")?,
            parse_json_str(object, "artifact_evidence_hash")?,
        ),
        producer: FactProducerProvenance::new(
            parse_json_str(object, "capability_kind")?,
            parse_json_str(object, "capability_version")?,
            parse_json_str(object, "adapter_kind")?,
            parse_json_str(object, "adapter_version")?,
        ),
    })
}

fn parse_fact_claim_id(value: &serde_json::Value) -> Result<FactClaimId> {
    let object = json_object(value, "fact claim id")?;
    require_version(object, "mfm.fact-claim-id.v1", "fact claim id")?;
    FactClaimId::new(
        parse_json_str(object, "source_run_id")?,
        json_u64(object, "source_seq")?,
        json_u32(object, "source_ordinal")?,
    )
}

fn parse_fact_visibility(value: &serde_json::Value) -> Result<FactVisibility> {
    let object = json_object(value, "fact visibility")?;
    match json_str(object, "kind")? {
        "run_private" => Ok(FactVisibility::RunPrivate),
        "indexed" => Ok(FactVisibility::Indexed {
            audience: parse_tag(json_str(object, "audience")?, "fact audience")?,
            scope: parse_tag(json_str(object, "scope")?, "fact visibility scope")?,
        }),
        value => Err(FactError::descriptor(format!(
            "unknown fact visibility kind {value:?}"
        ))),
    }
}

fn parse_optional_returned_field_summaries(
    value: &serde_json::Value,
) -> Result<Option<ReturnedFieldSummaries>> {
    if value.is_null() {
        return Ok(None);
    }
    let summaries = value
        .as_array()
        .ok_or_else(|| FactError::descriptor("returned field summaries must be an array"))?
        .iter()
        .map(parse_returned_fact_field_summary)
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(ReturnedFieldSummaries::new(summaries)))
}

fn parse_returned_fact_field_summary(
    value: &serde_json::Value,
) -> Result<ReturnedFactFieldSummary> {
    let object = json_object(value, "returned fact field summary")?;
    let fields = json_array(object, "fields")?
        .iter()
        .map(|value| parse_fact_field_value(value, "returned field value summary"))
        .collect::<Result<Vec<_>>>()?;
    Ok(ReturnedFactFieldSummary::new(
        parse_fact_claim_id(json_required(object, "fact_claim_id")?)?,
        fields,
    ))
}

fn parse_query_result_cardinality(value: &serde_json::Value) -> Result<QueryResultCardinality> {
    let object = json_object(value, "query result cardinality")?;
    match json_str(object, "kind")? {
        "exact" => Ok(QueryResultCardinality::Exact(json_u64(object, "value")?)),
        "at_least" => Ok(QueryResultCardinality::AtLeast(json_u64(object, "value")?)),
        value => Err(FactError::descriptor(format!(
            "unknown query result cardinality kind {value:?}"
        ))),
    }
}

fn canonical_json_bytes_from_canonical_json_str(value: &str) -> Result<CanonicalJsonBytes> {
    PlainCanonicalJsonBytes::from_canonical_json_slice(value.as_bytes())
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let json = serde_json::from_str::<serde_json::Value>(value)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let canonical = CanonicalJsonBytes::from_value(&plain_json_to_canonical_value(&json)?);
    if canonical.as_str() != value {
        return Err(FactError::canonical(
            "canonical JSON value did not round-trip",
        ));
    }
    Ok(canonical)
}

fn plain_json_to_canonical_value(value: &serde_json::Value) -> Result<CanonicalValue> {
    match value {
        serde_json::Value::Null => Ok(CanonicalValue::Null),
        serde_json::Value::Bool(value) => Ok(CanonicalValue::Bool(*value)),
        serde_json::Value::String(value) => Ok(CanonicalValue::String(value.clone())),
        serde_json::Value::Number(value) => {
            if let Some(value) = value.as_u64() {
                Ok(CanonicalValue::Unsigned(value))
            } else if let Some(value) = value.as_i64() {
                Ok(CanonicalValue::Signed(value))
            } else {
                Err(FactError::canonical(
                    "floating-point numbers are not valid canonical fact query JSON",
                ))
            }
        }
        serde_json::Value::Array(values) => values
            .iter()
            .map(plain_json_to_canonical_value)
            .collect::<Result<Vec<_>>>()
            .map(CanonicalValue::Array),
        serde_json::Value::Object(object) => {
            let entries = object
                .iter()
                .map(|(key, value)| Ok((key.clone(), plain_json_to_canonical_value(value)?)))
                .collect::<Result<Vec<_>>>()?;
            CanonicalValue::object(entries).map_err(|error| FactError::canonical(error.to_string()))
        }
    }
}

fn parse_tag<T>(value: &str, tag_name: &'static str) -> Result<T>
where
    T: FromStr,
{
    value
        .parse::<T>()
        .map_err(|_| FactError::descriptor(format!("unknown {tag_name} {value:?}")))
}

pub(super) fn json_object<'a>(
    value: &'a serde_json::Value,
    context: &'static str,
) -> Result<&'a serde_json::Map<String, serde_json::Value>> {
    value
        .as_object()
        .ok_or_else(|| FactError::descriptor(format!("{context} must be an object")))
}

pub(super) fn require_version(
    object: &serde_json::Map<String, serde_json::Value>,
    expected: &'static str,
    context: &'static str,
) -> Result<()> {
    let actual = json_str(object, "version")?;
    if actual == expected {
        Ok(())
    } else {
        Err(FactError::descriptor(format!(
            "{context} version {actual:?} is unsupported"
        )))
    }
}

pub(super) fn json_required<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<&'a serde_json::Value> {
    object
        .get(key)
        .ok_or_else(|| FactError::descriptor(format!("{key} is required")))
}

fn json_str<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<&'a str> {
    json_required(object, key)?
        .as_str()
        .ok_or_else(|| FactError::descriptor(format!("{key} must be a string")))
}

fn json_array<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<&'a [serde_json::Value]> {
    json_required(object, key)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| FactError::descriptor(format!("{key} must be an array")))
}

fn json_bool(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<bool> {
    json_required(object, key)?
        .as_bool()
        .ok_or_else(|| FactError::descriptor(format!("{key} must be a bool")))
}

fn json_u64(object: &serde_json::Map<String, serde_json::Value>, key: &'static str) -> Result<u64> {
    json_required(object, key)?
        .as_u64()
        .ok_or_else(|| FactError::descriptor(format!("{key} must be a u64")))
}

fn json_u32(object: &serde_json::Map<String, serde_json::Value>, key: &'static str) -> Result<u32> {
    let value = json_u64(object, key)?;
    u32::try_from(value).map_err(|_| FactError::descriptor(format!("{key} must fit in u32")))
}

fn json_optional_u64(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<Option<u64>> {
    let value = json_required(object, key)?;
    if value.is_null() {
        Ok(None)
    } else {
        value
            .as_u64()
            .map(Some)
            .ok_or_else(|| FactError::descriptor(format!("{key} must be null or u64")))
    }
}

fn parse_json_str<T>(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &'static str,
) -> Result<T>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    json_str(object, key)?
        .parse::<T>()
        .map_err(|error| FactError::descriptor(format!("{key} failed validation: {error}")))
}

fn parse_optional_identity<T>(value: &serde_json::Value) -> Result<Option<T>>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .ok_or_else(|| FactError::descriptor("optional identity must be null or string"))?
        .parse::<T>()
        .map(Some)
        .map_err(|error| FactError::descriptor(format!("identity failed validation: {error}")))
}

fn parse_optional_digest(value: &serde_json::Value) -> Result<Option<ContentDigest>> {
    parse_optional_identity(value)
}

fn parse_optional_string(value: &serde_json::Value) -> Result<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .map(|value| Some(value.to_owned()))
        .ok_or_else(|| FactError::descriptor("optional string must be null or string"))
}

pub(super) fn descriptor_fields_by_id(
    descriptor: &FactDescriptor,
) -> Result<BTreeMap<FactFieldId, &FactFieldDescriptor>> {
    let mut fields = BTreeMap::new();
    for field in descriptor.fields() {
        if fields.insert(field.field_id().clone(), field).is_some() {
            return Err(FactError::field(
                field.field_id().clone(),
                "duplicate descriptor field id",
            ));
        }
    }
    Ok(fields)
}

pub(super) fn required_query_field<'a>(
    fields_by_id: &'a BTreeMap<FactFieldId, &FactFieldDescriptor>,
    field_id: &FactFieldId,
) -> Result<&'a FactFieldDescriptor> {
    fields_by_id
        .get(field_id)
        .copied()
        .ok_or_else(|| FactError::field(field_id.clone(), "fact query references unknown field"))
}

pub(super) fn require_query_exposed(
    field: &FactFieldDescriptor,
    context: &'static str,
) -> Result<()> {
    if field.exposure() == FactFieldExposure::Hidden {
        return Err(FactError::field(
            field.field_id().clone(),
            format!("hidden field cannot be used in fact query {context}"),
        ));
    }
    Ok(())
}

pub(super) struct CompiledQueryWire {
    fact_kind: FactKind,
    resolved_descriptor: ContentDigest,
    predicates: Vec<FactQueryPredicate>,
    return_fields: Vec<FactFieldId>,
    ordering: FactOrderingName,
    limit: Option<u64>,
}

impl CompiledQueryWire {
    const VERSION: &'static str = "mfm.fact-query.v1";

    pub(super) fn from_parts(
        descriptor: &FactDescriptor,
        descriptor_hash: &ContentDigest,
        predicates: &[FactQueryPredicate],
        return_fields: &[FactFieldId],
        ordering: &FactOrderingName,
        limit: Option<u64>,
    ) -> Self {
        Self {
            fact_kind: descriptor.fact_kind().clone(),
            resolved_descriptor: descriptor_hash.clone(),
            predicates: predicates.to_vec(),
            return_fields: return_fields.to_vec(),
            ordering: ordering.clone(),
            limit,
        }
    }

    pub(super) fn parse(value: &serde_json::Value) -> Result<Self> {
        let object = json_object(value, "canonical fact query")?;
        require_version(object, Self::VERSION, "canonical fact query")?;
        let predicates = json_array(object, "predicates")?
            .iter()
            .map(QueryPredicateWire::parse)
            .collect::<Result<Vec<_>>>()?;
        let return_fields = json_array(object, "return_fields")?
            .iter()
            .map(parse_query_return_field)
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            fact_kind: FactKind::new(json_str(object, "fact_kind")?)?,
            resolved_descriptor: parse_json_str(object, "resolved_descriptor")?,
            predicates,
            return_fields,
            ordering: FactOrderingName::new(json_str(object, "ordering")?)?,
            limit: json_optional_u64(object, "limit")?,
        })
    }

    pub(super) fn validate_plan(&self, plan: &CanonicalFactQueryPlan) -> Result<()> {
        if &self.resolved_descriptor != plan.resolved_descriptor() {
            return Err(FactError::descriptor(
                "canonical fact query descriptor hash does not match plan",
            ));
        }
        if &self.ordering != plan.ordering().name() {
            return Err(FactError::descriptor(
                "canonical fact query ordering does not match plan",
            ));
        }
        if self.limit != plan.limit() {
            return Err(FactError::descriptor(
                "canonical fact query limit does not match plan",
            ));
        }
        Ok(())
    }

    pub(super) fn into_shape(self) -> Result<CompiledFactQueryShape> {
        CompiledFactQueryShape::new(self.predicates, self.return_fields)
    }

    fn canonical_value(&self) -> Result<CanonicalValue> {
        let predicates = self
            .predicates
            .iter()
            .map(QueryPredicateWire::canonical_value)
            .collect::<Result<Vec<_>>>()?;
        let return_fields = self
            .return_fields
            .iter()
            .map(|field_id| CanonicalValue::String(field_id.as_str().to_owned()))
            .collect::<Vec<_>>();
        canonical_object([
            ("version", CanonicalValue::String(Self::VERSION.to_owned())),
            (
                "fact_kind",
                CanonicalValue::String(self.fact_kind.as_str().to_owned()),
            ),
            (
                "resolved_descriptor",
                CanonicalValue::String(self.resolved_descriptor.as_str().to_owned()),
            ),
            ("predicates", CanonicalValue::Array(predicates)),
            ("return_fields", CanonicalValue::Array(return_fields)),
            (
                "ordering",
                CanonicalValue::String(self.ordering.as_str().to_owned()),
            ),
            (
                "limit",
                self.limit
                    .map(CanonicalValue::Unsigned)
                    .unwrap_or(CanonicalValue::Null),
            ),
        ])
    }

    pub(super) fn canonical_bytes(&self) -> Result<CanonicalJsonBytes> {
        self.canonical_value()
            .map(|value| CanonicalJsonBytes::from_value(&value))
    }
}

struct QueryPredicateWire;

impl QueryPredicateWire {
    fn parse(value: &serde_json::Value) -> Result<FactQueryPredicate> {
        let object = json_object(value, "query predicate")?;
        let field_id = FactFieldId::new(json_str(object, "field_id")?)?;
        let operator = parse_query_operator(&field_id, json_str(object, "operator")?)?;
        let value_type = parse_field_value_type(json_required(object, "value_type")?)?;
        let scalar_value = json_required(object, "value")?;
        let value = parse_canonical_scalar_value(value_type, scalar_value, &field_id)?;
        Ok(FactQueryPredicate::new(field_id, operator, value))
    }

    fn canonical_value(predicate: &FactQueryPredicate) -> Result<CanonicalValue> {
        canonical_object([
            (
                "field_id",
                CanonicalValue::String(predicate.field_id().as_str().to_owned()),
            ),
            (
                "operator",
                CanonicalValue::String(predicate.operator().as_str().to_owned()),
            ),
            (
                "value_type",
                CanonicalValue::String(predicate.value().value_type().as_str().to_owned()),
            ),
            ("value", predicate.value().canonical_value()),
        ])
    }
}

fn parse_query_return_field(value: &serde_json::Value) -> Result<FactFieldId> {
    let value = value
        .as_str()
        .ok_or_else(|| FactError::descriptor("query return field must be a string"))?;
    FactFieldId::new(value)
}

pub(crate) struct QueryScopeWire;

impl QueryScopeWire {
    fn parse(value: &serde_json::Value) -> Result<FactQueryScope> {
        let object = json_object(value, "fact query scope")?;
        Ok(FactQueryScope::new(
            parse_tag(json_str(object, "audience")?, "fact audience")?,
            parse_tag(json_str(object, "scope")?, "fact visibility scope")?,
        ))
    }

    pub(crate) fn canonical_value(scope: &FactQueryScope) -> Result<CanonicalValue> {
        canonical_object([
            (
                "audience",
                CanonicalValue::String(scope.audience().as_str().to_owned()),
            ),
            (
                "scope",
                CanonicalValue::String(scope.scope().as_str().to_owned()),
            ),
        ])
    }
}

pub(crate) struct ScopeDecisionEvidenceWire;

impl ScopeDecisionEvidenceWire {
    fn parse(value: &serde_json::Value) -> Result<ScopeDecisionEvidence> {
        let object = json_object(value, "scope decision evidence")?;
        Ok(ScopeDecisionEvidence::new(parse_json_str(
            object,
            "decision_hash",
        )?))
    }

    pub(crate) fn canonical_value(evidence: &ScopeDecisionEvidence) -> Result<CanonicalValue> {
        canonical_object([(
            "decision_hash",
            CanonicalValue::String(evidence.decision_hash().as_str().to_owned()),
        )])
    }
}

pub(crate) struct OrderingWire;

impl OrderingWire {
    fn parse(value: &serde_json::Value) -> Result<FactOrderingPolicy> {
        let object = json_object(value, "fact ordering")?;
        let terms = json_array(object, "terms")?
            .iter()
            .map(OrderingTermWire::parse)
            .collect::<Result<Vec<_>>>()?;
        FactOrderingPolicy::new(FactOrderingName::new(json_str(object, "name")?)?, terms)
    }

    pub(crate) fn canonical_value(ordering: &FactOrderingPolicy) -> Result<CanonicalValue> {
        let terms = ordering
            .terms()
            .iter()
            .map(OrderingTermWire::canonical_value)
            .collect::<Result<Vec<_>>>()?;
        canonical_object([
            (
                "name",
                CanonicalValue::String(ordering.name().as_str().to_owned()),
            ),
            ("terms", CanonicalValue::Array(terms)),
        ])
    }
}

pub(crate) struct OrderingTermWire;

impl OrderingTermWire {
    fn parse(value: &serde_json::Value) -> Result<FactOrderingTerm> {
        let object = json_object(value, "fact ordering term")?;
        Ok(FactOrderingTerm::new(
            FactFieldId::new(json_str(object, "field_id")?)?,
            parse_tag(json_str(object, "direction")?, "sort direction")?,
            parse_tag(json_str(object, "nulls")?, "null ordering")?,
            json_bool(object, "tie_breaker")?,
        ))
    }

    pub(crate) fn canonical_value(term: &FactOrderingTerm) -> Result<CanonicalValue> {
        canonical_object([
            (
                "field_id",
                CanonicalValue::String(term.field_id().as_str().to_owned()),
            ),
            (
                "direction",
                CanonicalValue::String(term.direction().as_str().to_owned()),
            ),
            (
                "nulls",
                CanonicalValue::String(term.nulls().as_str().to_owned()),
            ),
            ("tie_breaker", CanonicalValue::Bool(term.tie_breaker())),
        ])
    }
}
