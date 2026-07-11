use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use mfm_canonical::{
    sha256_digest_bytes, CanonicalJsonBytes, CanonicalValue, PlainCanonicalJsonBytes,
};
use mfm_ids::{ContentDigest, DigestAlgorithm, RunId, SchemaId};

use crate::extraction::{
    extract_field_scalar, json_to_fact_canonical_value, response_typed_paths, subject_typed_paths,
    validate_extracted_scalar, validate_ordering, FactJsonPathContext,
};
use crate::scalar::ScalarJsonContext;
use crate::subject::{FactSubjectNamespaceFieldV1, FactSubjectNamespaceV1};
use crate::*;

#[path = "codec_canonical.rs"]
mod canonical;
use self::canonical::{
    canonical_descriptor_value, canonical_fact_claim_id_value,
    canonical_fact_query_result_set_value, canonical_query_evidence_value,
    canonical_query_plan_value, canonical_returned_fact_field_summary_value,
    canonical_subject_material_value, canonical_subject_namespace_value,
};

/// Validates a fact descriptor.
pub fn validate_descriptor(descriptor: &FactDescriptor) -> Result<()> {
    let mut fields_by_id = BTreeMap::new();
    let mut subject_fields = 0usize;

    for field in &descriptor.fields {
        field.validate()?;
        if fields_by_id.insert(field.field_id.clone(), field).is_some() {
            return Err(FactError::descriptor(format!(
                "duplicate field id {}",
                field.field_id
            )));
        }

        if matches!(field.extraction, FactFieldExtraction::Subject(_)) {
            subject_fields += 1;
            if !field.required {
                return Err(FactError::field(
                    field.field_id.clone(),
                    "subject fields must be required in v1",
                ));
            }
        }
    }

    if subject_fields == 0 {
        return Err(FactError::descriptor(
            "descriptor must contain at least one subject field",
        ));
    }

    let mut ordering_names = BTreeSet::new();
    for ordering in &descriptor.orderings {
        if !ordering_names.insert(ordering.name.clone()) {
            return Err(FactError::descriptor(format!(
                "duplicate ordering name {}",
                ordering.name
            )));
        }
        validate_ordering(ordering, &fields_by_id)?;
    }

    Ok(())
}

/// Returns the schema id for fact descriptor artifacts.
pub fn fact_descriptor_schema_id() -> Result<SchemaId> {
    facts_schema_id("mfm.fact_descriptor", FACTS_KERNEL_CONTRACT_VERSION)
}

/// Returns the schema id for canonical fact query evidence artifacts.
pub fn fact_query_evidence_schema_id() -> Result<SchemaId> {
    facts_schema_id(
        "mfm.fact_query_evidence",
        FACT_QUERY_EVIDENCE_CONTRACT_VERSION,
    )
}

fn facts_schema_id(name: &'static str, version: &'static str) -> Result<SchemaId> {
    SchemaId::new(
        name,
        version,
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(format!("schema:{name}:{version}").as_bytes()),
    )
    .map_err(|error| FactError::descriptor(error.to_string()))
}

/// Returns canonical descriptor bytes.
pub fn canonical_fact_descriptor_bytes(descriptor: &FactDescriptor) -> Result<CanonicalJsonBytes> {
    validate_descriptor(descriptor)?;
    canonical_descriptor_value(descriptor).map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Decodes and validates canonical fact descriptor artifact bytes.
pub fn parse_canonical_fact_descriptor_bytes(bytes: &[u8]) -> Result<FactDescriptor> {
    let descriptor = serde_json::from_slice::<FactDescriptor>(bytes)
        .map_err(|error| FactError::descriptor(error.to_string()))?;
    let canonical = canonical_fact_descriptor_bytes(&descriptor)?;
    if canonical.as_bytes() != bytes {
        return Err(FactError::descriptor(
            "fact descriptor bytes are not canonical",
        ));
    }
    Ok(descriptor)
}

/// Derives the content digest for a fact descriptor.
pub fn fact_descriptor_hash(descriptor: &FactDescriptor) -> Result<ContentDigest> {
    Ok(canonical_fact_descriptor_bytes(descriptor)?.content_digest())
}

fn fact_subject_namespace(descriptor: &FactDescriptor) -> Result<FactSubjectNamespaceV1> {
    validate_descriptor(descriptor)?;
    let fields = descriptor
        .fields
        .iter()
        .filter(|field| matches!(field.extraction, FactFieldExtraction::Subject(_)))
        .map(|field| {
            FactSubjectNamespaceFieldV1::new(
                field.field_id.clone(),
                field.value_type,
                field.unit.clone(),
                field.scale,
            )
        })
        .collect();
    FactSubjectNamespaceV1::new(descriptor.fact_kind.clone(), fields)
}

fn canonical_fact_subject_namespace_bytes(
    namespace: &FactSubjectNamespaceV1,
) -> Result<CanonicalJsonBytes> {
    canonical_subject_namespace_value(namespace).map(|value| CanonicalJsonBytes::from_value(&value))
}

fn subject_namespace_hash(namespace: &FactSubjectNamespaceV1) -> Result<ContentDigest> {
    Ok(canonical_fact_subject_namespace_bytes(namespace)?.content_digest())
}

/// Derives the content digest for a descriptor's canonical subject namespace.
pub fn fact_subject_namespace_hash(descriptor: &FactDescriptor) -> Result<ContentDigest> {
    let namespace = fact_subject_namespace(descriptor)?;
    subject_namespace_hash(&namespace)
}

/// Returns canonical subject material bytes.
pub fn canonical_fact_subject_material_bytes(
    material: &FactSubjectMaterialV1,
) -> Result<CanonicalJsonBytes> {
    canonical_subject_material_value(material).map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Decodes and validates canonical subject material bytes.
pub fn parse_canonical_fact_subject_material_bytes(bytes: &[u8]) -> Result<FactSubjectMaterialV1> {
    let value = serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| FactError::descriptor("subject material must be an object"))?;
    let version = object
        .get("version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| FactError::descriptor("subject material version is required"))?;
    if version != FactSubjectMaterialV1::VERSION {
        return Err(FactError::descriptor(format!(
            "unsupported subject material version {version:?}"
        )));
    }
    let values = object
        .get("values")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| FactError::descriptor("subject material values are required"))?
        .iter()
        .map(|value| parse_fact_field_value(value, "subject material value"))
        .collect::<Result<Vec<_>>>()?;
    let material = FactSubjectMaterialV1::new(values)?;
    let canonical = canonical_fact_subject_material_bytes(&material)?;
    if canonical.as_bytes() != bytes {
        return Err(FactError::descriptor(
            "subject material bytes are not canonical",
        ));
    }
    Ok(material)
}

/// Derives the content digest for subject material.
pub fn subject_material_hash(material: &FactSubjectMaterialV1) -> Result<ContentDigest> {
    Ok(canonical_fact_subject_material_bytes(material)?.content_digest())
}

/// Derives a fact key from a subject namespace hash and subject material hash.
pub fn derive_fact_key(
    fact_subject_namespace_hash: ContentDigest,
    subject_material_hash: ContentDigest,
) -> Result<FactKey> {
    let value = canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-key.v1".to_owned()),
        ),
        (
            "fact_subject_namespace_hash",
            CanonicalValue::String(fact_subject_namespace_hash.as_str().to_owned()),
        ),
        (
            "subject_material_hash",
            CanonicalValue::String(subject_material_hash.as_str().to_owned()),
        ),
    ])?;
    let bytes = CanonicalJsonBytes::from_value(&value);
    Ok(FactKey::from_digest(ContentDigest::from_digest(
        DigestAlgorithm::Sha256JcsV1,
        sha256_digest_bytes(bytes.as_bytes()),
    )))
}

/// Derives a fact claim id from run-stream coordinates.
pub fn derive_fact_claim_id(
    source_run_id: RunId,
    source_seq: u64,
    source_ordinal: u32,
) -> Result<FactClaimId> {
    FactClaimId::new(source_run_id, source_seq, source_ordinal)
}

/// Returns canonical fact claim id bytes.
pub fn canonical_fact_claim_id_bytes(claim_id: &FactClaimId) -> Result<CanonicalJsonBytes> {
    let value = canonical_fact_claim_id_value(claim_id)?;
    Ok(CanonicalJsonBytes::from_value(&value))
}

/// Returns canonical query plan bytes.
pub fn canonical_fact_query_plan_bytes(
    plan: &CanonicalFactQueryPlan,
) -> Result<CanonicalJsonBytes> {
    canonical_query_plan_value(plan).map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Derives the content digest for a canonical query plan.
pub fn fact_query_plan_hash(plan: &CanonicalFactQueryPlan) -> Result<ContentDigest> {
    Ok(canonical_fact_query_plan_bytes(plan)?.content_digest())
}

/// Compiles a descriptor-scoped query input into a canonical fact query plan.
pub fn compile_fact_query_plan(
    descriptor: &FactDescriptor,
    input: FactQueryInput,
) -> Result<CanonicalFactQueryPlan> {
    validate_descriptor(descriptor)?;
    let descriptor_hash = fact_descriptor_hash(descriptor)?;
    let fields_by_id = descriptor_fields_by_id(descriptor)?;
    let ordering_descriptor = descriptor
        .orderings()
        .iter()
        .find(|ordering| ordering.name() == input.ordering())
        .ok_or_else(|| {
            FactError::ordering(
                input.ordering().clone(),
                "fact query ordering is not declared by descriptor",
            )
        })?;

    let mut predicates = input.predicates().to_vec();
    predicates.sort();
    for predicate in &predicates {
        let field = required_query_field(&fields_by_id, predicate.field_id())?;
        require_query_exposed(field, "predicate")?;
        if !field.operators().contains(&predicate.operator()) {
            return Err(FactError::field(
                predicate.field_id().clone(),
                format!(
                    "operator {:?} is not declared for field {}",
                    predicate.operator(),
                    predicate.field_id()
                ),
            ));
        }
        validate_extracted_scalar(field, predicate.value())?;
    }

    for return_field in input.return_fields() {
        let field = required_query_field(&fields_by_id, return_field)?;
        if field.exposure() != FactFieldExposure::Returnable {
            return Err(FactError::field(
                return_field.clone(),
                "field is not returnable by descriptor exposure policy",
            ));
        }
    }

    for term in ordering_descriptor.terms() {
        let field = required_query_field(&fields_by_id, term.field_id())?;
        require_query_exposed(field, "ordering")?;
    }

    let ordering = ordering_descriptor.clone();
    let canonical_query = CompiledQueryWire::from_parts(
        descriptor,
        &descriptor_hash,
        &predicates,
        input.return_fields(),
        ordering.name(),
        input.limit(),
    )
    .canonical_bytes()?;
    CanonicalFactQueryPlan::new(
        input.store_scope,
        input.query_scope,
        FactQueryCompilerVersion::new(FACT_QUERY_COMPILER_VERSION)?,
        FactCanonicalizerVersion::new(FACT_QUERY_CANONICALIZER_VERSION)?,
        descriptor_hash,
        input.scope_decision_evidence,
        canonical_query,
        ordering,
        input.limit,
    )
}

/// Parses and validates the canonical query shape embedded in a query plan.
pub fn parse_canonical_fact_query_shape(
    plan: &CanonicalFactQueryPlan,
) -> Result<CompiledFactQueryShape> {
    if plan.canonical_query().content_digest() != *plan.canonical_query_hash() {
        return Err(FactError::descriptor(
            "fact query plan canonical query hash mismatch",
        ));
    }
    let value = serde_json::from_slice::<serde_json::Value>(plan.canonical_query().as_bytes())
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let query = CompiledQueryWire::parse(&value)?;
    query.validate_plan(plan)?;
    query.into_shape()
}

/// Parses canonical fact-query evidence bytes into the typed facts-kernel contract.
pub fn parse_canonical_fact_query_evidence_bytes(bytes: &[u8]) -> Result<FactQueryEvidence> {
    PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let value = serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let object = json_object(&value, "fact query evidence")?;
    require_version(
        object,
        FACT_QUERY_EVIDENCE_CONTRACT_VERSION,
        "fact query evidence",
    )?;
    let plan = parse_canonical_fact_query_plan(json_required(object, "plan")?)?;
    let plan_hash = fact_query_plan_hash(&plan)?;
    let receipt = parse_fact_query_receipt(json_required(object, "receipt")?, &plan_hash)?;
    let selection = parse_fact_selection_evidence(json_required(object, "selection")?)?;
    let evidence = FactQueryEvidence::new(plan, receipt, selection);
    validate_fact_query_evidence(&evidence)?;
    Ok(evidence)
}

/// Derives the result-set digest for the rows and summaries pinned in a receipt.
pub fn fact_query_result_set_digest(
    returned_refs: &[InternalFactRef],
    returned_field_summaries: Option<&ReturnedFieldSummaries>,
) -> Result<ContentDigest> {
    canonical_fact_query_result_set_value(returned_refs, returned_field_summaries)
        .map(|value| CanonicalJsonBytes::from_value(&value).content_digest())
}

/// Returns canonical query evidence bytes.
pub fn canonical_fact_query_evidence_bytes(
    evidence: &FactQueryEvidence,
) -> Result<CanonicalJsonBytes> {
    canonical_query_evidence_value(evidence).map(|value| CanonicalJsonBytes::from_value(&value))
}

/// Derives the content digest for fact query evidence.
pub fn fact_query_evidence_hash(evidence: &FactQueryEvidence) -> Result<ContentDigest> {
    Ok(canonical_fact_query_evidence_bytes(evidence)?.content_digest())
}

/// Validates fact-query replay evidence before it can be recorded or replayed.
///
/// This is facts-kernel structural validation only; the evidence artifact's content digest and
/// retained authority bindings provide its integrity.
pub fn validate_fact_query_evidence(evidence: &FactQueryEvidence) -> Result<()> {
    let receipt = evidence.receipt();

    if receipt.read_frontier().store_scope() != evidence.plan().store_scope() {
        return Err(FactError::descriptor(
            "fact query receipt frontier store scope does not match plan",
        ));
    }
    if receipt.read_frontier().query_scope() != evidence.plan().query_scope() {
        return Err(FactError::descriptor(
            "fact query receipt frontier query scope does not match plan",
        ));
    }

    let expected_result_set_digest =
        fact_query_result_set_digest(receipt.returned_refs(), receipt.returned_field_summaries())?;
    if &expected_result_set_digest != receipt.result_set_digest() {
        return Err(FactError::descriptor(
            "fact query receipt result-set digest does not match returned refs and summaries",
        ));
    }

    validate_returned_summaries(receipt)?;
    validate_fact_selection_evidence(receipt, evidence.selection())
}

fn validate_returned_summaries(receipt: &FactQueryReceipt) -> Result<()> {
    let Some(summaries) = receipt.returned_field_summaries() else {
        return Ok(());
    };
    if summaries.summaries().len() != receipt.returned_refs().len() {
        return Err(FactError::descriptor(
            "returned field summaries must align one-for-one with returned refs",
        ));
    }
    for (summary, fact_ref) in summaries.summaries().iter().zip(receipt.returned_refs()) {
        if summary.fact_claim_id() != fact_ref.fact_claim_id() {
            return Err(FactError::descriptor(
                "returned field summary claim id does not match returned ref",
            ));
        }
    }
    Ok(())
}

fn validate_fact_selection_evidence(
    receipt: &FactQueryReceipt,
    selection: &FactSelectionEvidence,
) -> Result<()> {
    let returned_len = receipt.returned_refs().len() as u64;
    for window in selection.selected_indices().windows(2) {
        if window[0] >= window[1] {
            return Err(FactError::descriptor(
                "selected indices must be sorted and unique",
            ));
        }
    }
    for index in selection.selected_indices() {
        if *index >= returned_len {
            return Err(FactError::descriptor(
                "selected index is outside returned refs",
            ));
        }
    }

    if let Some(selected_digest) = selection.selected_summaries_digest() {
        let summaries = receipt.returned_field_summaries().ok_or_else(|| {
            FactError::descriptor("selected summaries digest requires returned field summaries")
        })?;
        let expected =
            selected_returned_field_summaries_digest(summaries, selection.selected_indices())?;
        if &expected != selected_digest {
            return Err(FactError::descriptor(
                "selected summaries digest does not match selected returned summaries",
            ));
        }
    }
    Ok(())
}

/// Derives a digest over the returned field summaries selected by receipt index.
pub fn selected_returned_field_summaries_digest(
    returned_field_summaries: &ReturnedFieldSummaries,
    selected_indices: &[u64],
) -> Result<ContentDigest> {
    let mut selected = Vec::with_capacity(selected_indices.len());
    for index in selected_indices {
        let index = usize::try_from(*index)
            .map_err(|_| FactError::descriptor("selected summary index overflows usize"))?;
        let summary = returned_field_summaries
            .summaries()
            .get(index)
            .ok_or_else(|| FactError::descriptor("selected summary index is outside summaries"))?;
        selected.push(canonical_returned_fact_field_summary_value(summary)?);
    }
    let value = canonical_object([
        (
            "version",
            CanonicalValue::String("mfm.fact-query-selected-summaries.v1".to_owned()),
        ),
        ("summaries", CanonicalValue::Array(selected)),
    ])?;
    Ok(CanonicalJsonBytes::from_value(&value).content_digest())
}

/// Extracts canonical subject material using the descriptor's subject fields.
pub fn extract_subject_material(
    descriptor: &FactDescriptor,
    subject: &CanonicalValue,
) -> Result<FactSubjectMaterialV1> {
    validate_descriptor(descriptor)?;
    let mut values = Vec::new();
    for field in &descriptor.fields {
        if !matches!(field.extraction, FactFieldExtraction::Subject(_)) {
            continue;
        }
        let scalar = extract_field_scalar(field, subject, &CanonicalValue::Null, None)?
            .ok_or_else(|| {
                FactError::field(field.field_id.clone(), "required subject field missing")
            })?;
        validate_extracted_scalar(field, &scalar)?;
        values.push(FactFieldValue::new(
            field.field_id.clone(),
            field.value_type,
            scalar,
        )?);
    }
    FactSubjectMaterialV1::new(values)
}

/// Builds descriptor-derived subject evidence from an already-canonical subject value.
pub fn fact_subject_evidence(
    descriptor: &FactDescriptor,
    subject: &CanonicalValue,
) -> Result<FactSubjectEvidence> {
    let subject_material = extract_subject_material(descriptor, subject)?;
    fact_subject_evidence_from_material(descriptor, &subject_material)
}

/// Builds descriptor-derived subject evidence from already-extracted subject material.
pub fn fact_subject_evidence_from_material(
    descriptor: &FactDescriptor,
    subject_material: &FactSubjectMaterialV1,
) -> Result<FactSubjectEvidence> {
    let subject_namespace = fact_subject_namespace(descriptor)?;
    let subject_namespace_hash = subject_namespace_hash(&subject_namespace)?;
    FactSubjectEvidence::from_material(subject_namespace_hash, subject_material)
}

/// Normalizes a typed fact subject JSON value using descriptor-declared subject scalar types.
///
/// This preserves non-field JSON structure while converting descriptor-declared subject paths to
/// canonical scalar values. Null is rejected at typed subject paths because subject material only
/// admits concrete scalar values.
pub fn typed_fact_subject_value(
    descriptor: &FactDescriptor,
    subject: &serde_json::Value,
) -> Result<CanonicalValue> {
    validate_descriptor(descriptor)?;
    let typed_paths = subject_typed_paths(descriptor)?;
    json_to_fact_canonical_value(subject, "", &typed_paths, FactJsonPathContext::Subject)
}

/// Builds descriptor-derived subject evidence from a typed fact subject JSON value.
pub fn typed_fact_subject_evidence(
    descriptor: &FactDescriptor,
    subject: &serde_json::Value,
) -> Result<FactSubjectEvidence> {
    let subject = typed_fact_subject_value(descriptor, subject)?;
    fact_subject_evidence(descriptor, &subject)
}

/// Extracts subject, result, and metadata terms for an indexed fact claim.
pub fn extract_terms(
    descriptor: &FactDescriptor,
    subject: &CanonicalValue,
    response: &CanonicalValue,
    metadata: &FactExtractionMetadata,
) -> Result<Vec<FactIndexTerm>> {
    validate_descriptor(descriptor)?;
    let mut terms = Vec::new();
    for field in &descriptor.fields {
        match extract_field_scalar(field, subject, response, Some(metadata))? {
            Some(scalar) => terms.push(FactIndexTerm::from_field(field, scalar)?),
            None if field.required => {
                return Err(FactError::field(
                    field.field_id.clone(),
                    "required field missing",
                ));
            }
            None => {}
        }
    }
    Ok(terms)
}

/// Decodes canonical fact response bytes using descriptor-declared response scalar types.
pub fn parse_canonical_fact_response_bytes(
    descriptor: &FactDescriptor,
    bytes: &[u8],
) -> Result<CanonicalValue> {
    validate_descriptor(descriptor)?;
    PlainCanonicalJsonBytes::from_canonical_json_slice(bytes)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let value = serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|error| FactError::canonical(error.to_string()))?;
    let typed_paths = response_typed_paths(descriptor)?;
    json_to_fact_canonical_value(&value, "", &typed_paths, FactJsonPathContext::Response)
}

/// Extracts index terms from persisted subject material plus canonical response material.
pub fn extract_terms_from_material(
    descriptor: &FactDescriptor,
    subject_material: &FactSubjectMaterialV1,
    response: &CanonicalValue,
    metadata: &FactExtractionMetadata,
) -> Result<Vec<FactIndexTerm>> {
    validate_descriptor(descriptor)?;
    let subject_values = subject_material
        .values()
        .iter()
        .map(|value| (value.field_id().clone(), value))
        .collect::<BTreeMap<_, _>>();
    let mut terms = Vec::new();
    for field in &descriptor.fields {
        let scalar = match field.extraction() {
            FactFieldExtraction::Subject(_) => {
                let Some(value) = subject_values.get(field.field_id()) else {
                    if field.required() {
                        return Err(FactError::field(
                            field.field_id.clone(),
                            "required subject field missing",
                        ));
                    }
                    continue;
                };
                if value.value_type() != field.value_type() {
                    return Err(FactError::field(
                        field.field_id.clone(),
                        "subject material value type does not match descriptor field",
                    ));
                }
                Some(value.value().clone())
            }
            FactFieldExtraction::Response(_) | FactFieldExtraction::Metadata(_) => {
                extract_field_scalar(field, &CanonicalValue::Null, response, Some(metadata))?
            }
        };
        match scalar {
            Some(scalar) => terms.push(FactIndexTerm::from_field(field, scalar)?),
            None if field.required() => {
                return Err(FactError::field(
                    field.field_id.clone(),
                    "required field missing",
                ));
            }
            None => {}
        }
    }
    Ok(terms)
}

fn parse_fact_field_value(
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

fn parse_canonical_fact_query_plan(value: &serde_json::Value) -> Result<CanonicalFactQueryPlan> {
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

fn parse_fact_query_receipt(
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

fn parse_fact_selection_evidence(value: &serde_json::Value) -> Result<FactSelectionEvidence> {
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

fn json_object<'a>(
    value: &'a serde_json::Value,
    context: &'static str,
) -> Result<&'a serde_json::Map<String, serde_json::Value>> {
    value
        .as_object()
        .ok_or_else(|| FactError::descriptor(format!("{context} must be an object")))
}

fn require_version(
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

fn json_required<'a>(
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

fn descriptor_fields_by_id(
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

fn required_query_field<'a>(
    fields_by_id: &'a BTreeMap<FactFieldId, &FactFieldDescriptor>,
    field_id: &FactFieldId,
) -> Result<&'a FactFieldDescriptor> {
    fields_by_id
        .get(field_id)
        .copied()
        .ok_or_else(|| FactError::field(field_id.clone(), "fact query references unknown field"))
}

fn require_query_exposed(field: &FactFieldDescriptor, context: &'static str) -> Result<()> {
    if field.exposure() == FactFieldExposure::Hidden {
        return Err(FactError::field(
            field.field_id().clone(),
            format!("hidden field cannot be used in fact query {context}"),
        ));
    }
    Ok(())
}

struct CompiledQueryWire {
    fact_kind: FactKind,
    resolved_descriptor: ContentDigest,
    predicates: Vec<FactQueryPredicate>,
    return_fields: Vec<FactFieldId>,
    ordering: FactOrderingName,
    limit: Option<u64>,
}

impl CompiledQueryWire {
    const VERSION: &'static str = "mfm.fact-query.v1";

    fn from_parts(
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

    fn parse(value: &serde_json::Value) -> Result<Self> {
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

    fn validate_plan(&self, plan: &CanonicalFactQueryPlan) -> Result<()> {
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

    fn into_shape(self) -> Result<CompiledFactQueryShape> {
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

    fn canonical_bytes(&self) -> Result<CanonicalJsonBytes> {
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

struct QueryScopeWire;

impl QueryScopeWire {
    fn parse(value: &serde_json::Value) -> Result<FactQueryScope> {
        let object = json_object(value, "fact query scope")?;
        Ok(FactQueryScope::new(
            parse_tag(json_str(object, "audience")?, "fact audience")?,
            parse_tag(json_str(object, "scope")?, "fact visibility scope")?,
        ))
    }

    fn canonical_value(scope: &FactQueryScope) -> Result<CanonicalValue> {
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

struct ScopeDecisionEvidenceWire;

impl ScopeDecisionEvidenceWire {
    fn parse(value: &serde_json::Value) -> Result<ScopeDecisionEvidence> {
        let object = json_object(value, "scope decision evidence")?;
        Ok(ScopeDecisionEvidence::new(parse_json_str(
            object,
            "decision_hash",
        )?))
    }

    fn canonical_value(evidence: &ScopeDecisionEvidence) -> Result<CanonicalValue> {
        canonical_object([(
            "decision_hash",
            CanonicalValue::String(evidence.decision_hash().as_str().to_owned()),
        )])
    }
}

struct OrderingWire;

impl OrderingWire {
    fn parse(value: &serde_json::Value) -> Result<FactOrderingPolicy> {
        let object = json_object(value, "fact ordering")?;
        let terms = json_array(object, "terms")?
            .iter()
            .map(OrderingTermWire::parse)
            .collect::<Result<Vec<_>>>()?;
        FactOrderingPolicy::new(FactOrderingName::new(json_str(object, "name")?)?, terms)
    }

    fn canonical_value(ordering: &FactOrderingPolicy) -> Result<CanonicalValue> {
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

struct OrderingTermWire;

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

    fn canonical_value(term: &FactOrderingTerm) -> Result<CanonicalValue> {
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

pub(crate) fn canonical_object(
    entries: impl IntoIterator<Item = (impl Into<String>, CanonicalValue)>,
) -> Result<CanonicalValue> {
    CanonicalValue::object(entries).map_err(|error| FactError::canonical(error.to_string()))
}
